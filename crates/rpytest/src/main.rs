//! rpytest - Rust-powered, drop-in replacement for pytest.

use anyhow::Result;
use clap::Parser;
use rpytest_core::protocol::{Request, Response};
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

mod cli;
mod config;
mod daemon;

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
                .add_directive(log_level.parse().unwrap()),
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
    let root = cli.rootdir.clone().unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    let config = config::load_config(&root)?;

    // Merge CLI args with config
    let effective_cli = cli.merge_with_config(&config);

    // Handle special commands
    if effective_cli.collect_only {
        return handle_collect_only(&effective_cli, &root).await;
    }

    if effective_cli.verify_dropin {
        return handle_verify_dropin(&effective_cli).await;
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
            output.info(&format!("Collected {} tests:", node_ids.len()));
            for node_id in &node_ids {
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

async fn handle_verify_dropin(cli: &Cli) -> Result<()> {
    let output = Output::new(cli.verbose, cli.quiet);
    output.header("Drop-in Compatibility Verification");
    output.warn("Not yet implemented. Will compare rpytest vs pytest outputs.");

    // TODO: Run both pytest and rpytest, diff outputs
    Ok(())
}

async fn handle_run(cli: &Cli, root: &std::path::Path) -> Result<()> {
    let output = Output::new(cli.verbose, cli.quiet);

    if !cli.no_header {
        output.header(&format!("rpytest {}", env!("CARGO_PKG_VERSION")));
    }

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

    let context_id = match response {
        Response::ContextReady { context_id, inventory_hash } => {
            debug!("Context ready: {} (hash: {})", context_id, inventory_hash);
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

    // Get list of tests to run
    let response = client
        .send(&Request::List {
            context_id: context_id.clone(),
            keyword: cli.keyword.clone(),
            marker: cli.marker.clone(),
        })
        .await?;

    let node_ids = match response {
        Response::TestList { node_ids } => node_ids,
        Response::Error { code, message } => {
            output.error(&format!("Failed to list tests: {:?} - {}", code, message));
            anyhow::bail!("List failed: {}", message);
        }
        _ => {
            output.error("Unexpected response from daemon");
            anyhow::bail!("Unexpected response");
        }
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
            total,
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
