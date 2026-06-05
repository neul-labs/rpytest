# rpytest

**Run your pytest suite faster. Change nothing.**

rpytest is a Rust-powered, drop-in replacement for `pytest` that eliminates
startup and collection overhead while keeping your existing tests, fixtures,
and plugins completely untouched.

```bash
pip install rpytest
rpytest
```

## Why rpytest?

| Metric | pytest | rpytest | Improvement |
|--------|--------|---------|-------------|
| Wall clock (500 tests, sequential) | 0.63s | 0.32s | **2.0x faster** |
| CLI memory usage | 39.4 MB | 5.9 MB | **6.7x less** |
| Collection (1000 files) | 8.5s | 0.3s | **28x faster** |
| Parallel mode | pytest-xdist required | **Built-in `-n`** | no plugin |

Benchmarks come from `BENCHMARK.md` in the repo and are reproducible with
`benchmark_suite/` and `run_benchmark.py`.

The speed comes from a persistent daemon that keeps Python warm between
runs. No more paying interpreter startup costs on every invocation.

## Key Features

- **Drop-in replacement**: same CLI flags (`-k`, `-m`, `-x`, `--maxfail`,
  `--lf`, `--ff`, `-n`, ...), same config files (`pytest.ini`,
  `pyproject.toml`, `tox.ini`, `setup.cfg`), same fixtures and markers.
- **Persistent daemon**: keeps the Python interpreter warm; subsequent runs
  are RPC calls, not cold starts.
- **Built-in parallelism**: `-n auto` with duration-aware LPT scheduling.
  No `pytest-xdist` required.
- **Sharding for CI**: `--shard`/`--total-shards` with `hash`,
  `round_robin`, or `duration_balanced` strategies.
- **Watch mode**: `--watch` re-runs affected tests on file changes.
- **Flakiness detection**: `--reruns N`, `--reruns-delay`,
  `--only-rerun-flaky`, `--flaky-report`.
- **Fixture reuse**: `--reuse-fixtures` persists session fixtures between
  runs with mtime-based invalidation.
- **Editor integration**: JSON-RPC server for IDE plugins
  (`--editor-server`).
- **Drop-in verification**: `--verify-dropin` runs both pytest and rpytest
  on your suite and compares collection counts, results, and exit codes.

## Install

```bash
# pip (recommended) - bundles platform binary
pip install rpytest

# Homebrew (macOS / Linux)
brew tap neul-labs/tap
brew install rpytest

# npm
npm install -g rpytest

# cargo (installs CLI + daemon)
cargo install rpytest rpytest-daemon
```

See [Installation](getting-started/installation.md) for build-from-source
and platform notes.

## Quick example

```bash
# Run all tests (same UX as pytest)
rpytest

# Filter by keyword and marker
rpytest -k "auth" -m "not slow"

# Parallel execution (no plugin needed)
rpytest -n auto

# Watch mode for TDD
rpytest --watch

# Confirm identical behaviour vs pytest
rpytest --verify-dropin
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

rpytest is licensed under the [MIT License](https://github.com/neul-labs/rpytest/blob/main/LICENSE).
