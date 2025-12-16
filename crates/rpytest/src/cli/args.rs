//! Command-line argument definitions with pytest parity.

use std::path::PathBuf;

use clap::{ArgAction, Parser};

use crate::config::Config;

/// Rust-powered, drop-in replacement for pytest.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "rpytest",
    version,
    about = "Rust-powered, drop-in replacement for pytest",
    after_help = "Use -- to pass unknown flags through to the pytest daemon for plugin compatibility.",
    disable_version_flag = true
)]
pub struct Cli {
    // === Test Selection ===
    /// Test paths, modules, or node IDs to run.
    #[arg(value_name = "PATH")]
    pub paths: Vec<String>,

    /// Only run tests matching the given keyword expression.
    /// Example: -k "auth and not slow"
    #[arg(short = 'k', long = "keyword", value_name = "EXPR")]
    pub keyword: Option<String>,

    /// Only run tests matching the given marker expression.
    /// Example: -m "not slow"
    #[arg(short = 'm', long = "marker", value_name = "EXPR")]
    pub marker: Option<String>,

    // === Execution Control ===
    /// Exit instantly on first error or failed test.
    #[arg(short = 'x', long = "exitfirst")]
    pub exitfirst: bool,

    /// Exit after first N failures or errors.
    #[arg(long = "maxfail", value_name = "N")]
    pub maxfail: Option<u32>,

    /// Run the last failed tests first.
    #[arg(long = "ff", visible_alias = "failed-first")]
    pub failed_first: bool,

    /// Run only the tests that failed in the last run.
    #[arg(long = "lf", visible_alias = "last-failed")]
    pub last_failed: bool,

    /// Run new tests first, then the rest.
    #[arg(long = "nf", visible_alias = "new-first")]
    pub new_first: bool,

    /// Number of parallel workers. Use "auto" for automatic detection.
    #[arg(long = "workers", short = 'n', value_name = "N")]
    pub workers: Option<String>,

    // === Output Control ===
    /// Increase verbosity (-v, -vv, -vvv).
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbose: u8,

    /// Decrease verbosity (-q, -qq).
    #[arg(short = 'q', long = "quiet", action = ArgAction::Count)]
    pub quiet: u8,

    /// Traceback print mode (auto/long/short/line/native/no).
    #[arg(
        long = "tb",
        value_name = "STYLE",
        value_parser = ["auto", "long", "short", "line", "native", "no"]
    )]
    pub tb: Option<String>,

    /// Don't show the pytest header.
    #[arg(long = "no-header")]
    pub no_header: bool,

    /// Show local variables in tracebacks.
    #[arg(short = 'l', long = "showlocals")]
    pub showlocals: bool,

    /// Summary report: (f)ailed, (E)rror, (s)kipped, (x)failed, (X)passed, (p)assed, (a)ll.
    #[arg(short = 'r', value_name = "CHARS")]
    pub report_summary: Option<String>,

    // === Collection ===
    /// Only collect tests, don't execute them.
    #[arg(long = "collect-only", visible_alias = "co")]
    pub collect_only: bool,

    /// Ignore paths during collection.
    #[arg(long = "ignore", value_name = "PATH")]
    pub ignore: Vec<PathBuf>,

    /// Ignore paths matching glob pattern.
    #[arg(long = "ignore-glob", value_name = "PATTERN")]
    pub ignore_glob: Vec<String>,

    // === Configuration ===
    /// Load configuration from this file.
    #[arg(short = 'c', long = "config-file", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Set the root directory for test discovery.
    #[arg(long = "rootdir", value_name = "DIR")]
    pub rootdir: Option<PathBuf>,

    /// Override ini option: "option=value".
    #[arg(short = 'o', long = "override-ini", value_name = "OPTION=VALUE")]
    pub override_ini: Vec<String>,

    // === Plugins ===
    /// Early-load a plugin module.
    #[arg(short = 'p', value_name = "PLUGIN")]
    pub plugins: Vec<String>,

    // === Output Formats ===
    /// Create JUnit XML report at the given path.
    #[arg(long = "junitxml", value_name = "PATH")]
    pub junitxml: Option<PathBuf>,

    /// Show N slowest test durations (0 for all).
    #[arg(long = "durations", value_name = "N")]
    pub durations: Option<u32>,

    /// Minimum duration in seconds to include in slowest list.
    #[arg(long = "durations-min", value_name = "SECS")]
    pub durations_min: Option<f64>,

    // === Capture ===
    /// Per-test capturing method (fd/sys/no/tee-sys).
    #[arg(long = "capture", short = 's', value_name = "METHOD")]
    pub capture: Option<String>,

    // === Debugging ===
    /// Start the interactive Python debugger on errors.
    #[arg(long = "pdb")]
    pub pdb: bool,

    /// Start debugger on test start.
    #[arg(long = "trace")]
    pub trace: bool,

    // === rpytest Extensions ===
    /// Watch mode: re-run tests on file changes.
    #[arg(long = "watch")]
    pub watch: bool,

    /// Verify drop-in compatibility with pytest.
    #[arg(long = "verify-dropin")]
    pub verify_dropin: bool,

    /// Show inventory status.
    #[arg(long = "inventory-status")]
    pub inventory_status: bool,

    /// Show rpytest version.
    #[arg(long = "version", short = 'V')]
    pub version: bool,

    // === Passthrough ===
    /// Unknown flags passed through to the daemon/plugins (use -- before them).
    #[arg(last = true, allow_hyphen_values = true)]
    pub passthrough: Vec<String>,
}

impl Cli {
    /// Merge CLI arguments with configuration file settings.
    /// CLI arguments take precedence over config.
    pub fn merge_with_config(&self, config: &Config) -> Self {
        let mut merged = self.clone();

        // Add testpaths from config if no paths specified on CLI
        if merged.paths.is_empty() && !config.testpaths.is_empty() {
            merged.paths = config
                .testpaths
                .iter()
                .map(|p| p.display().to_string())
                .collect();
        }

        // Apply addopts from config (these go to passthrough)
        // Note: In a full implementation, we'd parse addopts and apply
        // them to the appropriate fields. For now, just add to passthrough.
        for opt in &config.addopts {
            if !merged.passthrough.contains(opt) {
                merged.passthrough.push(opt.clone());
            }
        }

        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_args() {
        let cli = Cli::try_parse_from(["rpytest"]).unwrap();
        assert!(cli.paths.is_empty());
        assert!(!cli.exitfirst);
    }

    #[test]
    fn parse_keyword_filter() {
        let cli = Cli::try_parse_from(["rpytest", "-k", "test_auth"]).unwrap();
        assert_eq!(cli.keyword, Some("test_auth".to_string()));
    }

    #[test]
    fn parse_marker_filter() {
        let cli = Cli::try_parse_from(["rpytest", "-m", "slow"]).unwrap();
        assert_eq!(cli.marker, Some("slow".to_string()));
    }

    #[test]
    fn parse_combined_flags() {
        let cli = Cli::try_parse_from([
            "rpytest",
            "-k",
            "auth",
            "-m",
            "not slow",
            "-x",
            "--maxfail",
            "3",
            "-vvv",
        ])
        .unwrap();

        assert_eq!(cli.keyword, Some("auth".to_string()));
        assert_eq!(cli.marker, Some("not slow".to_string()));
        assert!(cli.exitfirst);
        assert_eq!(cli.maxfail, Some(3));
        assert_eq!(cli.verbose, 3);
    }

    #[test]
    fn parse_paths() {
        let cli =
            Cli::try_parse_from(["rpytest", "tests/", "test_foo.py::test_bar"]).unwrap();
        assert_eq!(cli.paths, vec!["tests/", "test_foo.py::test_bar"]);
    }

    #[test]
    fn parse_collect_only() {
        let cli = Cli::try_parse_from(["rpytest", "--collect-only"]).unwrap();
        assert!(cli.collect_only);

        let cli = Cli::try_parse_from(["rpytest", "--co"]).unwrap();
        assert!(cli.collect_only);
    }

    #[test]
    fn parse_last_failed() {
        let cli = Cli::try_parse_from(["rpytest", "--lf"]).unwrap();
        assert!(cli.last_failed);

        let cli = Cli::try_parse_from(["rpytest", "--last-failed"]).unwrap();
        assert!(cli.last_failed);
    }

    #[test]
    fn parse_failed_first() {
        let cli = Cli::try_parse_from(["rpytest", "--ff"]).unwrap();
        assert!(cli.failed_first);

        let cli = Cli::try_parse_from(["rpytest", "--failed-first"]).unwrap();
        assert!(cli.failed_first);
    }

    #[test]
    fn parse_verbosity() {
        let cli = Cli::try_parse_from(["rpytest", "-v"]).unwrap();
        assert_eq!(cli.verbose, 1);

        let cli = Cli::try_parse_from(["rpytest", "-vv"]).unwrap();
        assert_eq!(cli.verbose, 2);

        let cli = Cli::try_parse_from(["rpytest", "-q"]).unwrap();
        assert_eq!(cli.quiet, 1);

        let cli = Cli::try_parse_from(["rpytest", "-qq"]).unwrap();
        assert_eq!(cli.quiet, 2);
    }

    #[test]
    fn parse_tb_style() {
        let cli = Cli::try_parse_from(["rpytest", "--tb=short"]).unwrap();
        assert_eq!(cli.tb, Some("short".to_string()));
    }

    #[test]
    fn parse_junitxml() {
        let cli = Cli::try_parse_from(["rpytest", "--junitxml", "report.xml"]).unwrap();
        assert_eq!(cli.junitxml, Some(PathBuf::from("report.xml")));
    }

    #[test]
    fn parse_passthrough() {
        let cli = Cli::try_parse_from([
            "rpytest",
            "-k",
            "auth",
            "--",
            "--some-plugin-flag",
            "value",
        ])
        .unwrap();

        assert_eq!(cli.keyword, Some("auth".to_string()));
        assert_eq!(cli.passthrough, vec!["--some-plugin-flag", "value"]);
    }

    #[test]
    fn parse_rootdir() {
        let cli = Cli::try_parse_from(["rpytest", "--rootdir", "/path/to/project"]).unwrap();
        assert_eq!(cli.rootdir, Some(PathBuf::from("/path/to/project")));
    }

    #[test]
    fn parse_config_file() {
        let cli = Cli::try_parse_from(["rpytest", "-c", "custom_pytest.ini"]).unwrap();
        assert_eq!(cli.config, Some(PathBuf::from("custom_pytest.ini")));
    }

    #[test]
    fn parse_ignore() {
        let cli = Cli::try_parse_from([
            "rpytest",
            "--ignore",
            "tests/slow",
            "--ignore",
            "tests/integration",
        ])
        .unwrap();

        assert_eq!(
            cli.ignore,
            vec![PathBuf::from("tests/slow"), PathBuf::from("tests/integration")]
        );
    }

    #[test]
    fn parse_workers() {
        let cli = Cli::try_parse_from(["rpytest", "--workers", "4"]).unwrap();
        assert_eq!(cli.workers, Some("4".to_string()));

        let cli = Cli::try_parse_from(["rpytest", "-n", "auto"]).unwrap();
        assert_eq!(cli.workers, Some("auto".to_string()));
    }

    #[test]
    fn parse_rpytest_extensions() {
        let cli = Cli::try_parse_from(["rpytest", "--watch"]).unwrap();
        assert!(cli.watch);

        let cli = Cli::try_parse_from(["rpytest", "--verify-dropin"]).unwrap();
        assert!(cli.verify_dropin);
    }

    #[test]
    fn parse_override_ini() {
        let cli = Cli::try_parse_from([
            "rpytest",
            "-o",
            "addopts=-v",
            "-o",
            "testpaths=tests",
        ])
        .unwrap();

        assert_eq!(cli.override_ini, vec!["addopts=-v", "testpaths=tests"]);
    }
}
