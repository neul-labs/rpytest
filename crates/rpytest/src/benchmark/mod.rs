//! Benchmark suite for measuring rpytest performance.
//!
//! Provides infrastructure for comparing rpytest vs pytest performance
//! across different test suite sizes and characteristics.

#![allow(dead_code)]

mod report;
mod runner;
mod suites;

#[allow(unused_imports)]
pub use report::{format_report, BenchmarkReport};
#[allow(unused_imports)]
pub use runner::{BenchmarkConfig, BenchmarkResult, BenchmarkRunner};
#[allow(unused_imports)]
pub use suites::{SuiteGenerator, SuiteSize, TestSuite};
