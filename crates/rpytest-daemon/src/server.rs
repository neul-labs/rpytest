//! IPC server using NNG for communication with the CLI.

use crate::context::RepoContext;
use crate::error::{DaemonError, Result};
use crate::models::{DaemonConfig, ExecutionMode, TestNode};
use crate::storage::DaemonStorage;
use nng::options::{Options, RecvTimeout};
use nng::{Message, Protocol, Socket};
use parking_lot::Mutex;
use rmp_serde::Deserializer;
use rpytest_core::protocol::{ErrorCode, Request, Response, PROTOCOL_VERSION};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info};
use uuid::Uuid;

/// Type alias for the contexts map - each context is wrapped in Arc<tokio::sync::Mutex>
/// to allow concurrent access without removing/reinserting from the map
type ContextMap = Mutex<HashMap<String, Arc<tokio::sync::Mutex<RepoContext>>>>;

struct RequestJob {
    message: Vec<u8>,
    responder: oneshot::Sender<Vec<u8>>,
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
    /// Server configuration (planned feature)
    #[allow(dead_code)]
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

    /// Start the server in a background thread.
    pub fn run(&mut self) -> Result<()> {
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
        socket.set_opt::<RecvTimeout>(Some(Duration::from_millis(100)))?;

        info!("Daemon listening on {}", self.socket_url);

        // Shared state for shutdown
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();
        let running_for_ctrlc = running.clone();

        let storage = self.storage.clone();
        let runtime_contexts = Arc::clone(&self.contexts);
        let (request_tx, mut request_rx) = mpsc::unbounded_channel::<RequestJob>();

        // Create a tokio runtime for the server thread
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

        // Spawn the processing thread
        thread::spawn(move || {
            runtime.block_on(async move {
                while let Some(job) = request_rx.recv().await {
                    let response_bytes =
                        match Self::process_message(job.message, &storage, &runtime_contexts).await
                        {
                            Ok(buf) => buf,
                            Err(e) => {
                                error!("Processing error: {}", e);
                                let fallback = Response::Error {
                                    code: ErrorCode::InternalError,
                                    message: format!("Daemon error: {}", e),
                                };
                                rpytest_ipc::framing::encode(&fallback).unwrap_or_default()
                            }
                        };

                    if job.responder.send(response_bytes).is_err() {
                        debug!("Socket loop dropped before response send");
                    }
                }
            });
        });

        // Spawn socket loop thread to bridge blocking NNG with async runtime
        let socket_running = running_clone.clone();
        let socket_tx = request_tx.clone();
        thread::spawn(move || {
            let socket = socket;
            while socket_running.load(Ordering::SeqCst) {
                match socket.recv() {
                    Ok(msg) => {
                        let msg_bytes = msg.as_slice().to_vec();
                        let (resp_tx, resp_rx) = oneshot::channel();
                        if socket_tx
                            .send(RequestJob {
                                message: msg_bytes,
                                responder: resp_tx,
                            })
                            .is_err()
                        {
                            error!("Runtime channel closed, stopping socket loop");
                            break;
                        }

                        match resp_rx.blocking_recv() {
                            Ok(response_buf) => {
                                if response_buf.is_empty() {
                                    continue;
                                }
                                if let Err((_, e)) =
                                    socket.send(Message::from(response_buf.as_slice()))
                                {
                                    error!("Failed to send response: {}", e);
                                    break;
                                }
                            }
                            Err(_) => {
                                error!("Failed to receive daemon response");
                                break;
                            }
                        }
                    }
                    Err(nng::Error::TimedOut) | Err(nng::Error::TryAgain) => continue,
                    Err(e) => {
                        error!("Receive error: {}", e);
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        });

        // Wait for Ctrl+C or signal
        if let Err(e) = ctrlc::set_handler(move || {
            running_for_ctrlc.store(false, Ordering::SeqCst);
        }) {
            error!("Failed to set Ctrl+C handler: {}", e);
        }

        // Wait while running
        while running.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(100));
        }

        drop(request_tx);
        info!("Daemon shutting down");
        Ok(())
    }

    /// Process a single message.
    async fn process_message(
        msg_bytes: Vec<u8>,
        storage: &DaemonStorage,
        contexts: &Arc<ContextMap>,
    ) -> Result<Vec<u8>> {
        // Parse length-prefixed framing
        if msg_bytes.len() < 4 {
            return Err(DaemonError::Other(
                "Message too short for length prefix".to_string(),
            ));
        }

        let len =
            u32::from_le_bytes([msg_bytes[0], msg_bytes[1], msg_bytes[2], msg_bytes[3]]) as usize;

        if msg_bytes.len() < 4 + len {
            return Err(DaemonError::Other(format!(
                "Message incomplete: expected {} bytes, got {}",
                4 + len,
                msg_bytes.len()
            )));
        }

        let payload = &msg_bytes[4..4 + len];

        // Deserialize request
        let mut deserializer = Deserializer::new(payload);
        let request: Request = Deserialize::deserialize(&mut deserializer)?;

        // Process request
        let response = Self::process_request(request, storage, contexts).await;

        // Serialize response with length-prefixed framing
        let response_buf = rpytest_ipc::framing::encode(&response)?;

        Ok(response_buf)
    }

    /// Process a single request.
    async fn process_request(
        request: Request,
        storage: &DaemonStorage,
        contexts: &Arc<ContextMap>,
    ) -> Response {
        match request {
            Request::InitContext {
                protocol_version,
                repo_path,
                python_path,
                execution_mode,
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

                // Parse execution mode from request (default to Auto)
                let mode = execution_mode
                    .as_deref()
                    .and_then(|s| s.parse::<ExecutionMode>().ok())
                    .unwrap_or(ExecutionMode::Auto);

                // Generate stable context ID based on repo_path and execution_mode
                // This allows reusing contexts with warm workers across CLI invocations
                let context_key = format!(
                    "{}:{}:{}",
                    repo_path,
                    python_path.as_deref().unwrap_or("auto"),
                    mode
                );
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(context_key.as_bytes());
                let context_id = format!("ctx-{}", hex::encode(&hasher.finalize()[..8]));

                // Check if context already exists
                {
                    let contexts_lock = contexts.lock();
                    if let Some(existing_context) = contexts_lock.get(&context_id) {
                        let context = existing_context.lock().await;
                        let inventory_hash = context.inventory_hash.clone();
                        let execution_mode = context.execution_mode();
                        info!(
                            "Reusing existing context {} with {} execution mode",
                            context_id, execution_mode
                        );
                        return Response::ContextReady {
                            protocol_version: PROTOCOL_VERSION,
                            context_id,
                            inventory_hash,
                        };
                    }
                }

                // Create context with the requested execution mode (async for pooled mode support)
                let context = match RepoContext::new(
                    &context_id,
                    Path::new(&repo_path),
                    python_path.map(std::path::PathBuf::from),
                    Some(storage.clone()),
                    mode,
                )
                .await
                {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        return Response::Error {
                            code: ErrorCode::InternalError,
                            message: format!("Failed to create context: {}", e),
                        };
                    }
                };
                let inventory_hash = context.inventory_hash.clone();
                let execution_mode = context.execution_mode();

                // Store context wrapped in Arc<tokio::sync::Mutex> for concurrent access
                let mut contexts = contexts.lock();
                contexts.insert(
                    context_id.clone(),
                    Arc::new(tokio::sync::Mutex::new(context)),
                );

                info!(
                    "Created new context {} with {} execution mode",
                    context_id, execution_mode
                );

                Response::ContextReady {
                    protocol_version: PROTOCOL_VERSION,
                    context_id,
                    inventory_hash,
                }
            }

            Request::Collect { context_id, force } => {
                // Get the Arc, then release the map lock before locking the context
                let context_arc = {
                    let contexts = contexts.lock();
                    contexts.get(&context_id).cloned()
                };

                if let Some(context_arc) = context_arc {
                    let mut context = context_arc.lock().await;
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
                // Get the Arc, then release the map lock before locking the context
                // This prevents the race condition where context was removed/reinserted
                let context_arc = {
                    let contexts = contexts.lock();
                    contexts.get(&context_id).cloned()
                };

                if let Some(context_arc) = context_arc {
                    let mut context = context_arc.lock().await;
                    let run_result = context.run_tests(&node_ids, workers, maxfail).await;

                    match run_result {
                        Ok(summary) => Response::RunComplete {
                            total: summary.total,
                            passed: summary.passed,
                            failed: summary.failed,
                            skipped: summary.skipped,
                            errors: summary.errors,
                            duration_ms: summary.duration_ms,
                        },
                        Err(e) => Response::Error {
                            code: ErrorCode::InternalError,
                            message: format!("Run failed: {}", e),
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
                let context_arc = {
                    let contexts = contexts.lock();
                    contexts.get(&context_id).cloned()
                };

                if let Some(context_arc) = context_arc {
                    let context = context_arc.lock().await;
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
                let context_arc = {
                    let contexts = contexts.lock();
                    contexts.get(&context_id).cloned()
                };

                if let Some(context_arc) = context_arc {
                    let context = context_arc.lock().await;
                    let nodes: Vec<TestNode> = context.get_inventory();
                    Response::InventoryData {
                        hash: context.inventory_hash.clone(),
                        collected_at: context.last_collection_time as u64,
                        nodes: nodes.into_iter().map(|n| n.into()).collect(),
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
                let mut contexts = contexts.lock();
                if let Some(id) = context_id {
                    contexts.remove(&id);
                } else {
                    contexts.clear();
                }
                Response::ShutdownAck
            }

            Request::GetWorkerStatus { context_id: _ } => Response::WorkerStatus {
                active_workers: 0,
                idle_workers: 0,
                tests_executed: 0,
                avg_test_duration_ms: 0,
            },

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
                let context_arc = {
                    let contexts = contexts.lock();
                    contexts.get(&context_id).cloned()
                };

                if let Some(context_arc) = context_arc {
                    let context = context_arc.lock().await;
                    let _report = context.get_flakiness_report();
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
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_init_context() {
        let dir = TempDir::new().unwrap();

        // Create a test file
        let test_file = dir.path().join("test_example.py");
        fs::write(&test_file, "def test_simple():\n    assert True\n").unwrap();

        let socket_path = dir.path().join("test.sock");
        let storage_path = dir.path().join("storage");

        let server = DaemonServer::new(socket_path, storage_path).unwrap();
        assert!(server.contexts.lock().is_empty());
    }

    #[test]
    fn test_run_request_completes() {
        let dir = TempDir::new().unwrap();
        let repo_root = dir.path().join("repo");
        let tests_dir = repo_root.join("tests");
        std::fs::create_dir_all(&tests_dir).unwrap();
        fs::write(
            tests_dir.join("test_sample.py"),
            "def test_ok():\n    assert True\n",
        )
        .unwrap();

        std::env::set_var("RPYTEST_FAKE_PYTEST", "1");

        let storage_path = repo_root.join("storage");
        let storage = DaemonStorage::open(&storage_path).unwrap();
        let contexts = Arc::new(Mutex::new(HashMap::new()));

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let init_response = DaemonServer::process_request(
                Request::InitContext {
                    protocol_version: PROTOCOL_VERSION,
                    repo_path: repo_root.to_string_lossy().to_string(),
                    python_path: None,
                    execution_mode: Some("subprocess".to_string()),
                },
                &storage,
                &contexts,
            )
            .await;

            let context_id = match init_response {
                Response::ContextReady { context_id, .. } => context_id,
                other => panic!("Unexpected init response: {:?}", other),
            };

            let collect_response = DaemonServer::process_request(
                Request::Collect {
                    context_id: context_id.clone(),
                    force: true,
                },
                &storage,
                &contexts,
            )
            .await;

            match collect_response {
                Response::CollectionComplete { node_count, .. } => assert!(node_count > 0),
                other => panic!("Unexpected collect response: {:?}", other),
            }

            let inventory_response = DaemonServer::process_request(
                Request::GetInventory {
                    context_id: context_id.clone(),
                },
                &storage,
                &contexts,
            )
            .await;

            let nodes = match inventory_response {
                Response::InventoryData { nodes, .. } => nodes,
                other => panic!("Unexpected inventory response: {:?}", other),
            };
            assert!(!nodes.is_empty());

            let node_ids: Vec<String> = nodes.into_iter().map(|node| node.node_id).collect();

            let run_response = DaemonServer::process_request(
                Request::Run {
                    context_id: context_id.clone(),
                    node_ids: node_ids.clone(),
                    workers: Some(1),
                    maxfail: None,
                },
                &storage,
                &contexts,
            )
            .await;

            match run_response {
                Response::RunComplete {
                    total,
                    failed,
                    errors,
                    ..
                } => {
                    assert_eq!(total, node_ids.len());
                    assert_eq!(failed, 0);
                    assert_eq!(errors, 0);
                }
                other => panic!("Unexpected run response: {:?}", other),
            }
        });

        std::env::remove_var("RPYTEST_FAKE_PYTEST");
    }
}
