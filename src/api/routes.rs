use crate::api::handlers::{self, SharedState};
use crate::auth::middleware::{self, AuthState};
use axum::Router;
use axum::http::Method;
use axum::routing::get;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub fn router(state: SharedState, auth: Arc<AuthState>) -> Router {
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

    api.merge(usage)
        .merge(public)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(cors)
}
