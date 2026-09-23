//! Random puzzle selection.
//!
//! The whole design hangs on one measurement: counting a *filtered* set means
//! fetching every candidate from the 600 MB table to test its theme mask,
//! which takes roughly half a second. Counting the driving range scan alone
//! touches only a covering index and takes about a millisecond.
//!
//! So nothing ever counts the filtered set on the hot path. A cheap scan is
//! counted, a row is picked at a random offset, and its mask decides whether
//! to keep it. A rejection just tries again.

use crate::api::catalog::Catalog;
use crate::db::pool::SqlitePool;
use crate::import::parse::{GameRef, ThemeMask};
use anyhow::{Context, Result};
use rand::RngExt;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, params_from_iter};
use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};

/// How many rejections to tolerate before falling back to an exact query.
/// Generous, because each attempt costs about a millisecond.
const MAX_ATTEMPTS: usize = 32;

pub const RATING_FLOOR: i64 = 0;
pub const RATING_CEILING: i64 = 4000;
pub const DEFAULT_TOLERANCE: i64 = 100;

/// The band `rating ± tolerance`. A band that runs off either end is clamped
/// rather than rejected: asking for rating 3200 is a reasonable request.
pub fn band_around(rating: i64, tolerance: i64) -> (i64, i64) {
    (
        (rating - tolerance).max(RATING_FLOOR),
        (rating + tolerance).min(RATING_CEILING),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemesMode {
    /// The puzzle must carry every requested theme.
    #[default]
    All,
    /// The puzzle must carry at least one of them.
    Any,
}

impl ThemesMode {
    pub fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("all") {
            Some(Self::All)
        } else if value.eq_ignore_ascii_case("any") {
            Some(Self::Any)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PuzzleFilter {
    pub rating_min: Option<i64>,
    pub rating_max: Option<i64>,
    pub include: Vec<i64>,
    pub exclude: Vec<i64>,
    pub mode: ThemesMode,
    /// Resolved opening ids; a name can match several interned tag strings.
    pub opening_ids: Vec<i64>,
}

impl PuzzleFilter {
    fn bounds(&self) -> (i64, i64) {
        (
            self.rating_min.unwrap_or(RATING_FLOOR),
            self.rating_max.unwrap_or(RATING_CEILING),
        )
    }

    /// True when no predicate narrows the dataset at all, which unlocks the
    /// single primary-key lookup.
    fn is_unconstrained(&self) -> bool {
        self.rating_min.is_none()
            && self.rating_max.is_none()
            && self.include.is_empty()
            && self.exclude.is_empty()
            && self.opening_ids.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct PuzzleRow {
    pub puzzle_id: String,
    pub fen: String,
    pub moves: String,
    pub rating: i64,
    pub rating_deviation: i64,
    pub popularity: i64,
    pub nb_plays: i64,
    pub mask: ThemeMask,
    pub opening_id: Option<i64>,
    pub opening_tags: Option<String>,
    pub game: Option<GameRef>,
}

impl PuzzleRow {
    /// The opponent's move, which the client applies to `fen` before solving.
    pub fn initial_move(&self) -> &str {
        self.moves.split_whitespace().next().unwrap_or_default()
    }

    /// Everything after the opponent's move. This is the answer, and it is
    /// only ever served by the solution endpoint.
    pub fn solution(&self) -> Vec<&str> {
        self.moves.split_whitespace().skip(1).collect()
    }

    /// The side the solver plays. The FEN records the side to move *before*
    /// the opponent's move, so the solver is always the other colour.
    pub fn solver_color(&self) -> &'static str {
        match self.fen.split_whitespace().nth(1) {
            Some("w") => "black",
            _ => "white",
        }
    }
}

const COLUMNS: &str = "p.puzzle_id, p.fen, p.moves, p.rating, p.rating_deviation, \
                       p.popularity, p.nb_plays, p.theme_mask_lo, p.theme_mask_hi, \
                       p.opening_id, o.tags, p.game_id, p.game_ply, p.game_black";

const SOURCE: &str = "FROM puzzles p LEFT JOIN openings o ON o.id = p.opening_id";

fn map_row(row: &Row<'_>) -> rusqlite::Result<PuzzleRow> {
    let game_id: String = row.get(11)?;
    let game_ply: i64 = row.get(12)?;
    let game_black: bool = row.get(13)?;

    Ok(PuzzleRow {
        puzzle_id: row.get(0)?,
        fen: row.get(1)?,
        moves: row.get(2)?,
        rating: row.get(3)?,
        rating_deviation: row.get(4)?,
        popularity: row.get(5)?,
        nb_plays: row.get(6)?,
        mask: ThemeMask {
            lo: row.get(7)?,
            hi: row.get(8)?,
        },
        opening_id: row.get(9)?,
        opening_tags: row.get(10)?,
        game: (!game_id.is_empty()).then_some(GameRef {
            game_id,
            ply: game_ply,
            black: game_black,
        }),
    })
}

/// Identifies a countable range scan. `theme` is `None` for the rating-only
/// scan over the puzzles table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ScanKey {
    theme: Option<i64>,
    rating_min: i64,
    rating_max: i64,
}

/// A snapshot of the dataset, for the stats endpoint.
#[derive(Debug, Clone)]
pub struct DatasetStats {
    pub puzzles: i64,
    pub openings: i64,
    pub rating_min: i64,
    pub rating_max: i64,
    pub bands: Vec<(i64, i64, i64)>,
    pub meta: HashMap<String, String>,
}

pub struct Sampler {
    pool: SqlitePool,
    pub catalog: Catalog,
    counts: RwLock<HashMap<ScanKey, i64>>,
    /// The dataset is read-only for the life of the process, so its statistics
    /// are computed once. Recomputing them per request meant a GROUP BY over
    /// three million rows: 600 ms of server time for an answer that cannot
    /// change until the database is rebuilt and the process restarted.
    stats: OnceLock<DatasetStats>,
}

impl Sampler {
    pub fn new(pool: SqlitePool, catalog: Catalog) -> Self {
        Self {
            pool,
            catalog,
            counts: RwLock::new(HashMap::new()),
            stats: OnceLock::new(),
        }
    }

    pub fn total_puzzles(&self) -> i64 {
        self.catalog.puzzle_count
    }

    fn connection(&self) -> Result<crate::db::pool::PooledConnection> {
        self.pool.get().context("acquiring a database connection")
    }

    /// Counts a range scan, memoised. Only ever called for scans that ride a
    /// covering index, so a cache miss costs about a millisecond.
    fn count_scan(&self, conn: &Connection, key: ScanKey) -> Result<i64> {
        if let Some(cached) = self.counts.read().expect("counts lock").get(&key) {
            return Ok(*cached);
        }

        let count: i64 = match key.theme {
            Some(theme_id) => conn.query_row(
                "SELECT COUNT(*) FROM puzzle_themes
                 WHERE theme_id = ?1 AND rating BETWEEN ?2 AND ?3",
                (theme_id, key.rating_min, key.rating_max),
                |row| row.get(0),
            ),
            None => conn.query_row(
                "SELECT COUNT(*) FROM puzzles WHERE rating BETWEEN ?1 AND ?2",
                (key.rating_min, key.rating_max),
                |row| row.get(0),
            ),
        }
        .context("counting candidates")?;

        self.counts.write().expect("counts lock").insert(key, count);
        Ok(count)
    }

    pub fn by_puzzle_id(&self, puzzle_id: &str) -> Result<Option<PuzzleRow>> {
        let conn = self.connection()?;
        let sql = format!("SELECT {COLUMNS} {SOURCE} WHERE p.puzzle_id = ?1");
        conn.query_row(&sql, (puzzle_id,), map_row)
            .optional()
            .context("looking up puzzle by id")
    }

    fn by_row_id(&self, conn: &Connection, id: i64) -> Result<Option<PuzzleRow>> {
        let sql = format!("SELECT {COLUMNS} {SOURCE} WHERE p.id = ?1");
        conn.query_row(&sql, (id,), map_row)
            .optional()
            .context("fetching puzzle by row id")
    }

    /// Draws `count` distinct puzzles.
    ///
    /// The attempt budget is shared across the batch rather than fixed per
    /// slot. Collecting k distinct items from a pool of k takes about k*ln(k)
    /// draws, so a per-slot budget quietly gives up on small candidate sets:
    /// asking for 20 puzzles out of 9 would return six of them and call it a
    /// day. Against the real dataset duplicates are vanishingly rare and the
    /// budget is never approached.
    pub fn random(&self, filter: &PuzzleFilter, count: usize) -> Result<Vec<PuzzleRow>> {
        let conn = self.connection()?;
        let mut picked: Vec<PuzzleRow> = Vec::with_capacity(count);
        let mut seen: HashSet<String> = HashSet::with_capacity(count);
        let budget = count * 8 + 32;

        for _ in 0..budget {
            if picked.len() >= count {
                break;
            }
            match self.sample_one(&conn, filter)? {
                Some(row) => {
                    if seen.insert(row.puzzle_id.clone()) {
                        picked.push(row);
                    }
                }
                // The filter matches nothing at all; retrying cannot help.
                None => break,
            }
        }
        Ok(picked)
    }

    fn sample_one(&self, conn: &Connection, filter: &PuzzleFilter) -> Result<Option<PuzzleRow>> {
        if filter.is_unconstrained() {
            let total = self.total_puzzles();
            if total == 0 {
                return Ok(None);
            }
            let id = rand::rng().random_range(1..=total);
            return self.by_row_id(conn, id);
        }

        let (rating_min, rating_max) = filter.bounds();
        let want = ThemeMask::from_ids(&filter.include);
        let excl = ThemeMask::from_ids(&filter.exclude);

        // Pick the scan that drives sampling. With ALL semantics the rarest
        // requested theme is the cheapest scan that still contains every
        // possible answer. With ANY semantics each requested theme owns a
        // slice of the answer set, so one is chosen in proportion to its size.
        let drivers: Vec<(i64, i64)> = if filter.include.is_empty() {
            vec![]
        } else {
            let mut sized = Vec::with_capacity(filter.include.len());
            for &theme_id in &filter.include {
                let key = ScanKey {
                    theme: Some(theme_id),
                    rating_min,
                    rating_max,
                };
                sized.push((theme_id, self.count_scan(conn, key)?));
            }
            sized
        };

        let driver: Option<(i64, i64)> = match filter.mode {
            _ if drivers.is_empty() => None,
            ThemesMode::All => drivers.iter().copied().min_by_key(|(_, count)| *count),
            ThemesMode::Any => {
                let total: i64 = drivers.iter().map(|(_, count)| count).sum();
                if total == 0 {
                    None
                } else {
                    let mut target = rand::rng().random_range(0..total);
                    let mut chosen = drivers[0];
                    for candidate in &drivers {
                        if target < candidate.1 {
                            chosen = *candidate;
                            break;
                        }
                        target -= candidate.1;
                    }
                    Some(chosen)
                }
            }
        };

        let (scan_key, candidates) = match driver {
            Some((theme_id, count)) => (
                ScanKey {
                    theme: Some(theme_id),
                    rating_min,
                    rating_max,
                },
                count,
            ),
            None => {
                let key = ScanKey {
                    theme: None,
                    rating_min,
                    rating_max,
                };
                let count = self.count_scan(conn, key)?;
                (key, count)
            }
        };

        if candidates == 0 {
            return Ok(None);
        }

        for _ in 0..MAX_ATTEMPTS {
            let offset = rand::rng().random_range(0..candidates);
            let Some(row) = self.pick_at_offset(conn, scan_key, offset)? else {
                continue;
            };
            if self.accepts(&row, filter, want, excl) {
                return Ok(Some(row));
            }
        }

        // Highly selective filters can starve the sampler. Fall back to the
        // slow-but-exact path rather than pretending nothing matches.
        self.sample_exact(conn, filter, scan_key, want, excl)
    }

    fn pick_at_offset(
        &self,
        conn: &Connection,
        key: ScanKey,
        offset: i64,
    ) -> Result<Option<PuzzleRow>> {
        let id: Option<i64> = match key.theme {
            Some(theme_id) => conn.query_row(
                "SELECT puzzle_id FROM puzzle_themes
                 WHERE theme_id = ?1 AND rating BETWEEN ?2 AND ?3
                 LIMIT 1 OFFSET ?4",
                (theme_id, key.rating_min, key.rating_max, offset),
                |row| row.get(0),
            ),
            // Offset over the covering index alone. Selecting the full row
            // here instead would make SQLite materialise and discard every
            // skipped row, join included: measured at 136 ms against 3 ms.
            None => conn.query_row(
                "SELECT id FROM puzzles
                 WHERE rating BETWEEN ?1 AND ?2
                 ORDER BY rating, id LIMIT 1 OFFSET ?3",
                (key.rating_min, key.rating_max, offset),
                |row| row.get(0),
            ),
        }
        .optional()
        .context("picking a candidate")?;

        match id {
            Some(id) => self.by_row_id(conn, id),
            None => Ok(None),
        }
    }

    fn accepts(
        &self,
        row: &PuzzleRow,
        filter: &PuzzleFilter,
        want: ThemeMask,
        excl: ThemeMask,
    ) -> bool {
        if row.mask.lo & excl.lo != 0 || row.mask.hi & excl.hi != 0 {
            return false;
        }
        if !filter.opening_ids.is_empty()
            && !row
                .opening_id
                .is_some_and(|id| filter.opening_ids.contains(&id))
        {
            return false;
        }
        if filter.include.is_empty() {
            return true;
        }
        match filter.mode {
            ThemesMode::All => row.mask.lo & want.lo == want.lo && row.mask.hi & want.hi == want.hi,
            ThemesMode::Any => {
                // Choosing the driving theme in proportion to its size makes a
                // puzzle carrying k of the requested themes k times as likely
                // to be drawn. Accepting it with probability 1/k cancels that
                // out exactly, leaving a uniform sample over the union.
                let matches =
                    (row.mask.lo & want.lo).count_ones() + (row.mask.hi & want.hi).count_ones();
                matches > 0 && rand::rng().random_range(0..matches) == 0
            }
        }
    }

    /// The exact path: build the full predicate, count it, and pick uniformly.
    /// Correct for any filter, but it touches the puzzles table once per
    /// candidate, so it is reserved for filters the fast path cannot satisfy.
    fn sample_exact(
        &self,
        conn: &Connection,
        filter: &PuzzleFilter,
        key: ScanKey,
        want: ThemeMask,
        excl: ThemeMask,
    ) -> Result<Option<PuzzleRow>> {
        let (mut where_sql, mut params) = (Vec::new(), Vec::new());

        let source = match key.theme {
            Some(theme_id) => {
                where_sql.push("pt.theme_id = ?".to_string());
                params.push(Value::Integer(theme_id));
                where_sql.push("pt.rating BETWEEN ? AND ?".to_string());
                params.push(Value::Integer(key.rating_min));
                params.push(Value::Integer(key.rating_max));
                "FROM puzzle_themes pt JOIN puzzles p ON p.id = pt.puzzle_id \
                  LEFT JOIN openings o ON o.id = p.opening_id"
                    .to_string()
            }
            None => {
                where_sql.push("p.rating BETWEEN ? AND ?".to_string());
                params.push(Value::Integer(key.rating_min));
                params.push(Value::Integer(key.rating_max));
                SOURCE.to_string()
            }
        };

        if filter.mode == ThemesMode::All && !filter.include.is_empty() {
            where_sql.push("(p.theme_mask_lo & ?) = ?".to_string());
            params.push(Value::Integer(want.lo));
            params.push(Value::Integer(want.lo));
            where_sql.push("(p.theme_mask_hi & ?) = ?".to_string());
            params.push(Value::Integer(want.hi));
            params.push(Value::Integer(want.hi));
        }
        if !filter.exclude.is_empty() {
            where_sql.push("(p.theme_mask_lo & ?) = 0".to_string());
            params.push(Value::Integer(excl.lo));
            where_sql.push("(p.theme_mask_hi & ?) = 0".to_string());
            params.push(Value::Integer(excl.hi));
        }
        if !filter.opening_ids.is_empty() {
            let placeholders = vec!["?"; filter.opening_ids.len()].join(", ");
            where_sql.push(format!("p.opening_id IN ({placeholders})"));
            params.extend(filter.opening_ids.iter().map(|id| Value::Integer(*id)));
        }

        let predicate = where_sql.join(" AND ");
        let count: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) {source} WHERE {predicate}"),
                params_from_iter(params.iter()),
                |row| row.get(0),
            )
            .context("exact candidate count")?;

        if count == 0 {
            return Ok(None);
        }

        let offset = rand::rng().random_range(0..count);
        let mut pick_params = params.clone();
        pick_params.push(Value::Integer(offset));

        conn.query_row(
            &format!("SELECT {COLUMNS} {source} WHERE {predicate} LIMIT 1 OFFSET ?"),
            params_from_iter(pick_params.iter()),
            map_row,
        )
        .optional()
        .context("exact pick")
    }

    /// Resolves an opening name to the interned rows that mention it.
    ///
    /// `opening_tags` holds several space-separated tags per puzzle, so an
    /// exact match on the interned string would almost never be what a caller
    /// means. The table has around 1400 rows, making a scan free.
    pub fn resolve_openings(&self, name: &str) -> Result<Vec<i64>> {
        let conn = self.connection()?;
        let needle = format!("%{}%", name.replace(' ', "_"));
        let mut statement = conn
            .prepare("SELECT id FROM openings WHERE tags LIKE ?1")
            .context("preparing opening lookup")?;
        let ids = statement
            .query_map((needle,), |row| row.get::<_, i64>(0))
            .context("resolving opening")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("reading openings")?;
        Ok(ids)
    }

    pub fn stats(&self) -> Result<DatasetStats> {
        if let Some(cached) = self.stats.get() {
            return Ok(cached.clone());
        }
        let computed = self.compute_stats()?;
        Ok(self.stats.get_or_init(|| computed).clone())
    }

    fn compute_stats(&self) -> Result<DatasetStats> {
        let conn = self.connection()?;

        let openings: i64 = conn
            .query_row("SELECT COUNT(*) FROM openings", [], |row| row.get(0))
            .context("counting openings")?;
        let (rating_min, rating_max): (i64, i64) = conn
            .query_row("SELECT MIN(rating), MAX(rating) FROM puzzles", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .context("rating bounds")?;

        let mut statement = conn
            .prepare(
                "SELECT rating / 200 * 200 AS band, COUNT(*)
                 FROM puzzles GROUP BY band ORDER BY band",
            )
            .context("preparing distribution query")?;
        let bands = statement
            .query_map([], |row| {
                let from: i64 = row.get(0)?;
                Ok((from, from + 199, row.get(1)?))
            })
            .context("reading distribution")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collecting distribution")?;

        let mut meta = HashMap::new();
        let mut meta_statement = conn
            .prepare("SELECT key, value FROM meta")
            .context("preparing meta query")?;
        let rows = meta_statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .context("reading meta")?;
        for row in rows {
            let (key, value) = row.context("meta row")?;
            meta.insert(key, value);
        }

        Ok(DatasetStats {
            puzzles: self.catalog.puzzle_count,
            openings,
            rating_min,
            rating_max,
            bands,
            meta,
        })
    }
}
