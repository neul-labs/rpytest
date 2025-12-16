//! Daemon management and client wrapper.

mod client;
mod lifecycle;

pub use client::DaemonManager;
pub use lifecycle::{
    ContextCleaner, CleanupResult, DaemonInfo, DaemonState,
    HealthCheckResult, LifecycleConfig, LifecycleManager,
};
