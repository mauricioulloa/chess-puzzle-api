use crate::chess;
use crate::db;
use crate::import::parse::{self, MAX_THEMES, ThemeMask};
use anyhow::{Context, Result, bail};
use indicatif::{ProgressBar, ProgressStyle};
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::Duration;

/// Rows buffered before each transaction commit. Keeps peak memory bounded
/// (~15 MB) without paying per-row commit overhead.
const CHUNK: usize = 50_000;

/// One row of the Lichess dump.
///
/// Column order in the source CSV:
/// PuzzleId, FEN, Moves, Rating, RatingDeviation, Popularity, NbPlays,
/// Themes, GameUrl, OpeningTags
#[derive(Debug, Deserialize)]
struct Record {
    #[serde(rename = "PuzzleId")]
    puzzle_id: String,
    #[serde(rename = "FEN")]
    fen: String,
    #[serde(rename = "Moves")]
    moves: String,
    #[serde(rename = "Rating")]
    rating: i64,
    #[serde(rename = "RatingDeviation")]
    rating_deviation: i64,
    #[serde(rename = "Popularity")]
    popularity: i64,
    #[serde(rename = "NbPlays")]
    nb_plays: i64,
    #[serde(rename = "Themes")]
    themes: String,
    #[serde(rename = "GameUrl")]
    game_url: String,
    #[serde(rename = "OpeningTags", default)]
    opening_tags: String,
    /// Derived on import, not read from the dump.
    #[serde(skip)]
    pieces: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Filters {
    pub min_popularity: i64,
    pub min_plays: i64,
    pub limit: Option<usize>,
}

#[derive(Debug, Default)]
pub struct ImportStats {
    pub rows_read: u64,
    pub accepted: u64,
    pub rejected_popularity: u64,
    pub rejected_plays: u64,
    pub rejected_malformed: u64,
    pub theme_count: u64,
    pub opening_count: u64,
    pub unparsed_game_urls: u64,
}

/// Rejects rows we could never serve correctly. Cheap structural checks, run
/// on every row before the filters; accepted rows are then replayed through
/// the rules engine to count their pieces, which also catches illegal moves.
fn validate(rec: &Record) -> Result<(), &'static str> {
    if rec.puzzle_id.is_empty() {
        return Err("empty puzzle id");
    }
    // The first move belongs to the opponent, so a usable puzzle needs at
    // least two: that move plus one move of solution.
    if rec.moves.split_whitespace().count() < 2 {
        return Err("fewer than two moves");
    }
    let mut fen_fields = rec.fen.split_whitespace();
    let (Some(_placement), Some(side)) = (fen_fields.next(), fen_fields.next()) else {
        return Err("malformed FEN");
    };
    if side != "w" && side != "b" {
        return Err("FEN side to move is neither w nor b");
    }
    if fen_fields.count() != 4 {
        return Err("FEN does not have six fields");
    }
    Ok(())
}

fn open_reader(path: &Path) -> Result<Box<dyn Read>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let buffered = BufReader::with_capacity(1 << 20, file);
    if path.extension().and_then(|e| e.to_str()) == Some("zst") {
        Ok(Box::new(
            zstd::Decoder::new(buffered).context("initialising zstd decoder")?,
        ))
    } else {
        Ok(Box::new(buffered))
    }
}

fn progress_bar() -> ProgressBar {
    let bar = ProgressBar::no_length().with_style(
        ProgressStyle::with_template("  {spinner:.cyan} {human_pos} rows read · {msg}")
            .expect("valid template"),
    );
    bar.enable_steady_tick(Duration::from_millis(120));
    bar
}

/// Assigns dense ids to puzzles and interns themes and openings as they are
/// first seen.
#[derive(Default)]
struct Ids {
    last_puzzle: i64,
    themes: HashMap<String, i64>,
    openings: HashMap<String, i64>,
}

pub fn load(conn: &mut Connection, source: &Path, filters: Filters) -> Result<ImportStats> {
    db::apply_schema(conn)?;

    let mut csv_reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(open_reader(source)?);

    let mut stats = ImportStats::default();
    let mut ids = Ids::default();
    let mut chunk: Vec<Record> = Vec::with_capacity(CHUNK);
    let bar = progress_bar();

    for result in csv_reader.deserialize::<Record>() {
        stats.rows_read += 1;
        if stats.rows_read % 25_000 == 0 {
            bar.set_position(stats.rows_read);
            bar.set_message(format!("{} kept", stats.accepted));
        }

        let mut record = match result {
            Ok(record) => record,
            Err(err) => {
                stats.rejected_malformed += 1;
                if stats.rejected_malformed <= 5 {
                    tracing::warn!("skipping unparseable row {}: {err}", stats.rows_read);
                }
                continue;
            }
        };

        if let Err(reason) = validate(&record) {
            stats.rejected_malformed += 1;
            if stats.rejected_malformed <= 5 {
                tracing::warn!("skipping {}: {reason}", record.puzzle_id);
            }
            continue;
        }
        if record.popularity < filters.min_popularity {
            stats.rejected_popularity += 1;
            continue;
        }
        if record.nb_plays < filters.min_plays {
            stats.rejected_plays += 1;
            continue;
        }

        match chess::pieces_to_solve(&record.fen, &record.moves) {
            Ok(pieces) => record.pieces = pieces,
            Err(err) => {
                stats.rejected_malformed += 1;
                if stats.rejected_malformed <= 5 {
                    tracing::warn!("skipping {}: {err:#}", record.puzzle_id);
                }
                continue;
            }
        }

        stats.accepted += 1;
        chunk.push(record);

        if chunk.len() >= CHUNK {
            insert_chunk(conn, &mut chunk, &mut ids, &mut stats)?;
        }
        if filters
            .limit
            .is_some_and(|limit| stats.accepted as usize >= limit)
        {
            break;
        }
    }

    insert_chunk(conn, &mut chunk, &mut ids, &mut stats)?;
    bar.set_position(stats.rows_read);
    bar.finish_with_message(format!("{} kept", stats.accepted));

    stats.theme_count = ids.themes.len() as u64;
    stats.opening_count = ids.openings.len() as u64;
    Ok(stats)
}

fn insert_chunk(
    conn: &mut Connection,
    chunk: &mut Vec<Record>,
    ids: &mut Ids,
    stats: &mut ImportStats,
) -> Result<()> {
    if chunk.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction().context("starting transaction")?;
    {
        let mut insert_puzzle = tx.prepare_cached(
            "INSERT INTO puzzles
                 (id, puzzle_id, fen, moves, rating, rating_deviation,
                  popularity, nb_plays, pieces, theme_mask_lo, theme_mask_hi,
                  opening_id, game_id, game_ply, game_black)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )?;
        let mut insert_theme =
            tx.prepare_cached("INSERT INTO themes (name) VALUES (?1) RETURNING id")?;
        let mut insert_opening =
            tx.prepare_cached("INSERT INTO openings (tags) VALUES (?1) RETURNING id")?;
        let mut insert_link = tx.prepare_cached(
            "INSERT OR IGNORE INTO puzzle_themes (theme_id, rating, puzzle_id, pieces)
             VALUES (?1, ?2, ?3, ?4)",
        )?;

        for record in chunk.drain(..) {
            ids.last_puzzle += 1;
            let id = ids.last_puzzle;

            // Intern themes first so the mask and the junction rows agree.
            let mut mask = ThemeMask::default();
            let mut theme_row_ids: Vec<i64> = Vec::new();
            for theme in record.themes.split_whitespace() {
                let theme_id = match ids.themes.get(theme) {
                    Some(id) => *id,
                    None => {
                        if ids.themes.len() >= MAX_THEMES {
                            bail!(
                                "the dump has more than {MAX_THEMES} distinct themes, \
                                 which no longer fit the 128-bit theme mask"
                            );
                        }
                        let new_id: i64 = insert_theme
                            .query_row((theme,), |row| row.get(0))
                            .with_context(|| format!("interning theme {theme}"))?;
                        ids.themes.insert(theme.to_string(), new_id);
                        new_id
                    }
                };
                mask.set(theme_id);
                theme_row_ids.push(theme_id);
            }

            let opening_id = if record.opening_tags.is_empty() {
                None
            } else {
                Some(match ids.openings.get(&record.opening_tags) {
                    Some(id) => *id,
                    None => {
                        let new_id: i64 = insert_opening
                            .query_row((&record.opening_tags,), |row| row.get(0))
                            .with_context(|| {
                                format!("interning opening {}", record.opening_tags)
                            })?;
                        ids.openings.insert(record.opening_tags.clone(), new_id);
                        new_id
                    }
                })
            };

            // A URL we cannot rebuild is not worth discarding a puzzle over;
            // the API simply omits the link for that row.
            let game = parse::parse_game_url(&record.game_url);
            if game.is_none() {
                stats.unparsed_game_urls += 1;
            }
            let (game_id, game_ply, game_black) = match &game {
                Some(game) => (game.game_id.as_str(), game.ply, game.black),
                None => ("", 0, false),
            };

            insert_puzzle
                .execute((
                    id,
                    &record.puzzle_id,
                    &record.fen,
                    &record.moves,
                    record.rating,
                    record.rating_deviation,
                    record.popularity,
                    record.nb_plays,
                    record.pieces,
                    mask.lo,
                    mask.hi,
                    opening_id,
                    game_id,
                    game_ply,
                    game_black,
                ))
                .with_context(|| format!("inserting puzzle {}", record.puzzle_id))?;

            for theme_id in theme_row_ids {
                insert_link.execute((theme_id, record.rating, id, record.pieces))?;
            }
        }
    }
    tx.commit().context("committing transaction")?;
    Ok(())
}

/// Post-load work: theme counts, indexes, planner statistics and compaction.
pub fn finalise(conn: &Connection, vacuum: bool) -> Result<()> {
    tracing::info!("counting puzzles per theme");
    conn.execute_batch(
        "UPDATE themes
            SET puzzle_count = (
                SELECT COUNT(*) FROM puzzle_themes WHERE theme_id = themes.id
            );",
    )
    .context("updating theme counts")?;

    tracing::info!("building indexes");
    db::apply_indexes(conn)?;

    tracing::info!("analysing");
    conn.execute_batch("ANALYZE;").context("running ANALYZE")?;

    if vacuum {
        tracing::info!("vacuuming");
        conn.execute_batch("VACUUM;").context("running VACUUM")?;
    }
    Ok(())
}
