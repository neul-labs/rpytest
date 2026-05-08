//! Daemon lifecycle management with crash recovery.
//!
//! Provides automatic restart, health monitoring, and graceful shutdown.
//! Uses an explicit state machine to track daemon lifecycle transitions.

#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tracing::{debug, error, info, warn};

/// Daemon lifecycle manager configuration.
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    /// Socket path for daemon communication.
    pub socket_path: PathBuf,
    /// PID file path.
    pub pid_file: PathBuf,
    /// Log file path.
    pub log_file: PathBuf,
    /// Maximum restart attempts.
    pub max_restarts: u32,
    /// Restart backoff base (milliseconds).
    pub restart_backoff_ms: u64,
    /// Health check interval (milliseconds).
    pub health_check_interval_ms: u64,
    /// Health check timeout (milliseconds).
    pub health_check_timeout_ms: u64,
    /// Stale context age (seconds).
    pub stale_context_age_secs: u64,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        // Try XDG_RUNTIME_DIR first, fall back to temp dir
        let runtime_path = std::env::var("XDG_RUNTIME_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);

        Self {
            socket_path: runtime_path.join("rpytest.sock"),
            pid_file: runtime_path.join("rpytest.pid"),
            log_file: runtime_path.join("rpytest.log"),
            max_restarts: 3,
            restart_backoff_ms: 1000,
            health_check_interval_ms: 5000,
            health_check_timeout_ms: 2000,
            stale_context_age_secs: 3600, // 1 hour
        }
    }
}

/// Daemon process state.
///
/// State machine transitions:
/// - Stopped -> Starting (start called)
/// - Starting -> Running (process confirmed alive)
/// - Starting -> Unhealthy (process failed during startup)
/// - Running -> Unhealthy (health check failed)
/// - Running -> ShuttingDown (stop called)
/// - Unhealthy -> Starting (recovery attempt)
/// - Unhealthy -> ShuttingDown (stop called)
/// - ShuttingDown -> Stopped (process exited)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    /// Daemon is not running.
    Stopped,
    /// Daemon is starting up.
    Starting,
    /// Daemon is running and healthy.
    Running,
    /// Daemon is unhealthy (not responding).
    Unhealthy,
    /// Daemon is shutting down.
    ShuttingDown,
}

impl DaemonState {
    /// Returns true if the daemon is considered alive (Starting, Running, or Unhealthy).
    pub fn is_alive(&self) -> bool {
        matches!(
            self,
            DaemonState::Starting | DaemonState::Running | DaemonState::Unhealthy
        )
    }

    /// Returns true if the daemon can be started from this state.
    pub fn can_start(&self) -> bool {
        matches!(self, DaemonState::Stopped | DaemonState::Unhealthy)
    }

    /// Returns true if the daemon can be stopped from this state.
    pub fn can_stop(&self) -> bool {
        matches!(
            self,
            DaemonState::Starting | DaemonState::Running | DaemonState::Unhealthy
        )
    }

    /// Validate a state transition.
    ///
    /// Returns true if transitioning from `self` to `next` is valid.
    pub fn can_transition_to(&self, next: DaemonState) -> bool {
        match (self, next) {
            // Stopped can start
            (DaemonState::Stopped, DaemonState::Starting) => true,
            // Starting can become running or unhealthy
            (DaemonState::Starting, DaemonState::Running) => true,
            (DaemonState::Starting, DaemonState::Unhealthy) => true,
            (DaemonState::Starting, DaemonState::Stopped) => true, // startup failure
            // Running can become unhealthy or shutting down
            (DaemonState::Running, DaemonState::Unhealthy) => true,
            (DaemonState::Running, DaemonState::ShuttingDown) => true,
            // Unhealthy can recover (restart) or shut down
            (DaemonState::Unhealthy, DaemonState::Starting) => true,
            (DaemonState::Unhealthy, DaemonState::ShuttingDown) => true,
            // ShuttingDown eventually becomes stopped
            (DaemonState::ShuttingDown, DaemonState::Stopped) => true,
            // Same state is always valid (no-op)
            (current, next) if *current == next => true,
            // Everything else is invalid
            _ => false,
        }
    }
}

/// Events that can trigger state transitions in the lifecycle manager.
#[derive(Debug, Clone)]
pub enum LifecycleEvent {
    /// User requested daemon start.
    StartRequested,
    /// Process startup confirmed (PID file exists and process is alive).
    StartupConfirmed { pid: u32 },
    /// Process startup failed (could not spawn or ready timeout).
    StartupFailed { error: String },
    /// User requested daemon stop.
    StopRequested,
    /// Process exited (detected via PID check).
    ProcessExited { exit_code: Option<i32> },
    /// Health check succeeded.
    HealthCheckPassed,
    /// Health check failed.
    HealthCheckFailed { error: String },
    /// Recovery attempt initiated.
    RecoveryInitiated { attempt: u32 },
}

/// Information about a running daemon.
#[derive(Debug, Clone)]
pub struct DaemonInfo {
    /// Process ID.
    pub pid: u32,
    /// State.
    pub state: DaemonState,
    /// Uptime in seconds.
    pub uptime_secs: u64,
    /// Number of restarts.
    pub restart_count: u32,
    /// Last health check result.
    pub last_health_check: Option<HealthCheckResult>,
}

/// Result of a health check.
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    /// Whether the check passed.
    pub healthy: bool,
    /// Response time in milliseconds.
    pub response_time_ms: u64,
    /// Timestamp of the check.
    pub timestamp: u64,
    /// Error message if unhealthy.
    pub error: Option<String>,
}

/// Daemon lifecycle manager with explicit state machine.
pub struct LifecycleManager {
    config: LifecycleConfig,
    /// Current state in the lifecycle state machine.
    current_state: DaemonState,
    restart_count: u32,
    start_time: Option<Instant>,
    last_health_check: Option<HealthCheckResult>,
    /// Tracked PID for validation.
    tracked_pid: Option<u32>,
}

impl LifecycleManager {
    /// Create a new lifecycle manager.
    pub fn new(config: LifecycleConfig) -> Self {
        Self {
            config,
            current_state: DaemonState::Stopped,
            restart_count: 0,
            start_time: None,
            last_health_check: None,
            tracked_pid: None,
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(LifecycleConfig::default())
    }

    /// Get the current state.
    pub fn state(&self) -> DaemonState {
        self.current_state
    }

    /// Check if daemon is running.
    ///
    /// Validates the tracked PID against reality.
    pub fn is_running(&self) -> bool {
        self.tracked_pid.map(is_process_alive).unwrap_or(false)
    }

    /// Get daemon info.
    pub fn info(&self) -> Option<DaemonInfo> {
        let pid = self.tracked_pid?;
        if !is_process_alive(pid) {
            return None;
        }

        let uptime_secs = self.start_time.map(|t| t.elapsed().as_secs()).unwrap_or(0);

        Some(DaemonInfo {
            pid,
            state: self.current_state,
            uptime_secs,
            restart_count: self.restart_count,
            last_health_check: self.last_health_check.clone(),
        })
    }

    /// Apply a lifecycle event, transitioning state if valid.
    ///
    /// Returns the new state after processing the event.
    fn apply_event(&mut self, event: LifecycleEvent) -> DaemonState {
        let new_state = match (&self.current_state, &event) {
            // Start requested when stopped or unhealthy
            (DaemonState::Stopped, LifecycleEvent::StartRequested) => DaemonState::Starting,
            (DaemonState::Unhealthy, LifecycleEvent::StartRequested) => DaemonState::Starting,

            // Startup confirmed: transition to running
            (DaemonState::Starting, LifecycleEvent::StartupConfirmed { pid }) => {
                self.tracked_pid = Some(*pid);
                self.start_time = Some(Instant::now());
                DaemonState::Running
            }

            // Startup failed
            (DaemonState::Starting, LifecycleEvent::StartupFailed { .. }) => {
                self.tracked_pid = None;
                self.start_time = None;
                DaemonState::Stopped
            }

            // Stop requested from any alive state
            (DaemonState::Starting, LifecycleEvent::StopRequested) => DaemonState::ShuttingDown,
            (DaemonState::Running, LifecycleEvent::StopRequested) => DaemonState::ShuttingDown,
            (DaemonState::Unhealthy, LifecycleEvent::StopRequested) => DaemonState::ShuttingDown,

            // Process exited while shutting down
            (DaemonState::ShuttingDown, LifecycleEvent::ProcessExited { .. }) => {
                self.tracked_pid = None;
                self.start_time = None;
                DaemonState::Stopped
            }

            // Health check outcomes
            (DaemonState::Running, LifecycleEvent::HealthCheckFailed { .. }) => {
                DaemonState::Unhealthy
            }
            (DaemonState::Unhealthy, LifecycleEvent::HealthCheckPassed) => DaemonState::Running,
            (DaemonState::Starting, LifecycleEvent::HealthCheckFailed { .. }) => {
                DaemonState::Unhealthy
            }

            // Recovery from unhealthy
            (DaemonState::Unhealthy, LifecycleEvent::RecoveryInitiated { .. }) => {
                self.restart_count += 1;
                DaemonState::Starting
            }

            // Process unexpectedly exited while running or starting
            (DaemonState::Running, LifecycleEvent::ProcessExited { .. }) => {
                self.tracked_pid = None;
                self.start_time = None;
                DaemonState::Stopped
            }
            (DaemonState::Starting, LifecycleEvent::ProcessExited { .. }) => {
                self.tracked_pid = None;
                self.start_time = None;
                DaemonState::Stopped
            }

            // No-op: health check passed while running, or any event in stopped state
            (current, _) => *current,
        };

        if new_state != self.current_state {
            info!(
                "Daemon lifecycle: {:?} -> {:?} (event: {:?})",
                self.current_state, new_state, event
            );
            self.current_state = new_state;
        } else {
            debug!(
                "Daemon lifecycle: no-op event {:?} in state {:?}",
                event, self.current_state
            );
        }

        self.current_state
    }

    /// Validate tracked PID against reality and update state if process died.
    fn sync_with_reality(&mut self) {
        if let Some(pid) = self.tracked_pid {
            if !is_process_alive(pid) {
                warn!(
                    "Daemon process {} exited unexpectedly (was in {:?} state)",
                    pid, self.current_state
                );
                self.apply_event(LifecycleEvent::ProcessExited { exit_code: None });
            }
        }
    }

    /// Start the daemon.
    pub fn start(&mut self) -> Result<u32> {
        self.sync_with_reality();

        // Check if already running
        if self.current_state.is_alive() {
            if let Some(pid) = self.tracked_pid {
                info!(
                    "Daemon already running with PID {} in {:?} state",
                    pid, self.current_state
                );
                return Ok(pid);
            }
        }

        // Validate transition
        if !self.current_state.can_start() {
            anyhow::bail!("Cannot start daemon from {:?} state", self.current_state);
        }

        self.apply_event(LifecycleEvent::StartRequested);

        // Clean up stale socket/pid files
        self.cleanup_stale_files()?;

        // Start the daemon process
        let pid = match self.spawn_daemon() {
            Ok(pid) => pid,
            Err(e) => {
                self.apply_event(LifecycleEvent::StartupFailed {
                    error: e.to_string(),
                });
                return Err(e);
            }
        };

        self.apply_event(LifecycleEvent::StartupConfirmed { pid });

        info!(
            "Started daemon with PID {} in {:?} state",
            pid, self.current_state
        );
        Ok(pid)
    }

    /// Stop the daemon gracefully.
    pub fn stop(&mut self) -> Result<()> {
        self.sync_with_reality();

        if !self.current_state.can_stop() {
            info!(
                "Cannot stop daemon from {:?} state (already stopped or shutting down)",
                self.current_state
            );
            return Ok(());
        }

        if let Some(pid) = self.tracked_pid {
            if is_process_alive(pid) {
                info!("Stopping daemon (PID {})", pid);
                self.apply_event(LifecycleEvent::StopRequested);

                // Send SIGTERM
                #[cfg(unix)]
                {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }

                // Wait for shutdown (up to 5 seconds)
                let deadline = Instant::now() + Duration::from_secs(5);
                while Instant::now() < deadline {
                    if !is_process_alive(pid) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }

                // Force kill if still running
                if is_process_alive(pid) {
                    warn!("Daemon did not stop gracefully, forcing kill");
                    #[cfg(unix)]
                    {
                        unsafe {
                            libc::kill(pid as i32, libc::SIGKILL);
                        }
                    }
                }
            }
        }

        // Clean up files
        self.cleanup_stale_files()?;

        // Mark as stopped
        self.apply_event(LifecycleEvent::ProcessExited { exit_code: None });

        info!("Daemon stopped");
        Ok(())
    }

    /// Restart the daemon.
    pub fn restart(&mut self) -> Result<u32> {
        self.stop()?;
        std::thread::sleep(Duration::from_millis(500));
        self.restart_count += 1;
        self.start()
    }

    /// Attempt automatic recovery if daemon is unhealthy.
    pub fn recover(&mut self) -> Result<bool> {
        if self.restart_count >= self.config.max_restarts {
            error!(
                "Maximum restart attempts ({}) reached, not recovering",
                self.config.max_restarts
            );
            return Ok(false);
        }

        let backoff = self.config.restart_backoff_ms * (1 << self.restart_count);
        info!(
            "Attempting recovery (attempt {}/{}), backoff {}ms",
            self.restart_count + 1,
            self.config.max_restarts,
            backoff
        );

        std::thread::sleep(Duration::from_millis(backoff));
        self.apply_event(LifecycleEvent::RecoveryInitiated {
            attempt: self.restart_count + 1,
        });
        self.restart()?;

        Ok(true)
    }

    /// Perform a health check.
    pub fn health_check(&mut self) -> HealthCheckResult {
        let start = Instant::now();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Sync with reality first
        self.sync_with_reality();

        // Check if process is alive
        let pid = match self.tracked_pid {
            Some(p) => p,
            None => {
                let result = HealthCheckResult {
                    healthy: false,
                    response_time_ms: 0,
                    timestamp,
                    error: Some("No PID file found".to_string()),
                };
                self.last_health_check = Some(result.clone());
                return result;
            }
        };

        if !is_process_alive(pid) {
            let result = HealthCheckResult {
                healthy: false,
                response_time_ms: 0,
                timestamp,
                error: Some(format!("Process {} not alive", pid)),
            };
            self.last_health_check = Some(result.clone());
            return result;
        }

        // Check socket file exists
        if !self.config.socket_path.exists() {
            let result = HealthCheckResult {
                healthy: false,
                response_time_ms: start.elapsed().as_millis() as u64,
                timestamp,
                error: Some("Socket file not found".to_string()),
            };
            self.last_health_check = Some(result.clone());
            // Transition to unhealthy if we were running
            if self.current_state == DaemonState::Running {
                self.apply_event(LifecycleEvent::HealthCheckFailed {
                    error: "Socket file not found".to_string(),
                });
            }
            return result;
        }

        // Note: A full socket-based ping is available via rpytest_ipc::is_daemon_running()
        // which uses async IPC. For this sync health check, we rely on process + socket file
        // checks which are sufficient for most cases. The async client should be used when
        // actual daemon responsiveness verification is needed (e.g., before running tests).

        let result = HealthCheckResult {
            healthy: true,
            response_time_ms: start.elapsed().as_millis() as u64,
            timestamp,
            error: None,
        };
        self.last_health_check = Some(result.clone());

        // Transition back to running if we were unhealthy
        if self.current_state == DaemonState::Unhealthy {
            self.apply_event(LifecycleEvent::HealthCheckPassed);
        }

        result
    }

    /// Reset restart counter (call after successful operation).
    pub fn reset_restart_count(&mut self) {
        self.restart_count = 0;
    }

    fn spawn_daemon(&self) -> Result<u32> {
        // Find the rpytest-daemon binary
        let daemon_bin = find_daemon_binary()?;

        // Spawn daemon process
        let socket_path = self.config.socket_path.to_string_lossy();
        let child = Command::new(&daemon_bin)
            .args(["--socket", socket_path.as_ref()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to spawn daemon process")?;

        let pid = child.id();

        // Write PID file
        self.write_pid(pid)?;

        // Wait a moment for startup
        std::thread::sleep(Duration::from_millis(500));

        Ok(pid)
    }

    fn read_pid(&self) -> Option<u32> {
        match fs::read_to_string(&self.config.pid_file) {
            Ok(content) => match content.trim().parse() {
                Ok(pid) => Some(pid),
                Err(e) => {
                    debug!("Failed to parse PID from '{}': {}", content.trim(), e);
                    None
                }
            },
            Err(e) => {
                debug!("Failed to read PID file {:?}: {}", self.config.pid_file, e);
                None
            }
        }
    }

    fn write_pid(&self, pid: u32) -> Result<()> {
        let mut file =
            fs::File::create(&self.config.pid_file).context("Failed to create PID file")?;
        write!(file, "{}", pid).context("Failed to write PID")?;
        Ok(())
    }

    fn cleanup_stale_files(&self) -> Result<()> {
        // Remove stale socket
        if self.config.socket_path.exists() {
            fs::remove_file(&self.config.socket_path).context("Failed to remove stale socket")?;
            debug!("Removed stale socket file");
        }

        // Remove stale PID file
        if self.config.pid_file.exists() {
            if let Some(pid) = self.read_pid() {
                if !is_process_alive(pid) {
                    fs::remove_file(&self.config.pid_file)
                        .context("Failed to remove stale PID file")?;
                    debug!("Removed stale PID file");
                }
            }
        }

        Ok(())
    }
}

/// Check if a process is alive.
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) checks if process exists without sending signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    #[cfg(not(unix))]
    {
        // Fallback for non-Unix systems
        false
    }
}

/// Find the rpytest-daemon binary.
fn find_daemon_binary() -> Result<std::path::PathBuf> {
    use std::path::Path;

    // First, try to find it relative to the current executable
    if let Ok(current_exe) = std::env::current_exe() {
        let current_dir = current_exe.parent().unwrap_or_else(|| Path::new("."));

        // Check for rpytest-daemon in the same directory
        let daemon_path = current_dir.join("rpytest-daemon");
        if daemon_path.exists() {
            return Ok(daemon_path);
        }
    }

    // Try common locations
    let candidates = [
        std::path::PathBuf::from("target/debug/rpytest-daemon"),
        std::path::PathBuf::from("target/release/rpytest-daemon"),
        std::path::PathBuf::from("/usr/local/bin/rpytest-daemon"),
        std::path::PathBuf::from("/usr/bin/rpytest-daemon"),
    ];

    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    anyhow::bail!("Could not find rpytest-daemon binary. Please ensure it is built and in PATH.")
}

/// Stale context cleanup.
pub struct ContextCleaner {
    /// Maximum context age in seconds.
    max_age_secs: u64,
    /// Cache directory path.
    cache_dir: PathBuf,
}

impl ContextCleaner {
    /// Create a new context cleaner.
    pub fn new(cache_dir: impl Into<PathBuf>, max_age_secs: u64) -> Self {
        Self {
            max_age_secs,
            cache_dir: cache_dir.into(),
        }
    }

    /// Clean up stale contexts.
    pub fn cleanup(&self) -> Result<CleanupResult> {
        let mut result = CleanupResult::default();

        if !self.cache_dir.exists() {
            return Ok(result);
        }

        let now = SystemTime::now();

        // Iterate through context directories
        for entry in fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            // Check modification time
            if let Ok(metadata) = fs::metadata(&path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age.as_secs() >= self.max_age_secs {
                            // Remove stale context
                            if let Err(e) = fs::remove_dir_all(&path) {
                                warn!("Failed to remove stale context {:?}: {}", path, e);
                                result.errors += 1;
                            } else {
                                result.removed += 1;
                                result.bytes_freed += metadata.len();
                                info!("Removed stale context: {:?}", path);
                            }
                        } else {
                            result.kept += 1;
                        }
                    }
                }
            }
        }

        Ok(result)
    }
}

/// Result of cleanup operation.
#[derive(Debug, Default)]
pub struct CleanupResult {
    /// Number of contexts removed.
    pub removed: usize,
    /// Number of contexts kept.
    pub kept: usize,
    /// Number of errors.
    pub errors: usize,
    /// Bytes freed.
    pub bytes_freed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_lifecycle_config_default() {
        let config = LifecycleConfig::default();
        assert_eq!(config.max_restarts, 3);
        assert!(config
            .socket_path
            .to_string_lossy()
            .contains("rpytest.sock"));
    }

    #[test]
    fn test_daemon_state_transitions() {
        // Valid transitions
        assert!(DaemonState::Stopped.can_transition_to(DaemonState::Starting));
        assert!(DaemonState::Starting.can_transition_to(DaemonState::Running));
        assert!(DaemonState::Starting.can_transition_to(DaemonState::Unhealthy));
        assert!(DaemonState::Running.can_transition_to(DaemonState::Unhealthy));
        assert!(DaemonState::Running.can_transition_to(DaemonState::ShuttingDown));
        assert!(DaemonState::Unhealthy.can_transition_to(DaemonState::Starting));
        assert!(DaemonState::Unhealthy.can_transition_to(DaemonState::ShuttingDown));
        assert!(DaemonState::ShuttingDown.can_transition_to(DaemonState::Stopped));

        // Invalid transitions
        assert!(!DaemonState::Stopped.can_transition_to(DaemonState::Running));
        assert!(!DaemonState::Stopped.can_transition_to(DaemonState::Unhealthy));
        assert!(!DaemonState::Running.can_transition_to(DaemonState::Starting));
        assert!(!DaemonState::Running.can_transition_to(DaemonState::Stopped));
        assert!(!DaemonState::ShuttingDown.can_transition_to(DaemonState::Running));
        assert!(!DaemonState::ShuttingDown.can_transition_to(DaemonState::Unhealthy));
    }

    #[test]
    fn test_daemon_state_helpers() {
        assert!(DaemonState::Starting.is_alive());
        assert!(DaemonState::Running.is_alive());
        assert!(DaemonState::Unhealthy.is_alive());
        assert!(!DaemonState::Stopped.is_alive());
        assert!(!DaemonState::ShuttingDown.is_alive());

        assert!(DaemonState::Stopped.can_start());
        assert!(DaemonState::Unhealthy.can_start());
        assert!(!DaemonState::Starting.can_start());
        assert!(!DaemonState::Running.can_start());

        assert!(DaemonState::Starting.can_stop());
        assert!(DaemonState::Running.can_stop());
        assert!(DaemonState::Unhealthy.can_stop());
        assert!(!DaemonState::Stopped.can_stop());
        assert!(!DaemonState::ShuttingDown.can_stop());
    }

    #[test]
    fn test_lifecycle_manager_initial_state() {
        let manager = LifecycleManager::with_defaults();
        assert_eq!(manager.state(), DaemonState::Stopped);
        assert!(!manager.is_running());
    }

    #[test]
    fn test_lifecycle_event_transitions() {
        let mut manager = LifecycleManager::with_defaults();

        // Stopped -> Starting
        manager.apply_event(LifecycleEvent::StartRequested);
        assert_eq!(manager.state(), DaemonState::Starting);

        // Starting -> Running
        manager.apply_event(LifecycleEvent::StartupConfirmed { pid: 1234 });
        assert_eq!(manager.state(), DaemonState::Running);
        assert_eq!(manager.tracked_pid, Some(1234));

        // Running -> Unhealthy
        manager.apply_event(LifecycleEvent::HealthCheckFailed {
            error: "timeout".to_string(),
        });
        assert_eq!(manager.state(), DaemonState::Unhealthy);

        // Unhealthy -> Running (health check recovered)
        manager.apply_event(LifecycleEvent::HealthCheckPassed);
        assert_eq!(manager.state(), DaemonState::Running);

        // Running -> ShuttingDown
        manager.apply_event(LifecycleEvent::StopRequested);
        assert_eq!(manager.state(), DaemonState::ShuttingDown);

        // ShuttingDown -> Stopped
        manager.apply_event(LifecycleEvent::ProcessExited { exit_code: Some(0) });
        assert_eq!(manager.state(), DaemonState::Stopped);
        assert!(!manager.is_running());
        assert!(manager.tracked_pid.is_none());
    }

    #[test]
    fn test_lifecycle_startup_failure() {
        let mut manager = LifecycleManager::with_defaults();

        manager.apply_event(LifecycleEvent::StartRequested);
        assert_eq!(manager.state(), DaemonState::Starting);

        manager.apply_event(LifecycleEvent::StartupFailed {
            error: "binary not found".to_string(),
        });
        assert_eq!(manager.state(), DaemonState::Stopped);
    }

    #[test]
    fn test_lifecycle_recovery_transition() {
        let mut manager = LifecycleManager::with_defaults();

        // Setup: running -> unhealthy
        manager.apply_event(LifecycleEvent::StartRequested);
        manager.apply_event(LifecycleEvent::StartupConfirmed { pid: 1234 });
        manager.apply_event(LifecycleEvent::HealthCheckFailed {
            error: "timeout".to_string(),
        });
        assert_eq!(manager.state(), DaemonState::Unhealthy);

        // Recovery
        let old_restarts = manager.restart_count;
        manager.apply_event(LifecycleEvent::RecoveryInitiated { attempt: 1 });
        assert_eq!(manager.state(), DaemonState::Starting);
        assert_eq!(manager.restart_count, old_restarts + 1);
    }

    #[test]
    fn test_lifecycle_noop_events() {
        let mut manager = LifecycleManager::with_defaults();

        // Health check passed while stopped should not change state
        manager.apply_event(LifecycleEvent::HealthCheckPassed);
        assert_eq!(manager.state(), DaemonState::Stopped);

        // Start and then health check passed should stay running
        manager.apply_event(LifecycleEvent::StartRequested);
        manager.apply_event(LifecycleEvent::StartupConfirmed { pid: 1234 });
        manager.apply_event(LifecycleEvent::HealthCheckPassed);
        assert_eq!(manager.state(), DaemonState::Running);
    }

    #[test]
    fn test_lifecycle_manager_info_none_when_stopped() {
        let manager = LifecycleManager::with_defaults();
        assert!(manager.info().is_none());
    }

    #[test]
    fn test_health_check_no_daemon() {
        let mut manager = LifecycleManager::with_defaults();
        let result = manager.health_check();
        assert!(!result.healthy);
        assert!(result.error.is_some());
        assert_eq!(manager.state(), DaemonState::Stopped);
    }

    #[test]
    fn test_context_cleaner() {
        let tmp = tempdir().unwrap();
        let cleaner = ContextCleaner::new(tmp.path(), 0); // Immediate expiry

        // Create a fake context directory
        let ctx_dir = tmp.path().join("ctx-0001");
        fs::create_dir(&ctx_dir).unwrap();
        fs::write(ctx_dir.join("data"), "test").unwrap();

        // Sleep briefly to ensure modification time is in the past
        std::thread::sleep(Duration::from_millis(10));

        let result = cleaner.cleanup().unwrap();
        assert_eq!(result.removed, 1);
        assert!(!ctx_dir.exists());
    }
}
