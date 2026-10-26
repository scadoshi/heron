//! GitHub's REST API as a stats source, and its GraphQL API as the calendar source.
//!
//! Five requests per repository: the repository itself, its commits, its languages,
//! its contributor statistics and its weekly commit activity. One GraphQL query
//! for the contribution calendar.
//! Only the fields read are deserialized.

pub mod link;

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, LINK, RETRY_AFTER};
use reqwest::{Response, StatusCode};
use serde::Deserialize;
use serde::de::IgnoredAny;
use thiserror::Error;

use crate::domain::calendar::models::{Calendar, CalendarError, Day};
use crate::domain::calendar::ports::CalendarSource;
use crate::domain::secret::Secret;
use crate::domain::stats::models::errors::StatsError;
use crate::domain::stats::models::repo_name::RepoName;
use crate::domain::stats::models::repo_stats::{Language, RepoStats, WeekCommits};
use crate::domain::stats::ports::StatsSource;

/// GitHub rejects a request that carries no `User-Agent`.
const USER_AGENT: &str = "heron (+https://github.com/scadoshi/heron)";
/// The REST API version sent as `X-GitHub-Api-Version`, the dated version GitHub
/// documents at <https://docs.github.com/rest/about-the-rest-api/api-versions>.
const API_VERSION: &str = "2022-11-28";
/// GitHub ends any API request it takes longer than ten seconds to process, so a
/// request still open past that is not coming back.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The wait before the second ask for statistics GitHub is still computing.
const COMPUTING_PAUSE: Duration = Duration::from_secs(3);

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
    /// GraphQL answers nothing without a token.
    #[error("no github token")]
    NoToken,
    /// GraphQL answered 200 with an error in the body.
    #[error("github graphql: {0}")]
    GraphQl(String),
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

/// Client for the parts of GitHub's REST API the stats need, and the GraphQL query
/// behind the contribution calendar.
#[derive(Debug, Clone)]
pub struct GitHub {
    client: reqwest::Client,
    base: String,
    token: Option<Secret>,
    /// How long to wait before asking once more for statistics GitHub is
    /// still computing.
    computing_pause: Duration,
}

impl GitHub {
    /// A client for the API at `base`. With a token the rate limit is 5,000 requests
    /// an hour; without one it is 60 per IP.
    ///
    /// # Errors
    ///
    /// When the HTTP client cannot be built: its TLS backend fails to initialize.
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
            computing_pause: COMPUTING_PAUSE,
        })
    }

    /// The same client with a different wait before the second ask for
    /// statistics, so a test does not sit through the real one.
    #[cfg(test)]
    fn with_computing_pause(mut self, pause: Duration) -> Self {
        self.computing_pause = pause;
        self
    }

    /// One of GitHub's `/stats` answers, or `None` while GitHub is still
    /// computing it.
    ///
    /// GitHub computes these in the background and answers 202 (or 204) with
    /// an empty body until it is done, which takes a few seconds, and lets the
    /// result go cold again between refreshes for a repository nobody pushes
    /// to. The first ask starts the job, so one more after `computing_pause`
    /// catches most of them; a second 202 waits for the next refresh.
    async fn fetch_statistics(
        &self,
        repo: &RepoName,
        suffix: &str,
    ) -> Result<Option<Response>, GitHubError> {
        let response = self.get(repo, suffix).send().await?;
        if !is_computing(&response) {
            return Ok(Some(response));
        }
        tokio::time::sleep(self.computing_pause).await;
        let response = self.get(repo, suffix).send().await?;
        Ok((!is_computing(&response)).then_some(response))
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

    async fn fetch_repository(&self, repo: &RepoName) -> Result<RawRepository, GitHubError> {
        let response = check_status(self.get(repo, "").send().await?)?;
        Ok(response.json().await?)
    }

    /// Commits reachable from `branch`.
    ///
    /// Asked for one commit per page, GitHub names the last page in the `Link`
    /// header, and that page number is the count. A history of one page or none has
    /// no such header, and the count is the length of the body.
    async fn fetch_commits(&self, repo: &RepoName, branch: &str) -> Result<u64, GitHubError> {
        let response = self
            .get(repo, "/commits")
            .query(&[("per_page", "1"), ("sha", branch)])
            .send()
            .await?;
        // GitHub answers 409 for a repository with no commits.
        if response.status() == StatusCode::CONFLICT {
            return Ok(0);
        }
        let response = check_status(response)?;
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

    async fn fetch_languages(&self, repo: &RepoName) -> Result<Vec<Language>, GitHubError> {
        let response = check_status(self.get(repo, "/languages").send().await?)?;
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
    async fn fetch_churn(&self, repo: &RepoName) -> Result<Option<(u64, u64)>, GitHubError> {
        let Some(response) = self.fetch_statistics(repo, "/stats/contributors").await? else {
            return Ok(None);
        };
        let response = match check_status(response) {
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

    /// Commits per week over the last year, oldest first, or `None` while GitHub
    /// is still computing them.
    async fn fetch_activity(
        &self,
        repo: &RepoName,
    ) -> Result<Option<Vec<WeekCommits>>, GitHubError> {
        let Some(response) = self
            .fetch_statistics(repo, "/stats/commit_activity")
            .await?
        else {
            return Ok(None);
        };
        let response = match check_status(response) {
            Ok(response) => response,
            Err(limited @ GitHubError::RateLimited { .. }) => return Err(limited),
            Err(error) => {
                tracing::warn!("commit activity unavailable for {repo}: {error}");
                return Ok(None);
            }
        };
        let Ok(weeks) = response.json::<Vec<RawWeekActivity>>().await else {
            return Ok(None);
        };
        let mut weeks: Vec<WeekCommits> = weeks
            .into_iter()
            .filter_map(|week| {
                Some(WeekCommits {
                    week: DateTime::from_timestamp(week.week, 0)?,
                    commits: week.total,
                })
            })
            .collect();
        weeks.sort_by_key(|week| week.week);
        Ok(Some(weeks))
    }

    async fn fetch_stats(&self, repo: &RepoName) -> Result<RepoStats, GitHubError> {
        let repository = self.fetch_repository(repo).await?;
        // Checked before anything else is requested, so nothing about a private
        // repository is read, let alone served.
        if repository.private {
            return Err(GitHubError::Private);
        }
        let (commits, languages, churn, activity) = tokio::join!(
            self.fetch_commits(repo, &repository.default_branch),
            self.fetch_languages(repo),
            self.fetch_churn(repo),
            self.fetch_activity(repo),
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
            weekly_commits: activity?,
        })
    }
}

impl StatsSource for GitHub {
    async fn fetch_repo_stats(&self, repo: &RepoName) -> Result<RepoStats, StatsError> {
        self.fetch_stats(repo)
            .await
            .map_err(|error| error.into_stats_error(repo))
    }
}

/// The calendar the profile page draws: a year of days with a count and a shade.
const CALENDAR_QUERY: &str = "query($login: String!) { user(login: $login) { contributionsCollection { contributionCalendar { totalContributions weeks { contributionDays { date contributionCount contributionLevel } } } } } }";

impl GitHub {
    async fn fetch_calendar(&self, login: &str) -> Result<Calendar, GitHubError> {
        let Some(token) = &self.token else {
            return Err(GitHubError::NoToken);
        };
        let body = serde_json::json!({
            "query": CALENDAR_QUERY,
            "variables": { "login": login },
        });
        let response = self
            .client
            .post(format!("{}/graphql", self.base))
            .bearer_auth(token.read())
            .json(&body)
            .send()
            .await?;
        let response = check_status(response)?;
        let reply: RawGraphQl = response.json().await?;
        if let Some(error) = reply.errors.first() {
            return Err(GitHubError::GraphQl(error.message.clone()));
        }
        let calendar = reply
            .data
            .and_then(|data| data.user)
            .map(|user| user.contributions_collection.contribution_calendar)
            .ok_or_else(|| GitHubError::GraphQl(format!("no calendar for {login}")))?;
        let days = calendar
            .weeks
            .into_iter()
            .flat_map(|week| week.contribution_days)
            .map(|day| Day {
                date: day.date,
                count: day.contribution_count,
                level: match day.contribution_level.as_str() {
                    "FIRST_QUARTILE" => 1,
                    "SECOND_QUARTILE" => 2,
                    "THIRD_QUARTILE" => 3,
                    "FOURTH_QUARTILE" => 4,
                    _ => 0,
                },
            })
            .collect();
        Ok(Calendar {
            login: login.to_string(),
            total: calendar.total_contributions,
            days,
        })
    }
}

impl CalendarSource for GitHub {
    async fn fetch_calendar(&self, login: &str) -> Result<Calendar, CalendarError> {
        self.fetch_calendar(login)
            .await
            .map_err(|error| match error {
                GitHubError::NoToken => CalendarError::NoToken,
                GitHubError::RateLimited { reset_at } => CalendarError::RateLimited { reset_at },
                other => CalendarError::Upstream(anyhow::Error::new(other)),
            })
    }
}

/// GitHub's answer while it is still computing a `/stats` result.
fn is_computing(response: &Response) -> bool {
    matches!(
        response.status(),
        StatusCode::ACCEPTED | StatusCode::NO_CONTENT
    )
}

/// Passes a successful response through and classifies the rest.
fn check_status(response: Response) -> Result<Response, GitHubError> {
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

#[derive(Debug, Deserialize)]
struct RawGraphQl {
    data: Option<RawGraphQlData>,
    #[serde(default)]
    errors: Vec<RawGraphQlError>,
}

#[derive(Debug, Deserialize)]
struct RawGraphQlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct RawGraphQlData {
    user: Option<RawUser>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawUser {
    contributions_collection: RawContributions,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawContributions {
    contribution_calendar: RawCalendar,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCalendar {
    total_contributions: u32,
    weeks: Vec<RawCalendarWeek>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCalendarWeek {
    contribution_days: Vec<RawDay>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDay {
    date: NaiveDate,
    contribution_count: u32,
    contribution_level: String,
}

/// One week of `/stats/commit_activity`: the week's start as a unix timestamp
/// and its commit total. The per-day breakdown is not read.
#[derive(Debug, Deserialize)]
struct RawWeekActivity {
    week: i64,
    total: u32,
}
