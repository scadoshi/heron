//! HTTP layer: Axum server, error mapping, middleware, and routes.

/// The JSON the API answers with.
pub mod contracts;
/// Request handlers.
pub mod handlers;
/// Rate-limit key extraction.
pub mod middleware;
/// Paths and the routes behind them.
pub mod routes;

use crate::domain::{
    health::ports::ErasedHealthService,
    stats::{
        models::{errors::StatsError, repo_name::InvalidRepoName},
        ports::ErasedStatsService,
    },
};
use anyhow::Context;
use axum::{
    extract::Request,
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{sync::Arc, time::Duration};
use thiserror::Error;
use tokio::net;
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    cors::{AllowOrigin, CorsLayer},
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
};

// == error ==

/// What a handler can fail with.
///
/// `IntoResponse` is the single exit path. `InternalServerError` and
/// `ServiceUnavailable` carry detail for the log and send a fixed body, so nothing
/// internal reaches a client. `NotFound` sends its message as is.
#[derive(Debug, Error, Clone)]
#[allow(missing_docs)]
pub enum ApiError {
    #[error("{0}")]
    InternalServerError(String),
    #[error("{0}")]
    ServiceUnavailable(String),
    #[error("{0}")]
    NotFound(String),
}

/// One message for every way a repository can be absent. A different message for a
/// private repository would confirm it exists.
const REPOSITORY_NOT_FOUND: &str = "repository not found";

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        Self::InternalServerError(format!("{value:#}"))
    }
}

impl From<StatsError> for ApiError {
    fn from(value: StatsError) -> Self {
        match value {
            StatsError::UnknownRepo(_) | StatsError::PrivateRepo(_) => {
                Self::NotFound(REPOSITORY_NOT_FOUND.to_string())
            }
            unavailable @ (StatsError::RateLimited { .. } | StatsError::Upstream(_)) => {
                Self::ServiceUnavailable(unavailable.to_string())
            }
        }
    }
}

/// A name that does not parse cannot be a configured repository.
impl From<InvalidRepoName> for ApiError {
    fn from(_: InvalidRepoName) -> Self {
        Self::NotFound(REPOSITORY_NOT_FOUND.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::InternalServerError(detail) => {
                tracing::error!("{detail}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                )
            }
            ApiError::ServiceUnavailable(detail) => {
                tracing::warn!("503 {detail}");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "stats are temporarily unavailable".to_string(),
                )
            }
            ApiError::NotFound(message) => {
                tracing::warn!("404 {message}");
                (StatusCode::NOT_FOUND, message)
            }
        };
        (status, message).into_response()
    }
}

async fn not_found() -> ApiError {
    ApiError::NotFound("not found".to_string())
}

// == security headers ==

/// Adds `X-Content-Type-Options`, `X-Frame-Options`, and `Referrer-Policy` to every
/// response.
async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    response
}

// == server ==

/// Bind address and CORS origins for the HTTP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpServerConfig<'a> {
    /// Address to bind the TCP listener to.
    pub bind_address: &'a str,
    /// Origins permitted by CORS policy.
    pub allowed_origins: Vec<HeaderValue>,
}

/// Shared application state.
///
/// Services are held as type-erased trait objects (see the `ErasedXService` twins in
/// each domain's ports) so handlers stay free of generic parameters.
#[derive(Clone)]
#[allow(missing_docs)]
pub struct AppState {
    pub stats_service: Arc<dyn ErasedStatsService>,
    pub health_service: Arc<dyn ErasedHealthService>,
}

/// Axum HTTP server with its routes and middleware in place.
pub struct HttpServer {
    router: axum::Router,
    listener: net::TcpListener,
}

/// Assembles the router and the middleware stack. Apart from `HttpServer::new` so
/// tests can drive the router with `tower::ServiceExt::oneshot` and no socket.
pub fn build_router(
    state: AppState,
    allowed_origins: Vec<HeaderValue>,
) -> anyhow::Result<axum::Router> {
    // The request id is set before the trace layer runs, so the span can carry it.
    let trace_layer = tower_http::trace::TraceLayer::new_for_http().make_span_with(
        |request: &axum::extract::Request<_>| {
            let uri = request.uri().to_string();
            let request_id = request
                .extensions()
                .get::<tower_http::request_id::RequestId>()
                .and_then(|id| id.header_value().to_str().ok())
                .unwrap_or("");
            tracing::info_span!(
                "http_request",
                method = ?request.method(),
                uri,
                request_id = %request_id,
            )
        },
    );

    // Layers wrap what is above them, so the last one added is the first a request
    // meets: SetRequestId, PropagateRequestId, trace, CatchPanic, Compression, Cors,
    // security_headers, Timeout, RequestBodyLimit.
    let x_request_id = header::HeaderName::from_static("x-request-id");
    Ok(axum::Router::new()
        .merge(routes::routes()?)
        .fallback(not_found)
        .layer(RequestBodyLimitLayer::new(16 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(axum::middleware::from_fn(security_headers))
        .layer(
            CorsLayer::new()
                .allow_origin(allow_origin(allowed_origins))
                .allow_methods([Method::GET]),
        )
        .layer(CompressionLayer::new())
        .layer(CatchPanicLayer::new())
        .layer(trace_layer)
        .layer(PropagateRequestIdLayer::new(x_request_id.clone()))
        .layer(SetRequestIdLayer::new(x_request_id, MakeRequestUuid))
        .with_state(state))
}

/// A single origin is sent on every response, whatever the request's `Origin`
/// header says. A CDN caches by URL and ignores `Vary`, so a copy filled by a
/// request without `Origin` (a curl, the deploy workflow) would otherwise carry
/// no CORS header and every browser served it would fail. More than one origin
/// has to be mirrored per request.
fn allow_origin(mut origins: Vec<header::HeaderValue>) -> AllowOrigin {
    if origins.len() == 1 {
        AllowOrigin::exact(origins.remove(0))
    } else {
        AllowOrigin::list(origins)
    }
}

impl HttpServer {
    /// Builds the router and binds the listener.
    ///
    /// The services arrive already erased: their concrete types depend on the cache
    /// backend chosen at startup, and the caller is where that is known.
    pub async fn new(
        stats_service: Arc<dyn ErasedStatsService>,
        health_service: Arc<dyn ErasedHealthService>,
        config: HttpServerConfig<'_>,
    ) -> anyhow::Result<Self> {
        let state = AppState {
            stats_service,
            health_service,
        };
        let router = build_router(state, config.allowed_origins)?;
        let listener = net::TcpListener::bind(&config.bind_address)
            .await
            .with_context(|| format!("failed to listen on {}", config.bind_address))?;
        Ok(Self { router, listener })
    }

    /// Serves requests until a shutdown signal arrives, then drains what is in
    /// flight.
    pub async fn run(self) -> anyhow::Result<()> {
        tracing::info!("listening on {}", self.listener.local_addr()?);
        axum::serve(
            self.listener,
            self.router
                .into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("received error from running server")?;
        Ok(())
    }
}

/// Resolves on SIGINT (Ctrl-C) or SIGTERM (systemd `stop`).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut stream) = signal(SignalKind::terminate()) {
            stream.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("shutdown signal received (SIGINT)"),
        () = terminate => tracing::info!("shutdown signal received (SIGTERM)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::repo;
    use http_body_util::BodyExt;

    async fn body_of(error: ApiError) -> (StatusCode, String) {
        let response = error.into_response();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn a_500_never_sends_its_detail() {
        let detail = "connection refused (os error 111)\nstack backtrace: ...";
        let (status, body) = body_of(ApiError::InternalServerError(detail.to_string())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, "internal server error");
    }

    #[tokio::test]
    async fn a_503_never_sends_its_detail() {
        let error = StatsError::Upstream(anyhow::anyhow!("error sending request for url (x)"));
        let (status, body) = body_of(error.into()).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, "stats are temporarily unavailable");

        let (status, body) = body_of(StatsError::RateLimited { reset_at: None }.into()).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, "stats are temporarily unavailable");
    }

    #[tokio::test]
    async fn every_absent_repository_answers_the_same() {
        let unknown = body_of(StatsError::UnknownRepo(repo("a/b")).into()).await;
        let private = body_of(StatsError::PrivateRepo(repo("a/secret")).into()).await;
        let invalid = body_of(InvalidRepoName::Reserved.into()).await;
        assert_eq!(
            unknown,
            (StatusCode::NOT_FOUND, "repository not found".into())
        );
        assert_eq!(unknown, private);
        assert_eq!(unknown, invalid);
    }

    #[tokio::test]
    async fn anyhow_maps_to_a_500_carrying_the_chain() {
        let error = anyhow::anyhow!("root cause").context("while doing the thing");
        match ApiError::from(error) {
            ApiError::InternalServerError(detail) => {
                assert!(detail.contains("while doing the thing"));
                assert!(detail.contains("root cause"));
            }
            other => panic!("expected InternalServerError, got {other:?}"),
        }
    }
}
