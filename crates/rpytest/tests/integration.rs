//! Integration tests for rpytest CLI.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Create a temporary test project with a simple test file.
fn create_test_project() -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create a simple test file
    let test_content = r#"
def test_pass():
    """A passing test."""
    assert True

def test_also_pass():
    """Another passing test."""
    assert 1 + 1 == 2
"#;

    fs::write(temp.path().join("test_simple.py"), test_content).expect("Failed to write test file");

    temp
}

#[test]
fn test_version_flag() {
    let mut cmd = Command::cargo_bin("rpytest").unwrap();
    cmd.arg("--version");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("rpytest"));
}

#[test]
fn test_help_flag() {
    let mut cmd = Command::cargo_bin("rpytest").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("drop-in replacement for pytest"));
}

#[test]
fn test_verbose_flag() {
    let mut cmd = Command::cargo_bin("rpytest").unwrap();
    cmd.args(["--help", "-v"]);
    cmd.assert().success();
}

// Note: Full integration tests that require the Python daemon
// are best run manually with the daemon running.
// These tests verify CLI parsing and basic functionality.

#[test]
fn test_collect_only_flag_parsing() {
    // Just verify the flag is parsed correctly (doesn't require daemon)
    let mut cmd = Command::cargo_bin("rpytest").unwrap();
    cmd.args(["--collect-only", "--help"]);
    cmd.assert().success();
}

#[test]
fn test_rootdir_flag_parsing() {
    let temp = create_test_project();

    // Just verify the flag is parsed correctly
    let mut cmd = Command::cargo_bin("rpytest").unwrap();
    cmd.args(["--rootdir", temp.path().to_str().unwrap(), "--help"]);
    cmd.assert().success();
}

#[test]
fn test_keyword_and_marker_flags() {
    let mut cmd = Command::cargo_bin("rpytest").unwrap();
    cmd.args(["-k", "test_pass", "-m", "slow", "--help"]);
    cmd.assert().success();
}

#[test]
fn test_output_format_flags() {
    let mut cmd = Command::cargo_bin("rpytest").unwrap();
    cmd.args(["--tb", "short", "-q", "--no-header", "--help"]);
    cmd.assert().success();
}

#[test]
fn test_worker_flags() {
    let mut cmd = Command::cargo_bin("rpytest").unwrap();
    cmd.args(["-n", "4", "--help"]);
    cmd.assert().success();
}

#[test]
fn test_failed_first_flags() {
    let mut cmd = Command::cargo_bin("rpytest").unwrap();
    cmd.args(["--ff", "--lf", "--help"]);
    cmd.assert().success();
}

// Integration test that requires daemon - marked as ignored by default
#[test]
#[ignore = "Requires daemon to be running"]
fn test_collect_only_with_daemon() {
    let temp = create_test_project();

    let mut cmd = Command::cargo_bin("rpytest").unwrap();
    cmd.args(["--collect-only", "--rootdir", temp.path().to_str().unwrap()]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("test_pass"))
        .stdout(predicate::str::contains("test_also_pass"));
}

#[test]
#[ignore = "Requires daemon to be running"]
fn test_run_with_daemon() {
    let temp = create_test_project();

    let mut cmd = Command::cargo_bin("rpytest").unwrap();
    cmd.args(["--rootdir", temp.path().to_str().unwrap()]);

    // Output goes to stderr (UI output)
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("passed"));
}
