use crate::inbound::http::{
    AppState,
    contracts::health::{HttpCacheHealth, HttpHealth, HttpRoot},
};
use axum::{Json, extract::State};

/// Package name, version, and status.
pub async fn root() -> Json<HttpRoot> {
    Json(HttpRoot {
        message: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        status: "ready",
    })
}

/// Healthy if the server can answer at all.
pub async fn is_server_running() -> Json<HttpHealth> {
    Json(HttpHealth {
        status: "healthy",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Which cache backend is in use and whether it answers a ping.
pub async fn is_cache_running(State(state): State<AppState>) -> Json<HttpCacheHealth> {
    let status = match state.health_service.check_cache().await {
        Ok(()) => "healthy",
        Err(error) => {
            tracing::warn!("{error}");
            "unreachable"
        }
    };
    Json(HttpCacheHealth {
        backend: state.health_service.cache_backend(),
        status,
    })
}
