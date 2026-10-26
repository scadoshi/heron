//! [`Service`], health checks over the cache.

use crate::domain::health::models::HealthCheckFailed;
use crate::domain::health::ports::HealthService;
use crate::domain::stats::ports::Cache;

/// Health checks over the same cache adapter the stats service uses.
#[derive(Debug, Clone)]
pub struct Service<C> {
    cache: C,
}

impl<C: Cache> Service<C> {
    /// Creates a health service over `cache`.
    pub fn new(cache: C) -> Self {
        Self { cache }
    }
}

impl<C: Cache> HealthService for Service<C> {
    fn cache_backend(&self) -> &'static str {
        self.cache.backend()
    }

    async fn check_cache(&self) -> Result<(), HealthCheckFailed> {
        self.cache
            .ping()
            .await
            .map_err(|error| HealthCheckFailed(anyhow::Error::new(error)))
    }
}
