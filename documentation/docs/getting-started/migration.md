# Migration from pytest

rpytest is designed as a drop-in replacement for pytest. Most projects can switch with zero configuration changes.

## Basic Migration

### Step 1: Install rpytest

```bash
cargo install rpytest
pip install rpytest-daemon
```

### Step 2: Replace pytest with rpytest

```bash
# Before
pytest tests/ -v

# After
rpytest tests/ -v
```

### Step 3: Verify Compatibility

```bash
rpytest --verify-dropin tests/
```

This runs both pytest and rpytest and compares results.

## What Works Out of the Box

### CLI Flags

| Flag | Description | Supported |
|------|-------------|-----------|
| `-k EXPR` | Keyword filter | ✅ |
| `-m MARKER` | Marker filter | ✅ |
| `-x` | Exit on first failure | ✅ |
| `--maxfail=N` | Stop after N failures | ✅ |
| `-v` / `-vv` | Verbose output | ✅ |
| `-q` / `-qq` | Quiet output | ✅ |
| `--tb=STYLE` | Traceback style | ✅ |
| `--collect-only` | Collection only | ✅ |
| `-n N` | Parallel workers | ✅ (built-in) |
| `--lf` | Last failed | ✅ |
| `--ff` | Failed first | ✅ |
| `--nf` | New first | ✅ |
| `--durations=N` | Show slowest tests | ✅ |
| `--junitxml=PATH` | JUnit output | ✅ |
| `-p PLUGIN` | Load plugin | ✅ |
| `-o OPT=VAL` | Override ini | ✅ |

### Configuration Files

rpytest reads the same configuration files:

- `pytest.ini`
- `pyproject.toml` (`[tool.pytest.ini_options]`)
- `setup.cfg` (`[tool:pytest]`)
- `tox.ini` (`[pytest]`)

### Fixtures

All fixture scopes work:

```python
@pytest.fixture(scope="function")  # ✅
@pytest.fixture(scope="class")     # ✅
@pytest.fixture(scope="module")    # ✅
@pytest.fixture(scope="session")   # ✅
```

### Markers

Built-in markers work:

```python
@pytest.mark.skip          # ✅
@pytest.mark.skipif        # ✅
@pytest.mark.xfail         # ✅
@pytest.mark.parametrize   # ✅
@pytest.mark.usefixtures   # ✅
```

Custom markers work too:

```python
@pytest.mark.slow          # ✅
@pytest.mark.integration   # ✅
```

## CI/CD Migration

### GitHub Actions

```yaml
# Before
- name: Run tests
  run: pytest tests/ -v

# After
- name: Run tests
  run: rpytest tests/ -v
```

### With Sharding

Replace pytest-xdist distributed testing with rpytest's native sharding:

```yaml
# Before (pytest-xdist)
jobs:
  test:
    strategy:
      matrix:
        group: [1, 2, 3, 4]
    steps:
      - run: pytest --dist=loadfile -n auto --group=${{ matrix.group }} --group-count=4

# After (rpytest)
jobs:
  test:
    strategy:
      matrix:
        shard: [0, 1, 2, 3]
    steps:
      - run: rpytest --shard=${{ matrix.shard }} --total-shards=4 --shard-strategy=duration_balanced
```

## Plugin Compatibility

### Fully Compatible

Plugins that don't modify collection or execution deeply:

- pytest-cov
- pytest-mock
- pytest-env
- pytest-timeout

### Passthrough Support

Unknown flags are passed to the daemon:

```bash
rpytest tests/ -- --cov=myapp --cov-report=xml
```

### Not Yet Supported

Some plugins require deeper integration:

- pytest-xdist (use rpytest's built-in `-n` instead)
- pytest-bdd (partial support)
- pytest-django (partial support, testing in progress)

## Performance Comparison

After migration, you should see improvements:

```bash
# Measure pytest
time pytest tests/ -q

# Measure rpytest
time rpytest tests/ -q
```

Typical results:

| Metric | pytest | rpytest |
|--------|--------|---------|
| Cold start | 2.5s | 1.5s |
| Warm start | 0.6s | 0.3s |
| Memory | 40MB | 6MB |

## Rollback Plan

If you encounter issues, rolling back is simple:

```bash
# Just use pytest again
pytest tests/
```

Your configuration files, fixtures, and tests remain unchanged.

## Troubleshooting Migration

### Tests Pass in pytest But Fail in rpytest

Run verification:

```bash
rpytest --verify-dropin tests/
```

This will show any differences in:

- Number of tests collected
- Test outcomes
- Output differences

### Missing Plugin

If a plugin isn't working:

```bash
# Pass flags through
rpytest tests/ -- --plugin-flag=value
```

### Import Errors

Ensure the daemon package is installed in the same environment:

```bash
pip install rpytest-daemon
```

### Collection Differences

rpytest uses AST-based collection for speed. If you have dynamic test generation:

```bash
# Force pytest-style collection
rpytest tests/ --no-native-collection
```

### Orphaned Daemon Processes

If you previously used an older version of rpytest with a Python daemon, there may be orphaned Python daemon processes still running after upgrading to the Rust-based daemon. To check for and clean up these processes:

```bash
# Check for orphaned Python daemon processes
ps aux | grep rpytest_daemon | grep -v grep

# Kill any orphaned Python daemon processes
pkill -f "python.*rpytest_daemon"

# Alternatively, kill by specific pattern
pkill -9 -f rpytest_daemon

# Stop the current Rust daemon and restart fresh
rpytest --daemon-stop
rpytest tests/ --collect-only  # This will start a fresh daemon
```

Signs of orphaned daemon processes include:
- Socket conflicts when starting the daemon
- Old PID files pointing to non-existent processes
- Multiple daemon processes consuming memory

The Rust daemon (rpytest-daemon) runs as a separate binary and writes its PID to `/run/user/$UID/rpytest.pid`. Use `rpytest --daemon-status` to check the current daemon state.
