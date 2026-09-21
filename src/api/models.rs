//! Wire formats. Field names are camelCase because the primary consumer of a
//! puzzle API is a browser.

use crate::api::catalog::Catalog;
use crate::api::query::PuzzleRow;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleResponse {
    pub id: String,
    /// The position as Lichess stores it: *before* the opponent's move.
    pub fen: String,
    /// The opponent's move, in UCI. Apply it to `fen` to reach the position
    /// the player is asked to solve.
    pub initial_move: String,
    /// The colour the solver plays, already accounting for `initialMove`.
    pub solver_color: &'static str,
    pub rating: i64,
    pub rating_deviation: i64,
    pub popularity: i64,
    pub nb_plays: i64,
    pub themes: Vec<String>,
    pub opening_tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_url: Option<String>,
}

impl PuzzleResponse {
    pub fn new(row: &PuzzleRow, catalog: &Catalog) -> Self {
        Self {
            id: row.puzzle_id.clone(),
            fen: row.fen.clone(),
            initial_move: row.initial_move().to_string(),
            solver_color: row.solver_color(),
            rating: row.rating,
            rating_deviation: row.rating_deviation,
            popularity: row.popularity,
            nb_plays: row.nb_plays,
            themes: catalog
                .names_for(row.mask)
                .into_iter()
                .map(str::to_string)
                .collect(),
            opening_tags: row
                .opening_tags
                .as_deref()
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_string)
                .collect(),
            game_url: row.game.as_ref().map(|game| game.to_url()),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SolutionResponse {
    pub id: String,
    pub initial_move: String,
    /// The answer, in UCI, starting after `initialMove`.
    pub solution: Vec<String>,
}

impl SolutionResponse {
    pub fn new(row: &PuzzleRow) -> Self {
        Self {
            id: row.puzzle_id.clone(),
            initial_move: row.initial_move().to_string(),
            solution: row.solution().into_iter().map(str::to_string).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeResponse {
    pub name: String,
    pub puzzle_count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemesResponse {
    pub count: usize,
    pub themes: Vec<ThemeResponse>,
    /// Lichess documents what each theme means; duplicating their wording here
    /// would mean copying strings from a differently licensed project.
    pub documentation: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RatingBand {
    pub from: i64,
    pub to: i64,
    pub count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsResponse {
    pub puzzles: i64,
    pub themes: usize,
    pub openings: i64,
    pub rating_min: i64,
    pub rating_max: i64,
    pub rating_distribution: Vec<RatingBand>,
    pub source: Source,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub name: &'static str,
    pub url: &'static str,
    pub license: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_rows: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_popularity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_plays: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: &'static str,
    pub puzzles: i64,
}
