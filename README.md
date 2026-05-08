# rpytest

[![Crates.io](https://img.shields.io/crates/v/rpytest.svg)](https://crates.io/crates/rpytest)
[![License](https://img.shields.io/crates/l/rpytest.svg)](https://github.com/neul-labs/rpytest#license)
[![Documentation](https://img.shields.io/badge/docs-neullabs.com-blue)](https://docs.neullabs.com/rpytest)

**Run your pytest suite faster. Change nothing.**

rpytest is a Rust-powered, drop-in replacement for `pytest` that eliminates startup and collection overhead while keeping your existing tests, fixtures, and plugins completely untouched.

```bash
# Install and run - that's it
pip install rpytest
rpytest
```

## Why rpytest?

| Metric | pytest | rpytest | Improvement |
|--------|--------|---------|-------------|
| Wall clock (480 tests) | 2.91s | 1.55s | **1.9x faster** |
| CLI memory usage | 35.8 MB | 6.2 MB | **5.8x less** |

The speed comes from a persistent daemon that keeps Python warm between runs. No more paying interpreter startup costs on every invocation.

## Installation

```bash
# Via pip (recommended)
pip install rpytest

# Via Homebrew (macOS / Linux)
brew tap neul-labs/tap
brew install rpytest

# Via npm
npm install -g rpytest

# Via cargo
cargo install rpytest

# From source
git clone https://github.com/neul-labs/rpytest.git
cd rpytest && cargo install --path crates/rpytest
```

## Usage

rpytest mirrors pytest's CLI exactly. If you know pytest, you know rpytest.

```bash
# Run all tests
rpytest

# Run specific tests
rpytest tests/test_api.py::test_login

# Filter by keyword or marker
rpytest -k "auth" -m "not slow"

# Parallel execution (pytest-xdist compatible, no plugin needed)
rpytest -n auto

# Watch mode for TDD
rpytest --watch

# Verify identical behavior to pytest
rpytest --verify-dropin
```

## Key Features

### Instant Startup
The first run spawns a background daemon. Every subsequent run is a fast RPC call—no interpreter startup, no re-importing your test modules.

### Full pytest Compatibility
- All pytest CLI flags work identically
- Your plugins, fixtures, and conftest.py files run unchanged
- `pytest.ini`, `pyproject.toml`, and `tox.ini` configs are respected
- Exit codes and JUnit XML match pytest exactly

### Built-in Parallelism
```bash
rpytest -n 4  # Run on 4 workers
rpytest -n auto  # Auto-detect based on CPU cores
```

No pytest-xdist required. Duration-aware load balancing included.

### Watch Mode
```bash
rpytest --watch
```

File changes trigger automatic re-runs of affected tests.

### Flakiness Detection
```bash
rpytest --reruns 3  # Auto-retry failed tests
rpytest --flaky-report  # See which tests are flaky
```

### Sharding for CI
```bash
rpytest --shard 0 --total-shards 4  # Perfect for parallel CI jobs
```

## How It Works

1. **First run**: Spawns a Python daemon that collects your test suite once
2. **Subsequent runs**: Rust CLI filters tests and dispatches to warm Python workers
3. **Results stream back** in real-time for instant feedback

The daemon persists between runs, so repeated invocations (TDD loops, CI retries, `--last-failed`) skip all the startup work.

## Daemon Management

```bash
rpytest --daemon-status  # Check health
rpytest --daemon-stop    # Stop the daemon
rpytest --cleanup        # Clean stale contexts
```

## Drop-in Guarantee

Run the verification harness to confirm identical behavior:

```bash
rpytest --verify-dropin
```

This runs both pytest and rpytest on your suite and compares collection counts, pass/fail results, and exit codes.

## Configuration

rpytest reads your existing pytest configuration—no new config files needed:
- `pytest.ini`
- `pyproject.toml` (`[tool.pytest.ini_options]`)
- `tox.ini`
- `setup.cfg`

## Documentation

Full documentation at [docs.neullabs.com/rpytest](https://docs.neullabs.com/rpytest)

## Contributing

Contributions welcome!

```bash
git clone https://github.com/neul-labs/rpytest.git
cd rpytest
cargo build
cargo test
```

## License

Licensed under the [MIT License](LICENSE).
