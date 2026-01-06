//! Repository context management - handles inventory, collection, and state.

use crate::collector::NativeCollector;
use crate::error::{Result, DaemonError};
use crate::executor::{ExecutorConfig, PythonExecutor};
use crate::fixtures::FixtureManager;
use crate::flakiness::FlakinessTracker;
use crate::models::{
    RunSummary, RerunConfig, TestNode, TestOutcome, TestResult,
    ShardConfig,
};
use crate::scheduler::TestScheduler;
use crate::storage::DaemonStorage;
use crate::server::StreamingRun;
use rpytest_core::protocol::TestNodeInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use uuid::Uuid;
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

/// Represents a single test node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestNodeInternal {
    pub node_id: String,
    pub file_path: String,
    pub name: String,
    pub class_name: Option<String>,
    pub line_number: u32,
    pub markers: Vec<String>,
    pub skip: bool,
    pub xfail: bool,
}

/// Represents a repository execution context.
#[derive(Debug, Clone)]
pub struct RepoContext {
    /// Unique context ID
    pub context_id: String,
    /// Repository root path
    pub repo_path: PathBuf,
    /// Python interpreter path
    pub python_path: PathBuf,
    /// Test inventory (node_id -> TestNode)
    inventory: Arc<Mutex<HashMap<String, TestNode>>>,
    /// Inventory hash for cache validation
    pub inventory_hash: String,
    /// Duration history (node_id -> list of durations)
    duration_history: Arc<Mutex<HashMap<String, Vec<u64>>>>,
    /// Outcome history (node_id -> list of outcome strings)
    outcome_history: Arc<Mutex<HashMap<String, Vec<String>>>>,
    /// Scheduler for test ordering
    scheduler: Arc<Mutex<TestScheduler>>,
    /// Test executor
    executor: PythonExecutor,
    /// Native test collector
    native_collector: NativeCollector,
    /// Flakiness tracker
    flakiness_tracker: Arc<Mutex<FlakinessTracker>>,
    /// Fixture manager
    fixture_manager: Arc<Mutex<FixtureManager>>,
    /// Re-run configuration
    rerun_config: RerunConfig,
    /// Storage backend
    storage: Option<DaemonStorage>,
    /// Use native collection
    use_native: bool,
    /// Streaming runs
    streaming_runs: Arc<Mutex<HashMap<String, StreamingRun>>>,
    /// Collection time
    pub last_collection_time: f64,
    /// Total runs
    total_runs: u32,
}

impl RepoContext {
    /// Create a new context.
    pub fn new(
        context_id: &str,
        repo_path: &Path,
        python_path: Option<PathBuf>,
        storage: Option<DaemonStorage>,
    ) -> Self {
        let python_path = python_path.unwrap_or_else(|| {
            PathBuf::from(std::env::var("PYTHON_PATH").unwrap_or_else(|_| "python".to_string()))
        });

        let storage_path = repo_path.join(".rpytest");

        let flakiness_tracker = FlakinessTracker::new(Some(storage_path.join("flakiness.json")));

        RepoContext {
            context_id: context_id.to_string(),
            repo_path: repo_path.to_path_buf(),
            python_path: python_path.clone(),
            inventory: Arc::new(Mutex::new(HashMap::new())),
            inventory_hash: String::new(),
            duration_history: Arc::new(Mutex::new(HashMap::new())),
            outcome_history: Arc::new(Mutex::new(HashMap::new())),
            scheduler: Arc::new(Mutex::new(TestScheduler::new())),
            executor: PythonExecutor::new(python_path),
            native_collector: NativeCollector::new(repo_path),
            flakiness_tracker: Arc::new(Mutex::new(flakiness_tracker)),
            fixture_manager: Arc::new(Mutex::new(FixtureManager::new())),
            rerun_config: RerunConfig::default(),
            storage,
            use_native: true,
            streaming_runs: Arc::new(Mutex::new(HashMap::new())),
            last_collection_time: 0.0,
            total_runs: 0,
        }
    }

    /// Collect tests using cached inventory or in-process collection.
    pub fn collect(&mut self, force: bool) -> Result<(usize, u64)> {
        let start_time = SystemTime::now();
        let start_secs = start_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        // Check if we can use cached inventory
        if !force {
            if let Some(ref storage) = self.storage {
                let cached_inventory = storage.get_all_inventory()?;
                if !cached_inventory.is_empty() {
                    let mut inventory = self.inventory.lock().unwrap();
                    for node in cached_inventory {
                        inventory.insert(node.node_id.clone(), node);
                    }
                    self.inventory_hash = self.compute_hash();
                    self.last_collection_time = start_secs;
                    let duration_ms = start_time
                        .elapsed()
                        .unwrap_or(Duration::ZERO)
                        .as_millis() as u64;
                    return Ok((inventory.len(), duration_ms));
                }
            }
        }

        // Collect using native collector
        if self.use_native {
            let native_tests = self.native_collector.collect()?;

            let mut inventory = self.inventory.lock().unwrap();
            for test in native_tests {
                inventory.insert(
                    test.node_id.clone(),
                    TestNode {
                        node_id: test.node_id,
                        file_path: test.file_path,
                        name: test.name,
                        class_name: test.class_name,
                        line_number: test.line_number,
                        markers: test.markers,
                        skip: false,
                        xfail: false,
                    },
                );
            }

            // Save to storage
            if let Some(ref storage) = self.storage {
                storage.clear_inventory()?;
                for node in inventory.values().cloned().collect::<Vec<_>>() {
                    storage.save_test_node(&node)?;
                }
            }
        } else {
            // Fall back to pytest collection (not implemented in pure Rust yet)
            warn!("Pytest collection not yet implemented in pure Rust daemon");
        }

        self.inventory_hash = self.compute_hash();
        self.last_collection_time = start_secs;

        let duration_ms = start_time
            .elapsed()
            .unwrap_or(Duration::ZERO)
            .as_millis() as u64;

        info!(
            "Collected {} tests in {}ms",
            self.inventory.lock().unwrap().len(),
            duration_ms
        );

        Ok((self.inventory.lock().unwrap().len(), duration_ms))
    }

    /// Get all test node IDs.
    pub fn get_node_ids(&self) -> Vec<String> {
        self.inventory
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    /// Get all test nodes.
    pub fn get_inventory(&self) -> Vec<TestNode> {
        self.inventory.lock().unwrap().values().cloned().collect()
    }

    /// Get test node by ID.
    pub fn get_test_node(&self, node_id: &str) -> Option<TestNode> {
        self.inventory.lock().unwrap().get(node_id).cloned()
    }

    /// Filter tests by keyword.
    pub fn filter_by_keyword(&self, keyword: &str) -> Vec<TestNode> {
        if keyword.is_empty() {
            return self.get_inventory();
        }

        self.inventory
            .lock()
            .unwrap()
            .values()
            .filter(|node| {
                node.node_id.contains(keyword)
                    || node.name.contains(keyword)
                    || node
                        .markers
                        .iter()
                        .any(|m| m.contains(keyword))
            })
            .cloned()
            .collect()
    }

    /// Filter tests by marker.
    pub fn filter_by_marker(&self, marker: &str) -> Vec<TestNode> {
        if marker.is_empty() {
            return self.get_inventory();
        }

        self.inventory
            .lock()
            .unwrap()
            .values()
            .filter(|node| node.markers.iter().any(|m| m.contains(marker)))
            .cloned()
            .collect()
    }

    /// Run tests and return results.
    pub async fn run_tests(
        &mut self,
        node_ids: &[String],
        workers: Option<u32>,
        maxfail: Option<u32>,
    ) -> Result<RunSummary> {
        self.total_runs += 1;

        // Configure executor
        let mut config = ExecutorConfig::new();
        config.workers = workers;
        config.maxfail = maxfail;
        self.executor.configure(config);

        // Update scheduler with latest durations
        {
            let durations: Vec<(String, u64)> = {
                let history = self.duration_history.lock().unwrap();
                history
                    .iter()
                    .filter_map(|(node_id, durations)| {
                        durations.last().map(|d| (node_id.clone(), *d))
                    })
                    .collect()
            };

            let mut scheduler = self.scheduler.lock().unwrap();
            for (node_id, duration) in durations {
                scheduler.update_duration(&node_id, duration);
            }
        }

        // Run tests
        let results = self.executor.run_tests(node_ids).await;

        // Process results
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        let mut errors = 0;
        let start_time = SystemTime::now();

        for result in &results {
            // Update duration history
            {
                let mut durations = self.duration_history.lock().unwrap();
                let entry = durations.entry(result.node_id.clone()).or_default();
                entry.push(result.duration_ms);
                if entry.len() > 10 {
                    *entry = entry[entry.len() - 10..].to_vec();
                }
            }

            // Update outcome history
            {
                let mut outcomes = self.outcome_history.lock().unwrap();
                let entry = outcomes.entry(result.node_id.clone()).or_default();
                entry.push(result.outcome.clone().into());
            }

            // Update flakiness tracker
            {
                let mut tracker = self.flakiness_tracker.lock().unwrap();
                tracker.record_outcome(
                    &result.node_id,
                    result.outcome.clone(),
                    result.message.as_deref(),
                );
            }

            // Update scheduler
            {
                let mut scheduler = self.scheduler.lock().unwrap();
                scheduler.update_duration(&result.node_id, result.duration_ms);
            }

            // Count outcomes
            match result.outcome {
                TestOutcome::Passed => passed += 1,
                TestOutcome::Failed => failed += 1,
                TestOutcome::Skipped => skipped += 1,
                TestOutcome::Error => errors += 1,
                _ => {}
            }
        }

        // Save state
        self.save_state()?;

        let duration_ms = start_time
            .elapsed()
            .unwrap_or(Duration::ZERO)
            .as_millis() as u64;

        Ok(RunSummary {
            total: results.len(),
            passed,
            failed,
            skipped,
            errors,
            duration_ms,
        })
    }

    /// Save context state to storage.
    fn save_state(&self) -> Result<()> {
        if let Some(ref storage) = self.storage {
            // Save flakiness data
            let tracker = self.flakiness_tracker.lock().unwrap();
            tracker.save()?;

            // Save duration history
            let durations = self.duration_history.lock().unwrap();
            for (node_id, duration_list) in durations.iter() {
                storage.save_duration_history(node_id, duration_list)?;
            }
        }
        Ok(())
    }

    /// Compute inventory hash.
    fn compute_hash(&self) -> String {
        let inventory = self.inventory.lock().unwrap();
        let mut ids: Vec<&String> = inventory.keys().collect();
        ids.sort();

        let mut hasher = Sha256::default();
        for id in ids {
            hasher.update(id.as_bytes());
        }

        hex::encode(hasher.finalize())
    }

    /// Get scheduler status.
    pub fn get_scheduler_status(&self) -> serde_json::Value {
        let scheduler = self.scheduler.lock().unwrap();
        serde_json::json!({
            "tracked_tests": scheduler.tracked_count(),
            "default_duration_ms": scheduler.default_duration_ms,
        })
    }

    /// Get flakiness report.
    pub fn get_flakiness_report(&self) -> serde_json::Value {
        let tracker = self.flakiness_tracker.lock().unwrap();
        let flaky = tracker.get_flaky_tests();
        let unstable = tracker.get_unstable_tests();

        serde_json::json!({
            "flaky_tests": flaky.iter().map(|r| self.serialize_flakiness_record(r)).collect::<Vec<_>>(),
            "unstable_tests": unstable.iter().map(|r| self.serialize_flakiness_record(r)).collect::<Vec<_>>(),
            "stable_count": tracker.stable_count(),
            "total_tracked": tracker.total_tracked(),
        })
    }

    fn serialize_flakiness_record(&self, record: &crate::models::FlakinessRecord) -> serde_json::Value {
        serde_json::json!({
            "node_id": record.node_id,
            "failure_rate": record.outcomes.iter().filter(|o| *o == "failed" || *o == "error").count() as f64 / record.outcomes.len() as f64,
            "is_flaky": record.flaky_streak >= 2 && record.outcomes.iter().any(|o| *o == "passed"),
            "flaky_streak": record.flaky_streak,
            "consecutive_failures": record.consecutive_failures,
            "consecutive_passes": record.consecutive_passes,
            "total_runs": record.total_runs,
            "recent_outcomes": record.outcomes.clone(),
        })
    }
}
