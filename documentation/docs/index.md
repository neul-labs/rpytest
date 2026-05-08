# rpytest

**Rust-powered, drop-in replacement for pytest**

rpytest is a high-performance test runner that provides full pytest compatibility while delivering significant speed improvements through its hybrid Rust/Python architecture.

## Why rpytest?

| Feature | pytest | rpytest |
|---------|--------|---------|
| Execution Speed | Baseline | **1.2x faster** |
| Memory Usage | 39 MB | **6 MB** (6.7x less) |
| Startup Time | ~600ms | **~300ms** (2x faster) |
| Parallel Mode | pytest-xdist required | **Built-in** |

## Key Features

- **Drop-in Replacement**: Use the same CLI flags, configuration files, and plugins
- **Blazing Fast**: Rust CLI with warm Python daemon eliminates startup overhead
- **Built-in Parallelism**: Native `-n` flag support without pytest-xdist
- **Smart Sharding**: Duration-balanced test distribution for CI/CD
- **Flakiness Detection**: Automatic tracking and rerun of flaky tests
- **Watch Mode**: Re-run affected tests on file changes
- **Low Memory**: Lightweight Rust binary, shared daemon for multiple runs

## Quick Example

```bash
# Install rpytest (both CLI and daemon are required)
cargo install rpytest rpytest-daemon

# Run tests (same as pytest!)
rpytest tests/

# Run with 4 parallel workers
rpytest tests/ -n 4

# Watch mode
rpytest tests/ --watch

# Collect only
rpytest tests/ --collect-only
```

## Architecture

rpytest uses a hybrid architecture:

1. **Rust CLI** (6 MB): Fast startup, argument parsing, output formatting
2. **Python Daemon**: Warm pytest workers, test collection, execution
3. **IPC Layer**: MessagePack over NNG sockets for efficient communication

```
┌─────────────┐     IPC      ┌─────────────────┐
│  Rust CLI   │◄────────────►│  Python Daemon  │
│  (rpytest)  │  MessagePack │  (warm workers) │
└─────────────┘              └─────────────────┘
```

The daemon stays running between invocations, eliminating Python startup overhead for subsequent runs.

## Getting Started

<div class="grid cards" markdown>

- :material-download: **[Installation](getting-started/installation.md)**

  Install rpytest on your system

- :material-rocket-launch: **[Quick Start](getting-started/quickstart.md)**

  Run your first tests with rpytest

- :material-swap-horizontal: **[Migration Guide](getting-started/migration.md)**

  Switch from pytest to rpytest

</div>

## Compatibility

rpytest is designed to be a drop-in replacement for pytest:

- ✅ Same CLI flags (`-k`, `-m`, `-x`, `--maxfail`, etc.)
- ✅ Same configuration files (`pytest.ini`, `pyproject.toml`, `setup.cfg`)
- ✅ Same fixtures and markers
- ✅ Plugin passthrough support
- ✅ JUnit XML output
- ✅ Coverage integration

## License

rpytest is dual-licensed under MIT and Apache 2.0.
