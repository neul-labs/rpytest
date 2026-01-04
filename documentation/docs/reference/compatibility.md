# Compatibility

rpytest aims to be a drop-in replacement for pytest. This document details compatibility status and known differences.

## pytest Compatibility

### Supported Features

| Feature | Status | Notes |
|---------|--------|-------|
| Test discovery | Full | `test_*.py`, `Test*` classes |
| Assertions | Full | Native pytest assertions |
| Fixtures | Full | All scopes supported |
| Markers | Full | `@pytest.mark.*` |
| Parametrization | Full | `@pytest.mark.parametrize` |
| conftest.py | Full | Hierarchical fixtures |
| CLI arguments | Partial | Common flags supported |
| Plugins | Full | Via pytest execution |
| Hooks | Partial | Common hooks |

### CLI Compatibility

Supported pytest flags:

```bash
# Test selection
rpytest tests/                    # Path
rpytest tests/test_api.py         # File
rpytest tests/test_api.py::test_x # Node ID
rpytest -k "auth"                 # Keyword
rpytest -m "not slow"             # Marker

# Output
rpytest -v                        # Verbose
rpytest -vv                       # Very verbose
rpytest -q                        # Quiet
rpytest --tb=short                # Traceback style

# Execution
rpytest -x                        # Stop on first failure
rpytest --lf                      # Last failed
rpytest --ff                      # Failed first
rpytest -n 4                      # Parallel workers

# Collection
rpytest --collect-only            # List tests
rpytest --co                      # Short form

# Configuration
rpytest -c pytest.ini             # Config file
rpytest -o "key=value"            # Override option
```

### Unsupported/Different Flags

| Flag | Status | Alternative |
|------|--------|-------------|
| `--dist` | Not needed | Use `--shard` |
| `--boxed` | Not supported | Default isolation |
| `--forked` | Not supported | Use subprocess workers |
| `--cov` | Plugin-based | Use pytest directly |

## Python Version Support

| Version | Status |
|---------|--------|
| Python 3.8 | Supported |
| Python 3.9 | Supported |
| Python 3.10 | Supported |
| Python 3.11 | Supported |
| Python 3.12 | Supported |
| Python 3.13 | Supported |

## pytest Version Support

| Version | Status |
|---------|--------|
| pytest 7.0+ | Full support |
| pytest 8.0+ | Full support |
| pytest 9.0+ | Full support |

## Plugin Compatibility

### Fully Compatible

These plugins work without modification:

- pytest-mock
- pytest-asyncio
- pytest-django
- pytest-flask
- pytest-factoryboy
- pytest-freezegun
- pytest-timeout
- pytest-env

### Partially Compatible

These plugins work but may have limitations:

| Plugin | Notes |
|--------|-------|
| pytest-xdist | Use rpytest's native `-n` instead |
| pytest-cov | Use directly with pytest |
| pytest-html | Report generation may differ |
| pytest-sugar | Terminal output different |

### Not Recommended

| Plugin | Reason |
|--------|--------|
| pytest-parallel | Conflicts with rpytest parallelism |
| pytest-randomly | Seed handling may differ |

## Feature Parity

### Markers

```python
# All standard markers work
@pytest.mark.skip
@pytest.mark.skipif
@pytest.mark.xfail
@pytest.mark.parametrize
@pytest.mark.usefixtures

# Custom markers
@pytest.mark.slow
@pytest.mark.integration
```

### Fixtures

```python
# All scopes
@pytest.fixture(scope="function")
@pytest.fixture(scope="class")
@pytest.fixture(scope="module")
@pytest.fixture(scope="package")
@pytest.fixture(scope="session")

# Autouse
@pytest.fixture(autouse=True)

# Parametrized fixtures
@pytest.fixture(params=[1, 2, 3])
```

### Parametrization

```python
# Simple
@pytest.mark.parametrize("x", [1, 2, 3])

# Multiple parameters
@pytest.mark.parametrize("x,y", [(1, 2), (3, 4)])

# Nested
@pytest.mark.parametrize("x", [1, 2])
@pytest.mark.parametrize("y", ["a", "b"])

# IDs
@pytest.mark.parametrize("x", [1, 2], ids=["one", "two"])
```

### Hooks

Supported hooks:

```python
# conftest.py

def pytest_configure(config):
    """Modify configuration."""
    pass

def pytest_collection_modifyitems(items):
    """Modify collected tests."""
    pass

def pytest_runtest_setup(item):
    """Called before test setup."""
    pass

def pytest_runtest_teardown(item):
    """Called after test teardown."""
    pass
```

## Verification

### Drop-in Verification

Verify rpytest matches pytest behavior:

```bash
rpytest --verify-dropin tests/
```

This runs tests with both rpytest and pytest and compares results.

### Output Comparison

```bash
# Generate pytest results
pytest tests/ --junitxml=pytest-results.xml

# Generate rpytest results
rpytest tests/ --junitxml=rpytest-results.xml

# Compare
diff pytest-results.xml rpytest-results.xml
```

## Known Differences

### Output Format

rpytest output differs slightly from pytest:

```
# pytest
test_api.py::test_login PASSED

# rpytest
tests/test_api.py::test_login ✓
```

### Collection Order

Test collection order may differ:
- pytest: Import order
- rpytest: Filesystem order (faster)

### Fixture Timing

With `--reuse-fixtures`, session fixtures persist:
- pytest: Recreated each run
- rpytest: Reused across runs

### Error Messages

Stack traces may have slight formatting differences.

## Migration Guide

### From pytest

1. Replace `pytest` with `rpytest`:
   ```bash
   # Before
   pytest tests/ -v

   # After
   rpytest tests/ -v
   ```

2. Replace `pytest-xdist` with native parallelism:
   ```bash
   # Before
   pytest tests/ -n 4

   # After
   rpytest tests/ -n 4
   ```

3. Verify compatibility:
   ```bash
   rpytest --verify-dropin tests/
   ```

### CI/CD Migration

```yaml
# Before
- run: pip install pytest pytest-xdist
- run: pytest tests/ -n auto

# After
- run: pip install rpytest
- run: rpytest tests/ -n auto
```

### Mixed Usage

Run both for comparison:

```bash
# Development: rpytest for speed
rpytest tests/ --watch

# CI: pytest for coverage
pytest tests/ --cov
```

## Reporting Issues

If you find compatibility issues:

1. Check this document for known differences
2. Try with pytest to confirm different behavior
3. Report at https://github.com/anthropics/rpytest/issues

Include:
- pytest version
- rpytest version
- Minimal reproduction
- Expected vs actual behavior
