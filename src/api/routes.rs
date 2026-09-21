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

pub fn router(state: SharedState, auth: Arc<AuthState>) -> Router {
    let mcp_state = Arc::clone(&state);
    // Read-only and public, so any origin may call it from a browser.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET]);

    // Static segments take priority over `{id}`, so `/random` is not swallowed
    // by the lookup route. `random_is_not_swallowed_by_the_id_route` pins that.
    let api = Router::new()
        .route("/v1/puzzles/random", get(handlers::random))
        .route("/v1/puzzles/{id}", get(handlers::by_id))
        .route("/v1/puzzles/{id}/solution", get(handlers::solution))
        .route("/v1/themes", get(handlers::themes))
        .route("/v1/stats", get(handlers::stats))
        .route_layer(axum::middleware::from_fn_with_state(
            Arc::clone(&auth),
            middleware::enforce,
        ))
        .with_state(Arc::clone(&state));

    // The usage report reads api.db rather than the puzzle database, so it
    // carries its own state. It stays under the limiter: public does not mean
    // free to hammer.
    let usage = Router::new()
        .route("/v1/usage", get(handlers::usage))
        .route_layer(axum::middleware::from_fn_with_state(
            Arc::clone(&auth),
            middleware::enforce,
        ))
        .with_state(Arc::clone(&auth.store));

    // Health checks are what a host polls to decide whether to keep the
    // process alive. Rate limiting them would make a busy minute look like an
    // outage, so they sit outside the layer.
    // Documentation and health are what a newcomer and a host reach for
    // first; neither should ever be met with a 429.
    let public = Router::new()
        .route("/", get(handlers::landing))
        .route("/llms.txt", get(handlers::llms_txt))
        .route("/health", get(handlers::health))
        .route("/openapi.json", get(handlers::openapi))
        .route("/docs", get(handlers::docs))
        .with_state(state);

    // The MCP endpoint is a tower service rather than a handler, so it is
    // nested rather than routed. Stateless: no sessions to keep, no SSE to
    // hold open, which suits a read-only tool server and keeps a restart from
    // dropping anyone's connection.
    let mut mcp_config = StreamableHttpServerConfig::default();
    mcp_config.legacy_session_mode = false;
    mcp_config.json_response = true;

    let mcp = StreamableHttpService::new(
        move || Ok(PuzzleTools::new(Arc::clone(&mcp_state))),
        Arc::new(NeverSessionManager::default()),
        mcp_config,
    );

    let mcp_route = Router::new()
        .nest_service("/mcp", mcp)
        // An agent gets the same budget as anyone else.
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&auth),
            middleware::enforce,
        ));

    api.merge(usage)
        .merge(mcp_route)
        .merge(public)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(cors)
}
