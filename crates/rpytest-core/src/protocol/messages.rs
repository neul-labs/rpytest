//! Request and response types for the IPC protocol.

use serde::{Deserialize, Serialize};

/// Test node information returned from daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestNodeInfo {
    /// Unique node ID (pytest format).
    pub node_id: String,
    /// File path relative to repo root.
    pub file_path: String,
    /// Line number where test is defined.
    pub lineno: Option<u32>,
    /// Test function/method name.
    pub name: String,
    /// Parent class name (if method).
    pub class_name: Option<String>,
    /// Markers attached to this test.
    pub markers: Vec<String>,
    /// Whether test is marked as skip.
    pub skip: bool,
    /// Whether test is marked as xfail.
    pub xfail: bool,
}

/// Commands sent from CLI to daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Initialize a repository context within the daemon.
    InitContext {
        /// Absolute path to the repository root.
        repo_path: String,
        /// Optional path to Python interpreter.
        python_path: Option<String>,
    },

    /// Collect tests for a repository context.
    Collect {
        /// Context identifier returned from InitContext.
        context_id: String,
        /// Force full re-collection even if cache is valid.
        force: bool,
    },

    /// Run a set of tests.
    Run {
        /// Context identifier.
        context_id: String,
        /// List of test node IDs to run.
        node_ids: Vec<String>,
        /// Number of parallel workers (None = auto).
        workers: Option<u32>,
        /// Stop after N failures.
        maxfail: Option<u32>,
    },

    /// List tests matching filters (without running).
    List {
        /// Context identifier.
        context_id: String,
        /// Keyword expression filter.
        keyword: Option<String>,
        /// Marker expression filter.
        marker: Option<String>,
    },

    /// Get detailed inventory with full test metadata.
    GetInventory {
        /// Context identifier.
        context_id: String,
    },

    /// Get worker pool status.
    GetWorkerStatus {
        /// Context identifier.
        context_id: String,
    },

    /// Configure worker pool.
    ConfigureWorkers {
        /// Context identifier.
        context_id: String,
        /// Number of workers to maintain.
        num_workers: u32,
    },

    /// Shutdown the daemon or a specific context.
    Shutdown {
        /// If Some, shutdown only this context. If None, shutdown entire daemon.
        context_id: Option<String>,
    },

    /// Health check / ping.
    Ping,

    /// Start a streaming run (returns run_id, results come via GetRunProgress).
    RunStream {
        /// Context identifier.
        context_id: String,
        /// List of test node IDs to run.
        node_ids: Vec<String>,
        /// Number of parallel workers (None = auto).
        workers: Option<u32>,
        /// Stop after N failures.
        maxfail: Option<u32>,
    },

    /// Get progress and results from a streaming run.
    GetRunProgress {
        /// Context identifier.
        context_id: String,
        /// Run identifier from RunStream response.
        run_id: String,
    },

    // --- Phase 5: Flakiness ---

    /// Get flakiness report for all tracked tests.
    GetFlakinessReport {
        /// Context identifier.
        context_id: String,
    },

    /// Get flakiness info for a specific test.
    GetTestFlakiness {
        /// Context identifier.
        context_id: String,
        /// Test node ID.
        node_id: String,
    },

    /// Configure auto-rerun behavior.
    ConfigureRerun {
        /// Context identifier.
        context_id: String,
        /// Enable auto-rerun.
        enabled: bool,
        /// Maximum reruns per test.
        max_reruns: u32,
        /// Only rerun known flaky tests.
        only_flaky: bool,
        /// Delay between reruns in milliseconds.
        delay_ms: u32,
    },

    /// Get current rerun configuration.
    GetRerunConfig {
        /// Context identifier.
        context_id: String,
    },

    /// Run tests with auto-rerun enabled.
    RunWithRerun {
        /// Context identifier.
        context_id: String,
        /// List of test node IDs to run.
        node_ids: Vec<String>,
        /// Number of parallel workers (None = auto).
        workers: Option<u32>,
        /// Stop after N failures.
        maxfail: Option<u32>,
    },

    // --- Phase 5: Fixtures ---

    /// Configure session fixture reuse.
    ConfigureFixtureReuse {
        /// Context identifier.
        context_id: String,
        /// Enable fixture reuse.
        enabled: bool,
        /// Max fixture age in seconds.
        max_age_seconds: f64,
        /// Teardown on conftest.py changes.
        teardown_on_conftest_change: bool,
    },

    /// Get fixture configuration.
    GetFixtureConfig {
        /// Context identifier.
        context_id: String,
    },

    /// Get session status.
    GetSessionStatus {
        /// Context identifier.
        context_id: String,
    },

    // --- Phase 5: Sharding ---

    /// Get tests for a specific shard.
    GetShard {
        /// Context identifier.
        context_id: String,
        /// Tests to shard (empty = all inventory).
        node_ids: Vec<String>,
        /// This shard's index (0-based).
        shard_index: u32,
        /// Total number of shards.
        total_shards: u32,
        /// Sharding strategy.
        strategy: String,
    },

    /// Get sharding distribution info.
    GetShardInfo {
        /// Context identifier.
        context_id: String,
        /// Tests to shard (empty = all inventory).
        node_ids: Vec<String>,
        /// Total number of shards.
        total_shards: u32,
        /// Sharding strategy.
        strategy: String,
    },
}

/// Responses sent from daemon to CLI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Context successfully initialized.
    ContextReady {
        /// Unique context identifier.
        context_id: String,
        /// Hash of the current inventory for cache validation.
        inventory_hash: String,
    },

    /// Collection completed.
    CollectionComplete {
        /// Number of test nodes collected.
        node_count: usize,
        /// Collection duration in milliseconds.
        duration_ms: u64,
    },

    /// List of test node IDs matching the query.
    TestList {
        /// Matching node IDs.
        node_ids: Vec<String>,
    },

    /// Detailed inventory data.
    InventoryData {
        /// Inventory hash for cache validation.
        hash: String,
        /// Collection timestamp (Unix epoch ms).
        collected_at: u64,
        /// Test nodes with metadata.
        nodes: Vec<TestNodeInfo>,
    },

    /// Run completed.
    RunComplete {
        /// Total tests run.
        total: usize,
        /// Tests passed.
        passed: usize,
        /// Tests failed.
        failed: usize,
        /// Tests skipped.
        skipped: usize,
        /// Tests errored.
        errors: usize,
        /// Total duration in milliseconds.
        duration_ms: u64,
    },

    /// Worker pool status.
    WorkerStatus {
        /// Number of active workers.
        active_workers: u32,
        /// Number of idle workers.
        idle_workers: u32,
        /// Total tests executed by pool.
        tests_executed: u64,
        /// Average test duration in milliseconds.
        avg_test_duration_ms: u64,
    },

    /// Worker configuration acknowledged.
    WorkerConfigAck {
        /// New number of workers.
        num_workers: u32,
    },

    /// Shutdown acknowledged.
    ShutdownAck,

    /// Pong response to ping.
    Pong,

    /// Streaming run started.
    RunStarted {
        /// Unique run identifier for polling progress.
        run_id: String,
        /// Total tests to run.
        total_tests: usize,
    },

    /// Progress update with any completed test results.
    RunProgress {
        /// Run identifier.
        run_id: String,
        /// Total tests in this run.
        total: usize,
        /// Tests completed so far.
        completed: usize,
        /// Tests currently running.
        running: usize,
        /// Whether the run is complete.
        done: bool,
        /// Newly completed test results since last poll.
        results: Vec<TestResultInfo>,
    },

    /// Error response.
    Error {
        /// Error category.
        code: ErrorCode,
        /// Human-readable error message.
        message: String,
    },

    // --- Phase 5: Flakiness Responses ---

    /// Flakiness report for tracked tests.
    FlakinessReport {
        /// Tests identified as flaky.
        flaky_tests: Vec<FlakinessInfo>,
        /// Tests with some failures but not flaky.
        unstable_tests: Vec<FlakinessInfo>,
        /// Count of stable tests.
        stable_count: usize,
        /// Total tests tracked.
        total_tracked: usize,
    },

    /// Flakiness info for a single test.
    TestFlakiness {
        /// Test node ID.
        node_id: String,
        /// Failure rate (0.0-1.0).
        failure_rate: f64,
        /// Whether test is considered flaky.
        is_flaky: bool,
        /// Number of outcome flips.
        flaky_streak: u32,
        /// Consecutive failures.
        consecutive_failures: u32,
        /// Consecutive passes.
        consecutive_passes: u32,
        /// Total runs.
        total_runs: u32,
        /// Recent outcomes.
        recent_outcomes: Vec<String>,
    },

    /// Rerun configuration.
    RerunConfig {
        /// Whether enabled.
        enabled: bool,
        /// Max reruns per test.
        max_reruns: u32,
        /// Only rerun known flaky.
        only_flaky: bool,
        /// Delay between reruns ms.
        delay_ms: u32,
    },

    // --- Phase 5: Fixture Responses ---

    /// Fixture configuration.
    FixtureConfig {
        /// Whether enabled.
        enabled: bool,
        /// Max fixture age seconds.
        max_fixture_age_seconds: f64,
        /// Teardown on conftest change.
        teardown_on_conftest_change: bool,
        /// Teardown on test file change.
        teardown_on_test_file_change: bool,
        /// Scopes to reuse.
        scopes_to_reuse: Vec<String>,
    },

    /// Session status.
    SessionStatus {
        /// Session ID.
        session_id: String,
        /// Repo path.
        repo_path: String,
        /// Creation timestamp.
        created_at: f64,
        /// Last run timestamp.
        last_run_at: f64,
        /// Total runs.
        total_runs: u32,
        /// Whether enabled.
        enabled: bool,
    },

    // --- Phase 5: Sharding Responses ---

    /// Tests assigned to a shard.
    ShardedTests {
        /// Shard index.
        shard_index: u32,
        /// Total shards.
        total_shards: u32,
        /// Node IDs in this shard.
        node_ids: Vec<String>,
    },

    /// Sharding distribution info.
    ShardInfo {
        /// Strategy used.
        strategy: String,
        /// Total shards.
        total_shards: u32,
        /// Total tests.
        total_tests: usize,
        /// Test counts per shard.
        shard_test_counts: Vec<usize>,
        /// Duration estimates per shard.
        shard_durations_ms: Vec<u64>,
        /// Count imbalance percentage.
        count_imbalance_percent: f64,
        /// Duration imbalance percentage.
        duration_imbalance_percent: f64,
        /// Estimated wall time.
        estimated_wall_time_ms: u64,
    },

    /// Generic config acknowledgment.
    ConfigAck {
        /// Config type.
        config_type: String,
        /// The configuration.
        config: serde_json::Value,
    },
}

/// Individual test result info for streaming.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestResultInfo {
    /// Test node ID.
    pub node_id: String,
    /// Test outcome.
    pub outcome: String,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Optional failure message.
    pub message: Option<String>,
}

/// Flakiness info for a test.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlakinessInfo {
    /// Test node ID.
    pub node_id: String,
    /// Failure rate (0.0-1.0).
    pub failure_rate: f64,
    /// Number of outcome flips.
    pub flaky_streak: u32,
    /// Total runs.
    pub total_runs: u32,
    /// Consecutive failures.
    pub consecutive_failures: u32,
    /// Consecutive passes.
    pub consecutive_passes: u32,
}

/// Error codes for categorizing failures.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Context not found or not initialized.
    ContextNotFound,
    /// Collection failed (syntax error, import error, etc.).
    CollectionFailed,
    /// Invalid request parameters.
    InvalidRequest,
    /// Internal daemon error.
    InternalError,
    /// Operation timed out.
    Timeout,
    /// Python interpreter not found or invalid.
    PythonNotFound,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let requests = vec![
            Request::InitContext {
                repo_path: "/path/to/repo".to_string(),
                python_path: Some("/usr/bin/python3".to_string()),
            },
            Request::Collect {
                context_id: "ctx-123".to_string(),
                force: true,
            },
            Request::Run {
                context_id: "ctx-123".to_string(),
                node_ids: vec!["test_foo.py::test_bar".to_string()],
                workers: Some(4),
                maxfail: Some(1),
            },
            Request::List {
                context_id: "ctx-123".to_string(),
                keyword: Some("auth".to_string()),
                marker: None,
            },
            Request::GetInventory {
                context_id: "ctx-123".to_string(),
            },
            Request::Shutdown {
                context_id: Some("ctx-123".to_string()),
            },
            Request::Ping,
            Request::RunStream {
                context_id: "ctx-123".to_string(),
                node_ids: vec!["test_foo.py::test_bar".to_string()],
                workers: Some(4),
                maxfail: None,
            },
            Request::GetRunProgress {
                context_id: "ctx-123".to_string(),
                run_id: "run-123".to_string(),
            },
        ];

        for req in requests {
            let encoded = rmp_serde::to_vec(&req).unwrap();
            let decoded: Request = rmp_serde::from_slice(&encoded).unwrap();
            assert_eq!(req, decoded);
        }
    }

    #[test]
    fn response_roundtrip() {
        let responses = vec![
            Response::ContextReady {
                context_id: "ctx-123".to_string(),
                inventory_hash: "abc123".to_string(),
            },
            Response::CollectionComplete {
                node_count: 42,
                duration_ms: 150,
            },
            Response::TestList {
                node_ids: vec!["test_a".to_string(), "test_b".to_string()],
            },
            Response::InventoryData {
                hash: "abc123".to_string(),
                collected_at: 1234567890,
                nodes: vec![
                    TestNodeInfo {
                        node_id: "test.py::test_func".to_string(),
                        file_path: "test.py".to_string(),
                        lineno: Some(10),
                        name: "test_func".to_string(),
                        class_name: None,
                        markers: vec!["slow".to_string()],
                        skip: false,
                        xfail: false,
                    },
                ],
            },
            Response::RunComplete {
                total: 10,
                passed: 8,
                failed: 1,
                skipped: 1,
                errors: 0,
                duration_ms: 5000,
            },
            Response::ShutdownAck,
            Response::Pong,
            Response::RunStarted {
                run_id: "run-123".to_string(),
                total_tests: 10,
            },
            Response::RunProgress {
                run_id: "run-123".to_string(),
                total: 10,
                completed: 5,
                running: 2,
                done: false,
                results: vec![
                    TestResultInfo {
                        node_id: "test.py::test_foo".to_string(),
                        outcome: "passed".to_string(),
                        duration_ms: 100,
                        message: None,
                    },
                ],
            },
            Response::Error {
                code: ErrorCode::ContextNotFound,
                message: "Context not found".to_string(),
            },
        ];

        for resp in responses {
            let encoded = rmp_serde::to_vec(&resp).unwrap();
            let decoded: Response = rmp_serde::from_slice(&encoded).unwrap();
            assert_eq!(resp, decoded);
        }
    }
}
