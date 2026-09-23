//! A puzzle database built from the committed CSV fixture, shared by the
//! integration suites.

use chess_puzzle_api::api::catalog::Catalog;
use chess_puzzle_api::api::query::Sampler;
use chess_puzzle_api::db;
use chess_puzzle_api::import::load::{self, Filters};
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/puzzles_sample.csv"
);

/// The directory holds the database file, so it must outlive the sampler.
pub fn fixture_sampler() -> (TempDir, Arc<Sampler>) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("puzzles.db");

    let mut conn = db::open_for_import(&path).expect("open for import");
    load::load(
        &mut conn,
        Path::new(FIXTURE),
        Filters {
            min_popularity: 90,
            min_plays: 100,
            limit: None,
        },
    )
    .expect("import");
    load::finalise(&conn, false).expect("finalise");
    drop(conn);

    let pool = db::pool::open_read_only(&path, 2).expect("read pool");
    let catalog = Catalog::load(&pool.get().expect("connection")).expect("catalog");
    (dir, Arc::new(Sampler::new(pool, catalog)))
}
