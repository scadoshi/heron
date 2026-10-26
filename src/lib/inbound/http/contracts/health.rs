//! Bodies of the root and health endpoints.

use serde::Serialize;

/// Body of `GET /`.
#[derive(Debug, Serialize)]
pub struct HttpRoot {
    /// The package name, `heron`.
    pub message: &'static str,
    /// The running build's package version, as in `Cargo.toml`.
    pub version: &'static str,
    /// Always `ready`.
    pub status: &'static str,
}

/// Body of `GET /health`.
#[derive(Debug, Serialize)]
pub struct HttpHealth {
    /// Always `healthy`: a server that can answer is up.
    pub status: &'static str,
    /// The running build's package version, as in `Cargo.toml`.
    pub version: &'static str,
}

/// Body of `GET /health/cache`.
#[derive(Debug, Serialize)]
pub struct HttpCacheHealth {
    /// `memory`, `steller`, or `layered`.
    pub backend: &'static str,
    /// `healthy`, or `unreachable` when the backend did not answer a ping. Under
    /// `layered` this describes steller, and requests are still served from memory.
    pub status: &'static str,
}
