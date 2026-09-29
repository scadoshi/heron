//! The real executable, started the way systemd starts it: environment only, no
//! terminal, stopped with SIGTERM. GitHub is an in-process fake.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use axum::{Json, Router, routing::get};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncReadExt,
    net::TcpListener,
    process::{Child, Command},
};

/// A port nothing is listening on.
async fn free_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap()
}

/// An empty directory to run in, so no stray `.env` is read.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scotland-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Starts the server with exactly `vars` as its environment.
fn start(name: &str, vars: &[(&str, &str)]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_scotland-server"))
        .current_dir(scratch_dir(name))
        .env_clear()
        .envs(vars.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

/// Waits for the process to exit and returns its status with everything it printed.
async fn finish(mut child: Child) -> (ExitStatus, String) {
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("process did not exit")
        .unwrap();
    let mut output = String::new();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    stdout.read_to_string(&mut output).await.unwrap();
    stderr.read_to_string(&mut output).await.unwrap();
    (status, output)
}

/// Polls until `/health` is served. Connecting is not enough: a bound listener
/// accepts into its backlog whether or not anything is serving.
async fn wait_until_serving(address: SocketAddr) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if let Ok(response) = reqwest::get(format!("http://{address}/health")).await
            && response.status().is_success()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("never served /health on {address}");
}

fn sigterm(child: &Child) {
    let status = std::process::Command::new("kill")
        .args(["-TERM", &child.id().unwrap().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

/// A GitHub that knows one public repository with 66 commits. Returns its base URL
/// and a count of the times the repository was asked for.
async fn fake_github() -> (String, Arc<AtomicU32>) {
    let asked = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&asked);
    let router = Router::new()
        .route(
            "/repos/{owner}/{name}",
            get(move || async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "private": false,
                    "stargazers_count": 2,
                    "default_branch": "main",
                    "pushed_at": "2026-09-29T15:58:02Z",
                }))
            }),
        )
        .route(
            "/repos/{owner}/{name}/commits",
            get(|| async {
                (
                    [(
                        "link",
                        r#"<https://x.test/c?per_page=1&page=66>; rel="last""#,
                    )],
                    Json(json!([{ "sha": "abc" }])),
                )
            }),
        )
        .route(
            "/repos/{owner}/{name}/languages",
            get(|| async { Json(json!({ "Rust": 9000 })) }),
        )
        .route(
            "/repos/{owner}/{name}/stats/contributors",
            get(|| async { axum::http::StatusCode::ACCEPTED }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (base, asked)
}

#[tokio::test]
async fn a_missing_required_variable_exits_non_zero_and_names_it() {
    for missing in ["BIND_ADDRESS", "ALLOWED_ORIGINS", "GITHUB_REPOS"] {
        let vars: Vec<(&str, &str)> = [
            ("BIND_ADDRESS", "127.0.0.1:0"),
            ("ALLOWED_ORIGINS", "https://scottyfermo.com"),
            ("GITHUB_REPOS", "a/b"),
        ]
        .into_iter()
        .filter(|(key, _)| *key != missing)
        .collect();
        let (status, output) = finish(start("missing", &vars)).await;
        assert_eq!(status.code(), Some(1), "{missing}: {output}");
        assert!(output.contains(missing), "{missing}: {output}");
    }
}

#[tokio::test]
async fn a_steller_address_off_loopback_exits_non_zero() {
    let (status, output) = finish(start(
        "loopback",
        &[
            ("BIND_ADDRESS", "127.0.0.1:0"),
            ("ALLOWED_ORIGINS", "https://scottyfermo.com"),
            ("GITHUB_REPOS", "a/b"),
            ("CACHE_BACKEND", "layered"),
            ("STELLER_ADDRESS", "10.0.0.5:3000"),
        ],
    ))
    .await;
    assert_eq!(status.code(), Some(1), "{output}");
    assert!(output.contains("not loopback"), "{output}");
}

#[tokio::test]
async fn an_address_already_in_use_exits_non_zero() {
    let taken = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = taken.local_addr().unwrap().to_string();
    let (status, output) = finish(start(
        "in-use",
        &[
            ("BIND_ADDRESS", &address),
            ("ALLOWED_ORIGINS", "https://scottyfermo.com"),
            ("GITHUB_REPOS", "a/b"),
        ],
    ))
    .await;
    assert_eq!(status.code(), Some(1), "{output}");
    assert!(output.contains(&address), "{output}");
}

#[tokio::test]
async fn serves_and_exits_zero_on_sigterm() {
    let address = free_address().await;
    let child = start(
        "sigterm",
        &[
            ("BIND_ADDRESS", &address.to_string()),
            ("ALLOWED_ORIGINS", "https://scottyfermo.com"),
            ("GITHUB_REPOS", "a/b"),
        ],
    );
    wait_until_serving(address).await;
    sigterm(&child);
    let (status, output) = finish(child).await;
    assert_eq!(status.code(), Some(0), "{output}");
    assert!(output.contains("SIGTERM"), "{output}");
}

#[tokio::test]
async fn serves_stats_end_to_end_and_asks_github_once() {
    let address = free_address().await;
    let (github, asked) = fake_github().await;
    let child = start(
        "memory",
        &[
            ("BIND_ADDRESS", &address.to_string()),
            ("ALLOWED_ORIGINS", "https://scottyfermo.com"),
            ("GITHUB_REPOS", "scadoshi/steller"),
            ("GITHUB_API_BASE", &github),
            ("GITHUB_TOKEN", "ghp_a-token-that-must-never-be-printed"),
        ],
    );
    wait_until_serving(address).await;

    for _ in 0..3 {
        let body: Value = reqwest::get(format!("http://{address}/stats/scadoshi/steller"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(body["commits"], 66);
        assert_eq!(body["additions"], Value::Null);
        assert_eq!(body["stale"], false);
    }
    assert_eq!(asked.load(Ordering::SeqCst), 1);

    sigterm(&child);
    let (status, output) = finish(child).await;
    assert_eq!(status.code(), Some(0), "{output}");
    assert!(!output.contains("ghp_"), "{output}");
    assert!(output.contains("github token set"), "{output}");
}

#[tokio::test]
async fn layered_serves_from_memory_while_steller_is_down_and_says_so_once() {
    let address = free_address().await;
    let steller = free_address().await;
    let (github, asked) = fake_github().await;
    let child = start(
        "layered",
        &[
            ("BIND_ADDRESS", &address.to_string()),
            ("ALLOWED_ORIGINS", "https://scottyfermo.com"),
            ("GITHUB_REPOS", "scadoshi/steller"),
            ("GITHUB_API_BASE", &github),
            ("CACHE_BACKEND", "layered"),
            ("STELLER_ADDRESS", &steller.to_string()),
        ],
    );
    wait_until_serving(address).await;

    for _ in 0..5 {
        let response = reqwest::get(format!("http://{address}/stats/scadoshi/steller"))
            .await
            .unwrap();
        assert!(response.status().is_success());
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["commits"], 66);
    }
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    let health: Value = reqwest::get(format!("http://{address}/health/cache"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["backend"], "layered");
    assert_eq!(health["status"], "unreachable");

    sigterm(&child);
    let (status, output) = finish(child).await;
    assert_eq!(status.code(), Some(0), "{output}");
    assert_eq!(
        output.matches("steller cache is down").count(),
        1,
        "{output}"
    );
}
