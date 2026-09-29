# Structure

One crate. The library is `heron` (`src/lib/lib.rs`) and the binary is `heron` (`src/bin/heron.rs`).

## Layers

```
inbound/http  ──→  domain  ←──  outbound
                     ↑
                   config, bin
```

The arrow is the direction of `use`. `domain` imports from neither side. `inbound` and `outbound` import from `domain` and never from each other. The binary imports from all three, because wiring them together is its whole job.

| Layer | Holds | Knows about |
|---|---|---|
| `domain/` | `RepoName`, `Secret`, `CacheKey`, `RepoStats`, `Snapshot`, the ports, the two services | nothing outside itself |
| `inbound/http/` | `ApiError`, `AppState`, the router, handlers, `Http*` contracts | axum, and the domain's service ports |
| `outbound/` | `GitHub`, `MemoryCache`, `StellerCache`, `LayeredCache`, `SystemClock` | reqwest, TCP, the OS clock, and the domain's ports |
| `config.rs` | `Config`, `CacheBackend` | the environment |

## Ports

| Port | Defined in | Implemented by |
|---|---|---|
| `Clock` | `domain/clock/mod.rs` | `outbound::clock::SystemClock` |
| `StatsSource` | `domain/stats/ports.rs` | `outbound::github::GitHub` |
| `StatsCache` | `domain/stats/ports.rs` | `MemoryCache`, `StellerCache`, `LayeredCache` |
| `StatsService` | `domain/stats/ports.rs` | `domain::stats::services::Service` |
| `HealthService` | `domain/health/ports.rs` | `domain::health::services::Service` |

`StatsService` and `HealthService` each have an object-safe twin, `ErasedStatsService` and `ErasedHealthService`, which is what `AppState` holds. See "Type-erased services" in `decisions.md`.

## One request, end to end

`GET /stats/scadoshi/steller`, with nothing cached.

1. The request passes the middleware in `build_router`: request id, trace span, catch-panic, compression, CORS, security headers, timeout, body limit. Then the rate limiter, keyed by `CfConnectingIpKeyExtractor`.
2. `get_repo_stats` builds a `RepoName` from the two path segments. A name that does not parse is answered 404 here.
3. The handler calls `state.stats_service.repo_stats(&repo)`. That is a call through `Arc<dyn ErasedStatsService>`, forwarded by the blanket impl to `Service::repo_stats`.
4. `Service::resolve` looks the repository up in its refresh locks. Not there means not in the allowlist, and the answer is `StatsError::UnknownRepo` with GitHub never asked.
5. It reads `CacheKey::for_repo(&repo)` from the cache. Nothing is there.
6. It takes that repository's refresh lock and reads the cache again, since another request may have filled it meanwhile.
7. It calls `StatsSource::repo_stats`. `GitHub::read` asks for the repository, refuses if it is private, then asks for commits, languages and contributor statistics concurrently.
8. The service wraps the result in a `Snapshot` with `fetched_at` from the clock and `fresh_until` one fresh window later, serializes it, and stores it with the retain window.
9. It returns a `RepoReport`. The handler converts that into `HttpRepoStats`.
10. On the way out the stats routes add `Cache-Control: public, max-age=300`, since the response is a success.

The next request inside the fresh window stops at step 5.

## Freshness and retention

Two windows, and the difference between them is the point.

`STATS_FRESH_SECS` is how long a snapshot is served without asking GitHub. It is written into the snapshot as `fresh_until`. `STATS_RETAIN_SECS` is how long the cache keeps the bytes, passed to `StatsCache::set`.

Between the two, a snapshot is stale but still there. When a refresh fails in that stretch, the stale snapshot is served with `stale: true`. Config refuses to start unless the retain window is the longer one.

Only a source that could not answer falls back this way: `RateLimited` and `Upstream`. `PrivateRepo` and `UnknownRepo` do not, because a repository that went private has to stop being served.

## Tests

| Where | What | Needs |
|---|---|---|
| `#[cfg(test)]` beside the code | units, against the fakes in `src/lib/test_support.rs` | nothing |
| `src/lib/outbound/github/tests.rs` | the GitHub adapter against an in-process fake API | nothing |
| `tests/health.rs`, `tests/stats.rs` | the real router, service and memory cache, through `oneshot` | nothing |
| `tests/binary.rs` | the real executable, spawned | nothing |
| `tests/live_steller.rs` | the steller adapter | a running steller |
| `tests/live_steller_large.rs` | the reproduction of steller's 1024-byte limit | a running steller, and fails against it today |

More in `development/testing.md`.
