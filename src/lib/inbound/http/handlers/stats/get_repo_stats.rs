use crate::{
    domain::stats::models::repo_name::RepoName,
    inbound::http::{ApiError, AppState, contracts::stats::HttpRepoStats},
};
use axum::{
    Json,
    extract::{Path, State},
};

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
