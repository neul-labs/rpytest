//! rpytest - Rust-powered, drop-in replacement for pytest.

use anyhow::{Context, Result};
use clap::Parser;
use rpytest_core::protocol::{Request, Response};
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

mod benchmark;
mod cache;
mod cli;
mod config;
mod daemon;
mod editor;
mod verify;
mod watch;

use cli::args::Cli;
use cli::output::Output;
use daemon::DaemonManager;

fn main() -> Result<()> {
    // Parse CLI arguments first to get verbosity
    let cli = Cli::parse();

    // Handle version flag early (before logging setup)
    if cli.version {
        println!("rpytest {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Initialize logging based on verbosity
    let log_level = if cli.verbose >= 2 {
        "rpytest=debug"
    } else if cli.verbose >= 1 {
        "rpytest=info"
    } else if cli.quiet >= 1 {
        "rpytest=warn"
    } else {
        "rpytest=info"
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive(log_level.parse().unwrap_or_else(|_| {
                    // Fallback to info level if parsing fails (shouldn't happen with static strings)
                    tracing_subscriber::filter::Directive::from(tracing::Level::INFO)
                })),
        )
        .with_target(false)
        .init();

    // Run async runtime
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async_main(cli))
}

async fn async_main(cli: Cli) -> Result<()> {
    debug!("rpytest starting");

    // Load configuration
    let root = match cli.rootdir.clone() {
        Some(dir) => dir,
        None => std::env::current_dir().context("Failed to get current directory")?,
    };

    let config = config::load_config(&root)?;

    // Merge CLI args with config
    let effective_cli = cli.merge_with_config(&config);

    // Handle special commands
    if effective_cli.daemon {
        return handle_daemon_mode(&effective_cli).await;
    }

    if effective_cli.daemon_status {
        return handle_daemon_status(&effective_cli).await;
    }

    if effective_cli.daemon_stop {
        return handle_daemon_stop(&effective_cli).await;
    }

    if effective_cli.cleanup {
        return handle_cleanup(&effective_cli, &root).await;
    }

    if effective_cli.collect_only {
        return handle_collect_only(&effective_cli, &root).await;
    }

    if effective_cli.verify_dropin {
        return handle_verify_dropin(&effective_cli, &root).await;
    }

    if effective_cli.inventory_status {
        return handle_inventory_status(&effective_cli, &root).await;
    }

    if effective_cli.watch {
        return handle_watch(&effective_cli, &root).await;
    }

    // Normal test run
    handle_run(&effective_cli, &root).await
}

async fn handle_collect_only(cli: &Cli, root: &std::path::Path) -> Result<()> {
    let output = Output::new(cli.verbose, cli.quiet);

    // Connect to daemon
    let mut manager = DaemonManager::new();
    output.info("Connecting to daemon...");

    let client = match manager.connect().await {
        Ok(c) => c,
        Err(e) => {
            output.error(&format!("Failed to connect to daemon: {}", e));
            return Err(e);
        }
    };

    // Initialize context
    let repo_path = root.to_string_lossy().to_string();
    output.info(&format!("Initializing context for {}", repo_path));

    let response = client
        .send(&Request::InitContext {
            repo_path,
            python_path: None,
        })
        .await?;

    let context_id = match response {
        Response::ContextReady { context_id, inventory_hash } => {
            output.info(&format!("Context ready: {} (hash: {})", context_id, inventory_hash));
            context_id
        }
        Response::Error { code, message } => {
            output.error(&format!("Failed to initialize context: {:?} - {}", code, message));
            anyhow::bail!("Context initialization failed: {}", message);
        }
        _ => {
            output.error("Unexpected response from daemon");
            anyhow::bail!("Unexpected response");
        }
    };

    // List tests
    let response = client
        .send(&Request::List {
            context_id,
            keyword: cli.keyword.clone(),
            marker: cli.marker.clone(),
        })
        .await?;

    match response {
        Response::TestList { node_ids } => {
            // Apply path filtering if paths were specified
            let filtered_ids = if cli.paths.is_empty() {
                node_ids
            } else {
                filter_by_paths(&node_ids, &cli.paths)
            };
            output.info(&format!("Collected {} tests:", filtered_ids.len()));
            for node_id in &filtered_ids {
                println!("  {}", node_id);
            }
        }
        Response::Error { code, message } => {
            output.error(&format!("Failed to list tests: {:?} - {}", code, message));
        }
        _ => {
            output.error("Unexpected response from daemon");
        }
    }

    manager.disconnect().await?;
    Ok(())
}

async fn handle_verify_dropin(cli: &Cli, root: &std::path::Path) -> Result<()> {
    let output = Output::new(cli.verbose, cli.quiet);
    output.header("Drop-in Compatibility Verification");

    let config = verify::VerifyConfig {
        root: root.to_path_buf(),
        python: "python3".to_string(),
        pytest_args: cli.paths.clone(),
        strict_output: cli.verbose >= 2,
        timeout_secs: 300,
    };

    output.info("Running pytest...");
    let result = verify::verify_dropin(&config)?;

    // Display results
    output.info(&format!(
        "pytest:  {} tests in {:.2}s",
        result.pytest.tests_collected,
        result.pytest.duration.as_secs_f64()
    ));
    output.info(&format!(
        "rpytest: {} tests in {:.2}s",
        result.rpytest.tests_collected,
        result.rpytest.duration.as_secs_f64()
    ));

    // Show differences
    if !result.diffs.is_empty() {
        println!();
        output.warn(&format!("Found {} differences:", result.diffs.len()));
        for diff in &result.diffs {
            let prefix = if diff.is_critical() { "❌" } else { "⚠️" };
            println!("  {} {}: {} vs {}",
                prefix,
                diff.kind.description(),
                diff.expected,
                diff.actual
            );
            if cli.verbose >= 1 {
                println!("     {}", diff.context);
            }
        }
    }

    println!();
    if result.passed {
        output.info("✅ Verification PASSED - rpytest is compatible with pytest");
    } else {
        output.error("❌ Verification FAILED - compatibility issues detected");
    }

    if !result.passed {
        std::process::exit(1);
    }

    Ok(())
}

async fn handle_inventory_status(cli: &Cli, root: &std::path::Path) -> Result<()> {
    let output = Output::new(cli.verbose, cli.quiet);
    output.header("Inventory Status");

    // Connect to daemon
    let mut manager = DaemonManager::new();
    output.info("Connecting to daemon...");

    let client = match manager.connect().await {
        Ok(c) => c,
        Err(e) => {
            output.error(&format!("Failed to connect to daemon: {}", e));
            return Err(e);
        }
    };

    // Initialize context
    let repo_path = root.to_string_lossy().to_string();
    output.info(&format!("Initializing context for {}", repo_path));

    let response = client
        .send(&Request::InitContext {
            repo_path,
            python_path: None,
        })
        .await?;

    let context_id = match response {
        Response::ContextReady { context_id, inventory_hash } => {
            output.info(&format!("Context ready: {} (hash: {})", context_id, inventory_hash));
            context_id
        }
        Response::Error { code, message } => {
            output.error(&format!("Failed to initialize context: {:?} - {}", code, message));
            anyhow::bail!("Context initialization failed: {}", message);
        }
        _ => {
            output.error("Unexpected response from daemon");
            anyhow::bail!("Unexpected response");
        }
    };

    // Get inventory details
    let response = client
        .send(&Request::GetInventory { context_id: context_id.clone() })
        .await?;

    match response {
        Response::InventoryData { hash, collected_at, nodes } => {
            println!();
            println!("Inventory Hash: {}", hash);
            println!("Collected At:   {}", collected_at);
            println!("Total Tests:    {}", nodes.len());
            println!();

            // Group by file
            let mut by_file: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
            for node in &nodes {
                by_file.entry(node.file_path.clone()).or_default().push(node);
            }

            for (file, file_nodes) in by_file.iter() {
                println!("  {} ({} tests)", file, file_nodes.len());
                if cli.verbose >= 1 {
                    for node in file_nodes {
                        let markers_str = if node.markers.is_empty() {
                            String::new()
                        } else {
                            format!(" [{}]", node.markers.join(", "))
                        };
                        println!("    - {}{}", node.name, markers_str);
                    }
                }
            }
        }
        Response::Error { code, message } => {
            output.error(&format!("Failed to get inventory: {:?} - {}", code, message));
        }
        _ => {
            output.error("Unexpected response from daemon");
        }
    }

    manager.disconnect().await?;
    Ok(())
}

/// Filter node IDs by path patterns specified on command line.
/// Supports:
/// - Exact node IDs: "test_file.py::TestClass::test_method"
/// - File paths: "test_file.py" or "tests/test_file.py"
/// - Directory paths: "tests/"
/// - Partial matches: "test_file.py::TestClass" (matches all methods in class)
fn filter_by_paths(node_ids: &[String], paths: &[String]) -> Vec<String> {
    if paths.is_empty() {
        return node_ids.to_vec();
    }

    node_ids
        .iter()
        .filter(|node_id| {
            paths.iter().any(|path| {
                // Exact match (full node ID specified)
                if node_id.as_str() == path {
                    return true;
                }
                // Node ID starts with path (e.g., path="test.py::TestClass" matches "test.py::TestClass::test_method")
                if node_id.starts_with(path) {
                    return true;
                }
                // File path match (e.g., path="test_foo.py" matches "test_foo.py::test_bar")
                if let Some(file_part) = node_id.split("::").next() {
                    if file_part == path || file_part.ends_with(&format!("/{}", path)) {
                        return true;
                    }
                    // Directory match
                    if path.ends_with('/') && file_part.starts_with(path) {
                        return true;
                    }
                    // Also check without trailing slash
                    if file_part.starts_with(&format!("{}/", path)) {
                        return true;
                    }
                }
                false
            })
        })
        .cloned()
        .collect()
}

async fn handle_run(cli: &Cli, root: &std::path::Path) -> Result<()> {
    let output = Output::new(cli.verbose, cli.quiet);

    if !cli.no_header {
        output.header(&format!("rpytest {}", env!("CARGO_PKG_VERSION")));
    }

    // Initialize cache manager
    let mut cache = cache::CacheManager::new(root);

    // Connect to daemon
    let mut manager = DaemonManager::new();
    info!("Connecting to daemon...");

    let client = match manager.connect().await {
        Ok(c) => c,
        Err(e) => {
            output.error(&format!("Failed to connect to daemon: {}", e));
            return Err(e);
        }
    };

    // Initialize context
    let repo_path = root.to_string_lossy().to_string();
    debug!("Initializing context for {}", repo_path);

    let response = client
        .send(&Request::InitContext {
            repo_path,
            python_path: None,
        })
        .await?;

    let (context_id, inventory_hash) = match response {
        Response::ContextReady { context_id, inventory_hash } => {
            debug!("Context ready: {} (hash: {})", context_id, inventory_hash);
            (context_id, inventory_hash)
        }
        Response::Error { code, message } => {
            output.error(&format!("Failed to initialize context: {:?} - {}", code, message));
            anyhow::bail!("Context initialization failed: {}", message);
        }
        _ => {
            output.error("Unexpected response from daemon");
            anyhow::bail!("Unexpected response");
        }
    };

    // Check cache validity and get filtered tests
    let all_node_ids = if cli.keyword.is_some() || cli.marker.is_some() {
        // We have filters - try to use cache for Rust-side filtering
        let cache_valid = cache
            .is_cache_valid(&context_id, &inventory_hash)
            .unwrap_or(false);

        if cache_valid {
            debug!("Using cached inventory for filtering");
            cache
                .filter_tests(
                    &context_id,
                    cli.keyword.as_deref(),
                    cli.marker.as_deref(),
                )
                .unwrap_or_default()
        } else {
            debug!("Cache miss - fetching inventory from daemon");
            // Fetch inventory from daemon
            let response = client
                .send(&Request::GetInventory {
                    context_id: context_id.clone(),
                })
                .await?;

            match response {
                Response::InventoryData { hash, collected_at, nodes } => {
                    // Save to cache
                    if let Err(e) = cache.save_inventory(&context_id, &hash, collected_at, &nodes) {
                        debug!("Failed to save inventory to cache: {}", e);
                    }

                    // Filter tests locally
                    let filtered: Vec<String> = nodes
                        .iter()
                        .filter(|n| {
                            let keyword_match = cli.keyword.as_ref().map_or(true, |k| {
                                n.node_id.to_lowercase().contains(&k.to_lowercase())
                                    || n.name.to_lowercase().contains(&k.to_lowercase())
                            });
                            let marker_match = cli.marker.as_ref().map_or(true, |m| {
                                n.markers.iter().any(|marker| marker == m)
                            });
                            keyword_match && marker_match
                        })
                        .map(|n| n.node_id.clone())
                        .collect();
                    filtered
                }
                Response::Error { code, message } => {
                    output.error(&format!("Failed to get inventory: {:?} - {}", code, message));
                    anyhow::bail!("GetInventory failed: {}", message);
                }
                _ => {
                    // Fallback to daemon-side filtering
                    let response = client
                        .send(&Request::List {
                            context_id: context_id.clone(),
                            keyword: cli.keyword.clone(),
                            marker: cli.marker.clone(),
                        })
                        .await?;

                    match response {
                        Response::TestList { node_ids } => node_ids,
                        _ => vec![],
                    }
                }
            }
        }
    } else {
        // No filters - just get all tests from daemon
        let response = client
            .send(&Request::List {
                context_id: context_id.clone(),
                keyword: None,
                marker: None,
            })
            .await?;

        match response {
            Response::TestList { node_ids } => node_ids,
            Response::Error { code, message } => {
                output.error(&format!("Failed to list tests: {:?} - {}", code, message));
                anyhow::bail!("List failed: {}", message);
            }
            _ => {
                output.error("Unexpected response from daemon");
                anyhow::bail!("Unexpected response");
            }
        }
    };

    // Apply path filtering if paths were specified on command line
    let node_ids = if cli.paths.is_empty() {
        all_node_ids
    } else {
        let filtered = filter_by_paths(&all_node_ids, &cli.paths);
        debug!(
            "Filtered {} tests to {} based on paths: {:?}",
            all_node_ids.len(),
            filtered.len(),
            cli.paths
        );
        filtered
    };

    if node_ids.is_empty() {
        output.warn("No tests found matching the criteria");
        manager.disconnect().await?;
        return Ok(());
    }

    output.info(&format!("Running {} tests...", node_ids.len()));

    // Run tests
    let response = client
        .send(&Request::Run {
            context_id,
            node_ids,
            workers: cli.workers.as_ref().and_then(|w| w.parse().ok()),
            maxfail: cli.maxfail,
        })
        .await?;

    match response {
        Response::RunComplete {
            total: _,
            passed,
            failed,
            skipped,
            errors,
            duration_ms,
        } => {
            output.newline();
            output.summary(passed, failed, skipped, errors, duration_ms as f64 / 1000.0);

            // Set exit code based on results
            if failed > 0 || errors > 0 {
                std::process::exit(1);
            }
        }
        Response::Error { code, message } => {
            output.error(&format!("Test run failed: {:?} - {}", code, message));
            std::process::exit(2);
        }
        _ => {
            output.error("Unexpected response from daemon");
            std::process::exit(3);
        }
    }

    manager.disconnect().await?;
    Ok(())
}

async fn handle_daemon_mode(cli: &Cli) -> Result<()> {
    use std::process::{Command, Stdio};

    let output = Output::new(cli.verbose, cli.quiet);
    output.header("rpytest daemon");

    // Find Python interpreter (same logic as DaemonManager)
    let python = find_python()?;
    debug!("Using Python: {}", python.display());

    // Get socket path
    let socket_path = rpytest_ipc::default_socket_path();
    let socket_path_str = socket_path.to_string_lossy();

    output.info(&format!("Socket: {}", socket_path_str));
    output.info(&format!("Idle timeout: {}s", cli.daemon_idle_timeout));

    // Build args
    let mut args = vec![
        "-m".to_string(),
        "rpytest_daemon.cli".to_string(),
        "--socket".to_string(),
        socket_path_str.to_string(),
    ];

    // Add idle timeout
    if cli.daemon_idle_timeout > 0 {
        args.push("--idle-timeout".to_string());
        args.push(cli.daemon_idle_timeout.to_string());
    }

    // Add verbosity
    if cli.verbose >= 2 {
        args.push("-vv".to_string());
    } else if cli.verbose >= 1 {
        args.push("-v".to_string());
    }

    output.info("Starting daemon...");

    // Run in foreground (blocking)
    let status = Command::new(&python)
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("Failed to run daemon")?;

    if !status.success() {
        if let Some(code) = status.code() {
            std::process::exit(code);
        }
        std::process::exit(1);
    }

    Ok(())
}

/// Find Python interpreter (shared logic).
fn find_python() -> Result<std::path::PathBuf> {
    use std::path::PathBuf;

    // Check VIRTUAL_ENV
    if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
        let python = PathBuf::from(venv).join("bin/python");
        if python.exists() {
            return Ok(python);
        }
    }

    // Check PYTHON env var
    if let Ok(python) = std::env::var("PYTHON") {
        let path = PathBuf::from(&python);
        if path.exists() || which::which(&python).is_ok() {
            return Ok(path);
        }
    }

    // Look for .venv in current directory and parents
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir = Some(cwd.as_path());
        while let Some(d) = dir {
            let venv_python = d.join(".venv/bin/python");
            if venv_python.exists() {
                return Ok(venv_python);
            }
            dir = d.parent();
        }
    }

    // Try system Python
    for name in &["python3", "python"] {
        if let Ok(path) = which::which(name) {
            return Ok(path);
        }
    }

    anyhow::bail!("Could not find Python interpreter. Please ensure Python 3.9+ is installed.")
}

async fn handle_daemon_status(cli: &Cli) -> Result<()> {
    use daemon::{LifecycleManager, LifecycleConfig, DaemonState};

    let output = Output::new(cli.verbose, cli.quiet);
    output.header("Daemon Status");

    let config = LifecycleConfig::default();
    let mut manager = LifecycleManager::new(config.clone());

    // Get daemon state
    let state = manager.state();
    let state_str = match state {
        DaemonState::Stopped => "Stopped",
        DaemonState::Starting => "Starting",
        DaemonState::Running => "Running",
        DaemonState::Unhealthy => "Unhealthy",
        DaemonState::ShuttingDown => "Shutting Down",
    };

    println!("Status:      {}", state_str);
    println!("Socket:      {}", config.socket_path.display());
    println!("PID file:    {}", config.pid_file.display());

    if let Some(info) = manager.info() {
        println!("PID:         {}", info.pid);
        println!("Uptime:      {}s", info.uptime_secs);
        println!("Restarts:    {}", info.restart_count);
    }

    // Perform health check
    let health = manager.health_check();
    println!();
    println!("Health Check:");
    println!("  Healthy:   {}", if health.healthy { "Yes" } else { "No" });
    println!("  Response:  {}ms", health.response_time_ms);
    if let Some(error) = &health.error {
        println!("  Error:     {}", error);
    }

    // Try to ping daemon via IPC if running
    if state == DaemonState::Running {
        let mut daemon_manager = DaemonManager::new();
        if let Ok(client) = daemon_manager.connect().await {
            let response = client.send(&Request::Ping).await;
            if let Ok(Response::Pong) = response {
                println!();
                println!("IPC Connection: OK");
            }
        }
    }

    Ok(())
}

async fn handle_daemon_stop(cli: &Cli) -> Result<()> {
    use daemon::{LifecycleManager, LifecycleConfig};

    let output = Output::new(cli.verbose, cli.quiet);
    output.header("Stopping Daemon");

    // First try graceful shutdown via IPC
    let mut daemon_manager = DaemonManager::new();
    if daemon_manager.is_running().await {
        output.info("Sending shutdown request...");
        if let Err(e) = daemon_manager.shutdown_daemon().await {
            output.warn(&format!("Graceful shutdown failed: {}", e));
        }
    }

    // Use lifecycle manager for cleanup
    let config = LifecycleConfig::default();
    let mut manager = LifecycleManager::new(config);

    if manager.is_running() {
        output.info("Stopping daemon process...");
        manager.stop()?;
    }

    output.info("Daemon stopped");
    Ok(())
}

async fn handle_cleanup(cli: &Cli, root: &std::path::Path) -> Result<()> {
    use daemon::{ContextCleaner, LifecycleConfig};

    let output = Output::new(cli.verbose, cli.quiet);
    output.header("Cleanup");

    // Determine cache directory
    let cache_dir = root.join(".rpytest");

    output.info(&format!("Cache directory: {}", cache_dir.display()));
    output.info(&format!("Max age: {} seconds", cli.cleanup_max_age));

    // Create cleaner and run
    let cleaner = ContextCleaner::new(&cache_dir, cli.cleanup_max_age);
    let result = cleaner.cleanup()?;

    println!();
    println!("Cleanup Results:");
    println!("  Removed:   {} context(s)", result.removed);
    println!("  Kept:      {} context(s)", result.kept);
    println!("  Errors:    {}", result.errors);
    println!("  Freed:     {} bytes", result.bytes_freed);

    // Also clean up runtime files if requested
    let config = LifecycleConfig::default();
    if config.socket_path.exists() {
        if !daemon::LifecycleManager::new(config.clone()).is_running() {
            std::fs::remove_file(&config.socket_path).ok();
            output.info(&format!("Removed stale socket: {}", config.socket_path.display()));
        }
    }

    if result.removed > 0 {
        output.info(&format!("Cleaned up {} stale context(s)", result.removed));
    } else {
        output.info("No stale contexts to clean up");
    }

    Ok(())
}

async fn handle_watch(cli: &Cli, root: &std::path::Path) -> Result<()> {
    use std::time::Duration;
    use watch::{FileWatcher, DependencyGraph, WatchEventKind, filter_test_files};

    let output = Output::new(cli.verbose, cli.quiet);
    output.header(&format!("rpytest {} [watch mode]", env!("CARGO_PKG_VERSION")));

    // Initialize file watcher
    let watcher = match FileWatcher::new(root, 200) {
        Ok(w) => w,
        Err(e) => {
            output.error(&format!("Failed to start file watcher: {}", e));
            return Err(anyhow::anyhow!("Watch setup failed"));
        }
    };

    output.info(&format!("Watching {} for changes...", root.display()));
    output.info("Press Ctrl+C to stop");

    // Connect to daemon
    let mut manager = DaemonManager::new();
    let client = match manager.connect().await {
        Ok(c) => c,
        Err(e) => {
            output.error(&format!("Failed to connect to daemon: {}", e));
            return Err(e);
        }
    };

    // Initialize context
    let repo_path = root.to_string_lossy().to_string();
    let response = client
        .send(&Request::InitContext {
            repo_path: repo_path.clone(),
            python_path: None,
        })
        .await?;

    let context_id = match response {
        Response::ContextReady { context_id, .. } => context_id,
        Response::Error { code, message } => {
            output.error(&format!("Failed to initialize context: {:?} - {}", code, message));
            anyhow::bail!("Context initialization failed");
        }
        _ => anyhow::bail!("Unexpected response"),
    };

    // Build initial dependency graph
    let mut dep_graph = DependencyGraph::new();

    // Get initial inventory
    let response = client
        .send(&Request::GetInventory {
            context_id: context_id.clone(),
        })
        .await?;

    if let Response::InventoryData { nodes, .. } = response {
        for node in &nodes {
            let file_path = std::path::Path::new(&node.file_path);
            dep_graph.add_test(&node.node_id, file_path);
        }
        output.info(&format!("Tracking {} tests", nodes.len()));
    }

    // Run initial test suite
    output.newline();
    output.info("Running initial test suite...");

    let response = client
        .send(&Request::List {
            context_id: context_id.clone(),
            keyword: cli.keyword.clone(),
            marker: cli.marker.clone(),
        })
        .await?;

    let initial_tests = match response {
        Response::TestList { node_ids } => node_ids,
        _ => vec![],
    };

    if !initial_tests.is_empty() {
        let response = client
            .send(&Request::Run {
                context_id: context_id.clone(),
                node_ids: initial_tests,
                workers: cli.workers.as_ref().and_then(|w| w.parse().ok()),
                maxfail: cli.maxfail,
            })
            .await?;

        if let Response::RunComplete { passed, failed, skipped, errors, duration_ms, .. } = response {
            output.summary(passed, failed, skipped, errors, duration_ms as f64 / 1000.0);
        }
    }

    // Main watch loop
    output.newline();
    output.info("Waiting for file changes...");

    loop {
        // Wait for changes with 1 second timeout
        let events = match watcher.wait_for_changes(Some(Duration::from_secs(1))) {
            Some(e) if !e.is_empty() => e,
            _ => continue,
        };

        // Filter to relevant changes
        let changed_files: Vec<_> = events.iter().map(|e| e.path.clone()).collect();

        output.newline();
        output.info(&format!("Detected {} file change(s):", changed_files.len()));
        for path in &changed_files {
            let kind = events
                .iter()
                .find(|e| &e.path == path)
                .map(|e| match e.kind {
                    WatchEventKind::Modified => "modified",
                    WatchEventKind::Created => "created",
                    WatchEventKind::Deleted => "deleted",
                    WatchEventKind::Renamed => "renamed",
                })
                .unwrap_or("changed");
            output.info(&format!("  {} ({})", path.display(), kind));
        }

        // Compute affected tests
        let affected = dep_graph.compute_affected(&changed_files);

        let tests_to_run = if affected.run_all {
            // Re-collect and run all tests
            output.info("Conftest change detected - running all tests");

            // Re-collect
            let _ = client
                .send(&Request::Collect {
                    context_id: context_id.clone(),
                    force: true,
                })
                .await;

            // Get all tests
            let response = client
                .send(&Request::List {
                    context_id: context_id.clone(),
                    keyword: cli.keyword.clone(),
                    marker: cli.marker.clone(),
                })
                .await?;

            match response {
                Response::TestList { node_ids } => node_ids,
                _ => vec![],
            }
        } else if !affected.node_ids.is_empty() {
            affected.node_ids
        } else {
            // Check if any test files changed
            let test_file_changes = filter_test_files(events);
            if !test_file_changes.is_empty() {
                // Re-collect to pick up new tests
                let _ = client
                    .send(&Request::Collect {
                        context_id: context_id.clone(),
                        force: false,
                    })
                    .await;

                // Get tests from changed files
                let response = client
                    .send(&Request::List {
                        context_id: context_id.clone(),
                        keyword: cli.keyword.clone(),
                        marker: cli.marker.clone(),
                    })
                    .await?;

                match response {
                    Response::TestList { node_ids } => {
                        // Filter to tests in changed files
                        let changed_file_names: std::collections::HashSet<_> = test_file_changes
                            .iter()
                            .filter_map(|e| e.path.file_name().and_then(|n| n.to_str()))
                            .collect();

                        node_ids
                            .into_iter()
                            .filter(|nid| {
                                changed_file_names.iter().any(|f| nid.contains(f))
                            })
                            .collect()
                    }
                    _ => vec![],
                }
            } else {
                // Source file changed but no known dependencies
                output.info("No known affected tests - consider running full suite with Ctrl+R");
                continue;
            }
        };

        if tests_to_run.is_empty() {
            output.info("No tests to run");
            continue;
        }

        output.info(&format!("Running {} affected test(s)...", tests_to_run.len()));

        let response = client
            .send(&Request::Run {
                context_id: context_id.clone(),
                node_ids: tests_to_run,
                workers: cli.workers.as_ref().and_then(|w| w.parse().ok()),
                maxfail: cli.maxfail,
            })
            .await?;

        if let Response::RunComplete { passed, failed, skipped, errors, duration_ms, .. } = response {
            output.summary(passed, failed, skipped, errors, duration_ms as f64 / 1000.0);
        }

        output.newline();
        output.info("Waiting for file changes...");
    }
}
