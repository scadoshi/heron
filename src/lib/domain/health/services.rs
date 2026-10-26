use crate::domain::health::models::HealthCheckFailed;
use crate::domain::health::ports::HealthService;
use crate::domain::stats::ports::StatsCache;

/// Health checks over the same cache adapter the stats service uses.
#[derive(Debug, Clone)]
pub struct Service<C> {
    cache: C,
}

impl<C: StatsCache> Service<C> {
    /// Creates a health service over `cache`.
    pub fn new(cache: C) -> Self {
        Self { cache }
    }
}

impl<C: StatsCache> HealthService for Service<C> {
    fn cache_backend(&self) -> &'static str {
        self.cache.backend()
    }

    async fn check_cache(&self) -> Result<(), HealthCheckFailed> {
        self.cache.ping().await.map_err(|e| HealthCheckFailed(e.0))
    }
}
