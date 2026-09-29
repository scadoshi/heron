//! Paths and the routes behind them.

use crate::inbound::http::{
    AppState,
    handlers::{
        health::{is_cache_running, is_server_running, root},
        stats::{get_portfolio_stats::get_portfolio_stats, get_repo_stats::get_repo_stats},
    },
    middleware::CfConnectingIpKeyExtractor,
};
use anyhow::Context;
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, HeaderValue, Response, StatusCode, header},
    routing::get,
};
use std::sync::Arc;
use tower_governor::{GovernorLayer, errors::GovernorError, governor::GovernorConfigBuilder};
use tower_http::set_header::SetResponseHeaderLayer;

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
    if let Ok(value) = HeaderValue::from_str(&wait_secs.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
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
/// five minutes outlives whatever caused it.
fn cache_successes(response: &Response<Body>) -> Option<HeaderValue> {
    response
        .status()
        .is_success()
        .then(|| HeaderValue::from_static(STATS_CACHE_CONTROL))
}

type CacheHeaderFn = fn(&Response<Body>) -> Option<HeaderValue>;

// == routes ==

/// Every route. All are public and read-only.
pub fn routes() -> anyhow::Result<Router<AppState>> {
    let limit = Arc::new(
        GovernorConfigBuilder::default()
            .period(std::time::Duration::from_secs(REPLENISH_SECS))
            .burst_size(BURST)
            .key_extractor(CfConnectingIpKeyExtractor)
            .finish()
            .context("rate limit config: burst and period must be non-zero")?,
    );

    let cache_control: CacheHeaderFn = cache_successes;
    let stats = Router::new()
        .route(STATS_ROUTE, get(get_portfolio_stats))
        .route(REPO_STATS_ROUTE, get(get_repo_stats))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            cache_control,
        ));

    Ok(Router::new()
        .route(ROOT_ROUTE, get(root))
        .route(HEALTH_ROUTE, get(is_server_running))
        .route(CACHE_HEALTH_ROUTE, get(is_cache_running))
        .merge(stats)
        .layer(GovernorLayer::new(limit).error_handler(rate_limited)))
}
