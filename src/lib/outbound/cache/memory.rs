//! In-process cache, a map from key to bytes.

use std::collections::HashMap;
use std::future::{self, Future};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::domain::stats::models::cache_key::CacheKey;
use crate::domain::stats::models::errors::CacheError;
use crate::domain::stats::ports::Cache;

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

    /// The map, or an error once a panic while holding the lock has poisoned it.
    fn entries(&self) -> Result<MutexGuard<'_, Entries>, CacheError> {
        self.entries
            .lock()
            .map_err(|_| CacheError(anyhow::anyhow!("memory cache lock poisoned")))
    }

    fn lookup(&self, key: &CacheKey) -> Result<Option<Vec<u8>>, CacheError> {
        let mut entries = self.entries()?;
        Ok(match entries.get(&**key) {
            Some((deadline, value)) if Instant::now() < *deadline => Some(value.clone()),
            Some(_) => {
                entries.remove(&**key);
                None
            }
            None => None,
        })
    }

    fn store(&self, key: &CacheKey, value: &[u8], retain: Duration) -> Result<(), CacheError> {
        let deadline = Instant::now()
            .checked_add(retain)
            .ok_or_else(|| CacheError(anyhow::anyhow!("retain window overflows the clock")))?;
        self.entries()?
            .insert(key.to_string(), (deadline, value.to_vec()));
        Ok(())
    }
}

/// Nothing here waits on anything, so each method does its work when called and
/// returns a future that is already complete.
impl Cache for MemoryCache {
    fn backend(&self) -> &'static str {
        "memory"
    }

    fn get(
        &self,
        key: &CacheKey,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, CacheError>> + Send {
        future::ready(self.lookup(key))
    }

    fn set(
        &self,
        key: &CacheKey,
        value: &[u8],
        retain: Duration,
    ) -> impl Future<Output = Result<(), CacheError>> + Send {
        future::ready(self.store(key, value, retain))
    }

    /// Fails once the lock is poisoned, since `get` and `set` then fail too.
    fn ping(&self) -> impl Future<Output = Result<(), CacheError>> + Send {
        future::ready(self.entries().map(|_| ()))
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

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
        assert_eq!(cache.entries().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn a_poisoned_lock_fails_every_call_instead_of_panicking() {
        let cache = MemoryCache::new();
        let held = cache.clone();
        let poisoner = thread::spawn(move || {
            let _guard = held.entries.lock().unwrap();
            panic!("poisoning the lock on purpose");
        });
        assert!(poisoner.join().is_err(), "the thread should have panicked");
        assert!(cache.entries.is_poisoned());

        let read = cache.get(&key()).await;
        assert!(read.is_err(), "get on a poisoned lock: {read:?}");
        let write = cache.set(&key(), b"v", Duration::from_mins(1)).await;
        assert!(write.is_err(), "set on a poisoned lock: {write:?}");
        let ping = cache.ping().await;
        assert!(ping.is_err(), "ping on a poisoned lock: {ping:?}");
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
