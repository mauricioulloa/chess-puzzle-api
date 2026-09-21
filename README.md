# chess-puzzle-api

An open-source HTTP API that serves random chess puzzles by rating and theme,
backed by the [Lichess puzzle database](https://database.lichess.org/#puzzles).

Ask it for a puzzle at a given difficulty and tactical motif, get a FEN back.
Ask it for the solution by puzzle id, get the moves.

```
GET /v1/puzzles/random?rating=1500&themes=fork
GET /v1/puzzles/00008/solution
```

By [Mauri Ulloa](https://mauriulloa.com).

## Status

Early development. The importer is done; the HTTP layer is being built.

- [x] Puzzle database importer and curation pipeline
      (3.1M puzzles, 73 themes, 633 MB, imports in 29s)
- [x] Query layer and HTTP endpoints
- [x] API keys and rate limiting
- [x] OpenAPI documentation
- [x] Usage statistics, MCP server, discovery pages
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

Notation is computed per request rather than stored. Replaying a puzzle
through the rules engine takes about 2.5 microseconds against a query that
costs hundreds, so precomputing SAN would add tens of megabytes to save
nothing. Replaying all 3,128,032 puzzles takes under eight seconds, and every
one of them is legal chess.

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

## API

All responses are JSON. Every endpoint is a `GET`, CORS is open, and responses
are gzipped.

### `GET /v1/puzzles/random`

Returns one random puzzle, without its solution.

| Parameter | Default | Meaning |
| --- | --- | --- |
| `rating` | — | target rating; combined with `tolerance` into a band |
| `tolerance` | `100` | half-width of the band around `rating` |
| `ratingMin` / `ratingMax` | — | explicit band, as an alternative to `rating` |
| `themes` | — | comma-separated theme names (`theme` also accepted) |
| `themesMode` | `all` | `all` requires every theme, `any` requires one |
| `excludeThemes` | — | comma-separated themes to rule out |
| `opening` | — | opening tag, e.g. `Sicilian_Defense` |
| `count` | — | return a batch of up to 50 instead of one puzzle |
| `board` | `false` | include the position drawn as text |

```bash
curl "http://localhost:8080/v1/puzzles/random?rating=1500&themes=fork"
```

```json
{
  "id": "YoKhQ",
  "fen": "6k1/6p1/p3p3/3pP3/1P2nQ2/P5P1/1P2q1K1/7R w - - 1 37",
  "initialMove": "g2h3",
  "solverColor": "black",
  "rating": 1548,
  "ratingDeviation": 76,
  "popularity": 95,
  "nbPlays": 4455,
  "initialMoveSan": "Kh3",
  "positionFen": "6k1/6p1/p3p3/3pP3/1P2nQ2/P5PK/1P2q3/7R b - - 2 37",
  "analysisUrl": "https://lichess.org/analysis/6k1/6p1/p3p3/3pP3/1P2nQ2/P5PK/1P2q3/7R_b_-_-_2_37",
  "themes": [
    "endgame",
    "short",
    "crushing",
    "fork"
  ],
  "openingTags": [],
  "gameUrl": "https://lichess.org/uc3ICm4X#73"
}
```

`fen` is the position as Lichess stores it, *before* the opponent's move.
`positionFen` is the same position after `initialMove`, which is what the
player actually faces; `fen` is kept unchanged so existing clients do not
break. `solverColor` is the colour about to move in `positionFen`.

Every move comes in both notations. UCI (`g2h3`) is what a program wants. SAN
(`Kh3`) is what chess literature is written in, which makes it what people
read at a glance and what language models handle most reliably — a model
reasoning over `e6e7 b2b1 b3c1` does noticeably worse than over
`Rxe7 Qb1+ Nc1`, and UCI cannot express check or capture at all.

`board=true` adds the position drawn as text, which saves a model from
decoding a FEN in its head:

```
8 r . . . . . . k
7 p p . . r . . p
6 . . . . R p . Q
5 . . . p . . . .
4 . . . . . . . .
3 . N . P . . b .
2 P q P . . . P P
1 . . . . . . . K
  a b c d e f g h
```

With `count`, the response becomes `{"count": n, "puzzles": [...]}` so each
spelling has one stable shape.

### `GET /v1/puzzles/{id}`

The same object for a specific puzzle. Still no solution.

### `GET /v1/puzzles/{id}/solution`

```json
{
  "id": "00008",
  "initialMove": "f2g3",
  "solution": ["e6e7", "b2b1", "b3c1", "b1c1", "h6c1"],
  "solutionSan": ["Rxe7", "Qb1+", "Nc1", "Qxc1+", "Qxc1"]
}
```

### `GET /v1/themes`

All 73 themes with how many puzzles carry each. Start here: theme names are
what `themes` and `excludeThemes` expect, and an unknown name is a `400`, not
an empty result.

### `GET /v1/stats`

Dataset size, rating distribution in 200-point bands, and which dump the
database was built from.

### `GET /health`

### Errors

Errors carry a machine-readable `error`, a human `message` and sometimes a
`hint`. Unknown query parameters are rejected rather than ignored, so a typo
surfaces as a `400` instead of silently widening the search.

```json
{
  "error": "bad_request",
  "message": "Unknown themes: frok",
  "hint": "GET /v1/themes lists all 73 valid themes."
}
```

## Discovery

| Endpoint | Audience |
| --- | --- |
| `/` | a landing page for whoever pastes the bare domain into a browser |
| `/llms.txt` | what this API is, in plain text, for a language model |
| `/docs` | the rendered reference |
| `/openapi.json` | the machine-readable schema |

Both `/` and `/llms.txt` quote the host they were reached on, so the examples
they print work where the reader actually is rather than on one blessed
deployment. `llms.txt` leads with the thing a model most needs to be told:
that `fen` is the position *before* the opponent's move, and that presenting
it to a player unchanged shows the wrong position.

## Documentation

| Endpoint | What it is |
| --- | --- |
| `/docs` | a rendered API reference |
| `/openapi.json` | the OpenAPI 3.1 description, for client generation |

The spec is generated from the handler annotations, so it cannot describe a
route that does not exist. A copy lives at [`openapi.json`](openapi.json) for
tooling that would rather not start the server; a test fails if it drifts from
the code, and regenerating is:

```bash
cargo run -- openapi > openapi.json
```

Generating a client, for example:

```bash
npx @openapitools/openapi-generator-cli generate \
  -i https://your-host/openapi.json -g typescript-fetch -o ./client
```

Both endpoints sit outside the rate limiter, along with `/health`: reading the
documentation should not spend anyone's quota.


## For language models

`POST /mcp` is a Model Context Protocol server, so an agent calls this as
tools rather than being told to assemble HTTP requests from documentation.

| Tool | Purpose |
| --- | --- |
| `get_random_puzzle` | a puzzle by rating and theme, without the answer |
| `get_puzzle` | a specific puzzle by id, still without the answer |
| `get_solution` | the answer, in SAN and UCI |
| `list_themes` | the exact theme names, which are not guessable |
| `get_dataset_stats` | size, rating range and provenance |

Point an MCP client at `https://your-host/mcp`. The server is stateless: no
sessions to keep and no streams held open, so a restart drops nobody.

What the tools return is deliberately not the HTTP response shape. A model
wants different things from a program, so the board is always drawn rather
than opt-in, SAN leads, the opponent's move is already applied, and every
description states the thing that is otherwise got wrong. Keeping the solution
behind its own tool is the point: it lets an agent pose a puzzle to someone
without knowing the answer until it asks.

Also for models, without any MCP client:

- `/llms.txt` describes the API in plain text.
- `?board=true` on any puzzle endpoint draws the position, which is more
  reliable to reason over than a FEN.
- Errors name the tool or endpoint that fixes them — an unknown theme points
  at `list_themes` rather than just refusing.

## Usage statistics

`GET /v1/usage` publishes how the service is being used: request totals over
the last 30 days, a daily series, a breakdown by endpoint and status, and the
filters people actually ask for — which themes, which rating bands, how often
a batch or a drawn board is requested.

It is public on purpose. It is also publishable *by construction*: the
counters store whether a request carried **a** key, never **which**, and no
client address is stored anywhere. Per-key totals exist, but they live in the
`api_keys` table where only the operator sees them, via `keys list`.

Counters, not logs. A row per request would grow without bound, write to disk
on every hit, and — because a request carries an IP — become personal data
with a retention policy attached. Daily counters answer the same questions in
a few kilobytes a month.

Searches that match nothing are counted too, and deliberately: a filter that
returns a 404 is someone telling you what the dataset does not have, which is
the clearest signal there is about where the curation falls short of demand.

Endpoints are counted by route template, so three million puzzle ids do not
become three million rows.

## Authentication and rate limits

The API is open: no key is needed. A key only raises the ceiling.

| | Requests per minute |
| --- | --- |
| anonymous, per IP | 30 |
| with a key | set per key, 600 by default |

Every response carries `X-RateLimit-Limit`, `X-RateLimit-Remaining`,
`X-RateLimit-Reset` (seconds until the window rolls over) and
`X-RateLimit-Scope` (`anonymous` or `key`). A `429` adds `Retry-After`.

Send a key as a bearer token:

```bash
curl -H "Authorization: Bearer cpa_..." \
     "http://localhost:8080/v1/puzzles/random?rating=1500"
```

An unrecognised or revoked key returns `401` rather than quietly falling back
to the anonymous quota, because silently ignoring a key the caller believes is
working produces baffling `429`s later. A malformed `Authorization` header with
no bearer token is treated as anonymous.

`/health` sits outside the limiter: a host polling it should never be told the
service is over quota.

### Getting a key

Open an issue on this repository saying what you are building. Keys are issued
by hand.

### Issuing keys

```bash
chess-puzzle-api keys create --label "someone's tactics trainer" --rate-limit 600
chess-puzzle-api keys list
chess-puzzle-api keys revoke --label "someone's tactics trainer"
```

Only the SHA-256 of a key is stored, so `create` prints it exactly once and
nobody — including you — can recover it afterwards. A leaked backup of `api.db`
hands over no working keys. Revocation takes effect on the next request.


## Running the API

```bash
cargo run --release -- serve
```

| Flag | Environment variable | Default |
| --- | --- | --- |
| `--db` | `PUZZLES_DB` | `data/puzzles.db` |
| `--api-db` | `API_DB` | `data/api.db` |
| `--bind` | `BIND_ADDR` | `127.0.0.1:8080` |
| `--pool-size` | `POOL_SIZE` | `8` |
| `--anonymous-limit` | `ANONYMOUS_LIMIT` | `30` |
| `--trust-proxy-headers` | `TRUST_PROXY_HEADERS` | off |

Enable `--trust-proxy-headers` only behind a proxy that overwrites
`X-Forwarded-For`. Without one, any caller can reset their own rate limit by
inventing the header.

Measured on the full 3.1M-puzzle database, server-side, excluding client
overhead:

| Query | Median | p99 |
| --- | --- | --- |
| no filters | 0.15 ms | 0.26 ms |
| rating band | 1.97 ms | 3.74 ms |
| rating and theme | 0.47 ms | 0.80 ms |
| three themes with an exclusion | 0.28 ms | 0.67 ms |
| batch of 20 | 0.33 ms | 0.40 ms |


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

## Author

Built and maintained by **[Mauri Ulloa](https://mauriulloa.com)**.

Issues and pull requests are welcome. API keys are issued by opening an issue.

## License

[MIT](LICENSE) © Mauri Ulloa

The puzzle data is not covered by this licence: it is public domain (CC0) and
belongs to nobody.
