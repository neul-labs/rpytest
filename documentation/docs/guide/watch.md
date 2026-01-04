# Watch Mode

Watch mode automatically re-runs tests when files change, providing instant feedback during development.

## Basic Usage

```bash
rpytest --watch
```

This will:

1. Run all tests initially
2. Watch for file changes
3. Re-run affected tests automatically

Press `Ctrl+C` to stop.

## How It Works

```
┌─────────────────────────────┐
│       File Watcher          │
│   (inotify/fsevents/kqueue) │
└─────────────┬───────────────┘
              │ Change detected
              ▼
┌─────────────────────────────┐
│    Dependency Tracker       │
│  (test ← source mappings)   │
└─────────────┬───────────────┘
              │ Affected tests
              ▼
┌─────────────────────────────┐
│     Test Runner             │
│   (run affected only)       │
└─────────────────────────────┘
```

## Affected Test Detection

### Test File Changes

When a test file changes, tests in that file are re-run:

```
Modified: tests/test_auth.py
Running: tests/test_auth.py::test_login, tests/test_auth.py::test_logout
```

### Source File Changes

When a source file changes, dependent tests are re-run:

```
Modified: src/auth/login.py
Running: tests/test_auth.py::test_login, tests/test_api.py::test_auth_flow
```

### conftest.py Changes

When conftest.py changes, all tests in that directory tree are re-run:

```
Modified: tests/conftest.py
Running: All tests in tests/
```

## Filtering in Watch Mode

### Keyword Filter

```bash
rpytest --watch -k auth
```

Only runs tests matching "auth" when changes are detected.

### Marker Filter

```bash
rpytest --watch -m "not slow"
```

Skips slow tests during watch.

### Path Filter

```bash
rpytest tests/unit/ --watch
```

Only watches and runs tests in `tests/unit/`.

## Configuration

### Ignore Patterns

In `pyproject.toml`:

```toml
[tool.rpytest]
watch_ignore = [
    "*.pyc",
    "__pycache__",
    ".git",
    ".tox",
    "*.egg-info",
    "build",
    "dist",
]
```

### Debounce

Multiple rapid changes are debounced (200ms default):

```
File changed: auth.py
File changed: auth.py (debounced)
File changed: auth.py (debounced)
Running tests... (after 200ms quiet)
```

## Output

### Normal Output

```
=== rpytest 0.1.0 [watch mode]
Watching /path/to/project for changes...
Press Ctrl+C to stop

Running initial test suite...
=== 50 passed in 1.23s ===

Waiting for file changes...

Detected 1 file change(s):
  tests/test_auth.py (modified)
Running 5 affected test(s)...
=== 5 passed in 0.15s ===

Waiting for file changes...
```

### Verbose Output

```bash
rpytest --watch -v
```

Shows individual test results:

```
Detected 1 file change(s):
  tests/test_auth.py (modified)
Running 5 affected test(s)...
tests/test_auth.py::test_login PASSED
tests/test_auth.py::test_logout PASSED
tests/test_auth.py::test_register PASSED
tests/test_auth.py::TestAuth::test_refresh PASSED
tests/test_auth.py::TestAuth::test_revoke PASSED
=== 5 passed in 0.15s ===
```

## Editor Integration

### VS Code

Add to tasks.json:

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "Watch Tests",
      "type": "shell",
      "command": "rpytest --watch",
      "isBackground": true,
      "problemMatcher": {
        "pattern": {
          "regexp": "^(.+)::(\\d+)::(.+)$",
          "file": 1,
          "line": 2,
          "message": 3
        },
        "background": {
          "activeOnStart": true,
          "beginsPattern": "Running",
          "endsPattern": "passed|failed"
        }
      }
    }
  ]
}
```

### Terminal Split

Run in a split terminal for side-by-side development:

```bash
# Terminal 1: Watch mode
rpytest tests/unit/ --watch

# Terminal 2: Edit code
vim src/mymodule.py
```

## Best Practices

### Focus on Relevant Tests

```bash
# During feature development
rpytest tests/unit/test_new_feature.py --watch

# After core changes
rpytest tests/ --watch -m "not slow"
```

### Combine with Markers

Mark quick tests for watch mode:

```python
@pytest.mark.quick
def test_basic_auth():
    pass
```

```bash
rpytest --watch -m quick
```

### Handle Flaky Tests

If flaky tests interrupt flow:

```bash
rpytest --watch --reruns=2
```

## Troubleshooting

### High CPU Usage

Too many files being watched:

```toml
# pyproject.toml
[tool.rpytest]
watch_ignore = [
    "node_modules",
    ".venv",
    "*.log",
]
```

### Tests Not Re-running

Check if file changes are detected:

```bash
rpytest --watch -v
```

Should show:

```
Detected 1 file change(s):
  src/auth.py (modified)
```

If not, check:

1. File is in watched directory
2. File not in ignore patterns
3. File system events working (try `inotifywait` on Linux)

### Wrong Tests Running

Dependency tracking may miss indirect imports:

```bash
# Force full re-run on source changes
rpytest --watch --watch-all-sources
```

### Memory Growth

Long watch sessions may accumulate memory:

```bash
# Restart periodically
rpytest --watch --restart-interval=3600
```
