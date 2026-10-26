//! The system clock, behind the `Clock` port.

use chrono::{DateTime, Utc};

use crate::domain::clock::Clock;

/// Reads the operating system's clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
