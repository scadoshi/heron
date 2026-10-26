//! GitHub statistics for the allowlisted repositories.
//!
//! The service reads through a cache. Freshness lives inside the cached payload and
//! the cache's own expiry is a longer retention window, which is what lets a stale
//! value be served while GitHub is unreachable.

pub mod models;
pub mod ports;
pub mod services;
