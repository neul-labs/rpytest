//! Daemon lifecycle management.
//!
//! Handles spawning, connecting to, and managing the pytest daemon process.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use rpytest_ipc::{transport::DaemonClient, IpcError};
use tracing::{debug, info, warn};

/// Manages the daemon lifecycle and provides a client connection.
pub struct DaemonManager {
    socket_path: PathBuf,
    idle_timeout: u64,
    storage_path: Option<PathBuf>,
    client: Option<DaemonClient>,
}

impl DaemonManager {
    /// Create a new daemon manager with the default socket path.
    pub fn new() -> Self {
        Self {
            socket_path: rpytest_ipc::default_socket_path(),
            idle_timeout: 300, // Default 5 minute idle timeout
            storage_path: None,
            client: None,
        }
    }

    /// Create a new daemon manager with a custom socket path.
    pub fn with_socket_path(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            idle_timeout: 300,
            storage_path: None,
            client: None,
        }
    }

    /// Set the idle timeout for auto-spawned daemons.
    pub fn with_idle_timeout(mut self, timeout: u64) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Set the storage path for auto-spawned daemons.
    pub fn with_storage_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.storage_path = Some(path.into());
        self
    }

    /// Override the storage path in-place.
    pub fn set_storage_path(&mut self, path: Option<PathBuf>) {
        self.storage_path = path;
    }

    /// Get the socket path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Check if the daemon is currently running.
    pub async fn is_running(&self) -> bool {
        rpytest_ipc::transport::is_daemon_running(&self.socket_path).await
    }

    /// Connect to the daemon, starting it if necessary.
    ///
    /// Returns a mutable reference to the connected client.
    pub async fn connect(&mut self) -> Result<&mut DaemonClient> {
        // If already connected, return the existing client
        if let Some(ref mut client) = self.client {
            return Ok(client);
        }

        // Try to connect to existing daemon
        match DaemonClient::connect(&self.socket_path).await {
            Ok(client) => {
                info!("Connected to existing daemon");
                self.client = Some(client);
                // SAFETY: We just set self.client, so this unwrap is safe
                return Ok(self
                    .client
                    .as_mut()
                    .expect("Client should be set after connect"));
            }
            Err(IpcError::DaemonNotRunning(_)) => {
                debug!("Daemon not running, will attempt to spawn");
            }
            Err(e) => {
                warn!("Failed to connect to daemon: {}", e);
            }
        }

        // Spawn daemon and retry connection with exponential backoff
        self.spawn_daemon().await?;

        // Exponential backoff: start at 50ms, double each attempt, max 1s
        let max_retries = 10;
        let base_delay = Duration::from_millis(50);
        let max_delay = Duration::from_secs(1);

        for i in 0..max_retries {
            // Calculate delay with exponential backoff
            let delay = std::cmp::min(base_delay * (2_u32.pow(i)), max_delay);
            debug!(
                "Connection attempt {} of {} (delay: {:?})",
                i + 1,
                max_retries,
                delay
            );

            match DaemonClient::connect(&self.socket_path).await {
                Ok(client) => {
                    info!("Connected to daemon after spawn");
                    self.client = Some(client);
                    // SAFETY: We just set self.client, so this unwrap is safe
                    return Ok(self
                        .client
                        .as_mut()
                        .expect("Client should be set after connect"));
                }
                Err(e) => {
                    debug!("Connection attempt failed: {}", e);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        anyhow::bail!(
            "Failed to connect to daemon at {} after {} attempts",
            self.socket_path.display(),
            max_retries
        )
    }

    /// Spawn the daemon process.
    async fn spawn_daemon(&self) -> Result<()> {
        info!("Spawning Rust daemon...");

        // Find the rpytest-daemon binary
        let daemon_bin = self.find_daemon_binary()?;
        debug!("Using daemon binary: {}", daemon_bin.display());

        // Build the command to run the daemon
        let socket_path_str = self.socket_path.to_string_lossy();
        let idle_timeout_str = self.idle_timeout.to_string();

        let mut cmd = Command::new(&daemon_bin);
        cmd.args([
            "--socket",
            &socket_path_str,
            "--idle-timeout",
            &idle_timeout_str,
            "-v",
        ]);
        if let Some(storage) = &self.storage_path {
            cmd.arg("--storage");
            cmd.arg(storage);
        }

        // Add PID file path (use same location as LifecycleManager expects)
        // Try XDG_RUNTIME_DIR first, fall back to temp dir
        let pid_file_path = std::env::var("XDG_RUNTIME_DIR")
            .ok()
            .map(|dir| PathBuf::from(dir).join("rpytest.pid"))
            .or_else(|| Some(std::env::temp_dir().join("rpytest.pid")));
        if let Some(ref pid_path) = pid_file_path {
            cmd.arg("--pid-file");
            cmd.arg(pid_path);
        }

        // Spawn detached from parent
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        // On Unix, we can use setsid to fully detach
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Create a new session so the daemon doesn't get signals from the terminal
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }

        let child = cmd.spawn().context("Failed to spawn daemon process")?;
        info!("Daemon spawned with PID {}", child.id());

        // Give the daemon a moment to start (reduced from 200ms for performance)
        tokio::time::sleep(Duration::from_millis(50)).await;

        Ok(())
    }

    /// Find the rpytest-daemon binary.
    fn find_daemon_binary(&self) -> Result<PathBuf> {
        // First, try to find it relative to the rpytest binary
        if let Ok(current_exe) = std::env::current_exe() {
            let current_dir = current_exe.parent().unwrap_or_else(|| Path::new("."));

            // Check for rpytest-daemon in the same directory
            let daemon_path = current_dir.join("rpytest-daemon");
            if daemon_path.exists() {
                return Ok(daemon_path);
            }

            // Also check in target/debug or target/release directories
            let debug_daemon = current_dir.join("rpytest-daemon");
            if debug_daemon.exists() {
                return Ok(debug_daemon);
            }
        }

        // Try using cargo to find the binary (for development)
        let output = Command::new("cargo")
            .args(["build", "-p", "rpytest-daemon", "--bin", "rpytest-daemon"])
            .output();

        match output {
            Ok(o) if o.status.success() => {
                // Try to find the binary in the target directory
                if let Ok(current_exe) = std::env::current_exe() {
                    let current_dir = current_exe.parent().unwrap_or_else(|| Path::new("."));
                    let daemon_path = current_dir.join("rpytest-daemon");
                    if daemon_path.exists() {
                        return Ok(daemon_path);
                    }
                }
            }
            _ => {
                debug!("Failed to build daemon with cargo, continuing with other methods");
            }
        }

        // Fallback: try common locations
        let candidates = [
            PathBuf::from("target/debug/rpytest-daemon"),
            PathBuf::from("target/release/rpytest-daemon"),
            PathBuf::from("/usr/local/bin/rpytest-daemon"),
            PathBuf::from("/usr/bin/rpytest-daemon"),
        ];

        for candidate in candidates {
            if candidate.exists() {
                return Ok(candidate);
            }
        }

        anyhow::bail!(
            "Could not find rpytest-daemon binary. Please ensure it is built and in PATH."
        )
    }

    /// Disconnect from the daemon (doesn't stop the daemon).
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(client) = self.client.take() {
            client
                .close()
                .await
                .context("Failed to close daemon connection")?;
        }
        Ok(())
    }

    /// Request the daemon to shut down.
    pub async fn shutdown_daemon(&mut self) -> Result<()> {
        if let Some(ref mut client) = self.client {
            use rpytest_core::protocol::{Request, Response};

            let response = client
                .send(&Request::Shutdown { context_id: None })
                .await
                .context("Failed to send shutdown request")?;

            match response {
                Response::ShutdownAck => {
                    info!("Daemon acknowledged shutdown");
                }
                Response::Error { code, message } => {
                    warn!(
                        "Daemon returned error on shutdown: {:?} - {}",
                        code, message
                    );
                }
                _ => {
                    warn!("Unexpected response to shutdown request");
                }
            }
        }

        self.disconnect().await
    }
}

impl Default for DaemonManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_socket_path() {
        let manager = DaemonManager::new();
        assert!(manager.socket_path().to_string_lossy().contains("rpytest"));
    }

    #[test]
    fn custom_socket_path() {
        let manager = DaemonManager::with_socket_path("/tmp/custom.sock");
        assert_eq!(manager.socket_path(), Path::new("/tmp/custom.sock"));
    }
}
