//! In-memory fakes for the ports, shared by unit tests across modules. Compiled only
//! under `#[cfg(test)]`.

#![expect(
    missing_docs,
    clippy::arithmetic_side_effects,
    clippy::missing_panics_doc,
    reason = "test-only fakes: names carry the meaning, and an overflowing counter or a poisoned lock is a test bug worth panicking on"
)]

use std::collections::HashMap;
use std::fmt;
use std::future::{self, Future};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, TimeDelta, TimeZone, Utc};

use crate::domain::calendar::models::{Calendar, CalendarError, Day};
use crate::domain::calendar::ports::CalendarSource;
use crate::domain::clock::Clock;
use crate::domain::counts::models::{Counts, CountsError, Language};
use crate::domain::counts::ports::CountsSource;
use crate::domain::login::Login;
use crate::domain::stats::models::cache_key::CacheKey;
use crate::domain::stats::models::errors::{CacheError, StatsError};
use crate::domain::stats::models::repo_name::RepoName;
use crate::domain::stats::models::repo_stats::{LanguageBytes, RepoStats, WeekCommits};
use crate::domain::stats::ports::{Cache, StatsSource};

/// Parses a repository name known to be valid.
pub fn repo(raw: &str) -> RepoName {
    RepoName::new(raw).unwrap()
}

/// Stats with recognizable values for `repo`.
pub fn stats_for(repo: &RepoName, commits: u64) -> RepoStats {
    RepoStats {
        repo: repo.clone(),
        commits,
        stars: 3,
        default_branch: "main".to_string(),
        pushed_at: Some(Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap()),
        languages: vec![LanguageBytes {
            name: "Rust".to_string(),
            bytes: 1000,
        }],
        additions: Some(500),
        deletions: Some(100),
        weekly_commits: None,
    }
}

// == clock ==

/// A clock that moves only when told to.
#[derive(Clone)]
pub struct FakeClock(Arc<Mutex<DateTime<Utc>>>);

impl Default for FakeClock {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(
            Utc.with_ymd_and_hms(2026, 9, 29, 12, 0, 0).unwrap(),
        )))
    }
}

// Each fake keeps its state behind a mutex, which printing never locks.
impl fmt::Debug for FakeClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeClock").finish_non_exhaustive()
    }
}

impl FakeClock {
    pub fn advance(&self, by: Duration) {
        let mut now = self.0.lock().unwrap();
        *now += TimeDelta::from_std(by).unwrap();
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

// == source ==

/// How a [`FakeSource`] fails.
#[derive(Debug, Clone, Copy)]
pub enum Failure {
    Upstream,
    RateLimited,
    Private,
    Unknown,
}

impl Failure {
    fn into_error(self, repo: &RepoName) -> StatsError {
        match self {
            Self::Upstream => StatsError::Upstream(anyhow::anyhow!("connection refused")),
            Self::RateLimited => StatsError::RateLimited { reset_at: None },
            Self::Private => StatsError::PrivateRepo(repo.clone()),
            Self::Unknown => StatsError::UnknownRepo(repo.clone()),
        }
    }
}

struct SourceState {
    calls: u32,
    commits: u64,
    pushed_at: Option<DateTime<Utc>>,
    /// Whether the computed fields (churn, weekly commits) come back.
    computed: bool,
    failure: Option<Failure>,
    failing_repos: HashMap<RepoName, Failure>,
    delay: Option<Duration>,
}

/// A source that counts its calls and fails on request.
#[derive(Clone)]
pub struct FakeSource(Arc<Mutex<SourceState>>);

impl Default for FakeSource {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(SourceState {
            calls: 0,
            commits: 10,
            pushed_at: Some(Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap()),
            computed: true,
            failure: None,
            failing_repos: HashMap::new(),
            delay: None,
        })))
    }
}

impl fmt::Debug for FakeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeSource").finish_non_exhaustive()
    }
}

impl FakeSource {
    pub fn calls(&self) -> u32 {
        self.0.lock().unwrap().calls
    }

    pub fn set_commits(&self, commits: u64) {
        self.0.lock().unwrap().commits = commits;
    }

    pub fn set_pushed_at(&self, pushed_at: Option<DateTime<Utc>>) {
        self.0.lock().unwrap().pushed_at = pushed_at;
    }

    /// Whether answers carry the fields GitHub computes in the background.
    pub fn set_computed(&self, computed: bool) {
        self.0.lock().unwrap().computed = computed;
    }

    /// Every call fails from here on.
    pub fn fail_with(&self, failure: Failure) {
        self.0.lock().unwrap().failure = Some(failure);
    }

    /// Calls for `repo` fail from here on.
    pub fn fail_repo(&self, repo: &RepoName, failure: Failure) {
        self.0
            .lock()
            .unwrap()
            .failing_repos
            .insert(repo.clone(), failure);
    }

    /// Every call takes this long, so concurrent callers pile up behind it.
    pub fn set_delay(&self, delay: Duration) {
        self.0.lock().unwrap().delay = Some(delay);
    }
}

impl StatsSource for FakeSource {
    async fn fetch_repo_stats(&self, repo: &RepoName) -> Result<RepoStats, StatsError> {
        let (delay, outcome) = {
            let mut state = self.0.lock().unwrap();
            state.calls += 1;
            let failure = state.failing_repos.get(repo).copied().or(state.failure);
            let outcome = if let Some(failure) = failure {
                Err(failure.into_error(repo))
            } else {
                let mut answer = stats_for(repo, state.commits);
                answer.pushed_at = state.pushed_at;
                if state.computed {
                    answer.weekly_commits = Some(vec![WeekCommits {
                        week: Utc.with_ymd_and_hms(2026, 8, 30, 0, 0, 0).unwrap(),
                        commits: 4,
                    }]);
                } else {
                    answer.additions = None;
                    answer.deletions = None;
                    answer.weekly_commits = None;
                }
                Ok(answer)
            };
            (state.delay, outcome)
        };
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        outcome
    }
}

// == cache ==

#[derive(Default)]
struct CacheState {
    entries: HashMap<String, Vec<u8>>,
    fail_reads: bool,
    fail_writes: bool,
    fail_pings: bool,
    last_retain: Option<Duration>,
    /// Gets and sets asked of the cache, failed ones included.
    calls: u32,
}

/// A cache that never expires anything and fails on request.
#[derive(Clone, Default)]
pub struct FakeCache(Arc<Mutex<CacheState>>);

impl fmt::Debug for FakeCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeCache").finish_non_exhaustive()
    }
}

impl FakeCache {
    pub fn fail_reads(&self, fail: bool) {
        self.0.lock().unwrap().fail_reads = fail;
    }

    pub fn fail_writes(&self, fail: bool) {
        self.0.lock().unwrap().fail_writes = fail;
    }

    pub fn fail_pings(&self, fail: bool) {
        self.0.lock().unwrap().fail_pings = fail;
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap().entries.is_empty()
    }

    pub fn put(&self, key: &CacheKey, value: &[u8]) {
        self.0
            .lock()
            .unwrap()
            .entries
            .insert(key.to_string(), value.to_vec());
    }

    pub fn last_retain(&self) -> Option<Duration> {
        self.0.lock().unwrap().last_retain
    }

    pub fn calls(&self) -> u32 {
        self.0.lock().unwrap().calls
    }
}

impl Cache for FakeCache {
    fn backend(&self) -> &'static str {
        "fake"
    }

    fn get(
        &self,
        key: &CacheKey,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, CacheError>> + Send {
        let mut state = self.0.lock().unwrap();
        state.calls += 1;
        future::ready(if state.fail_reads {
            Err(CacheError(anyhow::anyhow!("read refused")))
        } else {
            Ok(state.entries.get(&**key).cloned())
        })
    }

    fn set(
        &self,
        key: &CacheKey,
        value: &[u8],
        retain: Duration,
    ) -> impl Future<Output = Result<(), CacheError>> + Send {
        let mut state = self.0.lock().unwrap();
        state.calls += 1;
        future::ready(if state.fail_writes {
            Err(CacheError(anyhow::anyhow!("write refused")))
        } else {
            state.last_retain = Some(retain);
            state.entries.insert(key.to_string(), value.to_vec());
            Ok(())
        })
    }

    fn ping(&self) -> impl Future<Output = Result<(), CacheError>> + Send {
        future::ready(if self.0.lock().unwrap().fail_pings {
            Err(CacheError(anyhow::anyhow!("no answer")))
        } else {
            Ok(())
        })
    }
}

// == counts source ==

struct CountsState {
    calls: u32,
    lines: u64,
    fail: bool,
}

/// A counts source that answers a fixed measurement and fails on request.
#[derive(Clone)]
pub struct FakeCountsSource(Arc<Mutex<CountsState>>);

impl Default for FakeCountsSource {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(CountsState {
            calls: 0,
            lines: 100,
            fail: false,
        })))
    }
}

impl fmt::Debug for FakeCountsSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeCountsSource").finish_non_exhaustive()
    }
}

/// Rust counts with `lines` lines.
pub fn counts_with(lines: u64) -> Counts {
    Counts {
        language: Language::Rust,
        lines,
        tests: 7,
        clippy_lints: Some(3),
    }
}

impl FakeCountsSource {
    pub fn calls(&self) -> u32 {
        self.0.lock().unwrap().calls
    }

    pub fn set_lines(&self, lines: u64) {
        self.0.lock().unwrap().lines = lines;
    }

    pub fn set_failing(&self, fail: bool) {
        self.0.lock().unwrap().fail = fail;
    }
}

impl CountsSource for FakeCountsSource {
    fn fetch_counts(
        &self,
        _repo: &RepoName,
    ) -> impl Future<Output = Result<Counts, CountsError>> + Send {
        let mut state = self.0.lock().unwrap();
        state.calls += 1;
        future::ready(if state.fail {
            Err(CountsError::Upstream(anyhow::anyhow!("tarball refused")))
        } else {
            Ok(counts_with(state.lines))
        })
    }
}

// == calendar source ==

struct CalendarState {
    calls: u32,
    total: u32,
    fail: bool,
}

/// A calendar source answering a fixed year and failing on request.
#[derive(Clone)]
pub struct FakeCalendar(Arc<Mutex<CalendarState>>);

impl Default for FakeCalendar {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(CalendarState {
            calls: 0,
            total: 5099,
            fail: false,
        })))
    }
}

impl fmt::Debug for FakeCalendar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeCalendar").finish_non_exhaustive()
    }
}

/// Three fixed days ending 2026-09-30. `total` is whatever the fake holds, not the
/// days' sum, so a test tells two answers apart by it.
pub fn calendar_for(login: &Login, total: u32) -> Calendar {
    let day = |d, count, level| Day {
        date: NaiveDate::from_ymd_opt(2026, 9, d).unwrap(),
        count,
        level,
    };
    Calendar {
        login: login.clone(),
        total,
        days: vec![day(28, 0, 0), day(29, 12, 2), day(30, 48, 3)],
    }
}

impl FakeCalendar {
    pub fn calls(&self) -> u32 {
        self.0.lock().unwrap().calls
    }

    pub fn set_total(&self, total: u32) {
        self.0.lock().unwrap().total = total;
    }

    pub fn set_failing(&self, fail: bool) {
        self.0.lock().unwrap().fail = fail;
    }
}

impl CalendarSource for FakeCalendar {
    fn fetch_calendar(
        &self,
        login: &Login,
    ) -> impl Future<Output = Result<Calendar, CalendarError>> + Send {
        let mut state = self.0.lock().unwrap();
        state.calls += 1;
        future::ready(if state.fail {
            Err(CalendarError::Upstream(anyhow::anyhow!("graphql refused")))
        } else {
            Ok(calendar_for(login, state.total))
        })
    }
}

#[test]
fn the_fake_calendar_holds_three_real_days() {
    let calendar = calendar_for(&Login::new("someone").unwrap(), 60);
    let dates: Vec<NaiveDate> = calendar.days.iter().map(|day| day.date).collect();
    assert!(
        !dates.contains(&NaiveDate::MIN),
        "an impossible date fell back to MIN: {dates:?}"
    );
    assert!(
        dates.windows(2).all(|pair| pair[0] < pair[1]),
        "days are not distinct and ascending: {dates:?}"
    );
}
