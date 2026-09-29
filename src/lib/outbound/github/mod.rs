//! GitHub's REST API as a stats source.
//!
//! Four requests per repository: the repository itself, its commits, its languages,
//! and its contributor statistics. Only the fields read are deserialized.

/// Parsing of the `Link` response header.
pub mod link;

use crate::domain::{
    secret::Secret,
    stats::{
        models::{
            errors::StatsError,
            repo_name::RepoName,
            repo_stats::{Language, RepoStats},
        },
        ports::StatsSource,
    },
};
use chrono::{DateTime, Utc};
use reqwest::{
    Response, StatusCode,
    header::{ACCEPT, HeaderMap, HeaderValue, LINK, RETRY_AFTER},
};
use serde::{Deserialize, de::IgnoredAny};
use std::{collections::HashMap, time::Duration};
use thiserror::Error;

/// GitHub rejects a request that carries no `User-Agent`.
const USER_AGENT: &str = "scotland-server (+https://github.com/scadoshi/scotland-server)";
const API_VERSION: &str = "2022-11-28";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Errors reading from GitHub.
#[derive(Debug, Error)]
pub enum GitHubError {
    /// 404: no such repository, or one this token cannot see.
    #[error("repository not found on github")]
    NotFound,
    /// The repository exists and is private.
    #[error("repository is private")]
    Private,
    /// The rate limit is spent.
    #[error("github rate limit exhausted")]
    RateLimited {
        /// When the limit resets, from `x-ratelimit-reset`.
        reset_at: Option<DateTime<Utc>>,
    },
    /// Any other status that is not a success.
    #[error("github returned status {0}")]
    Status(u16),
    /// Network failure, or a body that did not parse.
    #[error("github request failed: {0}")]
    Network(#[from] reqwest::Error),
}

impl GitHubError {
    fn into_stats_error(self, repo: &RepoName) -> StatsError {
        match self {
            Self::NotFound => StatsError::UnknownRepo(repo.clone()),
            Self::Private => StatsError::PrivateRepo(repo.clone()),
            Self::RateLimited { reset_at } => StatsError::RateLimited { reset_at },
            other => StatsError::Upstream(anyhow::Error::new(other)),
        }
    }
}

/// Client for the parts of GitHub's REST API the stats need.
#[derive(Debug, Clone)]
pub struct GitHub {
    client: reqwest::Client,
    base: String,
    token: Option<Secret>,
}

impl GitHub {
    /// A client for the API at `base`. With a token the rate limit is 5,000 requests
    /// an hour; without one it is 60 per IP.
    pub fn new(base: &str, token: Option<Secret>) -> Result<Self, reqwest::Error> {
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
        })
    }

    fn get(&self, repo: &RepoName, suffix: &str) -> reqwest::RequestBuilder {
        let request = self
            .client
            .get(format!("{}/repos/{repo}{suffix}", self.base));
        match &self.token {
            Some(token) => request.bearer_auth(token.read()),
            None => request,
        }
    }

    async fn repository(&self, repo: &RepoName) -> Result<RawRepository, GitHubError> {
        let response = accepted(self.get(repo, "").send().await?)?;
        Ok(response.json().await?)
    }

    /// Commits reachable from `branch`.
    ///
    /// Asked for one commit per page, GitHub names the last page in the `Link`
    /// header, and that page number is the count. A history of one page or none has
    /// no such header, and the count is the length of the body.
    async fn commits(&self, repo: &RepoName, branch: &str) -> Result<u64, GitHubError> {
        let response = self
            .get(repo, "/commits")
            .query(&[("per_page", "1"), ("sha", branch)])
            .send()
            .await?;
        // GitHub answers 409 for a repository with no commits.
        if response.status() == StatusCode::CONFLICT {
            return Ok(0);
        }
        let response = accepted(response)?;
        let last_page = response
            .headers()
            .get(LINK)
            .and_then(|value| value.to_str().ok())
            .and_then(link::last_page);
        if let Some(count) = last_page {
            return Ok(count);
        }
        let commits: Vec<IgnoredAny> = response.json().await?;
        Ok(u64::try_from(commits.len()).unwrap_or(u64::MAX))
    }

    async fn languages(&self, repo: &RepoName) -> Result<Vec<Language>, GitHubError> {
        let response = accepted(self.get(repo, "/languages").send().await?)?;
        let bytes_by_name: HashMap<String, u64> = response.json().await?;
        let mut languages: Vec<Language> = bytes_by_name
            .into_iter()
            .map(|(name, bytes)| Language { name, bytes })
            .collect();
        languages.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
        Ok(languages)
    }

    /// Lines added and deleted across all contributors, or `None` when GitHub has
    /// nothing to report yet.
    ///
    /// GitHub computes these in the background and answers 202 with an empty body
    /// until it is done. Nothing here waits for that: the next refresh asks again.
    async fn churn(&self, repo: &RepoName) -> Result<Option<(u64, u64)>, GitHubError> {
        let response = self.get(repo, "/stats/contributors").send().await?;
        if matches!(
            response.status(),
            StatusCode::ACCEPTED | StatusCode::NO_CONTENT
        ) {
            return Ok(None);
        }
        let response = match accepted(response) {
            Ok(response) => response,
            Err(limited @ GitHubError::RateLimited { .. }) => return Err(limited),
            Err(error) => {
                tracing::warn!("contributor statistics unavailable for {repo}: {error}");
                return Ok(None);
            }
        };
        let Ok(contributors) = response.json::<Vec<RawContributor>>().await else {
            return Ok(None);
        };
        let totals = contributors
            .iter()
            .flat_map(|contributor| &contributor.weeks)
            .fold((0u64, 0u64), |(added, deleted), week| {
                (added.saturating_add(week.a), deleted.saturating_add(week.d))
            });
        Ok(Some(totals))
    }

    async fn read(&self, repo: &RepoName) -> Result<RepoStats, GitHubError> {
        let repository = self.repository(repo).await?;
        // Checked before anything else is requested, so nothing about a private
        // repository is read, let alone served.
        if repository.private {
            return Err(GitHubError::Private);
        }
        let (commits, languages, churn) = tokio::join!(
            self.commits(repo, &repository.default_branch),
            self.languages(repo),
            self.churn(repo),
        );
        let churn = churn?;
        Ok(RepoStats {
            repo: repo.clone(),
            commits: commits?,
            stars: repository.stargazers_count,
            default_branch: repository.default_branch,
            pushed_at: repository.pushed_at,
            languages: languages?,
            additions: churn.map(|(added, _)| added),
            deletions: churn.map(|(_, deleted)| deleted),
        })
    }
}

impl StatsSource for GitHub {
    async fn repo_stats(&self, repo: &RepoName) -> Result<RepoStats, StatsError> {
        self.read(repo)
            .await
            .map_err(|error| error.into_stats_error(repo))
    }
}

/// Passes a successful response through and classifies the rest.
fn accepted(response: Response) -> Result<Response, GitHubError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status == StatusCode::NOT_FOUND {
        return Err(GitHubError::NotFound);
    }
    let headers = response.headers();
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    // The primary limit answers 403 or 429 with no requests remaining. The secondary
    // limit answers the same statuses with `retry-after` and says nothing of what
    // remains.
    let limited = matches!(
        status,
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
    ) && (header("x-ratelimit-remaining") == Some("0")
        || headers.contains_key(RETRY_AFTER));
    if limited {
        let reset_at = header("x-ratelimit-reset")
            .and_then(|seconds| seconds.parse::<i64>().ok())
            .and_then(|seconds| DateTime::from_timestamp(seconds, 0));
        return Err(GitHubError::RateLimited { reset_at });
    }
    Err(GitHubError::Status(status.as_u16()))
}

// == raw response shapes (only what is read) ==

#[derive(Debug, Deserialize)]
struct RawRepository {
    private: bool,
    stargazers_count: u32,
    default_branch: String,
    pushed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct RawContributor {
    #[serde(default)]
    weeks: Vec<RawWeek>,
}

/// One week of one contributor: `a` lines added, `d` lines deleted.
#[derive(Debug, Deserialize)]
struct RawWeek {
    #[serde(default)]
    a: u64,
    #[serde(default)]
    d: u64,
}

#[cfg(test)]
mod tests;
