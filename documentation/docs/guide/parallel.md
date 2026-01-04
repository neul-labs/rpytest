# Parallel Execution

rpytest has built-in parallel test execution, eliminating the need for pytest-xdist.

## Basic Usage

```bash
# Run with 4 parallel workers
rpytest -n 4

# Auto-detect number of CPUs
rpytest -n auto

# Sequential execution (single worker)
rpytest -n 1
```

## How It Works

Unlike pytest-xdist, rpytest's parallel execution uses a warm worker pool:

```
┌─────────────┐
│  Rust CLI   │
└──────┬──────┘
       │ IPC
       ▼
┌─────────────────────────────┐
│      Python Daemon          │
│  ┌─────┐ ┌─────┐ ┌─────┐   │
│  │ W1  │ │ W2  │ │ W3  │   │
│  └─────┘ └─────┘ └─────┘   │
│     Warm Worker Pool        │
└─────────────────────────────┘
```

Benefits:

- **No startup overhead**: Workers are pre-warmed
- **Shared fixtures**: Session fixtures loaded once
- **Smart scheduling**: LPT algorithm for load balancing

## Performance Comparison

| Configuration | pytest-xdist | rpytest |
|---------------|--------------|---------|
| 500 tests, -n 4 | 0.87s | 0.25s |
| 500 tests, -n auto | 1.90s | 0.20s |

rpytest is **3.5-9.5x faster** than pytest-xdist for parallel execution.

## Worker Count

### Auto Detection

```bash
rpytest -n auto
```

Uses `os.cpu_count()` to determine optimal worker count.

### Manual Setting

```bash
rpytest -n 4
rpytest -n 8
rpytest -n 16
```

### Configuration Default

```toml
# pyproject.toml
[tool.rpytest]
default_workers = "auto"
```

## Scheduling Algorithm

rpytest uses the LPT (Longest Processing Time) algorithm:

1. Tests are sorted by historical duration (longest first)
2. Each test is assigned to the worker with the least total work
3. New tests (no history) are distributed round-robin

This ensures balanced workloads across workers.

### Duration History

Test durations are tracked automatically:

```bash
# First run: round-robin distribution
rpytest tests/ -n 4

# Subsequent runs: duration-balanced distribution
rpytest tests/ -n 4
```

View duration history:

```bash
rpytest --inventory-status -v
```

## Fixture Handling

### Session Fixtures

Session-scoped fixtures are shared across workers:

```python
@pytest.fixture(scope="session")
def database():
    # Created once, shared by all workers
    db = create_database()
    yield db
    db.close()
```

### Module Fixtures

Module-scoped fixtures are created per-worker as needed:

```python
@pytest.fixture(scope="module")
def api_client():
    # Created once per module, per worker
    return APIClient()
```

### Function Fixtures

Function-scoped fixtures work normally:

```python
@pytest.fixture
def user():
    # Created for each test
    return User.create()
```

## Test Isolation

Each worker runs in isolation:

- Separate Python interpreter state
- No shared global variables
- Independent import state

!!! warning "Shared Resources"
    Tests modifying shared resources (files, databases) need proper synchronization:

    ```python
    @pytest.fixture
    def temp_file(tmp_path):
        # tmp_path is unique per test
        return tmp_path / "data.txt"
    ```

## Output Handling

### Default Output

Tests results are streamed as they complete:

```
test_a.py::test_1 PASSED
test_b.py::test_2 PASSED
test_a.py::test_3 FAILED
```

### Verbose Output

```bash
rpytest -n 4 -v
```

Shows worker assignment:

```
[worker 0] test_a.py::test_1 PASSED
[worker 1] test_b.py::test_2 PASSED
[worker 0] test_a.py::test_3 FAILED
```

### Quiet Output

```bash
rpytest -n 4 -q
```

Shows progress dots:

```
...F....
```

## Debugging Parallel Tests

### Single Worker Mode

Disable parallelism for debugging:

```bash
rpytest tests/test_failing.py -n 1 --tb=long
```

### Specific Test

```bash
rpytest tests/test_failing.py::test_specific -n 1 -vvv
```

### With Debugger

```bash
rpytest tests/test_failing.py::test_specific --pdb -n 1
```

## CI/CD Integration

### GitHub Actions

```yaml
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: rpytest tests/ -n auto --junitxml=report.xml
```

### With Sharding

For very large test suites, combine parallelism with sharding:

```yaml
jobs:
  test:
    strategy:
      matrix:
        shard: [0, 1, 2, 3]
    steps:
      - run: |
          rpytest tests/ \
            -n auto \
            --shard=${{ matrix.shard }} \
            --total-shards=4
```

## Troubleshooting

### Tests Pass Sequentially, Fail in Parallel

Common causes:

1. **Shared state**: Tests modify global variables
2. **File conflicts**: Tests use same temp files
3. **Database races**: Tests don't isolate transactions

Solutions:

```python
# Use unique temporary paths
@pytest.fixture
def unique_file(tmp_path_factory):
    return tmp_path_factory.mktemp("data") / "file.txt"

# Use database transactions
@pytest.fixture
def db_session():
    session = create_session()
    session.begin_nested()  # Savepoint
    yield session
    session.rollback()
```

### Slow Parallel Performance

If parallel mode is slower than sequential:

1. **Small test suite**: Overhead exceeds benefit
2. **Long setup**: Session fixtures dominate runtime
3. **I/O bound**: Workers compete for I/O

```bash
# Check if sequential is faster
time rpytest tests/ -n 1
time rpytest tests/ -n 4
```

### Memory Issues

With many workers:

```bash
# Reduce worker count
rpytest tests/ -n 4  # Instead of -n auto
```

Or use sharding instead:

```bash
rpytest tests/ --shard=0 --total-shards=4
```
