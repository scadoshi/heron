//! In-process cache.

use crate::domain::stats::{
    models::{cache_key::CacheKey, errors::CacheError},
    ports::StatsCache,
};
use std::{
    collections::HashMap,
    future::{Future, ready},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

/// Bytes and the instant they expire, by key.
type Entries = HashMap<String, (Instant, Vec<u8>)>;

/// A map from key to bytes with a deadline per entry.
///
/// Expiry is lazy: an entry past its deadline is dropped when it is next read. The
/// service stores one entry per allowlisted repository, so the map is bounded by the
/// allowlist and nothing sweeps it.
#[derive(Debug, Clone, Default)]
pub struct MemoryCache {
    entries: Arc<Mutex<Entries>>,
}

impl MemoryCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// A panic while holding the lock cannot leave the map half-written, since every
    /// critical section is a single insert or remove. So a poisoned lock is taken
    /// anyway.
    fn entries(&self) -> MutexGuard<'_, Entries> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lookup(&self, key: &CacheKey) -> Option<Vec<u8>> {
        let mut entries = self.entries();
        match entries.get(&**key) {
            Some((deadline, value)) if Instant::now() < *deadline => Some(value.clone()),
            Some(_) => {
                entries.remove(&**key);
                None
            }
            None => None,
        }
    }

    fn store(&self, key: &CacheKey, value: &[u8], retain: Duration) -> Result<(), CacheError> {
        let deadline = Instant::now()
            .checked_add(retain)
            .ok_or_else(|| CacheError(anyhow::anyhow!("retain window overflows the clock")))?;
        self.entries()
            .insert(key.to_string(), (deadline, value.to_vec()));
        Ok(())
    }
}

/// Nothing here waits on anything, so each method does its work when called and
/// returns a future that is already complete.
impl StatsCache for MemoryCache {
    fn backend(&self) -> &'static str {
        "memory"
    }

    fn get(
        &self,
        key: &CacheKey,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, CacheError>> + Send {
        ready(Ok(self.lookup(key)))
    }

    fn set(
        &self,
        key: &CacheKey,
        value: &[u8],
        retain: Duration,
    ) -> impl Future<Output = Result<(), CacheError>> + Send {
        ready(self.store(key, value, retain))
    }

    fn ping(&self) -> impl Future<Output = Result<(), CacheError>> + Send {
        ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::repo;

    fn key() -> CacheKey {
        CacheKey::for_repo(&repo("a/b"))
    }

    #[tokio::test]
    async fn returns_what_was_stored() {
        let cache = MemoryCache::new();
        cache
            .set(&key(), b"\x00\xffbytes\r\n", Duration::from_mins(1))
            .await
            .unwrap();
        assert_eq!(
            cache.get(&key()).await.unwrap().as_deref(),
            Some(&b"\x00\xffbytes\r\n"[..])
        );
    }

    #[tokio::test]
    async fn an_absent_key_is_none() {
        assert_eq!(MemoryCache::new().get(&key()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_value_expires_at_its_deadline() {
        let cache = MemoryCache::new();
        cache
            .set(&key(), b"v", Duration::from_millis(20))
            .await
            .unwrap();
        assert!(cache.get(&key()).await.unwrap().is_some());
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(cache.get(&key()).await.unwrap(), None);
        assert!(cache.entries().is_empty());
    }

    #[tokio::test]
    async fn a_second_set_replaces_the_value_and_the_deadline() {
        let cache = MemoryCache::new();
        cache
            .set(&key(), b"old", Duration::from_millis(20))
            .await
            .unwrap();
        cache
            .set(&key(), b"new", Duration::from_mins(1))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(
            cache.get(&key()).await.unwrap().as_deref(),
            Some(&b"new"[..])
        );
    }
}
