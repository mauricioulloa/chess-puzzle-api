//! The MCP surface, exercised over the real HTTP transport.

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
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tower::ServiceExt;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/puzzles_sample.csv"
);

fn harness() -> (TempDir, Arc<Sampler>, Arc<AuthState>) {
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

    let auth = Arc::new(AuthState {
        store: Arc::new(KeyStore::in_memory().expect("keys")),
        limiter: Arc::new(RateLimiter::new()),
        usage: Mutex::new(HashMap::new()),
        stats: Arc::new(chess_puzzle_api::usage::Collector::new()),
        anonymous_limit: u32::MAX,
        trust_proxy_headers: false,
    });

    (dir, Arc::new(Sampler::new(pool, catalog)), auth)
}

async fn rpc(sampler: &Arc<Sampler>, auth: &Arc<AuthState>, body: Value) -> Value {
    let response = routes::router(Arc::clone(sampler), Arc::clone(auth), &[])
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                // rmcp rejects a request without Host as DNS-rebinding defence.
                // Real clients always send it; a hand-built test request does not.
                .header("host", "127.0.0.1:8080")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .expect("request");

    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "MCP transport failed: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).expect("json-rpc response")
}

async fn call(sampler: &Arc<Sampler>, auth: &Arc<AuthState>, tool: &str, args: Value) -> Value {
    rpc(
        sampler,
        auth,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": args}
        }),
    )
    .await
}

#[tokio::test]
async fn initialize_describes_the_server_and_how_to_use_it() {
    let (_dir, sampler, auth) = harness();
    let response = rpc(
        &sampler,
        &auth,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                       "clientInfo": {"name": "test", "version": "1"}}
        }),
    )
    .await;

    let result = &response["result"];
    assert_eq!(result["serverInfo"]["name"], "chess-puzzle-api");
    assert!(!result["capabilities"]["tools"].is_null());

    // The instructions carry the things a model gets wrong unprompted.
    let instructions = result["instructions"].as_str().expect("instructions");
    assert!(instructions.contains("get_solution"));
    assert!(instructions.contains("SAN"));
    assert!(instructions.contains("list_themes"));
}

#[tokio::test]
async fn every_tool_is_advertised_with_a_schema() {
    let (_dir, sampler, auth) = harness();
    let response = rpc(
        &sampler,
        &auth,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}),
    )
    .await;

    let tools = response["result"]["tools"].as_array().expect("tools");
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();

    for expected in [
        "get_random_puzzle",
        "get_puzzle",
        "get_solution",
        "list_themes",
        "get_dataset_stats",
    ] {
        assert!(names.contains(&expected), "{expected} is not advertised");
    }

    for tool in tools {
        assert!(
            tool["description"].as_str().unwrap().len() > 40,
            "{} needs a description a model can act on",
            tool["name"]
        );
        assert!(!tool["inputSchema"].is_null());
    }
}

#[tokio::test]
async fn a_puzzle_arrives_ready_to_reason_about() {
    let (_dir, sampler, auth) = harness();
    let response = call(&sampler, &auth, "get_puzzle", json!({"puzzle_id": "00008"})).await;
    let puzzle = &response["result"]["structuredContent"];

    assert_eq!(puzzle["puzzle_id"], "00008");
    // The opponent's move is already applied, so the model is not asked to do
    // board arithmetic before it can start.
    assert!(puzzle["position_fen"].as_str().unwrap().contains(" w "));
    assert_eq!(puzzle["side_to_move"], "white");
    assert_eq!(puzzle["opponent_last_move"], "Bxg3");
    assert_eq!(puzzle["board"].as_str().unwrap().lines().count(), 9);
}

#[tokio::test]
async fn fetching_a_puzzle_never_reveals_its_answer() {
    let (_dir, sampler, auth) = harness();

    for (tool, args) in [
        ("get_puzzle", json!({"puzzle_id": "00008"})),
        ("get_random_puzzle", json!({"count": 5})),
    ] {
        let response = call(&sampler, &auth, tool, args).await;
        let serialised = response.to_string();

        assert!(
            !serialised.contains("Rxe7"),
            "{tool} leaked the answer in SAN"
        );
        assert!(
            !serialised.contains("e6e7"),
            "{tool} leaked the answer in UCI"
        );
        assert!(
            !serialised.contains("solution"),
            "{tool} mentions a solution"
        );
    }
}

#[tokio::test]
async fn the_solution_tool_returns_both_notations_aligned() {
    let (_dir, sampler, auth) = harness();
    let response = call(
        &sampler,
        &auth,
        "get_solution",
        json!({"puzzle_id": "00008"}),
    )
    .await;
    let solution = &response["result"]["structuredContent"];

    assert_eq!(
        solution["solution_san"],
        json!(["Rxe7", "Qb1+", "Nc1", "Qxc1+", "Qxc1"])
    );
    assert_eq!(
        solution["solution_uci"],
        json!(["e6e7", "b2b1", "b3c1", "b1c1", "h6c1"])
    );
    assert_eq!(solution["first_move_san"], "Rxe7");
}

#[tokio::test]
async fn filters_work_the_same_as_over_http() {
    let (_dir, sampler, auth) = harness();
    let response = call(
        &sampler,
        &auth,
        "get_random_puzzle",
        json!({"themes": ["fork"], "exclude_themes": ["endgame"]}),
    )
    .await;

    let puzzles = response["result"]["structuredContent"]["puzzles"]
        .as_array()
        .expect("puzzles");
    assert_eq!(puzzles.len(), 1);
    let themes = puzzles[0]["themes"].as_array().unwrap();
    assert!(themes.iter().any(|theme| theme == "fork"));
    assert!(!themes.iter().any(|theme| theme == "endgame"));
}

#[tokio::test]
async fn an_unknown_theme_tells_the_model_how_to_recover() {
    let (_dir, sampler, auth) = harness();
    let response = call(
        &sampler,
        &auth,
        "get_random_puzzle",
        json!({"themes": ["frok"]}),
    )
    .await;

    let message = response["error"]["message"]
        .as_str()
        .expect("an error message");
    assert!(message.contains("frok"), "say what was wrong");
    assert!(
        message.contains("list_themes"),
        "and which tool fixes it: {message}"
    );
}

#[tokio::test]
async fn an_impossible_filter_is_an_error_not_an_empty_list() {
    let (_dir, sampler, auth) = harness();
    let response = call(
        &sampler,
        &auth,
        "get_random_puzzle",
        json!({"rating": 3900, "tolerance": 50}),
    )
    .await;

    // An empty list would read as "no such thing exists"; the error says what
    // to change instead.
    let message = response["error"]["message"].as_str().expect("message");
    assert!(message.contains("Widen") || message.contains("drop a theme"));
}

#[tokio::test]
async fn a_public_hostname_must_be_allowed_explicitly() {
    let (_dir, sampler, auth) = harness();

    let call = |hosts: Vec<String>, host: &'static str| {
        let sampler = Arc::clone(&sampler);
        let auth = Arc::clone(&auth);
        async move {
            routes::router(sampler, auth, &hosts)
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/mcp")
                        .header("content-type", "application/json")
                        .header("accept", "application/json, text/event-stream")
                        .header("host", host)
                        .body(Body::from(
                            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})
                                .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .expect("request")
                .status()
        }
    };

    // The transport refuses hostnames it does not know: DNS-rebinding
    // protection. A deployment on a real domain has to name itself, and
    // forgetting to returns 403 to every caller — which is how this was found,
    // in production.
    assert_eq!(
        call(vec![], "chess.mauriulloa.com").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            vec!["chess.mauriulloa.com".to_string()],
            "chess.mauriulloa.com"
        )
        .await,
        StatusCode::OK
    );
    // Naming one host must not open the door to any other.
    assert_eq!(
        call(vec!["chess.mauriulloa.com".to_string()], "evil.example.com").await,
        StatusCode::FORBIDDEN
    );
}
