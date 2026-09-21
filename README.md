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
      (3.1M puzzles, 73 themes, 633 MB, imports in 29s)
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
unusually simple: the whole dataset lives in one read-only SQLite file that the
server memory-maps and never writes to. No database server, no connection
strings, no cache layer. A single static binary and a file.

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
  `(theme_id, rating, puzzle_id)`, which makes it a covering index. The rarest
  requested theme drives a range scan over it that never touches the `puzzles`
  table: counting 46k candidates that way takes **1 ms**.
- **Secondary predicates.** Each puzzle also carries its themes as a 128-bit
  mask split across two `INTEGER` columns, so "match all of these, exclude
  those" is a bitwise `AND` on the row. Substring matching on a text column
  would have to work to distinguish `mate` from `mateIn2`; the mask cannot get
  that wrong.
- **Rejection sampling.** Counting the *filtered* set is the trap: every
  candidate has to be fetched from the 600 MB table to test its mask, which
  measured at ~500 ms. Instead only the driving scan is counted, a row is
  picked at a random offset, and its mask is checked — about 1 ms per attempt.
  A failed check just retries. At a 55% acceptance rate that is under two
  attempts on average, so a fully filtered request lands in roughly 3 ms.

### Storage

The dataset is repetitive in ways worth exploiting:

- **Game URLs** are uniformly `https://lichess.org/{game_id}[/black]#{ply}` —
  every row in the dump, no exceptions — so the URL is decomposed on import and
  rebuilt on the way out instead of storing the constant prefix three million
  times.
- **Opening tags** are a small vocabulary, and four out of five puzzles carry
  none at all, so they are interned into a lookup table.

### Curation

The full dump contains every puzzle ever generated, including unpopular and
barely-played ones. By default the importer keeps puzzles with
`Popularity >= 90` and `NbPlays >= 100`, which drops roughly half the rows.

Those defaults are deliberately not stricter. Raising the popularity floor to
95 looks tempting — it would halve the database again — but the loss is not
evenly distributed. Popularity correlates inversely with difficulty, because
hard puzzles get downvoted:

| | at `>= 90` | at `>= 95` |
| --- | --- | --- |
| `underPromotion` puzzles | 645 | 63 |
| `castling` puzzles | 1082 | 110 |
| puzzles rated 2600+ | 57553 | 6776 |

A stricter filter would gut exactly the rare themes and high ratings where
variety matters most. Both thresholds are flags, so you can build whatever
slice you want.

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
