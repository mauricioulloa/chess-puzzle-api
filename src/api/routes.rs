use crate::api::handlers::{self, SharedState};
use axum::Router;
use axum::http::Method;
use axum::routing::get;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub fn router(state: SharedState) -> Router {
    // Read-only and public, so any origin may call it from a browser.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET]);

    Router::new()
        // Static segments take priority over `{id}`, so `/random` is not
        // swallowed by the lookup route. `route_conflicts` pins that down.
        .route("/v1/puzzles/random", get(handlers::random))
        .route("/v1/puzzles/{id}", get(handlers::by_id))
        .route("/v1/puzzles/{id}/solution", get(handlers::solution))
        .route("/v1/themes", get(handlers::themes))
        .route("/v1/stats", get(handlers::stats))
        .route("/health", get(handlers::health))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(cors)
        .with_state(state)
}
