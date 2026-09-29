use crate::domain::stats::models::repo_name::RepoName;
use chrono::{DateTime, Utc};
use thiserror::Error;

/// Why stats could not be served.
///
/// Cache failures are not here. The cache is an optimization, so its errors are
/// logged and never reach a caller.
#[derive(Debug, Error)]
pub enum StatsError {
    /// Not in the allowlist, or GitHub has no such repository.
    #[error("unknown repository: {0}")]
    UnknownRepo(RepoName),
    /// GitHub reports the repository as private.
    #[error("repository is private: {0}")]
    PrivateRepo(RepoName),
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

/// The cache could not be read, written, or reached.
#[derive(Debug, Error)]
#[error("cache unavailable: {0:#}")]
pub struct CacheError(pub anyhow::Error);
