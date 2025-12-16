# Architecture Overview

rpytest is split into a Rust control plane and a long-lived Python execution plane. The Rust half owns the CLI, scheduling, and reporting responsibilities; the Python half runs tests through the existing pytest engine. The following sections describe the flow in more detail.

## Components

- **Rust CLI / Daemon Manager** – Parses pytest-compatible flags, manages configuration, and ensures the daemon is running. Responsible for user interaction, watch mode, and persistence of cached data on disk.
- **Python Daemon** – A single host-level daemon hosts pytest, plugins, and project imports. It maintains separate execution contexts per repository, allowing multiple projects to share one process without leaking state.
- **Messaging Layer (async-nng)** – A duplex socket mesh built with async-nng moves commands and event streams between the CLI and daemon with minimal latency.
- **Test Inventory Store** – An in-memory table (backed by a sled cache) that records every node’s ID, path, markers, keywords, line numbers, and recent durations or statuses.
- **Worker Pool** – One or more warm Python worker interpreters controlled by the daemon. Workers receive batches of node IDs, execute them, and stream results back. The pool can expand or shrink depending on `--workers` flags and machine topology.
- **Result Aggregator** – Lives in Rust, collects execution events, enforces `--maxfail`, produces textual output, and can emit JUnit XML or other reports incrementally.
- **File Watcher / Dependency Tracker** – Uses the `notify` crate to detect filesystem events and maps them to affected test nodes via a dependency graph. Inventory and dependency data serialize efficiently with `rkyv` for fast cache loads. Powers incremental collection and "run affected tests" features.

## Control Flow

1. **Startup** – `rpytest` CLI checks if the host-level daemon is running. If not, it spawns the process and initializes shared infrastructure. Each repository registers its own execution context the first time it connects, so subsequent commands (even from other repos) attach with near-zero latency.
2. **Collection & Inventory** – Within each repository context, the daemon performs a full collection once, fills the inventory with node metadata, and returns a content-hash so the CLI can detect drift. When file changes are detected (via watch mode or manual invalidations) the daemon re-collects only the affected modules for that context and increments the inventory generation.
3. **Selection** – Flags such as `-k`, `-m`, markers, `-q`, or explicit node IDs are processed entirely in Rust by querying the cached inventory. This avoids invoking pytest’s Python-side selection logic until a concrete list of node IDs is known.
4. **Scheduling** – The CLI hands the filtered node list back to the daemon along with scheduling hints (max workers, sharding strategy, failed-first, etc.). The daemon assigns work to warm workers using duration history for load balancing. Because workers remain hot, there is no repeated import or fixture setup for each run.
5. **Execution** – Workers execute tests using pytest’s internals. Session-scoped fixtures can optionally persist between runs, while function-scoped fixtures continue to behave exactly as they do in vanilla pytest.
6. **Reporting** – As soon as a worker emits a result event, the daemon forwards it to the Rust process, which prints live progress, enforces stop conditions, and writes report files. History (durations, pass/fail counts, flakiness) is updated both in-memory and in an on-disk cache so subsequent invocations and CI jobs can benefit immediately.

## Data & IPC

- **Protocol** – async-nng sockets carry framed MessagePack-encoded commands (`Collect`, `Run`, `List`, `Shutdown`) and streaming events (`TestEvent`, `LogEvent`). MessagePack was chosen over JSON for compact binary encoding and faster serialization in hot paths. A single daemon listens on a well-known Unix socket (or TCP on Windows); every CLI request includes a repository context token so work queues, inventories, and worker pools stay isolated even though the process is shared.
- **Inventory persistence** – Metadata lives in sled under `.rpytest/`, namespaced by repository context. The shared daemon consults this store to hydrate per-repo inventories, duration maps, and connection metadata even after restarts or when multiple repos are active simultaneously.
- **File watching** – The `notify` crate consumes filesystem notifications; the daemon maps them to dependency graphs and emits `Invalidate` commands so only impacted modules are re-collected. Outside of watch mode, explicit `--invalidate` flags or detection of import errors trigger a resync.

## Error handling & failure modes

- **Daemon crash recovery** – If the daemon terminates unexpectedly, the CLI detects the closed socket and spawns a fresh instance. Cached inventory in sled allows rapid rehydration without full re-collection.
- **Worker failure isolation** – A worker crash (e.g., segfault in native extension) terminates only that worker. The scheduler reassigns pending tests to other workers and reports partial results. The test that caused the crash is marked as errored.
- **IPC timeouts** – Commands carry deadlines. If a response is not received within the timeout (configurable via `--timeout`), the CLI reports the operation as failed and optionally kills the unresponsive worker.
- **Collection errors** – Syntax errors or import failures during collection are reported per-module. Valid modules continue to populate the inventory; invalid ones are flagged so the user can fix them without losing the rest of the suite.
- **Stale cache detection** – Inventory entries include file content hashes. On startup, mismatches trigger targeted re-collection rather than serving stale data.
- **Graceful degradation** – If the daemon cannot start (port conflict, permission issues), the CLI falls back to spawning vanilla pytest with `RPYTEST_FALLBACK=1` semantics, ensuring tests still run.

## Compatibility strategy

- Pytest plugins continue to run inside the daemon's interpreter, so plugin hooks behave as usual.
- Node IDs, collection semantics, and reporting stay faithful to upstream pytest, making rpytest safe to drop into existing tooling.
- Advanced behaviors (session fixture reuse, aggressive caching) remain opt-in toggles so default runs match pytest’s behavior byte-for-byte.
