//! Cache adapters. Which one runs is chosen by `CACHE_BACKEND` at startup.

/// Two caches stacked: a primary, and a secondary that covers for it.
pub mod layered;

/// An in-process map.
pub mod memory;

/// steller, over a hand-written RESP client.
pub mod steller;
