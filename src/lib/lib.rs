//! heron: Scotty Fermo's personal server.
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
//! `src/bin/heron.rs` wires adapters into services and starts the server.

#![warn(missing_docs)]

pub mod config;
pub mod domain;
pub mod inbound;
pub mod outbound;

#[cfg(test)]
pub mod test_support;
