//! rpytest-daemon: Pure Rust daemon for test execution and management.
//!
//! This crate provides a high-performance Rust daemon for rpytest that handles:
//! - IPC communication with the CLI via NNG
//! - Test collection using native AST parsing
//! - Inventory and state management with sled storage
//! - Test scheduling and execution
//! - Flakiness tracking and fixture management

pub mod error;
pub mod models;
pub mod storage;
pub mod collector;
pub mod scheduler;
pub mod flakiness;
pub mod fixtures;
pub mod context;
pub mod executor;
pub mod server;

pub use error::{Result, DaemonError};
pub use models::{TestNode, TestResult, TestOutcome};
pub use storage::DaemonStorage;
pub use collector::NativeCollector;
pub use scheduler::TestScheduler;
pub use models::ScheduledTest;
pub use flakiness::FlakinessTracker;
pub use models::{FlakinessRecord, FixtureState, FixtureScope};
pub use fixtures::FixtureManager;
pub use context::RepoContext;
pub use executor::PythonExecutor;
pub use server::DaemonServer;
pub use models::DaemonConfig;

/// Re-export commonly used types
pub use rpytest_core::protocol::{Request, Response, ErrorCode, TestNodeInfo, PROTOCOL_VERSION};
