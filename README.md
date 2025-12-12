# rpytest

Rust-powered, drop-in replacement for `python -m pytest` that slashes collection and orchestration overhead while keeping your existing tests, fixtures, and plugins untouched.

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

rpytest’s core bet is that by shrinking the non-test portion close to zero, overall wall-clock time improves without touching user code. Expected end-to-end improvements:

| Suite profile                               | Expected speedup vs pytest |
| ------------------------------------------- | -------------------------- |
| Tiny / overhead-dominated unit tests        | 3–5×                       |
| Mixed unit + integration tests              | 1.3–2×                     |
| IO-heavy integration suites (DB/HTTP bound) | 1.1–1.5×                   |
| Re-running single tests during TDD          | Mostly startup wins (≈1.5×) |

## How it works

1. **One-time warm-up** – the first invocation spawns a long-lived Python daemon that imports pytest, plugins, and your application.
2. **Inventory creation** – the daemon collects the suite once, storing identifiers, markers, file locations, and recent timings.
3. **Rust-side filtering** – the CLI consumes the inventory to process flags like `-k`, `-m`, `--maxfail`, or explicit node IDs without waking Python.
4. **Warm worker pool** – the daemon maintains worker interpreters ready to run tests, drastically cutting process spawn and import time.
5. **Result streaming & caching** – results stream back to Rust immediately for reporting, aggregation, XML/JUnit generation, and durability.

### Core technologies

- **async-nng** powers the duplex, low-latency transport between the Rust CLI and the Python daemon, enabling high-volume event streaming without blocking test execution.
- **sled** stores the persisted inventory, duration history, and daemon metadata under `.rpytest/`, so restarts and CI jobs can resume instantly with warm caches.
- **ryv** tracks filesystem events and dependency relationships, feeding incremental collection, watch mode, and “run affected tests” workflows.

Because the daemon persists between runs, repeated commands (local TDD loops, CI retries, `--last-failed`, etc.) become simple RPC calls instead of full interpreter startups.

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

## Roadmap at a glance

- [ ] Implement the Rust CLI skeleton with argparse/flag parity.
- [ ] Build the Python daemon process with a stable IPC protocol.
- [ ] Persist test inventory and history to disk for CI reuse.
- [ ] Add optional session-fixture reuse for “turbo” iterative runs.
- [ ] Expose an editor/IDE protocol for listing and running nearest tests instantly.

See `docs/` for deeper dives into the daemon architecture and the capabilities it unlocks.
