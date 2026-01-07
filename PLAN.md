# rpytest Code Cleanup and Bug Fix Plan

## Overview
Address the immediate issues identified in the codebase assessment to improve code quality and fix functional bugs.

---

## Issue 1: Clean Up Dead Code and Unused Imports (~100 warnings)

### Files with Issues:
- `crates/rpytest-ipc/src/transport.rs:49` - Unused `address` field in `DaemonClient`
- `crates/rpytest-daemon/src/collector.rs` - Unused imports (`DaemonError`, `ExprCall`)
- `crates/rpytest-daemon/src/context.rs` - Multiple unused imports
- `crates/rpytest-daemon/src/executor.rs` - Multiple unused imports
- `crates/rpytest-daemon/src/fixtures.rs` - Unused imports
- `crates/rpytest-daemon/src/storage.rs` - Unused imports
- `crates/rpytest/src/watch/watcher.rs:210` - Unused `filter_source_files` function

### Plan:
1. Remove unused imports in each file
2. Either remove or use the `address` field in `DaemonClient` (currently unused)
3. Either remove or use `filter_source_files` function in watcher
4. Run `cargo fix --lib -p rpytest-daemon` to auto-apply safe fixes
5. Manually address remaining warnings that require judgment

---

## Issue 2: Fix Path Filtering Bug

### Symptom:
Running `./target/debug/rpytest run test_simple.py` returns "No tests found matching the criteria" even though tests exist.

### Root Cause:
The `filter_by_paths` function in `crates/rpytest/src/main.rs:437-472` has flawed matching logic:
- `node_id.starts_with(path)` doesn't handle partial path matches correctly
- File path matching `file_part == path` is too strict (doesn't handle relative paths like `./test_simple.py`)
- The `ends_with(&format!("/{}", path))` logic is incorrect for filenames

### Plan:
1. Add comprehensive test cases for `filter_by_paths` function
2. Improve matching logic to handle:
   - Exact node IDs: `test_file.py::TestClass::test_method`
   - Class-level: `test_file.py::TestClass` (matches all methods)
   - File-level: `test_file.py` or `./test_file.py`
   - Directory-level: `tests/` or `tests`
   - Relative paths with `./` prefix
3. Normalize paths before comparison (remove `./` prefix, handle trailing slashes)
4. Add integration test to verify path filtering works

---

## Issue 3: Improve Daemon PID File Handling

### Symptom:
`--daemon-status` shows "No PID file found" even when daemon is running. The Rust daemon doesn't write a PID file.

### Root Cause:
- `rpytest-daemon/src/main.rs` doesn't write a PID file
- `LifecycleManager` expects a PID file at `/tmp/rpytest.pid`
- When daemon is spawned via `DaemonManager::spawn_daemon()` in `crates/rpytest/src/daemon/client.rs:139-189`, it doesn't write PID file
- There's a mismatch: `LifecycleManager.spawn_daemon()` writes PID, but `DaemonManager.spawn_daemon()` doesn't

### Plan:
**Option A: Have daemon write its own PID file**
1. Add `--pid-file` argument to daemon CLI in `main.rs`
2. Have daemon write PID file at startup
3. Use same location as CLI expects (`/tmp/rpytest.pid` via `XDG_RUNTIME_DIR`)
4. Clean up PID file on daemon shutdown

**Option B: Have CLI write PID file (current approach)**
1. Have `DaemonManager.spawn_daemon()` write PID file after spawning
2. Use same PID file path as `LifecycleManager` expects
3. Clean up PID file on disconnect

### Recommended: Option A (daemon owns PID file)
- More robust - PID file exists as long as daemon runs
- Allows daemon to self-cleanup on crash
- Consistent with how system daemons typically work

### Implementation Steps for Option A:
1. Add `--pid-file` argument to daemon `Args` struct
2. Create `write_pid_file()` helper function in daemon
3. Write PID file early in `main()`, after successful server creation
4. Register shutdown hook to remove PID file
5. Update CLI to use same PID file path

---

## Issue 4: Fix Storage Locking Issue

### Symptom:
When daemon is already running and CLI tries to start another, storage fallback triggers:
```
Failed to initialize storage at /Users/.../Library/Caches/rpytest: IO error: could not acquire lock
Falling back to temp dir.
```

### Root Cause:
Both daemon instances try to use same sled storage directory without coordination.

### Plan:
1. Add file locking at storage initialization to fail fast if locked
2. Improve error message to indicate daemon is already running
3. Consider using separate storage per context to avoid contention

---

## Summary of Files to Modify

| File | Changes |
|------|---------|
| `crates/rpytest-ipc/src/transport.rs` | Remove unused `address` field |
| `crates/rpytest-daemon/src/collector.rs` | Remove unused imports |
| `crates/rpytest-daemon/src/context.rs` | Remove unused imports |
| `crates/rpytest-daemon/src/executor.rs` | Remove unused imports |
| `crates/rpytest-daemon/src/fixtures.rs` | Remove unused imports |
| `crates/rpytest-daemon/src/storage.rs` | Remove unused imports |
| `crates/rpytest/src/watch/watcher.rs` | Remove or use `filter_source_files` |
| `crates/rpytest/src/main.rs` | Fix `filter_by_paths` function, add tests |
| `crates/rpytest-daemon/src/main.rs` | Add PID file writing |
| `crates/rpytest-daemon/src/lib.rs` | Add PID file helper functions |
| `crates/rpytest/src/daemon/lifecycle.rs` | Ensure PID file path consistency |

---

## Testing Plan

1. **Unit Tests**:
   - Add tests for `filter_by_paths` with various path patterns
   - Test PID file creation and cleanup

2. **Integration Tests**:
   - Test path filtering with `rpytest run <path>`
   - Test daemon lifecycle (start, status, stop)
   - Test concurrent daemon detection

3. **Manual Testing**:
   - Run `--daemon-status` after starting daemon
   - Run specific test file filtering
   - Verify no storage lock warnings
