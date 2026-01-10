//! Worker pool for persistent Python test execution.
//!
//! This module provides a pool of warm Python worker processes that stay alive
//! between test runs, eliminating subprocess spawn overhead (200-300ms per batch).

use crate::error::{DaemonError, Result};
use crate::models::{TestOutcome, TestResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Python worker script that runs as a persistent process.
/// Communicates via JSON over stdin/stdout.
const WORKER_SCRIPT: &str = r#"
import sys
import json
import io
import os

# Save original stdout for JSON protocol communication
_original_stdout = sys.stdout

class WorkerPlugin:
    """Pytest plugin that collects results for the worker."""

    def __init__(self):
        self.results = []

    def pytest_runtest_logreport(self, report):
        if report.when == 'call':
            self.results.append({
                'nodeid': report.nodeid,
                'outcome': report.outcome,
                'duration': report.duration,
                'message': getattr(report, 'longreprtext', None)
            })
        elif report.when == 'setup' and report.outcome == 'skipped':
            self.results.append({
                'nodeid': report.nodeid,
                'outcome': 'skipped',
                'duration': report.duration,
                'message': getattr(report, 'longreprtext', None)
            })
        elif report.when in ('setup', 'teardown') and report.outcome == 'failed':
            self.results.append({
                'nodeid': report.nodeid,
                'outcome': 'error',
                'duration': report.duration,
                'message': getattr(report, 'longreprtext', None)
            })

def send_response(data):
    """Send JSON response to the daemon."""
    _original_stdout.write(json.dumps(data) + '\n')
    _original_stdout.flush()

# Signal ready
send_response({'status': 'ready'})

run_count = 0
max_runs = 100  # Recycle worker after N runs to prevent memory leaks

# Import pytest here to avoid startup time in the loop
import pytest

while run_count < max_runs:
    try:
        line = sys.stdin.readline()
        if not line:
            break

        request = json.loads(line)
        cmd = request.get('command')

        if cmd == 'run':
            # Clear test modules to ensure fresh imports
            to_remove = [k for k in sys.modules.keys()
                        if 'test_' in k or 'conftest' in k]
            for k in to_remove:
                del sys.modules[k]

            # Redirect stdout/stderr during pytest run to avoid interfering with JSON protocol
            captured_stdout = io.StringIO()
            captured_stderr = io.StringIO()
            sys.stdout = captured_stdout
            sys.stderr = captured_stderr

            try:
                plugin = WorkerPlugin()
                exit_code = pytest.main(
                    request['tests'] + ['-q', '--tb=short', '-p', 'no:cacheprovider'],
                    plugins=[plugin]
                )
            finally:
                # Restore stdout/stderr
                sys.stdout = _original_stdout
                sys.stderr = sys.__stderr__

            send_response({
                'status': 'done',
                'exit_code': exit_code,
                'results': plugin.results
            })

            run_count += 1

        elif cmd == 'ping':
            send_response({'status': 'pong'})

        elif cmd == 'shutdown':
            break

    except Exception as e:
        # Ensure stdout is restored even on exception
        sys.stdout = _original_stdout
        send_response({'status': 'error', 'message': str(e)})

# Signal shutdown
send_response({'status': 'shutdown'})
"#;

/// Request sent to a worker.
#[derive(Debug, Serialize)]
#[serde(tag = "command")]
enum WorkerRequest {
    #[serde(rename = "run")]
    Run { tests: Vec<String> },
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "shutdown")]
    Shutdown,
}

/// Response from a worker.
#[derive(Debug, Deserialize)]
struct WorkerResponse {
    status: String,
    #[serde(default)]
    exit_code: i32,
    #[serde(default)]
    results: Vec<WorkerResult>,
    #[serde(default)]
    message: Option<String>,
}

/// Individual test result from a worker.
#[derive(Debug, Deserialize)]
struct WorkerResult {
    nodeid: String,
    outcome: String,
    duration: f64,
    message: Option<String>,
}

impl WorkerResult {
    fn into_test_result(self) -> TestResult {
        let outcome = match self.outcome.as_str() {
            "passed" => TestOutcome::Passed,
            "failed" => TestOutcome::Failed,
            "skipped" => TestOutcome::Skipped,
            "error" => TestOutcome::Error,
            "xfail" => TestOutcome::Xfail,
            "xpass" => TestOutcome::Xpass,
            _ => TestOutcome::Error,
        };

        TestResult {
            node_id: self.nodeid,
            outcome,
            duration_ms: (self.duration * 1000.0) as u64,
            message: self.message,
            stdout: None,
            stderr: None,
        }
    }
}

/// A persistent Python worker process.
pub struct Worker {
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    id: usize,
    run_count: usize,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            .field("id", &self.id)
            .field("run_count", &self.run_count)
            .finish_non_exhaustive()
    }
}

impl Worker {
    /// Spawn a new worker process.
    pub async fn spawn(python_path: &PathBuf, id: usize, working_dir: &PathBuf) -> Result<Self> {
        debug!("Spawning worker {} with python: {} in dir: {}", id, python_path.display(), working_dir.display());

        let mut child = Command::new(python_path)
            .arg("-c")
            .arg(WORKER_SCRIPT)
            .current_dir(working_dir) // Run from repo root so test paths resolve correctly
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // Ignore stderr to avoid blocking
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| DaemonError::Other(format!("Failed to spawn worker {}: {}", id, e)))?;

        let stdin = child.stdin.take().ok_or_else(|| {
            DaemonError::Other(format!("Worker {} has no stdin", id))
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            DaemonError::Other(format!("Worker {} has no stdout", id))
        })?;

        let mut worker = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            id,
            run_count: 0,
        };

        // Wait for ready signal
        worker.wait_ready().await?;
        info!("Worker {} ready", id);

        Ok(worker)
    }

    /// Wait for the worker to signal ready.
    async fn wait_ready(&mut self) -> Result<()> {
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.stdout.read_line(&mut line),
        )
        .await
        .map_err(|_| DaemonError::Other(format!("Worker {} timed out during startup", self.id)))?
        .map_err(|e| DaemonError::Other(format!("Worker {} read error: {}", self.id, e)))?;

        let response: WorkerResponse = serde_json::from_str(&line)
            .map_err(|e| DaemonError::Other(format!("Worker {} invalid response: {}", self.id, e)))?;

        if response.status != "ready" {
            return Err(DaemonError::Other(format!(
                "Worker {} sent unexpected status: {}",
                self.id, response.status
            )));
        }

        Ok(())
    }

    /// Run tests and return results.
    pub async fn run_tests(&mut self, tests: Vec<String>) -> Result<Vec<TestResult>> {
        let request = WorkerRequest::Run { tests };
        let request_json = serde_json::to_string(&request)
            .map_err(|e| DaemonError::Other(format!("Failed to serialize request: {}", e)))?;

        // Send request
        self.stdin
            .write_all(format!("{}\n", request_json).as_bytes())
            .await
            .map_err(|e| DaemonError::Other(format!("Worker {} write error: {}", self.id, e)))?;
        self.stdin.flush().await.map_err(|e| {
            DaemonError::Other(format!("Worker {} flush error: {}", self.id, e))
        })?;

        // Read response (with timeout)
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(300), // 5 minute timeout for tests
            self.stdout.read_line(&mut line),
        )
        .await
        .map_err(|_| DaemonError::Other(format!("Worker {} timed out during test run", self.id)))?
        .map_err(|e| DaemonError::Other(format!("Worker {} read error: {}", self.id, e)))?;

        let response: WorkerResponse = serde_json::from_str(&line)
            .map_err(|e| DaemonError::Other(format!("Worker {} invalid response: {} (line: {})", self.id, e, line)))?;

        if response.status == "error" {
            return Err(DaemonError::Other(format!(
                "Worker {} error: {}",
                self.id,
                response.message.unwrap_or_default()
            )));
        }

        self.run_count += 1;
        debug!(
            "Worker {} completed run {} with {} results",
            self.id,
            self.run_count,
            response.results.len()
        );

        Ok(response.results.into_iter().map(|r| r.into_test_result()).collect())
    }

    /// Check if the worker needs recycling (too many runs).
    pub fn needs_recycle(&self) -> bool {
        self.run_count >= 100
    }

    /// Check if the worker is still alive.
    pub fn is_alive(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,  // Still running
            Ok(Some(_)) => false,  // Exited
            Err(_) => false,  // Error checking
        }
    }

    /// Gracefully shutdown the worker.
    pub async fn shutdown(&mut self) -> Result<()> {
        let request = WorkerRequest::Shutdown;
        let request_json = serde_json::to_string(&request).unwrap_or_default();

        let _ = self.stdin.write_all(format!("{}\n", request_json).as_bytes()).await;
        let _ = self.stdin.flush().await;

        // Give it a moment to shutdown gracefully
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Force kill if still running
        let _ = self.child.kill().await;

        Ok(())
    }
}

/// Pool of warm Python workers.
#[derive(Debug)]
pub struct WorkerPool {
    /// Available workers ready to accept work
    available: Mutex<Vec<Worker>>,
    /// Path to Python interpreter
    python_path: PathBuf,
    /// Working directory (repo root) for workers
    working_dir: PathBuf,
    /// Target pool size
    size: usize,
}

impl WorkerPool {
    /// Create a new worker pool with the specified size.
    pub async fn new(size: usize, python_path: PathBuf, working_dir: PathBuf) -> Result<Arc<Self>> {
        info!("Creating worker pool with {} workers in {}", size, working_dir.display());

        let pool = Arc::new(Self {
            available: Mutex::new(Vec::with_capacity(size)),
            python_path,
            working_dir,
            size,
        });

        // Spawn initial workers
        pool.ensure_workers().await?;

        Ok(pool)
    }

    /// Ensure the pool has enough workers.
    async fn ensure_workers(&self) -> Result<()> {
        let mut available = self.available.lock().await;
        let current = available.len();

        if current < self.size {
            for i in current..self.size {
                match Worker::spawn(&self.python_path, i, &self.working_dir).await {
                    Ok(worker) => available.push(worker),
                    Err(e) => {
                        warn!("Failed to spawn worker {}: {}", i, e);
                        // Continue with fewer workers
                    }
                }
            }
        }

        Ok(())
    }

    /// Get an available worker from the pool.
    async fn acquire(&self) -> Option<Worker> {
        let mut available = self.available.lock().await;

        // Find a healthy worker
        while let Some(mut worker) = available.pop() {
            if worker.is_alive() && !worker.needs_recycle() {
                return Some(worker);
            }
            // Worker is dead or needs recycling, let it drop
            debug!("Worker {} needs recycling", worker.id);
        }

        None
    }

    /// Return a worker to the pool.
    async fn release(&self, worker: Worker) {
        let mut available = self.available.lock().await;
        if available.len() < self.size {
            available.push(worker);
        }
        // Otherwise let it drop (pool is full)
    }

    /// Execute a batch of tests on an available worker.
    pub async fn execute_batch(&self, tests: Vec<String>) -> Vec<TestResult> {
        if tests.is_empty() {
            return Vec::new();
        }

        // Try to get a worker
        let worker = match self.acquire().await {
            Some(w) => w,
            None => {
                // No workers available, try to spawn one
                match Worker::spawn(&self.python_path, 999, &self.working_dir).await {
                    Ok(w) => w,
                    Err(e) => {
                        warn!("Failed to spawn worker: {}", e);
                        return tests
                            .into_iter()
                            .map(|id| TestResult {
                                node_id: id,
                                outcome: TestOutcome::Error,
                                duration_ms: 0,
                                message: Some(format!("Worker pool exhausted: {}", e)),
                                stdout: None,
                                stderr: None,
                            })
                            .collect();
                    }
                }
            }
        };

        let mut worker = worker;
        let results = match worker.run_tests(tests.clone()).await {
            Ok(r) => r,
            Err(e) => {
                warn!("Worker error: {}", e);
                tests
                    .into_iter()
                    .map(|id| TestResult {
                        node_id: id,
                        outcome: TestOutcome::Error,
                        duration_ms: 0,
                        message: Some(format!("Worker error: {}", e)),
                        stdout: None,
                        stderr: None,
                    })
                    .collect()
            }
        };

        // Return worker to pool
        self.release(worker).await;

        // Ensure we have workers for next request
        let _ = self.ensure_workers().await;

        results
    }

    /// Execute multiple batches in parallel.
    pub async fn execute_parallel(self: &Arc<Self>, batches: Vec<Vec<String>>) -> Vec<TestResult> {
        if batches.is_empty() {
            return Vec::new();
        }

        // Spawn a task for each batch
        let handles: Vec<_> = batches
            .into_iter()
            .map(|batch| {
                let pool = Arc::clone(self);
                tokio::spawn(async move { pool.execute_batch(batch).await })
            })
            .collect();

        // Collect results
        let mut all_results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(results) => all_results.extend(results),
                Err(e) => warn!("Task join error: {}", e),
            }
        }

        all_results
    }

    /// Shutdown all workers in the pool.
    pub async fn shutdown(&self) {
        let mut available = self.available.lock().await;
        for mut worker in available.drain(..) {
            let _ = worker.shutdown().await;
        }
    }

    /// Get the current number of available workers.
    pub async fn available_count(&self) -> usize {
        self.available.lock().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn get_python_path() -> PathBuf {
        // Try common Python paths
        for path in &["python3", "python", ".venv/bin/python"] {
            if let Ok(p) = which::which(path) {
                return p;
            }
        }
        PathBuf::from("python3")
    }

    #[tokio::test]
    async fn test_worker_spawn() {
        let python_path = get_python_path();
        let working_dir = std::env::current_dir().unwrap();
        let worker = Worker::spawn(&python_path, 0, &working_dir).await;

        // This test may fail if Python or pytest is not available
        if worker.is_err() {
            eprintln!("Skipping test: {:?}", worker.err());
            return;
        }

        let mut worker = worker.unwrap();
        assert!(worker.is_alive());

        // Cleanup
        let _ = worker.shutdown().await;
    }

    #[tokio::test]
    async fn test_worker_request_serialization() {
        let request = WorkerRequest::Run {
            tests: vec!["test_a.py::test_1".to_string()],
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"command\":\"run\""));
        assert!(json.contains("test_a.py::test_1"));
    }
}
