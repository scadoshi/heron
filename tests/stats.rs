//! Stats endpoints through the real router, service, and memory cache.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use axum::http::StatusCode;
use common::{ALLOWED_ORIGIN, DeadCache, FRESH_SECS, Outcome, TestApp, repo};
use heron::inbound::http::routes::STATS_ROUTE;
use serde_json::json;

#[tokio::test]
async fn repo_stats_has_the_documented_shape() {
    let reply = TestApp::new(&["scadoshi/steller"])
        .get("/stats/scadoshi/steller")
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.header("content-type"), Some("application/json"));
    assert_eq!(
        reply.json(),
        json!({
            "repo": "scadoshi/steller",
            "commits": 10,
            "stars": 2,
            "default_branch": "main",
            "pushed_at": "2026-09-29T15:58:02Z",
            "languages": [
                { "name": "Rust", "bytes": 9000 },
                { "name": "Shell", "bytes": 40 },
            ],
            "additions": 14115,
            "deletions": null,
            "fetched_at": "2026-09-29T16:04:41Z",
            "stale": false,
            "counts": null,
        })
    );
}

#[tokio::test]
async fn counts_appear_once_a_sweep_has_measured_the_repository() {
    let app = TestApp::new(&["scadoshi/steller"]);
    let sweep = app.counts.sweep().await;
    assert_eq!(sweep.measured.len(), 1);
    app.clock.advance(60);
    let reply = app.get("/stats/scadoshi/steller").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json()["counts"],
        json!({
            "language": "Rust",
            "lines": 6102,
            "tests": 242,
            "clippy_lints": 14,
            "measured_at": "2026-09-29T16:04:41Z",
        })
    );
}

#[tokio::test]
async fn portfolio_entries_carry_their_own_counts() {
    let app = TestApp::new(&["a/one", "a/two"]);
    app.counts.sweep().await;
    let body = app.get(STATS_ROUTE).await.json();
    assert_eq!(body["repos"][0]["counts"]["lines"], 6102);
    assert_eq!(body["repos"][1]["counts"]["tests"], 242);
}

#[tokio::test]
async fn portfolio_stats_has_the_documented_shape() {
    let app = TestApp::new(&["a/one", "a/two", "a/three"]);
    app.source.answer(&repo("a/two"), Outcome::Commits(32));
    app.source.answer(&repo("a/three"), Outcome::Broken("down"));
    let reply = app.get(STATS_ROUTE).await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(body["generated_at"], "2026-09-29T16:04:41Z");
    assert_eq!(
        body["totals"],
        json!({ "repos": 2, "commits": 42, "stars": 4 })
    );
    assert_eq!(body["unavailable"], json!(["a/three"]));
    let names: Vec<&str> = body["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["repo"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["a/one", "a/two"]);
    assert_eq!(body["repos"][1]["commits"], 32);
    assert_eq!(body.as_object().unwrap().len(), 4);
}

#[tokio::test]
async fn a_second_request_is_served_from_the_cache() {
    let app = TestApp::new(&["a/b"]);
    app.get("/stats/a/b").await;
    app.clock.advance(FRESH_SECS - 1);
    let reply = app.get("/stats/a/b").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(app.source.calls(), 1);
}

#[tokio::test]
async fn a_stale_value_is_served_and_marked_when_the_source_is_down() {
    let app = TestApp::new(&["a/b"]);
    app.get("/stats/a/b").await;
    app.clock.advance(FRESH_SECS);
    app.source.answer(&repo("a/b"), Outcome::RateLimited);
    let reply = app.get("/stats/a/b").await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = reply.json();
    assert_eq!(body["stale"], true);
    assert_eq!(body["commits"], 10);
    assert_eq!(body["fetched_at"], "2026-09-29T16:04:41Z");
}

#[tokio::test]
async fn a_dead_cache_costs_nothing_but_the_caching() {
    let app = TestApp::over(&["a/b"], DeadCache);
    for _ in 0..2 {
        let reply = app.get("/stats/a/b").await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.json()["commits"], 10);
    }
    assert_eq!(app.source.calls(), 2);
}

#[tokio::test]
async fn stats_responses_may_be_held_at_the_edge() {
    let app = TestApp::new(&["a/b"]);
    for path in [STATS_ROUTE, "/stats/a/b"] {
        assert_eq!(
            app.get(path).await.header("cache-control"),
            Some("public, max-age=300"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn an_error_is_never_held_at_the_edge() {
    let app = TestApp::new(&["a/b"]);
    app.source.answer(&repo("a/b"), Outcome::Broken("down"));
    for path in ["/stats/a/other", "/stats/a/b", STATS_ROUTE] {
        let reply = app.get(path).await;
        assert!(!reply.status.is_success(), "{path}");
        assert_eq!(reply.header("cache-control"), None, "{path}");
    }
}

#[tokio::test]
async fn every_absent_repository_answers_the_same() {
    let app = TestApp::new(&["a/private", "a/deleted"]);
    app.source.answer(&repo("a/private"), Outcome::Private);
    app.source.answer(&repo("a/deleted"), Outcome::Missing);
    let paths = [
        "/stats/a/private",
        "/stats/a/deleted",
        "/stats/a/not-configured",
        "/stats/a/..",
        "/stats/a%2Fb/c",
        "/stats/a/b%20c",
        "/stats/a/b%0d%0aHost:%20evil",
    ];
    for path in paths {
        let reply = app.get(path).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(reply.body, "repository not found", "{path}");
    }
}

#[tokio::test]
async fn a_repository_not_configured_never_reaches_the_source() {
    let app = TestApp::new(&["a/b"]);
    app.get("/stats/scadoshi/dotfiles").await;
    assert_eq!(app.source.calls(), 0);
}

#[tokio::test]
async fn a_failing_source_answers_503_without_its_detail() {
    let app = TestApp::new(&["a/b"]);
    app.source.answer(
        &repo("a/b"),
        Outcome::Broken("error sending request for url (http://10.1.2.3/internal)"),
    );
    for path in ["/stats/a/b", STATS_ROUTE] {
        let reply = app.get(path).await;
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(reply.body, "stats are temporarily unavailable", "{path}");
    }
}

#[tokio::test]
async fn an_unknown_path_is_a_plain_404() {
    let reply = TestApp::new(&["a/b"]).get("/admin").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.body, "not found");
}

#[tokio::test]
async fn only_get_is_routed() {
    let app = TestApp::new(&["a/b"]);
    for method in ["POST", "PUT", "DELETE", "PATCH"] {
        let reply = app.send(method, "/stats/a/b", &[]).await;
        assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
    }
    assert_eq!(app.source.calls(), 0);
}

/// The configured origin is the answer whatever the request says, including
/// when it says nothing: a CDN copy filled by a request without `Origin` still
/// has to work in a browser.
#[tokio::test]
async fn cors_names_the_configured_origin_on_every_response() {
    let app = TestApp::new(&["a/b"]);
    let allowed = app
        .get_with("/stats/a/b", &[("origin", ALLOWED_ORIGIN)])
        .await;
    assert_eq!(
        allowed.header("access-control-allow-origin"),
        Some(ALLOWED_ORIGIN)
    );
    let other = app
        .get_with("/stats/a/b", &[("origin", "https://evil.test")])
        .await;
    assert_eq!(
        other.header("access-control-allow-origin"),
        Some(ALLOWED_ORIGIN),
        "never the caller's origin"
    );
    let none = app.get("/stats/a/b").await;
    assert_eq!(
        none.header("access-control-allow-origin"),
        Some(ALLOWED_ORIGIN),
        "sent without an Origin header too"
    );
}

#[tokio::test]
async fn cors_preflight_offers_get_only() {
    let reply = TestApp::new(&["a/b"])
        .send(
            "OPTIONS",
            "/stats/a/b",
            &[
                ("origin", ALLOWED_ORIGIN),
                ("access-control-request-method", "GET"),
            ],
        )
        .await;
    assert_eq!(reply.header("access-control-allow-methods"), Some("GET"));
}

#[tokio::test]
async fn every_response_carries_the_security_headers_and_a_request_id() {
    let app = TestApp::new(&["a/b"]);
    for path in ["/stats/a/b", "/stats/a/other", "/health", "/admin"] {
        let reply = app.get(path).await;
        assert_eq!(reply.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(reply.header("x-frame-options"), Some("DENY"));
        assert_eq!(
            reply.header("referrer-policy"),
            Some("strict-origin-when-cross-origin")
        );
        assert!(reply.header("x-request-id").is_some(), "{path}");
    }
}

#[tokio::test]
async fn a_burst_past_the_limit_is_answered_429() {
    let app = TestApp::new(&["a/b"]);
    let client = [("cf-connecting-ip", "203.0.113.7")];
    for request in 0..30 {
        let reply = app.get_with("/health", &client).await;
        assert_eq!(reply.status, StatusCode::OK, "request {request}");
    }
    let reply = app.get_with("/health", &client).await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(reply.body, "too many requests, try again in a minute");
    assert!(reply.header("retry-after").is_some());
    assert_eq!(reply.header("cache-control"), None);

    let other = app
        .get_with("/health", &[("cf-connecting-ip", "198.51.100.4")])
        .await;
    assert_eq!(other.status, StatusCode::OK);
}
