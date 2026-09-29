//! Port traits for stats: where the numbers come from, where they are cached, and the
//! service the HTTP layer calls.

use crate::domain::{
    BoxFuture,
    stats::models::{
        cache_key::CacheKey,
        errors::{CacheError, StatsError},
        portfolio_stats::PortfolioStats,
        repo_name::RepoName,
        repo_stats::{RepoReport, RepoStats},
    },
};
use std::{future::Future, time::Duration};

/// Where stats are read from.
pub trait StatsSource: Clone + Send + Sync + 'static {
    /// Reads current stats for `repo`.
    fn repo_stats(
        &self,
        repo: &RepoName,
    ) -> impl Future<Output = Result<RepoStats, StatsError>> + Send;
}

/// A byte store with expiry. Bytes in and bytes out: the service owns the encoding.
pub trait StatsCache: Clone + Send + Sync + 'static {
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

/// Service port for stats.
pub trait StatsService: Clone + Send + Sync + 'static {
    /// Stats for one allowlisted repository, from cache when fresh.
    fn repo_stats(
        &self,
        repo: &RepoName,
    ) -> impl Future<Output = Result<RepoReport, StatsError>> + Send;

    /// Stats for every allowlisted repository. Fails only when none resolved.
    fn portfolio_stats(&self) -> impl Future<Output = Result<PortfolioStats, StatsError>> + Send;
}

/// Object-safe wrapper used by `AppState` so the concrete service type stays out of
/// the generic parameter list. Auto-implemented for any `StatsService`.
pub trait ErasedStatsService: Send + Sync + 'static {
    /// See [`StatsService::repo_stats`].
    fn repo_stats<'a>(
        &'a self,
        repo: &'a RepoName,
    ) -> BoxFuture<'a, Result<RepoReport, StatsError>>;

    /// See [`StatsService::portfolio_stats`].
    fn portfolio_stats(&self) -> BoxFuture<'_, Result<PortfolioStats, StatsError>>;
}

impl<T> ErasedStatsService for T
where
    T: StatsService,
{
    fn repo_stats<'a>(
        &'a self,
        repo: &'a RepoName,
    ) -> BoxFuture<'a, Result<RepoReport, StatsError>> {
        Box::pin(StatsService::repo_stats(self, repo))
    }

    fn portfolio_stats(&self) -> BoxFuture<'_, Result<PortfolioStats, StatsError>> {
        Box::pin(StatsService::portfolio_stats(self))
    }
}
