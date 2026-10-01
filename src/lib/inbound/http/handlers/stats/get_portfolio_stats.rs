use crate::inbound::http::{ApiError, AppState, contracts::stats::HttpPortfolioStats};
use axum::{Json, extract::State};

/// Stats for every configured repository, each with its latest source counts.
pub async fn get_portfolio_stats(
    State(state): State<AppState>,
) -> Result<Json<HttpPortfolioStats>, ApiError> {
    let portfolio = state.stats_service.portfolio_stats().await?;
    let mut counts = Vec::with_capacity(portfolio.repos.len());
    for report in &portfolio.repos {
        counts.push(state.counts_service.counts(&report.stats.repo).await);
    }
    Ok(Json(HttpPortfolioStats::new(portfolio, counts)))
}
