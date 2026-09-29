//! Stats models.

/// [`CacheKey`](cache_key::CacheKey), the versioned key a snapshot is stored under.
pub mod cache_key;

/// Error types for the source, the cache, and the service.
pub mod errors;

/// [`PortfolioStats`](portfolio_stats::PortfolioStats), every repository in one answer.
pub mod portfolio_stats;

/// [`RepoName`](repo_name::RepoName), a validated `owner/name`.
pub mod repo_name;

/// [`RepoStats`](repo_stats::RepoStats) and what the service returns around it.
pub mod repo_stats;

/// [`Snapshot`](snapshot::Snapshot), the cached payload.
pub mod snapshot;
