//! Editor protocol types (JSON-RPC style).

use serde::{Deserialize, Serialize};

/// Test location in a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestLocation {
    /// Test node ID.
    pub node_id: String,
    /// File path.
    pub file_path: String,
    /// Line number (1-indexed).
    pub line: u32,
    /// Test name.
    pub name: String,
    /// Parent class name (if method).
    pub class_name: Option<String>,
}

/// Request from editor to rpytest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum EditorRequest {
    /// List all tests in a file.
    #[serde(rename = "listTestsInFile")]
    ListTestsInFile {
        /// Path to the file.
        file_path: String,
    },

    /// Get the nearest test to a position.
    #[serde(rename = "getNearestTest")]
    GetNearestTest {
        /// Path to the file.
        file_path: String,
        /// Line number (1-indexed).
        line: u32,
    },

    /// Run a specific test by node ID.
    #[serde(rename = "runTest")]
    RunTest {
        /// Test node ID to run.
        node_id: String,
    },

    /// Run the nearest test to a position.
    #[serde(rename = "runNearestTest")]
    RunNearestTest {
        /// Path to the file.
        file_path: String,
        /// Line number (1-indexed).
        line: u32,
    },

    /// Run all tests in a file.
    #[serde(rename = "runTestsInFile")]
    RunTestsInFile {
        /// Path to the file.
        file_path: String,
    },

    /// Get status of a test.
    #[serde(rename = "getTestStatus")]
    GetTestStatus {
        /// Test node ID.
        node_id: String,
    },

    /// Initialize connection with project root.
    #[serde(rename = "initialize")]
    Initialize {
        /// Root directory of the project.
        root_path: String,
    },

    /// Shutdown the editor server.
    #[serde(rename = "shutdown")]
    Shutdown,
}

/// Test status from last run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TestStatus {
    /// Test hasn't been run yet.
    Unknown,
    /// Test passed.
    Passed,
    /// Test failed.
    Failed,
    /// Test was skipped.
    Skipped,
    /// Test errored.
    Error,
    /// Test is currently running.
    Running,
}

/// Test result details.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    /// Test node ID.
    pub node_id: String,
    /// Test status.
    pub status: TestStatus,
    /// Duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Failure message.
    pub message: Option<String>,
}

/// Response from rpytest to editor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EditorResponse {
    /// Successful initialization.
    #[serde(rename = "initialized")]
    Initialized {
        /// Server version.
        version: String,
        /// Number of tests found.
        test_count: usize,
    },

    /// List of tests in a file.
    #[serde(rename = "testList")]
    TestList {
        /// File path.
        file_path: String,
        /// Tests in the file.
        tests: Vec<TestLocation>,
    },

    /// Nearest test to position.
    #[serde(rename = "nearestTest")]
    NearestTest {
        /// The nearest test, if found.
        test: Option<TestLocation>,
    },

    /// Test run started.
    #[serde(rename = "runStarted")]
    RunStarted {
        /// Node IDs being run.
        node_ids: Vec<String>,
    },

    /// Test run completed.
    #[serde(rename = "runComplete")]
    RunComplete {
        /// Results for each test.
        results: Vec<TestResult>,
        /// Total duration in milliseconds.
        duration_ms: u64,
    },

    /// Test status.
    #[serde(rename = "testStatus")]
    TestStatusResponse {
        /// Test node ID.
        node_id: String,
        /// Current status.
        status: TestStatus,
        /// Last result if available.
        last_result: Option<TestResult>,
    },

    /// Shutdown acknowledged.
    #[serde(rename = "shutdownAck")]
    ShutdownAck,

    /// Error response.
    #[serde(rename = "error")]
    Error {
        /// Error code.
        code: i32,
        /// Error message.
        message: String,
    },
}

/// JSON-RPC request wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,
    /// Request ID.
    pub id: u64,
    /// The actual request.
    #[serde(flatten)]
    pub request: EditorRequest,
}

/// JSON-RPC response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,
    /// Request ID this responds to.
    pub id: u64,
    /// The result (if successful).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<EditorResponse>,
    /// The error (if failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code.
    pub code: i32,
    /// Error message.
    pub message: String,
}

impl JsonRpcResponse {
    /// Create a success response.
    pub fn success(id: u64, result: EditorResponse) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Create an error response.
    pub fn error(id: u64, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// Error codes
pub const ERR_PARSE_ERROR: i32 = -32700;
pub const ERR_INVALID_REQUEST: i32 = -32600;
pub const ERR_METHOD_NOT_FOUND: i32 = -32601;
pub const ERR_INVALID_PARAMS: i32 = -32602;
pub const ERR_INTERNAL_ERROR: i32 = -32603;
pub const ERR_NOT_INITIALIZED: i32 = -32002;
pub const ERR_TEST_NOT_FOUND: i32 = -32001;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_request() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            request: EditorRequest::ListTestsInFile {
                file_path: "test_foo.py".to_string(),
            },
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("listTestsInFile"));
        assert!(json.contains("test_foo.py"));
    }

    #[test]
    fn test_deserialize_request() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"getNearestTest","params":{"file_path":"test.py","line":10}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();

        match req.request {
            EditorRequest::GetNearestTest { file_path, line } => {
                assert_eq!(file_path, "test.py");
                assert_eq!(line, 10);
            }
            _ => panic!("Wrong request type"),
        }
    }

    #[test]
    fn test_serialize_response() {
        let resp = JsonRpcResponse::success(
            1,
            EditorResponse::TestList {
                file_path: "test.py".to_string(),
                tests: vec![],
            },
        );

        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("testList"));
    }
}
