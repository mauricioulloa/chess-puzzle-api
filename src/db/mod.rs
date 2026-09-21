use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub const SCHEMA: &str = include_str!("schema.sql");
pub const INDEXES: &str = include_str!("indexes.sql");

/// Meta keys written by the importer and surfaced by `GET /v1/stats`.
pub mod meta_keys {
    pub const SOURCE_URL: &str = "source_url";
    pub const IMPORTED_AT: &str = "imported_at";
    pub const PUZZLE_COUNT: &str = "puzzle_count";
    pub const THEME_COUNT: &str = "theme_count";
    pub const MIN_POPULARITY: &str = "filter_min_popularity";
    pub const MIN_PLAYS: &str = "filter_min_plays";
    pub const ROWS_READ: &str = "source_rows_read";
    pub const SCHEMA_VERSION: &str = "schema_version";
}

pub const SCHEMA_VERSION: &str = "1";

/// Opens a connection tuned for the bulk import: durability is pointless here
/// because a failed import is thrown away and re-run from the source dump.
pub fn open_for_import(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }
    let conn =
        Connection::open(path).with_context(|| format!("opening database {}", path.display()))?;
    conn.execute_batch(
        "PRAGMA journal_mode = OFF;
         PRAGMA synchronous = OFF;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -131072;",
    )
    .context("applying import pragmas")?;
    Ok(conn)
}

pub fn apply_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA).context("applying schema")
}

pub fn apply_indexes(conn: &Connection) -> Result<()> {
    conn.execute_batch(INDEXES).context("creating indexes")
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        (key, value),
    )
    .with_context(|| format!("writing meta key {key}"))?;
    Ok(())
}
