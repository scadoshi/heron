//! A value larger than one read. `#[ignore]`, and not part of CI.
//!
//! steller at 8628070 fails this: a command longer than 1024 bytes is answered
//! `ERR missing crlf terminator`. Redis passes it. See
//! `context/architecture/decisions.md`.
//!
//! ```sh
//! STELLER_ADDRESS=127.0.0.1:3000 cargo test --test live_steller_large -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use scotland::{
    domain::stats::{
        models::{cache_key::CacheKey, repo_name::RepoName},
        ports::StatsCache,
    },
    outbound::cache::steller::StellerCache,
};
use std::time::Duration;

#[tokio::test]
#[ignore = "needs a running steller, and fails until steller reads commands past 1024 bytes"]
async fn a_value_larger_than_one_read_round_trips() {
    let address = std::env::var("STELLER_ADDRESS").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let cache =
        StellerCache::new(address.parse().expect("STELLER_ADDRESS must be ip:port")).unwrap();
    let key = CacheKey::for_repo(&RepoName::new("live-test/large").unwrap());
    for size in [1000, 4096, 256 * 1024] {
        let value = vec![0xAB; size];
        cache
            .set(&key, &value, Duration::from_secs(5))
            .await
            .unwrap_or_else(|error| panic!("{size} bytes: {error}"));
        assert_eq!(cache.get(&key).await.unwrap(), Some(value), "{size} bytes");
    }
}
