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
fn junction_rows_match_the_denormalised_column() {
    let (conn, _) = import_fixture(default_filters());

    let from_column: i64 = scalar(
        &conn,
        "SELECT SUM(LENGTH(TRIM(themes)) - LENGTH(REPLACE(TRIM(themes), ' ', '')) + 1)
         FROM puzzles WHERE TRIM(themes) <> ''",
    );
    let from_junction: i64 = scalar(&conn, "SELECT COUNT(*) FROM puzzle_themes");
    assert_eq!(
        from_column, from_junction,
        "the junction table and the denormalised column must agree"
    );
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
