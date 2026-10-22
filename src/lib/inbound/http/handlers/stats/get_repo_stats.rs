use axum::Json;
use axum::extract::{Path, State};

use crate::domain::stats::models::repo_name::RepoName;
use crate::inbound::http::contracts::stats::HttpRepoStats;
use crate::inbound::http::{ApiError, AppState};

/// Stats for one configured repository, with its latest source counts.
pub async fn get_repo_stats(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
) -> Result<Json<HttpRepoStats>, ApiError> {
    let repo = RepoName::new(format!("{owner}/{name}"))?;
    let report = state.stats_service.repo_stats(&repo).await?;
    let counts = state.counts_service.counts(&repo).await;
    Ok(Json(HttpRepoStats::new(report, counts)))
}
