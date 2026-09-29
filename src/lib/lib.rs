//! scotland-server: Scotty Fermo's personal server.
//!
//! Serves GitHub statistics for the projects on scottyfermo.com as JSON, cached behind
//! a port so the store can be an in-process map or steller.
//!
//! # Layout
//!
//! Ports and adapters. The dependency arrow points inward: adapters depend on the
//! domain, never the reverse.
//!
//! - [`config`]: configuration from the environment, validated at startup
//! - [`domain`]: models, validation, ports, and services
//! - [`inbound`]: the HTTP API
//! - [`outbound`]: GitHub, the cache adapters, and the system clock
//!
//! `src/bin/scotland_server.rs` wires adapters into services and starts the server.

#![warn(missing_docs)]

/// Configuration from the environment.
pub mod config;

/// Core models, ports, and services.
pub mod domain;

/// The HTTP API.
pub mod inbound;

/// Adapters the domain's ports are implemented by.
pub mod outbound;

#[cfg(test)]
pub mod test_support;
