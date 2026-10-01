use super::rfc3339;
use crate::domain::{
    calendar::models::{CalendarReport, Day},
    counts::models::CountsReport,
    stats::models::{
        portfolio_stats::{PortfolioStats, Totals},
        repo_stats::{Language, RepoReport, WeekCommits},
    },
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

/// What the repository's source contains, counted by heron from a tarball of
/// the default branch.
#[derive(Debug, Serialize)]
pub struct HttpCounts {
    /// `Rust` or `C#`.
    pub language: String,
    /// Lines in every source file, blanks and comments included.
    pub lines: u64,
    /// Test attributes, one per test function.
    pub tests: u64,
    /// Clippy lints set to warn or deny. `null` for C#.
    pub clippy_lints: Option<u64>,
    /// When the source was measured.
    #[serde(serialize_with = "rfc3339::serialize")]
    pub measured_at: DateTime<Utc>,
}

impl From<CountsReport> for HttpCounts {
    fn from(report: CountsReport) -> Self {
        Self {
            language: report.counts.language.name().to_string(),
            lines: report.counts.lines,
            tests: report.counts.tests,
            clippy_lints: report.counts.clippy_lints,
            measured_at: report.measured_at,
        }
    }
}

/// Commits in one week.
#[derive(Debug, Serialize)]
pub struct HttpWeekCommits {
    /// The Sunday the week starts on, as `YYYY-MM-DD`.
    pub week: String,
    /// Commits that week.
    pub commits: u32,
}

impl From<WeekCommits> for HttpWeekCommits {
    fn from(week: WeekCommits) -> Self {
        Self {
            week: week.week.format("%Y-%m-%d").to_string(),
            commits: week.commits,
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
    /// Source counts. `null` until the first sweep has measured the repository.
    pub counts: Option<HttpCounts>,
    /// The last 52 weeks of commits, oldest first. `null` while GitHub is still
    /// computing them.
    pub weekly_commits: Option<Vec<HttpWeekCommits>>,
}

impl HttpRepoStats {
    /// The GitHub numbers with the latest measurement beside them.
    pub fn new(report: RepoReport, counts: Option<CountsReport>) -> Self {
        Self {
            counts: counts.map(Into::into),
            ..report.into()
        }
    }
}

impl From<RepoReport> for HttpRepoStats {
    fn from(report: RepoReport) -> Self {
        Self {
            counts: None,
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
            weekly_commits: report
                .stats
                .weekly_commits
                .map(|weeks| weeks.into_iter().map(Into::into).collect()),
        }
    }
}

/// Sums over the repositories in the answer.
#[derive(Debug, Serialize)]
pub struct HttpTotals {
    /// Repositories that resolved.
    pub repos: u32,
    /// Commits across them.
    pub commits: u64,
    /// Stars across them.
    pub stars: u64,
    /// Commits per week across them, oldest first; a repository still computing
    /// its weeks adds nothing. Empty when none has them.
    pub weekly_commits: Vec<HttpWeekCommits>,
}

impl From<Totals> for HttpTotals {
    fn from(totals: Totals) -> Self {
        Self {
            repos: totals.repos,
            commits: totals.commits,
            stars: totals.stars,
            weekly_commits: totals.weekly_commits.into_iter().map(Into::into).collect(),
        }
    }
}

/// One day of the contribution calendar.
#[derive(Debug, Serialize)]
pub struct HttpDay {
    /// `YYYY-MM-DD`.
    pub date: String,
    /// Contributions that day.
    pub count: u32,
    /// 0 for none through 4 for the top quartile, GitHub's own shading.
    pub level: u8,
}

impl From<Day> for HttpDay {
    fn from(day: Day) -> Self {
        Self {
            date: day.date.to_string(),
            count: day.count,
            level: day.level,
        }
    }
}

/// A year of the account's contributions on GitHub, across every repository, as
/// the profile page draws it.
#[derive(Debug, Serialize)]
pub struct HttpCalendar {
    /// The account.
    pub login: String,
    /// Contributions over the whole year.
    pub total: u32,
    /// Every day, oldest first, without gaps.
    pub days: Vec<HttpDay>,
    /// When the calendar was read from GitHub.
    #[serde(serialize_with = "rfc3339::serialize")]
    pub fetched_at: DateTime<Utc>,
    /// True when it is past its freshness window and GitHub could not be reached.
    pub stale: bool,
}

impl From<CalendarReport> for HttpCalendar {
    fn from(report: CalendarReport) -> Self {
        Self {
            login: report.calendar.login,
            total: report.calendar.total,
            days: report.calendar.days.into_iter().map(Into::into).collect(),
            fetched_at: report.fetched_at,
            stale: report.stale,
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
    /// The account's contribution calendar. `null` when it could not be read and
    /// nothing is cached, or when there is no token.
    pub calendar: Option<HttpCalendar>,
}

impl HttpPortfolioStats {
    /// `portfolio` with `counts` laid beside each repository, in the same order,
    /// and the calendar beside them all.
    pub fn new(
        portfolio: PortfolioStats,
        counts: Vec<Option<CountsReport>>,
        calendar: Option<CalendarReport>,
    ) -> Self {
        Self {
            calendar: calendar.map(Into::into),
            generated_at: portfolio.generated_at,
            totals: portfolio.totals.into(),
            unavailable: portfolio
                .unavailable
                .iter()
                .map(ToString::to_string)
                .collect(),
            repos: portfolio
                .repos
                .into_iter()
                .zip(counts.into_iter().chain(std::iter::repeat(None)))
                .map(|(report, counts)| HttpRepoStats::new(report, counts))
                .collect(),
        }
    }
}
