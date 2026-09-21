# chess-puzzle-api

An open-source HTTP API that serves random chess puzzles by rating and theme,
backed by the [Lichess puzzle database](https://database.lichess.org/#puzzles).

Ask it for a puzzle at a given difficulty and tactical motif, get a FEN back.
Ask it for the solution by puzzle id, get the moves.

```
GET /v1/puzzles/random?rating=1500&themes=fork
GET /v1/puzzles/00008/solution
```

## Status

Early development. The importer is done; the HTTP layer is being built.

- [x] Puzzle database importer and curation pipeline
- [ ] Query layer and HTTP endpoints
- [ ] API keys and rate limiting
- [ ] OpenAPI documentation
- [ ] Deployment

## Why it exists

The Lichess puzzle dump is excellent but it ships as a 250 MB compressed CSV
with five million rows. If all you want is "give me a random fork puzzle around
1500", you have to download it, parse it, index it and host it yourself. This
project does that once and puts an HTTP endpoint in front of it.

## Design

The puzzle data is **immutable** between imports, which makes the architecture
unusually simple: the whole dataset lives in a read-only SQLite file that ships
with the binary. No database server, no connection strings, no cache layer.
A single static binary and a file.

Two separate databases, because they have opposite lifecycles:

| Database | Mode | Contents |
| --- | --- | --- |
| `puzzles.db` | read-only | curated puzzles, themes, indexes |
| `api.db` | read-write | API keys and their usage |

### Random sampling

Uniform random selection over a filtered subset is the one genuinely
interesting query here.

- **No filters.** Puzzle ids are dense (`1..N`), so a random puzzle is
  `WHERE id = ?` with a random number: a single primary-key lookup.
- **With filters.** `puzzle_themes` is a `WITHOUT ROWID` table keyed by
  `(theme_id, rating, puzzle_id)`, which makes it a covering index. Filtering
  by theme and rating range is a range scan that never touches the `puzzles`
  table; a `COUNT` plus `LIMIT 1 OFFSET <random>` then picks uniformly from it.
- **Multiple themes.** The rarest requested theme drives the scan (themes carry
  a `puzzle_count`), and the rest are checked against a denormalised `themes`
  column on the row itself. This avoids intersecting large posting lists.

### Curation

The full dump contains every puzzle ever generated, including unpopular and
barely-played ones. By default the importer keeps puzzles with
`Popularity >= 90` and `NbPlays >= 100`, which trades a chunk of the row count
for a meaningfully better hit rate. Both thresholds are flags, so you can build
whatever slice you want.

Rows are also dropped when they are structurally unusable: fewer than two moves
(a puzzle needs the opponent's move plus at least one move of solution), or a
FEN that is not six space-separated fields.

## Building the puzzle database

```bash
cargo run --release -- import
```

This downloads the dump to `data/lichess_db_puzzle.csv.zst` (cached between
runs), filters it, and writes `data/puzzles.db`.

Useful flags:

| Flag | Default | Purpose |
| --- | --- | --- |
| `--output <path>` | `data/puzzles.db` | where to write the database |
| `--input <path>` | — | use a local `.csv.zst` or `.csv` instead of downloading |
| `--min-popularity <n>` | `90` | popularity floor (`-100..100`) |
| `--min-plays <n>` | `100` | minimum number of times the puzzle was played |
| `--limit <n>` | — | stop after N puzzles, for quick local runs |
| `--force` | — | overwrite an existing output database |
| `--refresh` | — | re-download even if the cached dump is present |
| `--no-vacuum` | — | skip the final `VACUUM` |

A quick local database for development:

```bash
cargo run --release -- import --limit 50000 --output data/dev.db --force
```

## Development

```bash
cargo test            # unit and integration tests
cargo clippy --all-targets
cargo fmt --all
```

The importer is covered by integration tests that run against
`tests/fixtures/puzzles_sample.csv`, a hand-written fixture exercising the
curation thresholds and the structural validation. No network access and no
250 MB download required to run the suite.

## Data source and attribution

Puzzle data comes from the [Lichess open database](https://database.lichess.org/),
released into the public domain under
[CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/).

Lichess is a free, open-source chess server. If you find this API useful, please
consider [supporting them](https://lichess.org/patron) — they produce the data
this project merely repackages.

This project is not affiliated with or endorsed by Lichess.

## License

[MIT](LICENSE)
