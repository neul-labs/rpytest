//! Benchmark runner for measuring test execution performance.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::Result;

/// Configuration for benchmark runs.
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    /// Root directory containing test suite.
    pub root: PathBuf,
    /// Python executable path.
    pub python: String,
    /// Number of warmup runs before measuring.
    pub warmup_runs: usize,
    /// Number of measured runs.
    pub measured_runs: usize,
    /// Timeout per run in seconds.
    pub timeout_secs: u64,
    /// Additional arguments for test runners.
    pub extra_args: Vec<String>,
    /// Whether to collect only (no execution).
    pub collect_only: bool,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            python: "python3".to_string(),
            warmup_runs: 1,
            measured_runs: 3,
            timeout_secs: 600,
            extra_args: vec![],
            collect_only: false,
        }
    }
}

/// Result of a single benchmark run.
#[derive(Debug, Clone)]
pub struct RunTiming {
    /// Duration of this run.
    pub duration: Duration,
    /// Exit code.
    pub exit_code: i32,
    /// Tests executed.
    pub tests_run: usize,
}

/// Result of a complete benchmark.
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    /// Name of the benchmark.
    pub name: String,
    /// Suite size.
    pub suite_size: usize,
    /// Pytest timings.
    pub pytest_timings: Vec<RunTiming>,
    /// Rpytest timings.
    pub rpytest_timings: Vec<RunTiming>,
    /// Calculated speedup (pytest_time / rpytest_time).
    pub speedup: f64,
    /// Pytest mean duration.
    pub pytest_mean: Duration,
    /// Rpytest mean duration.
    pub rpytest_mean: Duration,
    /// Pytest stddev.
    pub pytest_stddev: Duration,
    /// Rpytest stddev.
    pub rpytest_stddev: Duration,
}

impl BenchmarkResult {
    /// Calculate statistics from timings.
    pub fn calculate_stats(&mut self) {
        self.pytest_mean = mean_duration(&self.pytest_timings);
        self.rpytest_mean = mean_duration(&self.rpytest_timings);
        self.pytest_stddev = stddev_duration(&self.pytest_timings, self.pytest_mean);
        self.rpytest_stddev = stddev_duration(&self.rpytest_timings, self.rpytest_mean);

        if self.rpytest_mean.as_secs_f64() > 0.0 {
            self.speedup = self.pytest_mean.as_secs_f64() / self.rpytest_mean.as_secs_f64();
        }
    }

    /// Check if result meets performance target.
    pub fn meets_target(&self, min_speedup: f64) -> bool {
        self.speedup >= min_speedup
    }
}

fn mean_duration(timings: &[RunTiming]) -> Duration {
    if timings.is_empty() {
        return Duration::ZERO;
    }
    let total: Duration = timings.iter().map(|t| t.duration).sum();
    total / timings.len() as u32
}

fn stddev_duration(timings: &[RunTiming], mean: Duration) -> Duration {
    if timings.len() < 2 {
        return Duration::ZERO;
    }

    let mean_secs = mean.as_secs_f64();
    let variance: f64 = timings
        .iter()
        .map(|t| {
            let diff = t.duration.as_secs_f64() - mean_secs;
            diff * diff
        })
        .sum::<f64>()
        / (timings.len() - 1) as f64;

    Duration::from_secs_f64(variance.sqrt())
}

/// Runner for benchmarks.
pub struct BenchmarkRunner {
    config: BenchmarkConfig,
}

impl BenchmarkRunner {
    /// Create a new benchmark runner.
    pub fn new(config: BenchmarkConfig) -> Self {
        Self { config }
    }

    /// Run a complete benchmark comparing pytest vs rpytest.
    pub fn run_benchmark(&self, name: &str) -> Result<BenchmarkResult> {
        let mut result = BenchmarkResult {
            name: name.to_string(),
            suite_size: 0,
            pytest_timings: Vec::new(),
            rpytest_timings: Vec::new(),
            speedup: 0.0,
            pytest_mean: Duration::ZERO,
            rpytest_mean: Duration::ZERO,
            pytest_stddev: Duration::ZERO,
            rpytest_stddev: Duration::ZERO,
        };

        // Warmup runs
        for _ in 0..self.config.warmup_runs {
            self.run_pytest()?;
            self.run_rpytest()?;
        }

        // Measured runs
        for _ in 0..self.config.measured_runs {
            let pytest_timing = self.run_pytest()?;
            result.suite_size = pytest_timing.tests_run;
            result.pytest_timings.push(pytest_timing);

            let rpytest_timing = self.run_rpytest()?;
            result.rpytest_timings.push(rpytest_timing);
        }

        result.calculate_stats();
        Ok(result)
    }

    fn run_pytest(&self) -> Result<RunTiming> {
        let start = Instant::now();

        let mut cmd = Command::new(&self.config.python);
        cmd.arg("-m").arg("pytest").arg("-q").arg("--tb=no");

        if self.config.collect_only {
            cmd.arg("--collect-only");
        }

        cmd.args(&self.config.extra_args)
            .current_dir(&self.config.root);

        let output = cmd.output()?;
        let duration = start.elapsed();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let tests_run = parse_test_count(&stdout);

        Ok(RunTiming {
            duration,
            exit_code: output.status.code().unwrap_or(-1),
            tests_run,
        })
    }

    fn run_rpytest(&self) -> Result<RunTiming> {
        let start = Instant::now();

        // Find rpytest binary
        let rpytest_bin = find_rpytest_binary()?;

        let mut cmd = Command::new(&rpytest_bin);
        cmd.arg("-q").arg("--tb=no");

        if self.config.collect_only {
            cmd.arg("--collect-only");
        }

        cmd.args(&self.config.extra_args)
            .current_dir(&self.config.root);

        let output = cmd.output()?;
        let duration = start.elapsed();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let tests_run = parse_test_count(&stdout);

        Ok(RunTiming {
            duration,
            exit_code: output.status.code().unwrap_or(-1),
            tests_run,
        })
    }
}

fn find_rpytest_binary() -> Result<PathBuf> {
    // Try current exe directory first
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let rpytest = dir.join("rpytest");
            if rpytest.exists() {
                return Ok(rpytest);
            }
        }
    }

    // Try PATH
    if let Ok(path) = which::which("rpytest") {
        return Ok(path);
    }

    // Try cargo target directory
    let target_debug = PathBuf::from("target/debug/rpytest");
    if target_debug.exists() {
        return Ok(target_debug);
    }

    let target_release = PathBuf::from("target/release/rpytest");
    if target_release.exists() {
        return Ok(target_release);
    }

    anyhow::bail!("Could not find rpytest binary")
}

fn parse_test_count(output: &str) -> usize {
    // Parse pytest-style output for test count
    // Look for "X passed" or "collected X items"
    for line in output.lines() {
        if line.contains("passed") {
            if let Some(num) = line.split_whitespace().find(|s| s.parse::<usize>().is_ok()) {
                return num.parse().unwrap_or(0);
            }
        }
        if line.contains("collected") && line.contains("items") {
            if let Some(num) = line.split_whitespace().find(|s| s.parse::<usize>().is_ok()) {
                return num.parse().unwrap_or(0);
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mean_duration() {
        let timings = vec![
            RunTiming {
                duration: Duration::from_millis(100),
                exit_code: 0,
                tests_run: 10,
            },
            RunTiming {
                duration: Duration::from_millis(200),
                exit_code: 0,
                tests_run: 10,
            },
            RunTiming {
                duration: Duration::from_millis(300),
                exit_code: 0,
                tests_run: 10,
            },
        ];

        let mean = mean_duration(&timings);
        assert_eq!(mean, Duration::from_millis(200));
    }

    #[test]
    fn test_parse_test_count() {
        assert_eq!(parse_test_count("10 passed in 1.23s"), 10);
        assert_eq!(parse_test_count("collected 42 items"), 42);
        assert_eq!(parse_test_count("no tests"), 0);
    }

    #[test]
    fn test_speedup_calculation() {
        let mut result = BenchmarkResult {
            name: "test".to_string(),
            suite_size: 100,
            pytest_timings: vec![RunTiming {
                duration: Duration::from_millis(2000),
                exit_code: 0,
                tests_run: 100,
            }],
            rpytest_timings: vec![RunTiming {
                duration: Duration::from_millis(1000),
                exit_code: 0,
                tests_run: 100,
            }],
            speedup: 0.0,
            pytest_mean: Duration::ZERO,
            rpytest_mean: Duration::ZERO,
            pytest_stddev: Duration::ZERO,
            rpytest_stddev: Duration::ZERO,
        };

        result.calculate_stats();
        assert!((result.speedup - 2.0).abs() < 0.01);
    }
}
