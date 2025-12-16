//! Transport layer for daemon communication.
//!
//! Provides async client for connecting to the rpytest daemon over Unix sockets.

use std::path::Path;
use std::time::Duration;

use rpytest_core::protocol::{Request, Response};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::{debug, instrument};

use crate::framing::{self, FramingError};

/// Errors that can occur during IPC operations.
#[derive(Debug, Error)]
pub enum IpcError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(#[source] std::io::Error),

    #[error("Daemon not running at {0}")]
    DaemonNotRunning(String),

    #[error("Send failed: {0}")]
    SendFailed(#[source] std::io::Error),

    #[error("Receive failed: {0}")]
    ReceiveFailed(#[source] std::io::Error),

    #[error("Framing error: {0}")]
    Framing(#[from] FramingError),

    #[error("Connection closed unexpectedly")]
    ConnectionClosed,

    #[error("Operation timed out after {0:?}")]
    Timeout(Duration),
}

/// Client for communicating with the rpytest daemon.
pub struct DaemonClient {
    stream: UnixStream,
    read_buffer: Vec<u8>,
}

impl DaemonClient {
    /// Connect to the daemon at the given socket path.
    #[instrument(skip_all, fields(path = %path.as_ref().display()))]
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, IpcError> {
        let path = path.as_ref();

        debug!("Connecting to daemon");

        let stream = UnixStream::connect(path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound
                || e.kind() == std::io::ErrorKind::ConnectionRefused
            {
                IpcError::DaemonNotRunning(path.display().to_string())
            } else {
                IpcError::ConnectionFailed(e)
            }
        })?;

        debug!("Connected to daemon");

        Ok(Self {
            stream,
            read_buffer: Vec::with_capacity(4096),
        })
    }

    /// Send a request and wait for a response.
    #[instrument(skip(self))]
    pub async fn send(&mut self, request: &Request) -> Result<Response, IpcError> {
        // Encode and send
        let frame = framing::encode(request)?;
        self.stream
            .write_all(&frame)
            .await
            .map_err(IpcError::SendFailed)?;
        self.stream.flush().await.map_err(IpcError::SendFailed)?;

        debug!("Sent request, waiting for response");

        // Read response
        self.read_response().await
    }

    /// Send a request with a timeout.
    pub async fn send_timeout(
        &mut self,
        request: &Request,
        timeout: Duration,
    ) -> Result<Response, IpcError> {
        tokio::time::timeout(timeout, self.send(request))
            .await
            .map_err(|_| IpcError::Timeout(timeout))?
    }

    /// Read a single response from the stream.
    async fn read_response(&mut self) -> Result<Response, IpcError> {
        // Read length header (4 bytes)
        let mut header = [0u8; 4];
        self.stream
            .read_exact(&mut header)
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    IpcError::ConnectionClosed
                } else {
                    IpcError::ReceiveFailed(e)
                }
            })?;

        let len = u32::from_le_bytes(header) as usize;

        if len > framing::MAX_MESSAGE_SIZE {
            return Err(IpcError::Framing(FramingError::MessageTooLarge(
                len,
                framing::MAX_MESSAGE_SIZE,
            )));
        }

        // Read payload
        self.read_buffer.clear();
        self.read_buffer.resize(len, 0);
        self.stream
            .read_exact(&mut self.read_buffer)
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    IpcError::ConnectionClosed
                } else {
                    IpcError::ReceiveFailed(e)
                }
            })?;

        // Decode
        let response: Response = framing::decode(&self.read_buffer)?;
        debug!(?response, "Received response");

        Ok(response)
    }

    /// Check if the daemon is alive with a ping.
    pub async fn ping(&mut self) -> Result<bool, IpcError> {
        match self.send(&Request::Ping).await? {
            Response::Pong => Ok(true),
            _ => Ok(false),
        }
    }

    /// Close the connection gracefully.
    pub async fn close(mut self) -> Result<(), IpcError> {
        self.stream.shutdown().await.map_err(IpcError::SendFailed)?;
        Ok(())
    }
}

/// Check if the daemon is running at the given socket path.
pub async fn is_daemon_running(path: impl AsRef<Path>) -> bool {
    match DaemonClient::connect(path).await {
        Ok(mut client) => {
            let result = client.ping().await.unwrap_or(false);
            let _ = client.close().await;
            result
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would go here, but require a running daemon
    // or a mock server. For now, we test the framing layer separately.
}
