-- Immutable puzzle database, rebuilt from scratch by `chess-puzzle-api import`.
-- Served read-only; nothing in here is written at request time.

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- `id` is a dense 1..N rowid alias. Density is what makes uniform random
-- sampling a single primary-key lookup when no filters are applied.
CREATE TABLE IF NOT EXISTS puzzles (
    id               INTEGER PRIMARY KEY,
    puzzle_id        TEXT    NOT NULL,
    fen              TEXT    NOT NULL,
    moves            TEXT    NOT NULL,
    rating           INTEGER NOT NULL,
    rating_deviation INTEGER NOT NULL,
    popularity       INTEGER NOT NULL,
    nb_plays         INTEGER NOT NULL,
    themes           TEXT    NOT NULL,
    opening_tags     TEXT    NOT NULL DEFAULT '',
    game_url         TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS themes (
    id           INTEGER PRIMARY KEY,
    name         TEXT    NOT NULL UNIQUE,
    puzzle_count INTEGER NOT NULL DEFAULT 0
);

-- Covering index disguised as a table: ordering by (theme_id, rating, puzzle_id)
-- turns "random puzzle with theme T rated between A and B" into a range scan
-- that never touches the `puzzles` table.
CREATE TABLE IF NOT EXISTS puzzle_themes (
    theme_id  INTEGER NOT NULL,
    rating    INTEGER NOT NULL,
    puzzle_id INTEGER NOT NULL,
    PRIMARY KEY (theme_id, rating, puzzle_id)
) WITHOUT ROWID;
