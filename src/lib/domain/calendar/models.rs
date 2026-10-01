use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One day of the calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Day {
    /// The day, in the account's time zone as GitHub reports it.
    pub date: NaiveDate,
    /// Contributions that day: commits, issues, pull requests and reviews.
    pub count: u32,
    /// GitHub's shade for the day, 0 for none through 4 for the top quartile.
    pub level: u8,
}

/// The last year of contributions for one account, oldest day first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Calendar {
    /// The account.
    pub login: String,
    /// Contributions over the whole year.
    pub total: u32,
    /// Every day of the year, oldest first, without gaps.
    pub days: Vec<Day>,
}

/// The calendar as the service hands it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarReport {
    /// The calendar.
    pub calendar: Calendar,
    /// When it was read from GitHub.
    pub fetched_at: DateTime<Utc>,
    /// True when the freshness window has passed and a refresh failed.
    pub stale: bool,
}

/// Why the calendar could not be served.
#[derive(Debug, Error)]
pub enum CalendarError {
    /// GitHub's GraphQL API answers nothing without a token.
    #[error("the contribution calendar needs a github token")]
    NoToken,
    /// GitHub's rate limit is spent.
    #[error("github rate limit exhausted")]
    RateLimited {
        /// When the limit resets, if GitHub said.
        reset_at: Option<DateTime<Utc>>,
    },
    /// Anything else that went wrong talking to GitHub.
    #[error("github request failed: {0:#}")]
    Upstream(anyhow::Error),
}
