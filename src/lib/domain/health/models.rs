use thiserror::Error;

/// A dependency did not answer.
#[derive(Debug, Error)]
#[error("failed health check: {0:#}")]
pub struct HealthCheckFailed(pub anyhow::Error);
