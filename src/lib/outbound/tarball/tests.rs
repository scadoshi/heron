//! The adapter against an in-process fake of GitHub's tarball endpoint.

use super::*;
use crate::test_support::repo;
use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode as AxumStatus, header},
    response::{IntoResponse, Response as AxumResponse},
    routing::get,
};
use flate2::{Compression, write::GzEncoder};
use measure::Language;
use std::sync::{Arc, Mutex};

const TOKEN: &str = "ghp_a-token-that-must-never-be-printed";

/// A gzipped tar of one top-level directory holding `files`.
fn tarball(top: &str, files: &[(&str, &str)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (path, text) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(u64::try_from(text.len()).unwrap());
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("{top}/{path}"), text.as_bytes())
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

#[derive(Clone)]
struct Fake {
    status: u16,
    body: Vec<u8>,
    seen: Arc<Mutex<Vec<Request<()>>>>,
}

impl Fake {
    fn serving(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            body,
            seen: Arc::default(),
        }
    }

    /// Serves the fake and returns a source pointed at it, unpacking under `dir`.
    async fn serve(&self, token: Option<&str>, dir: &Path) -> Tarball {
        let router = Router::new()
            .route("/repos/{owner}/{name}/tarball", get(tarball_route))
            .with_state(self.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Tarball::new(
            &base,
            token.map(|t| Secret::new(t).unwrap()),
            dir.to_path_buf(),
        )
        .unwrap()
    }
}

async fn tarball_route(State(fake): State<Fake>, request: Request) -> AxumResponse {
    let (parts, _) = request.into_parts();
    fake.seen
        .lock()
        .unwrap()
        .push(Request::from_parts(parts, ()));
    (
        AxumStatus::from_u16(fake.status).unwrap(),
        [(header::CONTENT_TYPE, "application/x-gzip")],
        fake.body,
    )
        .into_response()
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("heron_tarball_{}_{name}", std::process::id()))
}

#[tokio::test]
async fn a_rust_tarball_is_unpacked_measured_and_removed() {
    let fake = Fake::serving(tarball(
        "scadoshi-steller-abc123",
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"steller\"\n\n[lints.clippy]\ntodo = \"deny\"\nunwrap_used = \"deny\"\n",
            ),
            (
                "src/lib.rs",
                "pub fn a() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\n",
            ),
            ("README.md", "not source\n"),
        ],
    ));
    let dir = scratch("rust");
    let source = fake.serve(Some(TOKEN), &dir).await;

    let counts = source.counts(&repo("scadoshi/steller")).await.unwrap();

    assert_eq!(
        counts,
        Counts {
            language: Language::Rust,
            lines: 6,
            tests: 1,
            clippy_lints: Some(2),
        }
    );
    let leftovers = fs::read_dir(&dir).unwrap().count();
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(leftovers, 0, "the unpack directory is removed");

    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen[0].uri().path(), "/repos/scadoshi/steller/tarball");
    assert_eq!(
        seen[0].headers()[header::AUTHORIZATION],
        format!("Bearer {TOKEN}")
    );
}

#[tokio::test]
async fn a_tarball_in_neither_language_is_not_measurable() {
    let fake = Fake::serving(tarball("x-y-1", &[("index.html", "<p>hi</p>\n")]));
    let dir = scratch("other");
    let source = fake.serve(None, &dir).await;
    let error = source.counts(&repo("x/y")).await.unwrap_err();
    let _ = fs::remove_dir_all(&dir);
    assert!(matches!(error, CountsError::NotMeasurable(_)), "{error}");
}

#[tokio::test]
async fn a_refusal_is_an_upstream_error_that_names_the_status() {
    let mut fake = Fake::serving(Vec::new());
    fake.status = 404;
    let dir = scratch("refused");
    let source = fake.serve(None, &dir).await;
    let error = source.counts(&repo("x/y")).await.unwrap_err();
    let _ = fs::remove_dir_all(&dir);
    assert!(error.to_string().contains("404"), "{error}");
}

#[tokio::test]
async fn bytes_that_are_not_a_tarball_are_an_upstream_error_and_leave_nothing_behind() {
    let fake = Fake::serving(b"this is not gzip".to_vec());
    let dir = scratch("garbage");
    let source = fake.serve(None, &dir).await;
    let error = source.counts(&repo("x/y")).await.unwrap_err();
    let leftovers = fs::read_dir(&dir).map_or(0, Iterator::count);
    let _ = fs::remove_dir_all(&dir);
    assert!(matches!(error, CountsError::Upstream(_)), "{error}");
    assert_eq!(leftovers, 0);
}

#[tokio::test]
#[ignore = "downloads scadoshi/gotcha from api.github.com; run with --ignored"]
async fn a_real_repository_measures_from_github() {
    let dir = scratch("live");
    let token = std::env::var("GITHUB_TOKEN")
        .ok()
        .and_then(|t| Secret::new(&t).ok());
    let source = Tarball::new("https://api.github.com", token, dir.clone()).unwrap();
    let counts = source.counts(&repo("scadoshi/gotcha")).await.unwrap();
    let leftovers = fs::read_dir(&dir).map_or(0, Iterator::count);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(counts.language, Language::Rust);
    assert!(counts.lines > 300 && counts.tests >= 11, "{counts:?}");
    assert_eq!(leftovers, 0);
}
