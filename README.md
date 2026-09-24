# chess-puzzle-api

Random chess puzzles by rating and tactical theme, from the
[Lichess puzzle database](https://database.lichess.org/#puzzles).
3.1 million puzzles, 73 themes, no API key required.

**Live at [chess.mauriulloa.com](https://chess.mauriulloa.com)**

```bash
curl "https://chess.mauriulloa.com/v1/puzzles/random?rating=1500&themes=fork"
```

By [Mauri Ulloa](https://mauriulloa.com) · [Full reference](https://chess.mauriulloa.com/docs)

## The one thing to know

`fen` is the position **before** the opponent's move, exactly as Lichess
stores it. Show it to a player unchanged and you are showing the wrong
position.

Use `positionFen` instead — the same position with `initialMove` already
applied — which is what the player is meant to solve. `solverColor` is the
side to move in it.

## Endpoints

| | |
| --- | --- |
| `GET /v1/puzzles/random` | a random puzzle, without the answer |
| `GET /v1/puzzles/{id}` | a specific puzzle, still without the answer |
| `GET /v1/puzzles/{id}/solution` | the answer |
| `GET /v1/themes` | every valid theme, its puzzle count and what it means |
| `GET /v1/stats` | dataset size, rating spread, provenance |
| `GET /v1/usage` | how much this service is being used |
| `POST /mcp` | the same operations as MCP tools, for agents |

Filters on `/v1/puzzles/random`:

| | |
| --- | --- |
| `rating`, `tolerance` | target rating and how far either side (default 100) |
| `ratingMin`, `ratingMax` | an explicit band instead |
| `themes`, `themesMode` | comma separated; `all` (default) or `any` |
| `excludeThemes` | themes to rule out |
| `opening` | e.g. `Sicilian_Defense` |
| `maxPieces` | at most this many pieces on the board, kings included |
| `count` | up to 50 at once |
| `board` | `true` draws the position as text |

An unknown theme is a `400`, not an empty result — call `/v1/themes` rather
than guessing. Unknown parameters are rejected too, so a typo surfaces
immediately instead of silently widening the search.

Ratings are Lichess's own puzzle ratings. Where a name helps — the trainer on
the landing page, the MCP guidance, and
[puzzle-sheets](https://puzzles.mauriulloa.com) — five contiguous bands are
used, so a rating belongs to exactly one: beginner under 1000, novice
1000–1399, intermediate 1400–1799, advanced 1800–2199, expert 2200+.

```json
{
  "id": "00008",
  "fen": "r6k/pp2r2p/4Rp1Q/3p4/8/1N1P2R1/PqP2bPP/7K b - - 0 24",
  "initialMove": "f2g3",
  "initialMoveSan": "Bxg3",
  "positionFen": "r6k/pp2r2p/4Rp1Q/3p4/8/1N1P2b1/PqP3PP/7K w - - 0 25",
  "solverColor": "white",
  "rating": 1797,
  "pieces": 19,
  "themes": ["middlegame", "crushing", "long", "hangingPiece"],
  "analysisUrl": "https://lichess.org/analysis/r6k/pp2r2p/...",
  "gameUrl": "https://lichess.org/787zsVup/black#48"
}
```

Moves come in both notations. UCI (`f2g3`) is what a program wants; SAN
(`Bxg3`) is what people read and what language models handle most reliably —
and UCI cannot express check or capture at all.

The solution is a separate request on purpose: it lets you put a puzzle in
front of someone without leaking what they are meant to find.

```json
{
  "solution": ["e6e7", "b2b1", "b3c1", "b1c1", "h6c1"],
  "solutionSan": ["Rxe7", "Qb1+", "Nc1", "Qxc1+", "Qxc1"]
}
```

[`/docs`](https://chess.mauriulloa.com/docs) has the full reference and
[`/openapi.json`](https://chess.mauriulloa.com/openapi.json) generates clients.

## For language models

`POST /mcp` is a Model Context Protocol server: an agent calls
`get_random_puzzle`, `get_solution`, `list_themes` and friends as tools rather
than assembling HTTP requests. Point any MCP client at
`https://chess.mauriulloa.com/mcp`.

For printable worksheets rather than one puzzle at a time,
[puzzle-sheets](https://puzzles.mauriulloa.com) is built on this API and has
its own MCP server.

Without an MCP client, [`/llms.txt`](https://chess.mauriulloa.com/llms.txt)
describes the API in plain text, and `?board=true` draws the position so a
model need not decode a FEN.

## Rate limits

30 requests a minute per IP by default (`ANONYMOUS_LIMIT`). A key raises that —
open an issue saying what you are building. Responses carry `X-RateLimit-Remaining`; a `429` carries
`Retry-After` in seconds.

```bash
curl -H "Authorization: Bearer cpa_..." https://chess.mauriulloa.com/v1/puzzles/random
```

## How it works

The dataset is immutable between imports, which keeps the architecture
unusually small: one read-only SQLite file, one static binary, no database
server and no cache layer.

Uniform random selection over a filtered subset is the only hard part.
Counting a *filtered* set means fetching every candidate from the puzzles
table to test it — seconds, once the file is larger than memory. So nothing
does: the rarest requested theme drives a range scan over a covering index
that also carries the rating and the piece count, a row is picked at a random
offset, and a 128-bit theme mask on the row checks whatever the scan could not
(a second required theme, an excluded one). A rejection just retries. Filtered
requests land in a few milliseconds of server time. No query may run longer
than five seconds: past that it is stopped and the caller gets a `503` with a
hint, so one expensive search cannot hold a connection hostage.

## What changes from the Lichess dump

No value is altered: FEN, moves, rating, rating deviation, popularity, play
count, themes, opening tags and puzzle ids are exactly Lichess's. The import
does three things to the dump.

- **Curation.** It keeps puzzles with `Popularity >= 90` and `NbPlays >= 100`:
  3.1 of 6.1 million rows. Not stricter, because popularity falls as
  difficulty rises, and a higher floor would gut the rare themes and high
  ratings. Rows that are malformed or whose first move is illegal are dropped
  too; the current dump has none.
- **Storage.** The game URL is kept as its parts and rebuilt on the way out;
  themes become ids, a bit mask and an index table; opening tags are stored
  once and referenced; each puzzle gets a dense internal id for fast random
  picks.
- **Derived data.** `pieces`, the number of pieces on the board the player
  solves, counted by replaying the opponent's move; puzzle counts per theme;
  and a `meta` table recording the source, the filters and the schema
  version.

Everything else the API shows — `positionFen`, SAN, the drawn board, the
analysis link — is computed per response and never stored.

## Self-hosting

```bash
cargo run --release -- import   # builds data/puzzles.db from the dump, ~40s
cargo run --release -- serve
```

`import` takes `--min-popularity`, `--min-plays` and `--limit` if you want a
different slice, and `--force` to rebuild over an existing file; a schema
change means rebuilding. `serve` reads `PUZZLES_DB`, `API_DB`, `BIND_ADDR`,
`ANONYMOUS_LIMIT`, `TRUST_PROXY_HEADERS` and `MCP_ALLOWED_HOSTS` — set the
last one to your domain or `/mcp` refuses every caller as a rebinding attempt.

`keys create --label "..."` mints an API key; only its hash is stored, so it
is printed once and cannot be recovered.

```bash
cargo test && cargo clippy --all-targets
```

Tests run against a small committed CSV fixture: no network, no 250 MB
download.

`scripts/verify_production.py` checks a running deployment end to end — it
re-derives every chess claim with python-chess, a separate implementation, so
agreement between the two means something.

## Operating

What the production deployment on Fly has taught, for whoever runs it next.

**Rebuilding the database** — needed whenever `SCHEMA_VERSION` changes.
`/health` says `schema_mismatch` while the loaded file is older than the
binary; the service keeps answering, but queries that need the new schema
fail until the rebuild.

1. Deploy the new binary, still pointing `PUZZLES_DB` at the current file.
2. Build the new file on the machine, next to the old one. Uploading one from
   a laptop does not work: `fly ssh sftp put` drops large transfers.
   ```bash
   fly ssh console -C "/usr/local/bin/chess-puzzle-api import --output /data/puzzles-v5.db --cache /data/lichess_db_puzzle.csv.zst --force --no-vacuum"
   ```
   `--no-vacuum` because `VACUUM` needs more memory than the machine has. On
   a shared CPU the import takes about ten minutes and spends its burst
   credit, so queries are slow for a while afterwards.
3. Point `PUZZLES_DB` in `fly.toml` at the new file and deploy.
4. Check `/health` for `"status":"ok"`.

**Deleting files from the volume.** The image has no shell and `sftp` has no
`rm`, so run `rm` from a throwaway image on the same machine, then redeploy.
The service is down in between, about two minutes; back up `api.db` and
`api.db-wal` first, since the keys cannot be recovered.
```bash
fly machine update <machine-id> --image busybox:stable --entrypoint "rm -v /data/<file>" --restart no --skip-health-checks --yes
fly deploy
```

**After a restart** the page cache is cold and the first requests read from
disk: a few seconds each until the indexes they touch are cached.

## Feedback

What is planned next, and why, is in [BACKLOG.md](BACKLOG.md).

This is early. If something is missing, awkward, or wrong — a filter you
expected, a field that would save you a round trip, an error message that did
not help — please
[open an issue](https://github.com/mauricioulloa/chess-puzzle-api/issues).
Reports of what you are building with it are just as welcome and will shape
what goes into the next version.

## Attribution

Puzzle data from the [Lichess open database](https://database.lichess.org/),
public domain under CC0. Lichess is free and ad-free —
[consider supporting them](https://lichess.org/patron). This project is not
affiliated with or endorsed by Lichess.

Chess pieces by
[Colin M.L. Burnett](https://en.wikipedia.org/wiki/User:Cburnett),
[CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/).

## License

[MIT](LICENSE) © [Mauri Ulloa](https://mauriulloa.com)
