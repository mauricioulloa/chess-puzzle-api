//! Keeps the committed `openapi.json` honest.
//!
//! The spec is generated from the handler annotations, so it cannot describe a
//! route that does not exist. What it *can* do is go stale in the repository
//! after someone changes a handler and forgets to regenerate, which is what
//! these tests catch.

use chess_puzzle_api::api::docs;

const COMMITTED: &str = include_str!("../openapi.json");

/// Every route the router actually serves.
const ROUTES: &[&str] = &[
    "/v1/puzzles/random",
    "/v1/puzzles/{id}",
    "/v1/puzzles/{id}/solution",
    "/v1/themes",
    "/v1/stats",
    "/v1/usage",
    "/health",
];

#[test]
fn the_committed_spec_is_up_to_date() {
    let generated = docs::spec_json().expect("spec generates");
    assert_eq!(
        generated.trim(),
        COMMITTED.trim(),
        "openapi.json is stale; regenerate it with `cargo run -- openapi > openapi.json`"
    );
}

#[test]
fn every_route_is_documented() {
    let spec: serde_json::Value =
        serde_json::from_str(&docs::spec_json().expect("spec")).expect("valid json");
    let paths = spec["paths"].as_object().expect("paths object");

    for route in ROUTES {
        assert!(
            paths.contains_key(*route),
            "{route} is missing from the spec"
        );
    }
    assert_eq!(
        paths.len(),
        ROUTES.len(),
        "the spec documents a route the router does not serve, or vice versa"
    );
}

#[test]
fn only_the_solution_endpoint_returns_a_solution() {
    let spec: serde_json::Value =
        serde_json::from_str(&docs::spec_json().expect("spec")).expect("valid json");
    let paths = spec["paths"].as_object().expect("paths");

    for (route, item) in paths {
        let describes_solution = item.to_string().contains("SolutionResponse");
        assert_eq!(
            describes_solution,
            route.ends_with("/solution"),
            "{route} should not describe a solution response"
        );
    }
}

#[test]
fn the_puzzle_schema_never_carries_the_answer() {
    let spec: serde_json::Value =
        serde_json::from_str(&docs::spec_json().expect("spec")).expect("valid json");
    let puzzle = &spec["components"]["schemas"]["PuzzleResponse"]["properties"];
    let properties = puzzle.as_object().expect("properties");

    for forbidden in ["solution", "moves"] {
        assert!(
            !properties.contains_key(forbidden),
            "PuzzleResponse must not expose `{forbidden}`"
        );
    }
    // The opponent's move is not the answer and is required to use the puzzle.
    assert!(properties.contains_key("initialMove"));
}

#[test]
fn error_responses_are_documented_for_every_route() {
    let spec: serde_json::Value =
        serde_json::from_str(&docs::spec_json().expect("spec")).expect("valid json");
    let paths = spec["paths"].as_object().expect("paths");

    for (route, item) in paths {
        if route == "/health" {
            continue; // deliberately outside the rate limiter
        }
        let responses = &item["get"]["responses"];

        // Everything behind the limiter can be throttled, and can be called
        // with a bad key.
        for status in ["401", "429"] {
            assert!(
                !responses[status].is_null(),
                "{route} does not document a {status}"
            );
        }

        // A 400 is only reachable where there is something to get wrong.
        // /v1/usage, /v1/themes and /v1/stats take no parameters, so claiming
        // they can reject one would be documenting a lie.
        let takes_parameters = !item["get"]["parameters"].is_null();
        assert_eq!(
            !responses["400"].is_null(),
            takes_parameters,
            "{route}: a 400 should be documented exactly when the route takes parameters"
        );
    }
}
