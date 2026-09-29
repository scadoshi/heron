//! In-memory fakes for the ports, shared by unit tests across modules. Compiled only
//! under `#[cfg(test)]`.

// Test-only helpers: names carry the meaning, and overflow in a counter is a bug in
// the test worth panicking on.
#![allow(missing_docs, clippy::arithmetic_side_effects)]

use crate::domain::{
    clock::Clock,
    stats::{
        models::{
            cache_key::CacheKey,
            errors::{CacheError, StatsError},
            repo_name::RepoName,
            repo_stats::{Language, RepoStats},
        },
        ports::{StatsCache, StatsSource},
    },
};
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use std::{
    collections::HashMap,
    future::{Future, ready},
    sync::{Arc, Mutex},
    time::Duration,
};

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
        languages: vec![Language {
            name: "Rust".to_string(),
            bytes: 1000,
        }],
        additions: Some(500),
        deletions: Some(100),
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

impl FakeClock {
    pub fn advance(&self, secs: u64) {
        let mut now = self.0.lock().unwrap();
        *now += TimeDelta::seconds(i64::try_from(secs).unwrap());
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
            failure: None,
            failing_repos: HashMap::new(),
            delay: None,
        })))
    }
}

impl FakeSource {
    pub fn calls(&self) -> u32 {
        self.0.lock().unwrap().calls
    }

    pub fn set_commits(&self, commits: u64) {
        self.0.lock().unwrap().commits = commits;
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
    pub fn delay(&self, delay: Duration) {
        self.0.lock().unwrap().delay = Some(delay);
    }
}

impl StatsSource for FakeSource {
    async fn repo_stats(&self, repo: &RepoName) -> Result<RepoStats, StatsError> {
        let (delay, outcome) = {
            let mut state = self.0.lock().unwrap();
            state.calls += 1;
            let failure = state.failing_repos.get(repo).copied().or(state.failure);
            let outcome = match failure {
                Some(failure) => Err(failure.into_error(repo)),
                None => Ok(stats_for(repo, state.commits)),
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
}

/// A cache that never expires anything and fails on request.
#[derive(Clone, Default)]
pub struct FakeCache(Arc<Mutex<CacheState>>);

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
}

impl StatsCache for FakeCache {
    fn backend(&self) -> &'static str {
        "fake"
    }

    fn get(
        &self,
        key: &CacheKey,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, CacheError>> + Send {
        let state = self.0.lock().unwrap();
        ready(if state.fail_reads {
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
        ready(if state.fail_writes {
            Err(CacheError(anyhow::anyhow!("write refused")))
        } else {
            state.last_retain = Some(retain);
            state.entries.insert(key.to_string(), value.to_vec());
            Ok(())
        })
    }

    fn ping(&self) -> impl Future<Output = Result<(), CacheError>> + Send {
        ready(if self.0.lock().unwrap().fail_pings {
            Err(CacheError(anyhow::anyhow!("no answer")))
        } else {
            Ok(())
        })
    }
}
