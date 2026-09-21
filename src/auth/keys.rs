//! API key storage.
//!
//! Keys live in their own small read-write database, separate from the
//! read-only puzzle data. Only the SHA-256 of a key is ever stored: a leaked
//! backup of `api.db` does not hand anyone a working key, and a lost key
//! cannot be recovered, only replaced.

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// Distinguishes our keys from any other token a caller might paste in.
const PREFIX: &str = "cpa_";
const KEY_BYTES: usize = 32;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS api_keys (
    id            INTEGER PRIMARY KEY,
    key_hash      TEXT    NOT NULL UNIQUE,
    label         TEXT    NOT NULL,
    rate_limit    INTEGER NOT NULL,
    created_at    TEXT    NOT NULL,
    revoked_at    TEXT,
    last_used_at  TEXT,
    request_count INTEGER NOT NULL DEFAULT 0
);
";

/// A key as shown by `keys list`: everything except the secret.
#[derive(Debug, Clone)]
pub struct KeySummary {
    pub id: i64,
    pub label: String,
    pub rate_limit: u32,
    pub created_at: String,
    pub revoked: bool,
    pub request_count: i64,
}

#[derive(Debug, Clone)]
pub struct KeyRecord {
    pub id: i64,
    pub label: String,
    pub rate_limit: u32,
}

pub fn hash(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

fn generate_key() -> String {
    let mut bytes = [0u8; KEY_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// The read-write side of the service: API keys and the usage counters,
/// which share `api.db` because they share a lifecycle.
///
/// A single connection behind a mutex rather than a pool: a lookup is an
/// indexed point query on a table with a handful of rows, which costs a few
/// microseconds, and writes happen only when a key is minted or when buffered
/// usage is flushed.
pub struct KeyStore {
    conn: Mutex<Connection>,
}

impl KeyStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
            .context("configuring the key database")?;
        conn.execute_batch(SCHEMA).context("creating key schema")?;
        conn.execute_batch(crate::usage::SCHEMA)
            .context("creating usage schema")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("in-memory key database")?;
        conn.execute_batch(SCHEMA).context("creating key schema")?;
        conn.execute_batch(crate::usage::SCHEMA)
            .context("creating usage schema")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Mints a key. The plaintext is returned exactly once and never stored.
    pub fn create(&self, label: &str, rate_limit: u32) -> Result<String> {
        if label.trim().is_empty() {
            bail!("a key needs a label so you can tell later who it was for");
        }
        let key = generate_key();
        let conn = self.conn.lock().expect("key store lock");
        conn.execute(
            "INSERT INTO api_keys (key_hash, label, rate_limit, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            (
                hash(&key),
                label.trim(),
                rate_limit,
                jiff::Timestamp::now().to_string(),
            ),
        )
        .context("storing the new key")?;
        Ok(key)
    }

    /// Looks a key up by its plaintext. Revoked keys resolve to `None`, so
    /// revocation takes effect on the very next request.
    pub fn lookup(&self, key: &str) -> Result<Option<KeyRecord>> {
        let conn = self.conn.lock().expect("key store lock");
        conn.query_row(
            "SELECT id, label, rate_limit FROM api_keys
             WHERE key_hash = ?1 AND revoked_at IS NULL",
            (hash(key),),
            |row| {
                Ok(KeyRecord {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    rate_limit: row.get::<_, i64>(2)? as u32,
                })
            },
        )
        .optional()
        .context("looking up an API key")
    }

    pub fn revoke(&self, label: &str) -> Result<usize> {
        let conn = self.conn.lock().expect("key store lock");
        let affected = conn
            .execute(
                "UPDATE api_keys SET revoked_at = ?1
                 WHERE label = ?2 AND revoked_at IS NULL",
                (jiff::Timestamp::now().to_string(), label),
            )
            .context("revoking key")?;
        Ok(affected)
    }

    pub fn list(&self) -> Result<Vec<KeySummary>> {
        let conn = self.conn.lock().expect("key store lock");
        let mut statement = conn
            .prepare(
                "SELECT id, label, rate_limit, created_at, revoked_at IS NOT NULL, request_count
                 FROM api_keys ORDER BY id",
            )
            .context("preparing key listing")?;
        let rows = statement
            .query_map([], |row| {
                Ok(KeySummary {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    rate_limit: row.get::<_, i64>(2)? as u32,
                    created_at: row.get(3)?,
                    revoked: row.get(4)?,
                    request_count: row.get(5)?,
                })
            })
            .context("listing keys")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("reading keys")?;
        Ok(rows)
    }

    /// Drains the aggregate usage counters to disk.
    pub fn flush_stats(
        &self,
        requests: &HashMap<crate::usage::RequestKey, u64>,
        filters: &HashMap<crate::usage::FilterKey, u64>,
    ) -> Result<()> {
        let mut conn = self.conn.lock().expect("key store lock");
        crate::usage::flush(&mut conn, requests, filters)
    }

    /// Runs a read against `api.db`. Used by the usage endpoint, which reads
    /// counters that live alongside the keys.
    pub fn read<T>(&self, query: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.conn.lock().expect("key store lock");
        query(&conn)
    }

    /// Writes buffered usage counts. Called on a timer rather than per
    /// request, so a burst of traffic does not turn into a write per hit.
    pub fn flush_usage(&self, usage: &HashMap<i64, u64>) -> Result<()> {
        if usage.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().expect("key store lock");
        let now = jiff::Timestamp::now().to_string();
        let tx = conn.transaction().context("usage transaction")?;
        {
            let mut statement = tx
                .prepare_cached(
                    "UPDATE api_keys
                     SET request_count = request_count + ?2, last_used_at = ?3
                     WHERE id = ?1",
                )
                .context("preparing usage update")?;
            for (id, count) in usage {
                statement
                    .execute((id, *count as i64, &now))
                    .context("updating usage")?;
            }
        }
        tx.commit().context("committing usage")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minted_key_validates_and_a_random_string_does_not() {
        let store = KeyStore::in_memory().unwrap();
        let key = store.create("test", 600).unwrap();

        assert!(key.starts_with(PREFIX));
        let found = store.lookup(&key).unwrap().expect("key should validate");
        assert_eq!(found.label, "test");
        assert_eq!(found.rate_limit, 600);

        assert!(store.lookup("cpa_nonsense").unwrap().is_none());
    }

    #[test]
    fn the_plaintext_key_is_never_stored() {
        let store = KeyStore::in_memory().unwrap();
        let key = store.create("test", 600).unwrap();

        let conn = store.conn.lock().unwrap();
        let stored: String = conn
            .query_row("SELECT key_hash FROM api_keys", [], |row| row.get(0))
            .unwrap();
        assert_ne!(stored, key);
        assert_eq!(stored, hash(&key));
        assert!(!stored.contains(key.trim_start_matches(PREFIX)));
    }

    #[test]
    fn keys_are_distinct() {
        let store = KeyStore::in_memory().unwrap();
        let a = store.create("a", 600).unwrap();
        let b = store.create("b", 600).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn revoking_takes_effect_immediately() {
        let store = KeyStore::in_memory().unwrap();
        let key = store.create("doomed", 600).unwrap();
        assert!(store.lookup(&key).unwrap().is_some());

        assert_eq!(store.revoke("doomed").unwrap(), 1);
        assert!(store.lookup(&key).unwrap().is_none());
        assert_eq!(
            store.revoke("doomed").unwrap(),
            0,
            "revoking twice is a no-op"
        );
    }

    #[test]
    fn usage_flushes_are_additive() {
        let store = KeyStore::in_memory().unwrap();
        store.create("counted", 600).unwrap();

        store.flush_usage(&HashMap::from([(1, 5)])).unwrap();
        store.flush_usage(&HashMap::from([(1, 3)])).unwrap();

        let listed = store.list().unwrap();
        assert_eq!(listed[0].request_count, 8);
    }

    #[test]
    fn a_label_is_required() {
        let store = KeyStore::in_memory().unwrap();
        assert!(store.create("   ", 600).is_err());
    }
}
