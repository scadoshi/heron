//! [`Service`], the sweep that measures and caches, and the reads requests make.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::Mutex;

use crate::domain::clock::Clock;
use crate::domain::counts::models::{Counts, CountsError, CountsReport, Sweep};
use crate::domain::counts::ports::{CountsService, CountsSource};
use crate::domain::stats::models::cache_key::CacheKey;
use crate::domain::stats::models::repo_name::RepoName;
use crate::domain::stats::models::snapshot::{self, Snapshot};
use crate::domain::stats::ports::{Cache, StatsService};

/// What the service measures and how long the cache keeps a measurement.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The allowlist, in the order a sweep walks it.
    pub repos: Vec<RepoName>,
    /// How long the cache keeps a measurement. A sweep refreshes one long before
    /// this when the repository is pushed to; this is the floor under a repository
    /// nobody touches.
    pub retain: Duration,
}

/// Counts service: the sweep writes, requests read.
#[derive(Clone)]
pub struct Service<S, C, T, K> {
    source: S,
    cache: C,
    stats: T,
    clock: K,
    repos: Arc<[RepoName]>,
    /// Sweeps run one at a time; a second caller waits for the first to finish.
    sweeping: Arc<Mutex<()>>,
    retain: Duration,
}

/// True when `repo` has no measurement, or was pushed to after the one it has.
/// A repository with a measurement and no known push is left alone.
fn needs_measuring(cached: Option<&Snapshot<Counts>>, pushed_at: Option<DateTime<Utc>>) -> bool {
    match cached {
        None => true,
        Some(snapshot) => pushed_at.is_some_and(|pushed_at| pushed_at > snapshot.fetched_at),
    }
}

/// Prints the settings only: the adapters need not be `Debug`, and the sweep
/// lock is never taken to print it.
impl<S, C, T, K> fmt::Debug for Service<S, C, T, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Service")
            .field("repos", &self.repos)
            .field("retain", &self.retain)
            .finish_non_exhaustive()
    }
}

impl<S: CountsSource, C: Cache, T: StatsService, K: Clock> Service<S, C, T, K> {
    /// Creates a counts service over the given adapters. `stats` is where a
    /// repository's `pushed_at` comes from.
    pub fn new(source: S, cache: C, stats: T, clock: K, settings: Settings) -> Self {
        Self {
            source,
            cache,
            stats,
            clock,
            repos: settings.repos.into(),
            sweeping: Arc::new(Mutex::new(())),
            retain: settings.retain,
        }
    }

    /// When GitHub last saw a push to `repo`, from the stats service's cached or
    /// fresh answer. `None` when stats could not be read.
    async fn pushed_at(&self, repo: &RepoName) -> Option<DateTime<Utc>> {
        match self.stats.fetch_repo_stats(repo).await {
            Ok(report) => report.stats.pushed_at,
            Err(error) => {
                tracing::warn!("stats for {repo} are unavailable to the sweep: {error}");
                None
            }
        }
    }

    async fn measure(&self, repo: &RepoName, key: &CacheKey) -> Result<(), CountsError> {
        let counts = self.source.fetch_counts(repo).await?;
        let retain = TimeDelta::from_std(self.retain).unwrap_or(TimeDelta::MAX);
        let measured = Snapshot::new(counts, self.clock.now(), retain);
        snapshot::write(&self.cache, key, &measured, self.retain).await;
        Ok(())
    }
}

impl<S: CountsSource, C: Cache, T: StatsService, K: Clock> CountsService for Service<S, C, T, K> {
    async fn fetch_counts(&self, repo: &RepoName) -> Option<CountsReport> {
        let measured: Snapshot<Counts> =
            snapshot::read(&self.cache, &CacheKey::for_counts(repo)).await?;
        Some(CountsReport {
            repo: repo.clone(),
            counts: measured.value,
            measured_at: measured.fetched_at,
        })
    }

    async fn sweep(&self) -> Sweep {
        let _one_at_a_time = self.sweeping.lock().await;
        let mut sweep = Sweep::default();
        for repo in self.repos.iter() {
            let key = CacheKey::for_counts(repo);
            let cached = snapshot::read(&self.cache, &key).await;
            let pushed_at = self.pushed_at(repo).await;
            if !needs_measuring(cached.as_ref(), pushed_at) {
                sweep.skipped.push(repo.clone());
                continue;
            }
            match self.measure(repo, &key).await {
                Ok(()) => {
                    tracing::info!("measured {repo}");
                    sweep.measured.push(repo.clone());
                }
                Err(error) => {
                    tracing::warn!("measuring {repo} failed, the cached value stands: {error}");
                    sweep.failed.push((repo.clone(), error));
                }
            }
        }
        sweep
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::domain::stats;
    use crate::test_support::{
        FakeCache, FakeClock, FakeCountsSource, FakeSource, counts_with, repo,
    };

    type Stats = stats::services::Service<FakeSource, FakeCache, FakeClock>;
    type Counting = Service<FakeCountsSource, FakeCache, Stats, FakeClock>;

    fn service(repos: &[&str]) -> (Counting, FakeCountsSource, FakeSource, FakeCache, FakeClock) {
        let github = FakeSource::default();
        let cache = FakeCache::default();
        let clock = FakeClock::default();
        let repos: Vec<RepoName> = repos.iter().map(|r| repo(r)).collect();
        let stats = Stats::new(
            github.clone(),
            cache.clone(),
            clock.clone(),
            stats::services::Settings {
                repos: repos.clone(),
                fresh: Duration::from_mins(15),
                retain: Duration::from_hours(168),
            },
        );
        let source = FakeCountsSource::default();
        let counting = Service::new(
            source.clone(),
            cache.clone(),
            stats,
            clock.clone(),
            Settings {
                repos,
                retain: Duration::from_hours(168),
            },
        );
        (counting, source, github, cache, clock)
    }

    #[tokio::test]
    async fn the_first_sweep_measures_every_repository_and_keeps_it_for_retain() {
        let (service, source, _, cache, clock) = service(&["a/b", "c/d"]);
        let sweep = service.sweep().await;
        assert_eq!(sweep.measured, [repo("a/b"), repo("c/d")]);
        assert_eq!(sweep.skipped, Vec::<RepoName>::new());
        assert_eq!(sweep.failed.len(), 0);
        assert_eq!(source.calls(), 2);
        assert_eq!(cache.last_retain(), Some(Duration::from_hours(168)));
        let report = service.fetch_counts(&repo("a/b")).await.unwrap();
        assert_eq!(report.counts.lines, 100);
        assert_eq!(report.measured_at, clock.now());
    }

    #[tokio::test]
    async fn a_repository_nobody_pushed_to_is_not_measured_again() {
        let (service, source, _, _, clock) = service(&["a/b"]);
        service.sweep().await;
        clock.advance(Duration::from_secs(3600));
        let sweep = service.sweep().await;
        assert_eq!(sweep.skipped, [repo("a/b")]);
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn a_push_after_the_measurement_measures_again() {
        let (service, source, github, _, clock) = service(&["a/b"]);
        service.sweep().await;
        source.set_lines(250);
        github.set_pushed_at(Some(clock.now() + TimeDelta::minutes(5)));
        // The stats service answers from its cache until its own freshness runs out.
        clock.advance(Duration::from_secs(901));
        let sweep = service.sweep().await;
        assert_eq!(sweep.measured, [repo("a/b")]);
        assert_eq!(source.calls(), 2);
        assert_eq!(
            service
                .fetch_counts(&repo("a/b"))
                .await
                .unwrap()
                .counts
                .lines,
            250
        );
    }

    #[tokio::test]
    async fn a_failed_measurement_leaves_the_cached_value_and_is_reported() {
        let (service, source, github, _, clock) = service(&["a/b"]);
        service.sweep().await;
        source.set_failing(true);
        github.set_pushed_at(Some(clock.now() + TimeDelta::minutes(5)));
        clock.advance(Duration::from_secs(901));
        let sweep = service.sweep().await;
        assert_eq!(sweep.failed.len(), 1);
        assert_eq!(
            service
                .fetch_counts(&repo("a/b"))
                .await
                .unwrap()
                .counts
                .lines,
            100
        );
    }

    #[tokio::test]
    async fn an_unmeasured_repository_is_measured_even_when_stats_are_unavailable() {
        let (service, source, github, _, _) = service(&["a/b"]);
        github.fail_with(crate::test_support::Failure::Upstream);
        let sweep = service.sweep().await;
        assert_eq!(sweep.measured, [repo("a/b")]);
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn a_measured_repository_is_left_alone_when_stats_are_unavailable() {
        let (service, source, github, _, clock) = service(&["a/b"]);
        service.sweep().await;
        github.fail_with(crate::test_support::Failure::Upstream);
        clock.advance(Duration::from_hours(8 * 24));
        let sweep = service.sweep().await;
        assert_eq!(sweep.skipped, [repo("a/b")]);
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn counts_only_read_the_cache() {
        let (service, source, _, _, _) = service(&["a/b"]);
        assert!(service.fetch_counts(&repo("a/b")).await.is_none());
        assert!(service.fetch_counts(&repo("x/y")).await.is_none());
        assert_eq!(source.calls(), 0);
    }

    #[test]
    fn needs_measuring_follows_the_push_not_the_clock() {
        let at = |h| Utc.with_ymd_and_hms(2026, 10, 1, h, 0, 0).unwrap();
        let snapshot = Snapshot {
            value: counts_with(1),
            fetched_at: at(12),
            fresh_until: at(13),
        };
        assert!(needs_measuring(None, None));
        assert!(needs_measuring(None, Some(at(1))));
        assert!(!needs_measuring(Some(&snapshot), None));
        assert!(!needs_measuring(Some(&snapshot), Some(at(12))));
        assert!(needs_measuring(Some(&snapshot), Some(at(13))));
    }
}
