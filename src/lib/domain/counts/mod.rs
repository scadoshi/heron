//! Lines, tests and lints counted from each allowlisted repository's source.
//!
//! Nothing here is measured on request. A sweep, run on a timer by the composition
//! root, measures a repository when nothing is cached for it or when GitHub's
//! `pushed_at` is newer than the cached measurement, and the service only ever reads
//! what the last sweep wrote.

pub mod models;
pub mod ports;
pub mod services;
