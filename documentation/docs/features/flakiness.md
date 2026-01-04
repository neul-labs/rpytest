# Flakiness Detection

rpytest automatically tracks test reliability and can detect and handle flaky tests.

## What is a Flaky Test?

A flaky test is one that passes and fails intermittently without code changes. Common causes:

- Race conditions
- Time-dependent logic
- External service dependencies
- Shared state between tests
- Non-deterministic data

## Automatic Rerun

### Basic Usage

```bash
rpytest tests/ --reruns=3
```

Failed tests are automatically retried up to 3 times.

### With Delay

Add delay between reruns (useful for rate-limited APIs):

```bash
rpytest tests/ --reruns=3 --reruns-delay=1000  # 1 second
```

### Only Known Flaky

Only rerun tests previously identified as flaky:

```bash
rpytest tests/ --reruns=3 --only-rerun-flaky
```

## Flakiness Tracking

rpytest tracks test outcomes over time to identify flaky tests.

### View Flakiness Report

```bash
rpytest tests/ --flaky-report
```

Output:

```
=== Flakiness Report ===

Flaky Tests (failure rate > 10%, alternating outcomes):
  tests/test_api.py::test_network_call
    Failure Rate: 15.2%
    Recent: PPFPPFP (7 runs)
    Flaky Streak: 3

  tests/test_db.py::test_concurrent_write
    Failure Rate: 8.5%
    Recent: PPPFPPP (7 runs)
    Flaky Streak: 1

Unstable Tests (high failure rate):
  tests/test_integration.py::test_external_service
    Failure Rate: 45.0%
    Recent: FPFFPFF (7 runs)
    Consecutive Failures: 2

Summary:
  Flaky: 2 tests
  Unstable: 1 test
  Stable: 497 tests
  Total Tracked: 500 tests
```

### Per-Test Details

```bash
rpytest tests/test_api.py::test_network_call --test-flakiness
```

Output:

```
Test: tests/test_api.py::test_network_call

Flakiness Status: FLAKY
Failure Rate: 15.2%
Total Runs: 33
Flaky Streak: 3

Recent Outcomes (last 10):
  P P F P P F P P P F

Statistics:
  Consecutive Passes: 0
  Consecutive Failures: 1
  Longest Pass Streak: 5
  Longest Fail Streak: 1
```

## Configuration

### In pyproject.toml

```toml
[tool.rpytest]
# Rerun configuration
reruns = 2
reruns_delay = 500  # ms
only_rerun_flaky = true

# Flakiness thresholds
flaky_failure_rate_threshold = 0.1  # 10%
flaky_min_runs = 5  # Minimum runs before marking flaky
```

### Command Line Override

```bash
rpytest tests/ --reruns=3 --reruns-delay=1000
```

## Marking Tests as Flaky

### Using Markers

```python
import pytest

@pytest.mark.flaky(reruns=3)
def test_network_dependent():
    # This test may fail due to network issues
    response = fetch_external_api()
    assert response.status == 200

@pytest.mark.flaky(reruns=5, reruns_delay=2)
def test_race_condition():
    # Needs multiple retries with delays
    result = async_operation()
    assert result.complete
```

### Conditional Flaky

```python
import pytest
import sys

@pytest.mark.flaky(reruns=3, condition=sys.platform == "linux")
def test_linux_specific():
    # Only flaky on Linux
    pass
```

## Handling Flaky Tests

### Strategy 1: Fix the Root Cause

Best approach - identify and fix the underlying issue:

```python
# Before: Flaky due to timing
def test_async_operation():
    start_operation()
    time.sleep(0.1)  # Hope it's done
    assert is_complete()

# After: Proper waiting
def test_async_operation():
    start_operation()
    wait_until(is_complete, timeout=5)
    assert is_complete()
```

### Strategy 2: Isolate Properly

```python
# Before: Shared state
DATABASE = []

def test_add_item():
    DATABASE.append("item")
    assert len(DATABASE) == 1  # Flaky if other tests run

# After: Isolated
@pytest.fixture
def database():
    return []

def test_add_item(database):
    database.append("item")
    assert len(database) == 1
```

### Strategy 3: Mark and Track

If immediate fix isn't possible:

```python
@pytest.mark.flaky(reruns=3)
@pytest.mark.xfail(reason="Known flaky - JIRA-1234")
def test_external_service():
    # Track in issue tracker
    pass
```

### Strategy 4: Quarantine

Move flaky tests to separate marker:

```python
@pytest.mark.quarantine
def test_flaky_integration():
    pass
```

```bash
# Regular CI
rpytest tests/ -m "not quarantine"

# Separate quarantine job
rpytest tests/ -m quarantine --reruns=5
```

## CI/CD Integration

### Rerun in CI

```yaml
# GitHub Actions
- name: Run tests with reruns
  run: rpytest tests/ --reruns=2 --junitxml=report.xml
```

### Flakiness Report in PR

```yaml
- name: Check flakiness
  run: |
    rpytest tests/ --flaky-report > flakiness.txt
    cat flakiness.txt

- name: Comment on PR
  if: github.event_name == 'pull_request'
  uses: actions/github-script@v6
  with:
    script: |
      const fs = require('fs');
      const report = fs.readFileSync('flakiness.txt', 'utf8');
      github.rest.issues.createComment({
        issue_number: context.issue.number,
        owner: context.repo.owner,
        repo: context.repo.repo,
        body: '```\n' + report + '\n```'
      });
```

### Track Flakiness Over Time

```yaml
- name: Upload flakiness data
  uses: actions/upload-artifact@v4
  with:
    name: flakiness-history
    path: .rpytest/flakiness/
    retention-days: 90
```

## Best Practices

1. **Don't ignore flaky tests** - They indicate real issues
2. **Set a flakiness budget** - e.g., "No more than 2% flaky tests"
3. **Track over time** - Monitor flakiness trends
4. **Fix proactively** - Address flaky tests before they become normal
5. **Use appropriate reruns** - 2-3 is usually enough

## Metrics

Track these metrics for test health:

| Metric | Good | Warning | Critical |
|--------|------|---------|----------|
| Flaky % | < 1% | 1-5% | > 5% |
| Rerun rate | < 2% | 2-10% | > 10% |
| Avg reruns | < 1.1 | 1.1-1.5 | > 1.5 |
