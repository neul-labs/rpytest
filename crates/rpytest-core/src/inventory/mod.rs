//! Test inventory management.
//!
//! This module provides data structures and operations for managing
//! test inventories - the collection of all known tests in a repository.

mod nodes;
mod store;

pub use nodes::{TestNode, TestNodeId, TestNodeKind};
pub use store::{Inventory, InventoryMeta};
