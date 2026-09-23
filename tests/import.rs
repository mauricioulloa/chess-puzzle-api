//! Exercises the importer against a small committed fixture so the curation
//! rules and the schema stay honest without downloading the 250 MB dump.

use chess_puzzle_api::import::load::{self, Filters, ImportStats};
use rusqlite::Connection;
use std::path::PathBuf;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/puzzles_sample.csv"
);

fn import_fixture(filters: Filters) -> (Connection, ImportStats) {
    let mut conn = Connection::open_in_memory().expect("in-memory database");
    let stats = load::load(&mut conn, &PathBuf::from(FIXTURE), filters).expect("import succeeds");
    load::finalise(&conn, false).expect("finalise succeeds");
    (conn, stats)
}

fn default_filters() -> Filters {
    Filters {
        min_popularity: 90,
        min_plays: 100,
        limit: None,
    }
}

fn scalar<T: rusqlite::types::FromSql>(conn: &Connection, sql: &str) -> T {
    conn.query_row(sql, [], |row| row.get(0)).expect(sql)
}

#[test]
fn curation_keeps_and_drops_the_expected_rows() {
    let (_conn, stats) = import_fixture(default_filters());

    assert_eq!(stats.rows_read, 14);
    assert_eq!(stats.accepted, 9);
    assert_eq!(stats.rejected_popularity, 2);
    assert_eq!(stats.rejected_plays, 1);
    assert_eq!(
        stats.rejected_malformed, 2,
        "one short move list, one bad FEN"
    );
}

#[test]
fn threshold_rows_are_inclusive() {
    let (conn, _) = import_fixture(default_filters());
    // 0013r sits exactly on both thresholds (popularity 90, plays 100).
    let kept: i64 = scalar(
        &conn,
        "SELECT COUNT(*) FROM puzzles WHERE puzzle_id = '0013r'",
    );
    assert_eq!(kept, 1, "rows exactly at the threshold must be kept");
}

#[test]
fn puzzle_ids_are_dense_and_sequential() {
    let (conn, stats) = import_fixture(default_filters());

    let count: i64 = scalar(&conn, "SELECT COUNT(*) FROM puzzles");
    let min_id: i64 = scalar(&conn, "SELECT MIN(id) FROM puzzles");
    let max_id: i64 = scalar(&conn, "SELECT MAX(id) FROM puzzles");

    assert_eq!(count, stats.accepted as i64);
    assert_eq!(min_id, 1);
    assert_eq!(
        max_id, count,
        "dense ids are what make unfiltered random sampling a single lookup"
    );
}

#[test]
fn themes_are_interned_and_counted() {
    let (conn, stats) = import_fixture(default_filters());

    let themes: i64 = scalar(&conn, "SELECT COUNT(*) FROM themes");
    assert_eq!(themes, 15);
    assert_eq!(stats.theme_count, 15);

    let middlegame: i64 = scalar(
        &conn,
        "SELECT puzzle_count FROM themes WHERE name = 'middlegame'",
    );
    assert_eq!(middlegame, 6);

    let fork: i64 = scalar(&conn, "SELECT puzzle_count FROM themes WHERE name = 'fork'");
    assert_eq!(fork, 2);

    // Dropped rows must not leave their themes behind.
    let unpopular: i64 = scalar(
        &conn,
        "SELECT COUNT(*) FROM themes WHERE name = 'rookEndgame'",
    );
    assert_eq!(unpopular, 1);
}

#[test]
fn the_theme_mask_agrees_with_the_junction_table() {
    let (conn, _) = import_fixture(default_filters());

    // Every junction row must have its bit set in the puzzle's mask, and the
    // popcount of the mask must equal the number of junction rows. If these
    // two representations ever drift, filtering silently returns wrong results.
    let mismatches: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM puzzle_themes pt
             JOIN puzzles p ON p.id = pt.puzzle_id
             WHERE CASE
                 WHEN pt.theme_id <= 64
                     THEN (p.theme_mask_lo >> (pt.theme_id - 1)) & 1
                     ELSE (p.theme_mask_hi >> (pt.theme_id - 65)) & 1
             END = 0",
            [],
            |row| row.get(0),
        )
        .expect("mask cross-check");
    assert_eq!(mismatches, 0, "every junction row must be set in the mask");

    let links: i64 = scalar(&conn, "SELECT COUNT(*) FROM puzzle_themes");
    let bits_set: i64 = conn
        .query_row(
            "WITH RECURSIVE bit(n) AS (SELECT 0 UNION ALL SELECT n + 1 FROM bit WHERE n < 63)
             SELECT SUM(((p.theme_mask_lo >> b.n) & 1) + ((p.theme_mask_hi >> b.n) & 1))
             FROM puzzles p, bit b",
            [],
            |row| row.get(0),
        )
        .expect("popcount");
    assert_eq!(
        bits_set, links,
        "mask popcount must equal the junction rows"
    );
}

#[test]
fn game_urls_survive_the_round_trip() {
    let (conn, _) = import_fixture(default_filters());

    let rebuilt: String = conn
        .query_row(
            "SELECT 'https://lichess.org/' || game_id
                    || CASE game_black WHEN 1 THEN '/black' ELSE '' END
                    || '#' || game_ply
             FROM puzzles WHERE puzzle_id = '00008'",
            [],
            |row| row.get(0),
        )
        .expect("puzzle 00008");
    assert_eq!(rebuilt, "https://lichess.org/787zsVup/black#48");
}

#[test]
fn openings_are_interned_and_nullable() {
    let (conn, _) = import_fixture(default_filters());

    // The fixture carries no opening tags at all.
    let with_opening: i64 = scalar(
        &conn,
        "SELECT COUNT(*) FROM puzzles WHERE opening_id IS NOT NULL",
    );
    assert_eq!(with_opening, 0);
}

#[test]
fn theme_and_rating_range_scan_returns_matches() {
    let (conn, _) = import_fixture(default_filters());

    let matches: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM puzzle_themes pt
             JOIN themes t ON t.id = pt.theme_id
             WHERE t.name = 'short' AND pt.rating BETWEEN 1000 AND 1800",
            [],
            |row| row.get(0),
        )
        .expect("range scan");
    // 00014 (1543), 000rO (1602), 000Vc (1755), 000Zo (1320), 0013r (1105)
    assert_eq!(matches, 5);
}

#[test]
fn moves_are_stored_intact_for_solution_lookup() {
    let (conn, _) = import_fixture(default_filters());

    let (fen, moves): (String, String) = conn
        .query_row(
            "SELECT fen, moves FROM puzzles WHERE puzzle_id = '00008'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("puzzle 00008");

    assert!(fen.starts_with("r6k/pp2r2p"));
    let moves: Vec<&str> = moves.split_whitespace().collect();
    assert_eq!(moves.len(), 6);
    assert_eq!(moves[0], "f2g3", "first move belongs to the opponent");
    assert_eq!(moves[1], "e6e7", "solution starts at the second move");
}

#[test]
fn limit_stops_the_import_early() {
    let (conn, stats) = import_fixture(Filters {
        limit: Some(3),
        ..default_filters()
    });

    assert_eq!(stats.accepted, 3);
    let count: i64 = scalar(&conn, "SELECT COUNT(*) FROM puzzles");
    assert_eq!(count, 3);
}

#[test]
fn loosening_the_filters_keeps_more_rows() {
    let (_conn, stats) = import_fixture(Filters {
        min_popularity: -100,
        min_plays: 0,
        limit: None,
    });

    // Everything except the two structurally invalid rows.
    assert_eq!(stats.accepted, 12);
    assert_eq!(stats.rejected_popularity, 0);
    assert_eq!(stats.rejected_plays, 0);
    assert_eq!(stats.rejected_malformed, 2);
}

#[tokio::test]
async fn a_missing_input_does_not_cost_the_existing_database() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let output = dir.path().join("puzzles.db");
    std::fs::write(&output, b"the database already being served").expect("seed file");

    let result = chess_puzzle_api::import::run(chess_puzzle_api::import::ImportArgs {
        output: output.clone(),
        input: Some(dir.path().join("missing.csv")),
        cache: dir.path().join("cache.csv.zst"),
        source_url: String::new(),
        min_popularity: 90,
        min_plays: 100,
        limit: None,
        force: true,
        no_vacuum: true,
        refresh: false,
    })
    .await;

    assert!(result.is_err());
    assert!(
        output.exists(),
        "--force replaces the database, never just deletes it"
    );
}
