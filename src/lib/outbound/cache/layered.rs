//! Two caches stacked: a primary, and a secondary that covers for it.

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::domain::stats::models::cache_key::CacheKey;
use crate::domain::stats::models::errors::CacheError;
use crate::domain::stats::ports::Cache;

/// How long a primary that failed is skipped. A hung primary costs its timeouts on
/// every call, and calls queue behind one another, so asking it on every request
/// stalls them all.
const PRIMARY_COOLDOWN: Duration = Duration::from_secs(30);

/// A primary cache with a secondary behind it.
///
/// Reads try the primary and fall through to the secondary on a miss or an error.
/// Writes go to both, and a value read from the primary is copied into the secondary,
/// so the secondary holds what was last served when the primary goes away. A write
/// fails only when both do. A primary that fails is skipped for `PRIMARY_COOLDOWN`, so
/// a hung one costs one timeout every cooldown instead of one on every call.
///
/// A key written while the primary missed the write is read from the secondary
/// until a later write reaches the primary, so a primary that comes back with an
/// older value does not replace the newer one.
#[derive(Clone)]
pub struct LayeredCache<P, S> {
    primary: P,
    secondary: S,
    /// How long the secondary keeps a value copied from the primary. The primary does
    /// not say how long its own copy has left, so this is the full window again.
    retain: Duration,
    /// Whether the primary failed last time it was asked. Logging keys off the
    /// change, so a dead primary is reported once and its recovery once.
    primary_down: Arc<AtomicBool>,
    /// How long a failed primary is left alone before it is asked again.
    cooldown: Duration,
    /// What the retry time counts from.
    started: Instant,
    /// Milliseconds after `started` before which a down primary is not asked.
    retry_at: Arc<AtomicU64>,
    /// Keys whose newest value only the secondary holds.
    ahead_of_primary: Arc<Mutex<HashSet<String>>>,
}

/// Prints the layers and settings; the key set is not locked to print it.
impl<P: fmt::Debug, S: fmt::Debug> fmt::Debug for LayeredCache<P, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayeredCache")
            .field("primary", &self.primary)
            .field("secondary", &self.secondary)
            .field("retain", &self.retain)
            .field("cooldown", &self.cooldown)
            .finish_non_exhaustive()
    }
}

impl<P: Cache, S: Cache> LayeredCache<P, S> {
    /// Stacks `primary` over `secondary`.
    pub fn new(primary: P, secondary: S, retain: Duration) -> Self {
        Self {
            primary,
            secondary,
            retain,
            primary_down: Arc::new(AtomicBool::new(false)),
            cooldown: PRIMARY_COOLDOWN,
            started: Instant::now(),
            retry_at: Arc::new(AtomicU64::new(0)),
            ahead_of_primary: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// The keys whose newest value only the secondary holds.
    fn ahead(&self) -> Result<MutexGuard<'_, HashSet<String>>, CacheError> {
        self.ahead_of_primary
            .lock()
            .map_err(|_| CacheError(anyhow!("layered cache key set lock poisoned")))
    }

    /// Records whether the secondary holds a newer value for `key` than the primary.
    fn set_ahead(&self, key: &CacheKey, ahead: bool) -> Result<(), CacheError> {
        let mut keys = self.ahead()?;
        if ahead {
            keys.insert(key.to_string());
        } else {
            keys.remove(&**key);
        }
        Ok(())
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Whether the primary is down and still inside its cooldown.
    fn primary_resting(&self) -> bool {
        self.primary_down.load(Ordering::Relaxed)
            && self.elapsed_ms() < self.retry_at.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn with_cooldown(mut self, cooldown: Duration) -> Self {
        self.cooldown = cooldown;
        self
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
        let cooldown = u64::try_from(self.cooldown.as_millis()).unwrap_or(u64::MAX);
        self.retry_at.store(
            self.elapsed_ms().saturating_add(cooldown),
            Ordering::Relaxed,
        );
        if !self.primary_down.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                "{} cache is down, serving from {}: {error}",
                self.primary.backend(),
                self.secondary.backend()
            );
        }
    }
}

impl<P: Cache, S: Cache> Cache for LayeredCache<P, S> {
    fn backend(&self) -> &'static str {
        "layered"
    }

    async fn get(&self, key: &CacheKey) -> Result<Option<Vec<u8>>, CacheError> {
        if self.primary_resting() {
            return self.secondary.get(key).await;
        }
        let ahead = self.ahead()?.contains(&**key);
        if ahead {
            match self.secondary.get(key).await {
                Ok(Some(value)) => return Ok(Some(value)),
                // The newer copy is gone, so the primary's is the newest there is.
                Ok(None) => self.set_ahead(key, false)?,
                Err(error) => tracing::debug!(
                    "{} failed a read it holds the newest value for: {error}",
                    self.secondary.backend()
                ),
            }
        }
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
        if self.primary_resting() {
            self.secondary.set(key, value, retain).await?;
            return self.set_ahead(key, true);
        }
        let primary = self.primary.set(key, value, retain).await;
        match &primary {
            Ok(()) => self.primary_answered(),
            Err(error) => self.primary_failed(error),
        }
        let secondary = self.secondary.set(key, value, retain).await;
        self.set_ahead(key, primary.is_err() && secondary.is_ok())?;
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
        layered_with(Duration::ZERO)
    }

    fn layered_with(
        cooldown: Duration,
    ) -> (LayeredCache<FakeCache, FakeCache>, FakeCache, FakeCache) {
        let primary = FakeCache::default();
        let secondary = FakeCache::default();
        let cache = LayeredCache::new(primary.clone(), secondary.clone(), Duration::from_mins(5))
            .with_cooldown(cooldown);
        (cache, primary, secondary)
    }

    #[tokio::test]
    async fn a_failed_primary_is_left_alone_until_its_cooldown_passes() {
        let (cache, primary, secondary) = layered_with(Duration::from_millis(200));
        secondary.put(&key(), b"v");
        primary.fail_reads(true);
        cache.get(&key()).await.unwrap();
        assert_eq!(primary.calls(), 1, "the first read asks the primary");

        let read = cache.get(&key()).await.unwrap();
        assert_eq!(
            read.as_deref(),
            Some(&b"v"[..]),
            "served from the secondary"
        );
        cache
            .set(&key(), b"w", Duration::from_mins(1))
            .await
            .unwrap();
        assert_eq!(
            primary.calls(),
            1,
            "the primary was asked during its cooldown"
        );
        let held = secondary.get(&key()).await.unwrap();
        assert_eq!(
            held.as_deref(),
            Some(&b"w"[..]),
            "the write reached the secondary"
        );

        tokio::time::sleep(Duration::from_millis(250)).await;
        primary.fail_reads(false);
        let other = CacheKey::for_repo(&repo("a/c"));
        cache.get(&other).await.unwrap();
        assert_eq!(
            primary.calls(),
            2,
            "the primary is asked again after the cooldown"
        );
        assert!(
            !cache.primary_down.load(Ordering::Relaxed),
            "an answer after the cooldown clears the down flag"
        );
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
    async fn a_write_the_primary_missed_is_not_undone_when_it_recovers() {
        let (cache, primary, secondary) = layered();
        primary.put(&key(), b"old");
        primary.fail_writes(true);
        cache
            .set(&key(), b"new", Duration::from_mins(1))
            .await
            .unwrap();
        primary.fail_writes(false);

        let read = cache.get(&key()).await.unwrap();
        assert_eq!(
            read.as_deref(),
            Some(&b"new"[..]),
            "the recovered primary's older value won"
        );
        let held = secondary.get(&key()).await.unwrap();
        assert_eq!(
            held.as_deref(),
            Some(&b"new"[..]),
            "the newer copy was overwritten"
        );

        cache
            .set(&key(), b"newer", Duration::from_mins(1))
            .await
            .unwrap();
        let read = cache.get(&key()).await.unwrap();
        assert_eq!(read.as_deref(), Some(&b"newer"[..]));
        let stored = primary.get(&key()).await.unwrap();
        assert_eq!(
            stored.as_deref(),
            Some(&b"newer"[..]),
            "a write reaching the primary again"
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
        let result = cache.set(&key(), b"v", Duration::from_mins(1)).await;
        assert!(
            result.is_err(),
            "both layers refused, yet the write answered {result:?}"
        );
    }

    #[tokio::test]
    async fn ping_reports_the_primary() {
        let (cache, primary, _) = layered();
        let result = cache.ping().await;
        assert!(result.is_ok(), "healthy primary: {result:?}");
        primary.fail_pings(true);
        let result = cache.ping().await;
        assert!(result.is_err(), "failing primary: {result:?}");
    }

    #[tokio::test]
    async fn the_down_flag_follows_the_primary() {
        let (cache, primary, _) = layered();
        primary.fail_reads(true);
        cache.get(&key()).await.unwrap();
        assert!(
            cache.primary_down.load(Ordering::Relaxed),
            "a failed read did not set the down flag"
        );
        primary.fail_reads(false);
        cache.get(&key()).await.unwrap();
        assert!(
            !cache.primary_down.load(Ordering::Relaxed),
            "a good read did not clear the down flag"
        );
    }
}
