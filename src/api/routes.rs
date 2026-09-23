use crate::api::handlers::{self, SharedState};
use crate::auth::middleware::{self, AuthState};
use crate::mcp::PuzzleTools;
use axum::Router;
use axum::http::Method;
use axum::routing::get;
use rmcp::transport::streamable_http_server::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::tower::StreamableHttpServerConfig;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

/// `mcp_allowed_hosts` are the public hostnames `/mcp` answers to, beyond the
/// loopback names rmcp allows out of the box (see `ServeArgs`).
pub fn router(sampler: SharedState, auth: Arc<AuthState>, mcp_allowed_hosts: &[String]) -> Router {
    // Static segments take priority over `{id}`, so `/random` is not swallowed
    // by the lookup route. `random_is_not_swallowed_by_the_id_route` pins that.
    let api = Router::new()
        .route("/v1/puzzles/random", get(handlers::random))
        .route("/v1/puzzles/{id}", get(handlers::by_id))
        .route("/v1/puzzles/{id}/solution", get(handlers::solution))
        .route("/v1/themes", get(handlers::themes))
        .route("/v1/stats", get(handlers::stats))
        .with_state(Arc::clone(&sampler));

    // The usage report reads api.db rather than the puzzle database, so it
    // carries its own state.
    let usage = Router::new()
        .route("/v1/usage", get(handlers::usage))
        .with_state(Arc::clone(&auth.store));

    // Stateless: no sessions to keep, no SSE to hold open, which suits a
    // read-only tool server and keeps a restart from dropping anyone.
    let mut mcp_config = StreamableHttpServerConfig::default();
    mcp_config.legacy_session_mode = false;
    mcp_config.json_response = true;
    mcp_config
        .allowed_hosts
        .extend(mcp_allowed_hosts.iter().cloned());
    let mcp_sampler = Arc::clone(&sampler);
    let mcp = StreamableHttpService::new(
        move || Ok(PuzzleTools::new(Arc::clone(&mcp_sampler))),
        Arc::new(NeverSessionManager::default()),
        mcp_config,
    );

    // Everything that serves data shares one budget, agents included.
    let limited = api.merge(usage).nest_service("/mcp", mcp).route_layer(
        axum::middleware::from_fn_with_state(Arc::clone(&auth), middleware::enforce),
    );

    // Documentation and health are what a newcomer and a host reach for
    // first; neither should ever be met with a 429.
    let public = Router::new()
        .route("/", get(handlers::landing))
        .route("/llms.txt", get(handlers::llms_txt))
        .route("/health", get(handlers::health))
        .route("/openapi.json", get(handlers::openapi))
        .route("/docs", get(handlers::docs))
        .with_state(sampler);

    // Read-only and public, so any origin may call it from a browser.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET]);

    limited
        .merge(public)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(cors)
}
