//! Lines, tests and lints counted from each allowlisted repository's source.
//!
//! Nothing here is measured on request. A sweep, run on a timer by the composition
//! root, measures a repository when nothing is cached for it or when GitHub's
//! `pushed_at` is newer than the cached measurement, and the service only ever reads
//! what the last sweep wrote.

/// Report, sweep summary and errors.
pub mod models;

/// Port traits for the source and the service.
pub mod ports;

/// Service implementation.
pub mod services;
