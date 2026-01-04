# Daemon Architecture

The Python daemon is the core execution engine of rpytest, handling test collection, scheduling, and execution.

## Lifecycle

```
┌─────────────────────────────────────────────────────────────┐
│                    Daemon Lifecycle                         │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  rpytest tests/  ──────► Daemon running?                   │
│                              │                              │
│                    No ◄──────┴──────► Yes                  │
│                     │                   │                   │
│                     ▼                   │                   │
│              Start daemon               │                   │
│              (background)               │                   │
│                     │                   │                   │
│                     └───────┬───────────┘                   │
│                             ▼                               │
│                      Send request                           │
│                             │                               │
│                             ▼                               │
│                    Receive results                          │
│                             │                               │
│                             ▼                               │
│                       Display output                        │
│                             │                               │
│                             ▼                               │
│                    Idle timeout?                            │
│                    (default: 300s)                          │
│                             │                               │
│                    Yes ◄────┴────► No (more requests)      │
│                     │                                       │
│                     ▼                                       │
│              Daemon exits                                   │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

## Auto-Start

When you run `rpytest`, it automatically starts the daemon if not running:

```bash
$ rpytest tests/
# If no daemon: starts one in background
# Connects and runs tests
# Daemon stays alive for 5 minutes of idle time
```

### Manual Control

```bash
# Run daemon in foreground (useful for debugging)
rpytest --daemon

# Custom idle timeout
rpytest --daemon --daemon-idle-timeout=600  # 10 minutes

# Disable auto-stop (daemon runs forever)
rpytest --daemon --daemon-idle-timeout=0
```

## IPC Communication

### Socket Location

```
Unix: /tmp/rpytest-{uid}/daemon.sock
```

### Protocol

Request-response over NNG sockets with MessagePack serialization:

```
┌──────────┐                    ┌──────────┐
│  Client  │                    │  Daemon  │
└────┬─────┘                    └────┬─────┘
     │                               │
     │  Collect { paths: [...] }     │
     │──────────────────────────────►│
     │                               │
     │  CollectResult { tests: [...]}│
     │◄──────────────────────────────│
     │                               │
     │  Run { tests: [...] }         │
     │──────────────────────────────►│
     │                               │
     │  TestResult { ... } (stream)  │
     │◄──────────────────────────────│
     │  TestResult { ... }           │
     │◄──────────────────────────────│
     │  RunComplete { summary }      │
     │◄──────────────────────────────│
```

### Message Types

```python
@dataclass
class CollectRequest:
    paths: List[str]
    markers: Optional[str]
    keywords: Optional[str]

@dataclass
class CollectResponse:
    tests: List[TestItem]
    duration_ms: int

@dataclass
class RunRequest:
    tests: List[str]  # node IDs
    parallel: int
    verbose: bool

@dataclass
class TestResult:
    node_id: str
    outcome: str  # passed, failed, skipped, error
    duration_ms: int
    message: Optional[str]
    stdout: Optional[str]
    stderr: Optional[str]
```

## Internal Architecture

```
┌────────────────────────────────────────────────────────────────┐
│                         DaemonServer                           │
├────────────────────────────────────────────────────────────────┤
│                                                                │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────┐   │
│  │  IPC Server │    │  Executor   │    │ Session Manager │   │
│  │  (NNG sock) │───►│             │◄───│                 │   │
│  └─────────────┘    └──────┬──────┘    └─────────────────┘   │
│                            │                                  │
│         ┌──────────────────┼──────────────────┐              │
│         ▼                  ▼                  ▼              │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐      │
│  │  Worker 1   │    │  Worker 2   │    │  Worker N   │      │
│  │ (subprocess)│    │ (subprocess)│    │ (subprocess)│      │
│  └─────────────┘    └─────────────┘    └─────────────┘      │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

### DaemonServer

Main server class that:
- Listens on Unix domain socket
- Routes requests to appropriate handlers
- Manages idle timeout for auto-shutdown
- Coordinates test execution

```python
class DaemonServer:
    def __init__(self, socket_path: str, idle_timeout: int = 300):
        self.socket_path = socket_path
        self.idle_timeout = idle_timeout
        self._last_activity = time.time()

    def run(self):
        while not self._should_shutdown():
            msg = self._receive(timeout=1.0)
            if msg:
                self._update_activity()
                self._handle(msg)
```

### Executor

Coordinates test execution:
- Maintains worker process pool
- Assigns tests to workers
- Collects and streams results
- Handles worker crashes

```python
class Executor:
    def __init__(self, parallel: int = 1):
        self.workers = [Worker() for _ in range(parallel)]
        self.scheduler = LPTScheduler()

    def run(self, tests: List[str]) -> Iterator[TestResult]:
        batches = self.scheduler.schedule(tests)
        for batch in batches:
            worker = self._get_available_worker()
            yield from worker.run(batch)
```

### Worker

Subprocess-based test runner:
- Fresh Python process per batch
- Captures stdout/stderr
- Handles timeouts
- Reports results back

```python
class Worker:
    def run(self, tests: List[str]) -> Iterator[TestResult]:
        proc = subprocess.Popen(
            [sys.executable, "-m", "rpytest_daemon.runner"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
        )
        # Send tests, receive results
```

## Collection Strategies

### Native Collector (Fast Path)

AST-based collection without imports:

```python
class NativeCollector:
    def collect(self, path: Path) -> List[TestItem]:
        tree = ast.parse(path.read_text())
        tests = []
        for node in ast.walk(tree):
            if isinstance(node, ast.FunctionDef):
                if node.name.startswith("test_"):
                    tests.append(self._make_item(node))
            elif isinstance(node, ast.ClassDef):
                if node.name.startswith("Test"):
                    tests.extend(self._collect_class(node))
        return tests
```

**Advantages**:
- 10-100x faster than import-based
- No side effects
- Parallel collection

**Limitations**:
- Can't detect dynamically generated tests
- Limited marker information

### Pytest Collector (Fallback)

Full pytest collection for complex cases:

```python
class PytestCollector:
    def collect(self, paths: List[Path]) -> List[TestItem]:
        # Uses pytest's collection machinery
        # Handles all edge cases
        # Slower but complete
```

## Scheduling

### LPT Algorithm

Longest Processing Time first for optimal load balancing:

```
Tests (by duration): [10s, 8s, 6s, 4s, 3s, 2s]
Workers: 2

Assignment:
  Worker 1: [10s, 4s, 2s] = 16s
  Worker 2: [8s, 6s, 3s] = 17s

Total time: 17s (vs 33s sequential)
```

### Duration Tracking

Test durations stored in `.rpytest/durations/`:

```json
{
  "tests/test_api.py::test_auth": 0.5,
  "tests/test_api.py::test_users": 1.2,
  "tests/test_db.py::test_query": 0.3
}
```

## Session Fixtures

Session-scoped fixtures persist across test runs:

```python
class SessionManager:
    def __init__(self):
        self._fixtures = {}
        self._fixture_ages = {}

    def get_fixture(self, name: str) -> Any:
        if name in self._fixtures:
            if not self._is_stale(name):
                return self._fixtures[name]
        return None

    def store_fixture(self, name: str, value: Any):
        self._fixtures[name] = value
        self._fixture_ages[name] = time.time()
```

### Invalidation

Fixtures are invalidated when:
- `conftest.py` changes
- Max age exceeded (default: 10 minutes)
- Manual cleanup (`rpytest --cleanup`)
- Daemon restart

## Error Handling

### Worker Crash

```
Worker process exits unexpectedly
  │
  ├── Mark current test as ERROR
  │
  ├── Spawn replacement worker
  │
  └── Continue with remaining tests
```

### IPC Timeout

```
No response within timeout
  │
  ├── Check if daemon alive
  │     │
  │     └── Dead? Restart daemon
  │
  └── Retry request (max 3 times)
```

## Debugging

### Daemon Logs

```bash
# Run daemon with debug logging
RUST_LOG=debug rpytest --daemon
```

### Check Status

```bash
# Check if daemon is running
rpytest --daemon-status

# View session info
rpytest --session-status
```

### Socket Inspection

```bash
# Check socket exists
ls -la /tmp/rpytest-$(id -u)/daemon.sock

# Monitor IPC traffic (requires socat)
socat -v UNIX-CONNECT:/tmp/rpytest-$(id -u)/daemon.sock -
```
