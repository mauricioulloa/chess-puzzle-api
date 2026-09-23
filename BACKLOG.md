# Backlog

What came out of building and shipping v0.1. Ordered by what unblocks the most,
not by what is easiest. Anything measured says so — those numbers were taken
from the live service or the real dataset, not estimated.

Items needing Mauri's own accounts cannot be done unattended; they are marked
**[needs you]**.

---

## Discoverability: people

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

**Do not** generate a page per puzzle. Three million thin pages is what search
engines classify as spam, and the penalty lands on the whole domain.

### 4. Shareable puzzle URLs — `/puzzle/{id}`

So a person can send someone a specific position. Worth having for sharing
even with most of them `noindex`; weak as a search strategy on its own.

### 5. Per-puzzle preview images

Each shared puzzle previewing as its own position. Much stronger on social,
but X and Facebook do not accept SVG, so this means rasterising to PNG on the
server and carrying an image library in a binary that is currently 12 MB.
Worth it only once sharing is actually happening.

---

## Discoverability: agents

Search engines are the wrong instrument here. Agents do not crawl — they read
registries. `/llms.txt` and `/mcp` already exist and nothing points at them.

### 6. Submit to the MCP registries — **[needs you]**

The official Model Context Protocol registry, plus Smithery, Glama, mcp.so and
PulseMCP. Mostly a form or a pull request; each needs an account. This is
paperwork, not engineering, and it is the single highest-leverage thing for
agent reach.

### 7. Submit the API to the usual lists

`public-apis` on GitHub, APIs.guru for the OpenAPI document, the Postman public
network. A Show HN and a post to r/chess and the Lichess forum are the human
equivalent.

---

## The educator and parent feature

Printable puzzle sheets. The market is real — scholastic chess, homeschooling,
clubs — the free alternatives are poor, and ChessKid is a subscription.

One differentiator is not being used anywhere today: **the data is CC0**. A
teacher photocopying a puzzle book is in a legal grey area. Here the sheet is
theirs, unconditionally. That belongs on the page in plain words.

### 8. A piece-count filter — the finding that matters

**Low rating does not mean simple position.** Measured on the real dataset:

| Slice | Median pieces | ≤12 pieces |
| --- | --- | --- |
| Mate in 1, under 1000 | 19 | 8% |
| Mate in 2, under 1300 | 16 | 11% |
| Fork, under 1200 | 14 | 37% |

A mate-in-one with nineteen pieces is an easy move inside a full middlegame. A
seven-year-old has to scan the whole board to find it. The classic teaching
diagram is sparse, and rating alone cannot select for that.

Also counter-intuitive and useful: **fork puzzles are better teaching material
than mate-in-ones** at beginner level, by a wide margin.

The fix is a piece-count column written at import and exposed as a filter
(`maxPieces`, or a friendlier "simple positions"). It costs a schema change and
a re-import, which takes 29 seconds. It is the difference between a sheet a
teacher uses and one they throw away.

Material is not the constraint: 313,405 mate-in-ones under 1000, of which even
the tidy 8% is ~25,000 puzzles.

### 9. The printable sheet itself — `/worksheet`

Six diagrams to a page, "White to move" under each, room to write the answer,
and an answer key on its own page so the teacher can keep it. Black and white
for print — the green board wastes ink and greys into mush.

Built with `@media print` and the browser's own "Save as PDF". No server-side
PDF machinery for v1.

### 10. Teaching presets

"Beginner: forks", "Mate in one", "First endgames" — a teacher should not have
to reason about Elo bands and theme ids. This is where the piece-count filter
and the curation actually surface.

---

## Product debt

### 11. Theme descriptions on `/v1/themes`

The endpoint returns names and counts but no explanations, so callers have to
guess what `hangingPiece` means. Lichess's own descriptions live in an
AGPL-licensed repository, which does not mix cleanly with MIT — writing fresh
ones avoids the question entirely.

### 12. Revisit the rate limit with real data

30/min anonymous was a guess and has never met real traffic. `/v1/usage` will
show whether it is too tight before anyone complains.

### 13. A playable board

Letting the visitor move the pieces and be told whether they are right, instead
of only revealing the answer. Deliberately deferred: it needs legal-move
generation in the browser and handling the opponent's replies. Worth doing once
there is evidence people are using the trainer.

---

## Not doing, and why

- **A page per puzzle for search.** Three million thin pages reads as spam.
- **Chess.com's piece artwork.** Proprietary. The Cburnett set is free, looks
  professional, and is already in place with its attribution.
- **Server-side PDF generation.** The browser prints to PDF perfectly well.
- **Accounts, saved worksheets, progress tracking.** A different product.
