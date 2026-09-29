use crate::domain::stats::models::repo_name::RepoName;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Bytes of one language in a repository, as GitHub's linguist counts them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct Language {
    pub name: String,
    pub bytes: u64,
}

/// What GitHub reports for one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoStats {
    /// The repository these numbers describe.
    pub repo: RepoName,
    /// Commits reachable from the default branch.
    pub commits: u64,
    /// Stargazers.
    pub stars: u32,
    /// Name of the default branch.
    pub default_branch: String,
    /// Last push to any branch. `None` for a repository that has never been pushed to.
    pub pushed_at: Option<DateTime<Utc>>,
    /// Sorted by bytes, largest first.
    pub languages: Vec<Language>,
    /// Lines added across all commits, every rewrite counted again. `None` while
    /// GitHub is still computing contributor statistics.
    pub additions: Option<u64>,
    /// Lines deleted across all commits. `None` on the same condition as `additions`.
    pub deletions: Option<u64>,
}

/// One repository's stats as the service hands them out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoReport {
    /// The numbers.
    pub stats: RepoStats,
    /// When they were read from GitHub.
    pub fetched_at: DateTime<Utc>,
    /// True when the freshness window has passed and a refresh failed.
    pub stale: bool,
}
