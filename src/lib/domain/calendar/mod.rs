//! A year of GitHub contributions for one account, the calendar the profile
//! page draws.
//!
//! Read through the cache like stats: fresh for `CALENDAR_FRESH_SECS`, kept for
//! `STATS_RETAIN_SECS`, and served stale when GitHub cannot answer.

/// The calendar, its report, and errors.
pub mod models;

/// Port traits for the source and the service.
pub mod ports;

/// Service implementation.
pub mod services;
