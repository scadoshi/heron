//! [`Snapshot`], the cached payload, and the reads and writes every service makes
//! of it.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::domain::stats::models::cache_key::CacheKey;
use crate::domain::stats::ports::Cache;

/// A value with the time it was fetched and the time it stops being fresh. This is
/// what the cache stores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot<T> {
    /// What the source answered at `fetched_at`.
    pub value: T,
    /// When the value was read from its source.
    pub fetched_at: DateTime<Utc>,
    /// The value is fresh strictly before this instant.
    pub fresh_until: DateTime<Utc>,
}

impl<T> Snapshot<T> {
    /// `value` as read at `fetched_at`, fresh for `fresh` after it. A window that
    /// runs past the end of the calendar keeps the snapshot fresh for good.
    pub fn new(value: T, fetched_at: DateTime<Utc>, fresh: TimeDelta) -> Self {
        Self {
            value,
            fetched_at,
            fresh_until: fetched_at
                .checked_add_signed(fresh)
                .unwrap_or(DateTime::<Utc>::MAX_UTC),
        }
    }

    /// Whether the value is still fresh at `now`.
    pub fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        now < self.fresh_until
    }
}

/// The snapshot under `key`. A cache error or an undecodable payload is logged and
/// is a miss.
pub async fn read<T: DeserializeOwned>(cache: &impl Cache, key: &CacheKey) -> Option<Snapshot<T>> {
    let bytes = match cache.get(key).await {
        Ok(bytes) => bytes?,
        Err(error) => {
            tracing::warn!("cache read failed for {key}, treating as a miss: {error}");
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            tracing::warn!("cached payload under {key} did not decode: {error}");
            None
        }
    }
}

/// Stores `snapshot` under `key` for `retain`. A failure is logged and swallowed.
pub async fn write<T: Serialize>(
    cache: &impl Cache,
    key: &CacheKey,
    snapshot: &Snapshot<T>,
    retain: Duration,
) {
    let bytes = match serde_json::to_vec(snapshot) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!("snapshot for {key} did not encode: {error}");
            return;
        }
    };
    if let Err(error) = cache.set(key, &bytes, retain).await {
        tracing::warn!("cache write failed for {key}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn fresh_before_the_deadline_and_stale_from_it() {
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
        let snapshot = Snapshot {
            value: (),
            fetched_at: now,
            fresh_until: now + TimeDelta::seconds(10),
        };
        assert!(snapshot.is_fresh(now + TimeDelta::seconds(9)));
        assert!(!snapshot.is_fresh(now + TimeDelta::seconds(10)));
    }

    #[test]
    fn new_is_fresh_for_the_window_after_it_was_fetched() {
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
        let snapshot = Snapshot::new((), now, TimeDelta::seconds(10));
        assert_eq!(snapshot.fetched_at, now);
        assert_eq!(snapshot.fresh_until, now + TimeDelta::seconds(10));
    }

    #[test]
    fn a_window_past_the_calendar_is_fresh_forever() {
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
        let snapshot = Snapshot::new((), now, TimeDelta::MAX);
        let a_century_later = now + TimeDelta::days(36_525);
        assert!(
            snapshot.is_fresh(a_century_later),
            "fresh_until is {}",
            snapshot.fresh_until
        );
    }
}
