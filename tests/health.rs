//! Root and health endpoints through the real router.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use axum::http::StatusCode;
use common::{DeadCache, TestApp};
use scotland::inbound::http::routes::{CACHE_HEALTH_ROUTE, HEALTH_ROUTE, ROOT_ROUTE};

#[tokio::test]
async fn root_names_the_package_and_version() {
    let reply = TestApp::new(&["a/b"]).get(ROOT_ROUTE).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(body["message"], "scotland-server");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["status"], "ready");
}

#[tokio::test]
async fn health_answers_without_touching_the_source() {
    let app = TestApp::new(&["a/b"]);
    let reply = app.get(HEALTH_ROUTE).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()["status"], "healthy");
    assert_eq!(app.source.calls(), 0);
}

#[tokio::test]
async fn cache_health_names_the_backend() {
    let reply = TestApp::new(&["a/b"]).get(CACHE_HEALTH_ROUTE).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(body["backend"], "memory");
    assert_eq!(body["status"], "healthy");
}

#[tokio::test]
async fn cache_health_reports_a_backend_that_does_not_answer() {
    let reply = TestApp::over(&["a/b"], DeadCache)
        .get(CACHE_HEALTH_ROUTE)
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(body["backend"], "dead");
    assert_eq!(body["status"], "unreachable");
}

#[tokio::test]
async fn health_responses_are_not_held_at_the_edge() {
    let app = TestApp::new(&["a/b"]);
    for path in [ROOT_ROUTE, HEALTH_ROUTE, CACHE_HEALTH_ROUTE] {
        assert_eq!(app.get(path).await.header("cache-control"), None, "{path}");
    }
}
