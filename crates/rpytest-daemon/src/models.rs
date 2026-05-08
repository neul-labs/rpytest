//! Data models for the rpytest daemon.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Test outcome types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TestOutcome {
    Passed,
    Failed,
    Skipped,
    Error,
    Xfail,
    Xpass,
}

impl From<&str> for TestOutcome {
    fn from(s: &str) -> Self {
        match s {
            "passed" => TestOutcome::Passed,
            "failed" => TestOutcome::Failed,
            "skipped" => TestOutcome::Skipped,
            "error" => TestOutcome::Error,
            "xfail" => TestOutcome::Xfail,
            "xpass" => TestOutcome::Xpass,
            _ => TestOutcome::Error,
        }
    }
}

impl From<TestOutcome> for String {
    fn from(outcome: TestOutcome) -> Self {
        match outcome {
            TestOutcome::Passed => "passed".to_string(),
            TestOutcome::Failed => "failed".to_string(),
            TestOutcome::Skipped => "skipped".to_string(),
            TestOutcome::Error => "error".to_string(),
            TestOutcome::Xfail => "xfail".to_string(),
            TestOutcome::Xpass => "xpass".to_string(),
        }
    }
}

/// Stability states for a test based on its recent outcome history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum StabilityState {
    /// Not enough data to determine stability.
    #[default]
    Unknown,
    /// Consistently passing.
    Stable { consecutive_passes: u32 },
    /// Consistently failing.
    Unstable { consecutive_failures: u32 },
    /// Outcomes are alternating (at least one flip detected).
    Flaky { streak_count: u32 },
    /// Confirmed flaky after repeated alternations.
    ConfirmedFlaky,
}

impl StabilityState {
    /// Returns true if the test is considered flaky.
    ///
    /// Requires at least 2 flips (streak_count >= 2) in the Flaky state,
    /// or being in ConfirmedFlaky state.
    pub fn is_flaky(&self) -> bool {
        matches!(
            self,
            StabilityState::Flaky { streak_count: 2.. } | StabilityState::ConfirmedFlaky
        )
    }

    /// Returns true if the test is confirmed flaky.
    pub fn is_confirmed_flaky(&self) -> bool {
        matches!(self, StabilityState::ConfirmedFlaky)
    }

    /// Returns true if the test is stable (consistently passing).
    pub fn is_stable(&self) -> bool {
        matches!(self, StabilityState::Stable { .. })
    }
}

/// Represents a single test node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestNode {
    pub node_id: String,
    pub file_path: String,
    pub name: String,
    pub class_name: Option<String>,
    pub line_number: u32,
    pub markers: Vec<String>,
    pub skip: bool,
    pub xfail: bool,
}

impl From<rpytest_core::protocol::TestNodeInfo> for TestNode {
    fn from(info: rpytest_core::protocol::TestNodeInfo) -> Self {
        TestNode {
            node_id: info.node_id,
            file_path: info.file_path,
            name: info.name,
            class_name: info.class_name,
            line_number: info.lineno.unwrap_or(0),
            markers: info.markers,
            skip: info.skip,
            xfail: info.xfail,
        }
    }
}

impl From<TestNode> for rpytest_core::protocol::TestNodeInfo {
    fn from(node: TestNode) -> Self {
        rpytest_core::protocol::TestNodeInfo {
            node_id: node.node_id,
            file_path: node.file_path,
            lineno: Some(node.line_number),
            name: node.name,
            class_name: node.class_name,
            markers: node.markers,
            skip: node.skip,
            xfail: node.xfail,
        }
    }
}

/// Result of a single test execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestResult {
    pub node_id: String,
    pub outcome: TestOutcome,
    pub duration_ms: u64,
    pub message: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

/// Summary of a test run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub duration_ms: u64,
}

/// Native test node discovered via AST parsing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NativeTestNode {
    pub node_id: String,
    pub file_path: String,
    pub name: String,
    pub class_name: Option<String>,
    pub line_number: u32,
    pub markers: Vec<String>,
    pub is_simple: bool,
    pub parameters: Vec<Value>,
    pub skip: bool,
    pub skip_reason: Option<String>,
    pub xfail: bool,
    pub xfail_reason: Option<String>,
    pub xfail_strict: bool,
}

/// Information about a parametrized test variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParameterizedTestNode {
    /// Parameter names (e.g., ["x", "y"] for "x,y")
    pub param_names: Vec<String>,
    /// Parameter values, custom IDs, and per-variant marks for each variant.
    /// Each tuple is (values, custom_id, marks) where:
    /// - values: the parameter values
    /// - custom_id: custom ID from pytest.param(id="...") if provided
    /// - marks: per-variant marks from pytest.param(marks=...)
    pub param_values: Vec<(Vec<String>, Option<String>, Vec<String>)>,
    /// Generated test ID for a specific variant
    pub test_id: String,
}

/// Configuration for auto-rerun behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RerunConfig {
    pub enabled: bool,
    pub max_reruns: u32,
    pub only_flaky: bool,
    pub delay_ms: u32,
}

impl Default for RerunConfig {
    fn default() -> Self {
        RerunConfig {
            enabled: false,
            max_reruns: 2,
            only_flaky: false,
            delay_ms: 0,
        }
    }
}

/// Result of a rerun attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RerunResult {
    pub node_id: String,
    pub original_outcome: TestOutcome,
    pub rerun_outcomes: Vec<TestOutcome>,
    pub final_outcome: TestOutcome,
    pub is_flaky: bool,
    pub message: Option<String>,
}

/// Record of test flakiness over time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlakinessRecord {
    pub node_id: String,
    pub outcomes: Vec<String>, // Last N outcomes (stored as strings)
    pub consecutive_failures: u32,
    pub consecutive_passes: u32,
    pub flaky_streak: u32,
    pub total_runs: u32,
    pub last_failure_message: Option<String>,
    #[serde(default)]
    pub stability: StabilityState,
}

/// Fixture scope levels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FixtureScope {
    Session,
    Package,
    Module,
    Class,
    Function,
}

impl From<&str> for FixtureScope {
    fn from(s: &str) -> Self {
        match s {
            "session" => FixtureScope::Session,
            "package" => FixtureScope::Package,
            "module" => FixtureScope::Module,
            "class" => FixtureScope::Class,
            _ => FixtureScope::Function,
        }
    }
}

/// State of a session fixture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FixtureState {
    pub name: String,
    pub scope: FixtureScope,
    pub created_at: f64,
    pub last_used: f64,
    pub use_count: u32,
    pub teardown_pending: bool,
}

/// Fixture configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FixtureConfig {
    pub enabled: bool,
    pub max_age_seconds: f64,
    pub teardown_on_conftest_change: bool,
    pub scopes_to_reuse: Vec<String>,
}

/// A test with scheduling metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTest {
    pub node_id: String,
    pub estimated_duration_ms: u64,
    pub priority: u64,
}

/// Shard configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ShardConfig {
    pub shard_index: u32,
    pub total_shards: u32,
    pub strategy: String,
}

/// Executor configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ExecutorConfig {
    pub workers: Option<u32>,
    pub maxfail: Option<u32>,
    pub batch_size: usize,
}

/// Execution mode for running tests.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Use embedded Python via PyO3 (fastest, requires embedded-python feature)
    #[default]
    Embedded,
    /// Use subprocess execution (compatible with all Python environments)
    Subprocess,
    /// Use worker pool for parallel subprocess execution (best throughput)
    Pooled,
    /// Automatically select based on availability
    Auto,
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionMode::Embedded => write!(f, "embedded"),
            ExecutionMode::Subprocess => write!(f, "subprocess"),
            ExecutionMode::Pooled => write!(f, "pooled"),
            ExecutionMode::Auto => write!(f, "auto"),
        }
    }
}

impl std::str::FromStr for ExecutionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "embedded" => Ok(ExecutionMode::Embedded),
            "subprocess" => Ok(ExecutionMode::Subprocess),
            "pooled" => Ok(ExecutionMode::Pooled),
            "auto" => Ok(ExecutionMode::Auto),
            _ => Err(format!(
                "Invalid execution mode: {}. Use 'embedded', 'subprocess', 'pooled', or 'auto'",
                s
            )),
        }
    }
}

/// Daemon runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub socket_path: PathBuf,
    pub storage_path: PathBuf,
    pub python_path: Option<PathBuf>,
    pub idle_timeout_secs: u32,
    pub max_workers: u32,
    /// Execution mode for running tests
    pub execution_mode: ExecutionMode,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        DaemonConfig {
            socket_path: PathBuf::from("/tmp/rpytest.sock"),
            storage_path: dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("rpytest"),
            python_path: None,
            idle_timeout_secs: 0,
            max_workers: 4,
            execution_mode: ExecutionMode::Auto,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_mode_from_str() {
        assert_eq!(
            "embedded".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Embedded
        );
        assert_eq!(
            "subprocess".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Subprocess
        );
        assert_eq!(
            "auto".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Auto
        );

        // Case insensitive
        assert_eq!(
            "EMBEDDED".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Embedded
        );
        assert_eq!(
            "Auto".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Auto
        );

        // Invalid
        assert!("invalid".parse::<ExecutionMode>().is_err());
    }

    #[test]
    fn test_execution_mode_display() {
        assert_eq!(ExecutionMode::Embedded.to_string(), "embedded");
        assert_eq!(ExecutionMode::Subprocess.to_string(), "subprocess");
        assert_eq!(ExecutionMode::Auto.to_string(), "auto");
    }

    #[test]
    fn test_execution_mode_default() {
        assert_eq!(ExecutionMode::default(), ExecutionMode::Embedded);
    }

    #[test]
    fn test_daemon_config_default() {
        let config = DaemonConfig::default();
        assert_eq!(config.execution_mode, ExecutionMode::Auto);
        assert_eq!(config.max_workers, 4);
    }

    #[test]
    fn test_test_outcome_conversions() {
        assert_eq!(TestOutcome::from("passed"), TestOutcome::Passed);
        assert_eq!(TestOutcome::from("failed"), TestOutcome::Failed);
        assert_eq!(TestOutcome::from("skipped"), TestOutcome::Skipped);
        assert_eq!(TestOutcome::from("error"), TestOutcome::Error);
        assert_eq!(TestOutcome::from("xfail"), TestOutcome::Xfail);
        assert_eq!(TestOutcome::from("xpass"), TestOutcome::Xpass);
        assert_eq!(TestOutcome::from("unknown"), TestOutcome::Error);

        assert_eq!(String::from(TestOutcome::Passed), "passed");
        assert_eq!(String::from(TestOutcome::Failed), "failed");
    }
}
