use crate::domain::{
    calendar::{
        models::{Calendar, CalendarError, CalendarReport},
        ports::{CalendarService, CalendarSource},
    },
    clock::Clock,
    stats::{
        models::{cache_key::CacheKey, snapshot::Snapshot},
        ports::StatsCache,
    },
};
use chrono::TimeDelta;
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

/// Whose calendar, and for how long it is served.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The GitHub account.
    pub login: String,
    /// How long a calendar is served without asking GitHub again.
    pub fresh: Duration,
    /// How long the cache keeps one. The gap past `fresh` is the window in which
    /// a stale calendar can cover for a failing source.
    pub retain: Duration,
}

/// Calendar service: reads through the cache and refreshes from the source.
#[derive(Clone)]
pub struct Service<S: CalendarSource, C: StatsCache, K: Clock> {
    source: S,
    cache: C,
    clock: K,
    login: Arc<str>,
    /// Concurrent callers on an expired calendar cause one refresh.
    refreshing: Arc<Mutex<()>>,
    fresh: TimeDelta,
    retain: Duration,
}

impl<S: CalendarSource, C: StatsCache, K: Clock> Service<S, C, K> {
    /// Creates a calendar service over the given adapters.
    pub fn new(source: S, cache: C, clock: K, settings: Settings) -> Self {
        Self {
            source,
            cache,
            clock,
            login: settings.login.into(),
            refreshing: Arc::new(Mutex::new(())),
            fresh: TimeDelta::from_std(settings.fresh).unwrap_or(TimeDelta::MAX),
            retain: settings.retain,
        }
    }

    fn key(&self) -> CacheKey {
        CacheKey::for_calendar(&self.login)
    }

    /// The snapshot under `key`. A cache error or an undecodable payload is a miss.
    async fn read(&self, key: &CacheKey) -> Option<Snapshot<Calendar>> {
        let bytes = match self.cache.get(key).await {
            Ok(bytes) => bytes?,
            Err(error) => {
                tracing::warn!("cache read failed for {key}, treating as a miss: {error}");
                return None;
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::warn!("cached payload under {key} did not decode: {error}");
                None
            }
        }
    }

    /// Stores `snapshot`. A failure is logged and swallowed.
    async fn write(&self, key: &CacheKey, snapshot: &Snapshot<Calendar>) {
        let bytes = match serde_json::to_vec(snapshot) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!("snapshot for {key} did not encode: {error}");
                return;
            }
        };
        if let Err(error) = self.cache.set(key, &bytes, self.retain).await {
            tracing::warn!("cache write failed for {key}: {error}");
        }
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

impl<S: CalendarSource, C: StatsCache, K: Clock> CalendarService for Service<S, C, K> {
    async fn calendar(&self) -> Result<CalendarReport, CalendarError> {
        let key = self.key();
        if let Some(report) = self.fresh_report(self.read(&key).await.as_ref()) {
            return Ok(report);
        }

        let _refreshing = self.refreshing.lock().await;
        // Another caller may have refreshed while this one waited for the lock.
        let cached = self.read(&key).await;
        if let Some(report) = self.fresh_report(cached.as_ref()) {
            return Ok(report);
        }

        match self.source.calendar(&self.login).await {
            Ok(calendar) => {
                let fetched_at = self.clock.now();
                let snapshot = Snapshot {
                    value: calendar,
                    fetched_at,
                    fresh_until: fetched_at
                        .checked_add_signed(self.fresh)
                        .unwrap_or(fetched_at),
                };
                self.write(&key, &snapshot).await;
                Ok(CalendarReport {
                    calendar: snapshot.value,
                    fetched_at,
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
                login: "scadoshi".to_string(),
                fresh: Duration::from_secs(fresh_secs),
                retain: Duration::from_hours(168),
            },
        );
        (service, source, clock)
    }

    #[tokio::test]
    async fn the_first_call_reads_github_and_the_next_one_reads_the_cache() {
        let (service, source, clock) = service(3600);
        let first = service.calendar().await.unwrap();
        assert_eq!(first.calendar.login, "scadoshi");
        assert_eq!(first.calendar.total, 5099);
        assert!(!first.stale);
        clock.advance(3599);
        let second = service.calendar().await.unwrap();
        assert_eq!(second, first);
        assert_eq!(source.calls(), 1);
    }

    #[tokio::test]
    async fn past_freshness_github_is_asked_again() {
        let (service, source, clock) = service(3600);
        service.calendar().await.unwrap();
        clock.advance(3600);
        source.set_total(5100);
        let report = service.calendar().await.unwrap();
        assert_eq!(report.calendar.total, 5100);
        assert_eq!(source.calls(), 2);
    }

    #[tokio::test]
    async fn a_stale_calendar_covers_for_github_and_is_marked() {
        let (service, source, clock) = service(3600);
        let first = service.calendar().await.unwrap();
        clock.advance(3600);
        source.fail(true);
        let report = service.calendar().await.unwrap();
        assert!(report.stale);
        assert_eq!(report.calendar, first.calendar);
        assert_eq!(report.fetched_at, first.fetched_at);
    }

    #[tokio::test]
    async fn with_nothing_cached_a_failure_is_the_answer() {
        let (service, source, _) = service(3600);
        source.fail(true);
        assert!(service.calendar().await.is_err());
    }
}
