//! Request and response types for the IPC protocol.

use serde::{Deserialize, Serialize};

/// Commands sent from CLI to daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Initialize a repository context within the daemon.
    InitContext {
        /// Absolute path to the repository root.
        repo_path: String,
        /// Optional path to Python interpreter.
        python_path: Option<String>,
    },

    /// Collect tests for a repository context.
    Collect {
        /// Context identifier returned from InitContext.
        context_id: String,
        /// Force full re-collection even if cache is valid.
        force: bool,
    },

    /// Run a set of tests.
    Run {
        /// Context identifier.
        context_id: String,
        /// List of test node IDs to run.
        node_ids: Vec<String>,
        /// Number of parallel workers (None = auto).
        workers: Option<u32>,
        /// Stop after N failures.
        maxfail: Option<u32>,
    },

    /// List tests matching filters (without running).
    List {
        /// Context identifier.
        context_id: String,
        /// Keyword expression filter.
        keyword: Option<String>,
        /// Marker expression filter.
        marker: Option<String>,
    },

    /// Shutdown the daemon or a specific context.
    Shutdown {
        /// If Some, shutdown only this context. If None, shutdown entire daemon.
        context_id: Option<String>,
    },

    /// Health check / ping.
    Ping,
}

/// Responses sent from daemon to CLI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Context successfully initialized.
    ContextReady {
        /// Unique context identifier.
        context_id: String,
        /// Hash of the current inventory for cache validation.
        inventory_hash: String,
    },

    /// Collection completed.
    CollectionComplete {
        /// Number of test nodes collected.
        node_count: usize,
        /// Collection duration in milliseconds.
        duration_ms: u64,
    },

    /// List of test node IDs matching the query.
    TestList {
        /// Matching node IDs.
        node_ids: Vec<String>,
    },

    /// Run completed.
    RunComplete {
        /// Total tests run.
        total: usize,
        /// Tests passed.
        passed: usize,
        /// Tests failed.
        failed: usize,
        /// Tests skipped.
        skipped: usize,
        /// Tests errored.
        errors: usize,
        /// Total duration in milliseconds.
        duration_ms: u64,
    },

    /// Shutdown acknowledged.
    ShutdownAck,

    /// Pong response to ping.
    Pong,

    /// Error response.
    Error {
        /// Error category.
        code: ErrorCode,
        /// Human-readable error message.
        message: String,
    },
}

/// Error codes for categorizing failures.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Context not found or not initialized.
    ContextNotFound,
    /// Collection failed (syntax error, import error, etc.).
    CollectionFailed,
    /// Invalid request parameters.
    InvalidRequest,
    /// Internal daemon error.
    InternalError,
    /// Operation timed out.
    Timeout,
    /// Python interpreter not found or invalid.
    PythonNotFound,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let requests = vec![
            Request::InitContext {
                repo_path: "/path/to/repo".to_string(),
                python_path: Some("/usr/bin/python3".to_string()),
            },
            Request::Collect {
                context_id: "ctx-123".to_string(),
                force: true,
            },
            Request::Run {
                context_id: "ctx-123".to_string(),
                node_ids: vec!["test_foo.py::test_bar".to_string()],
                workers: Some(4),
                maxfail: Some(1),
            },
            Request::List {
                context_id: "ctx-123".to_string(),
                keyword: Some("auth".to_string()),
                marker: None,
            },
            Request::Shutdown {
                context_id: Some("ctx-123".to_string()),
            },
            Request::Ping,
        ];

        for req in requests {
            let encoded = rmp_serde::to_vec(&req).unwrap();
            let decoded: Request = rmp_serde::from_slice(&encoded).unwrap();
            assert_eq!(req, decoded);
        }
    }

    #[test]
    fn response_roundtrip() {
        let responses = vec![
            Response::ContextReady {
                context_id: "ctx-123".to_string(),
                inventory_hash: "abc123".to_string(),
            },
            Response::CollectionComplete {
                node_count: 42,
                duration_ms: 150,
            },
            Response::TestList {
                node_ids: vec!["test_a".to_string(), "test_b".to_string()],
            },
            Response::RunComplete {
                total: 10,
                passed: 8,
                failed: 1,
                skipped: 1,
                errors: 0,
                duration_ms: 5000,
            },
            Response::ShutdownAck,
            Response::Pong,
            Response::Error {
                code: ErrorCode::ContextNotFound,
                message: "Context not found".to_string(),
            },
        ];

        for resp in responses {
            let encoded = rmp_serde::to_vec(&resp).unwrap();
            let decoded: Response = rmp_serde::from_slice(&encoded).unwrap();
            assert_eq!(resp, decoded);
        }
    }
}
