//! [`Service`], the calendar service over a source, a cache and a clock.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use chrono::TimeDelta;
use tokio::sync::Mutex;

use crate::domain::calendar::models::{Calendar, CalendarError, CalendarReport};
use crate::domain::calendar::ports::{CalendarService, CalendarSource};
use crate::domain::clock::Clock;
use crate::domain::login::Login;
use crate::domain::stats::models::cache_key::CacheKey;
use crate::domain::stats::models::snapshot::{self, Snapshot};
use crate::domain::stats::ports::Cache;

/// Whose calendar, and for how long it is served.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The GitHub account.
    pub login: Login,
    /// How long a calendar is served without asking GitHub again.
    pub fresh: Duration,
    /// How long the cache keeps one. The gap past `fresh` is the window in which
    /// a stale calendar can cover for a failing source.
    pub retain: Duration,
}

/// Calendar service: reads through the cache and refreshes from the source.
#[derive(Clone)]
pub struct Service<S, C, K> {
    source: S,
    cache: C,
    clock: K,
    login: Login,
    /// Concurrent callers on an expired calendar cause one refresh.
    refreshing: Arc<Mutex<()>>,
    fresh: TimeDelta,
    retain: Duration,
}

/// Prints the settings only: the adapters need not be `Debug`, and the refresh
/// lock is never taken to print it.
impl<S, C, K> fmt::Debug for Service<S, C, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Service")
            .field("login", &self.login)
            .field("fresh", &self.fresh)
            .field("retain", &self.retain)
            .finish_non_exhaustive()
    }
}

impl<S: CalendarSource, C: Cache, K: Clock> Service<S, C, K> {
    /// Creates a calendar service over the given adapters.
    pub fn new(source: S, cache: C, clock: K, settings: Settings) -> Self {
        Self {
            source,
            cache,
            clock,
            login: settings.login,
            refreshing: Arc::new(Mutex::new(())),
            fresh: TimeDelta::from_std(settings.fresh).unwrap_or(TimeDelta::MAX),
            retain: settings.retain,
        }
    }

    fn key(&self) -> CacheKey {
        CacheKey::for_calendar(&self.login)
    }

    fn fresh_report(&self, snapshot: Option<&Snapshot<Calendar>>) -> Option<CalendarReport> {
        let snapshot = snapshot.filter(|s| s.is_fresh(self.clock.now()))?;
        Some(CalendarReport {
            calendar: snapshot.value.clone(),
            fetched_at: snapshot.fetched_at,
            stale: false,
        })
    }
}

impl<S: CalendarSource, C: Cache, K: Clock> CalendarService for Service<S, C, K> {
    async fn fetch_calendar(&self) -> Result<CalendarReport, CalendarError> {
        let key = self.key();
        if let Some(report) = self.fresh_report(snapshot::read(&self.cache, &key).await.as_ref()) {
            return Ok(report);
        }

        let _refreshing = self.refreshing.lock().await;
        // Another caller may have refreshed while this one waited for the lock.
        let cached = snapshot::read(&self.cache, &key).await;
        if let Some(report) = self.fresh_report(cached.as_ref()) {
            return Ok(report);
        }

        match self.source.fetch_calendar(&self.login).await {
            Ok(calendar) => {
                let fetched = Snapshot::new(calendar, self.clock.now(), self.fresh);
                snapshot::write(&self.cache, &key, &fetched, self.retain).await;
                Ok(CalendarReport {
                    calendar: fetched.value,
                    fetched_at: fetched.fetched_at,
                    stale: false,
                })
            }
            Err(error) => {
                let Some(stale) = cached else {
                    return Err(error);
                };
                tracing::warn!("calendar refresh failed, serving the stale value: {error}");
                Ok(CalendarReport {
                    calendar: stale.value,
                    fetched_at: stale.fetched_at,
                    stale: true,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{FakeCache, FakeCalendar, FakeClock};

    fn service(
        fresh_secs: u64,
    ) -> (
        Service<FakeCalendar, FakeCache, FakeClock>,
        FakeCalendar,
        FakeClock,
    ) {
        let source = FakeCalendar::default();
        let clock = FakeClock::default();
        let service = Service::new(
            source.clone(),
            FakeCache::default(),
            clock.clone(),
            Settings {
                login: Login::new("scadoshi").unwrap(),
                fresh: Duration::from_secs(fresh_secs),
                retain: Duration::from_hours(168),
            },
        );
        (service, source, clock)
    }

    #[tokio::test]
    async fn the_first_call_reads_github_and_the_next_one_reads_the_cache() {
        let (service, source, clock) = service(3600);
        let first = service.fetch_calendar().await.unwrap();
        assert_eq!(first.calendar.login.as_str(), "scadoshi");
        assert_eq!(first.calendar.total, 5099);
        assert!(!first.stale);
        clock.advance(Duration::from_secs(3599));
        let second = service.fetch_calendar().await.unwrap();
        assert_eq!(second, first);
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn past_freshness_github_is_asked_again() {
        let (service, source, clock) = service(3600);
        service.fetch_calendar().await.unwrap();
        clock.advance(Duration::from_secs(3600));
        source.set_total(5100);
        let report = service.fetch_calendar().await.unwrap();
        assert_eq!(report.calendar.total, 5100);
        assert_eq!(source.calls(), 2);
    }

    #[tokio::test]
    async fn a_stale_calendar_covers_for_github_and_is_marked() {
        let (service, source, clock) = service(3600);
        let first = service.fetch_calendar().await.unwrap();
        clock.advance(Duration::from_secs(3600));
        source.set_failing(true);
        let report = service.fetch_calendar().await.unwrap();
        assert!(report.stale);
        assert_eq!(report.calendar, first.calendar);
        assert_eq!(report.fetched_at, first.fetched_at);
    }

    #[tokio::test]
    async fn with_nothing_cached_a_failure_is_the_answer() {
        let (service, source, _) = service(3600);
        source.set_failing(true);
        assert!(service.fetch_calendar().await.is_err());
    }
}
