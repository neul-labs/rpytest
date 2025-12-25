# Drop-in Compatibility Guide

This document describes how rpytest maintains behavioral parity with upstream pytest so teams can adopt the faster runner without rewriting tests, fixtures, or automation scripts.

## Compatibility pillars

1. **CLI flag parity** – The Rust CLI intentionally mirrors pytest’s argparse surface. Unsupported experimental flags are passed through verbatim to the daemon so plugins can consume them. Each release runs pytest’s own CLI compatibility suite to detect regressions.
2. **Config fidelity** – All standard configuration sources (`pytest.ini`, `pyproject.toml`, `tox.ini`, `setup.cfg`) are parsed using the same precedence rules as pytest. rpytest never invents new defaults that would alter selection, markers, or reporting.
3. **Plugin execution environment** – Plugins import and run inside the Python daemon. Hook ordering, `pytest_configure`, fixture factories, and monkeypatching semantics therefore match upstream behavior. Any plugin that works under pytest must work under rpytest unless it depends on undocumented internals; such cases are tracked and patched quickly.
4. **Node identity** – Test node IDs, markers, parametrization IDs, and collection ordering remain unchanged so tooling anchored on node paths continues to function (e.g., smoke-test filters, flaky-test allowlists, and CI dashboards).
5. **Exit codes & outputs** – Status codes, stdout/stderr formatting, logging levels, JUnit XML, coverage integration, and warnings follow pytest conventions. Deviations require both documentation and explicit opt-in flags.

## Safeguards & tests

- **Pytest upstream test suite** – rpytest executes pytest’s own test suite (focused on collection and reporting behavior) inside CI to catch drift immediately.
- **Plugin canary suite** – A curated list of popular plugins (xdist, asyncio, Django, hypothesis, etc.) is exercised nightly to ensure their hooks work end-to-end within the daemon.
- **Golden CLI snapshots** – Representative real-world projects run under both pytest and rpytest with outputs diffed automatically. Any textual difference is triaged before release.
- **Fallback path** – `RPYTEST_FALLBACK=1` environment variable forces the CLI to spawn vanilla pytest, allowing users to bisect issues without uninstalling rpytest.

## Adoption checklist

1. Install the `rpytest` binary alongside your existing Python environment.
2. Run `rpytest --verify-dropin` to execute a comparison harness that runs both pytest and rpytest on a subset of tests and surfaces any behavioral differences.
3. Update CI jobs to call `rpytest` instead of `python -m pytest`. Keep the fallback flag available (e.g., `RPYTEST_FALLBACK=1`) so you can revert quickly if necessary.
4. Monitor cache directories (default `.rpytest/`) into your CI artifacts if you want to benefit from sled-backed inventories between stages; otherwise they can be safely discarded.

By enforcing the safeguards above and keeping enhancements opt-in, rpytest stays a true drop-in replacement while delivering the performance advantages described elsewhere in the docs.

## pytest-xdist compatibility

rpytest provides built-in parallel execution that is compatible with pytest-xdist's `-n` flag:

```bash
# These work identically in both pytest-xdist and rpytest
rpytest -n auto          # Auto-detect CPU count
rpytest -n 4             # Use 4 workers
rpytest -n 1             # Sequential execution
```

**Key differences from pytest-xdist:**

| Feature | pytest-xdist | rpytest |
|---------|--------------|---------|
| Installation | Requires `pip install pytest-xdist` | Built-in, no plugin needed |
| Worker startup | Cold start per run | Warm workers (pre-loaded pytest) |
| Load balancing | Various strategies (`load`, `loadscope`, etc.) | Duration-aware LPT scheduling |
| Distributed testing | SSH/socket to remote machines | Local only (sharding for CI) |

**What's supported:**
- `-n auto` / `-n <number>` for parallel execution
- Duration-aware load balancing for optimal scheduling
- Session-scoped fixtures work correctly across workers

**What's different:**
- Distribution strategies (`--dist loadscope`, etc.) are not yet implemented
- Remote execution (`--tx ssh=...`) is not supported; use `--shard` for CI parallelism instead

For distributed CI testing across machines, use rpytest's native sharding:

```bash
# In CI matrix jobs:
rpytest --shard 0 --total-shards 4 --shard-strategy duration_balanced
rpytest --shard 1 --total-shards 4 --shard-strategy duration_balanced
# ... etc
```
