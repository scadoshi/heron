//! Time as a port, so freshness logic is testable without sleeping.

use chrono::{DateTime, Utc};

/// Source of the current time.
pub trait Clock: Clone + Send + Sync + 'static {
    /// The current instant in UTC.
    fn now(&self) -> DateTime<Utc>;
}
