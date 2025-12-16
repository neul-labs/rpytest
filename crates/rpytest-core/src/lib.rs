//! Core types and abstractions for rpytest.
//!
//! This crate provides the shared protocol messages, test outcome types,
//! and storage abstraction used across rpytest components.

pub mod inventory;
pub mod protocol;
pub mod storage;

pub use inventory::{Inventory, InventoryMeta, TestNode, TestNodeId, TestNodeKind};
pub use protocol::{ErrorCode, Outcome, Request, Response, TestEvent, TestNodeInfo};
pub use storage::{SledBackend, StorageBackend, StorageError, StorageResult};
