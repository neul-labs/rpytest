//! Core types and abstractions for rpytest.
//!
//! This crate provides the shared protocol messages, test outcome types,
//! and storage abstraction used across rpytest components.

pub mod protocol;
pub mod storage;

pub use protocol::{ErrorCode, Outcome, Request, Response, TestEvent};
pub use storage::StorageBackend;
