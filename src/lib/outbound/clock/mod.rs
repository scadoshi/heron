//! The system clock.

use crate::domain::clock::Clock;
use chrono::{DateTime, Utc};

/// Reads the operating system's clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
