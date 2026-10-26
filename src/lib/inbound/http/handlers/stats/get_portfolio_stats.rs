use axum::Json;
use axum::extract::State;

use crate::domain::calendar::models::CalendarError;
use crate::inbound::http::contracts::stats::HttpPortfolioStats;
use crate::inbound::http::{ApiError, AppState};

/// Stats for every configured repository, each with its latest source counts,
/// and the account's contribution calendar. The calendar cannot fail the
/// request: without one the field is `null`.
///
/// # Errors
///
/// Only when no repository resolved: the error of whichever failed last, mapped by
/// [`ApiError`]'s `From<StatsError>` to a 404 or a 503.
pub async fn get_portfolio_stats(
    State(state): State<AppState>,
) -> Result<Json<HttpPortfolioStats>, ApiError> {
    let portfolio = state.stats_service.portfolio_stats().await?;
    let mut counts = Vec::with_capacity(portfolio.repos.len());
    for report in &portfolio.repos {
        counts.push(state.counts_service.counts(&report.stats.repo).await);
    }
    let calendar = match state.calendar_service.calendar().await {
        Ok(report) => Some(report),
        Err(CalendarError::NoToken) => None,
        Err(error) => {
            tracing::warn!("calendar unavailable: {error}");
            None
        }
    };
    Ok(Json(HttpPortfolioStats::new(portfolio, counts, calendar)))
}
