# Changelog

All notable changes to rpytest are documented here.

## [0.1.2] - 2026-05-07

### Added

- **State Machine Architecture**
  - Explicit `WorkerState` in worker pool (Spawning → Ready → Busy → Recycling → Dead)
  - Explicit `ExecutionState` for hybrid auto mode (ColdStart → Warming → Warm)
  - Explicit `WatchState` for watch mode (Idle → Debouncing → ComputingAffected → Recollecting → Running)
  - Explicit `StabilityState` for flakiness tracking (Unknown → Stable → Unstable → Flaky → ConfirmedFlaky)
  - Explicit `LifecycleManager` for daemon lifecycle with event-driven transitions

- **Distribution**
  - npm package (`npm install -g rpytest`)
  - Homebrew tap (`brew tap neul-labs/tap && brew install rpytest`)
  - PyPI package with bundled platform binaries

### Fixed

- CI workflow paths (`daemon/` → `pip-package/`)
- Version alignment across Cargo, Python, and documentation
- All placeholder URLs updated to `github.com/neul-labs/rpytest`
- License changed to MIT-only throughout

## [0.1.0] - 2024-01-15

### Added

- **Core Features**
  - Drop-in pytest replacement with Rust CLI
  - Python daemon for test execution
  - Native test collection via AST parsing
  - Full pytest fixture compatibility

- **Parallel Execution**
  - Built-in `-n` flag for parallel test execution
  - LPT (Longest Processing Time) scheduling algorithm
  - Duration tracking for optimal load balancing

- **Sharding**
  - `--shard` and `--total-shards` for CI distribution
  - Three strategies: duration_balanced, hash, round_robin
  - `--shard-info` for distribution analysis

- **Watch Mode**
  - `--watch` for automatic test re-runs
  - Dependency tracking for affected test detection
  - Debounced file change detection

- **Flakiness Detection**
  - `--reruns` for automatic retry of failed tests
  - `--flaky-report` for flakiness analysis
  - Per-test flakiness tracking

- **Fixture Reuse**
  - `--reuse-fixtures` for session fixture persistence
  - Automatic invalidation on conftest changes
  - `--session-status` for fixture inspection

- **Editor Integration**
  - `--editor-server` for IDE integration
  - JSON-RPC protocol over stdio
  - Test discovery and run-at-cursor

- **Daemon Management**
  - `--daemon` for foreground daemon mode
  - `--daemon-idle-timeout` for auto-shutdown
  - Auto-start on first invocation

- **Output Formats**
  - `--json` for JSON output
  - `--junitxml` for JUnit XML reports
  - Verbose/quiet modes

### Performance

- CLI startup: <10ms (vs pytest ~200ms)
- Collection: 10-28x faster than pytest
- Memory: 6-8x less than pytest CLI
- Parallel efficiency: 90%+ utilization

## [Unreleased]

### Planned Features

- **Incremental Collection**: Only re-parse changed files
- **Result Caching**: Skip tests when code unchanged
- **Distributed Execution**: Run across multiple machines
- **Coverage Integration**: Native coverage support
- **VS Code Extension**: Official extension for VS Code
- **Neovim Plugin**: neotest adapter

### Known Issues

- Some pytest plugins may not work correctly
- Custom conftest hooks have limited support
- Output format differs slightly from pytest

## Versioning

rpytest follows [Semantic Versioning](https://semver.org):

- **MAJOR**: Breaking changes to CLI or behavior
- **MINOR**: New features, backwards compatible
- **PATCH**: Bug fixes, backwards compatible

## Release Schedule

- **Stable releases**: As needed for significant features
- **Patch releases**: As needed for bug fixes
- **Pre-releases**: For testing new features

## Upgrading

### From 0.1.x to 0.1.2

No breaking changes. Direct upgrade supported.

### Configuration Changes

Check `pyproject.toml` for deprecated options:

```toml
# Deprecated
[tool.rpytest]
workers = 4  # Use parallel instead

# Current
[tool.rpytest]
parallel = 4
```

## Contributing

Contributions are welcome on [GitHub](https://github.com/neul-labs/rpytest):

- Development setup
- Code style guidelines
- Testing requirements
- Pull request process
