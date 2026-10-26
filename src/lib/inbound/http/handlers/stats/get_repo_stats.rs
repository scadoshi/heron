//! `GET /stats/{owner}/{name}`.

use axum::Json;
use axum::extract::{Path, State};

use crate::domain::stats::models::repo_name::RepoName;
use crate::inbound::http::contracts::stats::HttpRepoStats;
use crate::inbound::http::{ApiError, AppState};

/// Stats for one configured repository, with its latest source counts.
///
/// # Errors
///
/// [`ApiError::NotFound`] for a name that does not parse, a repository not in
/// the allowlist, or one GitHub reports missing or private.
/// [`ApiError::ServiceUnavailable`] when GitHub cannot be read and nothing is
/// cached to cover for it.
pub async fn get_repo_stats(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
) -> Result<Json<HttpRepoStats>, ApiError> {
    let repo = RepoName::new(format!("{owner}/{name}"))?;
    let report = state.stats_service.repo_stats(&repo).await?;
    let counts = state.counts_service.counts(&repo).await;
    Ok(Json(HttpRepoStats::new(report, counts)))
}
