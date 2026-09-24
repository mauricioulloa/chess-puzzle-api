# Backlog

Ordered by priority, not by effort. Anything measured says so — those numbers
were taken from the live service or the real dataset, not estimated.

Items needing Mauri's own accounts cannot be done unattended; they are marked
**[needs you]**.

---

## 1 · Discoverability: people

The honest diagnosis, measured on the live page: **267 indexable words and 3
crawlable URLs**. No `robots.txt`, no sitemap, no favicon, no Open Graph, no
structured data. Three million puzzles behind a single page.

The problem is not missing meta tags. It is that there are no pages.

### 1. The basics

`robots.txt`, `sitemap.xml`, a favicon, `<link rel="canonical">`, Open Graph
and Twitter card tags, and schema.org structured data describing the site as a
free tool. Half a day, and it stops a shared link from previewing as nothing.

### 2. A static social preview image

Sharing a link today shows a blank card. One fixed design — a board and the
site name — captures most of the benefit for very little work.

### 3. Landing pages per theme and level

The real search play: roughly 50–350 pages ("Fork puzzles", "Mate in two for
beginners"), each with a working board and actual prose explaining the motif.
The theme descriptions now served by `/v1/themes` are a start on that prose.

**Do not** generate a page per puzzle. Three million thin pages is what search
engines classify as spam, and the penalty lands on the whole domain.

### 4. Shareable puzzle URLs — `/puzzle/{id}`

So a person can send someone a specific position. Worth having for sharing
even with most of them `noindex`; weak as a search strategy on its own.

### 5. Per-puzzle preview images

Each shared puzzle previewing as its own position. Much stronger on social,
but X and Facebook do not accept SVG, so this means rasterising to PNG on the
server and carrying an image library in a binary that is currently 14 MB.
Worth it only once sharing is actually happening.

---

## 2 · Discoverability: agents

Search engines are the wrong instrument here. Agents do not crawl — they read
registries. `/llms.txt` and `/mcp` already exist and nothing points at them.

### 6. Submit to the MCP registries — **[needs you]**

The official Model Context Protocol registry, plus Smithery, Glama, mcp.so and
PulseMCP. Mostly a form or a pull request; each needs an account. This is
paperwork, not engineering, and it is the single highest-leverage thing for
agent reach. puzzle-sheets has an MCP server of its own worth listing too.

### 7. Submit the API to the usual lists

`public-apis` on GitHub, APIs.guru for the OpenAPI document, the Postman public
network. A Show HN and a post to r/chess and the Lichess forum are the human
equivalent.

---

## 3 · Later

### 8. A playable board

Letting the visitor move the pieces and be told whether they are right, instead
of only revealing the answer. It needs legal-move generation in the browser and
handling the opponent's replies. Worth doing once there is evidence people are
using the trainer.

### 9. Look puzzles up in batches

`GET /v1/puzzles?ids=` and the same for solutions. A twelve-puzzle sheet from
puzzle-sheets costs 25 requests today; batches would make it three.

### 10. Revisit the rate limit with real data

30/min anonymous was a guess and has never met real traffic. `/v1/usage` will
show whether it is too tight before anyone complains.

### 11. Bring the verification script up to date

`scripts/verify_production.py` predates `pieces`, `maxPieces` and the theme
descriptions, and the keys it used are revoked. It should re-derive the piece
count with python-chess like every other chess claim.

### 12. Maintenance window

`/data/probe.bin`, an empty file left by an upload test, goes the next time
the machine is down for something else; removing it costs the two minutes of
downtime described under Operating in the README.

---

## Not doing, and why

- **A page per puzzle for search.** Three million thin pages reads as spam.
- **Chess.com's piece artwork.** Proprietary. The Cburnett set is free, looks
  professional, and is already in place with its attribution.
- **Printable puzzle sheets.** That is
  [puzzle-sheets](https://github.com/mauricioulloa/puzzle-sheets), which is
  built on this API. This repository serves puzzles and the filters to find
  them, like `maxPieces`; never layouts.
- **Accounts, saved puzzles, progress tracking.** A different product.
