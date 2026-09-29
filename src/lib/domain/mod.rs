//! The core: models, validation, the ports adapters plug into, and the services that
//! orchestrate them. Nothing here knows about HTTP, TCP, GitHub, RESP, or the system
//! clock.

/// The time port.
pub mod clock;

/// Cache liveness for monitoring.
pub mod health;

/// Redacting wrapper for credentials.
pub mod secret;

/// GitHub statistics for the allowlisted repositories.
pub mod stats;

/// Boxed future returned by the `ErasedXService` twins (see each domain's ports). One
/// alias so the erased signatures stay readable.
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
