# Architecture Overview

rpytest uses a hybrid Rust/Python architecture to achieve maximum performance while maintaining full pytest compatibility.

## High-Level Design

```
┌────────────────────────────────────────────────────────────────┐
│                         rpytest CLI                            │
│                        (Rust Binary)                           │
├────────────────────────────────────────────────────────────────┤
│  Argument Parsing │ Config Loading │ Output Formatting │ Watch │
└─────────────────────────────┬──────────────────────────────────┘
                              │ IPC (NNG + MessagePack)
                              ▼
┌────────────────────────────────────────────────────────────────┐
│                       Python Daemon                            │
│                   (rpytest_daemon package)                     │
├────────────────────────────────────────────────────────────────┤
│ Test Collection │ Scheduling │ Workers │ Fixtures │ Reporting  │
└────────────────────────────────────────────────────────────────┘
```

## Why Hybrid?

### Rust for CLI

- **Fast startup**: Sub-10ms cold start
- **Efficient I/O**: Native file watching, parallel file reading
- **Low memory**: Minimal overhead for coordination
- **Single binary**: No Python environment needed for invocation

### Python for Execution

- **Full compatibility**: Native pytest plugin loading
- **Fixture support**: Complex fixture scoping and dependency resolution
- **Plugin ecosystem**: All pytest plugins work out of the box
- **Dynamic features**: Runtime test generation, parametrization

## Component Breakdown

### 1. CLI Layer (`crates/rpytest/`)

```
crates/rpytest/src/
├── cli/
│   ├── args.rs       # Argument parsing (clap)
│   ├── output.rs     # Terminal output formatting
│   └── json_output.rs # JSON report generation
├── config/
│   └── mod.rs        # Config file loading (pyproject.toml)
├── daemon/
│   ├── client.rs     # Daemon communication
│   └── lifecycle.rs  # Auto-start/stop management
├── watch/
│   └── dependency.rs # File change tracking
└── main.rs           # Entry point
```

### 2. IPC Layer (`crates/rpytest-ipc/`)

Handles communication between Rust and Python:

```rust
// Message protocol
pub enum Request {
    Collect { paths: Vec<String>, ... },
    Run { tests: Vec<String>, ... },
    Shutdown,
}

pub enum Response {
    CollectResult { tests: Vec<TestItem> },
    TestResult { node_id: String, outcome: Outcome, ... },
    Error { message: String },
}
```

- **Transport**: NNG (nanomsg-next-gen) sockets
- **Serialization**: MessagePack for efficiency
- **Protocol**: Request-response with streaming results

### 3. Core Types (`crates/rpytest-core/`)

Shared data structures:

```rust
pub struct TestItem {
    pub node_id: String,
    pub name: String,
    pub path: PathBuf,
    pub line: usize,
    pub markers: Vec<String>,
}

pub enum Outcome {
    Passed,
    Failed { message: String },
    Skipped { reason: String },
    Error { message: String },
}
```

### 4. Python Daemon (`daemon/rpytest_daemon/`)

```
daemon/rpytest_daemon/
├── cli.py           # Daemon entry point
├── server.py        # IPC server implementation
├── protocol.py      # Message serialization
├── native_collector.py  # AST-based fast collection
├── executor.py      # Test execution coordination
├── worker.py        # Individual test runner
├── scheduler.py     # LPT-based work distribution
├── sharding.py      # CI/CD test distribution
└── fixtures.py      # Session fixture management
```

## Data Flow

### Collection Phase

```
1. CLI receives test paths
2. CLI sends Collect request to daemon
3. Daemon uses native_collector for fast AST parsing
4. Daemon falls back to pytest for complex cases
5. Daemon returns test list with metadata
6. CLI filters and displays results
```

### Execution Phase

```
1. CLI sends Run request with test IDs
2. Daemon scheduler assigns tests to workers
3. Workers execute via subprocess (isolation)
4. Results stream back through IPC
5. CLI formats and displays progress
6. Final summary computed and shown
```

## Key Design Decisions

### 1. Daemon Model

**Decision**: Persistent daemon vs. fresh process per run

**Rationale**:
- Session fixtures need to persist across runs
- Import caching saves startup time
- Worker process pool stays warm

### 2. Subprocess Workers

**Decision**: Each test run in subprocess vs. in-process

**Rationale**:
- Complete isolation between tests
- Crash in one test doesn't affect others
- Clean import state for each test

### 3. Native Collection

**Decision**: AST parsing in Python vs. pytest collection

**Rationale**:
- 10-100x faster than pytest's import-based collection
- No side effects during collection
- Falls back to pytest for edge cases

### 4. IPC Protocol

**Decision**: NNG + MessagePack vs. stdio/JSON

**Rationale**:
- Binary protocol is faster than text
- Async socket supports streaming results
- Structured messages prevent parsing errors

## Extension Points

### Custom Schedulers

```python
class CustomScheduler:
    def schedule(self, tests: List[TestItem]) -> List[Batch]:
        # Custom test ordering logic
        pass
```

### Custom Reporters

```python
class CustomReporter:
    def on_test_start(self, test: TestItem): ...
    def on_test_end(self, test: TestItem, result: Result): ...
    def on_session_end(self, summary: Summary): ...
```

## Performance Characteristics

| Component | Startup | Memory | Throughput |
|-----------|---------|--------|------------|
| Rust CLI | <10ms | ~5MB | N/A |
| Python Daemon | ~100ms | ~30MB | N/A |
| Worker Process | ~50ms | ~20MB | ~100 tests/s |
| IPC | <1ms/msg | ~1KB/msg | ~10k msg/s |

## Security Model

- Daemon runs as current user
- Socket permissions restricted to user
- No network exposure (Unix domain socket)
- Tests run in isolated subprocesses
