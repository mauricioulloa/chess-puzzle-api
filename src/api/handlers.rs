use crate::api::docs::{ApiDoc, DOCS_HTML};
use crate::api::errors::{ApiError, ApiResult, ErrorBody};
use crate::api::models::*;
use crate::api::pages::{self, SiteFacts};
use crate::api::query::{
    DEFAULT_TOLERANCE, PIECES_CEILING, PIECES_FLOOR, PuzzleFilter, PuzzleRow, RATING_CEILING,
    RATING_FLOOR, Sampler, ThemesMode, band_around,
};
use crate::api::theme_descriptions;
use crate::auth::keys::KeyStore;
use crate::db::{SCHEMA_VERSION, meta_keys};
use crate::usage::{self, FilterSample};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderName, header};
use axum::response::{Html, IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, OpenApi, ToSchema};

pub type SharedState = Arc<Sampler>;

/// The usage endpoint reads `api.db`, which the key store owns, so it carries
/// its own state rather than the puzzle sampler.
pub type UsageState = Arc<KeyStore>;

const MAX_COUNT: usize = 50;
const THEME_DOCS: &str = "https://lichess.org/training/themes";

/// `deny_unknown_fields` turns a typo into a 400 instead of a silently ignored
/// filter, which is the difference between "this theme has no puzzles" and
/// "you spelled the parameter wrong".
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RandomParams {
    rating: Option<i64>,
    tolerance: Option<i64>,
    rating_min: Option<i64>,
    rating_max: Option<i64>,
    #[serde(alias = "theme")]
    themes: Option<String>,
    themes_mode: Option<String>,
    #[serde(alias = "excludeTheme")]
    exclude_themes: Option<String>,
    opening: Option<String>,
    /// Only positions with at most this many pieces on the board, kings
    /// included.
    max_pieces: Option<u32>,
    count: Option<usize>,
    /// Include the position drawn as text. Off by default because it is by
    /// far the largest field in the response.
    board: Option<bool>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PuzzleParams {
    /// Include the position drawn as text.
    board: Option<bool>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleBatch {
    count: usize,
    puzzles: Vec<PuzzleResponse>,
}

/// SQLite queries are blocking CPU work. Running them on a Tokio worker
/// would stall every other request sharing that thread, so they go to the
/// blocking pool.
async fn blocking<T, F>(task: F) -> ApiResult<T>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(task).await {
        Ok(result) => result.map_err(ApiError::from),
        Err(err) => Err(ApiError::Internal(anyhow::anyhow!(err))),
    }
}

fn bad_request(message: impl Into<String>, hint: Option<String>) -> ApiError {
    ApiError::BadRequest {
        message: message.into(),
        hint,
    }
}

fn resolve_themes(sampler: &Sampler, raw: Option<&str>, field: &str) -> ApiResult<Vec<i64>> {
    let names = raw
        .unwrap_or_default()
        .split([',', ' '])
        .map(str::trim)
        .filter(|name| !name.is_empty());
    sampler.catalog.resolve(names).map_err(|unknown| {
        bad_request(
            format!("Unknown {field}: {}", unknown.join(", ")),
            Some(format!(
                "GET /v1/themes lists all {} valid themes.",
                sampler.catalog.len()
            )),
        )
    })
}

async fn find_puzzle(sampler: &SharedState, id: String) -> ApiResult<PuzzleRow> {
    let sampler = Arc::clone(sampler);
    let lookup_id = id.clone();
    blocking(move || sampler.by_puzzle_id(&lookup_id))
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("No puzzle with id `{id}`.")))
}

fn build_filter(sampler: &Sampler, params: &RandomParams) -> ApiResult<PuzzleFilter> {
    if params.rating.is_some() && (params.rating_min.is_some() || params.rating_max.is_some()) {
        return Err(bad_request(
            "Use either `rating` (with optional `tolerance`) or `ratingMin`/`ratingMax`, not both.",
            None,
        ));
    }
    if params.tolerance.is_some() && params.rating.is_none() {
        return Err(bad_request(
            "`tolerance` only means something alongside `rating`.",
            None,
        ));
    }

    // Validate what the caller actually typed, before any arithmetic. Deriving
    // a band first and complaining about the derived bound produces errors
    // like "rating=9999 is invalid; got 9899", which helps nobody.
    for (name, value) in [
        ("rating", params.rating),
        ("ratingMin", params.rating_min),
        ("ratingMax", params.rating_max),
    ] {
        if let Some(value) = value
            && !(RATING_FLOOR..=RATING_CEILING).contains(&value)
        {
            return Err(bad_request(
                format!(
                    "`{name}` must be between {RATING_FLOOR} and {RATING_CEILING}; got {value}."
                ),
                None,
            ));
        }
    }

    let (rating_min, rating_max) = match params.rating {
        Some(rating) => {
            let tolerance = params.tolerance.unwrap_or(DEFAULT_TOLERANCE);
            if tolerance < 0 {
                return Err(bad_request("`tolerance` cannot be negative.", None));
            }
            let (min, max) = band_around(rating, tolerance);
            (Some(min), Some(max))
        }
        None => (params.rating_min, params.rating_max),
    };

    if let (Some(min), Some(max)) = (rating_min, rating_max)
        && min > max
    {
        return Err(bad_request(
            format!("Empty rating range: {min} is above {max}."),
            None,
        ));
    }

    let include = resolve_themes(sampler, params.themes.as_deref(), "themes")?;
    let exclude = resolve_themes(sampler, params.exclude_themes.as_deref(), "excludeThemes")?;

    if let Some(&conflict) = include.iter().find(|id| exclude.contains(id)) {
        let name = sampler.catalog.name_of(conflict).unwrap_or("that theme");
        return Err(bad_request(
            format!("`{name}` is both required and excluded."),
            None,
        ));
    }

    let mode = match params.themes_mode.as_deref() {
        None => ThemesMode::All,
        Some(value) => ThemesMode::parse(value).ok_or_else(|| {
            bad_request(
                format!("`themesMode` must be `all` or `any`; got `{value}`."),
                None,
            )
        })?,
    };

    let opening_ids = match &params.opening {
        Some(name) => {
            let ids = sampler.resolve_openings(name)?;
            if ids.is_empty() {
                return Err(bad_request(
                    format!("No opening matches `{name}`."),
                    Some("Opening tags look like `Sicilian_Defense`.".to_string()),
                ));
            }
            ids
        }
        None => Vec::new(),
    };

    if let Some(max) = params.max_pieces
        && !(PIECES_FLOOR..=PIECES_CEILING).contains(&max)
    {
        return Err(bad_request(
            format!("`maxPieces` must be between {PIECES_FLOOR} and {PIECES_CEILING}; got {max}."),
            None,
        ));
    }

    Ok(PuzzleFilter {
        rating_min,
        rating_max,
        include,
        exclude,
        mode,
        opening_ids,
        max_pieces: params.max_pieces,
    })
}

/// Describes what a caller asked for, in canonical theme names so the
/// counters are not split across spellings. Never records anything about
/// *who* asked.
fn filter_sample(sampler: &Sampler, params: &RandomParams, filter: &PuzzleFilter) -> FilterSample {
    let names = |ids: &[i64]| -> Vec<String> {
        ids.iter()
            .filter_map(|&id| sampler.catalog.name_of(id))
            .map(str::to_string)
            .collect()
    };

    let mut options = Vec::new();
    if params.count.is_some() {
        options.push("batch");
    }
    if params.board.unwrap_or(false) {
        options.push("board");
    }
    if filter.mode == ThemesMode::Any {
        options.push("themesModeAny");
    }
    if !filter.opening_ids.is_empty() {
        options.push("opening");
    }
    if filter.max_pieces.is_some() {
        options.push("maxPieces");
    }
    if filter.include.is_empty() && filter.exclude.is_empty() && filter.rating_min.is_none() {
        options.push("unfiltered");
    }

    FilterSample {
        themes: names(&filter.include),
        excluded_themes: names(&filter.exclude),
        rating_band: filter
            .rating_min
            .zip(filter.rating_max)
            .map(|(min, max)| usage::rating_band((min + max) / 2)),
        options,
    }
}

fn describe(filter: &PuzzleFilter) -> String {
    let mut parts = Vec::new();
    if let (Some(min), Some(max)) = (filter.rating_min, filter.rating_max) {
        parts.push(format!("rating {min}-{max}"));
    }
    if !filter.include.is_empty() {
        parts.push(format!("{} theme(s)", filter.include.len()));
    }
    if let Some(max) = filter.max_pieces {
        parts.push(format!("at most {max} pieces"));
    }
    if parts.is_empty() {
        "those filters".to_string()
    } else {
        parts.join(" and ")
    }
}

#[utoipa::path(
    get,
    path = "/v1/puzzles/random",
    tag = "puzzles",
    params(RandomParams),
    responses(
        (status = 200, description = "A random puzzle. With `count`, a batch envelope instead.", body = PuzzleResponse),
        (status = 404, description = "The filters describe an empty slice of the dataset", body = ErrorBody),
        (status = 400, description = "A parameter was invalid or unrecognised", body = ErrorBody),
        (status = 401, description = "The API key is unknown or revoked", body = ErrorBody),
        (status = 429, description = "Rate limit exceeded", body = ErrorBody),
        (status = 503, description = "An exhaustive search ran past its time limit and was stopped", body = ErrorBody),
    )
)]
pub async fn random(
    State(sampler): State<SharedState>,
    Query(params): Query<RandomParams>,
) -> ApiResult<Response> {
    let batched = params.count.is_some();
    let count = params.count.unwrap_or(1);
    if count == 0 {
        return Err(bad_request("`count` must be at least 1.", None));
    }
    if count > MAX_COUNT {
        return Err(bad_request(
            format!("`count` is capped at {MAX_COUNT}; got {count}."),
            None,
        ));
    }

    let filter = build_filter(&sampler, &params)?;
    let rows = {
        let sampler = Arc::clone(&sampler);
        let filter = filter.clone();
        blocking(move || sampler.random(&filter, count)).await?
    };

    // A search that finds nothing still says what someone wanted, and that is
    // the most useful signal the counters carry: it shows where demand runs
    // past what the dataset holds. So the sample is attached to the 404 too,
    // which means building that response here rather than returning Err.
    let sample = filter_sample(&sampler, &params, &filter);

    if rows.is_empty() {
        let mut response =
            ApiError::NoMatch(format!("No puzzle matches {}.", describe(&filter))).into_response();
        response.extensions_mut().insert(sample);
        return Ok(response);
    }

    let puzzles: Vec<PuzzleResponse> = rows
        .iter()
        .map(|row| PuzzleResponse::new(row, &sampler.catalog, params.board.unwrap_or(false)))
        .collect();

    // A bare object when one puzzle was asked for, a batch envelope when
    // `count` was given, so each spelling has one stable shape.
    let mut response = if batched {
        Json(PuzzleBatch {
            count: puzzles.len(),
            puzzles,
        })
        .into_response()
    } else {
        Json(puzzles.into_iter().next().expect("non-empty")).into_response()
    };

    response.extensions_mut().insert(sample);
    Ok(response)
}

#[utoipa::path(
    get,
    path = "/v1/puzzles/{id}",
    tag = "puzzles",
    params(("id" = String, Path, description = "Lichess puzzle id, e.g. `00008`"), PuzzleParams),
    responses(
        (status = 200, description = "The puzzle, without its solution", body = PuzzleResponse),
        (status = 404, description = "No puzzle carries that id", body = ErrorBody),
        (status = 400, description = "A parameter was invalid or unrecognised", body = ErrorBody),
        (status = 401, description = "The API key is unknown or revoked", body = ErrorBody),
        (status = 429, description = "Rate limit exceeded", body = ErrorBody),
    )
)]
pub async fn by_id(
    State(sampler): State<SharedState>,
    Path(id): Path<String>,
    Query(params): Query<PuzzleParams>,
) -> ApiResult<Json<PuzzleResponse>> {
    let row = find_puzzle(&sampler, id).await?;
    Ok(Json(PuzzleResponse::new(
        &row,
        &sampler.catalog,
        params.board.unwrap_or(false),
    )))
}

#[utoipa::path(
    get,
    path = "/v1/puzzles/{id}/solution",
    tag = "puzzles",
    params(("id" = String, Path, description = "Lichess puzzle id, e.g. `00008`")),
    responses(
        (status = 200, description = "The moves that solve the puzzle", body = SolutionResponse),
        (status = 404, description = "No puzzle carries that id", body = ErrorBody),
        (status = 400, description = "A parameter was invalid or unrecognised", body = ErrorBody),
        (status = 401, description = "The API key is unknown or revoked", body = ErrorBody),
        (status = 429, description = "Rate limit exceeded", body = ErrorBody),
    )
)]
pub async fn solution(
    State(sampler): State<SharedState>,
    Path(id): Path<String>,
) -> ApiResult<Json<SolutionResponse>> {
    let row = find_puzzle(&sampler, id).await?;
    Ok(Json(SolutionResponse::new(&row)))
}

#[utoipa::path(
    get,
    path = "/v1/themes",
    tag = "reference",
    responses(
        (status = 200, description = "Every theme, with how many puzzles carry it and what it means", body = ThemesResponse),
        (status = 401, description = "The API key is unknown or revoked", body = ErrorBody),
        (status = 429, description = "Rate limit exceeded", body = ErrorBody),
    )
)]
pub async fn themes(State(sampler): State<SharedState>) -> Json<ThemesResponse> {
    let themes: Vec<ThemeResponse> = sampler
        .catalog
        .all()
        .iter()
        .map(|theme| ThemeResponse {
            name: theme.name.clone(),
            puzzle_count: theme.puzzle_count,
            description: theme_descriptions::description(&theme.name),
        })
        .collect();

    Json(ThemesResponse {
        count: themes.len(),
        themes,
        documentation: THEME_DOCS,
    })
}

#[utoipa::path(
    get,
    path = "/v1/stats",
    tag = "reference",
    responses(
        (status = 200, description = "Dataset size, rating distribution and provenance", body = StatsResponse),
        (status = 401, description = "The API key is unknown or revoked", body = ErrorBody),
        (status = 429, description = "Rate limit exceeded", body = ErrorBody),
    )
)]
pub async fn stats(State(sampler): State<SharedState>) -> ApiResult<Json<StatsResponse>> {
    let stats = {
        let sampler = Arc::clone(&sampler);
        blocking(move || sampler.stats()).await?
    };
    let meta = |key: &str| stats.meta.get(key).cloned();
    let number = |key: &str| meta(key).and_then(|value| value.parse().ok());

    Ok(Json(StatsResponse {
        puzzles: stats.puzzles,
        themes: sampler.catalog.len(),
        openings: stats.openings,
        rating_min: stats.rating_min,
        rating_max: stats.rating_max,
        rating_distribution: stats
            .bands
            .iter()
            .map(|(from, to, count)| RatingBand {
                from: *from,
                to: *to,
                count: *count,
            })
            .collect(),
        source: Source {
            name: "Lichess puzzle database",
            url: "https://database.lichess.org/#puzzles",
            license: "CC0 1.0",
            imported_at: meta(meta_keys::IMPORTED_AT),
            source_rows: number(meta_keys::ROWS_READ),
            min_popularity: number(meta_keys::MIN_POPULARITY),
            min_plays: number(meta_keys::MIN_PLAYS),
        },
    }))
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "reference",
    responses((status = 200, description = "The service is up and the dataset is loaded", body = HealthResponse))
)]
pub async fn health(State(site): State<SiteState>) -> Json<HealthResponse> {
    let sampler = &site.sampler;
    let schema_version = sampler.catalog.schema_version;
    Json(HealthResponse {
        status: if schema_version == Some(SCHEMA_VERSION) {
            "ok"
        } else {
            "schema_mismatch"
        },
        puzzles: sampler.total_puzzles(),
        schema_version,
        expected_schema_version: SCHEMA_VERSION,
    })
}

/// The generated OpenAPI 3.1 description.
pub async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// A rendered reference, for humans.
pub async fn docs() -> Html<&'static str> {
    Html(DOCS_HTML)
}

/// Reconstructs the URL callers reached us on, so the landing page and
/// llms.txt quote working examples instead of a hard-coded hostname that is
/// wrong everywhere except one deployment.
fn base_url(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost:8080");
    let is_local = host.starts_with("localhost") || host.starts_with("127.0.0.1");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or(if is_local { "http" } else { "https" });
    format!("{scheme}://{host}")
}

/// What the public pages need: the dataset, and the configured limit they
/// quote so the pages cannot disagree with the service.
pub struct Site {
    pub sampler: SharedState,
    pub anonymous_limit: u32,
}

pub type SiteState = Arc<Site>;

fn site_facts(site: &Site, headers: &HeaderMap) -> SiteFacts {
    SiteFacts {
        puzzles: site.sampler.total_puzzles(),
        themes: site.sampler.catalog.len(),
        base_url: base_url(headers),
        anonymous_limit: site.anonymous_limit,
        max_count: MAX_COUNT,
    }
}

/// What a person sees when they paste the bare domain into a browser.
pub async fn landing(State(site): State<SiteState>, headers: HeaderMap) -> Html<String> {
    Html(pages::landing(&site_facts(&site, &headers)))
}

/// The llms.txt convention: what this API is, in plain text.
pub async fn llms_txt(
    State(site): State<SiteState>,
    headers: HeaderMap,
) -> ([(HeaderName, &'static str); 1], String) {
    (
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        pages::llms_txt(&site_facts(&site, &headers)),
    )
}

#[utoipa::path(
    get,
    path = "/v1/usage",
    tag = "reference",
    responses(
        (status = 200, description = "Aggregate usage over the last 30 days", body = UsageResponse),
        (status = 401, description = "The API key is unknown or revoked", body = ErrorBody),
        (status = 429, description = "Rate limit exceeded", body = ErrorBody),
    )
)]
pub async fn usage(State(store): State<UsageState>) -> ApiResult<Json<UsageResponse>> {
    let report = blocking(move || store.read(usage::report)).await?;
    Ok(Json(report))
}
