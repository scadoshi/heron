use crate::domain::stats::models::repo_name::RepoName;
use chrono::{DateTime, Utc};
pub use measure::{Counts, Language};
use thiserror::Error;

/// One repository's counts as the service hands them out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountsReport {
    /// The repository these numbers describe.
    pub repo: RepoName,
    /// The numbers.
    pub counts: Counts,
    /// When the source was measured.
    pub measured_at: DateTime<Utc>,
}

/// What one sweep did, for the log and the tests.
#[derive(Debug, Default)]
pub struct Sweep {
    /// Repositories measured this pass, in allowlist order.
    pub measured: Vec<RepoName>,
    /// Repositories whose measurement failed; the cached value, if any, stands.
    pub failed: Vec<(RepoName, CountsError)>,
    /// Repositories left alone because nothing was pushed since they were measured.
    pub skipped: Vec<RepoName>,
}

/// Why a repository could not be measured.
#[derive(Debug, Error)]
pub enum CountsError {
    /// The checkout is neither a Rust nor a C# repository.
    #[error("nothing to measure in {0}")]
    NotMeasurable(RepoName),
    /// The source could not be fetched or read.
    #[error("measuring failed: {0:#}")]
    Upstream(anyhow::Error),
}
