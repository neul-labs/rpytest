//! Inventory storage and persistence.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::nodes::{TestNode, TestNodeId};
use crate::storage::{keys, StorageBackend, StorageError, StorageResult, SCHEMA_VERSION};

/// Metadata about an inventory snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryMeta {
    /// Hash of the inventory contents for change detection.
    pub hash: String,

    /// Timestamp when the inventory was collected (Unix epoch ms).
    pub collected_at: u64,

    /// Number of test nodes in the inventory.
    pub node_count: usize,

    /// Schema version used for this inventory.
    pub schema_version: u32,
}

impl Default for InventoryMeta {
    fn default() -> Self {
        Self {
            hash: String::new(),
            collected_at: 0,
            node_count: 0,
            schema_version: SCHEMA_VERSION,
        }
    }
}

/// An in-memory inventory of test nodes.
#[derive(Debug, Clone, Default)]
pub struct Inventory {
    /// Test nodes indexed by node ID.
    nodes: HashMap<TestNodeId, TestNode>,

    /// Metadata about this inventory.
    meta: InventoryMeta,
}

impl Inventory {
    /// Create a new empty inventory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a test node to the inventory.
    pub fn add(&mut self, node: TestNode) {
        self.nodes.insert(node.node_id.clone(), node);
        self.meta.node_count = self.nodes.len();
    }

    /// Get a test node by ID.
    pub fn get(&self, node_id: &str) -> Option<&TestNode> {
        self.nodes.get(node_id)
    }

    /// Get a mutable reference to a test node by ID.
    pub fn get_mut(&mut self, node_id: &str) -> Option<&mut TestNode> {
        self.nodes.get_mut(node_id)
    }

    /// Remove a test node by ID.
    pub fn remove(&mut self, node_id: &str) -> Option<TestNode> {
        let result = self.nodes.remove(node_id);
        self.meta.node_count = self.nodes.len();
        result
    }

    /// Check if the inventory contains a node.
    pub fn contains(&self, node_id: &str) -> bool {
        self.nodes.contains_key(node_id)
    }

    /// Get the number of nodes in the inventory.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if the inventory is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Get an iterator over all nodes.
    pub fn iter(&self) -> impl Iterator<Item = &TestNode> {
        self.nodes.values()
    }

    /// Get all node IDs.
    pub fn node_ids(&self) -> Vec<&TestNodeId> {
        self.nodes.keys().collect()
    }

    /// Get the inventory metadata.
    pub fn meta(&self) -> &InventoryMeta {
        &self.meta
    }

    /// Update metadata after collection.
    pub fn update_meta(&mut self, hash: String, collected_at: u64) {
        self.meta.hash = hash;
        self.meta.collected_at = collected_at;
        self.meta.node_count = self.nodes.len();
        self.meta.schema_version = SCHEMA_VERSION;
    }

    /// Filter nodes by keyword expression (-k).
    pub fn filter_by_keyword(&self, expr: &str) -> Vec<&TestNode> {
        self.nodes
            .values()
            .filter(|node| node.matches_keyword(expr))
            .collect()
    }

    /// Filter nodes by marker expression (-m).
    pub fn filter_by_marker(&self, expr: &str) -> Vec<&TestNode> {
        self.nodes
            .values()
            .filter(|node| node.matches_marker(expr))
            .collect()
    }

    /// Filter nodes by both keyword and marker.
    pub fn filter(&self, keyword: Option<&str>, marker: Option<&str>) -> Vec<&TestNode> {
        self.nodes
            .values()
            .filter(|node| {
                keyword.map_or(true, |k| node.matches_keyword(k))
                    && marker.map_or(true, |m| node.matches_marker(m))
            })
            .collect()
    }

    /// Get nodes sorted by historical duration (longest first).
    pub fn sorted_by_duration(&self) -> Vec<&TestNode> {
        let mut nodes: Vec<_> = self.nodes.values().collect();
        nodes.sort_by(|a, b| {
            b.avg_duration_ms
                .unwrap_or(0)
                .cmp(&a.avg_duration_ms.unwrap_or(0))
        });
        nodes
    }

    /// Get nodes sorted by failure rate (most flaky first).
    pub fn sorted_by_failure_rate(&self) -> Vec<&TestNode> {
        let mut nodes: Vec<_> = self.nodes.values().collect();
        nodes.sort_by(|a, b| {
            b.failure_rate()
                .partial_cmp(&a.failure_rate())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        nodes
    }

    /// Save the inventory to storage.
    pub fn save<S: StorageBackend>(&self, storage: &S, context_id: &str) -> StorageResult<()> {
        // Save each node
        for (node_id, node) in &self.nodes {
            let key = format!(
                "{}{}:{}",
                String::from_utf8_lossy(keys::INVENTORY),
                context_id,
                node_id
            );
            let value =
                rmp_serde::to_vec(node).map_err(|e| StorageError::Serialization(e.to_string()))?;
            storage.set(key.as_bytes(), &value)?;
        }

        // Save metadata
        let meta_key = format!(
            "{}{}:_meta",
            String::from_utf8_lossy(keys::CONTEXT),
            context_id
        );
        let meta_value = rmp_serde::to_vec(&self.meta)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        storage.set(meta_key.as_bytes(), &meta_value)?;

        // Save schema version
        let schema_value = SCHEMA_VERSION.to_le_bytes();
        storage.set(keys::SCHEMA, &schema_value)?;

        storage.flush()?;
        Ok(())
    }

    /// Load the inventory from storage.
    pub fn load<S: StorageBackend>(storage: &S, context_id: &str) -> StorageResult<Self> {
        // Check schema version
        if let Some(version_bytes) = storage.get(keys::SCHEMA)? {
            if version_bytes.len() >= 4 {
                let version = u32::from_le_bytes([
                    version_bytes[0],
                    version_bytes[1],
                    version_bytes[2],
                    version_bytes[3],
                ]);
                if version != SCHEMA_VERSION {
                    return Err(StorageError::Corrupted(format!(
                        "Schema version mismatch: expected {}, found {}",
                        SCHEMA_VERSION, version
                    )));
                }
            }
        }

        let mut inventory = Self::new();

        // Load metadata
        let meta_key = format!(
            "{}{}:_meta",
            String::from_utf8_lossy(keys::CONTEXT),
            context_id
        );
        if let Some(meta_bytes) = storage.get(meta_key.as_bytes())? {
            inventory.meta = rmp_serde::from_slice(&meta_bytes)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
        }

        // Load all nodes for this context
        let prefix = format!(
            "{}{}:",
            String::from_utf8_lossy(keys::INVENTORY),
            context_id
        );
        let entries = storage.scan_prefix(prefix.as_bytes())?;

        for (_key, value) in entries {
            let node: TestNode = rmp_serde::from_slice(&value)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            inventory.nodes.insert(node.node_id.clone(), node);
        }

        inventory.meta.node_count = inventory.nodes.len();
        Ok(inventory)
    }

    /// Clear the inventory for a context from storage.
    pub fn clear_storage<S: StorageBackend>(storage: &S, context_id: &str) -> StorageResult<()> {
        // Delete all nodes
        let prefix = format!(
            "{}{}:",
            String::from_utf8_lossy(keys::INVENTORY),
            context_id
        );
        let entries = storage.scan_prefix(prefix.as_bytes())?;
        for (key, _) in entries {
            storage.delete(&key)?;
        }

        // Delete metadata
        let meta_key = format!(
            "{}{}:_meta",
            String::from_utf8_lossy(keys::CONTEXT),
            context_id
        );
        storage.delete(meta_key.as_bytes())?;

        storage.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SledBackend;
    use tempfile::TempDir;

    fn create_test_inventory() -> Inventory {
        let mut inventory = Inventory::new();

        let mut node1 = TestNode::new("test_math.py::test_add", "test_math.py");
        node1.add_marker("unit");
        node1.build_keywords();
        inventory.add(node1);

        let mut node2 = TestNode::new("test_math.py::test_subtract", "test_math.py");
        node2.add_marker("unit");
        node2.build_keywords();
        inventory.add(node2);

        let mut node3 = TestNode::new("test_api.py::test_login", "test_api.py");
        node3.add_marker("integration");
        node3.add_marker("slow");
        node3.build_keywords();
        inventory.add(node3);

        inventory
    }

    #[test]
    fn test_basic_operations() {
        let inventory = create_test_inventory();

        assert_eq!(inventory.len(), 3);
        assert!(inventory.contains("test_math.py::test_add"));
        assert!(!inventory.contains("nonexistent"));

        let node = inventory.get("test_api.py::test_login").unwrap();
        assert!(node.has_marker("integration"));
    }

    #[test]
    fn test_keyword_filter() {
        let inventory = create_test_inventory();

        let results = inventory.filter_by_keyword("math");
        assert_eq!(results.len(), 2);

        let results = inventory.filter_by_keyword("login");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_marker_filter() {
        let inventory = create_test_inventory();

        let results = inventory.filter_by_marker("unit");
        assert_eq!(results.len(), 2);

        let results = inventory.filter_by_marker("integration");
        assert_eq!(results.len(), 1);

        let results = inventory.filter_by_marker("slow");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_combined_filter() {
        let inventory = create_test_inventory();

        let results = inventory.filter(Some("test"), Some("unit"));
        assert_eq!(results.len(), 2);

        let results = inventory.filter(Some("login"), Some("integration"));
        assert_eq!(results.len(), 1);

        let results = inventory.filter(Some("math"), Some("integration"));
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_persistence() {
        let tmp = TempDir::new().unwrap();
        let storage = SledBackend::open(tmp.path()).unwrap();

        // Save inventory
        let mut inventory = create_test_inventory();
        inventory.update_meta("abc123".to_string(), 1234567890);
        inventory.save(&storage, "ctx-1").unwrap();

        // Load inventory
        let loaded = Inventory::load(&storage, "ctx-1").unwrap();

        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded.meta().hash, "abc123");
        assert!(loaded.contains("test_math.py::test_add"));
    }

    #[test]
    fn test_clear_storage() {
        let tmp = TempDir::new().unwrap();
        let storage = SledBackend::open(tmp.path()).unwrap();

        let inventory = create_test_inventory();
        inventory.save(&storage, "ctx-1").unwrap();

        // Verify saved
        let loaded = Inventory::load(&storage, "ctx-1").unwrap();
        assert_eq!(loaded.len(), 3);

        // Clear
        Inventory::clear_storage(&storage, "ctx-1").unwrap();

        // Verify cleared
        let loaded = Inventory::load(&storage, "ctx-1").unwrap();
        assert_eq!(loaded.len(), 0);
    }
}
