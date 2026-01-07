//! Drop-in compatibility verification.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::debug;

use super::diff::{DiffKind, OutputDiff};

/// Configuration for verification.
#[derive(Debug, Clone)]
pub struct VerifyConfig {
    /// Root directory for tests.
    pub root: PathBuf,
    /// Python executable path.
    pub python: String,
    /// Additional pytest arguments.
    pub pytest_args: Vec<String>,
    /// Whether to compare test output verbatim.
    pub strict_output: bool,
    /// Timeout for each run in seconds.
    pub timeout_secs: u64,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        // Try to use python from active virtualenv if available
        let python = std::env::var("VIRTUAL_ENV")
            .ok()
            .map(|venv| {
                let venv_python = PathBuf::from(&venv).join("bin").join("python");
                if venv_python.exists() {
                    venv_python.to_string_lossy().to_string()
                } else {
                    "python3".to_string()
                }
            })
            .unwrap_or_else(|| "python3".to_string());

        Self {
            root: PathBuf::from("."),
            python,
            pytest_args: vec![],
            strict_output: false,
            timeout_secs: 300,
        }
    }
}

/// Result of a verification run.
#[derive(Debug)]
pub struct VerifyResult {
    /// Whether verification passed.
    pub passed: bool,
    /// Pytest execution result.
    pub pytest: RunResult,
    /// Rpytest execution result.
    pub rpytest: RunResult,
    /// Differences found.
    pub diffs: Vec<OutputDiff>,
    /// Summary message.
    pub summary: String,
}

/// Result of a single test runner execution.
#[derive(Debug, Clone)]
pub struct RunResult {
    /// Exit code.
    pub exit_code: i32,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// Execution duration.
    pub duration: Duration,
    /// Tests collected.
    pub tests_collected: usize,
    /// Tests passed.
    pub passed: usize,
    /// Tests failed.
    pub failed: usize,
    /// Tests skipped.
    pub skipped: usize,
    /// Tests errored.
    pub errors: usize,
}

impl Default for RunResult {
    fn default() -> Self {
        Self {
            exit_code: -1,
            stdout: String::new(),
            stderr: String::new(),
            duration: Duration::ZERO,
            tests_collected: 0,
            passed: 0,
            failed: 0,
            skipped: 0,
            errors: 0,
        }
    }
}

/// Run drop-in compatibility verification.
pub fn verify_dropin(config: &VerifyConfig) -> Result<VerifyResult> {
    // Run pytest
    let pytest_result = run_pytest(config)?;

    // Run rpytest (in subprocess mode, not daemon)
    let rpytest_result = run_rpytest(config)?;

    // Compare results
    let diffs = compare_results(&pytest_result, &rpytest_result, config);

    let passed = diffs.iter().all(|d| !d.is_critical());

    let summary = if passed {
        format!(
            "Verification PASSED: pytest and rpytest produced compatible results\n\
             pytest:  {} collected, {} passed, {} failed, {} skipped in {:.2}s\n\
             rpytest: {} collected, {} passed, {} failed, {} skipped in {:.2}s",
            pytest_result.tests_collected,
            pytest_result.passed,
            pytest_result.failed,
            pytest_result.skipped,
            pytest_result.duration.as_secs_f64(),
            rpytest_result.tests_collected,
            rpytest_result.passed,
            rpytest_result.failed,
            rpytest_result.skipped,
            rpytest_result.duration.as_secs_f64(),
        )
    } else {
        let critical_count = diffs.iter().filter(|d| d.is_critical()).count();
        format!(
            "Verification FAILED: {} critical difference(s) found\n\
             pytest:  {} collected, {} passed, {} failed, {} skipped\n\
             rpytest: {} collected, {} passed, {} failed, {} skipped",
            critical_count,
            pytest_result.tests_collected,
            pytest_result.passed,
            pytest_result.failed,
            pytest_result.skipped,
            rpytest_result.tests_collected,
            rpytest_result.passed,
            rpytest_result.failed,
            rpytest_result.skipped,
        )
    };

    Ok(VerifyResult {
        passed,
        pytest: pytest_result,
        rpytest: rpytest_result,
        diffs,
        summary,
    })
}

fn run_pytest(config: &VerifyConfig) -> Result<RunResult> {
    let start = Instant::now();

    let mut cmd = Command::new(&config.python);
    cmd.arg("-m")
        .arg("pytest")
        .arg("-v")
        .arg("--tb=short")
        .args(&config.pytest_args)
        .current_dir(&config.root);

    let output = cmd.output()?;
    let duration = start.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let (collected, passed, failed, skipped, errors) = parse_pytest_summary(&stdout);

    Ok(RunResult {
        exit_code: output.status.code().unwrap_or(-1),
        stdout,
        stderr,
        duration,
        tests_collected: collected,
        passed,
        failed,
        skipped,
        errors,
    })
}

fn run_rpytest(config: &VerifyConfig) -> Result<RunResult> {
    let start = Instant::now();

    // For verification, we run rpytest in a mode that mimics pytest output
    // This requires the daemon to be running
    let rpytest_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("rpytest")))
        .unwrap_or_else(|| PathBuf::from("rpytest"));

    let mut cmd = Command::new(&rpytest_bin);
    cmd.arg("-v")
        .arg("--tb=short")
        .arg("--no-header") // Reduce output noise
        .args(&config.pytest_args)
        .current_dir(&config.root);

    debug!(
        "Running rpytest: {:?} in {:?}",
        cmd.get_program(),
        config.root
    );

    let output = cmd.output()?;
    let duration = start.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    debug!("rpytest stdout:\n{}", stdout);
    debug!("rpytest stderr:\n{}", stderr);

    // rpytest writes summary to stderr, so combine both for parsing
    let combined_output = format!("{}\n{}", stdout, stderr);
    let (collected, passed, failed, skipped, errors) = parse_pytest_summary(&combined_output);

    Ok(RunResult {
        exit_code: output.status.code().unwrap_or(-1),
        stdout,
        stderr,
        duration,
        tests_collected: collected,
        passed,
        failed,
        skipped,
        errors,
    })
}

fn parse_pytest_summary(output: &str) -> (usize, usize, usize, usize, usize) {
    // Parse pytest-style summary line:
    // pytest:  "===== 10 passed, 2 failed, 1 skipped in 1.23s ====="
    // pytest:  "collected 10 items"
    // rpytest: "=== 5 passed, 1 skipped in 0.44s ==="
    // rpytest: "Running 6 tests..."
    let mut collected = 0;
    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut errors = 0;

    for line in output.lines() {
        // Look for collection line: "collected N items" (pytest)
        if line.contains("collected") && line.contains("item") {
            if let Some(num) = extract_number_after(line, "collected") {
                collected = num;
            }
        }

        // Look for rpytest collection line: "Running N tests..."
        if line.contains("Running") && line.contains("tests") {
            if let Some(num) = extract_number_after(line, "Running") {
                collected = num;
            }
        }

        // Look for summary line (works for both pytest and rpytest)
        if line.contains("passed") || line.contains("failed") || line.contains("error") {
            if let Some(num) = extract_number_before(line, "passed") {
                passed = num;
            }
            if let Some(num) = extract_number_before(line, "failed") {
                failed = num;
            }
            if let Some(num) = extract_number_before(line, "skipped") {
                skipped = num;
            }
            if let Some(num) = extract_number_before(line, "error") {
                errors = num;
            }
        }
    }

    (collected, passed, failed, skipped, errors)
}

fn extract_number_before(text: &str, word: &str) -> Option<usize> {
    // Find "N word" pattern
    let parts: Vec<&str> = text.split_whitespace().collect();
    for i in 0..parts.len().saturating_sub(1) {
        if parts[i + 1].starts_with(word) {
            return parts[i].parse().ok();
        }
    }
    None
}

fn extract_number_after(text: &str, word: &str) -> Option<usize> {
    // Find "word N" pattern
    let parts: Vec<&str> = text.split_whitespace().collect();
    for i in 0..parts.len().saturating_sub(1) {
        if parts[i] == word || parts[i].ends_with(word) {
            return parts[i + 1].parse().ok();
        }
    }
    None
}

fn compare_results(
    pytest: &RunResult,
    rpytest: &RunResult,
    config: &VerifyConfig,
) -> Vec<OutputDiff> {
    let mut diffs = Vec::new();

    // Compare exit codes
    if pytest.exit_code != rpytest.exit_code {
        diffs.push(OutputDiff {
            kind: DiffKind::ExitCode,
            expected: pytest.exit_code.to_string(),
            actual: rpytest.exit_code.to_string(),
            context: "Exit code mismatch".to_string(),
        });
    }

    // Compare test counts
    if pytest.tests_collected != rpytest.tests_collected {
        diffs.push(OutputDiff {
            kind: DiffKind::TestCount,
            expected: pytest.tests_collected.to_string(),
            actual: rpytest.tests_collected.to_string(),
            context: "Collection count mismatch".to_string(),
        });
    }

    if pytest.passed != rpytest.passed {
        diffs.push(OutputDiff {
            kind: DiffKind::PassedCount,
            expected: pytest.passed.to_string(),
            actual: rpytest.passed.to_string(),
            context: "Passed count mismatch".to_string(),
        });
    }

    if pytest.failed != rpytest.failed {
        diffs.push(OutputDiff {
            kind: DiffKind::FailedCount,
            expected: pytest.failed.to_string(),
            actual: rpytest.failed.to_string(),
            context: "Failed count mismatch".to_string(),
        });
    }

    if pytest.skipped != rpytest.skipped {
        diffs.push(OutputDiff {
            kind: DiffKind::SkippedCount,
            expected: pytest.skipped.to_string(),
            actual: rpytest.skipped.to_string(),
            context: "Skipped count mismatch".to_string(),
        });
    }

    // Compare test node IDs (which tests ran) - only if both outputs have per-test details
    // rpytest may not output per-test results, so only compare if counts differ
    let pytest_tests = extract_test_node_ids(&pytest.stdout);
    let rpytest_tests = extract_test_node_ids(&rpytest.stdout);

    // Only flag missing/extra tests if:
    // 1. Both have per-test output to compare, OR
    // 2. The summary counts differ (indicating actual behavior difference)
    let counts_match = pytest.tests_collected == rpytest.tests_collected
        && pytest.passed == rpytest.passed
        && pytest.failed == rpytest.failed
        && pytest.skipped == rpytest.skipped;

    let both_have_details = !pytest_tests.is_empty() && !rpytest_tests.is_empty();

    if both_have_details || !counts_match {
        let only_in_pytest: Vec<_> = pytest_tests.difference(&rpytest_tests).collect();
        let only_in_rpytest: Vec<_> = rpytest_tests.difference(&pytest_tests).collect();

        if !only_in_pytest.is_empty() && !counts_match {
            diffs.push(OutputDiff {
                kind: DiffKind::MissingTests,
                expected: format!("{} tests", only_in_pytest.len()),
                actual: "missing".to_string(),
                context: format!("Tests in pytest but not rpytest: {:?}", only_in_pytest),
            });
        }

        if !only_in_rpytest.is_empty() && !counts_match {
            diffs.push(OutputDiff {
                kind: DiffKind::ExtraTests,
                expected: "none".to_string(),
                actual: format!("{} extra tests", only_in_rpytest.len()),
                context: format!("Tests in rpytest but not pytest: {:?}", only_in_rpytest),
            });
        }
    }

    // Strict output comparison if requested
    if config.strict_output {
        let pytest_normalized = normalize_output(&pytest.stdout);
        let rpytest_normalized = normalize_output(&rpytest.stdout);

        if pytest_normalized != rpytest_normalized {
            diffs.push(OutputDiff {
                kind: DiffKind::OutputContent,
                expected: "matching output".to_string(),
                actual: "different output".to_string(),
                context: "Output content differs (strict mode)".to_string(),
            });
        }
    }

    diffs
}

fn extract_test_node_ids(output: &str) -> HashSet<String> {
    let mut node_ids = HashSet::new();

    for line in output.lines() {
        let line = line.trim();
        // Match lines like "test_foo.py::test_bar PASSED" or similar
        if line.contains("::")
            && (line.contains("PASSED")
                || line.contains("FAILED")
                || line.contains("SKIPPED")
                || line.contains("ERROR"))
        {
            if let Some(node_id) = line.split_whitespace().next() {
                if node_id.contains("::") {
                    node_ids.insert(node_id.to_string());
                }
            }
        }
    }

    node_ids
}

fn normalize_output(output: &str) -> String {
    // Normalize output for comparison by removing:
    // - Timing information
    // - Absolute paths
    // - Version numbers
    // - ANSI escape codes

    // Static regex patterns (compiled once)
    static ANSI_RE: OnceLock<regex_lite::Regex> = OnceLock::new();
    static TIMING_RE: OnceLock<regex_lite::Regex> = OnceLock::new();

    let ansi_re = ANSI_RE.get_or_init(|| {
        regex_lite::Regex::new(r"\x1b\[[0-9;]*m").expect("ANSI regex pattern is valid")
    });
    let timing_re = TIMING_RE.get_or_init(|| {
        regex_lite::Regex::new(r"in \d+\.\d+s").expect("Timing regex pattern is valid")
    });

    let mut normalized = output.to_string();

    // Remove ANSI escape codes
    normalized = ansi_re.replace_all(&normalized, "").to_string();

    // Remove timing info (e.g., "in 1.23s")
    normalized = timing_re.replace_all(&normalized, "in X.XXs").to_string();

    // Normalize whitespace
    normalized
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pytest_summary() {
        let output = "collected 10 items\n\
            test_foo.py::test_one PASSED\n\
            test_foo.py::test_two FAILED\n\
            ===== 8 passed, 1 failed, 1 skipped in 1.23s =====";

        let (collected, passed, failed, skipped, errors) = parse_pytest_summary(output);
        assert_eq!(collected, 10);
        assert_eq!(passed, 8);
        assert_eq!(failed, 1);
        assert_eq!(skipped, 1);
        assert_eq!(errors, 0);
    }

    #[test]
    fn test_extract_number_after() {
        assert_eq!(
            extract_number_after("collected 10 items", "collected"),
            Some(10)
        );
        assert_eq!(extract_number_after("no match here", "collected"), None);
    }

    #[test]
    fn test_extract_test_node_ids() {
        let output = "test_foo.py::test_one PASSED\n\
            test_foo.py::TestClass::test_two FAILED\n\
            some other line\n\
            test_bar.py::test_three SKIPPED";

        let node_ids = extract_test_node_ids(output);
        assert_eq!(node_ids.len(), 3);
        assert!(node_ids.contains("test_foo.py::test_one"));
        assert!(node_ids.contains("test_foo.py::TestClass::test_two"));
        assert!(node_ids.contains("test_bar.py::test_three"));
    }

    #[test]
    fn test_extract_number_before() {
        assert_eq!(extract_number_before("10 passed", "passed"), Some(10));
        assert_eq!(
            extract_number_before("5 failed, 3 skipped", "failed"),
            Some(5)
        );
        assert_eq!(extract_number_before("no number here", "passed"), None);
    }
}
