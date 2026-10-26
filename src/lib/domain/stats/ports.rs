//! Port traits for stats: where the numbers come from, where they are cached, and the
//! service the HTTP layer calls.

use std::future::Future;
use std::time::Duration;

use crate::domain::BoxFuture;
use crate::domain::stats::models::cache_key::CacheKey;
use crate::domain::stats::models::errors::{CacheError, StatsError};
use crate::domain::stats::models::portfolio_stats::PortfolioStats;
use crate::domain::stats::models::repo_name::RepoName;
use crate::domain::stats::models::repo_stats::{RepoReport, RepoStats};

/// Where stats are read from.
pub trait StatsSource: Clone + Send + Sync + 'static {
    /// Reads current stats for `repo`.
    fn fetch_repo_stats(
        &self,
        repo: &RepoName,
    ) -> impl Future<Output = Result<RepoStats, StatsError>> + Send;
}

/// A byte store with expiry, shared by stats, counts and the calendar and checked by
/// health. Bytes in and bytes out: each service owns its encoding.
pub trait Cache: Clone + Send + Sync + 'static {
    /// Name of the backend: `memory`, `steller`, or `layered`.
    fn backend(&self) -> &'static str;

    /// The value under `key`, or `None` when absent or expired.
    fn get(
        &self,
        key: &CacheKey,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, CacheError>> + Send;

    /// Stores `value` under `key` and drops it after `retain`.
    fn set(
        &self,
        key: &CacheKey,
        value: &[u8],
        retain: Duration,
    ) -> impl Future<Output = Result<(), CacheError>> + Send;

    /// Confirms the backend answers.
    fn ping(&self) -> impl Future<Output = Result<(), CacheError>> + Send;
}

/// Stats for the allowlisted repositories, read by the HTTP handlers and by the
/// counts sweep for `pushed_at`.
///
/// Reads through the cache. Past freshness one caller per repository refreshes
/// from the [`StatsSource`] while the others wait for its answer. When the source
/// is rate limited or unreachable the cached value is served with `stale` set; a
/// repository GitHub reports private or missing is never served stale.
pub trait StatsService: Clone + Send + Sync + 'static {
    /// Stats for one allowlisted repository, from cache when fresh.
    fn fetch_repo_stats(
        &self,
        repo: &RepoName,
    ) -> impl Future<Output = Result<RepoReport, StatsError>> + Send;

    /// Stats for every allowlisted repository. Fails only when none resolved.
    fn fetch_portfolio_stats(
        &self,
    ) -> impl Future<Output = Result<PortfolioStats, StatsError>> + Send;
}

/// Object-safe wrapper used by `AppState` so the concrete service type stays out of
/// the generic parameter list. Auto-implemented for any `StatsService`.
pub trait ErasedStatsService: Send + Sync + 'static {
    /// See [`StatsService::fetch_repo_stats`].
    fn fetch_repo_stats<'a>(
        &'a self,
        repo: &'a RepoName,
    ) -> BoxFuture<'a, Result<RepoReport, StatsError>>;

    /// See [`StatsService::fetch_portfolio_stats`].
    fn fetch_portfolio_stats(&self) -> BoxFuture<'_, Result<PortfolioStats, StatsError>>;
}

impl<T> ErasedStatsService for T
where
    T: StatsService,
{
    fn fetch_repo_stats<'a>(
        &'a self,
        repo: &'a RepoName,
    ) -> BoxFuture<'a, Result<RepoReport, StatsError>> {
        Box::pin(StatsService::fetch_repo_stats(self, repo))
    }

    fn fetch_portfolio_stats(&self) -> BoxFuture<'_, Result<PortfolioStats, StatsError>> {
        Box::pin(StatsService::fetch_portfolio_stats(self))
    }
}
