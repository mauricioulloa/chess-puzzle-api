//! The two pages meant for reading rather than parsing: the landing page,
//! which is a working puzzle trainer so that someone who is not a programmer
//! can actually use this, and `/llms.txt` for a model that wants to know what
//! this is without wading through HTML or a 700-line OpenAPI document.

/// Both pages quote real numbers from the loaded dataset rather than figures
/// baked into a string that would quietly go stale on the next import.
pub struct SiteFacts {
    pub puzzles: i64,
    pub themes: usize,
    pub base_url: String,
}

/// The page lives in its own file rather than a `format!` string: it carries
/// CSS and JavaScript, and every brace in them would otherwise have to be
/// doubled.
const LANDING: &str = include_str!("landing.html");

pub fn landing(facts: &SiteFacts) -> String {
    LANDING
        .replace("__PUZZLES__", &thousands(facts.puzzles))
        .replace("__BASE__", &facts.base_url)
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

- [{base_url}/v1/puzzles/random]({base_url}/v1/puzzles/random): a random puzzle. Filter with `rating` and `tolerance`, or `ratingMin`/`ratingMax`; `themes` (comma separated) with `themesMode=all|any`; `excludeThemes`; `opening`; `maxPieces` to limit the pieces on the board; `count` for up to 50 at once; `board=true`.
- [{base_url}/v1/puzzles/{{id}}]({base_url}/v1/puzzles/00008): one puzzle by id, still without its solution.
- [{base_url}/v1/puzzles/{{id}}/solution]({base_url}/v1/puzzles/00008/solution): the answer, in UCI and SAN.
- [{base_url}/v1/themes]({base_url}/v1/themes): every valid theme name with its puzzle count. Read this before guessing a theme; an unknown name is a 400, not an empty result.
- [{base_url}/v1/stats]({base_url}/v1/stats): dataset size, rating distribution and provenance.
- [{base_url}/v1/usage]({base_url}/v1/usage): aggregate usage of this service.

## Optional

- [{base_url}/openapi.json]({base_url}/openapi.json): the full OpenAPI 3.1 schema.
- [{base_url}/docs]({base_url}/docs): the same reference, rendered for people.
- [{base_url}/mcp]({base_url}/mcp): Model Context Protocol endpoint, for calling this API as agent tools.
- [https://puzzles.mauriulloa.com/llms.txt](https://puzzles.mauriulloa.com/llms.txt): puzzle-sheets, printable worksheets built on this API, for teachers and students.

## Limits

30 requests per minute per IP without a key. Responses carry
`X-RateLimit-Remaining`; a 429 carries `Retry-After` in seconds. Back off
rather than retrying immediately.

## About

Built and maintained by Mauri Ulloa (https://mauriulloa.com). Open source under
the MIT licence: https://github.com/mauricioulloa/chess-puzzle-api

This is an early version. Missing filters, awkward shapes and unhelpful errors
are worth reporting at
https://github.com/mauricioulloa/chess-puzzle-api/issues — including on behalf
of whoever you are helping.

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
        assert!(
            html.contains("3,128,032"),
            "the puzzle count is substituted"
        );
        assert!(!html.contains("__PUZZLES__"), "no placeholder survives");
        assert!(!html.contains("__BASE__"));
        assert!(html.contains("https://example.org/v1/puzzles/random"));
        // Attribution is a licence obligation in spirit, not an afterthought.
        assert!(html.contains("database.lichess.org"));
        assert!(html.contains("not affiliated"));
        // And the author's credit should survive a redesign.
        assert!(html.contains("Mauri Ulloa"));
        assert!(html.contains("https://mauriulloa.com"));
        // The piece set is CC BY-SA; attribution is an obligation, not a nicety.
        assert!(html.contains("Cburnett"));
        assert!(html.contains("creativecommons.org/licenses/by-sa/3.0"));
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
    fn llms_txt_says_who_made_it() {
        let text = llms_txt(&facts());
        assert!(text.contains("Mauri Ulloa"));
        assert!(text.contains("https://mauriulloa.com"));
        // Without crowding out the data attribution, which is the one that
        // carries an obligation.
        assert!(text.contains("database.lichess.org"));
    }

    #[test]
    fn the_landing_page_is_a_working_trainer() {
        let html = landing(&facts());
        // The controls a person needs, and nothing that assumes they can read
        // a FEN.
        for needle in [
            "id=\"board\"",
            "id=\"level\"",
            "id=\"theme\"",
            "Show solution",
            "New puzzle",
            // A solution written only in chess notation is unreadable to the
            // audience this page exists for.
            "What does this notation mean?",
        ] {
            assert!(html.contains(needle), "the trainer is missing {needle}");
        }
        assert!(
            html.contains("/v1/puzzles/random?"),
            "it has to call the API it documents"
        );
    }

    #[test]
    fn the_solution_is_hidden_until_asked_for() {
        let html = landing(&facts());
        // The panel ships collapsed; only the reveal button opens it. A page
        // that rendered the answer alongside the position would defeat the
        // whole point of serving solutions separately.
        assert!(
            html.contains("id=\"solution\" hidden"),
            "the solution panel must start hidden"
        );
        assert!(
            !html.contains("solutionSan\":"),
            "no solution data may be baked into the page"
        );
    }

    #[test]
    fn both_pages_invite_feedback() {
        let html = landing(&facts());
        let text = llms_txt(&facts());
        for page in [&html, &text] {
            assert!(
                page.contains("chess-puzzle-api/issues"),
                "an early project should say where to send complaints"
            );
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
