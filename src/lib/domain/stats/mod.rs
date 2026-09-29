//! GitHub statistics for the allowlisted repositories.
//!
//! The service reads through a cache. Freshness lives inside the cached payload and
//! the cache's own expiry is a longer retention window, which is what lets a stale
//! value be served while GitHub is unreachable.

/// Entities, value objects, and errors.
pub mod models;

/// Port traits for the source, the cache, and the service.
pub mod ports;

/// Service implementation.
pub mod services;
