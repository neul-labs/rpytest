# rpytest

Rust-powered, drop-in replacement for `python -m pytest` that slashes collection and orchestration overhead while keeping your existing tests, fixtures, and plugins untouched.

## Installation

```bash
# From crates.io (once published)
cargo install rpytest

# From source
git clone https://github.com/user/rpytest.git
cd rpytest
cargo build --release
cp target/release/rpytest ~/.local/bin/
```

Ensure the binary is on your `PATH` and that your Python environment has pytest installed.

## Quick start

```bash
# Run all tests (just like pytest)
rpytest

# Run specific tests
rpytest tests/test_api.py::test_login

# Filter by keyword or marker
rpytest -k "auth" -m "not slow"

# Parallel execution (pytest-xdist compatible)
rpytest -n auto

# Watch mode for TDD
rpytest --watch

# Verify drop-in compatibility
rpytest --verify-dropin
```

The first invocation spawns a background daemon and collects your test suite. Subsequent runs reuse the warm daemon for near-instant startup.

## What rpytest delivers

- **Drop-in CLI compatibility** – mirrors the pytest command surface so existing workflows, scripts, and CI jobs continue to run unchanged.
- **Rust control plane** – a fast binary handles collection, selection, scheduling, and reporting so Python only runs the actual tests.
- **Pytest daemon** – keeps Python, plugins, and fixtures resident in memory to drive repeated runs with near-zero startup latency.
- **Cached test inventory** – stores node IDs, markers, and metadata so `-k`, `-m`, and `::test_name` filtering happens instantly without re-collection.
- **Parallel-friendly scheduler** – coordinates warm Python workers (local processes today, extensible to remote) with minimal coordination overhead.
- **Rich telemetry** – retains durations, statuses, and flakiness data between runs for better `--failed-first`, balancing, and rerun logic.

## Why it matters

Traditional pytest invocations spend a surprising amount of time on framework work rather than executing your tests. A rough breakdown for real suites:

| Runtime component                | Typical share |
| -------------------------------- | ------------- |
| Interpreter + environment setup  | 5–15%         |
| Collection, fixtures, plugins    | 10–40%        |
| Reporting / coordination         | 5–10%         |
| Actual test bodies               | 40–80%        |

rpytest's core bet is that by shrinking the non-test portion close to zero, overall wall-clock time improves without touching user code. Measured improvements:

| Metric | pytest | rpytest | Improvement |
|--------|--------|---------|-------------|
| Execution Time (480 tests) | 0.51s | 0.48s | ~1.1x faster |
| Wall Clock (with startup) | 2.91s | 1.55s | **1.9x faster** |
| CLI Memory | 35.8 MB | 6.2 MB | **5.8x less** |

The largest wins come from eliminating repeated interpreter startup and collection overhead.

## How it works

1. **One-time warm-up** – the first `rpytest` invocation on a host spawns a long-lived Python daemon. New repositories register their own execution context inside that process the first time they connect.
2. **Inventory creation** – within each context, the daemon collects the suite once, storing identifiers, markers, file locations, and recent timings.
3. **Rust-side filtering** – the CLI consumes the inventory to process flags like `-k`, `-m`, `--maxfail`, or explicit node IDs without waking Python.
4. **Warm worker pool** – the daemon maintains worker interpreters ready to run tests, drastically cutting process spawn and import time.
5. **Result streaming & caching** – results stream back to Rust immediately for reporting, aggregation, XML/JUnit generation, and durability.

### Core technologies

- **async-nng** powers the duplex, low-latency transport between the Rust CLI and the Python daemon, enabling high-volume event streaming without blocking test execution.
- **sled** stores the persisted inventory, duration history, and daemon metadata under `.rpytest/`, so restarts and CI jobs can resume instantly with warm caches.
- **notify** watches filesystem events while **rkyv** enables zero-copy serialization of inventory and dependency data, together feeding incremental collection, watch mode, and "run affected tests" workflows.

Because the shared daemon (and each repo’s context) persist between runs, repeated commands (local TDD loops, CI retries, `--last-failed`, etc.) become simple RPC calls instead of full interpreter startups.

## Ergonomic workflows

- `rpytest path/to/test_file.py::TestSuite::test_case` – instant targeted runs after the first invocation thanks to cached inventory lookups.
- `rpytest --watch` – file watcher triggers incremental re-collection and runs only affected tests for tight TDD feedback loops.
- `rpytest --failed-first --maxfail=1` – leverages cached results to prioritize the most recent failures without re-discovering the suite.
- `rpytest --workers auto` – scales out across warm workers with smarter scheduling based on historical durations.

## Drop-in compatibility commitments

- **Flag parity** – every stable pytest CLI flag maps 1:1 to an rpytest option. Unknown flags are forwarded verbatim so existing shell scripts keep working.
- **Plugin ecosystem** – plugins execute inside the Python daemon, so hook ordering and side effects match upstream pytest. Any incompatibility is treated as a blocker before releases.
- **Config files** – `pytest.ini`, `pyproject.toml`, and `tox.ini` settings are read exactly as pytest would. No new config files are required to adopt rpytest.
- **Exit codes & reporting** – exit statuses, JUnit XML layouts, and log formatting follow pytest conventions to keep CI pipelines, coverage tools, and IDEs stable.
- **Opt-in enhancements** – advanced behaviors (fixture reuse, selective collection) stay behind explicit flags so default invocation mirrors `python -m pytest` byte-for-byte.
- **Single shared daemon, per-repo contexts** – one host-level daemon multiplexes every repository by assigning each a distinct context inside the process. Multiple CLI invocations (even from different repos or virtualenvs) share the same daemon without interfering because commands are namespaced by working directory.

See `docs/drop-in-compatibility.md` for the full compatibility plan and safeguards.

## Current Status

rpytest has completed implementation of Phases 0-7 of the roadmap:

- [x] **Phase 0**: Rust CLI skeleton with pytest flag parity
- [x] **Phase 1**: Python daemon process with async-nng IPC
- [x] **Phase 2**: Inventory & sled persistence with caching
- [x] **Phase 3**: Worker pool & parallel scheduling
- [x] **Phase 4**: Watch mode & editor protocol integration
- [x] **Phase 5**: Turbo features (flakiness detection, fixture reuse, sharding)
- [x] **Phase 6**: Verification harness & benchmark suite
- [x] **Phase 7**: Release readiness (packaging, crash recovery, documentation)

See `docs/roadmap.md` for detailed milestone descriptions.

## New in Latest Release

### Daemon Management
```bash
# Check daemon status and health
rpytest --daemon-status

# Stop the running daemon
rpytest --daemon-stop

# Clean up stale contexts
rpytest --cleanup
rpytest --cleanup --cleanup-max-age 7200
```

### Flakiness Detection & Auto-Rerun
```bash
# Rerun failed tests up to 3 times
rpytest --reruns 3 --reruns-delay 1000

# Only rerun known flaky tests
rpytest --only-rerun-flaky

# Show flakiness report
rpytest --flaky-report
```

### Sharding for Distributed Testing
```bash
# Run shard 0 of 4 total shards
rpytest --shard 0 --total-shards 4

# Use duration-balanced sharding strategy
rpytest --shard 1 --total-shards 4 --shard-strategy duration_balanced
```

### Session Fixture Reuse
```bash
# Enable fixture reuse between runs
rpytest --reuse-fixtures --fixture-max-age 600
```

### Parallel Test Execution (`-n` / pytest-xdist compatible)
```bash
# Run with automatic worker count (based on CPU cores)
rpytest -n auto

# Run with specific number of workers
rpytest -n 4

# Run sequentially (single worker)
rpytest -n 1
```

rpytest's `-n` flag provides pytest-xdist compatible parallel execution without requiring the xdist plugin. Tests are distributed across warm worker processes using duration-aware load balancing (LPT algorithm) for optimal scheduling.

**Parallel execution comparison (480 tests):**
| Runner | Time | Notes |
|--------|------|-------|
| pytest (sequential) | 0.51s | Baseline |
| pytest -n 4 (xdist) | 1.23s | Worker startup overhead |
| rpytest (default) | 0.48s | Hybrid execution |
| rpytest -n 4 | 0.99s | 20% faster than xdist |

For small-to-medium test suites, rpytest's default mode often outperforms explicit parallel modes because warm workers eliminate the overhead that parallelism is meant to amortize.

### Drop-in Compatibility Verification
```bash
# Verify rpytest produces identical results to pytest
rpytest --verify-dropin

# Verify specific test directory
rpytest --verify-dropin tests/

# Verbose comparison with diff details
rpytest --verify-dropin -vv
```

The verification harness runs both pytest and rpytest on your test suite and compares:
- Test collection counts
- Pass/fail/skip/error counts
- Exit codes

This ensures rpytest behaves identically to pytest before you switch your CI pipeline.
