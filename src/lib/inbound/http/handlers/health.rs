//! `GET /`, `GET /health` and `GET /health/cache`.

use axum::Json;
use axum::extract::State;

use crate::inbound::http::AppState;
use crate::inbound::http::contracts::health::{HttpCacheHealth, HttpHealth, HttpRoot};

/// Package name, version, and status.
pub async fn root() -> Json<HttpRoot> {
    Json(HttpRoot {
        message: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        status: "ready",
    })
}

/// Healthy if the server can answer at all.
pub async fn health() -> Json<HttpHealth> {
    Json(HttpHealth {
        status: "healthy",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Which cache backend is in use and whether it answers a ping.
pub async fn cache_health(State(state): State<AppState>) -> Json<HttpCacheHealth> {
    let status = match state.health_service.check_cache().await {
        Ok(()) => "healthy",
        Err(error) => {
            tracing::warn!("cache ping failed: {error}");
            "unreachable"
        }
    };
    Json(HttpCacheHealth {
        backend: state.health_service.cache_backend(),
        status,
    })
}
