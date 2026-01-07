//! Storage layer using sled for persistent data.

use crate::error::{DaemonError, Result};
use crate::models::{
    FixtureState, FlakinessRecord, NativeTestNode, ScheduledTest, TestNode,
};
use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};
use sled::{Db, Tree};
use std::path::PathBuf;
use tracing::{debug, error};

/// Storage tree names.
const TREE_INVENTORY: &str = "inventory";
const TREE_NATIVE_TESTS: &str = "native_tests";
const TREE_FLAKINESS: &str = "flakiness";
const TREE_FIXTURES: &str = "fixtures";
const TREE_DURATION_HISTORY: &str = "duration_history";
const TREE_CONTEXTS: &str = "contexts";
const TREE_SCHEDULER: &str = "scheduler";
const TREE_CONFIG: &str = "config";

/// Schema version for storage format compatibility.
const STORAGE_VERSION: u32 = 1;

/// Main storage wrapper for the daemon.
#[derive(Clone)]
pub struct DaemonStorage {
    db: Db,
    inventory: Tree,
    native_tests: Tree,
    flakiness: Tree,
    fixtures: Tree,
    duration_history: Tree,
    contexts: Tree,
    scheduler: Tree,
    config: Tree,
}

impl std::fmt::Debug for DaemonStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonStorage").finish_non_exhaustive()
    }
}

impl DaemonStorage {
    /// Open or create the storage database.
    pub fn open(storage_path: &PathBuf) -> Result<Self> {
        let config = sled::Config::default().path(storage_path).temporary(false);

        let db = config.open()?;

        // Open or create trees
        let inventory = db.open_tree(TREE_INVENTORY)?;
        let native_tests = db.open_tree(TREE_NATIVE_TESTS)?;
        let flakiness = db.open_tree(TREE_FLAKINESS)?;
        let fixtures = db.open_tree(TREE_FIXTURES)?;
        let duration_history = db.open_tree(TREE_DURATION_HISTORY)?;
        let contexts = db.open_tree(TREE_CONTEXTS)?;
        let scheduler = db.open_tree(TREE_SCHEDULER)?;
        let config_tree = db.open_tree(TREE_CONFIG)?;

        // Validate schema version
        if let Some(version_bytes) = config_tree.get("version")? {
            let version: u32 = rmp_serde::from_slice(&version_bytes)?;
            if version != STORAGE_VERSION {
                error!(
                    "Storage schema version mismatch: expected {}, found {}",
                    STORAGE_VERSION, version
                );
                return Err(DaemonError::Other(format!(
                    "Storage schema version mismatch: expected {}, found {}",
                    STORAGE_VERSION, version
                )));
            }
        } else {
            // First run - set version
            let mut version_bytes = Vec::new();
            STORAGE_VERSION.serialize(&mut Serializer::new(&mut version_bytes))?;
            config_tree.insert("version", version_bytes)?;
        }

        Ok(Self {
            db,
            inventory,
            native_tests,
            flakiness,
            fixtures,
            duration_history,
            contexts,
            scheduler,
            config: config_tree,
        })
    }

    /// Clear all data (for testing or reset).
    pub fn clear_all(&self) -> Result<()> {
        self.inventory.clear()?;
        self.native_tests.clear()?;
        self.flakiness.clear()?;
        self.fixtures.clear()?;
        self.duration_history.clear()?;
        self.contexts.clear()?;
        self.scheduler.clear()?;
        Ok(())
    }

    // ==================== Inventory ====================

    /// Save a test node to inventory.
    pub fn save_test_node(&self, node: &TestNode) -> Result<()> {
        let mut buf = Vec::new();
        node.serialize(&mut Serializer::new(&mut buf))?;
        self.inventory.insert(&node.node_id, buf)?;
        Ok(())
    }

    /// Load a test node from inventory.
    pub fn load_test_node(&self, node_id: &str) -> Result<Option<TestNode>> {
        if let Some(bytes) = self.inventory.get(node_id)? {
            let mut deserializer = Deserializer::new(&bytes[..]);
            let node: TestNode = Deserialize::deserialize(&mut deserializer)?;
            Ok(Some(node))
        } else {
            Ok(None)
        }
    }

    /// Get all test nodes in inventory.
    pub fn get_all_inventory(&self) -> Result<Vec<TestNode>> {
        let mut nodes = Vec::new();
        for item in self.inventory.iter() {
            let (_, bytes) = item?;
            let mut deserializer = Deserializer::new(&bytes[..]);
            let node: TestNode = Deserialize::deserialize(&mut deserializer)?;
            nodes.push(node);
        }
        Ok(nodes)
    }

    /// Get inventory count.
    pub fn inventory_count(&self) -> usize {
        self.inventory.len()
    }

    /// Clear inventory.
    pub fn clear_inventory(&self) -> Result<()> {
        self.inventory.clear()?;
        Ok(())
    }

    // ==================== Native Tests ====================

    /// Save a native test node.
    pub fn save_native_test(&self, node: &NativeTestNode) -> Result<()> {
        let mut buf = Vec::new();
        node.serialize(&mut Serializer::new(&mut buf))?;
        self.native_tests.insert(&node.node_id, buf)?;
        Ok(())
    }

    /// Load a native test node.
    pub fn load_native_test(&self, node_id: &str) -> Result<Option<NativeTestNode>> {
        if let Some(bytes) = self.native_tests.get(node_id)? {
            let mut deserializer = Deserializer::new(&bytes[..]);
            let node: NativeTestNode = Deserialize::deserialize(&mut deserializer)?;
            Ok(Some(node))
        } else {
            Ok(None)
        }
    }

    /// Get all native tests.
    pub fn get_all_native_tests(&self) -> Result<Vec<NativeTestNode>> {
        let mut nodes = Vec::new();
        for item in self.native_tests.iter() {
            let (_, bytes) = item?;
            let mut deserializer = Deserializer::new(&bytes[..]);
            let node: NativeTestNode = Deserialize::deserialize(&mut deserializer)?;
            nodes.push(node);
        }
        Ok(nodes)
    }

    /// Clear native tests.
    pub fn clear_native_tests(&self) -> Result<()> {
        self.native_tests.clear()?;
        Ok(())
    }

    // ==================== Flakiness ====================

    /// Save flakiness record.
    pub fn save_flakiness_record(&self, record: &FlakinessRecord) -> Result<()> {
        let mut buf = Vec::new();
        record.serialize(&mut Serializer::new(&mut buf))?;
        self.flakiness.insert(&record.node_id, buf)?;
        Ok(())
    }

    /// Load flakiness record.
    pub fn load_flakiness_record(&self, node_id: &str) -> Result<Option<FlakinessRecord>> {
        if let Some(bytes) = self.flakiness.get(node_id)? {
            let mut deserializer = Deserializer::new(&bytes[..]);
            let record: FlakinessRecord = Deserialize::deserialize(&mut deserializer)?;
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    /// Get all flakiness records.
    pub fn get_all_flakiness(&self) -> Result<Vec<FlakinessRecord>> {
        let mut records = Vec::new();
        for item in self.flakiness.iter() {
            let (_, bytes) = item?;
            let mut deserializer = Deserializer::new(&bytes[..]);
            let record: FlakinessRecord = Deserialize::deserialize(&mut deserializer)?;
            records.push(record);
        }
        Ok(records)
    }

    /// Delete flakiness record.
    pub fn delete_flakiness_record(&self, node_id: &str) -> Result<()> {
        self.flakiness.remove(node_id)?;
        Ok(())
    }

    // ==================== Fixtures ====================

    /// Save fixture state.
    pub fn save_fixture(&self, context_id: &str, fixture: &FixtureState) -> Result<()> {
        let key = format!("{}/{}", context_id, fixture.name);
        let mut buf = Vec::new();
        fixture.serialize(&mut Serializer::new(&mut buf))?;
        self.fixtures.insert(key, buf)?;
        Ok(())
    }

    /// Load fixture state.
    pub fn load_fixture(&self, context_id: &str, name: &str) -> Result<Option<FixtureState>> {
        let key = format!("{}/{}", context_id, name);
        if let Some(bytes) = self.fixtures.get(key)? {
            let mut deserializer = Deserializer::new(&bytes[..]);
            let fixture: FixtureState = Deserialize::deserialize(&mut deserializer)?;
            Ok(Some(fixture))
        } else {
            Ok(None)
        }
    }

    /// Get all fixtures for a context.
    pub fn get_context_fixtures(&self, context_id: &str) -> Result<Vec<FixtureState>> {
        let prefix = format!("{}/", context_id);
        let mut fixtures = Vec::new();
        for item in self.fixtures.scan_prefix(prefix) {
            let (_, bytes) = item?;
            let mut deserializer = Deserializer::new(&bytes[..]);
            let fixture: FixtureState = Deserialize::deserialize(&mut deserializer)?;
            fixtures.push(fixture);
        }
        Ok(fixtures)
    }

    /// Delete fixture.
    pub fn delete_fixture(&self, context_id: &str, name: &str) -> Result<()> {
        let key = format!("{}/{}", context_id, name);
        self.fixtures.remove(key)?;
        Ok(())
    }

    /// Clear all fixtures for a context.
    pub fn clear_context_fixtures(&self, context_id: &str) -> Result<()> {
        let prefix = format!("{}/", context_id);
        for item in self.fixtures.scan_prefix(prefix) {
            let (key, _) = item?;
            self.fixtures.remove(key)?;
        }
        Ok(())
    }

    // ==================== Duration History ====================

    /// Save duration history for a test.
    pub fn save_duration_history(&self, node_id: &str, durations: &[u64]) -> Result<()> {
        let mut buf = Vec::new();
        durations.serialize(&mut Serializer::new(&mut buf))?;
        self.duration_history.insert(node_id, buf)?;
        Ok(())
    }

    /// Load duration history for a test.
    pub fn load_duration_history(&self, node_id: &str) -> Result<Option<Vec<u64>>> {
        if let Some(bytes) = self.duration_history.get(node_id)? {
            let mut deserializer = Deserializer::new(&bytes[..]);
            let durations: Vec<u64> = Deserialize::deserialize(&mut deserializer)?;
            Ok(Some(durations))
        } else {
            Ok(None)
        }
    }

    /// Get average duration for a test.
    pub fn get_average_duration(&self, node_id: &str) -> Result<Option<u64>> {
        if let Some(durations) = self.load_duration_history(node_id)? {
            if durations.is_empty() {
                return Ok(None);
            }
            let sum: u64 = durations.iter().sum();
            Ok(Some(sum / durations.len() as u64))
        } else {
            Ok(None)
        }
    }

    // ==================== Scheduler ====================

    /// Save scheduled test.
    pub fn save_scheduled_test(&self, test: &ScheduledTest) -> Result<()> {
        let mut buf = Vec::new();
        test.serialize(&mut Serializer::new(&mut buf))?;
        self.scheduler.insert(&test.node_id, buf)?;
        Ok(())
    }

    /// Load all scheduled tests.
    pub fn get_all_scheduled_tests(&self) -> Result<Vec<ScheduledTest>> {
        let mut tests = Vec::new();
        for item in self.scheduler.iter() {
            let (_, bytes) = item?;
            let mut deserializer = Deserializer::new(&bytes[..]);
            let test: ScheduledTest = Deserialize::deserialize(&mut deserializer)?;
            tests.push(test);
        }
        Ok(tests)
    }

    /// Clear scheduler.
    pub fn clear_scheduler(&self) -> Result<()> {
        self.scheduler.clear()?;
        Ok(())
    }

    // ==================== Contexts ====================

    /// Save context metadata (simple string map).
    pub fn save_context(&self, context_id: &str, data: &serde_json::Value) -> Result<()> {
        let bytes = serde_json::to_vec(data)?;
        self.contexts.insert(context_id, bytes)?;
        Ok(())
    }

    /// Load context metadata.
    pub fn load_context(&self, context_id: &str) -> Result<Option<serde_json::Value>> {
        if let Some(bytes) = self.contexts.get(context_id)? {
            let data: serde_json::Value = serde_json::from_slice(&bytes)?;
            Ok(Some(data))
        } else {
            Ok(None)
        }
    }

    /// Get all context IDs.
    pub fn get_all_context_ids(&self) -> Vec<String> {
        self.contexts
            .iter()
            .filter_map(|item| item.ok())
            .map(|(key, _)| String::from_utf8_lossy(&key).to_string())
            .collect()
    }

    /// Delete context.
    pub fn delete_context(&self, context_id: &str) -> Result<()> {
        self.contexts.remove(context_id)?;
        Ok(())
    }

    // ==================== Config ====================

    /// Save config value.
    pub fn save_config(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        let bytes = serde_json::to_vec(value)?;
        self.config.insert(key, bytes)?;
        Ok(())
    }

    /// Load config value.
    pub fn load_config(&self, key: &str) -> Result<Option<serde_json::Value>> {
        if let Some(bytes) = self.config.get(key)? {
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    /// Flush to disk.
    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }
}
