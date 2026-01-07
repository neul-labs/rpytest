//! Benchmark suite for measuring rpytest performance.
//!
//! Provides infrastructure for comparing rpytest vs pytest performance
//! across different test suite sizes and characteristics.

mod report;
mod runner;
mod suites;

pub use report::{format_report, BenchmarkReport};
pub use runner::{BenchmarkConfig, BenchmarkResult, BenchmarkRunner};
pub use suites::{SuiteGenerator, SuiteSize, TestSuite};
