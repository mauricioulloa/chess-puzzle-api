use anyhow::{Context, Result};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OpenFlags;
use std::path::Path;

pub type SqlitePool = r2d2::Pool<SqliteConnectionManager>;
pub type PooledConnection = r2d2::PooledConnection<SqliteConnectionManager>;

/// Opens the puzzle database for reading only.
///
/// `SQLITE_OPEN_READ_ONLY` plus `query_only` means a bug in a handler cannot
/// mutate the dataset, and a large `mmap_size` lets the OS page cache do the
/// caching instead of duplicating pages into SQLite's own cache.
///
/// `cache_size` is deliberately small for that reason: it is per connection, so
/// anything generous here is multiplied by the pool and comes out of the heap,
/// where it competes with the page cache that is already holding those pages.
/// Small heap caches let the machine be sized by the working set, not the pool.
pub fn open_read_only(path: &Path, pool_size: u32) -> Result<SqlitePool> {
    if !path.exists() {
        anyhow::bail!(
            "{} does not exist; build it first with `chess-puzzle-api import`",
            path.display()
        );
    }

    let manager = SqliteConnectionManager::file(path)
        .with_flags(OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .with_init(|conn| {
            conn.execute_batch(
                "PRAGMA query_only = 1;
                 PRAGMA mmap_size = 1073741824;
                 PRAGMA cache_size = -2000;
                 PRAGMA temp_store = MEMORY;",
            )
        });

    r2d2::Pool::builder()
        .max_size(pool_size)
        .build(manager)
        .with_context(|| format!("opening {} for reading", path.display()))
}
