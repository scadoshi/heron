//! The core: models, validation, the ports adapters plug into, and the services that
//! orchestrate them. Nothing here knows about HTTP, TCP, GitHub, RESP, or the system
//! clock.

pub mod calendar;
use std::future::Future;
use std::pin::Pin;

pub mod clock;
pub mod counts;
pub mod health;
pub mod login;
pub mod secret;
pub mod stats;

/// Boxed future returned by the `ErasedXService` twins (see each domain's ports). One
/// alias so the erased signatures stay readable.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
