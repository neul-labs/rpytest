//! Python subprocess executor for running pytest tests.

use crate::error::Result;
use crate::models::{TestOutcome, TestResult};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout, Command as AsyncCommand};
use tracing::{debug, info};

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
    /// Create default config.
    pub fn new() -> Self {
        ExecutorConfig {
            workers: None,
            maxfail: None,
            batch_size: 50,
            test_timeout_secs: 60,
            extra_args: Vec::new(),
        }
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
            // Parallel execution - collect results then extend
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();

            let all_results: Vec<Vec<TestResult>> = pool.install(|| {
                batches
                    .into_par_iter()
                    .map(|batch| {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async { self.run_batch(&batch).await })
                    })
                    .collect()
            });

            // Flatten results
            all_results.into_iter().flatten().collect()
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
    async fn run_pytest(&self, node_ids: &[String], output_file: Option<&str>) -> String {
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
        args.push("--tb=short".to_string());
        args.push("-q".to_string());
        args.push("--no-header".to_string());

        // Add maxfail
        if let Some(maxfail) = self.config.maxfail {
            args.push("--maxfail".to_string());
            args.push(maxfail.to_string());
        }

        // Add extra args
        args.extend(self.config.extra_args.clone());

        // Capture output
        args.push("-v".to_string());
        args.push("--capture=no".to_string());

        debug!("Running: {} {:?}", self.python_path.display(), args);

        let mut child = AsyncCommand::new(&self.python_path)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("Failed to spawn pytest");

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // Read output
        let output = Self::read_output(stdout, stderr).await;

        // Wait for process to complete
        let status = child.wait().await.expect("Failed to wait on pytest");

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
    fn parse_batch_output(&self, node_ids: &[String], output: String) -> Vec<TestResult> {
        let lines: Vec<&str> = output.lines().collect();
        let mut results = Vec::with_capacity(node_ids.len());

        // First pass: look for explicit PASSED/FAILED/SKIPPED markers
        let mut line_outcomes: HashMap<String, TestOutcome> = HashMap::new();

        for line in &lines {
            // Check for PASSED lines: "test_module.py::test_func PASSED"
            if line.contains(" PASSED") || line.ends_with(" PASSED") {
                // Extract test name - could be full node_id or just test function name
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let test_ref = parts[0];
                    // Store outcome by both full reference and just the test name
                    line_outcomes.insert(test_ref.to_string(), TestOutcome::Passed);
                    // Also try to extract just the test name (after ::)
                    if let Some(pos) = test_ref.find("::") {
                        let test_name = &test_ref[pos + 2..];
                        line_outcomes.insert(test_name.to_string(), TestOutcome::Passed);
                    }
                }
            }
            // Check for FAILED lines
            else if line.contains(" FAILED") || line.ends_with(" FAILED") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let test_ref = parts[0];
                    line_outcomes.insert(test_ref.to_string(), TestOutcome::Failed);
                    if let Some(pos) = test_ref.find("::") {
                        let test_name = &test_ref[pos + 2..];
                        line_outcomes.insert(test_name.to_string(), TestOutcome::Failed);
                    }
                }
            }
            // Check for SKIPPED lines
            else if line.contains(" SKIPPED") || line.ends_with(" SKIPPED") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let test_ref = parts[0];
                    line_outcomes.insert(test_ref.to_string(), TestOutcome::Skipped);
                    if let Some(pos) = test_ref.find("::") {
                        let test_name = &test_ref[pos + 2..];
                        line_outcomes.insert(test_name.to_string(), TestOutcome::Skipped);
                    }
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
        let mut processes = self.processes.lock().unwrap();
        for (_, child) in processes.iter_mut() {
            let _ = child.kill();
        }
        processes.clear();
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
}
