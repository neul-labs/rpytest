//! Daemon lifecycle management.
//!
//! Handles spawning, connecting to, and managing the pytest daemon process.

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
    client: Option<DaemonClient>,
}

impl DaemonManager {
    /// Create a new daemon manager with the default socket path.
    pub fn new() -> Self {
        Self {
            socket_path: rpytest_ipc::default_socket_path(),
            idle_timeout: 300, // Default 5 minute idle timeout
            client: None,
        }
    }

    /// Create a new daemon manager with a custom socket path.
    pub fn with_socket_path(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            idle_timeout: 300,
            client: None,
        }
    }

    /// Set the idle timeout for auto-spawned daemons.
    pub fn with_idle_timeout(mut self, timeout: u64) -> Self {
        self.idle_timeout = timeout;
        self
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
        if self.client.is_some() {
            return Ok(self.client.as_mut().unwrap());
        }

        // Try to connect to existing daemon
        match DaemonClient::connect(&self.socket_path).await {
            Ok(client) => {
                info!("Connected to existing daemon");
                self.client = Some(client);
                return Ok(self.client.as_mut().unwrap());
            }
            Err(IpcError::DaemonNotRunning(_)) => {
                debug!("Daemon not running, will attempt to spawn");
            }
            Err(e) => {
                warn!("Failed to connect to daemon: {}", e);
            }
        }

        // Spawn daemon and retry connection
        self.spawn_daemon().await?;

        // Wait for daemon to be ready
        let max_retries = 10;
        let retry_delay = Duration::from_millis(100);

        for i in 0..max_retries {
            debug!("Connection attempt {} of {}", i + 1, max_retries);

            match DaemonClient::connect(&self.socket_path).await {
                Ok(client) => {
                    info!("Connected to daemon after spawn");
                    self.client = Some(client);
                    return Ok(self.client.as_mut().unwrap());
                }
                Err(e) => {
                    debug!("Connection attempt failed: {}", e);
                    tokio::time::sleep(retry_delay).await;
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
        info!("Spawning daemon...");

        // Find Python interpreter
        let python = self.find_python()?;
        debug!("Using Python: {}", python.display());

        // Build the command to run the daemon
        // Use python -m rpytest_daemon.cli to run the daemon module
        let socket_path_str = self.socket_path.to_string_lossy();
        let idle_timeout_str = self.idle_timeout.to_string();

        let mut cmd = Command::new(&python);
        cmd.args([
            "-m",
            "rpytest_daemon.cli",
            "--socket",
            &socket_path_str,
            "--idle-timeout",
            &idle_timeout_str,
            "-v",
        ]);

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

        // Give the daemon a moment to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        Ok(())
    }

    /// Find the Python interpreter to use.
    fn find_python(&self) -> Result<PathBuf> {
        // Check common Python paths in order of preference
        let mut candidates: Vec<Option<PathBuf>> = vec![
            // Virtual environment (if active)
            std::env::var("VIRTUAL_ENV")
                .ok()
                .map(|v| PathBuf::from(v).join("bin/python")),
            // Explicit PYTHON environment variable
            std::env::var("PYTHON").ok().map(PathBuf::from),
        ];

        // Look for .venv in current directory and parent directories
        if let Ok(cwd) = std::env::current_dir() {
            let mut dir = Some(cwd.as_path());
            while let Some(d) = dir {
                let venv_python = d.join(".venv/bin/python");
                if venv_python.exists() {
                    candidates.push(Some(venv_python));
                    break;
                }
                dir = d.parent();
            }
        }

        // Common system paths
        candidates.extend([
            Some(PathBuf::from("python3")),
            Some(PathBuf::from("python")),
            Some(PathBuf::from("/usr/bin/python3")),
            Some(PathBuf::from("/usr/bin/python")),
        ]);

        for candidate in candidates.into_iter().flatten() {
            if candidate.exists() || which::which(&candidate).is_ok() {
                return Ok(candidate);
            }
        }

        anyhow::bail!("Could not find Python interpreter. Please ensure Python 3.9+ is installed.")
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
                    warn!("Daemon returned error on shutdown: {:?} - {}", code, message);
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
