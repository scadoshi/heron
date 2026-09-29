use crate::domain::stats::models::{repo_name::RepoName, repo_stats::RepoReport};
use chrono::{DateTime, Utc};

/// Sums across the repositories that resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(missing_docs)]
pub struct Totals {
    pub repos: u32,
    pub commits: u64,
    pub stars: u64,
}

impl Totals {
    /// Sums `reports`, saturating.
    pub fn of(reports: &[RepoReport]) -> Self {
        reports.iter().fold(Self::default(), |totals, report| Self {
            repos: totals.repos.saturating_add(1),
            commits: totals.commits.saturating_add(report.stats.commits),
            stars: totals.stars.saturating_add(u64::from(report.stats.stars)),
        })
    }
}

/// Every allowlisted repository in one answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioStats {
    /// The repositories that resolved, in allowlist order.
    pub repos: Vec<RepoReport>,
    /// Sums over `repos`. Repositories in `unavailable` are not counted.
    pub totals: Totals,
    /// Allowlisted repositories that could not be resolved this time.
    pub unavailable: Vec<RepoName>,
    /// When this answer was assembled.
    pub generated_at: DateTime<Utc>,
}
