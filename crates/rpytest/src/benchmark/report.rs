//! Benchmark report generation.

use std::fmt::Write;

use super::runner::BenchmarkResult;

/// Collection of benchmark results.
#[derive(Debug, Default)]
pub struct BenchmarkReport {
    /// Benchmark results.
    pub results: Vec<BenchmarkResult>,
    /// Overall summary.
    pub summary: ReportSummary,
}

/// Summary of benchmark results.
#[derive(Debug, Default)]
pub struct ReportSummary {
    /// Total benchmarks run.
    pub total_benchmarks: usize,
    /// Benchmarks meeting target.
    pub passing_benchmarks: usize,
    /// Average speedup across all benchmarks.
    pub average_speedup: f64,
    /// Minimum speedup.
    pub min_speedup: f64,
    /// Maximum speedup.
    pub max_speedup: f64,
    /// Whether all targets were met.
    pub all_targets_met: bool,
}

impl BenchmarkReport {
    /// Create a new empty report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a benchmark result.
    pub fn add_result(&mut self, result: BenchmarkResult) {
        self.results.push(result);
        self.update_summary();
    }

    /// Update the summary statistics.
    fn update_summary(&mut self) {
        if self.results.is_empty() {
            return;
        }

        let speedups: Vec<f64> = self.results.iter().map(|r| r.speedup).collect();

        self.summary.total_benchmarks = self.results.len();
        self.summary.passing_benchmarks = self
            .results
            .iter()
            .filter(|r| r.meets_target(1.0)) // At least as fast as pytest
            .count();
        self.summary.average_speedup = speedups.iter().sum::<f64>() / speedups.len() as f64;
        self.summary.min_speedup = speedups
            .iter()
            .cloned()
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);
        self.summary.max_speedup = speedups
            .iter()
            .cloned()
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(0.0);
        self.summary.all_targets_met = self.results.iter().all(|r| r.meets_target(1.0));
    }
}

/// Format a benchmark report as a string.
pub fn format_report(report: &BenchmarkReport) -> String {
    let mut output = String::new();

    writeln!(
        output,
        "╔══════════════════════════════════════════════════════════════╗"
    )
    .unwrap();
    writeln!(
        output,
        "║                    BENCHMARK REPORT                          ║"
    )
    .unwrap();
    writeln!(
        output,
        "╠══════════════════════════════════════════════════════════════╣"
    )
    .unwrap();

    // Individual results
    for result in &report.results {
        writeln!(output).unwrap();
        writeln!(output, "  {} ({} tests)", result.name, result.suite_size).unwrap();
        writeln!(
            output,
            "  ├─ pytest:  {:.2}ms ± {:.2}ms",
            result.pytest_mean.as_secs_f64() * 1000.0,
            result.pytest_stddev.as_secs_f64() * 1000.0
        )
        .unwrap();
        writeln!(
            output,
            "  ├─ rpytest: {:.2}ms ± {:.2}ms",
            result.rpytest_mean.as_secs_f64() * 1000.0,
            result.rpytest_stddev.as_secs_f64() * 1000.0
        )
        .unwrap();

        let status = if result.speedup >= 2.0 {
            "🚀"
        } else if result.speedup >= 1.3 {
            "✅"
        } else if result.speedup >= 1.0 {
            "➖"
        } else {
            "⚠️"
        };

        writeln!(output, "  └─ speedup: {:.2}x {}", result.speedup, status).unwrap();
    }

    // Summary
    writeln!(output).unwrap();
    writeln!(
        output,
        "╠══════════════════════════════════════════════════════════════╣"
    )
    .unwrap();
    writeln!(
        output,
        "║                       SUMMARY                                ║"
    )
    .unwrap();
    writeln!(
        output,
        "╠══════════════════════════════════════════════════════════════╣"
    )
    .unwrap();
    writeln!(output).unwrap();

    writeln!(
        output,
        "  Total benchmarks:    {}",
        report.summary.total_benchmarks
    )
    .unwrap();
    writeln!(
        output,
        "  Passing (≥1.0x):     {}",
        report.summary.passing_benchmarks
    )
    .unwrap();
    writeln!(
        output,
        "  Average speedup:     {:.2}x",
        report.summary.average_speedup
    )
    .unwrap();
    writeln!(
        output,
        "  Min/Max speedup:     {:.2}x / {:.2}x",
        report.summary.min_speedup, report.summary.max_speedup
    )
    .unwrap();

    writeln!(output).unwrap();

    // Target evaluation
    writeln!(output, "  Targets:").unwrap();
    let medium_target = report
        .results
        .iter()
        .filter(|r| r.suite_size >= 50 && r.suite_size <= 500)
        .all(|r| r.speedup >= 1.3);
    let overhead_target = report
        .results
        .iter()
        .filter(|r| r.suite_size < 50)
        .all(|r| r.speedup >= 3.0);

    writeln!(
        output,
        "    Medium suites (≥1.3x): {}",
        if medium_target {
            "✅ PASS"
        } else {
            "❌ FAIL"
        }
    )
    .unwrap();
    writeln!(
        output,
        "    Overhead-bound (≥3.0x): {}",
        if overhead_target {
            "✅ PASS"
        } else {
            "❌ FAIL"
        }
    )
    .unwrap();

    writeln!(output).unwrap();
    writeln!(
        output,
        "╚══════════════════════════════════════════════════════════════╝"
    )
    .unwrap();

    output
}

/// Format as JSON for dashboards.
pub fn format_json(report: &BenchmarkReport) -> String {
    let results: Vec<serde_json::Value> = report
        .results
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "suite_size": r.suite_size,
                "pytest_mean_ms": r.pytest_mean.as_secs_f64() * 1000.0,
                "pytest_stddev_ms": r.pytest_stddev.as_secs_f64() * 1000.0,
                "rpytest_mean_ms": r.rpytest_mean.as_secs_f64() * 1000.0,
                "rpytest_stddev_ms": r.rpytest_stddev.as_secs_f64() * 1000.0,
                "speedup": r.speedup,
            })
        })
        .collect();

    serde_json::json!({
        "results": results,
        "summary": {
            "total_benchmarks": report.summary.total_benchmarks,
            "passing_benchmarks": report.summary.passing_benchmarks,
            "average_speedup": report.summary.average_speedup,
            "min_speedup": report.summary.min_speedup,
            "max_speedup": report.summary.max_speedup,
            "all_targets_met": report.summary.all_targets_met,
        }
    })
    .to_string()
}

/// Format as markdown for documentation.
pub fn format_markdown(report: &BenchmarkReport) -> String {
    let mut output = String::new();

    writeln!(output, "# Benchmark Results\n").unwrap();
    writeln!(output, "| Suite | Tests | pytest | rpytest | Speedup |").unwrap();
    writeln!(output, "|-------|-------|--------|---------|---------|").unwrap();

    for result in &report.results {
        writeln!(
            output,
            "| {} | {} | {:.0}ms | {:.0}ms | {:.2}x |",
            result.name,
            result.suite_size,
            result.pytest_mean.as_secs_f64() * 1000.0,
            result.rpytest_mean.as_secs_f64() * 1000.0,
            result.speedup
        )
        .unwrap();
    }

    writeln!(output).unwrap();
    writeln!(
        output,
        "**Average speedup: {:.2}x**",
        report.summary.average_speedup
    )
    .unwrap();

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::runner::RunTiming;
    use std::time::Duration;

    fn make_result(name: &str, size: usize, pytest_ms: u64, rpytest_ms: u64) -> BenchmarkResult {
        let mut result = BenchmarkResult {
            name: name.to_string(),
            suite_size: size,
            pytest_timings: vec![RunTiming {
                duration: Duration::from_millis(pytest_ms),
                exit_code: 0,
                tests_run: size,
            }],
            rpytest_timings: vec![RunTiming {
                duration: Duration::from_millis(rpytest_ms),
                exit_code: 0,
                tests_run: size,
            }],
            speedup: 0.0,
            pytest_mean: Duration::ZERO,
            rpytest_mean: Duration::ZERO,
            pytest_stddev: Duration::ZERO,
            rpytest_stddev: Duration::ZERO,
        };
        result.calculate_stats();
        result
    }

    #[test]
    fn test_report_summary() {
        let mut report = BenchmarkReport::new();
        report.add_result(make_result("small", 10, 200, 100)); // 2x
        report.add_result(make_result("medium", 100, 1500, 1000)); // 1.5x

        assert_eq!(report.summary.total_benchmarks, 2);
        assert_eq!(report.summary.passing_benchmarks, 2);
        assert!((report.summary.average_speedup - 1.75).abs() < 0.01);
        assert!((report.summary.min_speedup - 1.5).abs() < 0.01);
        assert!((report.summary.max_speedup - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_format_report() {
        let mut report = BenchmarkReport::new();
        report.add_result(make_result("test_suite", 50, 1000, 500));

        let formatted = format_report(&report);
        assert!(formatted.contains("test_suite"));
        assert!(formatted.contains("2.00x"));
        assert!(formatted.contains("BENCHMARK REPORT"));
    }
}
