//! Transport layer for daemon communication.
//!
//! Provides async client for connecting to the rpytest daemon over NNG sockets.

use std::path::Path;
use std::time::Duration;

use nng::{
    options::{Options, RecvTimeout, SendTimeout},
    Protocol, Socket,
};
use rpytest_core::protocol::{Request, Response};
use thiserror::Error;
use tracing::{debug, instrument};

use crate::framing::{self, FramingError};

/// Errors that can occur during IPC operations.
#[derive(Debug, Error)]
pub enum IpcError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Daemon not running at {0}")]
    DaemonNotRunning(String),

    #[error("Send failed: {0}")]
    SendFailed(String),

    #[error("Receive failed: {0}")]
    ReceiveFailed(String),

    #[error("Framing error: {0}")]
    Framing(#[from] FramingError),

    #[error("Connection closed unexpectedly")]
    ConnectionClosed,

    #[error("Operation timed out after {0:?}")]
    Timeout(Duration),

    #[error("NNG error: {0}")]
    Nng(#[from] nng::Error),
}

/// Client for communicating with the rpytest daemon.
pub struct DaemonClient {
    socket: Socket,
}

impl DaemonClient {
    /// Connect to the daemon at the given socket path.
    #[instrument(skip_all, fields(path = %path.as_ref().display()))]
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, IpcError> {
        let path = path.as_ref();
        let address = format!("ipc://{}", path.display());

        debug!("Connecting to daemon at {}", address);

        // Run NNG operations in a blocking task since nng is synchronous
        let addr = address.clone();
        let socket = tokio::task::spawn_blocking(move || -> Result<Socket, IpcError> {
            // Create a REQ socket (request-reply client)
            let socket = Socket::new(Protocol::Req0).map_err(|e| {
                IpcError::ConnectionFailed(format!("Failed to create socket: {}", e))
            })?;

            // Set timeouts
            socket
                .set_opt::<RecvTimeout>(Some(Duration::from_secs(30)))
                .map_err(|e| {
                    IpcError::ConnectionFailed(format!("Failed to set recv timeout: {}", e))
                })?;
            socket
                .set_opt::<SendTimeout>(Some(Duration::from_secs(10)))
                .map_err(|e| {
                    IpcError::ConnectionFailed(format!("Failed to set send timeout: {}", e))
                })?;

            // Connect to the daemon
            socket.dial(&addr).map_err(|e| {
                if matches!(e, nng::Error::ConnectionRefused) {
                    IpcError::DaemonNotRunning(addr.clone())
                } else {
                    // Treat other connection errors as "daemon not running"
                    // since the socket file might not exist
                    IpcError::DaemonNotRunning(format!("{}: {}", addr, e))
                }
            })?;

            Ok(socket)
        })
        .await
        .map_err(|e| IpcError::ConnectionFailed(format!("Task join error: {}", e)))??;

        debug!("Connected to daemon");

        Ok(Self { socket })
    }

    /// Send a request and wait for a response.
    #[instrument(skip(self))]
    pub async fn send(&self, request: &Request) -> Result<Response, IpcError> {
        // Encode the request with length prefix
        let frame = framing::encode(request)?;

        // Clone socket for the blocking task
        let socket = self.socket.clone();

        let response_bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, IpcError> {
            // Send the frame
            socket
                .send(&frame)
                .map_err(|(_, e)| IpcError::SendFailed(format!("Send failed: {}", e)))?;

            debug!("Sent request, waiting for response");

            // Receive response
            let msg = socket.recv().map_err(|e| {
                if matches!(e, nng::Error::Closed | nng::Error::ConnectionReset) {
                    IpcError::ConnectionClosed
                } else if matches!(e, nng::Error::TimedOut) {
                    IpcError::Timeout(Duration::from_secs(30))
                } else {
                    IpcError::ReceiveFailed(format!("Receive failed: {}", e))
                }
            })?;

            Ok(msg.to_vec())
        })
        .await
        .map_err(|e| IpcError::ReceiveFailed(format!("Task join error: {}", e)))??;

        // Parse the length-prefixed response
        if response_bytes.len() < 4 {
            return Err(IpcError::Framing(FramingError::InvalidFrame(
                "Response too short for length prefix".to_string(),
            )));
        }

        let len = u32::from_le_bytes(response_bytes[0..4].try_into().unwrap()) as usize;

        if response_bytes.len() < 4 + len {
            return Err(IpcError::Framing(FramingError::InvalidFrame(format!(
                "Response incomplete: expected {} bytes, got {}",
                4 + len,
                response_bytes.len()
            ))));
        }

        let payload = &response_bytes[4..4 + len];
        let response: Response = framing::decode(payload)?;

        debug!(?response, "Received response");

        Ok(response)
    }

    /// Send a request with a timeout.
    pub async fn send_timeout(
        &self,
        request: &Request,
        timeout: Duration,
    ) -> Result<Response, IpcError> {
        tokio::time::timeout(timeout, self.send(request))
            .await
            .map_err(|_| IpcError::Timeout(timeout))?
    }

    /// Check if the daemon is alive with a ping.
    pub async fn ping(&self) -> Result<bool, IpcError> {
        match self.send(&Request::Ping).await? {
            Response::Pong => Ok(true),
            _ => Ok(false),
        }
    }

    /// Close the connection gracefully.
    pub async fn close(self) -> Result<(), IpcError> {
        let socket = self.socket;
        tokio::task::spawn_blocking(move || {
            socket.close();
        })
        .await
        .map_err(|e| IpcError::ConnectionFailed(format!("Close failed: {}", e)))?;
        Ok(())
    }
}

/// Check if the daemon is running at the given socket path.
pub async fn is_daemon_running(path: impl AsRef<Path>) -> bool {
    match DaemonClient::connect(path).await {
        Ok(client) => {
            let ping_result = client.ping().await;
            // Log ping failures for debugging but still return false
            if let Err(ref e) = ping_result {
                debug!("Ping to daemon failed: {}", e);
            }
            let close_result = client.close().await;
            if let Err(ref e) = close_result {
                debug!("Failed to close daemon connection: {}", e);
            }
            ping_result.unwrap_or(false)
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_connect_nonexistent_socket() {
        let tmp = tempdir().unwrap();
        let socket_path = tmp.path().join("nonexistent.sock");

        let result = DaemonClient::connect(&socket_path).await;
        assert!(result.is_err());

        match result {
            Err(IpcError::DaemonNotRunning(_)) => {} // Expected
            Err(e) => panic!("Unexpected error type: {:?}", e),
            Ok(_) => panic!("Should have failed to connect"),
        }
    }

    #[tokio::test]
    async fn test_is_daemon_running_nonexistent() {
        let tmp = tempdir().unwrap();
        let socket_path = tmp.path().join("nonexistent.sock");

        let running = is_daemon_running(&socket_path).await;
        assert!(!running);
    }

    #[test]
    fn test_ipc_error_display() {
        let err = IpcError::ConnectionFailed("test error".to_string());
        assert_eq!(err.to_string(), "Connection failed: test error");

        let err = IpcError::DaemonNotRunning("/tmp/test.sock".to_string());
        assert_eq!(err.to_string(), "Daemon not running at /tmp/test.sock");

        let err = IpcError::Timeout(Duration::from_secs(5));
        assert_eq!(err.to_string(), "Operation timed out after 5s");
    }
}
