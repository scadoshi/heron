//! Integration-test harness.
//!
//! Builds the real router with [`build_router`] over the real service and the real
//! memory cache, with a stub behind the source port and a clock the test moves.
//! Requests go through `tower::ServiceExt::oneshot`: no socket, full middleware stack.

// Not every test file uses every helper, and clippy.toml's in-tests allowances do not
// reach helpers that are not marked `#[test]`.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, HeaderValue, Request, StatusCode},
};
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use heron::{
    domain::{
        clock::Clock,
        counts::{
            self,
            models::{Counts, CountsError, Language as CountsLanguage},
            ports::{CountsSource, ErasedCountsService},
        },
        health,
        stats::{
            self,
            models::{
                cache_key::CacheKey,
                errors::{CacheError, StatsError},
                repo_name::RepoName,
                repo_stats::{Language, RepoStats},
            },
            ports::{StatsCache, StatsSource},
            services::Settings,
        },
    },
    inbound::http::{AppState, build_router},
    outbound::cache::memory::MemoryCache,
};
use http_body_util::BodyExt;
use serde_json::Value;
use std::{
    collections::HashMap,
    future::{Future, ready},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;

pub const ALLOWED_ORIGIN: &str = "https://scottyfermo.com";
pub const FRESH_SECS: u64 = 600;

pub fn repo(raw: &str) -> RepoName {
    RepoName::new(raw).unwrap()
}

// == clock ==

#[derive(Clone)]
pub struct TestClock(Arc<Mutex<DateTime<Utc>>>);

impl Default for TestClock {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(
            Utc.with_ymd_and_hms(2026, 9, 29, 16, 4, 41).unwrap() + TimeDelta::milliseconds(123),
        )))
    }
}

impl TestClock {
    pub fn advance(&self, secs: u64) {
        *self.0.lock().unwrap() += TimeDelta::seconds(i64::try_from(secs).unwrap());
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

// == source ==

/// What the stub source answers for a repository.
#[derive(Clone)]
pub enum Outcome {
    Commits(u64),
    Private,
    Missing,
    RateLimited,
    /// Fails with this text as the cause.
    Broken(&'static str),
}

/// A source answering from a table. A repository not in the table has 10 commits.
#[derive(Clone, Default)]
pub struct StubSource {
    outcomes: Arc<Mutex<HashMap<RepoName, Outcome>>>,
    calls: Arc<AtomicU32>,
}

impl StubSource {
    pub fn answer(&self, repo: &RepoName, outcome: Outcome) {
        self.outcomes.lock().unwrap().insert(repo.clone(), outcome);
    }

    pub fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl StatsSource for StubSource {
    fn repo_stats(
        &self,
        repo: &RepoName,
    ) -> impl Future<Output = Result<RepoStats, StatsError>> + Send {
        ready(self.answer_for(repo))
    }
}

impl StubSource {
    fn answer_for(&self, repo: &RepoName) -> Result<RepoStats, StatsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.outcomes.lock().unwrap().get(repo).cloned();
        match outcome.unwrap_or(Outcome::Commits(10)) {
            Outcome::Commits(commits) => Ok(RepoStats {
                repo: repo.clone(),
                commits,
                stars: 2,
                default_branch: "main".to_string(),
                pushed_at: Some(Utc.with_ymd_and_hms(2026, 9, 29, 15, 58, 2).unwrap()),
                languages: vec![
                    Language {
                        name: "Rust".to_string(),
                        bytes: 9000,
                    },
                    Language {
                        name: "Shell".to_string(),
                        bytes: 40,
                    },
                ],
                additions: Some(14115),
                deletions: None,
            }),
            Outcome::Private => Err(StatsError::PrivateRepo(repo.clone())),
            Outcome::Missing => Err(StatsError::UnknownRepo(repo.clone())),
            Outcome::RateLimited => Err(StatsError::RateLimited { reset_at: None }),
            Outcome::Broken(cause) => Err(StatsError::Upstream(anyhow::anyhow!(cause))),
        }
    }
}

// == counts source ==

/// A counts source answering a fixed measurement for every repository.
#[derive(Clone, Default)]
pub struct StubCounts;

impl StubCounts {
    pub fn counts() -> Counts {
        Counts {
            language: CountsLanguage::Rust,
            lines: 6102,
            tests: 242,
            clippy_lints: Some(14),
        }
    }
}

impl CountsSource for StubCounts {
    fn counts(&self, _: &RepoName) -> impl Future<Output = Result<Counts, CountsError>> + Send {
        ready(Ok(Self::counts()))
    }
}

// == cache ==

/// A cache whose backend never answers.
#[derive(Clone)]
pub struct DeadCache;

impl StatsCache for DeadCache {
    fn backend(&self) -> &'static str {
        "dead"
    }

    fn get(
        &self,
        _: &CacheKey,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, CacheError>> + Send {
        ready(Err(CacheError(anyhow::anyhow!("no answer"))))
    }

    fn set(
        &self,
        _: &CacheKey,
        _: &[u8],
        _: Duration,
    ) -> impl Future<Output = Result<(), CacheError>> + Send {
        ready(Err(CacheError(anyhow::anyhow!("no answer"))))
    }

    fn ping(&self) -> impl Future<Output = Result<(), CacheError>> + Send {
        ready(Err(CacheError(anyhow::anyhow!("no answer"))))
    }
}

// == app ==

/// One response, read to the end.
pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|error| panic!("body is not json ({error}): {:?}", self.body))
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(|value| value.to_str().unwrap())
    }
}

pub struct TestApp {
    router: Router,
    pub source: StubSource,
    pub clock: TestClock,
    /// The counts service, so a test can run a sweep by hand.
    pub counts: Arc<dyn ErasedCountsService>,
}

impl TestApp {
    /// An app serving `repos` from the memory cache.
    pub fn new(repos: &[&str]) -> Self {
        Self::over(repos, MemoryCache::new())
    }

    /// An app serving `repos` from `cache`.
    pub fn over<C: StatsCache>(repos: &[&str], cache: C) -> Self {
        let source = StubSource::default();
        let clock = TestClock::default();
        let stats_service = stats::services::Service::new(
            source.clone(),
            cache.clone(),
            clock.clone(),
            Settings {
                repos: repos.iter().map(|raw| repo(raw)).collect(),
                fresh: Duration::from_secs(FRESH_SECS),
                retain: Duration::from_secs(FRESH_SECS * 10),
            },
        );
        let counts_service: Arc<dyn ErasedCountsService> =
            Arc::new(counts::services::Service::new(
                StubCounts,
                cache.clone(),
                stats_service.clone(),
                clock.clone(),
                counts::services::Settings {
                    repos: repos.iter().map(|raw| repo(raw)).collect(),
                    retain: Duration::from_secs(FRESH_SECS * 10),
                },
            ));
        let state = AppState {
            stats_service: Arc::new(stats_service),
            health_service: Arc::new(health::services::Service::new(cache)),
            counts_service: Arc::clone(&counts_service),
        };
        let router = build_router(state, vec![HeaderValue::from_static(ALLOWED_ORIGIN)]).unwrap();
        Self {
            router,
            source,
            clock,
            counts: counts_service,
        }
    }

    pub async fn get(&self, path: &str) -> Reply {
        self.get_with(path, &[]).await
    }

    pub async fn get_with(&self, path: &str, headers: &[(&str, &str)]) -> Reply {
        self.send("GET", path, headers).await
    }

    pub async fn send(&self, method: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
        let mut request = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut request = request.body(Body::empty()).unwrap();
        // The rate limiter falls back to the peer address, and a `oneshot` request
        // has none.
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            40000,
        )));
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }
}
