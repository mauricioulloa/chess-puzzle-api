//! A Model Context Protocol server, so an agent can call this API as tools
//! rather than being told to construct HTTP requests from documentation.
//!
//! The shapes here are deliberately not the HTTP response types. What a model
//! wants differs from what a program wants: the drawn board is always included
//! rather than opt-in, SAN leads, and every description states the one thing
//! that will otherwise be got wrong — that `fen` is the position *before* the
//! opponent's move.

use crate::api::catalog::Catalog;
use crate::api::query::{
    DEFAULT_TOLERANCE, PuzzleFilter, PuzzleRow, Sampler, ThemesMode, band_around,
};
use crate::chess;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ErrorData, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const MAX_COUNT: usize = 20;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RandomArgs {
    /// Target Elo rating. Puzzles are returned within `tolerance` of it.
    /// Roughly: 800 is a beginner, 1500 a club player, 2200+ is expert.
    pub rating: Option<i64>,
    /// How far either side of `rating` to search. Defaults to 100.
    pub tolerance: Option<i64>,
    /// Tactical themes the puzzle must carry, e.g. `fork`, `pin`, `mateIn2`.
    /// Call `list_themes` first; an unknown name is an error, not an empty
    /// result.
    pub themes: Option<Vec<String>>,
    /// `all` (default) requires every theme; `any` requires at least one.
    pub themes_mode: Option<String>,
    /// Themes the puzzle must not carry.
    pub exclude_themes: Option<Vec<String>>,
    /// How many puzzles to return, up to 20. Defaults to 1.
    pub count: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PuzzleIdArgs {
    /// The puzzle's Lichess id, as returned by `get_random_puzzle`.
    pub puzzle_id: String,
}

/// A puzzle as a model should see it: the position to solve, drawn, with the
/// answer withheld.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct McpPuzzle {
    pub puzzle_id: String,
    /// The position the player must solve, in FEN. The opponent's move has
    /// already been applied; this is the board to reason about.
    pub position_fen: String,
    /// The same position drawn. Uppercase is White, lowercase is Black.
    pub board: String,
    /// Which side the player moves.
    pub side_to_move: String,
    /// The opponent's move that created this position, in SAN.
    pub opponent_last_move: String,
    /// Elo difficulty.
    pub rating: i64,
    /// Tactical motifs present. These are hints; do not show them to a solver
    /// unless they ask.
    pub themes: Vec<String>,
    /// Opens the position on a real board, for a person.
    pub analysis_url: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct McpPuzzles {
    pub count: usize,
    pub puzzles: Vec<McpPuzzle>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct McpSolution {
    pub puzzle_id: String,
    /// The winning line in SAN, starting from `position_fen`. The player's
    /// moves are at even indices; the odd ones are the opponent's replies.
    pub solution_san: Vec<String>,
    /// The same moves in UCI, aligned index for index.
    pub solution_uci: Vec<String>,
    /// The player's first move — the answer, if only one move is wanted.
    pub first_move_san: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct McpTheme {
    pub name: String,
    pub puzzle_count: i64,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct McpThemes {
    pub count: usize,
    pub themes: Vec<McpTheme>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct McpStats {
    pub puzzles: i64,
    pub themes: usize,
    pub rating_min: i64,
    pub rating_max: i64,
    pub source: String,
    pub license: String,
}

fn invalid(message: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(message.into(), None)
}

fn internal(error: anyhow::Error) -> ErrorData {
    tracing::error!("mcp tool failed: {error:#}");
    ErrorData::internal_error("The puzzle service failed to answer.", None)
}

/// Runs a blocking SQLite query off the async workers.
async fn blocking<T: Send + 'static>(
    task: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, ErrorData> {
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|err| internal(anyhow::anyhow!(err)))?
        .map_err(internal)
}

fn to_mcp(row: &PuzzleRow, catalog: &Catalog) -> Result<McpPuzzle, ErrorData> {
    let annotated = chess::annotate(&row.fen, &row.moves)
        .map_err(|err| internal(err.context("annotating a puzzle")))?;

    Ok(McpPuzzle {
        puzzle_id: row.puzzle_id.clone(),
        position_fen: annotated.position_fen.clone(),
        board: annotated.board_ascii,
        side_to_move: chess::color_name(annotated.solver_color).to_string(),
        opponent_last_move: annotated.initial_move_san,
        rating: row.rating,
        themes: catalog
            .names_for(row.mask)
            .into_iter()
            .map(str::to_string)
            .collect(),
        analysis_url: chess::analysis_url(&annotated.position_fen),
    })
}

#[derive(Clone)]
pub struct PuzzleTools {
    sampler: Arc<Sampler>,
}

#[tool_router]
impl PuzzleTools {
    pub fn new(sampler: Arc<Sampler>) -> Self {
        Self { sampler }
    }

    fn resolve(&self, names: Option<&[String]>, field: &str) -> Result<Vec<i64>, ErrorData> {
        let names = names.unwrap_or_default().iter().map(String::as_str);
        self.sampler.catalog.resolve(names).map_err(|unknown| {
            invalid(format!(
                "Unknown {field}: {}. Call list_themes for the valid names.",
                unknown.join(", ")
            ))
        })
    }

    async fn lookup(&self, puzzle_id: &str) -> Result<PuzzleRow, ErrorData> {
        let sampler = Arc::clone(&self.sampler);
        let id = puzzle_id.to_string();
        blocking(move || sampler.by_puzzle_id(&id))
            .await?
            .ok_or_else(|| invalid(format!("No puzzle with id {puzzle_id}")))
    }

    #[tool(
        name = "get_random_puzzle",
        description = "Fetch one or more random chess puzzles, filtered by difficulty and \
                       tactical theme. Returns the position to solve as a FEN and as a drawn \
                       board, WITHOUT the answer — call get_solution with the puzzle_id for \
                       that. Use this when asked for a chess puzzle, a tactic to solve, or \
                       training material at a given level."
    )]
    async fn get_random_puzzle(
        &self,
        Parameters(args): Parameters<RandomArgs>,
    ) -> Result<Json<McpPuzzles>, ErrorData> {
        let count = args.count.unwrap_or(1);
        if count == 0 || count > MAX_COUNT {
            return Err(invalid(format!("count must be between 1 and {MAX_COUNT}")));
        }

        let tolerance = args.tolerance.unwrap_or(DEFAULT_TOLERANCE).max(0);
        let band = args.rating.map(|rating| band_around(rating, tolerance));

        let mode = match args.themes_mode.as_deref() {
            None => ThemesMode::All,
            Some(value) => ThemesMode::parse(value)
                .ok_or_else(|| invalid(format!("themes_mode must be all or any, got {value}")))?,
        };

        let filter = PuzzleFilter {
            rating_min: band.map(|(min, _)| min),
            rating_max: band.map(|(_, max)| max),
            include: self.resolve(args.themes.as_deref(), "themes")?,
            exclude: self.resolve(args.exclude_themes.as_deref(), "exclude_themes")?,
            mode,
            opening_ids: Vec::new(),
        };

        let sampler = Arc::clone(&self.sampler);
        let rows = blocking(move || sampler.random(&filter, count)).await?;

        if rows.is_empty() {
            return Err(invalid(
                "No puzzle matches those filters. Widen the rating range or drop a theme.",
            ));
        }

        let puzzles = rows
            .iter()
            .map(|row| to_mcp(row, &self.sampler.catalog))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Json(McpPuzzles {
            count: puzzles.len(),
            puzzles,
        }))
    }

    #[tool(
        name = "get_puzzle",
        description = "Fetch a specific chess puzzle by its id, without the solution. Use this \
                       to revisit a puzzle whose id you already have."
    )]
    async fn get_puzzle(
        &self,
        Parameters(args): Parameters<PuzzleIdArgs>,
    ) -> Result<Json<McpPuzzle>, ErrorData> {
        let row = self.lookup(&args.puzzle_id).await?;
        Ok(Json(to_mcp(&row, &self.sampler.catalog)?))
    }

    #[tool(
        name = "get_solution",
        description = "Reveal the solution to a puzzle, in SAN and UCI. Call this only when the \
                       answer is actually wanted — after someone has attempted the puzzle, or \
                       to check their move. Fetching a puzzle does not reveal its solution, \
                       which is what lets you pose one fairly."
    )]
    async fn get_solution(
        &self,
        Parameters(args): Parameters<PuzzleIdArgs>,
    ) -> Result<Json<McpSolution>, ErrorData> {
        let row = self.lookup(&args.puzzle_id).await?;
        let annotated = chess::annotate(&row.fen, &row.moves).map_err(internal)?;

        Ok(Json(McpSolution {
            puzzle_id: row.puzzle_id.clone(),
            first_move_san: annotated.solution_san.first().cloned().unwrap_or_default(),
            solution_san: annotated.solution_san,
            solution_uci: row.solution().into_iter().map(str::to_string).collect(),
        }))
    }

    #[tool(
        name = "list_themes",
        description = "List every tactical theme that can be used to filter puzzles, with how \
                       many puzzles carry each. Call this before guessing a theme name: names \
                       are exact, and an unknown one is an error rather than an empty result."
    )]
    async fn list_themes(&self) -> Result<Json<McpThemes>, ErrorData> {
        let themes: Vec<McpTheme> = self
            .sampler
            .catalog
            .all()
            .iter()
            .map(|theme| McpTheme {
                name: theme.name.clone(),
                puzzle_count: theme.puzzle_count,
            })
            .collect();

        Ok(Json(McpThemes {
            count: themes.len(),
            themes,
        }))
    }

    #[tool(
        name = "get_dataset_stats",
        description = "Describe the puzzle collection: how many puzzles, the rating range and \
                       where the data comes from."
    )]
    async fn get_dataset_stats(&self) -> Result<Json<McpStats>, ErrorData> {
        let sampler = Arc::clone(&self.sampler);
        let stats = blocking(move || sampler.stats()).await?;

        Ok(Json(McpStats {
            puzzles: stats.puzzles,
            themes: self.sampler.catalog.len(),
            rating_min: stats.rating_min,
            rating_max: stats.rating_max,
            source: "Lichess puzzle database".to_string(),
            license: "CC0 1.0 (public domain)".to_string(),
        }))
    }
}

#[tool_handler]
impl ServerHandler for PuzzleTools {
    fn get_info(&self) -> ServerConfig {
        // ServerConfig and Implementation are non-exhaustive, so they are
        // built from Default rather than with a struct literal.
        let mut server_info = Implementation::default();
        server_info.name = "chess-puzzle-api".to_string();
        server_info.title = Some("Chess Puzzles".to_string());
        server_info.version = env!("CARGO_PKG_VERSION").to_string();
        server_info.description = Some(
            "Random chess puzzles by rating and tactical theme, from the Lichess \
             puzzle database (CC0). Built by Mauri Ulloa."
                .to_string(),
        );
        server_info.website_url = Some("https://mauriulloa.com".to_string());

        let mut info = ServerConfig::default();
        info.server_info = server_info;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "Serves chess puzzles from the Lichess database (CC0). \
             Puzzles come without their solution: get_random_puzzle gives the position, \
             get_solution gives the answer. That separation is the point — it lets you pose \
             a puzzle to someone without revealing what they are meant to find. \
             \
             position_fen is already the board to solve; the opponent's move has been \
             applied. Prefer SAN (Qxe6+) over UCI (d1e6) when explaining a move to a person. \
             Theme names are exact — call list_themes rather than guessing."
                .to_string(),
        );
        info
    }
}
