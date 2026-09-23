-- Immutable puzzle database, rebuilt from scratch by `chess-puzzle-api import`.
-- Served read-only; nothing in here is written at request time.

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Opening tags are a small vocabulary repeated across millions of rows, and
-- four out of five puzzles carry none at all, so they are interned.
CREATE TABLE IF NOT EXISTS openings (
    id   INTEGER PRIMARY KEY,
    tags TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS themes (
    id           INTEGER PRIMARY KEY,
    name         TEXT    NOT NULL UNIQUE,
    puzzle_count INTEGER NOT NULL DEFAULT 0
);

-- `id` is a dense 1..N rowid alias. Density is what makes uniform random
-- sampling a single primary-key lookup when no filters are applied.
--
-- Themes are stored twice on purpose. `puzzle_themes` below drives the range
-- scan; the 128-bit mask here answers the secondary predicates (match all of
-- these, exclude those) with a bitwise AND instead of substring matching,
-- which would otherwise have to distinguish 'mate' from 'mateIn2'.
--
-- `pieces` counts the board the player solves, after the opponent's move.
--
-- The game URL is decomposed rather than stored: every row in the dump is
-- https://lichess.org/{game_id}[/black]#{ply}, so the prefix is dead weight.
CREATE TABLE IF NOT EXISTS puzzles (
    id               INTEGER PRIMARY KEY,
    puzzle_id        TEXT    NOT NULL,
    fen              TEXT    NOT NULL,
    moves            TEXT    NOT NULL,
    rating           INTEGER NOT NULL,
    rating_deviation INTEGER NOT NULL,
    popularity       INTEGER NOT NULL,
    nb_plays         INTEGER NOT NULL,
    pieces           INTEGER NOT NULL,
    theme_mask_lo    INTEGER NOT NULL,
    theme_mask_hi    INTEGER NOT NULL,
    opening_id       INTEGER REFERENCES openings (id),
    game_id          TEXT    NOT NULL,
    game_ply         INTEGER NOT NULL,
    game_black       INTEGER NOT NULL
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
