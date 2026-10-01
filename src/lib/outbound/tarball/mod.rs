//! GitHub's tarball download as a counts source.
//!
//! One request per repository: `GET /repos/{owner}/{name}/tarball` answers the
//! default branch as a gzipped tar, which is unpacked under a scratch directory,
//! measured, and deleted. Nothing stays on disk between measurements.

#[cfg(test)]
mod tests;

use crate::domain::{
    counts::{
        models::{Counts, CountsError},
        ports::CountsSource,
    },
    secret::Secret,
    stats::models::repo_name::RepoName,
};
use anyhow::Context;
use flate2::read::GzDecoder;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

/// GitHub rejects a request that carries no `User-Agent`.
const USER_AGENT: &str = "heron (+https://github.com/scadoshi/heron)";
const API_VERSION: &str = "2022-11-28";
/// A tarball is a few megabytes; the budget covers a slow link.
const REQUEST_TIMEOUT: Duration = Duration::from_mins(1);

/// Makes each unpack directory unique while the process lives.
static UNPACKS: AtomicU64 = AtomicU64::new(0);

/// Downloads a repository's default branch and measures it.
#[derive(Debug, Clone)]
pub struct Tarball {
    client: reqwest::Client,
    base: String,
    token: Option<Secret>,
    dir: PathBuf,
}

impl Tarball {
    /// A source for the API at `base`, unpacking under `dir`, which is created on
    /// first use.
    pub fn new(base: &str, token: Option<Secret>, dir: PathBuf) -> Result<Self, reqwest::Error> {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "x-github-api-version",
            HeaderValue::from_static(API_VERSION),
        );
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(headers)
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_string(),
            token,
            dir,
        })
    }

    async fn download(&self, repo: &RepoName) -> Result<Vec<u8>, CountsError> {
        let request = self
            .client
            .get(format!("{}/repos/{repo}/tarball", self.base));
        let request = match &self.token {
            Some(token) => request.bearer_auth(token.read()),
            None => request,
        };
        let response = request
            .send()
            .await
            .map_err(|error| CountsError::Upstream(error.into()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(CountsError::Upstream(anyhow::anyhow!(
                "github answered {status} for the tarball of {repo}"
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| CountsError::Upstream(error.into()))?;
        Ok(bytes.to_vec())
    }
}

impl CountsSource for Tarball {
    async fn counts(&self, repo: &RepoName) -> Result<Counts, CountsError> {
        let bytes = self.download(repo).await?;
        let work = self.dir.join(format!(
            "{}-{}",
            repo.to_string().replace('/', "-"),
            UNPACKS.fetch_add(1, Ordering::Relaxed)
        ));
        let repo = repo.clone();
        tokio::task::spawn_blocking(move || measure_archive(&bytes, &work, &repo))
            .await
            .map_err(|error| {
                CountsError::Upstream(anyhow::anyhow!("measure task failed: {error}"))
            })?
    }
}

/// Unpacks `bytes` under `work`, measures the checkout inside, and removes `work`
/// whether or not the measurement succeeded.
fn measure_archive(bytes: &[u8], work: &Path, repo: &RepoName) -> Result<Counts, CountsError> {
    let result = unpack_and_measure(bytes, work, repo);
    if let Err(error) = fs::remove_dir_all(work)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("could not remove {}: {error}", work.display());
    }
    result
}

fn unpack_and_measure(bytes: &[u8], work: &Path, repo: &RepoName) -> Result<Counts, CountsError> {
    let upstream = |error: anyhow::Error| CountsError::Upstream(error);
    fs::create_dir_all(work)
        .with_context(|| format!("creating {}", work.display()))
        .map_err(upstream)?;
    tar::Archive::new(GzDecoder::new(bytes))
        .unpack(work)
        .with_context(|| format!("unpacking the tarball of {repo}"))
        .map_err(upstream)?;
    let checkout = single_directory_in(work)
        .with_context(|| format!("the tarball of {repo} did not hold one directory"))
        .map_err(upstream)?;
    measure::measure(&checkout)
        .with_context(|| format!("measuring {repo}"))
        .map_err(upstream)?
        .ok_or_else(|| CountsError::NotMeasurable(repo.clone()))
}

/// GitHub's tarball holds one top-level directory, `owner-name-sha`.
fn single_directory_in(dir: &Path) -> anyhow::Result<PathBuf> {
    let mut directories = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir());
    match (directories.next(), directories.next()) {
        (Some(only), None) => Ok(only),
        (None, _) => anyhow::bail!("no directory"),
        (Some(_), Some(_)) => anyhow::bail!("more than one directory"),
    }
}
