//! Repository context management - handles inventory, collection, and state.

use crate::collector::NativeCollector;
use crate::error::Result;
use crate::executor::{create_executor, create_pooled_executor, ExecutorConfig, TestExecutor};
use crate::fixtures::FixtureManager;
use crate::flakiness::FlakinessTracker;
use crate::models::{ExecutionMode, RerunConfig, RunSummary, TestNode, TestOutcome};
use crate::scheduler::TestScheduler;
use crate::storage::DaemonStorage;
use parking_lot::Mutex as PLMutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

/// Find a Python interpreter that has pytest installed.
fn find_python_with_pytest(repo_path: &Path) -> PathBuf {
    // 1. Check VIRTUAL_ENV environment variable
    if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
        let venv_python = PathBuf::from(&venv).join("bin").join("python");
        if venv_python.exists() {
            return venv_python;
        }
    }

    // 2. Check for local .venv directory in repo
    let local_venv = repo_path.join(".venv").join("bin").join("python");
    if local_venv.exists() {
        return local_venv;
    }

    // 3. Check for venv directory in repo
    let venv_dir = repo_path.join("venv").join("bin").join("python");
    if venv_dir.exists() {
        return venv_dir;
    }

    // 4. Check PYTHON_PATH env var
    if let Ok(python_path) = std::env::var("PYTHON_PATH") {
        return PathBuf::from(python_path);
    }

    // 5. Fall back to python3
    PathBuf::from("python3")
}

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
#[derive(Debug)]
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
    /// Test executor (supports both embedded and subprocess modes)
    executor: Arc<PLMutex<Box<dyn TestExecutor>>>,
    /// Execution mode being used
    pub execution_mode: ExecutionMode,
    /// Native test collector
    native_collector: NativeCollector,
    /// Flakiness tracker
    flakiness_tracker: Arc<Mutex<FlakinessTracker>>,
    /// Fixture manager (planned feature)
    #[allow(dead_code)]
    fixture_manager: Arc<Mutex<FixtureManager>>,
    /// Re-run configuration (planned feature)
    #[allow(dead_code)]
    rerun_config: RerunConfig,
    /// Storage backend
    storage: Option<DaemonStorage>,
    /// Use native collection
    use_native: bool,
    /// Collection time
    pub last_collection_time: f64,
    /// Total runs
    total_runs: u32,
}

impl RepoContext {
    /// Create a new context.
    ///
    /// # Arguments
    /// * `context_id` - Unique identifier for this context
    /// * `repo_path` - Path to the repository root
    /// * `python_path` - Optional path to Python interpreter (auto-detected if None)
    /// * `storage` - Optional storage backend for persistence
    /// * `execution_mode` - Execution mode (Embedded, Subprocess, Pooled, or Auto)
    pub async fn new(
        context_id: &str,
        repo_path: &Path,
        python_path: Option<PathBuf>,
        storage: Option<DaemonStorage>,
        execution_mode: ExecutionMode,
    ) -> Result<Self> {
        let python_path = python_path.unwrap_or_else(|| find_python_with_pytest(repo_path));

        let storage_path = repo_path.join(".rpytest");

        let flakiness_tracker = FlakinessTracker::new(Some(storage_path.join("flakiness.json")));

        // Check RPYTEST_NO_POOL env var to disable pooled mode
        let no_pool = std::env::var("RPYTEST_NO_POOL").map_or(false, |v| v == "1" || v == "true");

        // Create executor based on execution mode
        // Auto mode uses subprocess (safe isolation). Pooled is opt-in only via --execution-mode pooled.
        let effective_mode = if no_pool && matches!(execution_mode, ExecutionMode::Pooled) {
            info!("RPYTEST_NO_POOL is set, downgrading pooled to subprocess");
            ExecutionMode::Subprocess
        } else {
            execution_mode
        };

        let (executor, actual_mode): (Box<dyn TestExecutor>, ExecutionMode) = match effective_mode {
            ExecutionMode::Pooled => {
                // Pooled mode: create async worker pool with repo_path as working directory
                let worker_count = num_cpus::get();
                info!("Creating pooled executor with {} workers in {}", worker_count, repo_path.display());
                let executor = create_pooled_executor(python_path.clone(), Some(worker_count), repo_path.to_path_buf()).await?;
                (executor, ExecutionMode::Pooled)
            }
            ExecutionMode::Auto => {
                // Auto mode: use subprocess for reliable test isolation
                // Pooled mode is available via --execution-mode pooled for projects that support it
                info!("Auto mode: using subprocess executor (use --execution-mode pooled for warm workers)");
                let executor = create_executor(ExecutionMode::Subprocess, python_path.clone())?;
                (executor, ExecutionMode::Subprocess)
            }
            other => {
                // Other modes: use sync creation
                let executor = create_executor(other, python_path.clone())?;
                let mode = match executor.execution_mode() {
                    "embedded" => ExecutionMode::Embedded,
                    "pooled" => ExecutionMode::Pooled,
                    _ => ExecutionMode::Subprocess,
                };
                (executor, mode)
            }
        };

        info!(
            "Created context {} with {} executor",
            context_id,
            executor.execution_mode(),
        );

        Ok(RepoContext {
            context_id: context_id.to_string(),
            repo_path: repo_path.to_path_buf(),
            python_path,
            inventory: Arc::new(Mutex::new(HashMap::new())),
            inventory_hash: String::new(),
            duration_history: Arc::new(Mutex::new(HashMap::new())),
            outcome_history: Arc::new(Mutex::new(HashMap::new())),
            scheduler: Arc::new(Mutex::new(TestScheduler::new())),
            executor: Arc::new(PLMutex::new(executor)),
            execution_mode: actual_mode,
            native_collector: NativeCollector::new(repo_path),
            flakiness_tracker: Arc::new(Mutex::new(flakiness_tracker)),
            fixture_manager: Arc::new(Mutex::new(FixtureManager::new())),
            rerun_config: RerunConfig::default(),
            storage,
            use_native: true,
            last_collection_time: 0.0,
            total_runs: 0,
        })
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
                    let duration_ms =
                        start_time.elapsed().unwrap_or(Duration::ZERO).as_millis() as u64;
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
                        skip: test.skip,
                        xfail: test.xfail,
                    },
                );
            }

            // Save to storage using batch operation (much faster than individual saves)
            if let Some(ref storage) = self.storage {
                storage.clear_inventory()?;
                let nodes: Vec<TestNode> = inventory.values().cloned().collect();
                storage.save_test_nodes_batch(&nodes)?;
            }
        } else {
            // Fall back to pytest collection (not implemented in pure Rust yet)
            warn!("Pytest collection not yet implemented in pure Rust daemon");
        }

        self.inventory_hash = self.compute_hash();
        self.last_collection_time = start_secs;

        let duration_ms = start_time.elapsed().unwrap_or(Duration::ZERO).as_millis() as u64;

        info!(
            "Collected {} tests in {}ms",
            self.inventory.lock().unwrap().len(),
            duration_ms
        );

        Ok((self.inventory.lock().unwrap().len(), duration_ms))
    }

    /// Get all test node IDs.
    pub fn get_node_ids(&self) -> Vec<String> {
        self.inventory.lock().unwrap().keys().cloned().collect()
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
                    || node.markers.iter().any(|m| m.contains(keyword))
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
        {
            let mut executor = self.executor.lock();
            executor.configure(config);
        }

        // Separate pre-skipped tests from runnable tests
        // Tests with skip=true from collection markers should be counted as skipped without running
        let (runnable_node_ids, pre_skipped_count): (Vec<String>, usize) = {
            let inventory = self.inventory.lock().unwrap();
            let mut runnable = Vec::with_capacity(node_ids.len());
            let mut skipped_count = 0;

            for node_id in node_ids {
                if let Some(node) = inventory.get(node_id) {
                    if node.skip {
                        skipped_count += 1;
                    } else {
                        runnable.push(node_id.clone());
                    }
                } else {
                    // Node not in inventory - run it anyway (might be a new test)
                    runnable.push(node_id.clone());
                }
            }
            (runnable, skipped_count)
        };

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

        // Run tests (excluding pre-skipped ones)
        // Clone the executor Arc to avoid holding the lock across await
        let executor = self.executor.clone();
        let start_time = SystemTime::now(); // Start timing BEFORE test execution
        let results = {
            let executor = executor.lock();
            executor.run_tests(&runnable_node_ids).await
        };

        // Process results
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        let mut errors = 0;

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
                TestOutcome::Xfail => {
                    // Expected failure that failed - don't count as passed or failed
                    // These are "successful failures"
                }
                TestOutcome::Xpass => {
                    // Expected failure that passed - don't count as passed to match pytest behavior
                    // pytest counts xpassed separately, not as "passed"
                }
            }
        }

        // Add pre-skipped tests (those with skip marker from collection) to skipped count
        skipped += pre_skipped_count;

        // Save state
        self.save_state()?;

        let duration_ms = start_time.elapsed().unwrap_or(Duration::ZERO).as_millis() as u64;

        Ok(RunSummary {
            total: results.len() + pre_skipped_count,
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
            // Save flakiness data (uses buffered writes)
            let mut tracker = self.flakiness_tracker.lock().unwrap();
            tracker.flush_if_dirty()?;

            // Save duration history in a single batch operation
            let durations = self.duration_history.lock().unwrap();
            let histories: Vec<(&str, &[u64])> = durations
                .iter()
                .map(|(id, d)| (id.as_str(), d.as_slice()))
                .collect();
            storage.save_duration_history_batch(&histories)?;
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

    fn serialize_flakiness_record(
        &self,
        record: &crate::models::FlakinessRecord,
    ) -> serde_json::Value {
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
