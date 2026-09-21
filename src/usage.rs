//! Aggregate usage accounting.
//!
//! Counters, not logs. A row per request would grow without bound, write to
//! disk on every hit, and — because a request carries an IP — turn into
//! personal data with a retention policy attached. Daily counters answer every
//! question worth asking here in a few kilobytes a month.
//!
//! Nothing recorded here identifies a caller. The aggregate tables store
//! whether a request carried *a* key, never *which*; per-key totals stay in
//! `api_keys` where only the operator can read them. That is what makes the
//! endpoint publishable rather than something that has to be protected.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::Mutex;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS usage_daily (
    day      TEXT    NOT NULL,
    endpoint TEXT    NOT NULL,
    status   INTEGER NOT NULL,
    keyed    INTEGER NOT NULL,
    count    INTEGER NOT NULL,
    PRIMARY KEY (day, endpoint, status, keyed)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS filter_usage_daily (
    day       TEXT    NOT NULL,
    dimension TEXT    NOT NULL,
    value     TEXT    NOT NULL,
    count     INTEGER NOT NULL,
    PRIMARY KEY (day, dimension, value)
) WITHOUT ROWID;
";

/// What a request contributes to the request counters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestKey {
    pub day: String,
    pub endpoint: String,
    pub status: u16,
    pub keyed: bool,
}

/// What a request contributes to the filter counters, e.g. which themes were
/// asked for. Collected only for endpoints that take filters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FilterKey {
    pub day: String,
    pub dimension: String,
    pub value: String,
}

/// Dimensions, named once so the writer and the reader cannot disagree.
pub mod dimensions {
    pub const THEME: &str = "theme";
    pub const EXCLUDED_THEME: &str = "excludedTheme";
    pub const RATING_BAND: &str = "ratingBand";
    pub const OPTION: &str = "option";
}

/// What a handler noticed about one request's filters, handed to the middleware
/// through the response so handlers do not each need their own plumbing.
#[derive(Debug, Clone, Default)]
pub struct FilterSample {
    pub themes: Vec<String>,
    pub excluded_themes: Vec<String>,
    pub rating_band: Option<String>,
    pub options: Vec<&'static str>,
}

/// Buckets a requested rating for reporting. The exact number asked for is
/// noise; the band is the thing anyone wants to see.
pub fn rating_band(rating: i64) -> String {
    let floor = (rating / 200) * 200;
    format!("{floor}-{}", floor + 199)
}

pub fn today() -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string()
}

/// In-memory counters, drained to disk by the maintenance task.
#[derive(Default)]
pub struct Collector {
    requests: Mutex<HashMap<RequestKey, u64>>,
    filters: Mutex<HashMap<FilterKey, u64>>,
}

impl Collector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_request(&self, endpoint: &str, status: u16, keyed: bool) {
        let key = RequestKey {
            day: today(),
            endpoint: endpoint.to_string(),
            status,
            keyed,
        };
        *self
            .requests
            .lock()
            .expect("usage lock")
            .entry(key)
            .or_insert(0) += 1;
    }

    pub fn record_filters(&self, sample: &FilterSample) {
        let day = today();
        let mut filters = self.filters.lock().expect("usage lock");
        let mut bump = |dimension: &str, value: &str| {
            *filters
                .entry(FilterKey {
                    day: day.clone(),
                    dimension: dimension.to_string(),
                    value: value.to_string(),
                })
                .or_insert(0) += 1;
        };

        for theme in &sample.themes {
            bump(dimensions::THEME, theme);
        }
        for theme in &sample.excluded_themes {
            bump(dimensions::EXCLUDED_THEME, theme);
        }
        if let Some(band) = &sample.rating_band {
            bump(dimensions::RATING_BAND, band);
        }
        for option in &sample.options {
            bump(dimensions::OPTION, option);
        }
    }

    pub fn take(&self) -> (HashMap<RequestKey, u64>, HashMap<FilterKey, u64>) {
        (
            std::mem::take(&mut self.requests.lock().expect("usage lock")),
            std::mem::take(&mut self.filters.lock().expect("usage lock")),
        )
    }
}

pub fn flush(
    conn: &mut Connection,
    requests: &HashMap<RequestKey, u64>,
    filters: &HashMap<FilterKey, u64>,
) -> Result<()> {
    if requests.is_empty() && filters.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction().context("usage transaction")?;
    {
        let mut insert_request = tx
            .prepare_cached(
                "INSERT INTO usage_daily (day, endpoint, status, keyed, count)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (day, endpoint, status, keyed)
                 DO UPDATE SET count = count + excluded.count",
            )
            .context("preparing request counter")?;
        for (key, count) in requests {
            insert_request
                .execute((
                    &key.day,
                    &key.endpoint,
                    key.status,
                    key.keyed,
                    *count as i64,
                ))
                .context("recording requests")?;
        }

        let mut insert_filter = tx
            .prepare_cached(
                "INSERT INTO filter_usage_daily (day, dimension, value, count)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (day, dimension, value)
                 DO UPDATE SET count = count + excluded.count",
            )
            .context("preparing filter counter")?;
        for (key, count) in filters {
            insert_filter
                .execute((&key.day, &key.dimension, &key.value, *count as i64))
                .context("recording filters")?;
        }
    }
    tx.commit().context("committing usage")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    #[test]
    fn buckets_ratings_into_bands() {
        assert_eq!(rating_band(1500), "1400-1599");
        assert_eq!(rating_band(1400), "1400-1599");
        assert_eq!(rating_band(1599), "1400-1599");
        assert_eq!(rating_band(1600), "1600-1799");
        assert_eq!(rating_band(0), "0-199");
    }

    #[test]
    fn counters_accumulate_across_flushes() {
        let mut conn = store();
        let collector = Collector::new();

        for _ in 0..3 {
            collector.record_request("/v1/puzzles/random", 200, false);
        }
        let (requests, filters) = collector.take();
        flush(&mut conn, &requests, &filters).unwrap();

        collector.record_request("/v1/puzzles/random", 200, false);
        let (requests, filters) = collector.take();
        flush(&mut conn, &requests, &filters).unwrap();

        let total: i64 = conn
            .query_row("SELECT count FROM usage_daily", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            total, 4,
            "a second flush must add to the first, not replace"
        );
    }

    #[test]
    fn taking_drains_the_buffer() {
        let collector = Collector::new();
        collector.record_request("/v1/themes", 200, true);

        let (requests, _) = collector.take();
        assert_eq!(requests.len(), 1);

        let (requests, _) = collector.take();
        assert!(
            requests.is_empty(),
            "a drained buffer must not be flushed twice"
        );
    }

    #[test]
    fn anonymous_and_keyed_traffic_are_counted_apart() {
        let mut conn = store();
        let collector = Collector::new();
        collector.record_request("/v1/puzzles/random", 200, false);
        collector.record_request("/v1/puzzles/random", 200, true);

        let (requests, filters) = collector.take();
        flush(&mut conn, &requests, &filters).unwrap();

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM usage_daily", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 2, "keyed and anonymous are different buckets");
    }

    #[test]
    fn no_counter_can_identify_a_caller() {
        let mut conn = store();
        let collector = Collector::new();
        collector.record_request("/v1/puzzles/random", 200, true);
        let (requests, filters) = collector.take();
        flush(&mut conn, &requests, &filters).unwrap();

        // The schema has no column that could hold a key id, an IP or anything
        // else tied to a person. This is what makes the endpoint publishable.
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('usage_daily')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(columns, ["day", "endpoint", "status", "keyed", "count"]);
    }

    #[test]
    fn filter_dimensions_are_recorded_separately() {
        let mut conn = store();
        let collector = Collector::new();
        collector.record_filters(&FilterSample {
            themes: vec!["fork".into(), "pin".into()],
            excluded_themes: vec!["endgame".into()],
            rating_band: Some("1400-1599".into()),
            options: vec!["batch"],
        });

        let (requests, filters) = collector.take();
        flush(&mut conn, &requests, &filters).unwrap();

        let by_dimension: Vec<(String, i64)> = conn
            .prepare("SELECT dimension, SUM(count) FROM filter_usage_daily GROUP BY dimension ORDER BY dimension")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();

        assert_eq!(
            by_dimension,
            vec![
                ("excludedTheme".to_string(), 1),
                ("option".to_string(), 1),
                ("ratingBand".to_string(), 1),
                ("theme".to_string(), 2),
            ]
        );
    }
}
