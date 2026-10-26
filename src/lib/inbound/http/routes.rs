//! Paths and the routes behind them.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode, header};
use axum::routing::get;
use tower_governor::GovernorLayer;
use tower_governor::errors::GovernorError;
use tower_governor::governor::GovernorConfigBuilder;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::inbound::http::AppState;
use crate::inbound::http::handlers::{health, stats};
use crate::inbound::http::middleware::CfConnectingIpKeyExtractor;

// == paths ==

/// Package name, version, and status.
pub const ROOT_ROUTE: &str = "/";
/// Server liveness.
pub const HEALTH_ROUTE: &str = "/health";
/// Cache backend and whether it answers.
pub const CACHE_HEALTH_ROUTE: &str = "/health/cache";
/// Stats for every configured repository.
pub const STATS_ROUTE: &str = "/stats";
/// Stats for one configured repository.
pub const REPO_STATS_ROUTE: &str = "/stats/{owner}/{name}";

// == rate limit ==

/// Requests one client may make in a burst.
const BURST: u32 = 30;
/// Seconds for one request of the burst to come back.
const REPLENISH_SECS: u64 = 2;

/// The 429 body. Fixed text, so the response is the same for every client.
fn too_many_requests(wait_secs: u64, headers: Option<HeaderMap>) -> Response<Body> {
    let mut response = Response::new(Body::from("too many requests, try again in a minute"));
    *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
    if let Some(headers) = headers {
        response.headers_mut().extend(headers);
    }
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from(wait_secs));
    response
}

fn rate_limited(error: GovernorError) -> Response<Body> {
    match error {
        GovernorError::TooManyRequests { wait_time, headers } => {
            too_many_requests(wait_time, headers)
        }
        other => other.into_response().map(Body::from),
    }
}

// == edge cache ==

/// How long Cloudflare's edge and browsers may hold a stats response.
const STATS_CACHE_CONTROL: &str = "public, max-age=300";

/// Sets `Cache-Control` on successful responses only. An error held at the edge for
/// [`STATS_CACHE_CONTROL`]'s `max-age` outlives whatever caused it.
fn cache_successes(response: &Response<Body>) -> Option<HeaderValue> {
    response
        .status()
        .is_success()
        .then(|| HeaderValue::from_static(STATS_CACHE_CONTROL))
}

type CacheHeaderFn = fn(&Response<Body>) -> Option<HeaderValue>;

// == routes ==

/// Every route. All are public and read-only.
///
/// # Errors
///
/// When the rate limiter rejects its configuration, which it does for a zero
/// `BURST` or `REPLENISH_SECS`.
pub fn routes() -> anyhow::Result<Router<AppState>> {
    let limit = Arc::new(
        GovernorConfigBuilder::default()
            .period(Duration::from_secs(REPLENISH_SECS))
            .burst_size(BURST)
            .key_extractor(CfConnectingIpKeyExtractor)
            .finish()
            .context("rate limit config: burst and period must be non-zero")?,
    );
    spawn_pruning(limit.limiter(), PRUNE_EVERY, |limiter| {
        limiter.retain_recent();
    });

    let cache_control: CacheHeaderFn = cache_successes;
    let stats_routes = Router::new()
        .route(STATS_ROUTE, get(stats::get_portfolio_stats))
        .route(REPO_STATS_ROUTE, get(stats::get_repo_stats))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            cache_control,
        ));

    Ok(Router::new()
        .route(ROOT_ROUTE, get(health::root))
        .route(HEALTH_ROUTE, get(health::health))
        .route(CACHE_HEALTH_ROUTE, get(health::cache_health))
        .merge(stats_routes)
        .layer(GovernorLayer::new(limit).error_handler(rate_limited)))
}

/// How often the rate limiter forgets addresses it has not seen for a full
/// replenish.
const PRUNE_EVERY: Duration = Duration::from_mins(1);

/// Calls `prune` on `limiter` every `every` until the limiter is dropped. The task
/// holds only a weak reference, so it ends with the router. Outside a tokio runtime
/// there is nothing to spawn on and nothing is pruned.
fn spawn_pruning<T: Send + Sync + 'static>(
    limiter: &Arc<T>,
    every: Duration,
    prune: impl Fn(&T) + Send + 'static,
) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let limiter = Arc::downgrade(limiter);
    runtime.spawn(async move {
        let mut ticks = tokio::time::interval(every);
        // The first tick is immediate, and there is nothing to prune yet.
        ticks.tick().await;
        loop {
            ticks.tick().await;
            let Some(limiter) = limiter.upgrade() else {
                break;
            };
            prune(&limiter);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::*;

    #[tokio::test]
    async fn idle_rate_limit_keys_are_pruned() {
        let config = GovernorConfigBuilder::default()
            .per_millisecond(1)
            .burst_size(1)
            .key_extractor(CfConnectingIpKeyExtractor)
            .finish()
            .unwrap();
        let limiter = config.limiter();
        for last in 1..=3 {
            let ip = IpAddr::from([10, 0, 0, last]);
            assert!(limiter.check_key(&ip).is_ok(), "first request from {ip}");
        }
        assert_eq!(limiter.len(), 3);

        spawn_pruning(limiter, Duration::from_millis(5), |limiter| {
            limiter.retain_recent();
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(limiter.len(), 0, "idle addresses were kept");
    }
}
