//! The adapter against an in-process fake of GitHub's API.

use super::*;
use crate::test_support::repo;
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderMap as AxumHeaders, StatusCode as AxumStatus, header},
    response::{IntoResponse, Response as AxumResponse},
    routing::{get, post},
};
use serde_json::json;
use std::sync::{Arc, Mutex};

const TOKEN: &str = "ghp_a-token-that-must-never-be-printed";

/// What the fake answers, and what it was asked.
#[derive(Clone)]
struct Fake {
    private: bool,
    /// Status and `Link` header for the commits request.
    commits: (u16, Option<&'static str>),
    commits_body: &'static str,
    contributors: (u16, &'static str),
    /// Status and body for the commit activity request.
    activity: (u16, &'static str),
    /// What the second and later asks for commit activity answer, when it
    /// differs from the first.
    activity_then: Option<(u16, &'static str)>,
    /// Status and body for the GraphQL calendar query.
    graphql: (u16, &'static str),
    /// Answered by every route when set, as status and headers.
    refusal: Option<(u16, Vec<(&'static str, &'static str)>)>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

struct Seen {
    path_and_query: String,
    headers: AxumHeaders,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            private: false,
            commits: (
                200,
                Some(
                    r#"<https://api.github.com/repositories/1/commits?per_page=1&page=2>; rel="next", <https://api.github.com/repositories/1/commits?per_page=1&page=66>; rel="last""#,
                ),
            ),
            commits_body: r#"[{"sha":"abc"}]"#,
            contributors: (
                200,
                r#"[{"total":2,"weeks":[{"w":1,"a":100,"d":10,"c":1},{"w":2,"a":5,"d":1,"c":1}]},
                    {"total":1,"weeks":[{"w":1,"a":7,"d":2,"c":1}]}]"#,
            ),
            activity: (
                200,
                r#"[{"week":1758412800,"total":3,"days":[0,1,0,2,0,0,0]},{"week":1757808000,"total":5,"days":[1,1,1,1,1,0,0]}]"#,
            ),
            graphql: (
                200,
                r#"{"data":{"user":{"contributionsCollection":{"contributionCalendar":{"totalContributions":61,"weeks":[
                    {"contributionDays":[{"date":"2026-09-28","contributionCount":0,"contributionLevel":"NONE"},{"date":"2026-09-29","contributionCount":13,"contributionLevel":"SECOND_QUARTILE"}]},
                    {"contributionDays":[{"date":"2026-09-30","contributionCount":48,"contributionLevel":"FOURTH_QUARTILE"}]}]}}}}}"#,
            ),
            activity_then: None,
            refusal: None,
            seen: Arc::default(),
        }
    }
}

impl Fake {
    fn record(&self, request: &Request) {
        self.seen.lock().unwrap().push(Seen {
            path_and_query: request
                .uri()
                .path_and_query()
                .map(ToString::to_string)
                .unwrap_or_default(),
            headers: request.headers().clone(),
        });
    }

    fn refuse(&self) -> Option<AxumResponse> {
        let (status, headers) = self.refusal.clone()?;
        let mut response = AxumStatus::from_u16(status).unwrap().into_response();
        for (name, value) in headers {
            response.headers_mut().insert(name, value.parse().unwrap());
        }
        Some(response)
    }

    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    fn saw(&self, fragment: &str) -> bool {
        self.count(fragment) > 0
    }

    fn count(&self, fragment: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| seen.path_and_query.contains(fragment))
            .count()
    }

    /// Serves the fake and returns a client pointed at it.
    async fn serve(&self, token: Option<&str>) -> GitHub {
        let router = Router::new()
            .route("/repos/{owner}/{name}", get(repository))
            .route("/repos/{owner}/{name}/commits", get(commits))
            .route("/repos/{owner}/{name}/languages", get(languages))
            .route(
                "/repos/{owner}/{name}/stats/contributors",
                get(contributors),
            )
            .route("/repos/{owner}/{name}/stats/commit_activity", get(activity))
            .route("/graphql", post(graphql))
            .with_state(self.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        GitHub::new(&base, token.map(|t| Secret::new(t).unwrap()))
            .unwrap()
            .with_computing_pause(Duration::ZERO)
    }
}

async fn repository(State(fake): State<Fake>, request: Request) -> AxumResponse {
    fake.record(&request);
    if let Some(refusal) = fake.refuse() {
        return refusal;
    }
    Json(json!({
        "private": fake.private,
        "stargazers_count": 3,
        "default_branch": "release/1.0 rc",
        "pushed_at": "2026-09-29T15:58:02Z",
        "description": "fields the adapter does not read are ignored",
    }))
    .into_response()
}

async fn commits(State(fake): State<Fake>, request: Request) -> AxumResponse {
    fake.record(&request);
    if let Some(refusal) = fake.refuse() {
        return refusal;
    }
    let (status, link) = fake.commits;
    let mut response = (
        AxumStatus::from_u16(status).unwrap(),
        [(header::CONTENT_TYPE, "application/json")],
        fake.commits_body,
    )
        .into_response();
    if let Some(link) = link {
        response
            .headers_mut()
            .insert(header::LINK, link.parse().unwrap());
    }
    response
}

async fn languages(State(fake): State<Fake>, request: Request) -> AxumResponse {
    fake.record(&request);
    if let Some(refusal) = fake.refuse() {
        return refusal;
    }
    Json(json!({ "Shell": 40, "Rust": 9000, "C": 40, "Python": 700 })).into_response()
}

async fn contributors(State(fake): State<Fake>, request: Request) -> AxumResponse {
    fake.record(&request);
    if let Some(refusal) = fake.refuse() {
        return refusal;
    }
    let (status, body) = fake.contributors;
    (
        AxumStatus::from_u16(status).unwrap(),
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

#[tokio::test]
async fn reads_every_field() {
    let fake = Fake::default();
    let stats = fake
        .serve(None)
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap();
    assert_eq!(stats.repo, repo("a/b"));
    assert_eq!(stats.commits, 66);
    assert_eq!(stats.stars, 3);
    assert_eq!(stats.default_branch, "release/1.0 rc");
    assert_eq!(
        stats.pushed_at.unwrap().to_rfc3339(),
        "2026-09-29T15:58:02+00:00"
    );
    assert_eq!(stats.additions, Some(112));
    assert_eq!(stats.deletions, Some(13));
    assert_eq!(fake.requests(), 5);
}

#[tokio::test]
async fn languages_are_sorted_by_bytes_then_name() {
    let stats = Fake::default()
        .serve(None)
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap();
    let names: Vec<&str> = stats.languages.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Rust", "Python", "C", "Shell"]);
}

#[tokio::test]
async fn the_branch_is_sent_encoded_with_one_commit_per_page() {
    let fake = Fake::default();
    fake.serve(None)
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap();
    assert!(fake.saw("/repos/a/b/commits?per_page=1&sha=release%2F1.0+rc"));
}

#[tokio::test]
async fn without_a_link_header_the_count_is_the_length_of_the_body() {
    let one = Fake {
        commits: (200, None),
        ..Fake::default()
    };
    let none = Fake {
        commits: (200, None),
        commits_body: "[]",
        ..Fake::default()
    };
    let stats = one
        .serve(None)
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap();
    assert_eq!(stats.commits, 1);
    let stats = none
        .serve(None)
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap();
    assert_eq!(stats.commits, 0);
}

#[tokio::test]
async fn a_repository_with_no_commits_counts_zero() {
    let fake = Fake {
        commits: (409, None),
        commits_body: r#"{"message":"Git Repository is empty."}"#,
        ..Fake::default()
    };
    let stats = fake
        .serve(None)
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap();
    assert_eq!(stats.commits, 0);
}

#[tokio::test]
async fn statistics_still_being_computed_are_none() {
    for status in [202, 204] {
        let fake = Fake {
            contributors: (status, ""),
            ..Fake::default()
        };
        let stats = fake
            .serve(None)
            .await
            .repo_stats(&repo("a/b"))
            .await
            .unwrap();
        assert_eq!(stats.additions, None, "{status}");
        assert_eq!(stats.deletions, None, "{status}");
        assert_eq!(stats.commits, 66);
    }
}

#[tokio::test]
async fn statistics_that_fail_or_do_not_parse_are_none() {
    for contributors in [(500, ""), (200, "{}"), (422, r#"{"message":"too large"}"#)] {
        let fake = Fake {
            contributors,
            ..Fake::default()
        };
        let stats = fake
            .serve(None)
            .await
            .repo_stats(&repo("a/b"))
            .await
            .unwrap();
        assert_eq!(stats.additions, None, "{contributors:?}");
        assert_eq!(stats.commits, 66);
    }
}

#[tokio::test]
async fn a_private_repository_is_refused_before_anything_else_is_read() {
    let fake = Fake {
        private: true,
        ..Fake::default()
    };
    let error = fake
        .serve(Some(TOKEN))
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap_err();
    assert!(matches!(error, StatsError::PrivateRepo(_)), "{error:?}");
    assert_eq!(fake.requests(), 1);
}

#[tokio::test]
async fn not_found_is_an_unknown_repository() {
    let fake = Fake {
        refusal: Some((404, vec![])),
        ..Fake::default()
    };
    let error = fake
        .serve(None)
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap_err();
    assert!(matches!(error, StatsError::UnknownRepo(_)), "{error:?}");
}

#[tokio::test]
async fn a_spent_rate_limit_carries_its_reset_time() {
    for status in [403, 429] {
        let fake = Fake {
            refusal: Some((
                status,
                vec![
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", "1790000000"),
                ],
            )),
            ..Fake::default()
        };
        let error = fake
            .serve(None)
            .await
            .repo_stats(&repo("a/b"))
            .await
            .unwrap_err();
        let StatsError::RateLimited { reset_at } = error else {
            panic!("expected RateLimited, got {error:?}");
        };
        assert_eq!(reset_at.unwrap().timestamp(), 1_790_000_000);
    }
}

#[tokio::test]
async fn the_secondary_rate_limit_is_recognized_by_retry_after() {
    let fake = Fake {
        refusal: Some((403, vec![("retry-after", "60")])),
        ..Fake::default()
    };
    let error = fake
        .serve(None)
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, StatsError::RateLimited { reset_at: None }),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_forbidden_that_is_not_a_rate_limit_is_upstream() {
    let fake = Fake {
        refusal: Some((403, vec![("x-ratelimit-remaining", "41")])),
        ..Fake::default()
    };
    let error = fake
        .serve(None)
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap_err();
    assert!(matches!(error, StatsError::Upstream(_)), "{error:?}");
    assert!(error.to_string().contains("403"), "{error}");
}

#[tokio::test]
async fn a_rate_limit_on_the_statistics_fails_the_read() {
    let fake = Fake {
        refusal: Some((403, vec![("x-ratelimit-remaining", "0")])),
        ..Fake::default()
    };
    let error = fake
        .serve(None)
        .await
        .churn(&repo("a/b"))
        .await
        .unwrap_err();
    assert!(matches!(error, GitHubError::RateLimited { .. }));
}

#[tokio::test]
async fn every_request_identifies_itself() {
    let fake = Fake::default();
    fake.serve(None)
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap();
    for seen in fake.seen.lock().unwrap().iter() {
        assert_eq!(seen.headers[header::USER_AGENT], USER_AGENT);
        assert_eq!(seen.headers[header::ACCEPT], "application/vnd.github+json");
        assert_eq!(seen.headers["x-github-api-version"], API_VERSION);
    }
}

#[tokio::test]
async fn the_token_is_sent_only_when_configured() {
    let with = Fake::default();
    with.serve(Some(TOKEN))
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap();
    for seen in with.seen.lock().unwrap().iter() {
        assert_eq!(
            seen.headers[header::AUTHORIZATION],
            format!("Bearer {TOKEN}")
        );
    }

    let without = Fake::default();
    without
        .serve(None)
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap();
    for seen in without.seen.lock().unwrap().iter() {
        assert!(!seen.headers.contains_key(header::AUTHORIZATION));
    }
}

#[tokio::test]
async fn the_token_never_appears_in_an_error_or_in_debug() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let github = GitHub::new(&base, Some(Secret::new(TOKEN).unwrap())).unwrap();
    let error = github.repo_stats(&repo("a/b")).await.unwrap_err();
    assert!(matches!(error, StatsError::Upstream(_)));
    assert!(!format!("{error}").contains(TOKEN));
    assert!(!format!("{error:?}").contains(TOKEN));
    assert!(!format!("{github:?}").contains(TOKEN));

    let refused = Fake {
        refusal: Some((500, vec![])),
        ..Fake::default()
    };
    let error = refused
        .serve(Some(TOKEN))
        .await
        .repo_stats(&repo("a/b"))
        .await
        .unwrap_err();
    assert!(!format!("{error} {error:?}").contains(TOKEN));
}

#[tokio::test]
async fn a_trailing_slash_on_the_base_is_dropped() {
    let fake = Fake::default();
    let github = fake.serve(None).await;
    let github = GitHub::new(&format!("{}/", github.base), None).unwrap();
    github.repo_stats(&repo("a/b")).await.unwrap();
    assert!(fake.saw("/repos/a/b/languages"));
    assert!(!fake.saw("//repos"));
}

async fn graphql(State(fake): State<Fake>, request: Request) -> AxumResponse {
    fake.record(&request);
    if let Some(refusal) = fake.refuse() {
        return refusal;
    }
    let (status, body) = fake.graphql;
    (
        AxumStatus::from_u16(status).unwrap(),
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

async fn activity(State(fake): State<Fake>, request: Request) -> AxumResponse {
    fake.record(&request);
    if let Some(refusal) = fake.refuse() {
        return refusal;
    }
    let (status, body) = match fake.activity_then {
        Some(then) if fake.count("/stats/commit_activity") > 1 => then,
        _ => fake.activity,
    };
    (
        AxumStatus::from_u16(status).unwrap(),
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

// == commit activity ==

#[tokio::test]
async fn weekly_commits_come_back_oldest_first() {
    let fake = Fake::default();
    let github = fake.serve(None).await;
    let stats = github.repo_stats(&repo("a/b")).await.unwrap();
    let weeks = stats.weekly_commits.unwrap();
    assert_eq!(weeks.len(), 2);
    assert!(weeks[0].week < weeks[1].week);
    assert_eq!((weeks[0].commits, weeks[1].commits), (5, 3));
    assert!(fake.saw("/stats/commit_activity"));
}

#[tokio::test]
async fn weekly_commits_are_none_while_github_is_still_computing_them() {
    let fake = Fake {
        activity: (202, ""),
        ..Fake::default()
    };
    let github = fake.serve(None).await;
    let stats = github.repo_stats(&repo("a/b")).await.unwrap();
    assert_eq!(stats.weekly_commits, None);
    assert_eq!(
        fake.count("/stats/commit_activity"),
        2,
        "asked once more, then left for the next refresh"
    );
}

#[tokio::test]
async fn statistics_github_finishes_during_the_pause_come_back_on_the_second_ask() {
    let fake = Fake {
        activity: (202, ""),
        activity_then: Some((
            200,
            r#"[{"week":1758412800,"total":3,"days":[0,1,0,2,0,0,0]}]"#,
        )),
        ..Fake::default()
    };
    let github = fake.serve(None).await;
    let stats = github.repo_stats(&repo("a/b")).await.unwrap();
    let weeks = stats.weekly_commits.unwrap();
    assert_eq!(weeks.len(), 1);
    assert_eq!(fake.count("/stats/commit_activity"), 2);
}

// == calendar ==

#[tokio::test]
async fn the_calendar_is_one_graphql_query_with_the_token_and_the_login() {
    let fake = Fake::default();
    let github = fake.serve(Some(TOKEN)).await;
    let calendar = CalendarSource::calendar(&github, "scadoshi").await.unwrap();
    assert_eq!(calendar.login, "scadoshi");
    assert_eq!(calendar.total, 61);
    assert_eq!(calendar.days.len(), 3);
    assert_eq!(calendar.days[0].level, 0);
    assert_eq!(calendar.days[1].count, 13);
    assert_eq!(calendar.days[1].level, 2);
    assert_eq!(calendar.days[2].level, 4);
    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path_and_query, "/graphql");
    assert_eq!(
        seen[0].headers[header::AUTHORIZATION],
        format!("Bearer {TOKEN}")
    );
}

#[tokio::test]
async fn without_a_token_the_calendar_is_not_asked_for() {
    let fake = Fake::default();
    let github = fake.serve(None).await;
    let error = CalendarSource::calendar(&github, "scadoshi")
        .await
        .unwrap_err();
    assert!(matches!(error, CalendarError::NoToken), "{error}");
    assert_eq!(fake.requests(), 0);
}

#[tokio::test]
async fn a_graphql_error_in_the_body_is_upstream() {
    let fake = Fake {
        graphql: (
            200,
            r#"{"data":null,"errors":[{"message":"Could not resolve to a User"}]}"#,
        ),
        ..Fake::default()
    };
    let github = fake.serve(Some(TOKEN)).await;
    let error = CalendarSource::calendar(&github, "nobody")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Could not resolve"), "{error}");
}
