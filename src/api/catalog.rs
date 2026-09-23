//! The theme catalogue, loaded once at startup.
//!
//! Theme ids are assigned by the importer in the order themes are first seen,
//! so they are stable only within a given database file. Everything that maps
//! a theme name to a bit therefore reads the mapping out of the database
//! rather than hard-coding it.

use crate::import::parse::ThemeMask;
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Theme {
    pub id: i64,
    pub name: String,
    pub puzzle_count: i64,
}

#[derive(Debug)]
pub struct Catalog {
    by_name: HashMap<String, Theme>,
    ordered: Vec<Theme>,
    pub puzzle_count: i64,
}

impl Catalog {
    pub fn load(conn: &Connection) -> Result<Self> {
        let mut statement = conn
            .prepare("SELECT id, name, puzzle_count FROM themes ORDER BY puzzle_count DESC")
            .context("preparing theme query")?;

        let ordered: Vec<Theme> = statement
            .query_map([], |row| {
                Ok(Theme {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    puzzle_count: row.get(2)?,
                })
            })
            .context("loading themes")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("reading themes")?;

        let by_name = ordered
            .iter()
            .map(|theme| (theme.name.to_ascii_lowercase(), theme.clone()))
            .collect();

        let puzzle_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM puzzles", [], |row| row.get(0))
            .context("counting puzzles")?;

        Ok(Self {
            by_name,
            ordered,
            puzzle_count,
        })
    }

    /// Theme names are matched case-insensitively so `mateIn2`, `matein2` and
    /// `MateIn2` all work; the canonical spelling comes back in responses.
    pub fn get(&self, name: &str) -> Option<&Theme> {
        self.by_name.get(&name.to_ascii_lowercase())
    }

    /// Maps names to theme ids, or returns every name that is not a theme.
    pub fn resolve<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Vec<i64>, Vec<&'a str>> {
        let mut ids = Vec::new();
        let mut unknown = Vec::new();
        for name in names {
            match self.get(name) {
                Some(theme) => ids.push(theme.id),
                None => unknown.push(name),
            }
        }
        if unknown.is_empty() {
            Ok(ids)
        } else {
            Err(unknown)
        }
    }

    pub fn name_of(&self, id: i64) -> Option<&str> {
        self.ordered
            .iter()
            .find(|theme| theme.id == id)
            .map(|theme| theme.name.as_str())
    }

    pub fn all(&self) -> &[Theme] {
        &self.ordered
    }

    pub fn len(&self) -> usize {
        self.ordered.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty()
    }

    /// Expands a mask back into the canonical theme names it represents.
    pub fn names_for(&self, mask: ThemeMask) -> Vec<&str> {
        self.ordered
            .iter()
            .filter(|theme| mask.contains(theme.id))
            .map(|theme| theme.name.as_str())
            .collect()
    }
}
