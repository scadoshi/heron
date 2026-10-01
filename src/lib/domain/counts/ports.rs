//! Port traits for counts: where a measurement comes from, and the service the HTTP
//! layer and the sweeper call.

use crate::domain::{
    BoxFuture,
    counts::models::{Counts, CountsError, CountsReport, Sweep},
    stats::models::repo_name::RepoName,
};
use std::future::Future;

/// Measures a repository's current source.
pub trait CountsSource: Clone + Send + Sync + 'static {
    /// Fetches `repo`'s default branch and counts it.
    fn counts(&self, repo: &RepoName) -> impl Future<Output = Result<Counts, CountsError>> + Send;
}

/// Service port for counts.
pub trait CountsService: Clone + Send + Sync + 'static {
    /// The last measurement of `repo`, or `None` when it has not been measured.
    fn counts(&self, repo: &RepoName) -> impl Future<Output = Option<CountsReport>> + Send;

    /// Measures every allowlisted repository that was pushed to since its last
    /// measurement, one at a time.
    fn sweep(&self) -> impl Future<Output = Sweep> + Send;
}

/// Object-safe wrapper used by `AppState` and the sweeper task. Auto-implemented
/// for any `CountsService`.
pub trait ErasedCountsService: Send + Sync + 'static {
    /// See [`CountsService::counts`].
    fn counts<'a>(&'a self, repo: &'a RepoName) -> BoxFuture<'a, Option<CountsReport>>;

    /// See [`CountsService::sweep`].
    fn sweep(&self) -> BoxFuture<'_, Sweep>;
}

impl<T> ErasedCountsService for T
where
    T: CountsService,
{
    fn counts<'a>(&'a self, repo: &'a RepoName) -> BoxFuture<'a, Option<CountsReport>> {
        Box::pin(CountsService::counts(self, repo))
    }

    fn sweep(&self) -> BoxFuture<'_, Sweep> {
        Box::pin(CountsService::sweep(self))
    }
}
