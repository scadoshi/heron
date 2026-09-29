//! Two caches stacked.

use crate::domain::stats::{
    models::{cache_key::CacheKey, errors::CacheError},
    ports::StatsCache,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// A primary cache with a secondary behind it.
///
/// Reads try the primary and fall through to the secondary on a miss or an error.
/// Writes go to both, and a value read from the primary is copied into the secondary,
/// so the secondary holds what was last served when the primary goes away. A write
/// fails only when both do.
#[derive(Debug, Clone)]
pub struct LayeredCache<P: StatsCache, S: StatsCache> {
    primary: P,
    secondary: S,
    /// How long the secondary keeps a value copied from the primary. The primary does
    /// not say how long its own copy has left, so this is the full window again.
    retain: Duration,
    /// Whether the primary failed last time it was asked. Logging keys off the
    /// change, so a dead primary is reported once and its recovery once.
    primary_down: Arc<AtomicBool>,
}

impl<P: StatsCache, S: StatsCache> LayeredCache<P, S> {
    /// Stacks `primary` over `secondary`.
    pub fn new(primary: P, secondary: S, retain: Duration) -> Self {
        Self {
            primary,
            secondary,
            retain,
            primary_down: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Stores `value` in the secondary unless it already holds exactly that.
    async fn copy_to_secondary(&self, key: &CacheKey, value: &[u8]) {
        if let Ok(Some(held)) = self.secondary.get(key).await
            && held == value
        {
            return;
        }
        if let Err(error) = self.secondary.set(key, value, self.retain).await {
            tracing::debug!("copy into {} failed: {error}", self.secondary.backend());
        }
    }

    fn primary_answered(&self) {
        if self.primary_down.swap(false, Ordering::Relaxed) {
            tracing::info!("{} cache is answering again", self.primary.backend());
        }
    }

    fn primary_failed(&self, error: &CacheError) {
        if !self.primary_down.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "{} cache is down, serving from {}: {error}",
                self.primary.backend(),
                self.secondary.backend()
            );
        }
    }
}

impl<P: StatsCache, S: StatsCache> StatsCache for LayeredCache<P, S> {
    fn backend(&self) -> &'static str {
        "layered"
    }

    async fn get(&self, key: &CacheKey) -> Result<Option<Vec<u8>>, CacheError> {
        match self.primary.get(key).await {
            Ok(Some(value)) => {
                self.primary_answered();
                self.copy_to_secondary(key, &value).await;
                return Ok(Some(value));
            }
            Ok(None) => self.primary_answered(),
            Err(error) => self.primary_failed(&error),
        }
        self.secondary.get(key).await
    }

    async fn set(&self, key: &CacheKey, value: &[u8], retain: Duration) -> Result<(), CacheError> {
        let primary = self.primary.set(key, value, retain).await;
        match &primary {
            Ok(()) => self.primary_answered(),
            Err(error) => self.primary_failed(error),
        }
        let secondary = self.secondary.set(key, value, retain).await;
        primary.or(secondary)
    }

    /// Reports the primary, so its health is visible while the secondary covers.
    async fn ping(&self) -> Result<(), CacheError> {
        self.primary.ping().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{FakeCache, repo};

    fn key() -> CacheKey {
        CacheKey::for_repo(&repo("a/b"))
    }

    fn layered() -> (LayeredCache<FakeCache, FakeCache>, FakeCache, FakeCache) {
        let primary = FakeCache::default();
        let secondary = FakeCache::default();
        (
            LayeredCache::new(primary.clone(), secondary.clone(), Duration::from_mins(5)),
            primary,
            secondary,
        )
    }

    #[tokio::test]
    async fn a_primary_hit_is_copied_into_the_secondary() {
        let (cache, primary, secondary) = layered();
        primary.put(&key(), b"v");
        assert_eq!(cache.get(&key()).await.unwrap().as_deref(), Some(&b"v"[..]));
        assert_eq!(
            secondary.get(&key()).await.unwrap().as_deref(),
            Some(&b"v"[..])
        );

        primary.fail_reads(true);
        assert_eq!(cache.get(&key()).await.unwrap().as_deref(), Some(&b"v"[..]));
    }

    #[tokio::test]
    async fn a_newer_primary_value_replaces_the_copy() {
        let (cache, primary, secondary) = layered();
        secondary.put(&key(), b"old");
        primary.put(&key(), b"new");
        cache.get(&key()).await.unwrap();
        assert_eq!(
            secondary.get(&key()).await.unwrap().as_deref(),
            Some(&b"new"[..])
        );
    }

    #[tokio::test]
    async fn writes_reach_both_layers() {
        let (cache, primary, secondary) = layered();
        cache
            .set(&key(), b"v", Duration::from_mins(1))
            .await
            .unwrap();
        assert_eq!(
            primary.get(&key()).await.unwrap().as_deref(),
            Some(&b"v"[..])
        );
        assert_eq!(
            secondary.get(&key()).await.unwrap().as_deref(),
            Some(&b"v"[..])
        );
    }

    #[tokio::test]
    async fn reads_prefer_the_primary() {
        let (cache, primary, secondary) = layered();
        primary.put(&key(), b"primary");
        secondary.put(&key(), b"secondary");
        assert_eq!(
            cache.get(&key()).await.unwrap().as_deref(),
            Some(&b"primary"[..])
        );
    }

    #[tokio::test]
    async fn a_failing_primary_falls_through_to_the_secondary() {
        let (cache, primary, _) = layered();
        cache
            .set(&key(), b"v", Duration::from_mins(1))
            .await
            .unwrap();
        primary.fail_reads(true);
        assert_eq!(cache.get(&key()).await.unwrap().as_deref(), Some(&b"v"[..]));
    }

    #[tokio::test]
    async fn a_primary_miss_falls_through_to_the_secondary() {
        let (cache, _, secondary) = layered();
        secondary.put(&key(), b"v");
        assert_eq!(cache.get(&key()).await.unwrap().as_deref(), Some(&b"v"[..]));
    }

    #[tokio::test]
    async fn a_write_succeeds_while_either_layer_takes_it() {
        let (cache, primary, secondary) = layered();
        primary.fail_writes(true);
        cache
            .set(&key(), b"v", Duration::from_mins(1))
            .await
            .unwrap();
        assert_eq!(
            secondary.get(&key()).await.unwrap().as_deref(),
            Some(&b"v"[..])
        );

        secondary.fail_writes(true);
        assert!(
            cache
                .set(&key(), b"v", Duration::from_mins(1))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn ping_reports_the_primary() {
        let (cache, primary, _) = layered();
        assert!(cache.ping().await.is_ok());
        primary.fail_pings(true);
        assert!(cache.ping().await.is_err());
    }

    #[tokio::test]
    async fn the_down_flag_follows_the_primary() {
        let (cache, primary, _) = layered();
        primary.fail_reads(true);
        cache.get(&key()).await.unwrap();
        assert!(cache.primary_down.load(Ordering::Relaxed));
        primary.fail_reads(false);
        cache.get(&key()).await.unwrap();
        assert!(!cache.primary_down.load(Ordering::Relaxed));
    }
}
