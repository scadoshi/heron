use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A value with the time it was fetched and the time it stops being fresh. This is
/// what the cache stores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot<T> {
    /// The cached value.
    pub value: T,
    /// When the value was read from its source.
    pub fetched_at: DateTime<Utc>,
    /// The value is fresh strictly before this instant.
    pub fresh_until: DateTime<Utc>,
}

impl<T> Snapshot<T> {
    /// Whether the value is still fresh at `now`.
    pub fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        now < self.fresh_until
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    #[test]
    fn fresh_before_the_deadline_and_stale_from_it() {
        let now = Utc::now();
        let snapshot = Snapshot {
            value: (),
            fetched_at: now,
            fresh_until: now + TimeDelta::seconds(10),
        };
        assert!(snapshot.is_fresh(now + TimeDelta::seconds(9)));
        assert!(!snapshot.is_fresh(now + TimeDelta::seconds(10)));
    }
}
