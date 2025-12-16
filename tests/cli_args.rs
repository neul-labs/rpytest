//! Integration tests for CLI argument parsing.

use assert_cmd::Command;
use predicates::prelude::*;

fn rpytest() -> Command {
    Command::cargo_bin("rpytest").unwrap()
}

#[test]
fn help_shows_pytest_flags() {
    rpytest()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("-k"))
        .stdout(predicate::str::contains("-m"))
        .stdout(predicate::str::contains("--maxfail"))
        .stdout(predicate::str::contains("--collect-only"))
        .stdout(predicate::str::contains("--lf"))
        .stdout(predicate::str::contains("--ff"))
        .stdout(predicate::str::contains("--tb"))
        .stdout(predicate::str::contains("--rootdir"))
        .stdout(predicate::str::contains("--watch"))
        .stdout(predicate::str::contains("drop-in replacement for pytest"));
}

#[test]
fn version_flag() {
    rpytest()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("rpytest"))
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn short_version_flag() {
    rpytest()
        .arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::contains("rpytest"));
}

#[test]
fn collect_only_flag() {
    rpytest()
        .arg("--collect-only")
        .assert()
        .success()
        .stderr(predicate::str::contains("Collecting tests"));
}

#[test]
fn collect_only_alias() {
    rpytest()
        .arg("--co")
        .assert()
        .success()
        .stderr(predicate::str::contains("Collecting tests"));
}

#[test]
fn invalid_tb_style() {
    rpytest()
        .args(["--tb", "invalid"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid"));
}

#[test]
fn valid_tb_styles() {
    for style in ["auto", "long", "short", "line", "native", "no"] {
        rpytest()
            .args(["--tb", style, "--help"])
            .assert()
            .success();
    }
}

#[test]
fn keyword_filter() {
    rpytest()
        .args(["-k", "test_auth"])
        .assert()
        .success()
        .stderr(predicate::str::contains("-k test_auth"));
}

#[test]
fn marker_filter() {
    rpytest()
        .args(["-m", "slow"])
        .assert()
        .success()
        .stderr(predicate::str::contains("-m slow"));
}

#[test]
fn combined_flags() {
    rpytest()
        .args(["-k", "auth", "-m", "not slow", "-x", "--maxfail", "3", "-v"])
        .assert()
        .success();
}

#[test]
fn test_paths() {
    rpytest()
        .args(["tests/", "test_foo.py::test_bar"])
        .assert()
        .success()
        .stderr(predicate::str::contains("tests/"))
        .stderr(predicate::str::contains("test_foo.py::test_bar"));
}

#[test]
fn verbosity_flags() {
    rpytest()
        .args(["-v", "--help"])
        .assert()
        .success();

    rpytest()
        .args(["-vv", "--help"])
        .assert()
        .success();

    rpytest()
        .args(["-q", "--help"])
        .assert()
        .success();

    rpytest()
        .args(["-qq", "--help"])
        .assert()
        .success();
}

#[test]
fn workers_flag() {
    rpytest()
        .args(["--workers", "4"])
        .assert()
        .success();

    rpytest()
        .args(["-n", "auto"])
        .assert()
        .success();
}

#[test]
fn junitxml_flag() {
    rpytest()
        .args(["--junitxml", "report.xml"])
        .assert()
        .success();
}

#[test]
fn rootdir_flag() {
    rpytest()
        .args(["--rootdir", "/tmp"])
        .assert()
        .success();
}

#[test]
fn config_file_flag() {
    rpytest()
        .args(["-c", "custom_pytest.ini"])
        .assert()
        .success();
}

#[test]
fn last_failed_flags() {
    rpytest()
        .arg("--lf")
        .assert()
        .success();

    rpytest()
        .arg("--last-failed")
        .assert()
        .success();
}

#[test]
fn failed_first_flags() {
    rpytest()
        .arg("--ff")
        .assert()
        .success();

    rpytest()
        .arg("--failed-first")
        .assert()
        .success();
}

#[test]
fn ignore_flags() {
    rpytest()
        .args(["--ignore", "tests/slow", "--ignore", "tests/integration"])
        .assert()
        .success();
}

#[test]
fn override_ini_flag() {
    rpytest()
        .args(["-o", "addopts=-v"])
        .assert()
        .success();
}

#[test]
fn rpytest_extensions() {
    rpytest()
        .arg("--watch")
        .assert()
        .success();

    rpytest()
        .arg("--verify-dropin")
        .assert()
        .success()
        .stderr(predicate::str::contains("Drop-in"));

    rpytest()
        .arg("--inventory-status")
        .assert()
        .success();
}

#[test]
fn passthrough_unknown_flags() {
    rpytest()
        .args(["-k", "auth", "--", "--some-plugin-flag", "value"])
        .assert()
        .success()
        .stderr(predicate::str::contains("--some-plugin-flag"));
}
