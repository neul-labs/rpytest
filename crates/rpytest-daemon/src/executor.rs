//! Python subprocess executor for running pytest tests.
//!
//! This module provides two execution strategies:
//! - `PythonExecutor`: Subprocess-based execution (default, always available)
//! - `EmbeddedExecutor`: PyO3-based embedded execution (requires `embedded-python` feature)
//!
//! Use `create_executor()` to automatically select the best available executor.

use crate::error::{DaemonError, Result};
use crate::models::{ExecutionMode, TestOutcome, TestResult};
use crate::pool::WorkerPool;
use crate::scheduler::TestScheduler;
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::process::Command as AsyncCommand;
use tracing::{debug, error, info, warn};

/// Trait for test executors providing a unified interface for running tests.
#[async_trait]
pub trait TestExecutor: Send + Sync + std::fmt::Debug {
    /// Run a single test and return the result.
    async fn run_test(&self, node_id: &str) -> Result<TestResult>;

    /// Run multiple tests and return all results.
    async fn run_tests(&self, node_ids: &[String]) -> Vec<TestResult>;

    /// Configure the executor with the given settings.
    fn configure(&mut self, config: ExecutorConfig);

    /// Get the execution mode name for logging.
    fn execution_mode(&self) -> &'static str;

    /// Kill all running processes.
    fn kill_all(&self);
}

/// Create the best available executor based on the requested mode.
///
/// - `Embedded`: Use PyO3 embedded Python (fails if feature not enabled or Python unavailable)
/// - `Subprocess`: Always use subprocess execution
/// - `Pooled`: Use worker pool (requires async - use `create_pooled_executor` instead)
/// - `Auto`: Try embedded first, fall back to subprocess
pub fn create_executor(mode: ExecutionMode, python_path: PathBuf) -> Result<Box<dyn TestExecutor>> {
    match mode {
        ExecutionMode::Embedded => {
            #[cfg(feature = "embedded-python")]
            {
                // Pass python_path to EmbeddedExecutor for virtualenv support
                match crate::embedded::EmbeddedExecutor::new(Some(python_path)) {
                    Ok(executor) => {
                        info!("Using embedded Python executor");
                        Ok(Box::new(executor))
                    }
                    Err(e) => Err(crate::error::DaemonError::Other(format!(
                        "Failed to create embedded executor: {}",
                        e
                    ))),
                }
            }
            #[cfg(not(feature = "embedded-python"))]
            {
                let _ = python_path; // Silence unused variable warning
                Err(crate::error::DaemonError::Other(
                    "Embedded Python execution requires the 'embedded-python' feature".to_string(),
                ))
            }
        }
        ExecutionMode::Subprocess => {
            info!("Using subprocess executor");
            Ok(Box::new(PythonExecutor::new(python_path)))
        }
        ExecutionMode::Pooled => {
            // Pooled mode requires async initialization - use create_pooled_executor instead
            Err(crate::error::DaemonError::Other(
                "Pooled executor requires async initialization. Use create_pooled_executor() instead.".to_string(),
            ))
        }
        ExecutionMode::Auto => {
            #[cfg(feature = "embedded-python")]
            {
                if crate::embedded::EmbeddedExecutor::is_available() {
                    // Pass python_path to EmbeddedExecutor for virtualenv support
                    match crate::embedded::EmbeddedExecutor::new(Some(python_path.clone())) {
                        Ok(executor) => {
                            info!("Auto-selected embedded Python executor");
                            return Ok(Box::new(executor));
                        }
                        Err(e) => {
                            info!(
                                "Embedded executor unavailable ({}), falling back to subprocess",
                                e
                            );
                        }
                    }
                }
            }
            info!("Using subprocess executor (auto-selected)");
            Ok(Box::new(PythonExecutor::new(python_path)))
        }
    }
}

/// Create a pooled executor with warm workers.
///
/// This is async because it needs to spawn and wait for worker processes.
pub async fn create_pooled_executor(
    python_path: PathBuf,
    worker_count: Option<usize>,
    working_dir: PathBuf,
) -> Result<Box<dyn TestExecutor>> {
    let workers = worker_count.unwrap_or_else(num_cpus::get);
    info!(
        "Creating pooled executor with {} workers in {}",
        workers,
        working_dir.display()
    );

    let executor = PooledExecutor::new(python_path, workers, working_dir).await?;
    Ok(Box::new(executor))
}

/// Executor configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutorConfig {
    /// Number of parallel workers
    pub workers: Option<u32>,
    /// Stop after N failures
    pub maxfail: Option<u32>,
    /// Batch size for grouping tests
    pub batch_size: usize,
    /// Timeout per test in seconds
    pub test_timeout_secs: u64,
    /// Extra pytest arguments
    pub extra_args: Vec<String>,
}

impl ExecutorConfig {
    /// Create default config with adaptive batch sizing.
    pub fn new() -> Self {
        ExecutorConfig {
            workers: None,
            maxfail: None,
            batch_size: Self::optimal_batch_size(),
            test_timeout_secs: 60,
            extra_args: Vec::new(),
        }
    }

    /// Calculate optimal batch size based on CPU count.
    /// Larger batches reduce process spawn overhead but too large = poor parallelism.
    pub fn optimal_batch_size() -> usize {
        let cpus = num_cpus::get();
        // Scale batch size with CPU count, minimum 500 for fewer subprocess spawns
        // Each subprocess has ~200-300ms startup overhead
        (cpus * 50).max(500)
    }
}

/// Executor that runs tests via Python subprocess.
#[derive(Debug, Clone)]
pub struct PythonExecutor {
    /// Python interpreter path
    python_path: PathBuf,
    /// Executor configuration
    config: ExecutorConfig,
    /// Running processes
    processes: Arc<Mutex<HashMap<String, tokio::process::Child>>>,
}

struct PytestOutput {
    text: String,
    failed: bool,
}

impl Default for PythonExecutor {
    fn default() -> Self {
        PythonExecutor {
            python_path: PathBuf::from("python"),
            config: ExecutorConfig::new(),
            processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl PythonExecutor {
    /// Create a new executor.
    pub fn new(python_path: PathBuf) -> Self {
        PythonExecutor {
            python_path,
            config: ExecutorConfig::new(),
            processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Configure the executor.
    pub fn configure(&mut self, config: ExecutorConfig) {
        self.config = config;
    }

    /// Get Python path.
    pub fn python_path(&self) -> &PathBuf {
        &self.python_path
    }

    /// Run a single test.
    pub async fn run_test(&self, node_id: &str) -> Result<TestResult> {
        let node_ids = [node_id.to_owned()];
        let output = self.run_pytest(&node_ids).await?;
        let mut result = self.parse_batch_output(&node_ids, &output).remove(0);
        result.duration_ms = Self::extract_duration(&output.text).unwrap_or(0);
        if result.message.is_none() {
            result.message = Self::extract_message(&output.text);
        }
        Ok(result)
    }

    /// Run multiple tests in batch.
    pub async fn run_tests(&self, node_ids: &[String]) -> Vec<TestResult> {
        if node_ids.is_empty() {
            return Vec::new();
        }

        // Split into batches
        let batches: Vec<Vec<String>> = node_ids
            .chunks(self.config.batch_size)
            .map(|c| c.to_vec())
            .collect();

        // Run batches concurrently if workers > 1
        let workers = self.config.workers.unwrap_or(1) as usize;

        if workers > 1 && batches.len() > 1 {
            // Parallel execution using tokio's native concurrency
            // Limit concurrent batches to worker count using semaphore
            let semaphore = Arc::new(tokio::sync::Semaphore::new(workers));
            let mut handles = Vec::with_capacity(batches.len());

            for batch in batches {
                let permit = semaphore.clone().acquire_owned().await;
                let executor = self.clone();

                let handle = tokio::spawn(async move {
                    let _permit = permit; // Hold permit until batch completes
                    executor.run_batch(&batch).await
                });
                handles.push(handle);
            }

            // Collect results from all handles
            let mut all_results = Vec::with_capacity(node_ids.len());
            for handle in handles {
                match handle.await {
                    Ok(batch_results) => all_results.extend(batch_results),
                    Err(e) => {
                        error!("Batch execution failed: {}", e);
                        // Continue with other batches
                    }
                }
            }
            all_results
        } else {
            // Sequential execution
            let mut results = Vec::with_capacity(node_ids.len());
            for batch in batches {
                let batch_results = self.run_batch(&batch).await;
                results.extend(batch_results);
            }
            results
        }
    }

    /// Run a batch of tests.
    async fn run_batch(&self, node_ids: &[String]) -> Vec<TestResult> {
        match self.run_pytest(node_ids).await {
            Ok(output) => self.parse_batch_output(node_ids, &output),
            Err(error) => {
                let message = error.to_string();
                node_ids
                    .iter()
                    .map(|node_id| TestResult {
                        node_id: node_id.clone(),
                        outcome: TestOutcome::Error,
                        duration_ms: 0,
                        message: Some(message.clone()),
                        stdout: None,
                        stderr: None,
                    })
                    .collect()
            }
        }
    }

    /// Run pytest with the given arguments.
    async fn run_pytest(&self, node_ids: &[String]) -> Result<PytestOutput> {
        // Allow tests to bypass spawning a real Python interpreter.
        if std::env::var("RPYTEST_FAKE_PYTEST").is_ok() {
            let mut output = String::new();
            for node_id in node_ids {
                output.push_str(&format!("{} PASSED\n", node_id));
            }
            output.push_str(&format!("{} passed in 0.01s\n", node_ids.len()));
            return Ok(PytestOutput {
                text: output,
                failed: false,
            });
        }

        let mut args: Vec<String> = vec!["-m".to_string(), "pytest".to_string()];

        // Add node IDs
        for node_id in node_ids {
            args.push(node_id.clone());
        }

        // Add configuration
        args.push("--tb=short".to_string());
        args.push("--no-header".to_string());

        // Add maxfail
        if let Some(maxfail) = self.config.maxfail {
            args.push("--maxfail".to_string());
            args.push(maxfail.to_string());
        }

        // Add extra args
        args.extend(self.config.extra_args.clone());

        // Keep each test name and outcome together on a plain terminal line.
        args.push("--verbosity=1".to_string());
        args.push("--override-ini=verbosity_test_cases=1".to_string());
        args.push("--capture=fd".to_string());
        args.push("--color=no".to_string());

        debug!("Running: {} {:?}", self.python_path.display(), args);

        let output = AsyncCommand::new(&self.python_path)
            .args(&args)
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| {
                DaemonError::PythonExecution(format!("could not run pytest: {}", error))
            })?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        let failed = match output.status.code() {
            Some(0) => false,
            Some(1) => true,
            _ => {
                let detail = stderr.lines().find(|line| !line.trim().is_empty());
                let mut message = format!("pytest terminated with {}", output.status);
                if let Some(detail) = detail {
                    message.push_str(": ");
                    message.push_str(detail);
                }
                return Err(DaemonError::PythonExecution(message));
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(PytestOutput {
            text: format!("{}\n{}", stdout, stderr),
            failed,
        })
    }

    /// Parse explicit outcomes for the requested test IDs.
    fn parse_batch_output(&self, node_ids: &[String], output: &PytestOutput) -> Vec<TestResult> {
        let requested: HashSet<&str> = node_ids.iter().map(String::as_str).collect();
        let mut outcomes = HashMap::with_capacity(node_ids.len());
        for line in output.text.lines() {
            let line = line.trim_start();
            for (index, _) in line.match_indices(' ') {
                let node_id = &line[..index];
                if !requested.contains(node_id) {
                    continue;
                }
                let outcome = match line[index..].split_whitespace().next() {
                    Some("PASSED") => TestOutcome::Passed,
                    Some("FAILED") => TestOutcome::Failed,
                    Some("SKIPPED") => TestOutcome::Skipped,
                    Some("ERROR") => TestOutcome::Error,
                    Some("XFAIL") => TestOutcome::Xfail,
                    Some("XPASS") => TestOutcome::Xpass,
                    _ => continue,
                };
                outcomes.insert(node_id, outcome);
                break;
            }
        }

        let mut results: Vec<_> = node_ids
            .iter()
            .map(|node_id| {
                let (outcome, message) = match outcomes.remove(node_id.as_str()) {
                    Some(outcome) => (outcome, None),
                    None => (
                        TestOutcome::Error,
                        Some("pytest did not report an outcome for this test".to_string()),
                    ),
                };
                TestResult {
                    node_id: node_id.clone(),
                    outcome,
                    duration_ms: 0,
                    message,
                    stdout: None,
                    stderr: None,
                }
            })
            .collect();
        if output.failed
            && !results
                .iter()
                .any(|result| matches!(result.outcome, TestOutcome::Failed | TestOutcome::Error))
        {
            if let Some(result) = results.first_mut() {
                result.outcome = TestOutcome::Error;
                result.message =
                    Some("pytest reported a failure without a matching outcome".to_string());
            }
        }
        results
    }

    /// Extract duration from pytest output.
    fn extract_duration(output: &str) -> Option<u64> {
        // Look for patterns like "0.12s" or "12ms"
        for line in output.lines() {
            if line.contains("passed") || line.contains("failed") {
                // Try to extract duration
                if let Some(idx) = line.find('[') {
                    if let Some(end_idx) = line[idx..].find(']') {
                        let duration_str = &line[idx + 1..idx + end_idx];
                        if duration_str.contains("s") {
                            if let Ok(seconds) = duration_str.replace("s", "").parse::<f64>() {
                                return Some((seconds * 1000.0) as u64);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Extract error message from output.
    fn extract_message(output: &str) -> Option<String> {
        // Look for assertion errors or exceptions
        for line in output.lines() {
            if line.contains("AssertionError")
                || line.contains("Error:")
                || line.contains("FAILED:")
            {
                return Some(line.to_string());
            }
        }
        None
    }

    /// Kill all running processes.
    pub fn kill_all(&self) {
        let mut processes = self.processes.lock();
        for (_, child) in processes.iter_mut() {
            drop(child.kill());
        }
        processes.clear();
    }
}

#[async_trait]
impl TestExecutor for PythonExecutor {
    async fn run_test(&self, node_id: &str) -> Result<TestResult> {
        PythonExecutor::run_test(self, node_id).await
    }

    async fn run_tests(&self, node_ids: &[String]) -> Vec<TestResult> {
        PythonExecutor::run_tests(self, node_ids).await
    }

    fn configure(&mut self, config: ExecutorConfig) {
        PythonExecutor::configure(self, config)
    }

    fn execution_mode(&self) -> &'static str {
        "subprocess"
    }

    fn kill_all(&self) {
        PythonExecutor::kill_all(self)
    }
}

/// Executor using a warm worker pool for maximum throughput.
///
/// Pre-spawns Python worker processes that stay alive between test runs,
/// eliminating subprocess spawn overhead (200-300ms per batch).
#[derive(Debug)]
pub struct PooledExecutor {
    /// Worker pool
    pool: Arc<WorkerPool>,
    /// Test scheduler for load balancing
    scheduler: TestScheduler,
    /// Configuration
    config: ExecutorConfig,
}

impl PooledExecutor {
    /// Create a new pooled executor.
    pub async fn new(
        python_path: PathBuf,
        worker_count: usize,
        working_dir: PathBuf,
    ) -> Result<Self> {
        let pool = WorkerPool::new(worker_count, python_path, working_dir)
            .await
            .map_err(|e| {
                crate::error::DaemonError::Other(format!("Failed to create worker pool: {}", e))
            })?;

        Ok(Self {
            pool,
            scheduler: TestScheduler::new(),
            config: ExecutorConfig::new(),
        })
    }

    /// Update duration history for a test.
    pub fn update_duration(&mut self, node_id: &str, duration_ms: u64) {
        self.scheduler.update_duration(node_id, duration_ms);
    }

    /// Get the scheduler for external updates.
    pub fn scheduler_mut(&mut self) -> &mut TestScheduler {
        &mut self.scheduler
    }
}

#[async_trait]
impl TestExecutor for PooledExecutor {
    async fn run_test(&self, node_id: &str) -> Result<TestResult> {
        let results = self.run_tests(&[node_id.to_string()]).await;
        results.into_iter().next().ok_or_else(|| {
            crate::error::DaemonError::Other(format!("No result for test: {}", node_id))
        })
    }

    async fn run_tests(&self, node_ids: &[String]) -> Vec<TestResult> {
        if node_ids.is_empty() {
            return Vec::new();
        }

        let workers = self.config.workers.unwrap_or(num_cpus::get() as u32) as usize;

        // Use smarter batching: minimum batch size of 50 tests to avoid pytest.main() overhead
        // Each pytest.main() call has ~50-100ms overhead, so small batches are inefficient
        const MIN_BATCH_SIZE: usize = 50;
        let test_count = node_ids.len();

        // Calculate how many workers we actually need
        let actual_workers = if test_count <= MIN_BATCH_SIZE {
            1 // Run all in single worker for small test sets
        } else {
            // Use enough workers to keep batches >= MIN_BATCH_SIZE
            let max_workers_needed = (test_count + MIN_BATCH_SIZE - 1) / MIN_BATCH_SIZE;
            workers.min(max_workers_needed)
        };

        // Use load-balanced batching
        let batches = self.scheduler.split_balanced(node_ids, actual_workers);

        info!(
            "Running {} tests in {} batches (target batch size: {})",
            node_ids.len(),
            batches.len(),
            if batches.is_empty() {
                0
            } else {
                node_ids.len() / batches.len()
            }
        );

        // Execute batches in parallel using the worker pool
        self.pool.execute_parallel(batches).await
    }

    fn configure(&mut self, config: ExecutorConfig) {
        self.config = config;
    }

    fn execution_mode(&self) -> &'static str {
        "pooled"
    }

    fn kill_all(&self) {
        // Workers are managed by the pool - they'll be cleaned up on drop
        // For now, we don't forcefully kill them mid-execution
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_run_simple_test() {
        let pytest_available = AsyncCommand::new("python")
            .args(["-m", "pytest", "--version"])
            .output()
            .await
            .is_ok_and(|output| output.status.success());
        if !pytest_available {
            return;
        }

        let dir = TempDir::new_in(".").unwrap();
        let test_file = dir.path().join("test_example.py");
        fs::write(
            &test_file,
            "def test_simple():\n    print('test output')\n    assert True\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("pytest.ini"),
            "[pytest]\naddopts = -qq --color=yes -s\n",
        )
        .unwrap();

        let dirname = dir.path().file_name().unwrap().to_string_lossy();
        let node_id = format!("{}/test_example.py::test_simple", dirname);
        let executor = PythonExecutor::new(PathBuf::from("python"));
        let result = executor.run_test(&node_id).await.unwrap();
        assert_eq!(result.node_id, node_id);
        assert_eq!(result.outcome, TestOutcome::Passed);
    }

    #[test]
    fn test_create_executor_subprocess() {
        let executor = create_executor(ExecutionMode::Subprocess, PathBuf::from("python"));
        assert!(executor.is_ok());
        let executor = executor.unwrap();
        assert_eq!(executor.execution_mode(), "subprocess");
    }

    #[test]
    fn test_create_executor_auto() {
        // Auto mode should succeed (either embedded or subprocess)
        let executor = create_executor(ExecutionMode::Auto, PathBuf::from("python"));
        assert!(executor.is_ok());
        let executor = executor.unwrap();
        // Should be either "embedded" or "subprocess"
        assert!(
            executor.execution_mode() == "embedded" || executor.execution_mode() == "subprocess"
        );
    }

    #[cfg(feature = "embedded-python")]
    #[test]
    fn test_create_executor_embedded() {
        // Only test if embedded Python is available
        if crate::embedded::EmbeddedExecutor::is_available() {
            let executor = create_executor(ExecutionMode::Embedded, PathBuf::from("python"));
            assert!(executor.is_ok());
            let executor = executor.unwrap();
            assert_eq!(executor.execution_mode(), "embedded");
        }
    }

    #[test]
    fn test_executor_config_default() {
        let config = ExecutorConfig::new();
        assert!(config.batch_size > 0);
        assert_eq!(config.test_timeout_secs, 60);
        assert!(config.workers.is_none());
        assert!(config.maxfail.is_none());
    }

    #[test]
    fn test_python_executor_configure() {
        let mut executor = PythonExecutor::new(PathBuf::from("python"));
        let mut config = ExecutorConfig::new();
        config.workers = Some(4);
        config.maxfail = Some(10);
        executor.configure(config);
        // Config is stored internally - verify it was set
        assert_eq!(executor.execution_mode(), "subprocess");
    }

    #[test]
    fn test_python_executor_default() {
        let executor = PythonExecutor::default();
        assert_eq!(executor.python_path(), &PathBuf::from("python"));
    }

    #[test]
    fn test_parse_batch_output_passed() {
        let executor = PythonExecutor::default();
        let output = "test_file.py::test_one PASSED\ntest_file.py::test_two PASSED\n";
        let node_ids = vec![
            "test_file.py::test_one".to_string(),
            "test_file.py::test_two".to_string(),
        ];

        let results = executor.parse_batch_output(&node_ids, &test_output(output, false));
        assert_eq!(results.len(), 2);
        assert!(matches!(results[0].outcome, TestOutcome::Passed));
        assert!(matches!(results[1].outcome, TestOutcome::Passed));
    }

    #[test]
    fn test_parse_batch_output_failed() {
        let executor = PythonExecutor::default();
        let output = "test_file.py::test_fail FAILED\n";
        let node_ids = vec!["test_file.py::test_fail".to_string()];

        let results = executor.parse_batch_output(&node_ids, &test_output(output, true));
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, TestOutcome::Failed));
    }

    #[test]
    fn test_parse_batch_output_skipped() {
        let executor = PythonExecutor::default();
        let output = "test_file.py::test_skip SKIPPED\n";
        let node_ids = vec!["test_file.py::test_skip".to_string()];

        let results = executor.parse_batch_output(&node_ids, &test_output(output, false));
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, TestOutcome::Skipped));
    }

    #[test]
    fn test_parse_batch_output_xfail() {
        let executor = PythonExecutor::default();
        let output = "test_file.py::test_xfail XFAIL\n";
        let node_ids = vec!["test_file.py::test_xfail".to_string()];

        let results = executor.parse_batch_output(&node_ids, &test_output(output, false));
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, TestOutcome::Xfail));
    }

    #[test]
    fn test_parse_batch_output_xpass() {
        let executor = PythonExecutor::default();
        let output = "test_file.py::test_xpass XPASS\n";
        let node_ids = vec!["test_file.py::test_xpass".to_string()];

        let results = executor.parse_batch_output(&node_ids, &test_output(output, false));
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, TestOutcome::Xpass));
    }

    #[test]
    fn test_parse_batch_output_mixed() {
        let executor = PythonExecutor::default();
        let output = "test_file.py::test_pass PASSED\ntest_file.py::test_fail FAILED\ntest_file.py::test_skip SKIPPED\n";
        let node_ids = vec![
            "test_file.py::test_pass".to_string(),
            "test_file.py::test_fail".to_string(),
            "test_file.py::test_skip".to_string(),
        ];

        let results = executor.parse_batch_output(&node_ids, &test_output(output, true));
        assert_eq!(results.len(), 3);
        assert!(matches!(results[0].outcome, TestOutcome::Passed));
        assert!(matches!(results[1].outcome, TestOutcome::Failed));
        assert!(matches!(results[2].outcome, TestOutcome::Skipped));
    }

    #[test]
    fn test_parse_batch_output_requires_explicit_outcome() {
        let executor = PythonExecutor::default();
        let output = "other_file.py::test_one PASSED\n= 1 passed in 0.01s =\n";
        let node_ids = vec!["test_file.py::test_one".to_string()];
        let results = executor.parse_batch_output(&node_ids, &test_output(output, false));
        assert_eq!(results[0].outcome, TestOutcome::Error);
    }

    #[test]
    fn test_parse_batch_output_matches_ids_with_spaces() {
        let executor = PythonExecutor::default();
        let node_ids = vec!["test_file.py::test_one[with spaces]".to_string()];
        let output = "test_file.py::test_one[with spaces] SKIPPED (reason) [100%]\n";
        let results = executor.parse_batch_output(&node_ids, &test_output(output, false));
        assert_eq!(results[0].outcome, TestOutcome::Skipped);
    }

    #[test]
    fn test_parse_batch_output_retains_unreported_nonzero_exit() {
        let executor = PythonExecutor::default();
        let node_ids = vec!["test_file.py::test_one".to_string()];
        let output = "test_file.py::test_one PASSED [100%]\n";
        let results = executor.parse_batch_output(&node_ids, &test_output(output, true));
        assert_eq!(results[0].outcome, TestOutcome::Error);
    }

    fn test_output(text: &str, failed: bool) -> PytestOutput {
        PytestOutput {
            text: text.into(),
            failed,
        }
    }

    #[test]
    fn test_extract_duration() {
        assert_eq!(
            PythonExecutor::extract_duration("= 1 passed in [0.12s] ="),
            Some(120)
        );
        assert_eq!(
            PythonExecutor::extract_duration("= 1 failed in [1.5s] ="),
            Some(1500)
        );
        assert_eq!(PythonExecutor::extract_duration("some random text"), None);
    }

    #[test]
    fn test_extract_message() {
        assert_eq!(
            PythonExecutor::extract_message("AssertionError: expected 1 got 2"),
            Some("AssertionError: expected 1 got 2".to_string())
        );
        assert_eq!(
            PythonExecutor::extract_message("Error: something failed"),
            Some("Error: something failed".to_string())
        );
        assert_eq!(PythonExecutor::extract_message("test passed ok"), None);
    }

    #[test]
    fn test_python_executor_kill_all() {
        let executor = PythonExecutor::default();
        // Should not panic even with no processes
        executor.kill_all();
    }

    #[test]
    fn test_executor_config_optimal_batch_size() {
        let batch_size = ExecutorConfig::optimal_batch_size();
        // Minimum batch size is 500 to reduce subprocess spawn overhead
        assert!(batch_size >= 500);
    }

    #[tokio::test]
    async fn test_run_tests_empty() {
        let executor = PythonExecutor::default();
        let results = executor.run_tests(&[]).await;
        assert!(results.is_empty());
    }
}
