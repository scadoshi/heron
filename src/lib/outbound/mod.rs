//! Outbound adapters: the things the domain's ports are implemented by.

/// Cache adapters behind the `StatsCache` port.
pub mod cache;

/// The system clock behind the `Clock` port.
pub mod clock;

/// GitHub's REST API behind the `StatsSource` port.
pub mod github;
