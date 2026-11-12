//! Bringing a repository's weekly commit series up to the current week.
//!
//! GitHub computes the series in the background and can answer "still computing"
//! for days, so a cached series stops at the week it was last read. The commits
//! made since then are counted into the series' own weeks instead.

use chrono::{DateTime, TimeDelta, Utc};

use crate::domain::stats::models::repo_stats::WeekCommits;

/// Weeks in the series, as GitHub's commit activity has them.
pub const WEEKS_KEPT: usize = 52;

/// One week.
const WEEK: TimeDelta = TimeDelta::weeks(1);

/// Whether `weeks` (oldest first) stops before the week holding `now`. An empty
/// series has no week to start from, so it is never behind.
#[must_use]
pub fn is_behind(weeks: &[WeekCommits], now: DateTime<Utc>) -> bool {
    weeks
        .last()
        .and_then(|last| last.week.checked_add_signed(WEEK))
        .is_some_and(|next| next <= now)
}

/// `weeks` (oldest first) brought up to the week holding `now`. Its last week and
/// every week after it are counted again from `commits`, the times of the default
/// branch's commits since that last week began, and the series keeps its last
/// [`WEEKS_KEPT`] weeks. The weeks fall on the series' own dates, seven days apart,
/// since GitHub dates some repositories' weeks a day off others. An empty series
/// comes back empty.
#[must_use]
pub fn top_up(
    weeks: &[WeekCommits],
    commits: &[DateTime<Utc>],
    now: DateTime<Utc>,
) -> Vec<WeekCommits> {
    let Some((last, kept)) = weeks.split_last() else {
        return Vec::new();
    };
    let mut fresh: Vec<WeekCommits> = Vec::new();
    let mut start = last.week;
    while start <= now {
        let Some(end) = start.checked_add_signed(WEEK) else {
            break;
        };
        let counted = commits.iter().filter(|t| **t >= start && **t < end).count();
        fresh.push(WeekCommits {
            week: start,
            commits: u32::try_from(counted).unwrap_or(u32::MAX),
        });
        start = end;
    }
    let series: Vec<WeekCommits> = kept.iter().copied().chain(fresh).collect();
    let drop = series.len().saturating_sub(WEEKS_KEPT);
    series.into_iter().skip(drop).collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, month, day, hour, 0, 0).unwrap()
    }

    /// 52 weeks of one commit each, the last starting on `last`.
    fn year_to(last: DateTime<Utc>) -> Vec<WeekCommits> {
        (0_i64..52)
            .rev()
            .map(|back| WeekCommits {
                week: last.checked_sub_signed(TimeDelta::weeks(back)).unwrap(),
                commits: 1,
            })
            .collect()
    }

    #[test]
    fn a_series_from_september_is_behind_in_november_and_a_current_one_is_not() {
        let stale = year_to(at(9, 27, 0));
        assert!(is_behind(&stale, at(11, 12, 2)));
        let current = year_to(at(11, 8, 0));
        assert!(!is_behind(&current, at(11, 12, 2)));
        assert!(!is_behind(&[], at(11, 12, 2)), "no week to start from");
    }

    #[test]
    fn a_stale_series_ends_at_the_current_week_with_the_commits_since() {
        let stale = year_to(at(9, 27, 0));
        let commits = [
            at(9, 28, 9),
            at(9, 30, 9),
            at(10, 20, 9),
            at(11, 9, 1),
            at(11, 12, 1),
        ];
        let topped = top_up(&stale, &commits, at(11, 12, 2));
        assert_eq!(topped.len(), WEEKS_KEPT, "still a year of weeks");
        let tail: Vec<(DateTime<Utc>, u32)> = topped
            .iter()
            .rev()
            .take(8)
            .rev()
            .map(|w| (w.week, w.commits))
            .collect();
        assert_eq!(
            tail,
            [
                (at(9, 20, 0), 1),
                (at(9, 27, 0), 2),
                (at(10, 4, 0), 0),
                (at(10, 11, 0), 0),
                (at(10, 18, 0), 1),
                (at(10, 25, 0), 0),
                (at(11, 1, 0), 0),
                (at(11, 8, 0), 2),
            ],
            "the last known week counted again, then every week to the current one"
        );
    }

    #[test]
    fn weeks_keep_the_series_own_day() {
        // GitHub dates this repository's weeks on Saturdays.
        let stale = year_to(at(9, 26, 0));
        let topped = top_up(&stale, &[at(10, 3, 5)], at(10, 5, 0));
        let tail: Vec<(DateTime<Utc>, u32)> = topped
            .iter()
            .rev()
            .take(2)
            .rev()
            .map(|w| (w.week, w.commits))
            .collect();
        assert_eq!(tail, [(at(9, 26, 0), 0), (at(10, 3, 0), 1)]);
    }

    #[test]
    fn an_empty_series_stays_empty() {
        assert_eq!(top_up(&[], &[at(10, 3, 5)], at(11, 12, 2)), []);
    }
}
