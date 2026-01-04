# Fixture Reuse

rpytest can reuse session-scoped fixtures across multiple test runs, dramatically reducing setup time.

## The Problem

Session fixtures are expensive:

```python
@pytest.fixture(scope="session")
def database():
    db = create_database()  # 30 seconds
    load_test_data(db)       # 10 seconds
    yield db
    db.close()
```

Every `rpytest` invocation recreates this fixture, even if nothing changed.

## The Solution

Enable fixture reuse:

```bash
rpytest tests/ --reuse-fixtures
```

The daemon keeps session fixtures alive between runs.

## How It Works

```
Run 1: rpytest tests/ --reuse-fixtures
  → Create database fixture (40s)
  → Run tests (10s)
  → Keep fixture in daemon

Run 2: rpytest tests/ --reuse-fixtures
  → Reuse database fixture (0s)
  → Run tests (10s)

Run 3: rpytest tests/ --reuse-fixtures
  → Reuse database fixture (0s)
  → Run tests (10s)
```

## Configuration

### Enable by Default

```toml
# pyproject.toml
[tool.rpytest]
enable_fixture_reuse = true
fixture_max_age = 600  # 10 minutes
```

### Command Line

```bash
# Enable
rpytest tests/ --reuse-fixtures

# Set max age
rpytest tests/ --reuse-fixtures --fixture-max-age=1800
```

## Fixture Invalidation

Fixtures are invalidated when:

### 1. conftest.py Changes

```
Modified: tests/conftest.py
→ All fixtures invalidated
→ Session fixtures recreated
```

### 2. Max Age Exceeded

```
Fixture age: 601 seconds
Max age: 600 seconds
→ Fixture invalidated
→ Recreated on next run
```

### 3. Explicit Cleanup

```bash
rpytest --cleanup
```

### 4. Daemon Restart

```bash
rpytest --daemon-stop
# Next run recreates all fixtures
```

## Session Status

View active fixtures:

```bash
rpytest tests/ --session-status
```

Output:

```
Session Status
==============
Session ID: sess-abc123
Repo: /path/to/project
Created: 2024-01-15 10:30:00
Last Run: 2024-01-15 10:45:00
Total Runs: 5

Active Session Fixtures:
  database
    Age: 15 minutes
    Created: 2024-01-15 10:30:00
    Reused: 4 times

  redis_client
    Age: 15 minutes
    Created: 2024-01-15 10:30:00
    Reused: 4 times

Fixture Reuse: ENABLED
Max Age: 600 seconds
```

## Supported Scopes

| Scope | Reusable | Notes |
|-------|----------|-------|
| `session` | ✅ | Fully supported |
| `package` | ✅ | Reused within package |
| `module` | ❌ | Too granular |
| `class` | ❌ | Too granular |
| `function` | ❌ | Never reused |

## Best Practices

### 1. Design for Reuse

```python
@pytest.fixture(scope="session")
def database(request):
    db = create_database()

    def cleanup():
        # Clean test data, keep structure
        db.execute("TRUNCATE ALL TABLES")

    request.addfinalizer(cleanup)
    return db
```

### 2. Use Transactions for Isolation

```python
@pytest.fixture(scope="session")
def database():
    return create_database()

@pytest.fixture
def db_session(database):
    # Each test gets isolated transaction
    session = database.begin_transaction()
    yield session
    session.rollback()
```

### 3. Handle State Carefully

```python
@pytest.fixture(scope="session")
def api_client():
    client = APIClient()
    return client

@pytest.fixture
def fresh_api_client(api_client):
    # Reset state for each test
    api_client.clear_cache()
    api_client.reset_rate_limits()
    return api_client
```

## Caveats

### State Leakage

If fixtures maintain state, it can leak between runs:

```python
# Dangerous: Accumulates state
@pytest.fixture(scope="session")
def counter():
    return {"count": 0}

def test_increment(counter):
    counter["count"] += 1
    assert counter["count"] == 1  # May fail on rerun!
```

Solution:

```python
@pytest.fixture
def counter():
    return {"count": 0}  # Fresh each test
```

### External Dependencies

External services may not match fixture state:

```python
@pytest.fixture(scope="session")
def external_api_state():
    setup_external_api()
    return get_api_state()

# If external API resets, fixture is stale
```

Solution:

```python
@pytest.fixture(scope="session")
def external_api_state():
    state = get_api_state()
    if not state.is_valid():
        setup_external_api()
        state = get_api_state()
    return state
```

### Memory Usage

Long-running sessions accumulate memory:

```bash
# Monitor daemon memory
rpytest --daemon-status

# Restart if needed
rpytest --daemon-stop
rpytest tests/ --reuse-fixtures
```

## CI/CD Considerations

### Fixture Cache Between Jobs

For multi-shard CI, each runner has its own daemon:

```yaml
# Fixtures created per-runner
jobs:
  test:
    strategy:
      matrix:
        shard: [0, 1, 2, 3]
    steps:
      - run: rpytest tests/ --shard=${{ matrix.shard }} --reuse-fixtures
```

### Persistent Daemon

For long CI pipelines:

```yaml
- name: Start daemon
  run: rpytest --daemon &

- name: Run unit tests
  run: rpytest tests/unit/ --reuse-fixtures

- name: Run integration tests
  run: rpytest tests/integration/ --reuse-fixtures

- name: Stop daemon
  run: rpytest --daemon-stop
```

## Troubleshooting

### Fixtures Not Reusing

1. Check if enabled:
   ```bash
   rpytest --session-status
   ```

2. Check fixture age:
   ```bash
   rpytest --session-status | grep Age
   ```

3. Check for conftest changes:
   ```bash
   git status tests/conftest.py
   ```

### Stale Fixtures

If tests fail due to stale fixtures:

```bash
# Force recreation
rpytest --daemon-stop
rpytest tests/ --reuse-fixtures
```

### Memory Growth

If daemon uses too much memory:

```bash
# Reduce max age
rpytest tests/ --reuse-fixtures --fixture-max-age=300

# Or disable for heavy fixtures
# Move to module scope instead of session
```
