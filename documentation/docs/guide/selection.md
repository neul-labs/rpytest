# Test Selection

rpytest provides powerful test selection capabilities, compatible with pytest's filtering mechanisms.

## By Path

### Run All Tests

```bash
rpytest
```

### Specific Directory

```bash
rpytest tests/
rpytest tests/unit/
rpytest tests/integration/ tests/e2e/
```

### Specific File

```bash
rpytest tests/test_auth.py
rpytest tests/unit/test_models.py tests/unit/test_views.py
```

### Specific Test Function

```bash
rpytest tests/test_auth.py::test_login
```

### Specific Test Class

```bash
rpytest tests/test_auth.py::TestAuthentication
```

### Specific Test Method

```bash
rpytest tests/test_auth.py::TestAuthentication::test_login
```

### Parametrized Test Instance

```bash
rpytest "tests/test_math.py::test_add[1-2-3]"
```

## By Keyword (-k)

The `-k` flag filters tests by keyword expression.

### Simple Match

```bash
# Tests containing "login" in name
rpytest -k login
```

### Case Insensitive

```bash
# Both work
rpytest -k Login
rpytest -k LOGIN
```

### Boolean Expressions

```bash
# AND
rpytest -k "login and success"

# OR
rpytest -k "login or signup"

# NOT
rpytest -k "not slow"

# Combined
rpytest -k "auth and not integration"
rpytest -k "(login or signup) and not slow"
```

### Match Patterns

Keywords match against:

- Test function name
- Test class name
- Module name
- Markers

```python
# test_authentication.py
class TestLogin:
    def test_valid_credentials(self): ...
    def test_invalid_password(self): ...

class TestSignup:
    def test_new_user(self): ...
```

```bash
# Matches: TestLogin::test_valid_credentials, TestLogin::test_invalid_password
rpytest -k TestLogin

# Matches: test_valid_credentials
rpytest -k valid

# Matches: TestLogin::test_invalid_password
rpytest -k "Login and invalid"
```

## By Marker (-m)

### Using Built-in Markers

```python
import pytest

@pytest.mark.skip
def test_not_ready(): ...

@pytest.mark.skipif(sys.platform == "win32", reason="Unix only")
def test_unix_only(): ...

@pytest.mark.xfail
def test_known_issue(): ...
```

### Using Custom Markers

```python
import pytest

@pytest.mark.slow
def test_large_dataset(): ...

@pytest.mark.integration
def test_database_connection(): ...

@pytest.mark.smoke
def test_basic_functionality(): ...
```

Register in configuration:

```ini
[pytest]
markers =
    slow: marks tests as slow
    integration: integration tests
    smoke: quick smoke tests
```

### Marker Expressions

```bash
# Run only slow tests
rpytest -m slow

# Skip slow tests
rpytest -m "not slow"

# Run smoke tests
rpytest -m smoke

# Run integration but not slow
rpytest -m "integration and not slow"

# Multiple conditions
rpytest -m "(smoke or unit) and not flaky"
```

## By Last Run State

### Failed First (--ff)

Run tests that failed in the last run first, then the rest:

```bash
rpytest --ff
```

### Last Failed Only (--lf)

Run only tests that failed in the last run:

```bash
rpytest --lf
```

### New First (--nf)

Run new tests (not in cache) first:

```bash
rpytest --nf
```

## By Duration

### Show Slowest Tests

```bash
rpytest --durations=10          # 10 slowest
rpytest --durations=0           # All tests by duration
```

### Filter by Duration

Use markers combined with configuration:

```python
# conftest.py
import pytest

def pytest_collection_modifyitems(items, config):
    for item in items:
        # Mark tests expected to take > 10s
        if hasattr(item, 'timeout') and item.timeout > 10:
            item.add_marker(pytest.mark.slow)
```

## Combining Selections

All selection methods can be combined:

```bash
# Tests in tests/unit/ containing "auth" but not marked slow
rpytest tests/unit/ -k auth -m "not slow"

# Specific file, keyword filter, verbose
rpytest tests/test_api.py -k "get or post" -v

# Failed tests from last run, in specific directory
rpytest tests/integration/ --lf
```

## Collection Only

Preview which tests would run without executing:

```bash
rpytest --collect-only
rpytest --co                    # Short form

# With filters
rpytest -k auth --co
rpytest -m slow --co
```

Output:

```
<Module tests/test_auth.py>
  <Class TestLogin>
    <Function test_valid_credentials>
    <Function test_invalid_password>
  <Function test_logout>

3 tests collected
```

## Ignore Paths

### Ignore Directory

```bash
rpytest --ignore=tests/slow/
rpytest --ignore=tests/integration/ --ignore=tests/e2e/
```

### Ignore by Pattern

```bash
rpytest --ignore-glob="**/slow_*.py"
rpytest --ignore-glob="tests/**/test_integration_*.py"
```

### In Configuration

```ini
[pytest]
norecursedirs = .git __pycache__ .tox build dist slow_tests
```

## Deselection Patterns

### In conftest.py

```python
def pytest_collection_modifyitems(config, items):
    # Skip all integration tests on CI unless explicitly requested
    if os.environ.get('CI') and not config.getoption('-m'):
        skip_integration = pytest.mark.skip(reason="Skip integration on CI")
        for item in items:
            if "integration" in item.keywords:
                item.add_marker(skip_integration)
```

### Command Line Override

```bash
# Skip integration tests
rpytest -m "not integration"

# Force include
rpytest -m "integration"
```
