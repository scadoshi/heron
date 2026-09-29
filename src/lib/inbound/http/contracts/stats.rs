use super::rfc3339;
use crate::domain::stats::models::{
    portfolio_stats::{PortfolioStats, Totals},
    repo_stats::{Language, RepoReport},
};
use chrono::{DateTime, Utc};
use serde::Serialize;

/// One language of a repository.
#[derive(Debug, Serialize)]
#[allow(missing_docs)]
pub struct HttpLanguage {
    pub name: String,
    pub bytes: u64,
}

impl From<Language> for HttpLanguage {
    fn from(language: Language) -> Self {
        Self {
            name: language.name,
            bytes: language.bytes,
        }
    }
}

/// Body of `GET /stats/{owner}/{name}`, and one entry of `GET /stats`.
#[derive(Debug, Serialize)]
pub struct HttpRepoStats {
    /// `owner/name`.
    pub repo: String,
    /// Commits reachable from the default branch.
    pub commits: u64,
    /// Stargazers.
    pub stars: u32,
    /// Name of the default branch.
    pub default_branch: String,
    /// Last push to any branch. `null` for a repository never pushed to.
    #[serde(serialize_with = "rfc3339::option::serialize")]
    pub pushed_at: Option<DateTime<Utc>>,
    /// Largest first.
    pub languages: Vec<HttpLanguage>,
    /// Lines added across all commits, every rewrite counted again. `null` while
    /// GitHub is still computing it.
    pub additions: Option<u64>,
    /// Lines deleted across all commits. `null` on the same condition.
    pub deletions: Option<u64>,
    /// When these numbers were read from GitHub.
    #[serde(serialize_with = "rfc3339::serialize")]
    pub fetched_at: DateTime<Utc>,
    /// True when the numbers are past their freshness window and GitHub could not
    /// be reached for new ones.
    pub stale: bool,
}

impl From<RepoReport> for HttpRepoStats {
    fn from(report: RepoReport) -> Self {
        Self {
            repo: report.stats.repo.to_string(),
            commits: report.stats.commits,
            stars: report.stats.stars,
            default_branch: report.stats.default_branch,
            pushed_at: report.stats.pushed_at,
            languages: report.stats.languages.into_iter().map(Into::into).collect(),
            additions: report.stats.additions,
            deletions: report.stats.deletions,
            fetched_at: report.fetched_at,
            stale: report.stale,
        }
    }
}

/// Sums over the repositories in the answer.
#[derive(Debug, Serialize)]
#[allow(missing_docs)]
pub struct HttpTotals {
    pub repos: u32,
    pub commits: u64,
    pub stars: u64,
}

impl From<Totals> for HttpTotals {
    fn from(totals: Totals) -> Self {
        Self {
            repos: totals.repos,
            commits: totals.commits,
            stars: totals.stars,
        }
    }
}

/// Body of `GET /stats`.
#[derive(Debug, Serialize)]
pub struct HttpPortfolioStats {
    /// When this answer was assembled.
    #[serde(serialize_with = "rfc3339::serialize")]
    pub generated_at: DateTime<Utc>,
    /// Sums over `repos`. Repositories in `unavailable` are not counted.
    pub totals: HttpTotals,
    /// Configured repositories that could not be read this time.
    pub unavailable: Vec<String>,
    /// In the order they are configured.
    pub repos: Vec<HttpRepoStats>,
}

impl From<PortfolioStats> for HttpPortfolioStats {
    fn from(portfolio: PortfolioStats) -> Self {
        Self {
            generated_at: portfolio.generated_at,
            totals: portfolio.totals.into(),
            unavailable: portfolio
                .unavailable
                .iter()
                .map(ToString::to_string)
                .collect(),
            repos: portfolio.repos.into_iter().map(Into::into).collect(),
        }
    }
}
