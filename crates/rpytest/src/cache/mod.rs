//! Local cache management for rpytest.
//!
//! Manages the `.rpytest/` directory containing:
//! - Test inventory cache (sled database)
//! - Duration history
//! - Outcome history

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rpytest_core::{Inventory, SledBackend, StorageBackend, TestNode, TestNodeInfo};
use tracing::debug;

/// Name of the cache directory.
const CACHE_DIR: &str = ".rpytest";

/// Name of the sled database within the cache directory.
const DB_NAME: &str = "cache.db";

/// Cache manager for a repository.
pub struct CacheManager {
    /// Path to the repository root.
    repo_root: PathBuf,
    /// Path to the cache directory.
    cache_dir: PathBuf,
    /// Storage backend (lazily initialized).
    storage: Option<SledBackend>,
}

impl CacheManager {
    /// Create a new cache manager for a repository.
    pub fn new(repo_root: impl AsRef<Path>) -> Self {
        let repo_root = repo_root.as_ref().to_path_buf();
        let cache_dir = repo_root.join(CACHE_DIR);

        Self {
            repo_root,
            cache_dir,
            storage: None,
        }
    }

    /// Get the cache directory path.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Ensure the cache directory exists.
    pub fn ensure_cache_dir(&self) -> Result<()> {
        if !self.cache_dir.exists() {
            std::fs::create_dir_all(&self.cache_dir).context("Failed to create cache directory")?;
            debug!("Created cache directory: {}", self.cache_dir.display());
        }
        Ok(())
    }

    /// Open the storage backend.
    fn open_storage(&mut self) -> Result<&SledBackend> {
        if self.storage.is_none() {
            self.ensure_cache_dir()?;
            let db_path = self.cache_dir.join(DB_NAME);
            let storage = SledBackend::open(&db_path).context("Failed to open cache database")?;
            self.storage = Some(storage);
        }
        Ok(self.storage.as_ref().unwrap())
    }

    /// Get the cached inventory for a context.
    pub fn get_inventory(&mut self, context_id: &str) -> Result<Option<Inventory>> {
        let storage = self.open_storage()?;

        match Inventory::load(storage, context_id) {
            Ok(inv) if !inv.is_empty() => {
                debug!("Loaded {} tests from cache for {}", inv.len(), context_id);
                Ok(Some(inv))
            }
            Ok(_) => Ok(None),
            Err(e) => {
                debug!("Cache load failed: {}", e);
                Ok(None)
            }
        }
    }

    /// Get the cached inventory hash.
    pub fn get_inventory_hash(&mut self, context_id: &str) -> Result<Option<String>> {
        let inventory = self.get_inventory(context_id)?;
        Ok(inventory.map(|inv| inv.meta().hash.clone()))
    }

    /// Save inventory to cache.
    pub fn save_inventory(
        &mut self,
        context_id: &str,
        hash: &str,
        collected_at: u64,
        nodes: &[TestNodeInfo],
    ) -> Result<()> {
        let storage = self.open_storage()?;

        let mut inventory = Inventory::new();

        // Convert TestNodeInfo to TestNode
        for info in nodes {
            let mut node = TestNode::new(&info.node_id, &info.file_path);
            node.lineno = info.lineno;
            if let Some(ref class) = info.class_name {
                node.class_name = Some(class.clone());
            }
            for marker in &info.markers {
                node.add_marker(marker);
            }
            node.build_keywords();
            inventory.add(node);
        }

        inventory.update_meta(hash.to_string(), collected_at);
        inventory.save(storage, context_id)?;

        debug!(
            "Saved {} tests to cache for {} (hash: {})",
            nodes.len(),
            context_id,
            hash
        );

        Ok(())
    }

    /// Check if cached inventory is valid (hash matches).
    pub fn is_cache_valid(&mut self, context_id: &str, current_hash: &str) -> Result<bool> {
        match self.get_inventory_hash(context_id)? {
            Some(cached_hash) => {
                let valid = cached_hash == current_hash;
                debug!(
                    "Cache validation: cached={}, current={}, valid={}",
                    cached_hash, current_hash, valid
                );
                Ok(valid)
            }
            None => Ok(false),
        }
    }

    /// Clear the cache for a context.
    pub fn clear_context(&mut self, context_id: &str) -> Result<()> {
        let storage = self.open_storage()?;
        Inventory::clear_storage(storage, context_id)?;
        debug!("Cleared cache for {}", context_id);
        Ok(())
    }

    /// Clear all cached data.
    pub fn clear_all(&mut self) -> Result<()> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir).context("Failed to remove cache directory")?;
            self.storage = None;
            debug!("Cleared all cache data");
        }
        Ok(())
    }

    /// Filter cached inventory by keyword and marker.
    pub fn filter_tests(
        &mut self,
        context_id: &str,
        keyword: Option<&str>,
        marker: Option<&str>,
    ) -> Result<Vec<String>> {
        let inventory = self.get_inventory(context_id)?;

        match inventory {
            Some(inv) => {
                let filtered = inv.filter(keyword, marker);
                let node_ids: Vec<String> =
                    filtered.into_iter().map(|n| n.node_id.clone()).collect();
                debug!(
                    "Filtered {} -> {} tests (keyword={:?}, marker={:?})",
                    inv.len(),
                    node_ids.len(),
                    keyword,
                    marker
                );
                Ok(node_ids)
            }
            None => Ok(vec![]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_nodes() -> Vec<TestNodeInfo> {
        vec![
            TestNodeInfo {
                node_id: "test_math.py::test_add".to_string(),
                file_path: "test_math.py".to_string(),
                lineno: Some(10),
                name: "test_add".to_string(),
                class_name: None,
                markers: vec!["unit".to_string()],
                skip: false,
                xfail: false,
            },
            TestNodeInfo {
                node_id: "test_math.py::test_subtract".to_string(),
                file_path: "test_math.py".to_string(),
                lineno: Some(20),
                name: "test_subtract".to_string(),
                class_name: None,
                markers: vec!["unit".to_string()],
                skip: false,
                xfail: false,
            },
            TestNodeInfo {
                node_id: "test_api.py::test_login".to_string(),
                file_path: "test_api.py".to_string(),
                lineno: Some(5),
                name: "test_login".to_string(),
                class_name: None,
                markers: vec!["integration".to_string(), "slow".to_string()],
                skip: false,
                xfail: false,
            },
        ]
    }

    #[test]
    fn test_cache_save_load() {
        let tmp = TempDir::new().unwrap();
        let mut cache = CacheManager::new(tmp.path());

        let nodes = create_test_nodes();
        cache
            .save_inventory("ctx-1", "hash123", 1234567890, &nodes)
            .unwrap();

        let loaded = cache.get_inventory("ctx-1").unwrap().unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded.meta().hash, "hash123");
    }

    #[test]
    fn test_cache_validation() {
        let tmp = TempDir::new().unwrap();
        let mut cache = CacheManager::new(tmp.path());

        let nodes = create_test_nodes();
        cache
            .save_inventory("ctx-1", "hash123", 1234567890, &nodes)
            .unwrap();

        assert!(cache.is_cache_valid("ctx-1", "hash123").unwrap());
        assert!(!cache.is_cache_valid("ctx-1", "different").unwrap());
        assert!(!cache.is_cache_valid("ctx-2", "hash123").unwrap());
    }

    #[test]
    fn test_filter_tests() {
        let tmp = TempDir::new().unwrap();
        let mut cache = CacheManager::new(tmp.path());

        let nodes = create_test_nodes();
        cache
            .save_inventory("ctx-1", "hash123", 1234567890, &nodes)
            .unwrap();

        // Filter by keyword
        let results = cache.filter_tests("ctx-1", Some("math"), None).unwrap();
        assert_eq!(results.len(), 2);

        // Filter by marker
        let results = cache
            .filter_tests("ctx-1", None, Some("integration"))
            .unwrap();
        assert_eq!(results.len(), 1);

        // Combined filter
        let results = cache
            .filter_tests("ctx-1", Some("test"), Some("unit"))
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_clear_context() {
        let tmp = TempDir::new().unwrap();
        let mut cache = CacheManager::new(tmp.path());

        let nodes = create_test_nodes();
        cache
            .save_inventory("ctx-1", "hash123", 1234567890, &nodes)
            .unwrap();

        assert!(cache.get_inventory("ctx-1").unwrap().is_some());

        cache.clear_context("ctx-1").unwrap();

        assert!(cache.get_inventory("ctx-1").unwrap().is_none());
    }
}
