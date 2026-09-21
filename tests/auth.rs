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
            anonymous_limit,
            trust_proxy_headers: false,
        }),
    }
}

async fn request(harness: &Harness, uri: &str, key: Option<&str>) -> (StatusCode, Value, Headers) {
    let mut builder = Request::builder().uri(uri);
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    let response = routes::router(Arc::clone(&harness.sampler), Arc::clone(&harness.auth))
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
    let response = routes::router(Arc::clone(&harness.sampler), Arc::clone(&harness.auth))
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
