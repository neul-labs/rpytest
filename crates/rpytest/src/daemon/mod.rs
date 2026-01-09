//! Daemon management and client wrapper.

mod client;
mod lifecycle;

pub use client::DaemonManager;
#[allow(unused_imports)]
pub use lifecycle::{
    CleanupResult, ContextCleaner, DaemonInfo, DaemonState, HealthCheckResult, LifecycleConfig,
    LifecycleManager,
};
