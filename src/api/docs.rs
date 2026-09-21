//! The OpenAPI description, generated from the handler annotations so it
//! cannot drift from the code it documents.

use crate::api::errors::ErrorBody;
use crate::api::handlers;
use crate::api::models::*;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "chess-puzzle-api",
        version = env!("CARGO_PKG_VERSION"),
        description = "Random chess puzzles by rating and theme, from the Lichess puzzle \
                       database (CC0). The `fen` of a puzzle is the position *before* the \
                       opponent's move: apply `initialMove` to reach the position to solve. \
                       Solutions are served only by the solution endpoint.",
        license(name = "MIT", url = "https://opensource.org/licenses/MIT")
    ),
    paths(
        handlers::random,
        handlers::by_id,
        handlers::solution,
        handlers::themes,
        handlers::stats,
        handlers::health,
        handlers::usage,
    ),
    components(schemas(
        PuzzleResponse,
        SolutionResponse,
        ThemeResponse,
        ThemesResponse,
        RatingBand,
        StatsResponse,
        Source,
        HealthResponse,
        UsageResponse,
        UsageTotals,
        DailyTally,
        Tally,
        PopularFilters,
        ErrorBody,
    )),
    tags(
        (name = "puzzles", description = "Fetching puzzles and their solutions"),
        (name = "reference", description = "Theme catalogue, dataset statistics and health"),
    )
)]
pub struct ApiDoc;

/// A documentation page that pulls its renderer from a CDN.
///
/// The alternative, vendoring Swagger UI into the binary, adds a download to
/// every build — including the Docker build — to ship a page most callers
/// never open. The spec itself is served locally and is what tooling consumes.
pub const DOCS_HTML: &str = r#"<!doctype html>
<html>
  <head>
    <title>chess-puzzle-api</title>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
  </head>
  <body>
    <script id="api-reference" data-url="/openapi.json"></script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
  </body>
</html>
"#;

/// The spec as pretty-printed JSON, exactly as `openapi.json` holds it.
pub fn spec_json() -> anyhow::Result<String> {
    use utoipa::OpenApi;
    Ok(serde_json::to_string_pretty(&ApiDoc::openapi())?)
}
