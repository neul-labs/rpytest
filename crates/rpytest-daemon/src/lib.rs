//! rpytest-daemon: Pure Rust daemon for test execution and management.
//!
//! This crate provides a high-performance Rust daemon for rpytest that handles:
//! - IPC communication with the CLI via NNG
//! - Test collection using native AST parsing
//! - Inventory and state management with sled storage
//! - Test scheduling and execution
//! - Flakiness tracking and fixture management
//!
//! ## Features
//!
//! - `embedded-python` (default): Use PyO3 to embed Python for direct pytest execution,
//!   eliminating subprocess overhead.
//! - `subprocess-only`: Disable embedded Python, always use subprocess execution.

pub mod collector;
pub mod context;
pub mod error;
pub mod executor;
pub mod fixtures;
pub mod flakiness;
pub mod models;
pub mod pool;
pub mod scheduler;
pub mod server;
pub mod storage;

#[cfg(feature = "embedded-python")]
pub mod embedded;

pub use collector::NativeCollector;
pub use context::RepoContext;
pub use error::{DaemonError, Result};
pub use executor::{
    create_executor, create_pooled_executor, PooledExecutor, PythonExecutor, TestExecutor,
};

#[cfg(feature = "embedded-python")]
pub use embedded::EmbeddedExecutor;
pub use fixtures::FixtureManager;
pub use flakiness::FlakinessTracker;
pub use models::DaemonConfig;
pub use models::ExecutionMode;
pub use models::ScheduledTest;
pub use models::{FixtureScope, FixtureState, FlakinessRecord};
pub use models::{TestNode, TestOutcome, TestResult};
pub use pool::WorkerPool;
pub use scheduler::TestScheduler;
pub use server::DaemonServer;
pub use storage::DaemonStorage;

/// Re-export commonly used types
pub use rpytest_core::protocol::{ErrorCode, Request, Response, TestNodeInfo, PROTOCOL_VERSION};
