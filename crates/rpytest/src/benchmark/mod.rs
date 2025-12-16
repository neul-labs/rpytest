//! Benchmark suite for measuring rpytest performance.
//!
//! Provides infrastructure for comparing rpytest vs pytest performance
//! across different test suite sizes and characteristics.

mod runner;
mod report;
mod suites;

pub use runner::{BenchmarkRunner, BenchmarkConfig, BenchmarkResult};
pub use report::{BenchmarkReport, format_report};
pub use suites::{TestSuite, SuiteGenerator, SuiteSize};
