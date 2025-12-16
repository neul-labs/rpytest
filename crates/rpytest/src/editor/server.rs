//! Editor server for handling JSON-RPC requests.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;

use anyhow::Result;
use tracing::{debug, error, info};

use super::protocol::*;

/// Editor server state.
pub struct EditorServer {
    /// Root path of the project.
    root_path: Option<PathBuf>,
    /// Context ID for daemon communication.
    context_id: Option<String>,
    /// Cache of test locations by file.
    test_cache: HashMap<String, Vec<TestLocation>>,
    /// Last known test statuses.
    test_statuses: HashMap<String, TestStatus>,
    /// Whether initialized.
    initialized: bool,
}

impl EditorServer {
    /// Create a new editor server.
    pub fn new() -> Self {
        Self {
            root_path: None,
            context_id: None,
            test_cache: HashMap::new(),
            test_statuses: HashMap::new(),
            initialized: false,
        }
    }

    /// Handle a JSON-RPC request.
    pub fn handle_request(&mut self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id;

        match &request.request {
            EditorRequest::Initialize { root_path } => {
                self.handle_initialize(id, root_path)
            }
            EditorRequest::ListTestsInFile { file_path } => {
                self.handle_list_tests_in_file(id, file_path)
            }
            EditorRequest::GetNearestTest { file_path, line } => {
                self.handle_get_nearest_test(id, file_path, *line)
            }
            EditorRequest::RunTest { node_id } => {
                self.handle_run_test(id, node_id)
            }
            EditorRequest::RunNearestTest { file_path, line } => {
                self.handle_run_nearest_test(id, file_path, *line)
            }
            EditorRequest::RunTestsInFile { file_path } => {
                self.handle_run_tests_in_file(id, file_path)
            }
            EditorRequest::GetTestStatus { node_id } => {
                self.handle_get_test_status(id, node_id)
            }
            EditorRequest::Shutdown => {
                self.handle_shutdown(id)
            }
        }
    }

    fn handle_initialize(&mut self, id: u64, root_path: &str) -> JsonRpcResponse {
        self.root_path = Some(PathBuf::from(root_path));
        self.initialized = true;

        info!("Editor server initialized for {}", root_path);

        // TODO: Connect to daemon and collect tests
        let test_count = 0;

        JsonRpcResponse::success(
            id,
            EditorResponse::Initialized {
                version: env!("CARGO_PKG_VERSION").to_string(),
                test_count,
            },
        )
    }

    fn handle_list_tests_in_file(&mut self, id: u64, file_path: &str) -> JsonRpcResponse {
        if !self.initialized {
            return JsonRpcResponse::error(id, ERR_NOT_INITIALIZED, "Server not initialized");
        }

        // Return cached tests or empty list
        let tests = self.test_cache
            .get(file_path)
            .cloned()
            .unwrap_or_default();

        JsonRpcResponse::success(
            id,
            EditorResponse::TestList {
                file_path: file_path.to_string(),
                tests,
            },
        )
    }

    fn handle_get_nearest_test(&mut self, id: u64, file_path: &str, line: u32) -> JsonRpcResponse {
        if !self.initialized {
            return JsonRpcResponse::error(id, ERR_NOT_INITIALIZED, "Server not initialized");
        }

        let tests = self.test_cache
            .get(file_path)
            .cloned()
            .unwrap_or_default();

        // Find the test whose line is closest to but not greater than the cursor line
        let nearest = tests
            .into_iter()
            .filter(|t| t.line <= line)
            .max_by_key(|t| t.line);

        JsonRpcResponse::success(
            id,
            EditorResponse::NearestTest { test: nearest },
        )
    }

    fn handle_run_test(&mut self, id: u64, node_id: &str) -> JsonRpcResponse {
        if !self.initialized {
            return JsonRpcResponse::error(id, ERR_NOT_INITIALIZED, "Server not initialized");
        }

        // Mark as running
        self.test_statuses.insert(node_id.to_string(), TestStatus::Running);

        // TODO: Actually run the test via daemon
        // For now, just acknowledge
        JsonRpcResponse::success(
            id,
            EditorResponse::RunStarted {
                node_ids: vec![node_id.to_string()],
            },
        )
    }

    fn handle_run_nearest_test(&mut self, id: u64, file_path: &str, line: u32) -> JsonRpcResponse {
        if !self.initialized {
            return JsonRpcResponse::error(id, ERR_NOT_INITIALIZED, "Server not initialized");
        }

        let tests = self.test_cache
            .get(file_path)
            .cloned()
            .unwrap_or_default();

        let nearest = tests
            .into_iter()
            .filter(|t| t.line <= line)
            .max_by_key(|t| t.line);

        match nearest {
            Some(test) => {
                self.test_statuses.insert(test.node_id.clone(), TestStatus::Running);
                JsonRpcResponse::success(
                    id,
                    EditorResponse::RunStarted {
                        node_ids: vec![test.node_id],
                    },
                )
            }
            None => JsonRpcResponse::error(id, ERR_TEST_NOT_FOUND, "No test found near cursor"),
        }
    }

    fn handle_run_tests_in_file(&mut self, id: u64, file_path: &str) -> JsonRpcResponse {
        if !self.initialized {
            return JsonRpcResponse::error(id, ERR_NOT_INITIALIZED, "Server not initialized");
        }

        let tests = self.test_cache
            .get(file_path)
            .cloned()
            .unwrap_or_default();

        if tests.is_empty() {
            return JsonRpcResponse::error(id, ERR_TEST_NOT_FOUND, "No tests in file");
        }

        let node_ids: Vec<String> = tests.iter().map(|t| t.node_id.clone()).collect();

        for node_id in &node_ids {
            self.test_statuses.insert(node_id.clone(), TestStatus::Running);
        }

        JsonRpcResponse::success(
            id,
            EditorResponse::RunStarted { node_ids },
        )
    }

    fn handle_get_test_status(&self, id: u64, node_id: &str) -> JsonRpcResponse {
        if !self.initialized {
            return JsonRpcResponse::error(id, ERR_NOT_INITIALIZED, "Server not initialized");
        }

        let status = self.test_statuses
            .get(node_id)
            .cloned()
            .unwrap_or(TestStatus::Unknown);

        JsonRpcResponse::success(
            id,
            EditorResponse::TestStatusResponse {
                node_id: node_id.to_string(),
                status,
                last_result: None,
            },
        )
    }

    fn handle_shutdown(&mut self, id: u64) -> JsonRpcResponse {
        info!("Editor server shutting down");
        self.initialized = false;
        JsonRpcResponse::success(id, EditorResponse::ShutdownAck)
    }

    /// Update test cache with tests from inventory.
    pub fn update_test_cache(&mut self, tests: Vec<TestLocation>) {
        self.test_cache.clear();

        for test in tests {
            self.test_cache
                .entry(test.file_path.clone())
                .or_default()
                .push(test);
        }

        // Sort tests by line number in each file
        for tests in self.test_cache.values_mut() {
            tests.sort_by_key(|t| t.line);
        }

        debug!("Updated test cache with {} files", self.test_cache.len());
    }

    /// Update test status after a run.
    pub fn update_test_status(&mut self, node_id: &str, status: TestStatus) {
        self.test_statuses.insert(node_id.to_string(), status);
    }

    /// Run the server in stdio mode (for editor integration).
    pub fn run_stdio(&mut self) -> Result<()> {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let mut reader = BufReader::new(stdin.lock());
        let mut writer = stdout.lock();

        info!("Editor server started in stdio mode");

        loop {
            // Read Content-Length header
            let mut header = String::new();
            if reader.read_line(&mut header)? == 0 {
                break; // EOF
            }

            if !header.starts_with("Content-Length:") {
                continue;
            }

            let length: usize = header
                .trim()
                .strip_prefix("Content-Length:")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);

            if length == 0 {
                continue;
            }

            // Read empty line
            let mut empty = String::new();
            reader.read_line(&mut empty)?;

            // Read content
            let mut content = vec![0u8; length];
            reader.read_exact(&mut content)?;

            // Parse and handle
            let content_str = String::from_utf8_lossy(&content);
            match serde_json::from_str::<JsonRpcRequest>(&content_str) {
                Ok(request) => {
                    debug!("Received request: {:?}", request.request);
                    let response = self.handle_request(&request);

                    // Send response
                    let response_json = serde_json::to_string(&response)?;
                    write!(writer, "Content-Length: {}\r\n\r\n{}", response_json.len(), response_json)?;
                    writer.flush()?;

                    // Check for shutdown
                    if matches!(request.request, EditorRequest::Shutdown) {
                        break;
                    }
                }
                Err(e) => {
                    error!("Failed to parse request: {}", e);
                    let response = JsonRpcResponse::error(0, ERR_PARSE_ERROR, e.to_string());
                    let response_json = serde_json::to_string(&response)?;
                    write!(writer, "Content-Length: {}\r\n\r\n{}", response_json.len(), response_json)?;
                    writer.flush()?;
                }
            }
        }

        info!("Editor server stopped");
        Ok(())
    }
}

impl Default for EditorServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize() {
        let mut server = EditorServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            request: EditorRequest::Initialize {
                root_path: "/project".to_string(),
            },
        };

        let response = server.handle_request(&request);
        assert!(response.result.is_some());
        assert!(server.initialized);
    }

    #[test]
    fn test_not_initialized() {
        let mut server = EditorServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            request: EditorRequest::ListTestsInFile {
                file_path: "test.py".to_string(),
            },
        };

        let response = server.handle_request(&request);
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, ERR_NOT_INITIALIZED);
    }

    #[test]
    fn test_nearest_test() {
        let mut server = EditorServer::new();
        server.initialized = true;

        server.update_test_cache(vec![
            TestLocation {
                node_id: "test.py::test_foo".to_string(),
                file_path: "test.py".to_string(),
                line: 10,
                name: "test_foo".to_string(),
                class_name: None,
            },
            TestLocation {
                node_id: "test.py::test_bar".to_string(),
                file_path: "test.py".to_string(),
                line: 20,
                name: "test_bar".to_string(),
                class_name: None,
            },
        ]);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            request: EditorRequest::GetNearestTest {
                file_path: "test.py".to_string(),
                line: 15,
            },
        };

        let response = server.handle_request(&request);
        if let Some(EditorResponse::NearestTest { test }) = response.result {
            assert!(test.is_some());
            assert_eq!(test.unwrap().node_id, "test.py::test_foo");
        } else {
            panic!("Wrong response type");
        }
    }
}
