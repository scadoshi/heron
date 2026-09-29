//! The steller adapter against a running steller. Every test is `#[ignore]`.
//!
//! ```sh
//! STELLER_ADDRESS=127.0.0.1:3000 cargo test --test live_steller -- --ignored
//! ```
//!
//! The same tests pass against Redis, which is what Redis-compatible means.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::{StubSource, TestClock, repo};
use heron::{
    domain::stats::{
        models::cache_key::CacheKey,
        ports::{StatsCache, StatsService},
        services::{Service, Settings},
    },
    outbound::cache::steller::StellerCache,
};
use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn steller() -> StellerCache {
    let address = std::env::var("STELLER_ADDRESS").unwrap_or_else(|_| "127.0.0.1:3000".into());
    StellerCache::new(address.parse().expect("STELLER_ADDRESS must be ip:port")).unwrap()
}

/// A repository name no other run or test has used, so runs do not read each
/// other's keys.
fn unique_repo() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "live-test/run-{nanos}-{}",
        NEXT.fetch_add(1, Ordering::SeqCst)
    )
}

fn unique_key() -> CacheKey {
    CacheKey::for_repo(&repo(&unique_repo()))
}

#[tokio::test]
#[ignore = "needs a running steller"]
async fn ping_answers() {
    steller().ping().await.unwrap();
}

#[tokio::test]
#[ignore = "needs a running steller"]
async fn an_absent_key_is_none() {
    assert_eq!(steller().get(&unique_key()).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "needs a running steller"]
async fn set_then_get_round_trips_a_binary_payload() {
    let cache = steller();
    let key = unique_key();
    let value: Vec<u8> = (0..=255u8).chain(*b"\r\n$5\r\n+OK\r\n").collect();
    cache
        .set(&key, &value, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(cache.get(&key).await.unwrap(), Some(value));
}

#[tokio::test]
#[ignore = "needs a running steller"]
async fn an_empty_value_is_stored_and_is_not_absence() {
    let cache = steller();
    let key = unique_key();
    cache.set(&key, b"", Duration::from_secs(5)).await.unwrap();
    assert_eq!(cache.get(&key).await.unwrap(), Some(Vec::new()));
}

#[tokio::test]
#[ignore = "needs a running steller"]
async fn a_second_set_replaces_the_value() {
    let cache = steller();
    let key = unique_key();
    cache
        .set(&key, b"old", Duration::from_secs(5))
        .await
        .unwrap();
    cache
        .set(&key, b"new", Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(cache.get(&key).await.unwrap().as_deref(), Some(&b"new"[..]));
}

#[tokio::test]
#[ignore = "needs a running steller"]
async fn the_key_is_gone_after_its_window() {
    let cache = steller();
    let key = unique_key();
    cache
        .set(&key, b"short-lived", Duration::from_millis(150))
        .await
        .unwrap();
    assert!(cache.get(&key).await.unwrap().is_some());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(cache.get(&key).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "needs a running steller"]
async fn a_snapshot_sized_value_round_trips() {
    let cache = steller();
    let key = unique_key();
    let value = vec![0xAB; 600];
    cache
        .set(&key, &value, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(cache.get(&key).await.unwrap(), Some(value));
}

/// Holds whether the server takes the large value or rejects it.
#[tokio::test]
#[ignore = "needs a running steller"]
async fn the_command_after_a_large_value_is_answered_correctly() {
    let cache = steller();
    let small = unique_key();
    cache
        .set(&small, b"small", Duration::from_secs(5))
        .await
        .unwrap();

    let large = unique_key();
    let stored = cache
        .set(&large, &vec![0xAB; 256 * 1024], Duration::from_secs(5))
        .await
        .is_ok();

    assert_eq!(
        cache.get(&small).await.unwrap().as_deref(),
        Some(&b"small"[..])
    );
    assert_eq!(cache.get(&large).await.unwrap().is_some(), stored);
    cache.ping().await.unwrap();
}

#[tokio::test]
#[ignore = "needs a running steller"]
async fn many_tasks_share_the_connection() {
    let cache = steller();
    let tasks: Vec<_> = (0..32)
        .map(|n: u32| {
            let cache = cache.clone();
            tokio::spawn(async move {
                let key = unique_key();
                let value = n.to_string().into_bytes();
                cache
                    .set(&key, &value, Duration::from_secs(5))
                    .await
                    .unwrap();
                assert_eq!(cache.get(&key).await.unwrap(), Some(value));
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }
}

#[tokio::test]
#[ignore = "needs a running steller"]
async fn the_service_reads_back_what_it_stored_in_steller() {
    let name = unique_repo();
    let source = StubSource::default();
    let settings = || Settings {
        repos: vec![repo(&name)],
        fresh: Duration::from_mins(1),
        retain: Duration::from_mins(2),
    };
    let first = Service::new(source.clone(), steller(), TestClock::default(), settings());
    let report = first.repo_stats(&repo(&name)).await.unwrap();

    // A second service with its own connection, as after a restart.
    let second = Service::new(source.clone(), steller(), TestClock::default(), settings());
    let again = second.repo_stats(&repo(&name)).await.unwrap();

    assert_eq!(again, report);
    assert_eq!(source.calls(), 1);
}
