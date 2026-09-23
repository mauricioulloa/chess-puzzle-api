//! Lossless decomposition of the source columns into their stored form.

/// Lichess game URLs are uniformly `https://lichess.org/{game_id}[/black]#{ply}`.
/// Storing the constant prefix three million times is pure waste, so the URL is
/// taken apart on import and rebuilt on the way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRef {
    pub game_id: String,
    pub ply: i64,
    pub black: bool,
}

impl GameRef {
    pub fn to_url(&self) -> String {
        let side = if self.black { "/black" } else { "" };
        format!("https://lichess.org/{}{}#{}", self.game_id, side, self.ply)
    }
}

pub fn parse_game_url(url: &str) -> Option<GameRef> {
    let rest = url.strip_prefix("https://lichess.org/")?;
    let (path, ply) = rest.rsplit_once('#')?;
    let ply: i64 = ply.parse().ok()?;
    let (game_id, black) = match path.strip_suffix("/black") {
        Some(id) => (id, true),
        None => (path, false),
    };
    if game_id.is_empty() || game_id.contains('/') {
        return None;
    }
    Some(GameRef {
        game_id: game_id.to_string(),
        ply,
        black,
    })
}

/// The largest number of distinct themes the 128-bit mask can represent.
pub const MAX_THEMES: usize = 128;

/// A set of themes as two i64 halves, which is how SQLite stores integers.
/// Bit `n` corresponds to the theme with database id `n + 1`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ThemeMask {
    pub lo: i64,
    pub hi: i64,
}

impl ThemeMask {
    pub fn set(&mut self, theme_id: i64) {
        let bit = (theme_id - 1) as u32;
        if bit < 64 {
            self.lo |= 1i64 << bit;
        } else {
            self.hi |= 1i64 << (bit - 64);
        }
    }

    pub fn from_ids(ids: &[i64]) -> Self {
        let mut mask = Self::default();
        for &id in ids {
            mask.set(id);
        }
        mask
    }

    pub fn contains(&self, theme_id: i64) -> bool {
        let bit = (theme_id - 1) as u32;
        if bit < 64 {
            self.lo & (1i64 << bit) != 0
        } else {
            self.hi & (1i64 << (bit - 64)) != 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_white_to_move_url() {
        let parsed = parse_game_url("https://lichess.org/787zsVup#48").unwrap();
        assert_eq!(parsed.game_id, "787zsVup");
        assert_eq!(parsed.ply, 48);
        assert!(!parsed.black);
    }

    #[test]
    fn parses_a_black_to_move_url() {
        let parsed = parse_game_url("https://lichess.org/787zsVup/black#48").unwrap();
        assert_eq!(parsed.game_id, "787zsVup");
        assert!(parsed.black);
    }

    #[test]
    fn round_trips_both_variants() {
        for url in [
            "https://lichess.org/787zsVup#48",
            "https://lichess.org/F8M8OS71/black#32",
        ] {
            assert_eq!(parse_game_url(url).unwrap().to_url(), url);
        }
    }

    #[test]
    fn rejects_urls_it_cannot_rebuild() {
        for url in [
            "https://example.com/787zsVup#48",
            "https://lichess.org/787zsVup",
            "https://lichess.org/787zsVup#notaply",
            "https://lichess.org/#48",
        ] {
            assert!(parse_game_url(url).is_none(), "{url} should not parse");
        }
    }

    #[test]
    fn mask_covers_both_halves() {
        let mut mask = ThemeMask::default();
        mask.set(1);
        mask.set(64);
        mask.set(65);
        mask.set(128);

        assert!(mask.contains(1) && mask.contains(64));
        assert!(mask.contains(65) && mask.contains(128));
        assert!(!mask.contains(2) && !mask.contains(66));
        assert_eq!(mask.lo, 1i64 | (1i64 << 63));
        assert_eq!(mask.hi, 1i64 | (1i64 << 63));
    }
}
