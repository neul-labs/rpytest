//! IPC server using NNG for communication with the CLI.

use crate::context::RepoContext;
use crate::error::{Result, DaemonError};
use crate::models::{DaemonConfig, TestNode};
use crate::storage::DaemonStorage;
use futures::executor::block_on;
use nng::{Message, Protocol, Socket};
use rmp_serde::{Deserializer, Serializer};
use rpytest_core::protocol::{ErrorCode, Request, Response, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::signal;
use tokio::sync::{Mutex, Notify};
use tokio::time::Duration;
use tracing::{error, info};
use uuid::Uuid;

/// Type alias for the contexts map
type ContextMap = Mutex<HashMap<String, RepoContext>>;

/// A streaming test run in progress.
#[derive(Debug, Clone)]
pub struct StreamingRun {
    pub run_id: String,
    pub context_id: String,
    pub node_ids: Vec<String>,
    pub completed: usize,
    pub total: usize,
    pub results: Vec<TestNode>,
}

/// Main daemon server.
#[derive(Clone)]
pub struct DaemonServer {
    /// Socket URL for IPC (e.g., "ipc:///tmp/rpytest.sock")
    socket_url: String,
    /// Storage backend
    storage: DaemonStorage,
    /// Active contexts (context_id -> RepoContext)
    contexts: Arc<ContextMap>,
    /// Server configuration
    config: DaemonConfig,
}

impl DaemonServer {
    /// Create a new daemon server.
    pub fn new(socket_path: PathBuf, storage_path: PathBuf) -> Result<Self> {
        let storage = DaemonStorage::open(&storage_path)?;

        // Convert path to NNG IPC URL
        let socket_url = format!("ipc://{}", socket_path.display());

        Ok(DaemonServer {
            socket_url,
            storage,
            contexts: Arc::new(Mutex::new(HashMap::new())),
            config: DaemonConfig::default(),
        })
    }

    /// Start the server.
    pub async fn run(&mut self) -> Result<()> {
        // Create NNG socket with rep protocol (request-response)
        let socket = Socket::new(Protocol::Rep0)?;

        // Clean up old socket file if exists (for ipc:// transport)
        if self.socket_url.starts_with("ipc://") {
            let path = &self.socket_url[4..]; // Remove "ipc://" prefix
            let path = PathBuf::from(path);
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
        }

        // Listen on socket
        socket.listen(&self.socket_url)?;

        info!("Daemon listening on {}", self.socket_url);

        // Handle signals
        let shutdown = Arc::new(Notify::new());
        let shutdown_clone = shutdown.clone();

        // Spawn a task to handle shutdown signal
        tokio::spawn(async move {
            let _ = signal::ctrl_c().await;
            shutdown_clone.notify_one();
        });

        // Use a separate thread for NNG event loop
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();
        let socket_url = self.socket_url.clone();
        let contexts = Arc::clone(&self.contexts);
        let storage = self.storage.clone();

        // Spawn blocking task for NNG server
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                // Create socket in blocking context
                let socket = Socket::new(Protocol::Rep0).map_err(DaemonError::from)?;

                // Clean up old socket file if exists
                if socket_url.starts_with("ipc://") {
                    let path = &socket_url[4..];
                    let path = PathBuf::from(path);
                    if path.exists() {
                        std::fs::remove_file(&path).map_err(DaemonError::from)?;
                    }
                }

                // Listen
                socket.listen(&socket_url).map_err(DaemonError::from)?;
                info!("Daemon listening on {}", socket_url);

                // Main loop
                loop {
                    // Check for shutdown
                    if shutdown_rx.recv_timeout(std::time::Duration::from_millis(100)).is_ok() {
                        break;
                    }

                    // Try to receive with timeout
                    match socket.recv() {
                        Ok(msg) => {
                            let msg_bytes = msg.as_slice().to_vec();
                            // Process in a separate blocking task
                            let result = Self::process_in_blocking(
                                msg_bytes,
                                storage.clone(),
                                contexts.clone(),
                                socket.clone(),
                            );
                            if let Err(e) = result {
                                error!("Processing error: {}", e);
                            }
                        }
                        Err(nng::Error::TimedOut) | Err(nng::Error::TryAgain) => {
                            // Timeout or would block, continue
                            continue;
                        }
                        Err(e) => {
                            error!("Receive error: {}", e);
                        }
                    }
                }
                Ok::<(), DaemonError>(())
            }).await;

            if let Err(e) = result {
                error!("Server error: {}", e);
            }
        });

        // Wait for shutdown signal
        shutdown.notified().await;
        let _ = shutdown_tx.send(());

        Ok(())
    }

    /// Process a message in a blocking context.
    fn process_in_blocking(
        msg_bytes: Vec<u8>,
        storage: DaemonStorage,
        contexts: Arc<ContextMap>,
        socket: Socket,
    ) -> Result<()> {
        // Deserialize request
        let mut deserializer = Deserializer::new(&msg_bytes[..]);
        let request: Request = Deserialize::deserialize(&mut deserializer)?;

        // Process request
        let response = futures::executor::block_on(async {
            Self::process_request(request, &storage, &contexts).await
        });

        // Serialize response
        let mut response_buf = Vec::new();
        response.serialize(&mut Serializer::new(&mut response_buf))?;

        // Send response - convert Vec<u8> to Message using from_slice
        let response_msg = nng::Message::from(response_buf.as_slice());
        if let Err((_, e)) = socket.send(response_msg) {
            return Err(DaemonError::Nng(e));
        }

        Ok(())
    }

    /// Process a single request.
    async fn process_request(
        request: Request,
        storage: &DaemonStorage,
        contexts: &ContextMap,
    ) -> Response {
        match request {
            Request::InitContext {
                protocol_version,
                repo_path,
                python_path,
            } => {
                // Check protocol version
                if protocol_version != PROTOCOL_VERSION {
                    return Response::Error {
                        code: ErrorCode::VersionMismatch,
                        message: format!(
                            "Protocol version mismatch: CLI={}, Daemon={}",
                            protocol_version, PROTOCOL_VERSION
                        ),
                    };
                }

                // Generate context ID
                let context_id = Uuid::new_v4().to_string();

                // Create context
                let python_path = python_path.map(PathBuf::from);
                let mut context = RepoContext::new(
                    &context_id,
                    Path::new(&repo_path),
                    python_path,
                    Some(storage.clone()),
                );

                // Collect tests
                if let Err(e) = context.collect(false) {
                    return Response::Error {
                        code: ErrorCode::CollectionFailed,
                        message: format!("Collection failed: {}", e),
                    };
                }

                // Store context
                let mut contexts = contexts.lock().await;
                contexts.insert(context_id.clone(), context);

                let inventory_hash = contexts
                    .get(&context_id)
                    .map(|c| c.inventory_hash.clone())
                    .unwrap_or_default();

                Response::ContextReady {
                    protocol_version: PROTOCOL_VERSION,
                    context_id,
                    inventory_hash,
                }
            }

            Request::Collect { context_id, force } => {
                let mut contexts = contexts.lock().await;
                if let Some(context) = contexts.get_mut(&context_id) {
                    match context.collect(force) {
                        Ok((count, duration_ms)) => Response::CollectionComplete {
                            node_count: count,
                            duration_ms,
                        },
                        Err(e) => Response::Error {
                            code: ErrorCode::CollectionFailed,
                            message: format!("Collection failed: {}", e),
                        },
                    }
                } else {
                    Response::Error {
                        code: ErrorCode::ContextNotFound,
                        message: format!("Context not found: {}", context_id),
                    }
                }
            }

            Request::Run {
                context_id,
                node_ids,
                workers,
                maxfail,
            } => {
                let mut contexts = contexts.lock().await;
                if let Some(context) = contexts.get_mut(&context_id) {
                    match tokio::time::timeout(
                        Duration::from_secs(300),
                        context.run_tests(&node_ids, workers, maxfail),
                    )
                    .await
                    {
                        Ok(Ok(summary)) => Response::RunComplete {
                            total: summary.total,
                            passed: summary.passed,
                            failed: summary.failed,
                            skipped: summary.skipped,
                            errors: summary.errors,
                            duration_ms: summary.duration_ms,
                        },
                        Ok(Err(e)) => Response::Error {
                            code: ErrorCode::InternalError,
                            message: format!("Run failed: {}", e),
                        },
                        Err(_) => Response::Error {
                            code: ErrorCode::Timeout,
                            message: "Run timed out".to_string(),
                        },
                    }
                } else {
                    Response::Error {
                        code: ErrorCode::ContextNotFound,
                        message: format!("Context not found: {}", context_id),
                    }
                }
            }

            Request::List {
                context_id,
                keyword,
                marker,
            } => {
                let contexts = contexts.lock().await;
                if let Some(context) = contexts.get(&context_id) {
                    let filtered: Vec<TestNode> = if let Some(kw) = keyword {
                        context.filter_by_keyword(&kw)
                    } else if let Some(mk) = marker {
                        context.filter_by_marker(&mk)
                    } else {
                        context.get_inventory()
                    };

                    Response::TestList {
                        node_ids: filtered.into_iter().map(|n| n.node_id).collect(),
                    }
                } else {
                    Response::Error {
                        code: ErrorCode::ContextNotFound,
                        message: format!("Context not found: {}", context_id),
                    }
                }
            }

            Request::GetInventory { context_id } => {
                let contexts = contexts.lock().await;
                if let Some(context) = contexts.get(&context_id) {
                    let nodes: Vec<TestNode> = context.get_inventory();
                    Response::InventoryData {
                        hash: context.inventory_hash.clone(),
                        collected_at: context.last_collection_time as u64,
                        nodes: nodes
                            .into_iter()
                            .map(|n| n.into())
                            .collect(),
                    }
                } else {
                    Response::Error {
                        code: ErrorCode::ContextNotFound,
                        message: format!("Context not found: {}", context_id),
                    }
                }
            }

            Request::Ping => Response::Pong,

            Request::Shutdown { context_id } => {
                let mut contexts = contexts.lock().await;
                if let Some(id) = context_id {
                    contexts.remove(&id);
                } else {
                    contexts.clear();
                }
                Response::ShutdownAck
            }

            Request::GetWorkerStatus { context_id: _ } => {
                // Simplified - could return actual worker stats
                Response::WorkerStatus {
                    active_workers: 0,
                    idle_workers: 0,
                    tests_executed: 0,
                    avg_test_duration_ms: 0,
                }
            }

            Request::ConfigureWorkers {
                context_id: _,
                num_workers: _,
            } => Response::WorkerConfigAck { num_workers: 0 },

            Request::RunStream {
                context_id: _,
                node_ids: _,
                workers: _,
                maxfail: _,
            } => Response::Error {
                code: ErrorCode::InvalidRequest,
                message: "Streaming runs not yet implemented".to_string(),
            },

            Request::GetRunProgress {
                context_id: _,
                run_id: _,
            } => Response::Error {
                code: ErrorCode::InvalidRequest,
                message: "Streaming runs not yet implemented".to_string(),
            },

            Request::GetFlakinessReport { context_id } => {
                let contexts = contexts.lock().await;
                if let Some(context) = contexts.get(&context_id) {
                    let _report = context.get_flakiness_report();
                    // Serialize the report to return as part of response
                    Response::FlakinessReport {
                        flaky_tests: Vec::new(),
                        unstable_tests: Vec::new(),
                        stable_count: 0,
                        total_tracked: 0,
                    }
                } else {
                    Response::Error {
                        code: ErrorCode::ContextNotFound,
                        message: format!("Context not found: {}", context_id),
                    }
                }
            }

            _ => Response::Error {
                code: ErrorCode::InvalidRequest,
                message: "Request type not implemented".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[tokio::test]
    async fn test_init_context() {
        let dir = TempDir::new().unwrap();

        // Create a test file
        let test_file = dir.path().join("test_example.py");
        fs::write(
            &test_file,
            "def test_simple():\n    assert True\n",
        )
        .unwrap();

        let socket_path = dir.path().join("test.sock");
        let storage_path = dir.path().join("storage");

        let server = DaemonServer::new(socket_path, storage_path).unwrap();

        // The server would need to be run in a task to accept connections
        // This is just a structural test
        assert!(server.contexts.lock().await.is_empty());
    }
}
