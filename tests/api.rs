//! End-to-end tests for the query layer and the HTTP surface, running against
//! a database built from the committed CSV fixture.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chess_puzzle_api::api::catalog::Catalog;
use chess_puzzle_api::api::query::{PuzzleFilter, Sampler, ThemesMode};
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

fn build_sampler() -> (TempDir, Arc<Sampler>) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("puzzles.db");

    let mut conn = db::open_for_import(&path).expect("open for import");
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

    let pool = db::pool::open_read_only(&path, 2).expect("read pool");
    let catalog = {
        let conn = pool.get().expect("connection");
        Catalog::load(&conn).expect("catalog")
    };
    (dir, Arc::new(Sampler::new(pool, catalog)))
}

/// A limit high enough that these tests never trip it; rate limiting has its
/// own suite.
fn permissive_auth() -> Arc<AuthState> {
    Arc::new(AuthState {
        store: Arc::new(KeyStore::in_memory().expect("key store")),
        limiter: Arc::new(RateLimiter::new()),
        usage: Mutex::new(HashMap::new()),
        anonymous_limit: u32::MAX,
        trust_proxy_headers: false,
    })
}

async fn get(sampler: Arc<Sampler>, uri: &str) -> (StatusCode, Value) {
    let response = routes::router(sampler, permissive_auth())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

// --- the one that matters most -------------------------------------------

#[tokio::test]
async fn no_endpoint_but_solution_reveals_the_answer() {
    let (_dir, sampler) = build_sampler();

    for uri in [
        "/v1/puzzles/random",
        "/v1/puzzles/random?count=5",
        "/v1/puzzles/00008",
    ] {
        let (status, body) = get(Arc::clone(&sampler), uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");

        let serialised = body.to_string();
        assert!(
            !serialised.contains("solution"),
            "{uri} leaked a solution field"
        );
        // 00008's answer starts e6e7; initialMove f2g3 is the opponent's move
        // and is allowed to appear.
        assert!(!serialised.contains("e6e7"), "{uri} leaked solution moves");
    }
}

#[tokio::test]
async fn the_solution_endpoint_returns_the_moves_after_the_opponents() {
    let (_dir, sampler) = build_sampler();
    let (status, body) = get(sampler, "/v1/puzzles/00008/solution").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["initialMove"], "f2g3");
    assert_eq!(
        body["solution"],
        serde_json::json!(["e6e7", "b2b1", "b3c1", "b1c1", "h6c1"])
    );
}

// --- routing and errors ---------------------------------------------------

#[tokio::test]
async fn random_is_not_swallowed_by_the_id_route() {
    let (_dir, sampler) = build_sampler();

    let (status, body) = get(Arc::clone(&sampler), "/v1/puzzles/random").await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(
        body["id"], "random",
        "static route must win over the {{id}} route"
    );

    let (status, _) = get(sampler, "/v1/puzzles/definitelynotanid").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bad_parameters_are_rejected_rather_than_ignored() {
    let (_dir, sampler) = build_sampler();

    for (uri, expected) in [
        ("/v1/puzzles/random?themes=frok", StatusCode::BAD_REQUEST),
        ("/v1/puzzles/random?count=0", StatusCode::BAD_REQUEST),
        ("/v1/puzzles/random?count=51", StatusCode::BAD_REQUEST),
        (
            "/v1/puzzles/random?themesMode=maybe",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/puzzles/random?rating=1500&ratingMin=1200",
            StatusCode::BAD_REQUEST,
        ),
        ("/v1/puzzles/random?tolerance=50", StatusCode::BAD_REQUEST),
        ("/v1/puzzles/random?typo=1", StatusCode::BAD_REQUEST),
    ] {
        let (status, _) = get(Arc::clone(&sampler), uri).await;
        assert_eq!(status, expected, "{uri}");
    }
}

#[tokio::test]
async fn out_of_range_ratings_name_the_parameter_the_caller_typed() {
    let (_dir, sampler) = build_sampler();
    let (status, body) = get(sampler, "/v1/puzzles/random?rating=9999").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    let message = body["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("9999"),
        "error should quote what the caller typed, got: {message}"
    );
}

#[tokio::test]
async fn an_impossible_filter_is_a_no_match_not_an_empty_success() {
    let (_dir, sampler) = build_sampler();
    let (status, body) = get(sampler, "/v1/puzzles/random?ratingMin=3900&ratingMax=4000").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "no_match");
}

// --- sampling semantics ---------------------------------------------------

#[tokio::test]
async fn batch_returns_distinct_puzzles_and_caps_at_what_exists() {
    let (_dir, sampler) = build_sampler();
    // The fixture only holds nine puzzles; asking for 20 must not hang or
    // repeat, it must return the nine that exist.
    let (status, body) = get(sampler, "/v1/puzzles/random?count=20").await;

    assert_eq!(status, StatusCode::OK);
    let puzzles = body["puzzles"].as_array().expect("array");
    assert_eq!(puzzles.len(), 9);

    let mut ids: Vec<&str> = puzzles.iter().map(|p| p["id"].as_str().unwrap()).collect();
    ids.sort_unstable();
    let distinct = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), distinct, "a batch must not repeat a puzzle");
}

#[test]
fn filters_are_honoured_across_many_draws() {
    let (_dir, sampler) = build_sampler();
    let fork = sampler.catalog.get("fork").expect("fork theme").id;
    let endgame = sampler.catalog.get("endgame").expect("endgame theme").id;

    let filter = PuzzleFilter {
        include: vec![fork],
        exclude: vec![endgame],
        ..Default::default()
    };

    let mut seen = Vec::new();
    for _ in 0..40 {
        let rows = sampler.random(&filter, 1).expect("sample");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!(row.mask.contains(fork), "{} lacks fork", row.puzzle_id);
        assert!(
            !row.mask.contains(endgame),
            "{} should have been excluded",
            row.puzzle_id
        );
        seen.push(row.puzzle_id.clone());
    }
    // 000rO is the only fixture puzzle with fork and without endgame.
    assert!(seen.iter().all(|id| id == "000rO"));
}

#[test]
fn rating_bounds_are_respected() {
    let (_dir, sampler) = build_sampler();
    let filter = PuzzleFilter {
        rating_min: Some(1500),
        rating_max: Some(1700),
        ..Default::default()
    };

    for _ in 0..40 {
        for row in sampler.random(&filter, 1).expect("sample") {
            assert!(
                (1500..=1700).contains(&row.rating),
                "{} rated {} escaped the band",
                row.puzzle_id,
                row.rating
            );
        }
    }
}

#[test]
fn unconstrained_sampling_eventually_reaches_every_puzzle() {
    let (_dir, sampler) = build_sampler();
    let filter = PuzzleFilter::default();

    let mut seen = std::collections::HashSet::new();
    for _ in 0..400 {
        for row in sampler.random(&filter, 1).expect("sample") {
            seen.insert(row.puzzle_id.clone());
        }
    }
    assert_eq!(
        seen.len(),
        9,
        "uniform sampling must cover all nine puzzles"
    );
}

#[test]
fn any_mode_can_return_either_theme() {
    let (_dir, sampler) = build_sampler();
    let pin = sampler.catalog.get("pin").expect("pin").id;
    let fork = sampler.catalog.get("fork").expect("fork").id;

    let filter = PuzzleFilter {
        include: vec![pin, fork],
        mode: ThemesMode::Any,
        ..Default::default()
    };

    let mut seen = std::collections::HashSet::new();
    for _ in 0..200 {
        for row in sampler.random(&filter, 1).expect("sample") {
            assert!(row.mask.contains(pin) || row.mask.contains(fork));
            seen.insert(row.puzzle_id.clone());
        }
    }
    // 000ul has pin; 000rO and 0013r have fork.
    assert!(
        seen.len() >= 3,
        "any-mode should reach all three, saw {seen:?}"
    );
}

#[test]
fn solver_colour_is_the_opposite_of_the_fen_side_to_move() {
    let (_dir, sampler) = build_sampler();

    let row = sampler
        .by_puzzle_id("00008")
        .expect("query")
        .expect("found");
    // FEN says black to move; that move is the opponent's, so white solves.
    assert!(row.fen.contains(" b "));
    assert_eq!(row.solver_color(), "white");
    assert_eq!(row.initial_move(), "f2g3");
}
