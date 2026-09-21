//! The two pages that exist for discovery rather than for data: a landing page
//! for a person who pastes the bare domain into a browser, and `/llms.txt` for
//! a model that wants to know what this is without parsing HTML or a 700-line
//! OpenAPI document.

/// Both pages quote real numbers from the loaded dataset rather than figures
/// baked into a string that would quietly go stale on the next import.
pub struct SiteFacts {
    pub puzzles: i64,
    pub themes: usize,
    pub base_url: String,
}

pub fn landing(facts: &SiteFacts) -> String {
    let SiteFacts {
        puzzles,
        themes,
        base_url,
    } = facts;
    let puzzles = thousands(*puzzles);

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>chess-puzzle-api</title>
<meta name="description" content="A free API serving random chess puzzles by rating and theme, from the Lichess puzzle database.">
<style>
  :root {{
    color-scheme: light dark;
    --bg: #fbfaf8; --fg: #1a1a1a; --muted: #5c5c5c;
    --line: #e3e0da; --code-bg: #f2efe9; --accent: #3c6e47;
  }}
  @media (prefers-color-scheme: dark) {{
    :root {{
      --bg: #16171a; --fg: #e8e6e3; --muted: #9a9894;
      --line: #2c2e33; --code-bg: #1f2125; --accent: #8fc79a;
    }}
  }}
  * {{ box-sizing: border-box; }}
  body {{
    background: var(--bg); color: var(--fg); margin: 0;
    font: 16px/1.65 ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
  }}
  main {{ max-width: 46rem; margin: 0 auto; padding: 3rem 1rem 5rem; }}
  h1 {{ font-size: 1.9rem; margin: 0 0 .4rem; letter-spacing: -.02em; }}
  h2 {{ font-size: 1.05rem; margin: 2.5rem 0 .75rem; letter-spacing: .02em;
        text-transform: uppercase; color: var(--muted); }}
  p {{ margin: 0 0 1rem; }}
  .lede {{ font-size: 1.1rem; color: var(--muted); margin-bottom: 2rem; }}
  pre {{
    background: var(--code-bg); border: 1px solid var(--line); border-radius: 6px;
    padding: .9rem 1rem; overflow-x: auto; margin: 0 0 1rem;
    font: 13.5px/1.6 ui-monospace, SFMono-Regular, Menlo, monospace;
  }}
  code {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .92em; }}
  p code, li code {{ background: var(--code-bg); padding: .1em .35em; border-radius: 3px; }}
  a {{ color: var(--accent); text-underline-offset: 2px; }}
  ul {{ padding-left: 1.2rem; margin: 0 0 1rem; }}
  li {{ margin-bottom: .4rem; }}
  .stats {{ display: flex; gap: 2.5rem; flex-wrap: wrap; margin: 0 0 1rem; padding: 0; list-style: none; }}
  .stats div {{ }}
  .stats b {{ display: block; font-size: 1.5rem; font-weight: 600; }}
  .stats span {{ color: var(--muted); font-size: .85rem; }}
  footer {{ margin-top: 3.5rem; padding-top: 1.5rem; border-top: 1px solid var(--line);
            color: var(--muted); font-size: .9rem; }}
</style>
</head>
<body>
<main>
  <h1>chess-puzzle-api</h1>
  <p class="lede">Random chess puzzles by rating and theme. No key required.</p>

  <div class="stats">
    <div><b>{puzzles}</b><span>puzzles</span></div>
    <div><b>{themes}</b><span>tactical themes</span></div>
    <div><b>CC0</b><span>public domain data</span></div>
  </div>

  <h2>Try it</h2>
  <pre><code>curl "{base_url}/v1/puzzles/random?rating=1500&amp;themes=fork"</code></pre>
  <p>
    You get the position, its rating and its themes — but not the answer.
    Ask for that separately with
    <code>/v1/puzzles/{{id}}/solution</code>.
  </p>

  <h2>For people</h2>
  <ul>
    <li><a href="/docs">Full API reference</a>, rendered.</li>
    <li>Every puzzle carries an <code>analysisUrl</code> that opens the position on a real board.</li>
    <li>Moves come in SAN (<code>Qxe6+</code>) as well as UCI (<code>d1e6</code>).</li>
  </ul>

  <h2>For code</h2>
  <ul>
    <li><a href="/openapi.json">OpenAPI 3.1 description</a>, generated from the handlers.</li>
    <li>Open CORS, gzipped JSON, stable field names.</li>
    <li>30 requests a minute anonymously; a key raises that.</li>
  </ul>

  <h2>For language models</h2>
  <ul>
    <li><a href="/llms.txt">llms.txt</a> — what this API is, in plain text.</li>
    <li><code>{base_url}/mcp</code> — an MCP server, so an agent can call it as a tool.</li>
    <li><code>?board=true</code> draws the position as text, so a model need not decode a FEN.</li>
  </ul>

  <footer>
    Puzzle data from the
    <a href="https://database.lichess.org/#puzzles">Lichess open database</a>,
    released under CC0. Lichess is free and ad-free —
    <a href="https://lichess.org/patron">consider supporting them</a>.
    This project is not affiliated with Lichess.
    <br>
    Source on <a href="https://github.com/mauricioulloa/chess-puzzle-api">GitHub</a>, MIT licensed.
  </footer>
</main>
</body>
</html>
"##
    )
}

/// Follows the llms.txt convention: an H1, a blockquote summary, then linked
/// sections. Kept short on purpose — a model that needs the exact schema is
/// pointed at the OpenAPI document rather than handed a copy of it.
pub fn llms_txt(facts: &SiteFacts) -> String {
    let SiteFacts {
        puzzles,
        themes,
        base_url,
    } = facts;
    let puzzles = thousands(*puzzles);

    format!(
        r#"# chess-puzzle-api

> A free HTTP API serving random chess puzzles by rating and tactical theme,
> drawn from the Lichess puzzle database ({puzzles} puzzles, {themes} themes,
> CC0 public domain). No API key is required.

Ask for a puzzle and you get the position without the answer; the solution is
a separate request. This is deliberate, so the API can be put in front of a
player without leaking what they are meant to find.

## Key concepts

- `fen` is the position as Lichess stores it, BEFORE the opponent's move.
- `initialMove` is that opponent move. `positionFen` is the position after it,
  which is the one the player is asked to solve.
- `solverColor` is the colour to move in `positionFen`.
- Moves come in both UCI (`d1e6`) and SAN (`Qxe6+`). Prefer SAN when reasoning
  or explaining; UCI cannot express check or capture.
- `board=true` adds the position drawn as ASCII, which is more reliable to
  reason over than a FEN.

## Endpoints

- [{base_url}/v1/puzzles/random]({base_url}/v1/puzzles/random): a random puzzle. Filter with `rating` and `tolerance`, or `ratingMin`/`ratingMax`; `themes` (comma separated) with `themesMode=all|any`; `excludeThemes`; `opening`; `count` for up to 50 at once; `board=true`.
- [{base_url}/v1/puzzles/{{id}}]({base_url}/v1/puzzles/00008): one puzzle by id, still without its solution.
- [{base_url}/v1/puzzles/{{id}}/solution]({base_url}/v1/puzzles/00008/solution): the answer, in UCI and SAN.
- [{base_url}/v1/themes]({base_url}/v1/themes): every valid theme name with its puzzle count. Read this before guessing a theme; an unknown name is a 400, not an empty result.
- [{base_url}/v1/stats]({base_url}/v1/stats): dataset size, rating distribution and provenance.
- [{base_url}/v1/usage]({base_url}/v1/usage): aggregate usage of this service.

## Optional

- [{base_url}/openapi.json]({base_url}/openapi.json): the full OpenAPI 3.1 schema.
- [{base_url}/docs]({base_url}/docs): the same reference, rendered for people.
- [{base_url}/mcp]({base_url}/mcp): Model Context Protocol endpoint, for calling this API as agent tools.

## Limits

30 requests per minute per IP without a key. Responses carry
`X-RateLimit-Remaining`; a 429 carries `Retry-After` in seconds. Back off
rather than retrying immediately.

## Attribution

Puzzle data comes from https://database.lichess.org/#puzzles under CC0. This
project is not affiliated with or endorsed by Lichess.
"#
    )
}

/// 3128032 is hard to read; 3,128,032 is not.
fn thousands(value: i64) -> String {
    let digits = value.abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if value < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> SiteFacts {
        SiteFacts {
            puzzles: 3_128_032,
            themes: 73,
            base_url: "https://example.org".to_string(),
        }
    }

    #[test]
    fn groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(3_128_032), "3,128,032");
        assert_eq!(thousands(-1_234), "-1,234");
    }

    #[test]
    fn the_landing_page_quotes_the_live_dataset() {
        let html = landing(&facts());
        assert!(html.contains("3,128,032"));
        assert!(html.contains("73"));
        assert!(html.contains("https://example.org/v1/puzzles/random"));
        // Attribution is a licence obligation in spirit, not an afterthought.
        assert!(html.contains("database.lichess.org"));
        assert!(html.contains("not affiliated"));
    }

    #[test]
    fn llms_txt_follows_the_convention() {
        let text = llms_txt(&facts());
        let mut lines = text.lines();
        assert_eq!(lines.next().unwrap(), "# chess-puzzle-api");
        assert_eq!(lines.next().unwrap(), "");
        assert!(
            lines.next().unwrap().starts_with('>'),
            "a blockquote summary must follow the H1"
        );
        assert!(text.contains("## Endpoints"));
    }

    #[test]
    fn every_link_points_where_its_text_says() {
        let text = llms_txt(&facts());
        for line in text.lines().filter(|line| line.starts_with("- [")) {
            let label = line[3..].split(']').next().unwrap();
            let href = line.split("](").nth(1).unwrap().split(')').next().unwrap();
            // `{id}` stands in for a real id in the link, so compare the stem.
            let stem = label.split("/{").next().unwrap();
            assert!(href.starts_with(stem), "link text {label} points at {href}");
        }
    }

    #[test]
    fn llms_txt_explains_the_trap_in_the_data() {
        let text = llms_txt(&facts());
        // A model that misses this will present the wrong position to a user.
        assert!(text.contains("BEFORE the opponent's move"));
        assert!(text.contains("positionFen"));
    }
}
