//! CLI entry point for the rpytest daemon.

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use rpytest_daemon::{DaemonError, DaemonServer};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use tracing::{info, warn};
use tracing_subscriber::{self, EnvFilter};

/// Pure Rust daemon for rpytest - handles test execution, collection, and state management.
#[derive(Parser, Debug)]
#[command(name = "rpytest-daemon", version)]
struct Args {
    /// Socket path for IPC communication.
    #[arg(long = "socket", short = 's')]
    socket: PathBuf,

    /// Storage directory path.
    #[arg(long = "storage", short = 't')]
    storage: Option<PathBuf>,

    /// PID file path.
    #[arg(long = "pid-file")]
    pid_file: Option<PathBuf>,

    /// Idle timeout in seconds (0 = no timeout).
    #[arg(long = "idle-timeout", default_value = "300")]
    idle_timeout: u64,

    /// Verbosity level.
    #[arg(long = "verbose", short = 'v', action = clap::ArgAction::Count)]
    verbose: u8,

    /// Log format.
    #[arg(long = "log-format", value_enum, default_value = "pretty")]
    log_format: LogFormat,
}

#[derive(Copy, Clone, Debug, Default, ValueEnum)]
enum LogFormat {
    #[default]
    Pretty,
    Json,
    Compact,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    let log_level = match args.verbose {
        0 => "rpytest_daemon=info",
        1 => "rpytest_daemon=debug",
        _ => "rpytest_daemon=trace",
    };

    let subscriber = tracing_subscriber::fmt().with_env_filter(
        EnvFilter::from_default_env().add_directive(
            log_level
                .parse()
                .unwrap_or_else(|_| tracing::Level::INFO.into()),
        ),
    );

    match args.log_format {
        LogFormat::Pretty => subscriber.with_target(false).init(),
        LogFormat::Json => subscriber.json().with_target(false).init(),
        LogFormat::Compact => subscriber.compact().with_target(false).init(),
    }

    info!("Starting rpytest daemon");
    let socket_path = args.socket.clone();
    info!("Socket: {}", socket_path.display());

    // Determine storage path, falling back to /tmp if needed
    let storage = resolve_storage_path(args.storage);
    info!("Storage: {}", storage.display());
    info!("Idle timeout: {}s", args.idle_timeout);

    // Determine PID file path
    let pid_file = args.pid_file.or_else(|| {
        // Try XDG_RUNTIME_DIR first, fall back to temp dir
        std::env::var("XDG_RUNTIME_DIR")
            .ok()
            .map(|dir| PathBuf::from(dir).join("rpytest.pid"))
            .or_else(|| Some(std::env::temp_dir().join("rpytest.pid")))
    });

    // Write PID file
    let pid_file_path = if let Some(ref pid_path) = pid_file {
        if let Err(e) = write_pid_file(pid_path) {
            warn!("Failed to write PID file: {}", e);
        } else {
            info!("PID file: {}", pid_path.display());
        }
        Some(pid_path.clone())
    } else {
        None
    };

    // Create and run server synchronously
    let mut server = match DaemonServer::new(socket_path.clone(), storage.clone()) {
        Ok(server) => server,
        Err(DaemonError::Storage(e)) => {
            // Check if this is a lock error (another daemon running)
            let error_msg = e.to_string();
            let is_lock_error = error_msg.contains("lock")
                || error_msg.contains("WouldBlock")
                || error_msg.contains("locked");

            if is_lock_error {
                warn!(
                    "Storage at {} is locked - another daemon may be running. Falling back to temp dir.",
                    storage.display()
                );
            } else {
                warn!(
                    "Failed to initialize storage at {}: {}. Falling back to temp dir.",
                    storage.display(),
                    e
                );
            }
            let fallback = std::env::temp_dir().join("rpytest");
            if let Err(err) = std::fs::create_dir_all(&fallback) {
                return Err(anyhow::anyhow!(
                    "Failed to prepare fallback storage at {}: {}",
                    fallback.display(),
                    err
                ));
            }
            info!(
                "Retrying daemon server with fallback storage: {}",
                fallback.display()
            );
            DaemonServer::new(socket_path, fallback)
                .context("Failed to create daemon server with fallback storage")?
        }
        Err(other) => return Err(other.into()),
    };

    // Handle idle timeout (simplified - just run the server)
    info!("Starting daemon server (Ctrl+C to stop)...");

    // Run the daemon (this blocks until shutdown)
    let result = server.run().context("Daemon server error");

    // Clean up PID file on exit
    if let Some(ref pid_path) = pid_file_path {
        if let Err(e) = remove_pid_file(pid_path) {
            warn!("Failed to remove PID file: {}", e);
        }
    }

    result
}

fn resolve_storage_path(preferred: Option<PathBuf>) -> PathBuf {
    // Candidate order: CLI override, OS cache dir, temp dir, repo-local fallback
    let mut candidates = Vec::new();

    if let Some(path) = preferred {
        candidates.push((path, "CLI override"));
    }

    let default_cache = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("rpytest");
    candidates.push((default_cache, "system cache"));

    candidates.push((std::env::temp_dir().join("rpytest"), "temp dir"));
    candidates.push((
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".rpytest")
            .join("daemon"),
        "workspace fallback",
    ));

    for (path, label) in candidates {
        match std::fs::create_dir_all(&path) {
            Ok(_) => {
                info!("Using storage path ({label}): {}", path.display());
                return path;
            }
            Err(e) => {
                warn!(
                    "Failed to prepare storage dir {} ({label}): {}",
                    path.display(),
                    e
                );
            }
        }
    }

    PathBuf::from("/tmp/rpytest")
}

/// Write the PID file with the current process ID.
fn write_pid_file(pid_path: &PathBuf) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let pid = std::process::id();
    let mut file = fs::File::create(pid_path)?;
    writeln!(file, "{}", pid)?;
    Ok(())
}

/// Remove the PID file.
fn remove_pid_file(pid_path: &PathBuf) -> Result<()> {
    if pid_path.exists() {
        fs::remove_file(pid_path)?;
    }
    Ok(())
}
