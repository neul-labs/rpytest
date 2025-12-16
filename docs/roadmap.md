# Roadmap to Full Functionality

This roadmap tracks the milestones required to turn rpytest into a production-grade, drop-in replacement for `python -m pytest`. Each phase builds on the last, with explicit deliverables and success criteria.

## Phase 0 – Foundations

- **Rust CLI skeleton** with argparse parity for core pytest flags (`-k`, `-m`, `-q`, `--maxfail`, etc.).
- **Command routing layer** that can forward unknown/experimental flags transparently to future subsystems.
- **Async-nng IPC harness** stub that can send/receive framed messages, plus a noop Python daemon placeholder.
- **Unit-test harness** for CLI parsing and IPC framing (Rust) plus contract tests for any Python stubs so future changes remain stable.

## Phase 1 – Shared daemon + IPC

- **Python daemon process** using async-nng to accept RPCs.
- **Repository context registry** so the single daemon can serve multiple worktrees concurrently without state bleed.
- **Bootstrap protocol** covering `InitContext`, `ShutdownContext`, health checks, and logging streams.
- **Basic execution RPCs**: run a provided list of node IDs by shelling out to vanilla pytest, return results.

## Phase 2 – Inventory & sled persistence

- **Collection RPC** that populates the in-memory inventory for a repo context (node IDs, file/line, markers, keywords).
- **sled-backed cache** under `.rpytest/` for inventory snapshots, duration history, and daemon metadata.
- **Cache versioning** – embed a schema version in all persisted data; on version mismatch, invalidate and rebuild rather than corrupt. This ensures upgrades are seamless and avoids "upgrade path" complexity in later phases.
- **Rust-side selectors** using cached inventory for `-k`, `-m`, node paths, and `--maxfail` pre-filtering.
- **CLI status commands** (`rpytest --list`, `rpytest --inventory-status`) powered entirely by the cache.
- **Unit tests** covering inventory serialization/deserialization, sled persistence boundaries, and selector correctness.

## Phase 3 – Worker pool & scheduling

- **Warm worker interpreters** managed by the daemon, each with attach/detach lifecycle APIs.
- **Scheduler** that dispatches tests to workers using duration history for balancing.
- **Concurrent CLI support** so multiple commands enqueue work per repo context without starvation.
- **Result streaming** with incremental reporting, stop conditions, and live progress bars.
- **Load-test harness** that simulates concurrent CLI clients and asserts fairness, along with unit tests for scheduler heuristics.

## Phase 4 – Watch mode & ryv integration

- **ryv-powered file watcher** hooked into the CLI with repository context awareness.
- **Dependency graph mapping** so file changes invalidate only affected nodes.
- **`rpytest --watch`** that performs incremental collection + targeted execution on change.
- **Editor protocol** (simple JSON-RPC over async-nng) for “run nearest test” and “list tests in file”.
- **Integration tests** validating that file edits trigger the correct incremental collections and that editor commands return deterministic results.

## Phase 5 – Turbo / advanced features

- **Opt-in session fixture reuse** toggles per repo context with safety guards.
- **Flakiness heuristics**: auto-rerun failed tests N times, annotate outputs, track streaks in sled.
- **Sharding & remote execution hooks** for future worker types (containers, remote hosts).

## Phase 6 – Verification & benchmarks

- **Compatibility harness** that runs pytest’s upstream test suite plus the plugin canary matrix under rpytest nightly.
- **`rpytest --verify-dropin`** command to run pytest vs rpytest locally and diff outputs.
- **Benchmark suite** spanning tiny unit-test repos, medium mixed suites, and IO-heavy benchmarks, with public dashboards.
- **Success criteria**: 1.3–2× gains on medium suites, ≥3× on overhead-bound suites, zero regressions in compatibility harness.
- **Documentation checkpoint** – update README, architecture, compatibility guide, and user docs to reflect feature status and benchmark methodology.

## Phase 7 – Release readiness

- **Installer/packaging** (prebuilt binaries, pip wrapper).
- **Crash recovery & daemon lifecycle management** (auto restart, stale context cleanup).
- **Documentation freeze** covering architecture, daemon capabilities, drop-in guide, benchmarking methodology, and troubleshooting.
- **GA checklist**: signed artifacts, upgrade path for cached data, support SLAs.
- **Test coverage gates** ensuring unit/integration suites stay green before tagging releases.

Tracking progress across these phases ensures rpytest reaches “full functionality” while maintaining the drop-in guarantee and measurable performance wins.
