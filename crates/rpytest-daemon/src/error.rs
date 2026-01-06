//! Error types for the rpytest daemon.

use thiserror::Error;
use rpytest_ipc::transport::IpcError;

#[derive(Error, Debug)]
pub enum DaemonError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] rmp_serde::encode::Error),

    #[error("Deserialization error: {0}")]
    Deserialization(#[from] rmp_serde::decode::Error),

    #[error("Storage error: {0}")]
    Storage(#[from] sled::Error),

    #[error("IPC error: {0}")]
    Ipc(#[from] IpcError),

    #[error("NNG error: {0}")]
    Nng(#[from] nng::Error),

    #[error("Context not found: {0}")]
    ContextNotFound(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Collection failed: {0}")]
    CollectionFailed(String),

    #[error("Python execution failed: {0}")]
    PythonExecution(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("AST parsing error: {0}")]
    AstParsing(String),

    #[error("Watcher error: {0}")]
    Watcher(#[from] notify::Error),

    #[error("Walkdir error: {0}")]
    Walkdir(#[from] walkdir::Error),

    #[error("Other error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, DaemonError>;
