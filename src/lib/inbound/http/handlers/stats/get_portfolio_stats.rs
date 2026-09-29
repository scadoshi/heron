use crate::inbound::http::{ApiError, AppState, contracts::stats::HttpPortfolioStats};
use axum::{Json, extract::State};

/// Stats for every configured repository.
pub async fn get_portfolio_stats(
    State(state): State<AppState>,
) -> Result<Json<HttpPortfolioStats>, ApiError> {
    let portfolio = state.stats_service.portfolio_stats().await?;
    Ok(Json(portfolio.into()))
}
