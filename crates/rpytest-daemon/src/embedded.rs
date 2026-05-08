//! Embedded Python execution using PyO3.
//!
//! This module provides native Python execution by embedding the Python interpreter
//! directly into the Rust daemon, eliminating subprocess overhead.
//!
//! Key features:
//! - Direct pytest.main() calls without subprocess spawn
//! - Streaming results via custom pytest plugin
//! - Module cache cleanup between runs
//! - Signal handler and event loop management
//! - Python 3.8+ support via abi3 stable ABI

use crate::error::{DaemonError, Result};
use crate::executor::{ExecutorConfig, TestExecutor};
use crate::models::{TestOutcome, TestResult};
use async_trait::async_trait;
use parking_lot::Mutex;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyModule};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tracing::{debug, info, warn};

/// Ensure PYTHONHOME is set before PyO3 initializes the interpreter.
///
/// Without PYTHONHOME, the embedded Python interpreter cannot find the standard
/// library (e.g., `encodings` module), causing a fatal error:
///   "Fatal Python error: Failed to import encodings module"
///
/// This function detects the correct prefix by running the system `python3`.
fn ensure_pythonhome() {
    if std::env::var("PYTHONHOME").is_ok() {
        debug!("PYTHONHOME already set");
        return;
    }

    // Try to detect from VIRTUAL_ENV first (resolve to base_prefix)
    if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
        // In a virtualenv, we need the base prefix (the actual Python installation),
        // not the virtualenv prefix itself.
        let python = std::path::Path::new(&venv).join("bin").join("python3");
        if python.exists() {
            if let Some(base_prefix) = query_python_base_prefix(&python) {
                info!(
                    "Auto-detected PYTHONHOME from VIRTUAL_ENV's base_prefix: {}",
                    base_prefix
                );
                std::env::set_var("PYTHONHOME", &base_prefix);
                return;
            }
        }
    }

    // Fall back to system python3
    if let Some(base_prefix) = query_python_base_prefix(std::path::Path::new("python3")) {
        info!("Auto-detected PYTHONHOME from python3: {}", base_prefix);
        std::env::set_var("PYTHONHOME", &base_prefix);
        return;
    }

    warn!("Could not auto-detect PYTHONHOME; embedded Python may fail to initialize");
}

/// Run `python3 -c "import sys; print(sys.base_prefix)"` and return the output.
fn query_python_base_prefix(python: &std::path::Path) -> Option<String> {
    match std::process::Command::new(python)
        .args(["-c", "import sys; print(sys.base_prefix)"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => {
            let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !prefix.is_empty() {
                Some(prefix)
            } else {
                None
            }
        }
        Ok(_) => {
            debug!("python3 exited with non-zero status while querying base_prefix");
            None
        }
        Err(e) => {
            debug!("Failed to run python3 for base_prefix detection: {}", e);
            None
        }
    }
}

/// Global cache for compiled plugin class - avoids recompiling Python on every run
static CACHED_PLUGIN_CLASS: OnceLock<Py<PyAny>> = OnceLock::new();

/// Track if this is the first test run (skip cleanup on first run)
static FIRST_RUN: AtomicBool = AtomicBool::new(true);

/// Python plugin code for collecting test results.
/// This plugin hooks into pytest's reporting system to stream results to Rust.
/// OPTIMIZED: Batches results locally and sends them all at once in pytest_sessionfinish
const COLLECTOR_PLUGIN: &str = r#"
import pytest

class RpytestCollectorPlugin:
    """Pytest plugin that collects results and streams them to Rust."""

    def __init__(self, collector):
        self.collector = collector
        self._collected_count = 0
        self._passed = 0
        self._failed = 0
        self._skipped = 0
        self._errors = 0
        # OPTIMIZATION: Batch results locally instead of calling Rust for each test
        self._results_batch = []

    def pytest_runtest_logreport(self, report):
        """Called for each test phase (setup, call, teardown)."""
        # Only record the 'call' phase for test outcomes
        # setup/teardown phases are tracked separately
        if report.when == 'call':
            # Batch locally instead of calling Rust per-result
            self._results_batch.append((
                report.nodeid,
                report.outcome,
                report.duration,
                getattr(report, 'longreprtext', None)
            ))

            # Track summary counts
            if report.outcome == 'passed':
                self._passed += 1
            elif report.outcome == 'failed':
                self._failed += 1
            elif report.outcome == 'skipped':
                self._skipped += 1
        elif report.when == 'setup' and report.outcome == 'skipped':
            # Handle skip during setup (e.g., skip marker)
            self._results_batch.append((
                report.nodeid,
                'skipped',
                report.duration,
                getattr(report, 'longreprtext', None)
            ))
            self._skipped += 1
        elif report.when in ('setup', 'teardown') and report.outcome == 'failed':
            # Handle errors during setup/teardown
            self._results_batch.append((
                report.nodeid,
                'error',
                report.duration,
                getattr(report, 'longreprtext', None)
            ))
            self._errors += 1

    def pytest_collection_finish(self, session):
        """Called after collection is complete."""
        self._collected_count = len(session.items)
        self.collector.set_collected_count(self._collected_count)

    def pytest_sessionfinish(self, session, exitstatus):
        """Called after the entire session finishes."""
        # Single Rust call with all results (instead of one per test)
        if self._results_batch:
            self.collector.report_batch(self._results_batch)
        self.collector.set_exit_status(exitstatus)
"#;

/// Rust-side result collector that receives results from the pytest plugin.
#[pyclass]
pub struct RustResultCollector {
    results: Arc<Mutex<Vec<TestResult>>>,
    collected_count: Arc<Mutex<usize>>,
    exit_status: Arc<Mutex<i32>>,
}

#[pymethods]
impl RustResultCollector {
    #[new]
    fn new() -> Self {
        RustResultCollector {
            results: Arc::new(Mutex::new(Vec::new())),
            collected_count: Arc::new(Mutex::new(0)),
            exit_status: Arc::new(Mutex::new(-1)),
        }
    }

    /// Called by pytest hook for each test result.
    #[pyo3(signature = (nodeid, outcome, duration, message=None))]
    fn report(&self, nodeid: String, outcome: String, duration: f64, message: Option<String>) {
        let test_outcome = match outcome.as_str() {
            "passed" => TestOutcome::Passed,
            "failed" => TestOutcome::Failed,
            "skipped" => TestOutcome::Skipped,
            "error" => TestOutcome::Error,
            "xfail" => TestOutcome::Xfail,
            "xpass" => TestOutcome::Xpass,
            _ => TestOutcome::Error,
        };

        let result = TestResult {
            node_id: nodeid,
            outcome: test_outcome,
            duration_ms: (duration * 1000.0) as u64,
            message,
            stdout: None,
            stderr: None,
        };

        self.results.lock().push(result);
    }

    /// Set the total collected test count.
    fn set_collected_count(&self, count: usize) {
        *self.collected_count.lock() = count;
    }

    /// Set the pytest exit status.
    fn set_exit_status(&self, status: i32) {
        *self.exit_status.lock() = status;
    }

    /// OPTIMIZATION: Receive all results in a single batch (reduces FFI overhead)
    fn report_batch(&self, results: Vec<(String, String, f64, Option<String>)>) {
        let mut lock = self.results.lock();
        for (nodeid, outcome, duration, message) in results {
            let test_outcome = match outcome.as_str() {
                "passed" => TestOutcome::Passed,
                "failed" => TestOutcome::Failed,
                "skipped" => TestOutcome::Skipped,
                "error" => TestOutcome::Error,
                "xfail" => TestOutcome::Xfail,
                "xpass" => TestOutcome::Xpass,
                _ => TestOutcome::Error,
            };

            lock.push(TestResult {
                node_id: nodeid,
                outcome: test_outcome,
                duration_ms: (duration * 1000.0) as u64,
                message,
                stdout: None,
                stderr: None,
            });
        }
    }
}

impl RustResultCollector {
    /// Extract all collected results.
    pub fn take_results(&self) -> Vec<TestResult> {
        std::mem::take(&mut *self.results.lock())
    }

    /// Get the collected test count.
    pub fn collected_count(&self) -> usize {
        *self.collected_count.lock()
    }

    /// Get the pytest exit status.
    pub fn exit_status(&self) -> i32 {
        *self.exit_status.lock()
    }
}

/// Configuration for the embedded executor.
#[derive(Debug, Clone, Default)]
pub struct EmbeddedExecutorConfig {
    /// Number of parallel workers (uses xdist if > 1)
    pub workers: Option<u32>,
    /// Stop after N failures
    pub maxfail: Option<u32>,
    /// Timeout per test in seconds
    pub test_timeout_secs: u64,
    /// Extra pytest arguments
    pub extra_args: Vec<String>,
}

/// Embedded Python executor that runs pytest directly without subprocess.
#[derive(Debug)]
pub struct EmbeddedExecutor {
    config: EmbeddedExecutorConfig,
    /// Track whether Python has been initialized
    initialized: bool,
    /// Python version info
    python_version: Option<(u8, u8, u8)>,
    /// Path to Python interpreter (used to derive virtualenv site-packages)
    python_path: Option<PathBuf>,
}

impl EmbeddedExecutor {
    /// Create a new embedded executor with optional python_path for virtualenv support.
    pub fn new(python_path: Option<PathBuf>) -> Result<Self> {
        // MUST happen before any Python::with_gil() call, otherwise the
        // interpreter can't find the stdlib and aborts with a fatal error.
        ensure_pythonhome();

        let mut executor = EmbeddedExecutor {
            config: EmbeddedExecutorConfig::default(),
            initialized: false,
            python_version: None,
            python_path,
        };

        // Initialize and validate Python
        executor.initialize()?;

        Ok(executor)
    }

    /// Configure the executor.
    pub fn configure(&mut self, config: EmbeddedExecutorConfig) {
        self.config = config;
    }

    /// Initialize embedded Python and validate environment.
    fn initialize(&mut self) -> Result<()> {
        Python::with_gil(|py| {
            // Get Python version
            let version = py.version_info();
            self.python_version = Some((version.major, version.minor, version.patch));

            // Validate minimum version (3.8+)
            if version.major < 3 || (version.major == 3 && version.minor < 8) {
                return Err(DaemonError::Other(format!(
                    "Python 3.8+ required, found {}.{}.{}",
                    version.major, version.minor, version.patch
                )));
            }

            // FIRST: Setup sys.path with virtualenv packages
            // This must happen BEFORE importing pytest so we can find it in the venv
            self.setup_python_path(py)?;

            // THEN: Verify pytest is available (now it can be found in venv)
            py.import_bound("pytest").map_err(|e| {
                DaemonError::Other(format!("pytest not installed or not importable: {}", e))
            })?;

            self.initialized = true;
            info!(
                "Embedded Python {}.{}.{} initialized successfully",
                version.major, version.minor, version.patch
            );

            Ok(())
        })
    }

    /// Setup Python path to include virtual environment packages.
    ///
    /// Priority order:
    /// 1. Derive site-packages from stored python_path (e.g., .venv/bin/python → .venv/lib/pythonX.Y/site-packages)
    /// 2. Use VIRTUAL_ENV environment variable as fallback
    ///
    /// Note: Handles version mismatches by scanning for any pythonX.Y directory
    fn setup_python_path(&self, py: Python) -> Result<()> {
        let sys = py
            .import_bound("sys")
            .map_err(|e| DaemonError::Other(format!("Failed to import sys: {}", e)))?;

        let path = sys
            .getattr("path")
            .map_err(|e| DaemonError::Other(format!("Failed to get sys.path: {}", e)))?;

        let path_list: &Bound<PyList> = path
            .downcast()
            .map_err(|e| DaemonError::Other(format!("sys.path is not a list: {}", e)))?;

        // Priority 1: Use stored python_path to derive site-packages
        // python_path is like /path/to/.venv/bin/python
        // Derive: /path/to/.venv/lib/pythonX.Y/site-packages
        if let Some(ref python_path) = self.python_path {
            // Go up from bin/python to venv root
            if let Some(venv_root) = python_path.parent().and_then(|p| p.parent()) {
                if let Some(site_packages) = Self::find_site_packages(venv_root) {
                    let sp_str = site_packages.to_string_lossy().to_string();
                    if !path_list.contains(&sp_str).unwrap_or(false) {
                        path_list.insert(0, &sp_str).map_err(|e| {
                            DaemonError::Other(format!("Failed to insert into sys.path: {}", e))
                        })?;
                        debug!("Added {} to sys.path from python_path", sp_str);
                    }
                }
            }
        }

        // Priority 2: Check VIRTUAL_ENV environment variable (fallback)
        if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
            let venv_root = std::path::Path::new(&venv);
            if let Some(site_packages) = Self::find_site_packages(venv_root) {
                let sp_str = site_packages.to_string_lossy().to_string();
                if !path_list.contains(&sp_str).unwrap_or(false) {
                    path_list.insert(0, &sp_str).map_err(|e| {
                        DaemonError::Other(format!("Failed to insert into sys.path: {}", e))
                    })?;
                    debug!("Added {} to sys.path from VIRTUAL_ENV", sp_str);
                }
            }
        }

        Ok(())
    }

    /// Find site-packages directory in a venv by scanning for pythonX.Y directories.
    /// This handles version mismatches between PyO3's Python and the venv's Python.
    fn find_site_packages(venv_root: &std::path::Path) -> Option<PathBuf> {
        let lib_dir = venv_root.join("lib");
        if !lib_dir.exists() {
            return None;
        }

        // Scan for pythonX.Y directories
        if let Ok(entries) = std::fs::read_dir(&lib_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("python")
                    && entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                {
                    let site_packages = entry.path().join("site-packages");
                    if site_packages.exists() {
                        debug!("Found site-packages at: {}", site_packages.display());
                        return Some(site_packages);
                    }
                }
            }
        }

        None
    }

    /// Clear test modules from sys.modules to ensure fresh imports.
    /// OPTIMIZED: Only clears modules we KNOW are test-related (O(test_paths) instead of O(all_modules))
    fn clear_test_modules(py: Python, test_paths: &[&str]) -> PyResult<()> {
        let sys = py.import_bound("sys")?;
        let modules_attr = sys.getattr("modules")?;
        let modules: &Bound<PyDict> = modules_attr.downcast()?;

        // Pre-allocate for efficiency
        let mut keys_to_remove = Vec::with_capacity(test_paths.len() * 2);

        for test_path in test_paths {
            // Convert file path to module path (e.g., "tests/test_foo.py" -> "tests.test_foo")
            let module_path = test_path.trim_end_matches(".py").replace('/', ".");

            // Check if this exact module exists
            if modules.contains(&module_path)? {
                keys_to_remove.push(module_path.clone());
            }

            // Also check for conftest in same directory
            if let Some(parent) = std::path::Path::new(test_path).parent() {
                let parent_str = parent.to_string_lossy();
                if !parent_str.is_empty() {
                    let conftest = format!("{}.conftest", parent_str.replace('/', "."));
                    if modules.contains(&conftest)? {
                        keys_to_remove.push(conftest);
                    }
                } else {
                    // Root-level conftest
                    if modules.contains("conftest")? {
                        keys_to_remove.push("conftest".to_string());
                    }
                }
            }
        }

        for key in &keys_to_remove {
            let _ = modules.del_item(key);
        }

        if !keys_to_remove.is_empty() {
            debug!(
                "Cleared {} test modules from sys.modules",
                keys_to_remove.len()
            );
        }
        Ok(())
    }

    /// Reset signal handlers to defaults.
    fn reset_signal_handlers(py: Python) -> PyResult<()> {
        let signal = py.import_bound("signal")?;

        // Reset SIGALRM (used by pytest-timeout)
        if let (Ok(sigalrm), Ok(sig_dfl)) = (signal.getattr("SIGALRM"), signal.getattr("SIG_DFL")) {
            let _ = signal.call_method1("signal", (sigalrm, sig_dfl));
        }

        // Reset SIGTERM
        if let (Ok(sigterm), Ok(sig_dfl)) = (signal.getattr("SIGTERM"), signal.getattr("SIG_DFL")) {
            let _ = signal.call_method1("signal", (sigterm, sig_dfl));
        }

        debug!("Reset signal handlers");
        Ok(())
    }

    /// Reset asyncio event loop for clean state.
    fn reset_asyncio(py: Python) -> PyResult<()> {
        if let Ok(asyncio) = py.import_bound("asyncio") {
            // Create a fresh event loop
            if let Ok(new_loop) = asyncio.call_method0("new_event_loop") {
                let _ = asyncio.call_method1("set_event_loop", (new_loop,));
            }
        }
        debug!("Reset asyncio event loop");
        Ok(())
    }

    /// Check if pytest-xdist is available.
    fn is_xdist_available(py: Python) -> bool {
        py.import_bound("xdist").is_ok()
    }

    /// Get or create the cached plugin class to avoid recompiling Python on every run.
    fn get_or_create_plugin_class(py: Python) -> Result<Bound<'_, PyAny>> {
        if let Some(cached) = CACHED_PLUGIN_CLASS.get() {
            return Ok(cached.bind(py).clone());
        }

        let plugin_module = PyModule::from_code_bound(
            py,
            COLLECTOR_PLUGIN,
            "rpytest_collector_plugin.py",
            "rpytest_collector_plugin",
        )
        .map_err(|e| DaemonError::Other(format!("Failed to compile plugin module: {}", e)))?;

        let plugin_class = plugin_module
            .getattr("RpytestCollectorPlugin")
            .map_err(|e| DaemonError::Other(format!("Failed to get plugin class: {}", e)))?;

        let _ = CACHED_PLUGIN_CLASS.set(plugin_class.unbind());

        Ok(CACHED_PLUGIN_CLASS.get().unwrap().bind(py).clone())
    }

    /// Run tests using embedded Python.
    pub fn run_tests(&self, node_ids: &[String]) -> Result<Vec<TestResult>> {
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        if !self.initialized {
            return Err(DaemonError::Other(
                "Embedded Python not initialized".to_string(),
            ));
        }

        Python::with_gil(|py| {
            // Cleanup from previous runs
            let test_paths: Vec<&str> = node_ids
                .iter()
                .filter_map(|id| id.split("::").next())
                .collect();

            // OPTIMIZATION: Skip cleanup on first run - no stale state yet
            let is_first_run = FIRST_RUN.swap(false, Ordering::SeqCst);
            if !is_first_run {
                Self::clear_test_modules(py, &test_paths)?;
                // Only reset signal handlers if we've run tests before
                Self::reset_signal_handlers(py)?;
                // Only reset asyncio if we've run async tests before
                Self::reset_asyncio(py)?;
            }

            // Create result collector
            let collector = Py::new(py, RustResultCollector::new()).map_err(|e| {
                DaemonError::Other(format!("Failed to create result collector: {}", e))
            })?;

            // OPTIMIZATION: Cache the plugin class to avoid recompiling Python on every run
            let plugin_class = Self::get_or_create_plugin_class(py)?;

            let plugin_instance = plugin_class
                .call1((collector.clone_ref(py),))
                .map_err(|e| {
                    DaemonError::Other(format!("Failed to create plugin instance: {}", e))
                })?;

            // Build pytest arguments
            let mut args: Vec<String> = node_ids.to_vec();

            // Add verbose output
            args.push("-v".to_string());
            args.push("--tb=short".to_string());
            args.push("--no-header".to_string());

            // Disable cache provider to avoid state pollution
            args.push("-p".to_string());
            args.push("no:cacheprovider".to_string());

            // Add maxfail
            if let Some(maxfail) = self.config.maxfail {
                args.push(format!("--maxfail={}", maxfail));
            }

            // Add xdist workers if available and requested
            if let Some(workers) = self.config.workers {
                if workers > 1 && Self::is_xdist_available(py) {
                    args.push("-n".to_string());
                    args.push(workers.to_string());
                    args.push("--dist=loadscope".to_string());
                } else if workers > 1 {
                    warn!("pytest-xdist not available, running tests sequentially");
                }
            }

            // Add extra args
            args.extend(self.config.extra_args.clone());

            // Convert args to Python list
            let py_args = PyList::new_bound(py, &args);

            // Create plugins list
            let plugins = PyList::new_bound(py, [plugin_instance]);

            // Import pytest and run
            let pytest = py
                .import_bound("pytest")
                .map_err(|e| DaemonError::Other(format!("Failed to import pytest: {}", e)))?;

            debug!("Running pytest with {} tests", node_ids.len());

            // Call pytest.main(args, plugins=[plugin])
            let kwargs = PyDict::new_bound(py);
            kwargs
                .set_item("plugins", plugins)
                .map_err(|e| DaemonError::Other(format!("Failed to set plugins kwarg: {}", e)))?;

            let exit_code: i32 = pytest
                .call_method("main", (py_args,), Some(&kwargs))
                .map_err(|e| DaemonError::Other(format!("pytest.main() failed: {}", e)))?
                .extract()
                .unwrap_or(-1);

            debug!("pytest.main() completed with exit code {}", exit_code);

            // Extract results from collector
            let collector_ref = collector.borrow(py);
            let results = collector_ref.take_results();

            // If no results were collected via the plugin, create error results
            if results.is_empty() && !node_ids.is_empty() {
                warn!(
                    "No results collected from pytest plugin (exit code: {})",
                    exit_code
                );
                // Return error results for all requested tests
                return Ok(node_ids
                    .iter()
                    .map(|id| TestResult {
                        node_id: id.clone(),
                        outcome: if exit_code == 0 {
                            TestOutcome::Passed
                        } else {
                            TestOutcome::Error
                        },
                        duration_ms: 0,
                        message: Some(format!("pytest exit code: {}", exit_code)),
                        stdout: None,
                        stderr: None,
                    })
                    .collect());
            }

            info!(
                "Collected {} results from {} tests",
                results.len(),
                node_ids.len()
            );

            Ok(results)
        })
    }

    /// Run a single test.
    pub fn run_test(&self, node_id: &str) -> Result<TestResult> {
        let results = self.run_tests(&[node_id.to_string()])?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| DaemonError::Other(format!("No result for test: {}", node_id)))
    }

    /// Get Python version info.
    pub fn python_version(&self) -> Option<(u8, u8, u8)> {
        self.python_version
    }

    /// Check if embedded Python is available and working.
    pub fn is_available() -> bool {
        // Ensure PYTHONHOME is set before touching the interpreter.
        ensure_pythonhome();

        // Use catch_unwind to prevent a fatal Python init error from
        // crashing the entire daemon; instead we return false and let
        // the caller fall back to subprocess mode.
        std::panic::catch_unwind(|| {
            Python::with_gil(|py| {
                // Check Python version
                let version = py.version_info();
                if version.major < 3 || (version.major == 3 && version.minor < 8) {
                    return false;
                }

                // Check pytest is importable
                py.import_bound("pytest").is_ok()
            })
        })
        .unwrap_or_else(|_| {
            warn!("Embedded Python initialization panicked; falling back to subprocess mode");
            false
        })
    }
}

impl Default for EmbeddedExecutor {
    fn default() -> Self {
        Self::new(None).expect("Failed to create EmbeddedExecutor")
    }
}

#[async_trait]
impl TestExecutor for EmbeddedExecutor {
    async fn run_test(&self, node_id: &str) -> Result<TestResult> {
        // EmbeddedExecutor::run_test is synchronous, but we wrap it for the async trait
        EmbeddedExecutor::run_test(self, node_id)
    }

    async fn run_tests(&self, node_ids: &[String]) -> Vec<TestResult> {
        // EmbeddedExecutor::run_tests is synchronous
        match EmbeddedExecutor::run_tests(self, node_ids) {
            Ok(results) => results,
            Err(e) => {
                warn!("Embedded execution failed: {}", e);
                // Return error results for all tests
                node_ids
                    .iter()
                    .map(|id| TestResult {
                        node_id: id.clone(),
                        outcome: TestOutcome::Error,
                        duration_ms: 0,
                        message: Some(format!("Embedded execution error: {}", e)),
                        stdout: None,
                        stderr: None,
                    })
                    .collect()
            }
        }
    }

    fn configure(&mut self, config: ExecutorConfig) {
        self.config = EmbeddedExecutorConfig {
            workers: config.workers,
            maxfail: config.maxfail,
            test_timeout_secs: config.test_timeout_secs,
            extra_args: config.extra_args.clone(),
        };
    }

    fn execution_mode(&self) -> &'static str {
        "embedded"
    }

    fn kill_all(&self) {
        // Embedded execution doesn't spawn processes, nothing to kill
        // But we could potentially interrupt the GIL in the future
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_executor_creation() {
        // This test will only pass if Python 3.8+ with pytest is available
        if EmbeddedExecutor::is_available() {
            let executor = EmbeddedExecutor::new(None);
            assert!(executor.is_ok());

            let executor = executor.unwrap();
            let version = executor.python_version();
            assert!(version.is_some());

            let (major, minor, _) = version.unwrap();
            assert!(major >= 3);
            assert!(minor >= 8 || major > 3);
        }
    }

    #[test]
    fn test_rust_result_collector() {
        Python::with_gil(|py| {
            let collector = Py::new(py, RustResultCollector::new()).unwrap();
            let collector_ref = collector.borrow(py);

            // Simulate reporting a result
            collector_ref.report(
                "test_file.py::test_func".to_string(),
                "passed".to_string(),
                0.123,
                None,
            );

            let results = collector_ref.take_results();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].node_id, "test_file.py::test_func");
            assert!(matches!(results[0].outcome, TestOutcome::Passed));
            assert_eq!(results[0].duration_ms, 123);
        });
    }

    #[test]
    fn test_rust_result_collector_multiple() {
        Python::with_gil(|py| {
            let collector = Py::new(py, RustResultCollector::new()).unwrap();
            let collector_ref = collector.borrow(py);

            // Report multiple results
            collector_ref.report(
                "test_a.py::test_1".to_string(),
                "passed".to_string(),
                0.05,
                None,
            );
            collector_ref.report(
                "test_b.py::test_2".to_string(),
                "failed".to_string(),
                0.1,
                Some("AssertionError".to_string()),
            );
            collector_ref.report(
                "test_c.py::test_3".to_string(),
                "skipped".to_string(),
                0.001,
                Some("reason: not implemented".to_string()),
            );

            let results = collector_ref.take_results();
            assert_eq!(results.len(), 3);
            assert!(matches!(results[0].outcome, TestOutcome::Passed));
            assert!(matches!(results[1].outcome, TestOutcome::Failed));
            assert!(matches!(results[2].outcome, TestOutcome::Skipped));
            assert_eq!(results[1].message, Some("AssertionError".to_string()));
        });
    }

    #[test]
    fn test_rust_result_collector_take_clears() {
        Python::with_gil(|py| {
            let collector = Py::new(py, RustResultCollector::new()).unwrap();
            let collector_ref = collector.borrow(py);

            collector_ref.report(
                "test.py::test_1".to_string(),
                "passed".to_string(),
                0.01,
                None,
            );

            // First take should get results
            let results1 = collector_ref.take_results();
            assert_eq!(results1.len(), 1);

            // Second take should be empty
            let results2 = collector_ref.take_results();
            assert!(results2.is_empty());
        });
    }

    #[test]
    fn test_rust_result_collector_outcomes() {
        Python::with_gil(|py| {
            let collector = Py::new(py, RustResultCollector::new()).unwrap();
            let collector_ref = collector.borrow(py);

            // Test all outcome types
            let outcomes = vec![
                ("passed", TestOutcome::Passed),
                ("failed", TestOutcome::Failed),
                ("skipped", TestOutcome::Skipped),
                ("error", TestOutcome::Error),
                ("xfail", TestOutcome::Xfail),
                ("xpass", TestOutcome::Xpass),
            ];

            for (i, (outcome_str, _)) in outcomes.iter().enumerate() {
                collector_ref.report(
                    format!("test.py::test_{}", i),
                    outcome_str.to_string(),
                    0.01,
                    None,
                );
            }

            let results = collector_ref.take_results();
            assert_eq!(results.len(), outcomes.len());

            for (i, (_, expected_outcome)) in outcomes.iter().enumerate() {
                assert_eq!(
                    results[i].outcome, *expected_outcome,
                    "Mismatch at index {}",
                    i
                );
            }
        });
    }

    #[test]
    fn test_embedded_executor_execution_mode() {
        if EmbeddedExecutor::is_available() {
            let executor = EmbeddedExecutor::new(None).unwrap();
            assert_eq!(executor.execution_mode(), "embedded");
        }
    }

    #[test]
    fn test_embedded_executor_configure() {
        use crate::executor::TestExecutor;

        if EmbeddedExecutor::is_available() {
            let mut executor = EmbeddedExecutor::new(None).unwrap();
            let config = ExecutorConfig {
                workers: Some(4),
                maxfail: Some(10),
                batch_size: 100,
                test_timeout_secs: 120,
                extra_args: vec!["--tb=long".to_string()],
            };
            // Use trait method which accepts ExecutorConfig
            TestExecutor::configure(&mut executor, config);
            // Configuration should not fail
            assert_eq!(executor.execution_mode(), "embedded");
        }
    }

    #[test]
    fn test_embedded_executor_kill_all() {
        if EmbeddedExecutor::is_available() {
            let executor = EmbeddedExecutor::new(None).unwrap();
            // Should not panic
            executor.kill_all();
        }
    }

    #[test]
    fn test_duration_conversion() {
        Python::with_gil(|py| {
            let collector = Py::new(py, RustResultCollector::new()).unwrap();
            let collector_ref = collector.borrow(py);

            // Test duration conversion (seconds to milliseconds)
            collector_ref.report(
                "test.py::test_1".to_string(),
                "passed".to_string(),
                1.5, // 1.5 seconds
                None,
            );
            collector_ref.report(
                "test.py::test_2".to_string(),
                "passed".to_string(),
                0.001, // 1 millisecond
                None,
            );

            let results = collector_ref.take_results();
            assert_eq!(results[0].duration_ms, 1500);
            assert_eq!(results[1].duration_ms, 1);
        });
    }

    #[test]
    fn test_embedded_executor_run_passing_test() {
        use std::fs;
        use tempfile::TempDir;

        if !EmbeddedExecutor::is_available() {
            return;
        }

        // Create a temporary test file
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_example.py");
        fs::write(&test_file, "def test_passing():\n    assert 1 + 1 == 2\n").unwrap();

        let executor = EmbeddedExecutor::new(None).unwrap();
        let node_id = format!("{}::test_passing", test_file.to_string_lossy());

        let result = executor.run_test(&node_id);
        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(matches!(result.outcome, TestOutcome::Passed));
    }

    #[test]
    fn test_embedded_executor_run_failing_test() {
        use std::fs;
        use tempfile::TempDir;

        if !EmbeddedExecutor::is_available() {
            return;
        }

        // Create a temporary test file with a failing test
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_fail.py");
        fs::write(&test_file, "def test_failing():\n    assert 1 == 2\n").unwrap();

        let executor = EmbeddedExecutor::new(None).unwrap();
        let node_id = format!("{}::test_failing", test_file.to_string_lossy());

        let result = executor.run_test(&node_id);
        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(matches!(result.outcome, TestOutcome::Failed));
    }

    #[test]
    fn test_embedded_executor_run_multiple_tests() {
        use std::fs;
        use tempfile::TempDir;

        if !EmbeddedExecutor::is_available() {
            return;
        }

        // Create a temporary test file with multiple tests
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_multi.py");
        fs::write(
            &test_file,
            "def test_one():\n    assert True\n\ndef test_two():\n    assert True\n\ndef test_three():\n    assert False\n",
        )
        .unwrap();

        let executor = EmbeddedExecutor::new(None).unwrap();
        let file_path = test_file.to_string_lossy();
        let node_ids = vec![
            format!("{}::test_one", file_path),
            format!("{}::test_two", file_path),
            format!("{}::test_three", file_path),
        ];

        let results = executor.run_tests(&node_ids);
        assert!(results.is_ok());
        let results = results.unwrap();
        assert_eq!(results.len(), 3);
        assert!(matches!(results[0].outcome, TestOutcome::Passed));
        assert!(matches!(results[1].outcome, TestOutcome::Passed));
        assert!(matches!(results[2].outcome, TestOutcome::Failed));
    }

    #[test]
    fn test_embedded_executor_run_empty() {
        if !EmbeddedExecutor::is_available() {
            return;
        }

        let executor = EmbeddedExecutor::new(None).unwrap();
        let results = executor.run_tests(&[]);
        assert!(results.is_ok());
        assert!(results.unwrap().is_empty());
    }

    #[test]
    fn test_embedded_executor_run_skipped_test() {
        use std::fs;
        use tempfile::TempDir;

        if !EmbeddedExecutor::is_available() {
            return;
        }

        // Create a temporary test file with a skipped test
        let dir = TempDir::new().unwrap();
        let test_file = dir.path().join("test_skip.py");
        fs::write(
            &test_file,
            "import pytest\n\n@pytest.mark.skip(reason='test skip')\ndef test_skipped():\n    assert True\n",
        )
        .unwrap();

        let executor = EmbeddedExecutor::new(None).unwrap();
        let node_id = format!("{}::test_skipped", test_file.to_string_lossy());

        let result = executor.run_test(&node_id);
        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(matches!(result.outcome, TestOutcome::Skipped));
    }
}
