//! Port traits for the calendar: where it comes from, and the service the HTTP
//! layer calls.

use crate::domain::{
    BoxFuture,
    calendar::models::{Calendar, CalendarError, CalendarReport},
};
use std::future::Future;

/// Where the calendar is read from.
pub trait CalendarSource: Clone + Send + Sync + 'static {
    /// The last year of contributions for `login`.
    fn calendar(&self, login: &str)
    -> impl Future<Output = Result<Calendar, CalendarError>> + Send;
}

/// Service port for the calendar.
pub trait CalendarService: Clone + Send + Sync + 'static {
    /// The configured account's calendar, from cache when fresh.
    fn calendar(&self) -> impl Future<Output = Result<CalendarReport, CalendarError>> + Send;
}

/// Object-safe wrapper used by `AppState`. Auto-implemented for any
/// `CalendarService`.
pub trait ErasedCalendarService: Send + Sync + 'static {
    /// See [`CalendarService::calendar`].
    fn calendar(&self) -> BoxFuture<'_, Result<CalendarReport, CalendarError>>;
}

impl<T> ErasedCalendarService for T
where
    T: CalendarService,
{
    fn calendar(&self) -> BoxFuture<'_, Result<CalendarReport, CalendarError>> {
        Box::pin(CalendarService::calendar(self))
    }
}
