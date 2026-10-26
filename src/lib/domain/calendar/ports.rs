//! Port traits for the calendar: where it comes from, and the service the HTTP
//! layer calls.

use std::future::Future;

use crate::domain::BoxFuture;
use crate::domain::calendar::models::{Calendar, CalendarError, CalendarReport};

/// Where the calendar is read from.
pub trait CalendarSource: Clone + Send + Sync + 'static {
    /// The last year of contributions for `login`.
    fn fetch_calendar(
        &self,
        login: &str,
    ) -> impl Future<Output = Result<Calendar, CalendarError>> + Send;
}

/// The configured account's calendar, as `GET /stats` serves it.
///
/// Reads through the cache. Past freshness one caller refreshes from the
/// [`CalendarSource`] while the others wait for its answer, and when that refresh
/// fails the cached calendar is served with `stale` set. With nothing cached, the
/// source's error is the answer.
pub trait CalendarService: Clone + Send + Sync + 'static {
    /// The configured account's calendar, from cache when fresh.
    fn fetch_calendar(&self) -> impl Future<Output = Result<CalendarReport, CalendarError>> + Send;
}

/// Object-safe wrapper used by `AppState`. Auto-implemented for any
/// `CalendarService`.
pub trait ErasedCalendarService: Send + Sync + 'static {
    /// See [`CalendarService::fetch_calendar`].
    fn fetch_calendar(&self) -> BoxFuture<'_, Result<CalendarReport, CalendarError>>;
}

impl<T> ErasedCalendarService for T
where
    T: CalendarService,
{
    fn fetch_calendar(&self) -> BoxFuture<'_, Result<CalendarReport, CalendarError>> {
        Box::pin(CalendarService::fetch_calendar(self))
    }
}
