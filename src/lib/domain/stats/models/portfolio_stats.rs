use crate::domain::stats::models::{
    repo_name::RepoName,
    repo_stats::{RepoReport, WeekCommits},
};
#[cfg(test)]
use chrono::Datelike;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

/// Sums across the repositories that resolved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[allow(missing_docs)]
pub struct Totals {
    pub repos: u32,
    pub commits: u64,
    pub stars: u64,
    /// Commits per week across every repository that has its weeks, oldest
    /// first. Empty when none does.
    pub weekly_commits: Vec<WeekCommits>,
}

impl Totals {
    /// Sums `reports`, saturating. Weeks are summed by their start, so a
    /// repository still computing its weeks simply adds nothing.
    pub fn of(reports: &[RepoReport]) -> Self {
        let mut by_week: BTreeMap<DateTime<Utc>, u32> = BTreeMap::new();
        for week in reports
            .iter()
            .filter_map(|report| report.stats.weekly_commits.as_deref())
            .flatten()
        {
            let total = by_week.entry(week.week).or_default();
            *total = total.saturating_add(week.commits);
        }
        let mut totals = reports.iter().fold(Self::default(), |totals, report| Self {
            repos: totals.repos.saturating_add(1),
            commits: totals.commits.saturating_add(report.stats.commits),
            stars: totals.stars.saturating_add(u64::from(report.stats.stars)),
            weekly_commits: Vec::new(),
        });
        totals.weekly_commits = by_week
            .into_iter()
            .map(|(week, commits)| WeekCommits { week, commits })
            .collect();
        totals
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{repo, stats_for};
    use chrono::TimeZone;

    #[test]
    fn weekly_commits_are_summed_by_week_and_a_repository_without_them_adds_nothing() {
        let sunday = |d| Utc.with_ymd_and_hms(2026, 9, d, 0, 0, 0).unwrap();
        let report = |name: &str, weeks: Option<Vec<(u32, u32)>>| {
            let mut stats = stats_for(&repo(name), 1);
            stats.weekly_commits = weeks.map(|weeks| {
                weeks
                    .into_iter()
                    .map(|(d, commits)| WeekCommits {
                        week: sunday(d),
                        commits,
                    })
                    .collect()
            });
            RepoReport {
                stats,
                fetched_at: sunday(27),
                stale: false,
            }
        };
        let totals = Totals::of(&[
            report("a/one", Some(vec![(13, 2), (20, 3)])),
            report("a/two", Some(vec![(20, 4), (27, 1)])),
            report("a/three", None),
        ]);
        assert_eq!(totals.repos, 3);
        let weeks: Vec<(u32, u32)> = totals
            .weekly_commits
            .iter()
            .map(|week| (week.week.day(), week.commits))
            .collect();
        assert_eq!(weeks, [(13, 2), (20, 7), (27, 1)]);
    }
}
