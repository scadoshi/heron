//! scotland-server: Scotty Fermo's personal server.
//!
//! Serves GitHub statistics for the projects on scottyfermo.com as JSON, cached behind
//! a port so the store can be an in-process map or steller.
//!
//! # Layout
//!
//! Ports and adapters. The dependency arrow points inward: adapters depend on the
//! domain, never the reverse.

#![warn(missing_docs)]

/// Core models, ports, and services.
pub mod domain;

/// Adapters the domain's ports are implemented by.
pub mod outbound;

#[cfg(test)]
pub mod test_support;
