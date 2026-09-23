-- Applied after the bulk load: building these up front would slow the import
-- down by roughly an order of magnitude.
CREATE UNIQUE INDEX IF NOT EXISTS idx_puzzles_puzzle_id ON puzzles (puzzle_id);
-- `pieces` is carried, not sorted on, so a piece cap stays index-only.
CREATE INDEX IF NOT EXISTS idx_puzzles_rating ON puzzles (rating, id, pieces);
