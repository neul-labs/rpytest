//! Streaming event types for test execution.

use serde::{Deserialize, Serialize};

/// Outcome of a single test execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    /// Test passed.
    Passed,
    /// Test failed with assertion or other failure.
    Failed {
        /// Failure message or traceback.
        message: String,
    },
    /// Test was skipped.
    Skipped {
        /// Reason for skipping, if provided.
        reason: Option<String>,
    },
    /// Test encountered an error (not a test failure).
    Error {
        /// Error message or traceback.
        message: String,
    },
    /// Test was expected to fail and did fail.
    XFail {
        /// Reason for expected failure.
        reason: Option<String>,
    },
    /// Test was expected to fail but passed.
    XPass,
}

impl Outcome {
    /// Returns true if this outcome represents a successful test.
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            Outcome::Passed | Outcome::Skipped { .. } | Outcome::XFail { .. }
        )
    }

    /// Returns true if this outcome represents a failure that should be counted.
    pub fn is_failure(&self) -> bool {
        matches!(self, Outcome::Failed { .. } | Outcome::XPass)
    }

    /// Returns true if this outcome represents an error.
    pub fn is_error(&self) -> bool {
        matches!(self, Outcome::Error { .. })
    }
}

/// Event emitted during test execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TestEvent {
    /// A test is about to start.
    TestStart {
        /// Test node ID.
        node_id: String,
    },

    /// A test has completed.
    TestFinish {
        /// Test node ID.
        node_id: String,
        /// Test outcome.
        outcome: Outcome,
        /// Duration in milliseconds.
        duration_ms: u64,
        /// Captured stdout, if any.
        stdout: Option<String>,
        /// Captured stderr, if any.
        stderr: Option<String>,
    },

    /// Collection is starting.
    CollectionStart,

    /// A test file was collected.
    ItemCollected {
        /// Test node ID.
        node_id: String,
        /// File path.
        file_path: String,
        /// Line number.
        line_number: Option<u32>,
        /// Markers applied to this test.
        markers: Vec<String>,
    },

    /// Collection has finished.
    CollectionFinish {
        /// Total items collected.
        count: usize,
    },

    /// Session is starting.
    SessionStart {
        /// Total tests to run.
        test_count: usize,
    },

    /// Session has finished.
    SessionFinish {
        /// Exit code (0 = all passed).
        exit_code: i32,
    },

    /// A warning was emitted.
    Warning {
        /// Warning message.
        message: String,
        /// Source location, if known.
        location: Option<String>,
    },
}

/// Log event for daemon logging.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogEvent {
    /// Log level.
    pub level: LogLevel,
    /// Log message.
    pub message: String,
    /// Optional source module.
    pub module: Option<String>,
    /// Timestamp in milliseconds since epoch.
    pub timestamp_ms: u64,
}

/// Log levels matching Python's logging module.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_classification() {
        assert!(Outcome::Passed.is_success());
        assert!(Outcome::Skipped { reason: None }.is_success());
        assert!(Outcome::XFail { reason: None }.is_success());

        assert!(Outcome::Failed {
            message: "".to_string()
        }
        .is_failure());
        assert!(Outcome::XPass.is_failure());

        assert!(Outcome::Error {
            message: "".to_string()
        }
        .is_error());
    }

    #[test]
    fn test_event_roundtrip() {
        let events = vec![
            TestEvent::TestStart {
                node_id: "test_foo.py::test_bar".to_string(),
            },
            TestEvent::TestFinish {
                node_id: "test_foo.py::test_bar".to_string(),
                outcome: Outcome::Passed,
                duration_ms: 42,
                stdout: Some("output".to_string()),
                stderr: None,
            },
            TestEvent::CollectionStart,
            TestEvent::ItemCollected {
                node_id: "test_foo.py::test_bar".to_string(),
                file_path: "test_foo.py".to_string(),
                line_number: Some(10),
                markers: vec!["slow".to_string()],
            },
            TestEvent::CollectionFinish { count: 100 },
            TestEvent::SessionStart { test_count: 50 },
            TestEvent::SessionFinish { exit_code: 0 },
            TestEvent::Warning {
                message: "Deprecated API".to_string(),
                location: Some("test_foo.py:20".to_string()),
            },
        ];

        for event in events {
            let encoded = rmp_serde::to_vec(&event).unwrap();
            let decoded: TestEvent = rmp_serde::from_slice(&encoded).unwrap();
            assert_eq!(event, decoded);
        }
    }
}
