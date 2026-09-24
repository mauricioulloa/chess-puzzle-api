"""
End-to-end verification of a running deployment.

Checks properties rather than status codes. The chess claims are re-derived
with python-chess, an implementation entirely separate from the shakmaty the
service uses, so agreement between them means something.

    pip install chess
    python scripts/verify_production.py <api-key>

BASE_URL overrides the target. The key needs a high rate limit: the suite
makes several hundred requests, well past the anonymous allowance. Mint one
for the run and revoke it afterwards:

    fly ssh console -C "/usr/local/bin/chess-puzzle-api keys create --label verification --rate-limit 3000"
    fly ssh console -C "/usr/local/bin/chess-puzzle-api keys revoke --label verification"
"""
import json, os, sys, urllib.request
import chess

BASE = os.environ.get("BASE_URL", "https://chess.mauriulloa.com").rstrip("/")
KEY = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("API_KEY", "")

PASS, FAIL = [], []
def check(name, ok, detail=""):
    (PASS if ok else FAIL).append(name)
    print(f"  {'PASS' if ok else 'FAIL'}  {name}" + (f"  — {detail}" if detail and not ok else ""))

def get(path, key=True):
    req = urllib.request.Request(BASE + path)
    if key:
        req.add_header("Authorization", f"Bearer {KEY}")
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.status, json.loads(r.read()), dict(r.headers)

def status_of(path, key=True):
    try:
        s, _, _ = get(path, key)
        return s
    except urllib.error.HTTPError as e:
        return e.code

def rpc(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(BASE + "/mcp", data=body, method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Accept", "application/json, text/event-stream")
    req.add_header("Authorization", f"Bearer {KEY}")
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read())

print("\n=== 1. chess correctness, re-derived with python-chess ===")
SAMPLE = 120
bad_position, bad_san, bad_legal, bad_colour, bad_pieces = [], [], [], [], []
ratings_seen = []
for i in range(SAMPLE):
    _, p, _ = get("/v1/puzzles/random")
    ratings_seen.append(p["rating"])

    board = chess.Board(p["fen"])
    mv = chess.Move.from_uci(p["initialMove"])
    if mv not in board.legal_moves:
        bad_legal.append(p["id"]); continue
    san = board.san(mv)
    if san != p["initialMoveSan"]:
        bad_san.append((p["id"], san, p["initialMoveSan"]))
    board.push(mv)
    if board.fen() != p["positionFen"]:
        bad_position.append((p["id"], board.fen(), p["positionFen"]))
    expected_colour = "white" if board.turn == chess.WHITE else "black"
    if expected_colour != p["solverColor"]:
        bad_colour.append(p["id"])
    if len(board.piece_map()) != p["pieces"]:
        bad_pieces.append((p["id"], len(board.piece_map()), p["pieces"]))

check(f"initialMove is legal in fen ({SAMPLE} puzzles)", not bad_legal, str(bad_legal[:3]))
check("initialMoveSan matches an independent engine", not bad_san, str(bad_san[:2]))
check("positionFen == fen with initialMove applied", not bad_position, str(bad_position[:1]))
check("solverColor is the side to move in positionFen", not bad_colour, str(bad_colour[:3]))
check("pieces counts the board the player solves", not bad_pieces, str(bad_pieces[:3]))

print("\n=== 2. solutions are real, playable lines ===")
bad_solution, checked = [], 0
for i in range(40):
    _, p, _ = get("/v1/puzzles/random")
    _, s, _ = get(f"/v1/puzzles/{p['id']}/solution")
    board = chess.Board(p["positionFen"])
    ok = True
    if len(s["solution"]) != len(s["solutionSan"]):
        ok = False
    for uci, expect_san in zip(s["solution"], s["solutionSan"]):
        mv = chess.Move.from_uci(uci)
        if mv not in board.legal_moves:
            ok = False; break
        if board.san(mv) != expect_san:
            ok = False; break
        board.push(mv)
    checked += 1
    if not ok:
        bad_solution.append(p["id"])
check(f"every solution move is legal and its SAN matches ({checked} solutions)",
      not bad_solution, str(bad_solution[:3]))

print("\n=== 3. the answer never leaks ===")
# `initialMove` and `initialMoveSan` are published on purpose: without the
# opponent's move a client cannot reach the position. Everything else must be
# free of the solution.
#
# Comparing SAN across the whole response would cry wolf, because SAN is not a
# unique name for a move — it is relative to the position it is read from. In
# about 4% of puzzles a move in the solution is written the same as the
# opponent's move while being an entirely different move. UCI is compared
# instead, and the fields that legitimately carry the opponent's move are
# excluded.
leaks, overlaps = [], []
for i in range(30):
    _, p, _ = get("/v1/puzzles/random")
    _, s, _ = get(f"/v1/puzzles/{p['id']}/solution")
    if s["solution"] and s["solution"][0] == p["initialMove"]:
        overlaps.append(p["id"])
    elsewhere = json.dumps({k: v for k, v in p.items()
                            if k not in ("initialMove", "initialMoveSan")})
    for move in s["solution"] + s["solutionSan"]:
        if move in elsewhere:
            leaks.append((p["id"], move))
check("no solution move appears outside the opponent-move fields", not leaks, str(leaks[:3]))
check("no solution begins with the opponent's move", not overlaps, str(overlaps[:3]))
_, batch, _ = get("/v1/puzzles/random?count=25")
check("nor does a batch of 25 carry a solution field",
      "solution" not in json.dumps(batch).lower())

print("\n=== 4. filters are obeyed, not approximated ===")
off_band, missing_theme, not_excluded = [], [], []
for i in range(40):
    _, p, _ = get("/v1/puzzles/random?ratingMin=1600&ratingMax=1800&themes=fork&excludeThemes=endgame")
    if not 1600 <= p["rating"] <= 1800: off_band.append((p["id"], p["rating"]))
    if "fork" not in p["themes"]: missing_theme.append(p["id"])
    if "endgame" in p["themes"]: not_excluded.append(p["id"])
check("rating stays inside the requested band (40 draws)", not off_band, str(off_band[:3]))
check("the required theme is always present", not missing_theme, str(missing_theme[:3]))
check("the excluded theme never appears", not not_excluded, str(not_excluded[:3]))

_, any_mode, _ = get("/v1/puzzles/random?themes=underPromotion,enPassant&themesMode=any&count=30")
ok_any = all(("underPromotion" in q["themes"] or "enPassant" in q["themes"])
             for q in any_mode["puzzles"])
check("themesMode=any returns puzzles carrying at least one", ok_any)

crowded, miscounted = [], []
for i in range(20):
    _, p, _ = get("/v1/puzzles/random?themes=fork&maxPieces=10")
    if p["pieces"] > 10: crowded.append((p["id"], p["pieces"]))
    if len(chess.Board(p["positionFen"]).piece_map()) != p["pieces"]:
        miscounted.append(p["id"])
check("maxPieces is a ceiling, not a suggestion (20 draws)", not crowded, str(crowded[:3]))
check("and the count it filters on is the real one", not miscounted, str(miscounted[:3]))

print("\n=== 5. randomness is real ===")
ids = []
for i in range(60):
    _, p, _ = get("/v1/puzzles/random")
    ids.append(p["id"])
distinct = len(set(ids))
check(f"60 unfiltered draws gave {distinct} distinct puzzles", distinct >= 58)
spread = max(ratings_seen) - min(ratings_seen)
check(f"ratings span {spread} points across the sample", spread > 1000)
_, b, _ = get("/v1/puzzles/random?count=50")
bids = [q["id"] for q in b["puzzles"]]
check("a batch of 50 has no repeats", len(set(bids)) == len(bids) == 50)

print("\n=== 6. errors behave ===")
for path, expect, label in [
    ("/v1/puzzles/random?themes=frok", 400, "unknown theme"),
    ("/v1/puzzles/random?typo=1", 400, "unknown parameter"),
    ("/v1/puzzles/random?count=99", 400, "count over the cap"),
    ("/v1/puzzles/random?rating=9999", 400, "rating out of range"),
    ("/v1/puzzles/random?maxPieces=1", 400, "maxPieces below two kings"),
    ("/v1/puzzles/random?maxPieces=2", 404, "maxPieces nothing can meet"),
    ("/v1/puzzles/random?ratingMin=3900&ratingMax=4000", 404, "filter matches nothing"),
    ("/v1/puzzles/notanid", 404, "unknown puzzle id"),
]:
    check(f"{label} -> {expect}", status_of(path) == expect)

print("\n=== 7. MCP agrees with HTTP ===")
tools = {t["name"] for t in rpc("tools/list", {})["result"]["tools"]}
check("all five tools advertised", tools == {
    "get_random_puzzle", "get_puzzle", "get_solution", "list_themes", "get_dataset_stats"},
    str(tools))
_, http_p, _ = get("/v1/puzzles/00008")
mcp_p = rpc("tools/call", {"name": "get_puzzle", "arguments": {"puzzle_id": "00008"}})["result"]["structuredContent"]
check("MCP and HTTP describe the same position for the same id",
      mcp_p["position_fen"] == http_p["positionFen"])
check("MCP omits the solution too", "solution" not in json.dumps(mcp_p).lower())
mcp_s = rpc("tools/call", {"name": "get_solution", "arguments": {"puzzle_id": "00008"}})["result"]["structuredContent"]
_, http_s, _ = get("/v1/puzzles/00008/solution")
check("MCP and HTTP give the same solution", mcp_s["solution_san"] == http_s["solutionSan"])
mcp_themes = rpc("tools/call", {"name": "list_themes", "arguments": {}})["result"]["structuredContent"]
check("MCP themes carry descriptions",
      all(t.get("description") for t in mcp_themes["themes"]))

print("\n=== 8. consistency across endpoints ===")
_, health, _ = get("/health")
_, stats, _ = get("/v1/stats")
_, themes, _ = get("/v1/themes")
check("health and stats report the same puzzle count",
      health["puzzles"] == stats["puzzles"], f"{health['puzzles']} vs {stats['puzzles']}")
check("themes endpoint and stats agree on theme count",
      themes["count"] == stats["themes"])
check("every theme has at least one puzzle",
      all(t["puzzleCount"] > 0 for t in themes["themes"]))
check("rating distribution sums to the total",
      sum(b["count"] for b in stats["ratingDistribution"]) == stats["puzzles"])
check("health reports the schema this binary expects",
      health["status"] == "ok" and health["schemaVersion"] == health["expectedSchemaVersion"],
      str(health))
check("every theme says what it means",
      all(t.get("description") for t in themes["themes"]))
check("provenance figures are numbers",
      all(isinstance(stats["source"].get(k), int)
          for k in ("sourceRows", "minPopularity", "minPlays")), str(stats["source"]))

print("\n=== 9. rate limiting and headers ===")
_, _, h = get("/v1/puzzles/random")
check("X-RateLimit headers present", all(k in h for k in
      ("x-ratelimit-limit", "x-ratelimit-remaining", "x-ratelimit-reset")))
_, _, anon = get("/v1/themes", key=False)
check("the key's higher limit is in effect",
      int(h["x-ratelimit-limit"]) > int(anon["x-ratelimit-limit"]),
      f'{h["x-ratelimit-limit"]} vs anonymous {anon["x-ratelimit-limit"]}')
check("scope reports the key", h["x-ratelimit-scope"] == "key")
try:
    urllib.request.urlopen(urllib.request.Request(
        BASE + "/v1/puzzles/random", headers={"Authorization": "Bearer cpa_notarealkey"}), timeout=20)
    check("an invalid key is rejected", False)
except urllib.error.HTTPError as e:
    check("an invalid key is rejected with 401", e.code == 401)

print("\n=== 10. discovery surfaces ===")
for path, needle in [("/", "1000–1399"), ("/llms.txt", "BEFORE the opponent"),
                     ("/docs", "<script"), ("/openapi.json", "openapi")]:
    req = urllib.request.Request(BASE + path)
    req.add_header("Authorization", f"Bearer {KEY}")
    with urllib.request.urlopen(req, timeout=30) as r:
        body = r.read().decode("utf-8", "replace")
    check(f"{path} serves its content", needle in body)
    if path in ("/", "/llms.txt"):
        check(f"{path} invites feedback", "issues" in body)
        check(f"{path} credits Lichess", "lichess" in body.lower())

print(f"\n{'='*58}")
print(f"  {len(PASS)} passed, {len(FAIL)} failed")
if FAIL:
    for f in FAIL: print(f"    FAILED: {f}")
print(f"{'='*58}")
sys.exit(1 if FAIL else 0)
