use crate::domain::stats::models::{
    repo_name::RepoName,
    repo_stats::{RepoReport, WeekCommits},
};
#[cfg(test)]
use chrono::Datelike;
use chrono::{DateTime, Utc};

/// Sums across the repositories that resolved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[allow(missing_docs)]
pub struct Totals {
    pub repos: u32,
    pub commits: u64,
    pub stars: u64,
    /// Commits per week across every repository that has its weeks, oldest
    /// first, as many weeks as the longest history. Empty when none does.
    pub weekly_commits: Vec<WeekCommits>,
}

impl Totals {
    /// Sums `reports`, saturating. Weeks are summed by position counted back
    /// from the newest, not by date: GitHub dates one repository's weeks a day
    /// off another's, and every repository's last week is the current one. The
    /// week keeps the latest date any repository gave it. A repository still
    /// computing its weeks adds nothing.
    pub fn of(reports: &[RepoReport]) -> Self {
        let histories: Vec<&[WeekCommits]> = reports
            .iter()
            .filter_map(|report| report.stats.weekly_commits.as_deref())
            .collect();
        let longest = histories.iter().map(|weeks| weeks.len()).max().unwrap_or(0);
        // Index 0 is the newest week; reversed at the end.
        let mut newest_first: Vec<Option<WeekCommits>> = vec![None; longest];
        for weeks in histories {
            for (back, week) in weeks.iter().rev().enumerate() {
                let Some(slot) = newest_first.get_mut(back) else {
                    break;
                };
                *slot = Some(match slot {
                    Some(sum) => WeekCommits {
                        week: sum.week.max(week.week),
                        commits: sum.commits.saturating_add(week.commits),
                    },
                    None => *week,
                });
            }
        }
        let mut totals = reports.iter().fold(Self::default(), |totals, report| Self {
            repos: totals.repos.saturating_add(1),
            commits: totals.commits.saturating_add(report.stats.commits),
            stars: totals.stars.saturating_add(u64::from(report.stats.stars)),
            weekly_commits: Vec::new(),
        });
        totals.weekly_commits = newest_first.into_iter().rev().flatten().collect();
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
    fn weekly_commits_are_summed_by_position_from_the_newest() {
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
        // a/two dates its weeks a day early, as GitHub does for some
        // repositories; by position its last week is still the current one,
        // and a/three, still computing, adds nothing.
        let totals = Totals::of(&[
            report("a/one", Some(vec![(13, 2), (20, 3), (27, 5)])),
            report("a/two", Some(vec![(19, 4), (26, 1)])),
            report("a/three", None),
        ]);
        assert_eq!(totals.repos, 3);
        let weeks: Vec<(u32, u32)> = totals
            .weekly_commits
            .iter()
            .map(|week| (week.week.day(), week.commits))
            .collect();
        assert_eq!(weeks, [(13, 2), (20, 7), (27, 6)]);
    }
}
