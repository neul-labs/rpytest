# Daemon Capabilities

The long-lived Python daemon is central to rpytest’s performance and ergonomics. This document details every capability that persistence unlocks and how each one improves developer workflows.

## Near-zero startup

- The heavy import cost (Python interpreter, pytest, plugins, Django/FastAPI apps, etc.) happens once when the host-level daemon boots.
- Subsequent commands from any repository reuse the hot interpreter, so single-test reruns in a TDD loop begin in tens of milliseconds instead of hundreds.
- async-nng keeps the control channel hot so repeated invocations reuse the same sockets without reconnect penalties.
- Multiple CLI processes (e.g., a watch task plus an ad-hoc targeted run from another repo) connect concurrently to the shared daemon; the scheduler isolates them via repository context tokens.

## Persistent, queryable inventory

- The daemon keeps the full suite inventory in memory: node IDs, files, markers, keywords, line numbers, last status, and timing data.
- CLI filters (`-k`, `-m`, marker expressions, explicit `::test_name`) run purely against that cache, eliminating repeated collection.
- Editors can query the inventory to implement “list tests in file” or “run nearest test” without spawning pytest.

## Incremental collection

- File changes invalidate only the relevant modules, so the daemon re-collects a minimal slice of the tree.
- Large suites avoid paying the full-collection tax on every command; only the first run needs a full pass.
- The `notify` crate detects file changes while a dependency graph (serialized efficiently via `rkyv`) links edits to concrete pytest nodes, ensuring incremental updates stay accurate even in complex packages.

## Watch mode

- `rpytest --watch` wires the file watcher to the incremental collector and scheduler.
- On change, the CLI asks the daemon to re-collect affected modules and run just those tests (or related tests if dependency mapping is configured).
- Output streams continuously, mirroring modern JavaScript/Vitest watch experiences but with pytest semantics.

## Warm worker pool

- Workers stay alive between runs, so no process spawn or module import penalty for each invocation.
- Better scheduling becomes possible because duration history stays in RAM; rpytest can hand longer tests to idle workers proactively.
- The same mechanism underpins cheaper parallelism locally and in CI compared to `pytest-xdist`.

## Built-in parallel execution (`-n` flag)

- rpytest provides pytest-xdist compatible `-n` support without requiring the plugin.
- `-n auto` auto-detects CPU cores; `-n 4` uses exactly 4 workers.
- Unlike xdist, workers are pre-warmed, eliminating cold start overhead.
- Duration-aware load balancing (LPT algorithm) distributes tests optimally.
- For small-to-medium test suites, the default hybrid execution often outperforms explicit `-n` modes because the warm daemon eliminates the overhead that parallel execution is meant to amortize.

## Cross-run history and heuristics

- Durations, pass/fail streaks, and failure messages accumulate in memory and are optionally flushed to disk.
- Features such as `--failed-first`, `--last-failed`, auto-reruns for flaky tests, or balanced sharding exploit this fresh history instead of relying on pytest’s slower cache.
- sled backs this history so the daemon can restart without losing context, and CI jobs can pre-load metadata for smarter sharding.

## Optional fixture reuse (“turbo mode”)

- Session-scoped fixtures can be flagged as reusable so that costly resources (databases, service mocks, datasets) remain initialized across runs.
- Disabled by default to preserve vanilla semantics; when enabled, dramatically cuts the cost of repeated local runs.

## IDE and editor integration

- Because the daemon exposes RPC endpoints, editors can maintain a persistent connection for commands like “list tests”, “run nearest”, or “show last result inline”.
- The latency matches other language servers, giving Python developers a first-class testing experience without bespoke per-editor plugins.

## Multi-command CI workflows

- CI jobs often run multiple pytest invocations (full suite, failed-first rerun, targeted smoke tests). Holding a daemon for the entire job removes duplicate interpreter startups.
- Results, coverage artifacts, and failure metadata can be streamed and archived incrementally, enabling richer reporting dashboards.

These capabilities collectively narrow the gap between pytest’s flexibility and the snappy feedback loops developers expect from modern tooling.
