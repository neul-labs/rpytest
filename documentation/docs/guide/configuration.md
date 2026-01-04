# Configuration

rpytest reads the same configuration files as pytest, ensuring seamless compatibility.

## Configuration Files

rpytest searches for configuration in this order:

1. `pytest.ini`
2. `pyproject.toml`
3. `setup.cfg`
4. `tox.ini`

## pytest.ini

```ini
[pytest]
testpaths = tests
python_files = test_*.py *_test.py
python_classes = Test*
python_functions = test_*
addopts = -v --tb=short
markers =
    slow: marks tests as slow
    integration: integration tests
filterwarnings =
    ignore::DeprecationWarning
```

## pyproject.toml

```toml
[tool.pytest.ini_options]
testpaths = ["tests"]
python_files = ["test_*.py", "*_test.py"]
python_classes = ["Test*"]
python_functions = ["test_*"]
addopts = "-v --tb=short"
markers = [
    "slow: marks tests as slow",
    "integration: integration tests",
]
filterwarnings = [
    "ignore::DeprecationWarning",
]

# rpytest-specific options
[tool.rpytest]
daemon_idle_timeout = 300
default_workers = "auto"
```

## setup.cfg

```ini
[tool:pytest]
testpaths = tests
python_files = test_*.py *_test.py
addopts = -v --tb=short
```

## Common Options

### testpaths

Directories to search for tests.

```ini
[pytest]
testpaths = tests src/tests
```

### python_files

Patterns for test file names.

```ini
[pytest]
python_files = test_*.py *_test.py check_*.py
```

### python_classes

Patterns for test class names.

```ini
[pytest]
python_classes = Test* *Tests *Suite
```

### python_functions

Patterns for test function names.

```ini
[pytest]
python_functions = test_* check_*
```

### addopts

Default command-line options.

```ini
[pytest]
addopts = -v --tb=short -x
```

### markers

Register custom markers.

```ini
[pytest]
markers =
    slow: marks tests as slow (deselect with '-m "not slow"')
    integration: integration tests requiring external services
    smoke: quick smoke tests for CI
```

### filterwarnings

Configure warning filters.

```ini
[pytest]
filterwarnings =
    error
    ignore::UserWarning
    ignore::DeprecationWarning:some_module.*
```

### norecursedirs

Directories to skip during collection.

```ini
[pytest]
norecursedirs = .git .tox venv __pycache__ build dist
```

### minversion

Minimum pytest version required.

```ini
[pytest]
minversion = 7.0
```

## Override Configuration

Override any ini option from the command line:

```bash
rpytest -o "addopts=" -o "testpaths=specific_tests"
```

## conftest.py

`conftest.py` files are fully supported for:

- Fixtures
- Hooks
- Plugins
- Configuration

```python
# conftest.py
import pytest

# Fixtures available to all tests in directory
@pytest.fixture(scope="session")
def database():
    db = connect()
    yield db
    db.close()

# Custom markers
def pytest_configure(config):
    config.addinivalue_line(
        "markers", "slow: marks tests as slow"
    )

# Modify collection
def pytest_collection_modifyitems(items):
    for item in items:
        if "slow" in item.nodeid:
            item.add_marker(pytest.mark.slow)
```

## rpytest-Specific Configuration

### pyproject.toml

```toml
[tool.rpytest]
# Daemon settings
daemon_idle_timeout = 300      # Auto-stop after 5 minutes idle
daemon_socket = "/tmp/rpytest.sock"

# Default parallel workers
default_workers = "auto"

# Sharding defaults
default_shard_strategy = "duration_balanced"

# Fixture reuse
enable_fixture_reuse = false
fixture_max_age = 600

# Watch mode
watch_ignore = ["*.pyc", "__pycache__", ".git"]
```

### Environment Variables

```bash
# Python interpreter
export PYTHON=/path/to/python3

# Runtime directory
export XDG_RUNTIME_DIR=/run/user/1000

# Logging
export RPYTEST_LOG=debug
```

## Per-Directory Configuration

Use `conftest.py` for directory-specific settings:

```python
# tests/integration/conftest.py
import pytest

# All tests in this directory are marked as integration
def pytest_collection_modifyitems(items):
    for item in items:
        item.add_marker(pytest.mark.integration)

# Slower timeout for integration tests
@pytest.fixture(autouse=True)
def slow_timeout():
    pytest.timeout = 60
```

## CI/CD Configuration

### GitHub Actions

```yaml
# .github/workflows/test.yml
name: Tests
on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Set up Python
        uses: actions/setup-python@v5
        with:
          python-version: '3.11'

      - name: Install dependencies
        run: |
          pip install -e .
          pip install rpytest-daemon

      - name: Run tests
        run: rpytest tests/ -v --junitxml=report.xml

      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: test-results
          path: report.xml
```

### Sharded Testing

```yaml
jobs:
  test:
    strategy:
      matrix:
        shard: [0, 1, 2, 3]
    steps:
      - run: |
          rpytest tests/ \
            --shard=${{ matrix.shard }} \
            --total-shards=4 \
            --shard-strategy=duration_balanced
```

## Debugging Configuration

### Show Effective Configuration

```bash
rpytest --co -v 2>&1 | head -20
```

### Override for Debugging

```bash
rpytest -o "addopts=" tests/test_specific.py -vvv --tb=long
```
