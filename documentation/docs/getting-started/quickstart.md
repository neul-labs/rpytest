# Quick Start

This guide will get you running tests with rpytest in under 5 minutes.

## Your First Test Run

If you have an existing pytest project, simply replace `pytest` with `rpytest`:

```bash
# Before
pytest tests/

# After
rpytest tests/
```

That's it! rpytest uses the same CLI interface as pytest.

## Common Commands

### Run All Tests

```bash
rpytest
```

### Run Specific File or Directory

```bash
rpytest tests/test_auth.py
rpytest tests/unit/
```

### Run Specific Test

```bash
rpytest tests/test_auth.py::test_login
rpytest tests/test_auth.py::TestAuth::test_login
```

### Filter by Keyword

```bash
# Run tests containing "auth" in their name
rpytest -k auth

# Run tests containing "auth" but not "slow"
rpytest -k "auth and not slow"
```

### Filter by Marker

```bash
# Run only tests marked as "slow"
rpytest -m slow

# Run tests NOT marked as slow
rpytest -m "not slow"
```

### Parallel Execution

```bash
# Run with 4 workers
rpytest -n 4

# Auto-detect number of CPUs
rpytest -n auto
```

### Stop on First Failure

```bash
rpytest -x
```

### Verbose Output

```bash
rpytest -v      # Verbose
rpytest -vv     # Very verbose
rpytest -vvv    # Debug level
```

## Example Project

Create a simple test file:

```python
# test_example.py

def test_addition():
    assert 1 + 1 == 2

def test_string():
    assert "hello".upper() == "HELLO"

class TestMath:
    def test_multiply(self):
        assert 2 * 3 == 6

    def test_divide(self):
        assert 10 / 2 == 5
```

Run it:

```bash
$ rpytest test_example.py -v

=== rpytest 0.1.0
test_example.py::test_addition PASSED
test_example.py::test_string PASSED
test_example.py::TestMath::test_multiply PASSED
test_example.py::TestMath::test_divide PASSED

=== 4 passed in 0.05s ===
```

## Using Fixtures

rpytest supports all pytest fixtures:

```python
# conftest.py
import pytest

@pytest.fixture
def database():
    db = connect_to_db()
    yield db
    db.close()

# test_db.py
def test_query(database):
    result = database.query("SELECT 1")
    assert result == 1
```

## Using Markers

```python
import pytest

@pytest.mark.slow
def test_large_dataset():
    # This test takes a while
    pass

@pytest.mark.skip(reason="Not implemented yet")
def test_future_feature():
    pass

@pytest.mark.parametrize("x,y,expected", [
    (1, 2, 3),
    (2, 3, 5),
    (10, 20, 30),
])
def test_addition(x, y, expected):
    assert x + y == expected
```

Run:

```bash
# Skip slow tests
rpytest -m "not slow"

# Only slow tests
rpytest -m slow
```

## Watch Mode

Re-run tests automatically when files change:

```bash
rpytest --watch
```

Press `Ctrl+C` to stop.

## Collect Only

See which tests would run without executing them:

```bash
rpytest --collect-only
```

## Next Steps

- [CLI Reference](../guide/cli.md) - Full list of command-line options
- [Configuration](../guide/configuration.md) - Configure rpytest with pytest.ini
- [Parallel Execution](../guide/parallel.md) - Speed up your test suite
