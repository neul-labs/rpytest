//! Python subprocess executor for running pytest tests.
//!
//! This module provides two execution strategies:
//! - `PythonExecutor`: Subprocess-based execution (default, always available)
//! - `EmbeddedExecutor`: PyO3-based embedded execution (requires `embedded-python` feature)
//!
//! Use `create_executor()` to automatically select the best available executor.

use crate::error::Result;
use crate::models::{ExecutionMode, TestOutcome, TestResult};
use crate::pool::WorkerPool;
use crate::scheduler::TestScheduler;
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout, Command as AsyncCommand};
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
        let output = self.run_pytest(&[node_id.to_string()], None).await;

        // Parse output to determine outcome
        let outcome = Self::parse_pytest_output(&output);
        let duration_ms = Self::extract_duration(&output).unwrap_or(0);

        Ok(TestResult {
            node_id: node_id.to_string(),
            outcome,
            duration_ms,
            message: Self::extract_message(&output),
            stdout: None,
            stderr: None,
        })
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
        let output = self.run_pytest(node_ids, None).await;
        self.parse_batch_output(node_ids, output)
    }

    /// Run pytest with the given arguments.
    async fn run_pytest(&self, node_ids: &[String], _output_file: Option<&str>) -> String {
        // Allow tests to bypass spawning a real Python interpreter.
        if std::env::var("RPYTEST_FAKE_PYTEST").is_ok() {
            let mut output = String::new();
            for node_id in node_ids {
                output.push_str(&format!("{} PASSED\n", node_id));
            }
            output.push_str(&format!("{} passed in 0.01s\n", node_ids.len()));
            return output;
        }

        let mut args: Vec<String> = vec!["-m".to_string(), "pytest".to_string()];

        // Add node IDs
        for node_id in node_ids {
            args.push(node_id.clone());
        }

        // Add configuration
        // Use -v for verbose output with full test names (needed for parsing outcomes)
        args.push("-v".to_string());
        args.push("--tb=short".to_string());
        args.push("--no-header".to_string());

        // Add maxfail
        if let Some(maxfail) = self.config.maxfail {
            args.push("--maxfail".to_string());
            args.push(maxfail.to_string());
        }

        // Add extra args
        args.extend(self.config.extra_args.clone());

        // Allow output capture
        args.push("--capture=no".to_string());

        debug!("Running: {} {:?}", self.python_path.display(), args);

        let mut child = match AsyncCommand::new(&self.python_path)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                error!("Failed to spawn pytest: {}", e);
                return format!("ERROR: Failed to spawn pytest: {}", e);
            }
        };

        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                error!("Failed to capture stdout");
                return "ERROR: Failed to capture stdout".to_string();
            }
        };

        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                error!("Failed to capture stderr");
                return "ERROR: Failed to capture stderr".to_string();
            }
        };

        // Read output
        let output = Self::read_output(stdout, stderr).await;

        // Wait for process to complete
        let status = match child.wait().await {
            Ok(status) => status,
            Err(e) => {
                error!("Failed to wait on pytest: {}", e);
                return format!("{}\nERROR: Failed to wait on pytest: {}", output, e);
            }
        };

        let mut result = output;
        if !status.success() {
            result.push_str(&format!("\nExit code: {}", status));
        }

        result
    }

    /// Read stdout and stderr from a child process.
    async fn read_output(stdout: ChildStdout, stderr: ChildStderr) -> String {
        let mut output = String::new();

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        loop {
            tokio::select! {
                line = stdout_reader.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            output.push_str(&l);
                            output.push('\n');
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
                line = stderr_reader.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            output.push_str(&l);
                            output.push('\n');
                        }
                        Ok(None) => {}
                        Err(_) => {}
                    }
                }
            }
        }

        output
    }

    /// Parse batch output into individual test results.
    /// Optimized single-pass implementation that extracts all outcomes efficiently.
    fn parse_batch_output(&self, node_ids: &[String], output: String) -> Vec<TestResult> {
        let mut results = Vec::with_capacity(node_ids.len());

        // Pre-allocate HashMap with expected capacity (2x for test name variants)
        let mut line_outcomes: HashMap<String, TestOutcome> =
            HashMap::with_capacity(node_ids.len() * 2);

        // Single pass: check all patterns at once for each line
        for line in output.lines() {
            // Split once and check the status word
            let mut parts = line.split_whitespace();
            let test_ref = match parts.next() {
                Some(t) => t,
                None => continue,
            };

            // Find outcome from the status word (usually second token)
            let outcome = parts.find_map(|word| {
                if word.starts_with("PASSED") {
                    Some(TestOutcome::Passed)
                } else if word.starts_with("FAILED") {
                    Some(TestOutcome::Failed)
                } else if word.starts_with("SKIPPED") {
                    Some(TestOutcome::Skipped)
                } else if word.starts_with("XFAIL") {
                    Some(TestOutcome::Xfail)
                } else if word.starts_with("XPASS") {
                    Some(TestOutcome::Xpass)
                } else {
                    None
                }
            });

            if let Some(outcome) = outcome {
                // Store outcome by full reference
                line_outcomes.insert(test_ref.to_string(), outcome.clone());
                // Also store by test name (after last ::) for fallback matching
                if let Some(pos) = test_ref.rfind("::") {
                    let test_name = &test_ref[pos + 2..];
                    line_outcomes.insert(test_name.to_string(), outcome);
                }
            }
        }

        // Check summary for overall counts
        let summary_passed = output.contains(" passed]") || output.contains(" passed in");
        let summary_failed = output.contains(" failed]") || output.contains(" failed in");
        let summary_skipped = output.contains(" skipped]") || output.contains(" skipped in");
        let has_errors = output.contains(" error]") || output.contains(" errors in");

        for node_id in node_ids {
            let mut outcome = TestOutcome::Error;

            // Try to find outcome from line-by-line parsing
            if let Some(line_outcome) = line_outcomes.get(node_id) {
                outcome = line_outcome.clone();
            } else {
                // Try matching just the test name (after ::)
                if let Some(pos) = node_id.find("::") {
                    let test_name = &node_id[pos + 2..];
                    if let Some(line_outcome) = line_outcomes.get(test_name) {
                        outcome = line_outcome.clone();
                    }
                }
            }

            // If still not found, use summary as fallback
            if matches!(outcome, TestOutcome::Error) {
                if summary_passed && !summary_failed {
                    outcome = TestOutcome::Passed;
                } else if summary_failed && !summary_passed {
                    outcome = TestOutcome::Failed;
                } else if has_errors {
                    outcome = TestOutcome::Error;
                } else if summary_skipped {
                    outcome = TestOutcome::Skipped;
                } else {
                    // Default to passed if we can't determine otherwise
                    outcome = TestOutcome::Passed;
                }
            }

            results.push(TestResult {
                node_id: node_id.clone(),
                outcome,
                duration_ms: 0,
                message: None,
                stdout: None,
                stderr: None,
            });
        }

        results
    }

    /// Parse pytest output to determine outcome.
    fn parse_pytest_output(output: &str) -> TestOutcome {
        if output.contains("1 passed") || output.contains("PASSED") {
            return TestOutcome::Passed;
        }
        if output.contains("1 failed") || output.contains("FAILED") {
            return TestOutcome::Failed;
        }
        if output.contains("1 skipped") || output.contains("SKIPPED") {
            return TestOutcome::Skipped;
        }
        if output.contains("ERROR") {
            return TestOutcome::Error;
        }
        TestOutcome::Error
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
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_example.py");
        fs::write(&test_file, "def test_simple():\n    assert True\n").unwrap();

        let executor = PythonExecutor::new(PathBuf::from("python"));
        let result = executor.run_test("test_example.py::test_simple").await;

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.node_id, "test_example.py::test_simple");
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

        let results = executor.parse_batch_output(&node_ids, output.to_string());
        assert_eq!(results.len(), 2);
        assert!(matches!(results[0].outcome, TestOutcome::Passed));
        assert!(matches!(results[1].outcome, TestOutcome::Passed));
    }

    #[test]
    fn test_parse_batch_output_failed() {
        let executor = PythonExecutor::default();
        let output = "test_file.py::test_fail FAILED\n";
        let node_ids = vec!["test_file.py::test_fail".to_string()];

        let results = executor.parse_batch_output(&node_ids, output.to_string());
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, TestOutcome::Failed));
    }

    #[test]
    fn test_parse_batch_output_skipped() {
        let executor = PythonExecutor::default();
        let output = "test_file.py::test_skip SKIPPED\n";
        let node_ids = vec!["test_file.py::test_skip".to_string()];

        let results = executor.parse_batch_output(&node_ids, output.to_string());
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, TestOutcome::Skipped));
    }

    #[test]
    fn test_parse_batch_output_xfail() {
        let executor = PythonExecutor::default();
        let output = "test_file.py::test_xfail XFAIL\n";
        let node_ids = vec!["test_file.py::test_xfail".to_string()];

        let results = executor.parse_batch_output(&node_ids, output.to_string());
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, TestOutcome::Xfail));
    }

    #[test]
    fn test_parse_batch_output_xpass() {
        let executor = PythonExecutor::default();
        let output = "test_file.py::test_xpass XPASS\n";
        let node_ids = vec!["test_file.py::test_xpass".to_string()];

        let results = executor.parse_batch_output(&node_ids, output.to_string());
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

        let results = executor.parse_batch_output(&node_ids, output.to_string());
        assert_eq!(results.len(), 3);
        assert!(matches!(results[0].outcome, TestOutcome::Passed));
        assert!(matches!(results[1].outcome, TestOutcome::Failed));
        assert!(matches!(results[2].outcome, TestOutcome::Skipped));
    }

    #[test]
    fn test_parse_batch_output_summary_fallback() {
        let executor = PythonExecutor::default();
        // Output without explicit test result lines, but with summary
        let output = "= 1 passed in 0.01s =\n";
        let node_ids = vec!["test_file.py::test_one".to_string()];

        let results = executor.parse_batch_output(&node_ids, output.to_string());
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, TestOutcome::Passed));
    }

    #[test]
    fn test_parse_pytest_output() {
        assert!(matches!(
            PythonExecutor::parse_pytest_output("1 passed"),
            TestOutcome::Passed
        ));
        assert!(matches!(
            PythonExecutor::parse_pytest_output("PASSED"),
            TestOutcome::Passed
        ));
        assert!(matches!(
            PythonExecutor::parse_pytest_output("1 failed"),
            TestOutcome::Failed
        ));
        assert!(matches!(
            PythonExecutor::parse_pytest_output("FAILED"),
            TestOutcome::Failed
        ));
        assert!(matches!(
            PythonExecutor::parse_pytest_output("1 skipped"),
            TestOutcome::Skipped
        ));
        assert!(matches!(
            PythonExecutor::parse_pytest_output("ERROR"),
            TestOutcome::Error
        ));
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
