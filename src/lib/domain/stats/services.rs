use crate::domain::{
    clock::Clock,
    stats::{
        models::{
            cache_key::CacheKey,
            errors::StatsError,
            portfolio_stats::{PortfolioStats, Totals},
            repo_name::RepoName,
            repo_stats::{RepoReport, RepoStats},
            snapshot::Snapshot,
        },
        ports::{StatsCache, StatsService, StatsSource},
    },
};
use chrono::TimeDelta;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{sync::Mutex, task::JoinSet};

/// What the service is allowed to serve and for how long.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The allowlist. Any other repository is answered as unknown.
    pub repos: Vec<RepoName>,
    /// How long a snapshot is served without asking the source again.
    pub fresh: Duration,
    /// How long the cache keeps a snapshot. The gap past `fresh` is the window in
    /// which a stale value can cover for a failing source.
    pub retain: Duration,
}

/// Stats service: reads through the cache and refreshes from the source.
#[derive(Clone)]
pub struct Service<S: StatsSource, C: StatsCache, K: Clock> {
    source: S,
    cache: C,
    clock: K,
    repos: Arc<[RepoName]>,
    /// One lock per allowlisted repository, so concurrent callers on an expired key
    /// cause one refresh. Built once from the allowlist and never grown.
    refresh_locks: Arc<HashMap<RepoName, Mutex<()>>>,
    fresh: TimeDelta,
    retain: Duration,
}

impl<S: StatsSource, C: StatsCache, K: Clock> Service<S, C, K> {
    /// Creates a stats service over the given adapters.
    pub fn new(source: S, cache: C, clock: K, settings: Settings) -> Self {
        let refresh_locks = settings
            .repos
            .iter()
            .map(|repo| (repo.clone(), Mutex::new(())))
            .collect();
        Self {
            source,
            cache,
            clock,
            repos: settings.repos.into(),
            refresh_locks: Arc::new(refresh_locks),
            fresh: TimeDelta::from_std(settings.fresh).unwrap_or(TimeDelta::MAX),
            retain: settings.retain,
        }
    }

    /// The snapshot under `key`. A cache error or an undecodable payload is a miss.
    async fn read(&self, key: &CacheKey) -> Option<Snapshot<RepoStats>> {
        let bytes = match self.cache.get(key).await {
            Ok(bytes) => bytes?,
            Err(error) => {
                tracing::warn!("cache read failed for {key}, treating as a miss: {error}");
                return None;
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::warn!("cached payload under {key} did not decode: {error}");
                None
            }
        }
    }

    /// Stores `snapshot`. A failure is logged and swallowed.
    async fn write(&self, key: &CacheKey, snapshot: &Snapshot<RepoStats>) {
        let bytes = match serde_json::to_vec(snapshot) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!("snapshot for {key} did not encode: {error}");
                return;
            }
        };
        if let Err(error) = self.cache.set(key, &bytes, self.retain).await {
            tracing::warn!("cache write failed for {key}: {error}");
        }
    }

    fn fresh_report(&self, snapshot: Option<&Snapshot<RepoStats>>) -> Option<RepoReport> {
        let snapshot = snapshot.filter(|s| s.is_fresh(self.clock.now()))?;
        Some(RepoReport {
            stats: snapshot.value.clone(),
            fetched_at: snapshot.fetched_at,
            stale: false,
        })
    }

    async fn resolve(&self, repo: &RepoName) -> Result<RepoReport, StatsError> {
        let Some(refresh_lock) = self.refresh_locks.get(repo) else {
            return Err(StatsError::UnknownRepo(repo.clone()));
        };
        let key = CacheKey::for_repo(repo);

        if let Some(report) = self.fresh_report(self.read(&key).await.as_ref()) {
            return Ok(report);
        }

        let _refreshing = refresh_lock.lock().await;
        // Another caller may have refreshed while this one waited for the lock.
        let cached = self.read(&key).await;
        if let Some(report) = self.fresh_report(cached.as_ref()) {
            return Ok(report);
        }

        match self.source.repo_stats(repo).await {
            Ok(stats) => {
                let fetched_at = self.clock.now();
                let snapshot = Snapshot {
                    value: stats,
                    fetched_at,
                    fresh_until: fetched_at
                        .checked_add_signed(self.fresh)
                        .unwrap_or(fetched_at),
                };
                self.write(&key, &snapshot).await;
                Ok(RepoReport {
                    stats: snapshot.value,
                    fetched_at,
                    stale: false,
                })
            }
            // A repository that went private or was deleted must stop being served,
            // so only a source that could not answer falls back to the stale value.
            Err(error @ (StatsError::RateLimited { .. } | StatsError::Upstream(_))) => {
                let Some(stale) = cached else {
                    return Err(error);
                };
                tracing::warn!("refresh failed for {repo}, serving the stale value: {error}");
                Ok(RepoReport {
                    stats: stale.value,
                    fetched_at: stale.fetched_at,
                    stale: true,
                })
            }
            Err(error) => Err(error),
        }
    }
}

impl<S: StatsSource, C: StatsCache, K: Clock> StatsService for Service<S, C, K> {
    async fn repo_stats(&self, repo: &RepoName) -> Result<RepoReport, StatsError> {
        self.resolve(repo).await
    }

    async fn portfolio_stats(&self) -> Result<PortfolioStats, StatsError> {
        let mut tasks = JoinSet::new();
        for (position, repo) in self.repos.iter().cloned().enumerate() {
            let service = self.clone();
            tasks.spawn(async move {
                let result = service.resolve(&repo).await;
                (position, repo, result)
            });
        }

        let mut resolved = Vec::new();
        let mut unavailable = Vec::new();
        let mut last_error = None;
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((position, _, Ok(report))) => resolved.push((position, report)),
                Ok((position, repo, Err(error))) => {
                    tracing::warn!("{repo} is unavailable: {error}");
                    unavailable.push((position, repo));
                    last_error = Some(error);
                }
                Err(error) => {
                    last_error = Some(StatsError::Upstream(anyhow::anyhow!(
                        "stats task failed: {error}"
                    )));
                }
            }
        }

        if resolved.is_empty()
            && let Some(error) = last_error
        {
            return Err(error);
        }

        // Tasks finish in any order. Sort back into allowlist order.
        resolved.sort_by_key(|(position, _)| *position);
        unavailable.sort_by_key(|(position, _)| *position);
        let repos: Vec<RepoReport> = resolved.into_iter().map(|(_, report)| report).collect();

        Ok(PortfolioStats {
            totals: Totals::of(&repos),
            repos,
            unavailable: unavailable.into_iter().map(|(_, repo)| repo).collect(),
            generated_at: self.clock.now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Failure, FakeCache, FakeClock, FakeSource, repo};

    const FRESH_SECS: u64 = 60;

    type TestService = Service<FakeSource, FakeCache, FakeClock>;

    fn service(repos: &[&str]) -> (TestService, FakeSource, FakeCache, FakeClock) {
        let source = FakeSource::default();
        let cache = FakeCache::default();
        let clock = FakeClock::default();
        let service = Service::new(
            source.clone(),
            cache.clone(),
            clock.clone(),
            Settings {
                repos: repos.iter().map(|r| repo(r)).collect(),
                fresh: Duration::from_secs(FRESH_SECS),
                retain: Duration::from_hours(1),
            },
        );
        (service, source, cache, clock)
    }

    #[tokio::test]
    async fn a_fresh_snapshot_is_served_without_calling_the_source() {
        let (service, source, _, clock) = service(&["a/b"]);
        service.repo_stats(&repo("a/b")).await.unwrap();
        clock.advance(FRESH_SECS - 1);
        let report = service.repo_stats(&repo("a/b")).await.unwrap();
        assert_eq!(source.calls(), 1);
        assert!(!report.stale);
    }

    #[tokio::test]
    async fn a_stale_snapshot_triggers_one_refresh() {
        let (service, source, _, clock) = service(&["a/b"]);
        service.repo_stats(&repo("a/b")).await.unwrap();
        clock.advance(FRESH_SECS);
        source.set_commits(99);
        let report = service.repo_stats(&repo("a/b")).await.unwrap();
        assert_eq!(source.calls(), 2);
        assert_eq!(report.stats.commits, 99);
        assert_eq!(report.fetched_at, clock.now());
    }

    #[tokio::test]
    async fn a_source_failure_serves_the_stale_snapshot_and_marks_it() {
        let (service, source, _, clock) = service(&["a/b"]);
        let first = service.repo_stats(&repo("a/b")).await.unwrap();
        clock.advance(FRESH_SECS);
        for failure in [Failure::Upstream, Failure::RateLimited] {
            source.fail_with(failure);
            let report = service.repo_stats(&repo("a/b")).await.unwrap();
            assert!(report.stale);
            assert_eq!(report.stats, first.stats);
            assert_eq!(report.fetched_at, first.fetched_at);
        }
    }

    #[tokio::test]
    async fn a_source_failure_with_an_empty_cache_returns_the_error() {
        let (service, source, _, _) = service(&["a/b"]);
        source.fail_with(Failure::Upstream);
        let error = service.repo_stats(&repo("a/b")).await.unwrap_err();
        assert!(matches!(error, StatsError::Upstream(_)));
    }

    #[tokio::test]
    async fn a_repository_that_went_private_is_not_served_from_the_stale_value() {
        let (service, source, _, clock) = service(&["a/b"]);
        service.repo_stats(&repo("a/b")).await.unwrap();
        clock.advance(FRESH_SECS);
        source.fail_with(Failure::Private);
        let error = service.repo_stats(&repo("a/b")).await.unwrap_err();
        assert!(matches!(error, StatsError::PrivateRepo(_)));
    }

    #[tokio::test]
    async fn a_cache_read_error_is_a_miss_and_the_request_succeeds() {
        let (service, source, cache, _) = service(&["a/b"]);
        service.repo_stats(&repo("a/b")).await.unwrap();
        cache.fail_reads(true);
        let report = service.repo_stats(&repo("a/b")).await.unwrap();
        assert_eq!(source.calls(), 2);
        assert!(!report.stale);
    }

    #[tokio::test]
    async fn a_cache_write_error_still_returns_the_value() {
        let (service, _, cache, _) = service(&["a/b"]);
        cache.fail_writes(true);
        let report = service.repo_stats(&repo("a/b")).await.unwrap();
        assert_eq!(report.stats.repo, repo("a/b"));
        assert!(cache.is_empty());
    }

    #[tokio::test]
    async fn an_undecodable_payload_is_a_miss() {
        let (service, source, cache, _) = service(&["a/b"]);
        cache.put(&CacheKey::for_repo(&repo("a/b")), b"not json");
        service.repo_stats(&repo("a/b")).await.unwrap();
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn an_unknown_repository_is_rejected_and_the_source_is_never_called() {
        let (service, source, _, _) = service(&["a/b"]);
        let error = service.repo_stats(&repo("a/other")).await.unwrap_err();
        assert!(matches!(error, StatsError::UnknownRepo(_)));
        assert_eq!(source.calls(), 0);
    }

    #[tokio::test]
    async fn concurrent_callers_on_an_expired_key_cause_one_source_call() {
        let (service, source, _, _) = service(&["a/b"]);
        source.delay(Duration::from_millis(20));
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let service = service.clone();
                tokio::spawn(async move { service.repo_stats(&repo("a/b")).await })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn the_snapshot_is_stored_with_the_retain_window() {
        let (service, _, cache, _) = service(&["a/b"]);
        service.repo_stats(&repo("a/b")).await.unwrap();
        assert_eq!(cache.last_retain(), Some(Duration::from_hours(1)));
    }

    #[tokio::test]
    async fn portfolio_stats_keeps_allowlist_order_and_sums_totals() {
        let (service, _, _, _) = service(&["a/one", "a/two", "a/three"]);
        let portfolio = service.portfolio_stats().await.unwrap();
        let names: Vec<&str> = portfolio.repos.iter().map(|r| &*r.stats.repo).collect();
        assert_eq!(names, ["a/one", "a/two", "a/three"]);
        assert_eq!(portfolio.totals.repos, 3);
        assert_eq!(portfolio.totals.commits, 30);
        assert!(portfolio.unavailable.is_empty());
    }

    #[tokio::test]
    async fn portfolio_stats_returns_partial_results_and_names_what_is_missing() {
        let (service, source, _, _) = service(&["a/one", "a/two"]);
        source.fail_repo(&repo("a/two"), Failure::Upstream);
        let portfolio = service.portfolio_stats().await.unwrap();
        assert_eq!(portfolio.repos.len(), 1);
        assert_eq!(portfolio.unavailable, vec![repo("a/two")]);
        assert_eq!(portfolio.totals.repos, 1);
    }

    #[tokio::test]
    async fn portfolio_stats_fails_when_nothing_resolved() {
        let (service, source, _, _) = service(&["a/one", "a/two"]);
        source.fail_with(Failure::RateLimited);
        let error = service.portfolio_stats().await.unwrap_err();
        assert!(matches!(error, StatsError::RateLimited { .. }));
    }
}
