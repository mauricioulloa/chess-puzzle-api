//! What each theme means, in a sentence.
//!
//! Written for this project rather than copied: Lichess's own descriptions
//! live in an AGPL repository, which does not mix cleanly with MIT. The names
//! are Lichess's; a theme missing here is simply served without a description.

const DESCRIPTIONS: &[(&str, &str)] = &[
    (
        "advancedPawn",
        "A pawn deep in enemy territory, close to promoting, is at the heart of it.",
    ),
    (
        "advantage",
        "The solution wins a clear but not yet decisive advantage.",
    ),
    (
        "anastasiaMate",
        "A knight and a rook or queen trap the king against the edge, hemmed in by its own piece.",
    ),
    (
        "arabianMate",
        "A knight and a rook mate the king in a corner.",
    ),
    (
        "attackingF2F7",
        "The attack aims at f2 or f7, the square next to the king that only the king defends.",
    ),
    (
        "attraction",
        "A sacrifice lures a piece, often the king, onto a square where it can be exploited.",
    ),
    (
        "backRankMate",
        "Mate on the back rank, where the king is shut in by its own pawns.",
    ),
    (
        "balestraMate",
        "A bishop delivers mate while a queen cuts off the king's escape squares.",
    ),
    ("bishopEndgame", "An endgame with only bishops and pawns."),
    (
        "blindSwineMate",
        "Two rooks on the seventh rank combine to mate the king.",
    ),
    (
        "bodenMate",
        "Two bishops on crossing diagonals mate a king boxed in by its own pieces.",
    ),
    (
        "capturingDefender",
        "Taking a piece that guards another leaves that piece undefended.",
    ),
    ("castling", "Castling is the key move."),
    (
        "clearance",
        "A move vacates a square, line or diagonal so another piece can use it.",
    ),
    (
        "collinearMove",
        "A piece moves along the line it already stands on, often retreating to set up the win.",
    ),
    (
        "cornerMate",
        "A king trapped in the corner is mated, typically by a rook or queen with a knight's help.",
    ),
    (
        "crushing",
        "The solution wins decisively, such as a large material gain or a forced mate.",
    ),
    (
        "defensiveMove",
        "The only move that avoids losing material or the game.",
    ),
    (
        "deflection",
        "A defender is forced away from a square or piece it was guarding.",
    ),
    (
        "discoveredAttack",
        "One piece moves out of the way and uncovers an attack by another behind it.",
    ),
    (
        "discoveredCheck",
        "Moving a piece uncovers check from the piece behind it.",
    ),
    (
        "doubleBishopMate",
        "Two bishops on adjacent diagonals deliver mate.",
    ),
    (
        "doubleCheck",
        "Two pieces give check at once, so only a king move can answer it.",
    ),
    (
        "dovetailMate",
        "A queen mates a king whose two retreat squares are blocked by its own pieces.",
    ),
    ("enPassant", "The solution involves capturing en passant."),
    (
        "endgame",
        "A puzzle from the endgame, with little material left.",
    ),
    (
        "epauletteMate",
        "Mate along a rank, with the king flanked on both sides by its own pieces.",
    ),
    (
        "equality",
        "The side to move is worse and finds the way to a draw or an equal position.",
    ),
    (
        "exposedKing",
        "The king has lost its shelter, and the attack exploits it.",
    ),
    ("fork", "One piece attacks two or more at once."),
    ("hangingPiece", "An undefended piece can simply be taken."),
    (
        "hookMate",
        "A rook, supported by a knight and a pawn, mates a king next to its own pawn.",
    ),
    (
        "interference",
        "A piece is placed between an attacker and what it defends, cutting the line.",
    ),
    (
        "intermezzo",
        "Instead of the expected move, an in-between move comes first and changes everything.",
    ),
    (
        "killBoxMate",
        "A rook next to the king, supported by a queen diagonally behind it, delivers mate.",
    ),
    (
        "kingsideAttack",
        "An attack on a king castled on the kingside.",
    ),
    ("knightEndgame", "An endgame with only knights and pawns."),
    ("long", "A solution three moves long."),
    ("master", "Taken from a game played by a titled player."),
    (
        "masterVsMaster",
        "Taken from a game between two titled players.",
    ),
    ("mate", "The solution ends in checkmate."),
    ("mateIn1", "Checkmate in one move."),
    ("mateIn2", "Checkmate in two moves."),
    ("mateIn3", "Checkmate in three moves."),
    ("mateIn4", "Checkmate in four moves."),
    ("mateIn5", "Checkmate in five moves or more."),
    ("middlegame", "A puzzle from the middlegame."),
    (
        "morphysMate",
        "A bishop mates a cornered king while a rook blocks the file beside it.",
    ),
    ("oneMove", "The whole solution is a single move."),
    ("opening", "A puzzle from the opening."),
    (
        "operaMate",
        "A rook mates on the back rank, protected by a bishop.",
    ),
    ("pawnEndgame", "An endgame with only kings and pawns."),
    (
        "pillsburysMate",
        "A rook and a bishop mate a king cornered behind its pawns.",
    ),
    (
        "pin",
        "A piece cannot move without exposing a more valuable piece, or the king, behind it.",
    ),
    ("promotion", "Promoting a pawn is the key idea."),
    ("queenEndgame", "An endgame with only queens and pawns."),
    (
        "queenRookEndgame",
        "An endgame with queens, rooks and pawns.",
    ),
    (
        "queensideAttack",
        "An attack on a king castled on the queenside.",
    ),
    (
        "quietMove",
        "The key move neither checks nor captures, yet sets up an unstoppable threat.",
    ),
    ("rookEndgame", "An endgame with only rooks and pawns."),
    (
        "sacrifice",
        "Material is given up on purpose for a bigger gain.",
    ),
    ("short", "A solution two moves long."),
    (
        "skewer",
        "A valuable piece is attacked and, when it moves, exposes a lesser one behind it.",
    ),
    (
        "smotheredMate",
        "A knight mates a king that cannot move because its own pieces surround it.",
    ),
    ("superGM", "Taken from a game between elite grandmasters."),
    (
        "swallowstailMate",
        "A queen mates a king whose two diagonal retreats are blocked by its own pieces.",
    ),
    (
        "trappedPiece",
        "A piece has no safe square left and is won.",
    ),
    (
        "triangleMate",
        "A queen and a rook form a triangle around the king to deliver mate.",
    ),
    (
        "underPromotion",
        "Promoting to a knight, bishop or rook rather than a queen is the key.",
    ),
    ("veryLong", "A solution four moves long or more."),
    (
        "vukovicMate",
        "A rook mates the king, protected by a knight, with a third piece covering escape.",
    ),
    (
        "xRayAttack",
        "A piece attacks or defends through another piece standing on the same line.",
    ),
    (
        "zugzwang",
        "The opponent is forced to move and every move worsens the position.",
    ),
];

pub fn description(theme: &str) -> Option<&'static str> {
    DESCRIPTIONS
        .iter()
        .find(|(name, _)| *name == theme)
        .map(|(_, description)| *description)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_descriptions_are_sentences() {
        for (index, (name, description)) in DESCRIPTIONS.iter().enumerate() {
            assert!(
                DESCRIPTIONS[..index].iter().all(|(other, _)| other != name),
                "{name} twice"
            );
            assert!(description.ends_with('.'), "{name}");
        }
    }

    #[test]
    fn covers_every_theme_in_the_dump() {
        // The 73 theme names in the current Lichess dump.
        assert_eq!(DESCRIPTIONS.len(), 73);
        assert!(description("fork").is_some());
        assert!(description("notATheme").is_none());
    }
}
