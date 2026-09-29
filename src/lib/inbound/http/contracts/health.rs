use serde::Serialize;

/// Body of `GET /`.
#[derive(Debug, Serialize)]
#[allow(missing_docs)]
pub struct HttpRoot {
    pub message: &'static str,
    pub version: &'static str,
    pub status: &'static str,
}

/// Body of `GET /health`.
#[derive(Debug, Serialize)]
#[allow(missing_docs)]
pub struct HttpHealth {
    pub status: &'static str,
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
