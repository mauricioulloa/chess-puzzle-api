//! Authentication and rate limiting over the real HTTP stack.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chess_puzzle_api::api::catalog::Catalog;
use chess_puzzle_api::api::query::Sampler;
use chess_puzzle_api::api::routes;
use chess_puzzle_api::auth::keys::KeyStore;
use chess_puzzle_api::auth::middleware::AuthState;
use chess_puzzle_api::auth::ratelimit::RateLimiter;
use chess_puzzle_api::db;
use chess_puzzle_api::import::load::{self, Filters};
use http_body_util::BodyExt;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tower::ServiceExt;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/puzzles_sample.csv"
);

struct Harness {
    _dir: TempDir,
    sampler: Arc<Sampler>,
    auth: Arc<AuthState>,
}

fn harness(anonymous_limit: u32) -> Harness {
    build(anonymous_limit, false)
}

fn harness_behind_proxy(anonymous_limit: u32) -> Harness {
    build(anonymous_limit, true)
}

fn build(anonymous_limit: u32, trust_proxy_headers: bool) -> Harness {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("puzzles.db");

    let mut conn = db::open_for_import(&path).expect("open");
    load::load(
        &mut conn,
        &PathBuf::from(FIXTURE),
        Filters {
            min_popularity: 90,
            min_plays: 100,
            limit: None,
        },
    )
    .expect("import");
    load::finalise(&conn, false).expect("finalise");
    drop(conn);

    let pool = db::pool::open_read_only(&path, 2).expect("pool");
    let catalog = {
        let conn = pool.get().expect("connection");
        Catalog::load(&conn).expect("catalog")
    };

    Harness {
        _dir: dir,
        sampler: Arc::new(Sampler::new(pool, catalog)),
        auth: Arc::new(AuthState {
            store: Arc::new(KeyStore::in_memory().expect("key store")),
            limiter: Arc::new(RateLimiter::new()),
            usage: Mutex::new(HashMap::new()),
            stats: Arc::new(chess_puzzle_api::usage::Collector::new()),
            anonymous_limit,
            trust_proxy_headers,
        }),
    }
}

async fn request(harness: &Harness, uri: &str, key: Option<&str>) -> (StatusCode, Value, Headers) {
    let mut builder = Request::builder().uri(uri);
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    let response = routes::router(Arc::clone(&harness.sampler), Arc::clone(&harness.auth), &[])
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .expect("request");

    let status = response.status();
    let headers = Headers {
        limit: header(&response, "x-ratelimit-limit"),
        remaining: header(&response, "x-ratelimit-remaining"),
        scope: header(&response, "x-ratelimit-scope"),
        retry_after: header(&response, "retry-after"),
    };
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        headers,
    )
}

#[derive(Debug)]
struct Headers {
    limit: Option<String>,
    remaining: Option<String>,
    scope: Option<String>,
    retry_after: Option<String>,
}

fn header(response: &axum::response::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[tokio::test]
async fn anonymous_callers_are_allowed_but_throttled() {
    let harness = harness(3);

    for expected_remaining in (0..3).rev() {
        let (status, _, headers) = request(&harness, "/v1/puzzles/random", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.limit.as_deref(), Some("3"));
        assert_eq!(headers.remaining, Some(expected_remaining.to_string()));
        assert_eq!(headers.scope.as_deref(), Some("anonymous"));
    }

    let (status, body, headers) = request(&harness, "/v1/puzzles/random", None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"], "rate_limited");
    assert!(headers.retry_after.is_some(), "429 must say when to retry");
}

#[tokio::test]
async fn a_key_raises_the_ceiling() {
    let harness = harness(1);
    let key = harness.auth.store.create("generous", 50).expect("key");

    // Anonymous budget is one request; the key's is fifty.
    for _ in 0..10 {
        let (status, _, headers) = request(&harness, "/v1/puzzles/random", Some(&key)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.limit.as_deref(), Some("50"));
        assert_eq!(headers.scope.as_deref(), Some("key"));
    }
}

#[tokio::test]
async fn an_unknown_key_is_rejected_rather_than_treated_as_anonymous() {
    let harness = harness(1000);

    let (status, body, _) = request(&harness, "/v1/puzzles/random", Some("cpa_madeup")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn a_revoked_key_stops_working_immediately() {
    let harness = harness(1000);
    let key = harness.auth.store.create("temporary", 100).expect("key");

    let (status, _, _) = request(&harness, "/v1/puzzles/random", Some(&key)).await;
    assert_eq!(status, StatusCode::OK);

    harness.auth.store.revoke("temporary").expect("revoke");

    let (status, _, _) = request(&harness, "/v1/puzzles/random", Some(&key)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn health_is_never_rate_limited() {
    let harness = harness(1);

    // Exhaust the anonymous budget on a real endpoint first.
    request(&harness, "/v1/puzzles/random", None).await;
    let (status, _, _) = request(&harness, "/v1/puzzles/random", None).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    // A host polling health must not be told the service is over quota.
    for _ in 0..5 {
        let (status, _, _) = request(&harness, "/health", None).await;
        assert_eq!(status, StatusCode::OK);
    }
}

#[tokio::test]
async fn key_usage_is_counted() {
    let harness = harness(1000);
    let key = harness.auth.store.create("counted", 100).expect("key");

    for _ in 0..4 {
        request(&harness, "/v1/puzzles/random", Some(&key)).await;
    }

    let usage = harness.auth.take_usage();
    assert_eq!(usage.get(&1), Some(&4));
    assert!(
        harness.auth.take_usage().is_empty(),
        "taking usage must drain the buffer so flushes are not double counted"
    );
}

#[tokio::test]
async fn a_malformed_authorization_header_falls_back_to_anonymous() {
    let harness = harness(1000);

    // No Bearer scheme at all: treat it as an anonymous caller rather than
    // failing, since a stray header should not break a public endpoint.
    let response = routes::router(Arc::clone(&harness.sampler), Arc::clone(&harness.auth), &[])
        .oneshot(
            Request::builder()
                .uri("/v1/puzzles/random")
                .header("authorization", "Basic abc123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "x-ratelimit-scope").as_deref(),
        Some("anonymous")
    );
}

// --- usage accounting ----------------------------------------------------

/// Counters buffer in memory and the endpoint reads disk, so tests drain the
/// buffer the way the maintenance task does.
fn flush(harness: &Harness) {
    let (requests, filters) = harness.auth.stats.take();
    harness
        .auth
        .store
        .flush_stats(&requests, &filters)
        .expect("flush");
}

#[tokio::test]
async fn usage_counts_requests_by_endpoint_and_status() {
    let harness = harness(1000);

    for _ in 0..3 {
        request(&harness, "/v1/puzzles/random", None).await;
    }
    request(&harness, "/v1/themes", None).await;
    request(&harness, "/v1/puzzles/random?themes=nonsense", None).await; // 400
    flush(&harness);

    let (status, body, _) = request(&harness, "/v1/usage", None).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(body["totals"]["requests"], 5);
    assert_eq!(body["totals"]["errors"], 1, "the 400 counts as an error");

    let endpoints: Vec<&str> = body["byEndpoint"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["value"].as_str().unwrap())
        .collect();
    assert!(endpoints.contains(&"/v1/puzzles/random"));
    assert!(endpoints.contains(&"/v1/themes"));
}

#[tokio::test]
async fn endpoints_are_counted_by_route_not_by_url() {
    let harness = harness(1000);

    // Three different ids must not become three rows, or the table would grow
    // once per puzzle ever requested.
    for id in ["00008", "00014", "000rO"] {
        request(&harness, &format!("/v1/puzzles/{id}"), None).await;
    }
    flush(&harness);

    let (_, body, _) = request(&harness, "/v1/usage", None).await;
    let lookups: Vec<&serde_json::Value> = body["byEndpoint"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["value"] == "/v1/puzzles/{id}")
        .collect();

    assert_eq!(lookups.len(), 1, "one row for the route template");
    assert_eq!(lookups[0]["requests"], 3);
}

#[tokio::test]
async fn anonymous_and_keyed_traffic_are_reported_apart() {
    let harness = harness(1000);
    let key = harness.auth.store.create("reported", 100).expect("key");

    request(&harness, "/v1/puzzles/random", None).await;
    request(&harness, "/v1/puzzles/random", Some(&key)).await;
    request(&harness, "/v1/puzzles/random", Some(&key)).await;
    flush(&harness);

    let (_, body, _) = request(&harness, "/v1/usage", None).await;
    assert_eq!(body["totals"]["anonymous"], 1);
    assert_eq!(body["totals"]["keyed"], 2);
}

#[tokio::test]
async fn the_public_report_never_identifies_a_caller() {
    let harness = harness(1000);
    let key = harness
        .auth
        .store
        .create("a-very-distinctive-label", 100)
        .expect("key");

    request(&harness, "/v1/puzzles/random", Some(&key)).await;
    flush(&harness);

    let (_, body, _) = request(&harness, "/v1/usage", None).await;
    let serialised = body.to_string();

    assert!(
        !serialised.contains("a-very-distinctive-label"),
        "a public report must not name key holders"
    );
    assert!(
        !serialised.contains(&key),
        "and certainly not the key itself"
    );
    assert!(!serialised.contains("127.0.0.1"), "nor any client address");
}

#[tokio::test]
async fn popular_filters_are_recorded_in_canonical_names() {
    let harness = harness(1000);

    // Mixed case on the way in; the report should not split the count across
    // spellings of the same theme.
    request(&harness, "/v1/puzzles/random?themes=fork", None).await;
    request(&harness, "/v1/puzzles/random?themes=FORK", None).await;
    request(
        &harness,
        "/v1/puzzles/random?themes=fork&excludeThemes=endgame&rating=1500",
        None,
    )
    .await;
    flush(&harness);

    let (_, body, _) = request(&harness, "/v1/usage", None).await;
    let themes = body["popular"]["themes"].as_array().unwrap();
    assert_eq!(themes.len(), 1, "one entry, not one per spelling");
    assert_eq!(themes[0]["value"], "fork");
    assert_eq!(themes[0]["requests"], 3);

    assert_eq!(body["popular"]["excludedThemes"][0]["value"], "endgame");
    assert_eq!(body["popular"]["ratingBands"][0]["value"], "1400-1599");
}

#[tokio::test]
async fn searches_that_find_nothing_are_still_recorded() {
    let harness = harness(1000);

    // Nothing in the fixture is a fork outside the endgame in this band.
    let (status, _, _) = request(
        &harness,
        "/v1/puzzles/random?themes=fork&excludeThemes=endgame&rating=1500",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    flush(&harness);

    let (_, body, _) = request(&harness, "/v1/usage", None).await;
    let themes = body["popular"]["themes"].as_array().unwrap();

    assert_eq!(
        themes[0]["value"], "fork",
        "what someone asked for and did not get is the most useful signal \
         the counters carry: it shows where demand outruns the dataset"
    );
    assert_eq!(body["totals"]["errors"], 1);
}

// --- who the caller is -----------------------------------------------------

async fn get_with_headers(harness: &Harness, headers: &[(&str, &str)]) -> StatusCode {
    let mut builder = Request::builder().uri("/v1/puzzles/random");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    routes::router(Arc::clone(&harness.sampler), Arc::clone(&harness.auth), &[])
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .expect("request")
        .status()
}

#[tokio::test]
async fn a_caller_cannot_mint_rate_limit_buckets_by_forging_a_forwarded_header() {
    let harness = harness_behind_proxy(1);

    // A proxy appends what it saw, so the rightmost entry is the only one it
    // wrote. Everything left of it came from the caller. Reading the leftmost
    // value would let this caller have a fresh quota on every request just by
    // varying the prefix.
    let first = get_with_headers(&harness, &[("x-forwarded-for", "1.1.1.1, 203.0.113.9")]).await;
    assert_eq!(first, StatusCode::OK);

    let second = get_with_headers(&harness, &[("x-forwarded-for", "2.2.2.2, 203.0.113.9")]).await;
    assert_eq!(
        second,
        StatusCode::TOO_MANY_REQUESTS,
        "both requests came from 203.0.113.9 and must share one budget"
    );
}

#[tokio::test]
async fn distinct_clients_behind_the_same_proxy_keep_separate_budgets() {
    let harness = harness_behind_proxy(1);

    assert_eq!(
        get_with_headers(&harness, &[("x-forwarded-for", "203.0.113.1")]).await,
        StatusCode::OK
    );
    assert_eq!(
        get_with_headers(&harness, &[("x-forwarded-for", "203.0.113.2")]).await,
        StatusCode::OK,
        "throttling one client must not throttle the next"
    );
}

#[tokio::test]
async fn flys_own_header_is_preferred_over_the_forwarded_list() {
    let harness = harness_behind_proxy(1);

    // Fly sets Fly-Client-IP wholesale rather than appending, so there is
    // nothing for a caller to prepend to it.
    let headers = [
        ("x-forwarded-for", "1.1.1.1, 203.0.113.9"),
        ("fly-client-ip", "198.51.100.7"),
    ];
    assert_eq!(get_with_headers(&harness, &headers).await, StatusCode::OK);

    let other = [
        ("x-forwarded-for", "2.2.2.2, 203.0.113.9"),
        ("fly-client-ip", "198.51.100.7"),
    ];
    assert_eq!(
        get_with_headers(&harness, &other).await,
        StatusCode::TOO_MANY_REQUESTS,
        "the same Fly-Client-IP is the same caller whatever else is sent"
    );
}

#[tokio::test]
async fn forwarded_headers_are_ignored_unless_the_proxy_is_trusted() {
    // Default deployment: no proxy in front, so the headers are a caller's
    // invention and must not be believed.
    let harness = harness(1);

    assert_eq!(
        get_with_headers(&harness, &[("x-forwarded-for", "1.1.1.1")]).await,
        StatusCode::OK
    );
    assert_eq!(
        get_with_headers(&harness, &[("x-forwarded-for", "2.2.2.2")]).await,
        StatusCode::TOO_MANY_REQUESTS,
        "without a trusted proxy every request is the same unknown peer"
    );
}
