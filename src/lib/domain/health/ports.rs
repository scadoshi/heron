//! Port traits for health checks.

use crate::domain::{BoxFuture, health::models::HealthCheckFailed};
use std::future::Future;

/// Service port for health checks.
pub trait HealthService: Clone + Send + Sync + 'static {
    /// Name of the cache backend in use: `memory`, `steller`, or `layered`.
    fn cache_backend(&self) -> &'static str;

    /// Pings the cache. For `layered` this reports the primary, so a dead steller
    /// shows here while requests keep succeeding from the memory layer.
    fn check_cache(&self) -> impl Future<Output = Result<(), HealthCheckFailed>> + Send;
}

/// Object-safe wrapper used by `AppState` so the concrete service type stays out of
/// the generic parameter list. Auto-implemented for any `HealthService`.
pub trait ErasedHealthService: Send + Sync + 'static {
    /// See [`HealthService::cache_backend`].
    fn cache_backend(&self) -> &'static str;

    /// See [`HealthService::check_cache`].
    fn check_cache(&self) -> BoxFuture<'_, Result<(), HealthCheckFailed>>;
}

impl<T> ErasedHealthService for T
where
    T: HealthService,
{
    fn cache_backend(&self) -> &'static str {
        HealthService::cache_backend(self)
    }

    fn check_cache(&self) -> BoxFuture<'_, Result<(), HealthCheckFailed>> {
        Box::pin(HealthService::check_cache(self))
    }
}
