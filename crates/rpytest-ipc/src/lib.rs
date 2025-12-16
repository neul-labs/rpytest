//! IPC transport layer for rpytest CLI-daemon communication.
//!
//! This crate provides MessagePack framing over Unix sockets (or TCP on Windows)
//! for communication between the rpytest CLI and the Python daemon.

pub mod framing;
pub mod transport;

pub use framing::{decode, encode};
pub use transport::{DaemonClient, IpcError};

/// Default socket path for the daemon.
#[cfg(unix)]
pub fn default_socket_path() -> std::path::PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(runtime_dir).join("rpytest.sock")
}

#[cfg(windows)]
pub fn default_socket_path() -> String {
    r"\\.\pipe\rpytest".to_string()
}
