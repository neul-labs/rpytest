# CLI Reference

Complete reference for all rpytest command-line options.

## Synopsis

```bash
rpytest [OPTIONS] [PATHS]... [-- PASSTHROUGH...]
```

## Test Selection

### Paths

```bash
rpytest                              # Run all tests
rpytest tests/                       # Run tests in directory
rpytest tests/test_auth.py           # Run specific file
rpytest tests/test_auth.py::test_login  # Run specific test
rpytest tests/test_auth.py::TestClass::test_method  # Run class method
```

### `-k`, `--keyword` EXPR

Select tests by keyword expression.

```bash
rpytest -k auth                      # Tests containing "auth"
rpytest -k "auth and login"          # Tests containing both
rpytest -k "auth or signup"          # Tests containing either
rpytest -k "not slow"                # Tests NOT containing "slow"
rpytest -k "auth and not integration"  # Combined
```

### `-m`, `--marker` EXPR

Select tests by marker expression.

```bash
rpytest -m slow                      # Tests marked @pytest.mark.slow
rpytest -m "not slow"                # Tests NOT marked slow
rpytest -m "smoke and not flaky"     # Combined markers
```

## Execution Control

### `-x`, `--exitfirst`

Exit on first failure.

```bash
rpytest -x
```

### `--maxfail` N

Exit after N failures.

```bash
rpytest --maxfail=3
```

### `-n`, `--workers` N

Number of parallel workers.

```bash
rpytest -n 4                         # 4 workers
rpytest -n auto                      # Auto-detect CPUs
rpytest -n 1                         # Sequential (single worker)
```

### `--lf`, `--last-failed`

Run only tests that failed in the last run.

```bash
rpytest --lf
```

### `--ff`, `--failed-first`

Run failed tests first, then the rest.

```bash
rpytest --ff
```

### `--nf`, `--new-first`

Run new tests first, then the rest.

```bash
rpytest --nf
```

## Output Control

### `-v`, `--verbose`

Increase verbosity. Can be repeated.

```bash
rpytest -v                           # Verbose
rpytest -vv                          # Very verbose
rpytest -vvv                         # Debug level
```

### `-q`, `--quiet`

Decrease verbosity. Can be repeated.

```bash
rpytest -q                           # Quiet
rpytest -qq                          # Very quiet
```

### `--tb` STYLE

Traceback print mode.

| Style | Description |
|-------|-------------|
| `auto` | Default, short for many failures |
| `long` | Full traceback |
| `short` | Shorter traceback |
| `line` | Single line per failure |
| `native` | Python standard traceback |
| `no` | No traceback |

```bash
rpytest --tb=short
```

### `--no-header`

Don't show the rpytest header.

```bash
rpytest --no-header
```

### `-l`, `--showlocals`

Show local variables in tracebacks.

```bash
rpytest -l
```

### `-r` CHARS

Show summary for specified test outcomes.

| Char | Meaning |
|------|---------|
| `f` | Failed |
| `E` | Error |
| `s` | Skipped |
| `x` | Xfailed |
| `X` | Xpassed |
| `p` | Passed |
| `a` | All except passed |

```bash
rpytest -r fEs                       # Failed, Error, skipped
```

### `--durations` N

Show N slowest test durations.

```bash
rpytest --durations=10               # Show 10 slowest
rpytest --durations=0                # Show all
```

### `--durations-min` SECS

Minimum duration to include in slowest list.

```bash
rpytest --durations=0 --durations-min=1.0  # Only tests > 1s
```

## Collection

### `--collect-only`, `--co`

Only collect tests, don't run them.

```bash
rpytest --collect-only
rpytest --co                         # Short alias
```

### `--ignore` PATH

Ignore a path during collection.

```bash
rpytest --ignore=tests/slow/
rpytest --ignore=tests/integration/ --ignore=tests/e2e/
```

### `--ignore-glob` PATTERN

Ignore paths matching glob pattern.

```bash
rpytest --ignore-glob="**/slow_*.py"
```

## Configuration

### `-c`, `--config-file` FILE

Load configuration from specific file.

```bash
rpytest -c custom_pytest.ini
```

### `--rootdir` DIR

Set root directory for test discovery.

```bash
rpytest --rootdir=/path/to/project
```

### `-o`, `--override-ini` OPTION=VALUE

Override ini configuration option.

```bash
rpytest -o "addopts=-v"
rpytest -o "testpaths=tests" -o "python_files=check_*.py"
```

## Output Formats

### `--json`

Output machine-readable JSON format.

```bash
rpytest --json > results.json
```

### `--junitxml` PATH

Create JUnit XML report.

```bash
rpytest --junitxml=report.xml
```

## Capture

### `--capture`, `-s` METHOD

Per-test capturing method.

| Method | Description |
|--------|-------------|
| `fd` | File descriptor level (default) |
| `sys` | sys.stdout/stderr level |
| `no` | No capturing |
| `tee-sys` | Capture and also output |

```bash
rpytest -s                           # No capture (same as --capture=no)
rpytest --capture=tee-sys            # Capture but also show
```

## Debugging

### `--pdb`

Start debugger on errors.

```bash
rpytest --pdb
```

### `--trace`

Start debugger at test start.

```bash
rpytest --trace
```

## Flakiness & Rerun

### `--reruns` N

Rerun failed tests up to N times.

```bash
rpytest --reruns=3
```

### `--reruns-delay` MS

Delay between reruns in milliseconds.

```bash
rpytest --reruns=3 --reruns-delay=1000
```

### `--only-rerun-flaky`

Only rerun tests known to be flaky.

```bash
rpytest --reruns=3 --only-rerun-flaky
```

### `--flaky-report`

Show flakiness report after run.

```bash
rpytest --flaky-report
```

## Sharding

### `--shard` INDEX

Run only tests in shard INDEX (0-indexed).

```bash
rpytest --shard=0 --total-shards=4
```

### `--total-shards` N

Total number of shards.

```bash
rpytest --shard=0 --total-shards=4
```

### `--shard-strategy` STRATEGY

Sharding strategy.

| Strategy | Description |
|----------|-------------|
| `hash` | Consistent hash-based |
| `round_robin` | Round-robin distribution |
| `duration_balanced` | Balance by test duration (default) |

```bash
rpytest --shard=0 --total-shards=4 --shard-strategy=duration_balanced
```

## Fixture Reuse

### `--reuse-fixtures`

Enable session fixture reuse between runs.

```bash
rpytest --reuse-fixtures
```

### `--fixture-max-age` SECS

Maximum age for reused fixtures (default: 600).

```bash
rpytest --reuse-fixtures --fixture-max-age=300
```

## Daemon Management

### `--daemon`

Run as daemon (foreground mode).

```bash
rpytest --daemon
```

### `--daemon-status`

Show daemon status.

```bash
rpytest --daemon-status
```

### `--daemon-stop`

Stop the running daemon.

```bash
rpytest --daemon-stop
```

### `--daemon-idle-timeout` SECS

Daemon idle timeout (default: 300).

```bash
rpytest --daemon --daemon-idle-timeout=600
```

### `--cleanup`

Clean up stale test contexts.

```bash
rpytest --cleanup
```

### `--cleanup-max-age` SECS

Maximum age for cleanup (default: 3600).

```bash
rpytest --cleanup --cleanup-max-age=7200
```

## rpytest Extensions

### `--watch`

Watch mode: re-run tests on file changes.

```bash
rpytest --watch
```

### `--verify-dropin`

Verify drop-in compatibility with pytest.

```bash
rpytest --verify-dropin
```

### `--inventory-status`

Show test inventory status.

```bash
rpytest --inventory-status
```

### `-V`, `--version`

Show version.

```bash
rpytest --version
```

## Passthrough Arguments

Arguments after `--` are passed to the daemon/plugins:

```bash
rpytest tests/ -- --cov=myapp --cov-report=html
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `VIRTUAL_ENV` | Virtual environment path |
| `PYTHON` | Python interpreter path |
| `XDG_RUNTIME_DIR` | Runtime directory for socket |
| `RPYTEST_LOG` | Log level (debug, info, warn) |
