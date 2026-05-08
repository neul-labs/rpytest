//! rpytest - Rust-powered, drop-in replacement for pytest.

// Use mimalloc as the global allocator for better performance
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::{Context, Result};
use clap::Parser;
use rpytest_core::protocol::{Request, Response};
use tracing::{debug, info, warn};
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

/// Parse worker count from CLI argument, logging a warning on failure.
fn parse_workers(cli_workers: &Option<String>) -> Option<u32> {
    match cli_workers {
        Some(ref w) => match w.parse::<u32>() {
            Ok(n) if n > 0 => Some(n),
            Ok(_) => {
                warn!("Worker count must be positive, using auto");
                None
            }
            Err(_) => {
                warn!("Invalid worker count '{}', using auto", w);
                None
            }
        },
        None => None,
    }
}

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
        .with_env_filter(EnvFilter::from_default_env().add_directive(
            log_level.parse().unwrap_or_else(|_| {
                // Fallback to info level if parsing fails (shouldn't happen with static strings)
                tracing_subscriber::filter::Directive::from(tracing::Level::INFO)
            }),
        ))
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
    if let Some(storage) = &cli.daemon_storage {
        manager.set_storage_path(Some(storage.clone()));
    }
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
            protocol_version: rpytest_core::protocol::PROTOCOL_VERSION,
            repo_path,
            python_path: None,
            execution_mode: Some(cli.execution_mode.clone()),
        })
        .await?;

    let context_id = match response {
        Response::ContextReady {
            context_id,
            inventory_hash,
            ..
        } => {
            output.info(&format!(
                "Context ready: {} (hash: {})",
                context_id, inventory_hash
            ));
            context_id
        }
        Response::Error { code, message } => {
            output.error(&format!(
                "Failed to initialize context: {:?} - {}",
                code, message
            ));
            anyhow::bail!("Context initialization failed: {}", message);
        }
        _ => {
            output.error("Unexpected response from daemon");
            anyhow::bail!("Unexpected response");
        }
    };

    // Collect tests (force collection for collect-only mode)
    output.info("Collecting tests...");
    let collect_response = client
        .send(&Request::Collect {
            context_id: context_id.clone(),
            force: true,
        })
        .await?;

    match collect_response {
        Response::CollectionComplete {
            node_count,
            duration_ms,
        } => {
            output.info(&format!(
                "Collected {} tests in {}ms",
                node_count, duration_ms
            ));
        }
        Response::Error { code, message } => {
            output.error(&format!(
                "Failed to collect tests: {:?} - {}",
                code, message
            ));
            anyhow::bail!("Collection failed: {}", message);
        }
        _ => {
            output.error("Unexpected response from daemon during collection");
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
            debug!("Received TestList with {} node_ids", node_ids.len());
            // Apply path filtering if paths were specified
            let filtered_ids = if cli.paths.is_empty() {
                node_ids
            } else {
                filter_by_paths(&node_ids, &cli.paths)
            };
            debug!(
                "Filtered to {} tests (paths: {:?})",
                filtered_ids.len(),
                cli.paths
            );
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
        pytest_args: cli.paths.clone(),
        strict_output: cli.verbose >= 2,
        ..verify::VerifyConfig::default()
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
            println!(
                "  {} {}: {} vs {}",
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
    if let Some(storage) = &cli.daemon_storage {
        manager.set_storage_path(Some(storage.clone()));
    }
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
            protocol_version: rpytest_core::protocol::PROTOCOL_VERSION,
            repo_path,
            python_path: None,
            execution_mode: Some(cli.execution_mode.clone()),
        })
        .await?;

    let context_id = match response {
        Response::ContextReady {
            context_id,
            inventory_hash,
            ..
        } => {
            output.info(&format!(
                "Context ready: {} (hash: {})",
                context_id, inventory_hash
            ));
            context_id
        }
        Response::Error { code, message } => {
            output.error(&format!(
                "Failed to initialize context: {:?} - {}",
                code, message
            ));
            anyhow::bail!("Context initialization failed: {}", message);
        }
        _ => {
            output.error("Unexpected response from daemon");
            anyhow::bail!("Unexpected response");
        }
    };

    // Get inventory details
    let response = client
        .send(&Request::GetInventory {
            context_id: context_id.clone(),
        })
        .await?;

    match response {
        Response::InventoryData {
            hash,
            collected_at,
            nodes,
        } => {
            println!();
            println!("Inventory Hash: {}", hash);
            println!("Collected At:   {}", collected_at);
            println!("Total Tests:    {}", nodes.len());
            println!();

            // Group by file
            let mut by_file: std::collections::HashMap<String, Vec<_>> =
                std::collections::HashMap::new();
            for node in &nodes {
                by_file
                    .entry(node.file_path.clone())
                    .or_default()
                    .push(node);
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
            output.error(&format!(
                "Failed to get inventory: {:?} - {}",
                code, message
            ));
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
/// - File paths: "test_file.py" or "tests/test_file.py" or "./test_file.py"
/// - Directory paths: "tests/" or "tests"
/// - Partial matches: "test_file.py::TestClass" (matches all methods in class)
fn filter_by_paths(node_ids: &[String], paths: &[String]) -> Vec<String> {
    if paths.is_empty() {
        return node_ids.to_vec();
    }

    node_ids
        .iter()
        .filter(|node_id| {
            paths.iter().any(|path| {
                let path = normalize_path(path);

                // Exact match (full node ID specified)
                if node_id.as_str() == path {
                    return true;
                }

                // Node ID starts with path (e.g., path="test.py::TestClass" matches "test.py::TestClass::test_method")
                if node_id.starts_with(&path) {
                    return true;
                }

                // File path match - extract file part from node_id
                if let Some(file_part) = node_id.split("::").next() {
                    let file_part = normalize_path(file_part);

                    // Check if the file part ends with the path (handles relative and absolute paths)
                    if file_part == path {
                        return true;
                    }

                    // Check if file path ends with the requested path
                    // This handles: path="test_file.py" matching "/full/path/test_file.py"
                    if file_part.ends_with(&format!("/{}", path))
                        || path.ends_with(&format!("/{}", file_part))
                    {
                        return true;
                    }

                    // Check if the path is a directory (with or without trailing slash)
                    if path.ends_with('/') || path.ends_with("/.") {
                        let dir_path = path.trim_end_matches('/').trim_end_matches("/.");
                        if file_part == dir_path || file_part.starts_with(&format!("{}/", dir_path)) {
                            return true;
                        }
                    }

                    // Also check if file part starts with the path (for directory matches without trailing slash)
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

/// Normalize a path by removing leading ./ and trailing slashes
fn normalize_path(path: &str) -> String {
    let mut result = path.to_string();

    // Remove leading ./
    if result.starts_with("./") {
        result = result[2..].to_string();
    }

    // Remove all trailing slashes for consistency
    while result.ends_with('/') && result.len() > 1 {
        result.pop();
    }

    result
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
    if let Some(storage) = &cli.daemon_storage {
        manager.set_storage_path(Some(storage.clone()));
    }
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
            protocol_version: rpytest_core::protocol::PROTOCOL_VERSION,
            repo_path,
            python_path: None,
            execution_mode: Some(cli.execution_mode.clone()),
        })
        .await?;

    let (context_id, _initial_hash) = match response {
        Response::ContextReady {
            context_id,
            inventory_hash,
            ..
        } => {
            debug!("Context ready: {} (hash: {})", context_id, inventory_hash);
            (context_id, inventory_hash)
        }
        Response::Error { code, message } => {
            output.error(&format!(
                "Failed to initialize context: {:?} - {}",
                code, message
            ));
            anyhow::bail!("Context initialization failed: {}", message);
        }
        _ => {
            output.error("Unexpected response from daemon");
            anyhow::bail!("Unexpected response");
        }
    };

    // Collect tests if needed (inventory is empty)
    debug!("Getting inventory from daemon");
    let inventory_response = client
        .send(&Request::GetInventory {
            context_id: context_id.clone(),
        })
        .await?;

    let inventory_hash = match inventory_response {
        Response::InventoryData {
            hash,
            collected_at: _,
            nodes,
        } => {
            if nodes.is_empty() {
                // No cached inventory - need to collect
                debug!("Inventory empty, triggering collection");
                output.info("Collecting tests...");

                let collect_response = client
                    .send(&Request::Collect {
                        context_id: context_id.clone(),
                        force: true,
                    })
                    .await?;

                match collect_response {
                    Response::CollectionComplete {
                        node_count,
                        duration_ms,
                    } => {
                        debug!("Collected {} tests in {}ms", node_count, duration_ms);
                        output.info(&format!(
                            "Collected {} tests in {}ms",
                            node_count, duration_ms
                        ));
                    }
                    Response::Error { code, message } => {
                        output.error(&format!(
                            "Failed to collect tests: {:?} - {}",
                            code, message
                        ));
                        anyhow::bail!("Collection failed: {}", message);
                    }
                    _ => {
                        output.error("Unexpected response from daemon during collection");
                        anyhow::bail!("Unexpected response");
                    }
                }
            }
            hash
        }
        Response::Error { code, message } => {
            output.error(&format!(
                "Failed to get inventory: {:?} - {}",
                code, message
            ));
            anyhow::bail!("GetInventory failed: {}", message);
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
                .filter_tests(&context_id, cli.keyword.as_deref(), cli.marker.as_deref())
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
                Response::InventoryData {
                    hash,
                    collected_at,
                    nodes,
                } => {
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
                            let marker_match = cli
                                .marker
                                .as_ref()
                                .map_or(true, |m| n.markers.iter().any(|marker| marker == m));
                            keyword_match && marker_match
                        })
                        .map(|n| n.node_id.clone())
                        .collect();
                    filtered
                }
                Response::Error { code, message } => {
                    output.error(&format!(
                        "Failed to get inventory: {:?} - {}",
                        code, message
                    ));
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
            workers: parse_workers(&cli.workers),
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

    // Get socket path
    let socket_path = rpytest_ipc::default_socket_path();
    let socket_path_str = socket_path.to_string_lossy();

    output.info(&format!("Socket: {}", socket_path_str));
    output.info(&format!("Idle timeout: {}s", cli.daemon_idle_timeout));

    // Find the rpytest-daemon binary
    let daemon_bin = find_daemon_binary()?;
    debug!("Using daemon binary: {}", daemon_bin.display());

    // Build args
    let mut args = vec!["--socket".to_string(), socket_path_str.to_string()];

    // Add idle timeout
    if cli.daemon_idle_timeout > 0 {
        args.push("--idle-timeout".to_string());
        args.push(cli.daemon_idle_timeout.to_string());
    }

    if let Some(storage) = &cli.daemon_storage {
        args.push("--storage".to_string());
        args.push(storage.to_string_lossy().to_string());
    }

    // Add verbosity
    if cli.verbose >= 2 {
        args.push("-vv".to_string());
    } else if cli.verbose >= 1 {
        args.push("-v".to_string());
    }

    output.info("Starting daemon...");

    // Run in foreground (blocking)
    let status = Command::new(&daemon_bin)
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

async fn handle_daemon_status(cli: &Cli) -> Result<()> {
    use daemon::{DaemonState, LifecycleConfig, LifecycleManager};

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
        if let Some(storage) = &cli.daemon_storage {
            daemon_manager.set_storage_path(Some(storage.clone()));
        }
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
    use daemon::{LifecycleConfig, LifecycleManager};

    let output = Output::new(cli.verbose, cli.quiet);
    output.header("Stopping Daemon");

    // First try graceful shutdown via IPC
    let mut daemon_manager = DaemonManager::new();
    if let Some(storage) = &cli.daemon_storage {
        daemon_manager.set_storage_path(Some(storage.clone()));
    }
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
    if config.socket_path.exists()
        && !daemon::LifecycleManager::new(config.clone()).is_running() {
            std::fs::remove_file(&config.socket_path).ok();
            output.info(&format!(
                "Removed stale socket: {}",
                config.socket_path.display()
            ));
        }

    if result.removed > 0 {
        output.info(&format!("Cleaned up {} stale context(s)", result.removed));
    } else {
        output.info("No stale contexts to clean up");
    }

    Ok(())
}

async fn handle_watch(cli: &Cli, root: &std::path::Path) -> Result<()> {
    use std::time::{Duration, Instant};
    use watch::{
        DependencyGraph, FileWatcher, RecollectReason, WatchEvent, WatchEventKind,
        WatchFileEvent, WatchState, WatcherEventKind,
    };

    let output = Output::new(cli.verbose, cli.quiet);
    output.header(&format!(
        "rpytest {} [watch mode]",
        env!("CARGO_PKG_VERSION")
    ));

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
    if let Some(storage) = &cli.daemon_storage {
        manager.set_storage_path(Some(storage.clone()));
    }
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
            protocol_version: rpytest_core::protocol::PROTOCOL_VERSION,
            repo_path: repo_path.clone(),
            python_path: None,
            execution_mode: Some(cli.execution_mode.clone()),
        })
        .await?;

    let context_id = match response {
        Response::ContextReady { context_id, .. } => context_id,
        Response::Error { code, message } => {
            output.error(&format!(
                "Failed to initialize context: {:?} - {}",
                code, message
            ));
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
                workers: parse_workers(&cli.workers),
                maxfail: cli.maxfail,
            })
            .await?;

        if let Response::RunComplete {
            passed,
            failed,
            skipped,
            errors,
            duration_ms,
            ..
        } = response
        {
            output.summary(passed, failed, skipped, errors, duration_ms as f64 / 1000.0);
        }
    }

    // Main watch loop with explicit state machine
    output.newline();
    output.info("Waiting for file changes...");

    let mut state = WatchState::Idle;

    loop {
        // State machine-driven watch loop
        match &state {
            WatchState::Idle => {
                // Wait for changes with 1 second timeout
                let events = match watcher.wait_for_changes(Some(Duration::from_secs(1))) {
                    Some(e) if !e.is_empty() => e,
                    _ => continue,
                };

                // Map watcher events to state machine events
                let file_events: Vec<WatchFileEvent> = events
                    .iter()
                    .map(|e| WatchFileEvent {
                        path: e.path.clone(),
                        kind: match e.kind {
                            WatcherEventKind::Modified => WatchEventKind::Modified,
                            WatcherEventKind::Created => WatchEventKind::Created,
                            WatcherEventKind::Deleted => WatchEventKind::Deleted,
                            WatcherEventKind::Renamed => WatchEventKind::Renamed,
                        },
                    })
                    .collect();

                state = state
                    .transition(&WatchEvent::FileChanges(file_events))
                    .unwrap_or_else(|e| {
                        warn!("Invalid watch state transition: {}", e);
                        WatchState::Idle
                    });
            }
            WatchState::Debouncing { .. } => {
                // Simple debounce: sleep for 300ms and then transition
                tokio::time::sleep(Duration::from_millis(300)).await;
                state = state.transition(&WatchEvent::Debounced).unwrap_or_else(|e| {
                    warn!("Invalid watch state transition: {}", e);
                    WatchState::Idle
                });
            }
            WatchState::ComputingAffected { changed_files } => {
                output.newline();
                output.info(&format!("Detected {} file change(s):", changed_files.len()));
                for path in changed_files {
                    output.info(&format!("  {}", path.display()));
                }

                // Compute affected tests
                let affected = dep_graph.compute_affected(changed_files);

                if affected.run_all {
                    state = WatchState::Recollecting {
                        reason: RecollectReason::ConftestChanged,
                    };
                } else if !affected.node_ids.is_empty() {
                    // Run specific affected tests
                    state = WatchState::Running {
                        test_count: affected.node_ids.len(),
                        start_time: Instant::now(),
                    };

                    output.info(&format!(
                        "Running {} affected test(s)...",
                        affected.node_ids.len()
                    ));

                    let response = client
                        .send(&Request::Run {
                            context_id: context_id.clone(),
                            node_ids: affected.node_ids,
                            workers: parse_workers(&cli.workers),
                            maxfail: cli.maxfail,
                        })
                        .await;

                    match response {
                        Ok(Response::RunComplete {
                            passed,
                            failed,
                            skipped,
                            errors,
                            duration_ms,
                            ..
                        }) => {
                            output.summary(
                                passed,
                                failed,
                                skipped,
                                errors,
                                duration_ms as f64 / 1000.0,
                            );
                        }
                        Ok(_) => {}
                        Err(e) => {
                            output.error(&format!("Run failed: {}", e));
                        }
                    }

                    state = state
                        .transition(&WatchEvent::RunComplete)
                        .unwrap_or(WatchState::Idle);

                    output.newline();
                    output.info("Waiting for file changes...");
                } else {
                    // Check if test files changed
                    let test_file_changes: Vec<_> = changed_files
                        .iter()
                        .filter(|p| {
                            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                            name.starts_with("test_")
                                || name.ends_with("_test.py")
                                || name == "conftest.py"
                        })
                        .cloned()
                        .collect();

                    if !test_file_changes.is_empty() {
                        let changed_file_names: std::collections::HashSet<_> =
                            test_file_changes
                                .iter()
                                .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
                                .map(|s| s.to_string())
                                .collect();

                        state = WatchState::Recollecting {
                            reason: RecollectReason::TestFilesChanged {
                                file_names: changed_file_names,
                            },
                        };
                    } else {
                        output.info(
                            "No known affected tests - consider running full suite with Ctrl+R"
                        );
                        state = WatchState::Idle;
                    }
                }
            }
            WatchState::Recollecting { reason } => {
                match reason {
                    RecollectReason::ConftestChanged => {
                        output.info("Conftest change detected - running all tests");
                    }
                    RecollectReason::TestFilesChanged { file_names } => {
                        output.info(&format!(
                            "Test file(s) changed: {}",
                            file_names.iter().cloned().collect::<Vec<_>>().join(", ")
                        ));
                    }
                }

                // Re-collect
                let _ = client
                    .send(&Request::Collect {
                        context_id: context_id.clone(),
                        force: matches!(reason, RecollectReason::ConftestChanged),
                    })
                    .await;

                // Get tests
                let response = client
                    .send(&Request::List {
                        context_id: context_id.clone(),
                        keyword: cli.keyword.clone(),
                        marker: cli.marker.clone(),
                    })
                    .await;

                let tests_to_run = match response {
                    Ok(Response::TestList { node_ids }) => {
                        if let RecollectReason::TestFilesChanged { file_names } = reason {
                            node_ids
                                .into_iter()
                                .filter(|nid| file_names.iter().any(|f| nid.contains(f)))
                                .collect()
                        } else {
                            node_ids
                        }
                    }
                    _ => vec![],
                };

                if tests_to_run.is_empty() {
                    output.info("No tests to run");
                    state = WatchState::Idle;
                } else {
                    state = WatchState::Running {
                        test_count: tests_to_run.len(),
                        start_time: Instant::now(),
                    };

                    output.info(&format!("Running {} test(s)...", tests_to_run.len()));

                    let response = client
                        .send(&Request::Run {
                            context_id: context_id.clone(),
                            node_ids: tests_to_run,
                            workers: parse_workers(&cli.workers),
                            maxfail: cli.maxfail,
                        })
                        .await;

                    match response {
                        Ok(Response::RunComplete {
                            passed,
                            failed,
                            skipped,
                            errors,
                            duration_ms,
                            ..
                        }) => {
                            output.summary(
                                passed,
                                failed,
                                skipped,
                                errors,
                                duration_ms as f64 / 1000.0,
                            );
                        }
                        Ok(_) => {}
                        Err(e) => {
                            output.error(&format!("Run failed: {}", e));
                        }
                    }

                    state = state
                        .transition(&WatchEvent::RunComplete)
                        .unwrap_or(WatchState::Idle);

                    output.newline();
                    output.info("Waiting for file changes...");
                }
            }
            WatchState::Running { .. } => {
                // Should not reach here - Running state is handled inline
                warn!("Unexpected Running state in watch loop");
                state = WatchState::Idle;
            }
            WatchState::WaitingForTrigger => {
                // Not yet implemented - return to idle
                state = WatchState::Idle;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path_basic() {
        assert_eq!(normalize_path("test.py"), "test.py");
        assert_eq!(normalize_path("./test.py"), "test.py");
        assert_eq!(normalize_path("tests/"), "tests");
        assert_eq!(normalize_path("tests//"), "tests");
        assert_eq!(normalize_path("./tests/"), "tests");
        assert_eq!(normalize_path("/absolute/path"), "/absolute/path");
    }

    #[test]
    fn test_filter_by_paths_empty() {
        let node_ids = vec![
            "test_a.py::test_one".to_string(),
            "test_b.py::test_two".to_string(),
        ];
        let result = filter_by_paths(&node_ids, &[]);
        assert_eq!(result, node_ids);
    }

    #[test]
    fn test_filter_by_paths_exact_node_id() {
        let node_ids = vec![
            "test_a.py::test_one".to_string(),
            "test_a.py::test_two".to_string(),
            "test_b.py::test_three".to_string(),
        ];
        let result = filter_by_paths(&node_ids, &["test_a.py::test_one".to_string()]);
        assert_eq!(result, vec!["test_a.py::test_one"]);
    }

    #[test]
    fn test_filter_by_paths_file_name() {
        let node_ids = vec![
            "example_tests/test_a.py::test_one".to_string(),
            "example_tests/test_a.py::test_two".to_string(),
            "example_tests/test_b.py::test_three".to_string(),
        ];

        // Filter by file name only
        let result = filter_by_paths(&node_ids, &["test_a.py".to_string()]);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"example_tests/test_a.py::test_one".to_string()));
        assert!(result.contains(&"example_tests/test_a.py::test_two".to_string()));
    }

    #[test]
    fn test_filter_by_paths_with_prefix() {
        let node_ids = vec![
            "/full/path/test_a.py::test_one".to_string(),
            "/full/path/test_a.py::test_two".to_string(),
            "/full/path/test_b.py::test_three".to_string(),
        ];

        // Filter by file name only (should match regardless of prefix)
        let result = filter_by_paths(&node_ids, &["test_a.py".to_string()]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_by_paths_directory() {
        let node_ids = vec![
            "tests/unit/test_a.py::test_one".to_string(),
            "tests/unit/test_b.py::test_two".to_string(),
            "tests/integration/test_c.py::test_three".to_string(),
        ];

        // Filter by directory
        let result = filter_by_paths(&node_ids, &["tests/unit/".to_string()]);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"tests/unit/test_a.py::test_one".to_string()));
        assert!(result.contains(&"tests/unit/test_b.py::test_two".to_string()));
    }

    #[test]
    fn test_filter_by_paths_directory_no_trailing_slash() {
        let node_ids = vec![
            "tests/unit/test_a.py::test_one".to_string(),
            "tests/unit/test_b.py::test_two".to_string(),
            "tests/integration/test_c.py::test_three".to_string(),
        ];

        // Filter by directory without trailing slash
        let result = filter_by_paths(&node_ids, &["tests/unit".to_string()]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_by_paths_class() {
        let node_ids = vec![
            "test_example.py::TestClass::test_one".to_string(),
            "test_example.py::TestClass::test_two".to_string(),
            "test_example.py::OtherClass::test_three".to_string(),
        ];

        // Filter by class
        let result = filter_by_paths(&node_ids, &["test_example.py::TestClass".to_string()]);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"test_example.py::TestClass::test_one".to_string()));
        assert!(result.contains(&"test_example.py::TestClass::test_two".to_string()));
    }

    #[test]
    fn test_filter_by_paths_multiple() {
        let node_ids = vec![
            "test_a.py::test_one".to_string(),
            "test_b.py::test_two".to_string(),
            "test_c.py::test_three".to_string(),
        ];

        // Filter by multiple paths
        let result = filter_by_paths(&node_ids, &["test_a.py".to_string(), "test_c.py".to_string()]);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"test_a.py::test_one".to_string()));
        assert!(result.contains(&"test_c.py::test_three".to_string()));
    }

    #[test]
    fn test_filter_by_paths_with_leading_dot_slash() {
        let node_ids = vec![
            "./test_a.py::test_one".to_string(),
            "./test_b.py::test_two".to_string(),
        ];

        // Filter with ./ prefix
        let result = filter_by_paths(&node_ids, &["test_a.py".to_string()]);
        assert_eq!(result.len(), 1);
        assert!(result.contains(&"./test_a.py::test_one".to_string()));
    }
}
