# rpytest Benchmark Results

Benchmarks comparing rpytest vs pytest performance on a test suite of 480 tests.

**Test Environment:**
- CPU: AMD Ryzen 9 (16 cores)
- OS: Linux 6.17.7
- Python: 3.12.3
- pytest: 9.0.2
- pytest-xdist: 3.5.0

## Summary

| Metric | pytest | rpytest | Improvement |
|--------|--------|---------|-------------|
| Execution Time | 0.51s | 0.48s | **~1.1x faster** |
| CLI Memory | 35.8 MB | 6.2 MB | **5.8x less** |
| Wall Clock (with startup) | 2.91s | 1.55s | **1.9x faster** |

## Parallel Execution Comparison

rpytest provides built-in `-n` support compatible with pytest-xdist, but without requiring the plugin.

| Runner | Time (480 tests) | Notes |
|--------|------------------|-------|
| pytest (sequential) | 0.51s | Baseline |
| pytest -n 4 (xdist) | 1.23s | Worker startup overhead |
| pytest -n auto (xdist) | 3.06s | Many workers, high overhead |
| rpytest (default) | 0.48s | Hybrid execution, warm workers |
| rpytest -n 1 | 1.49s | Sequential mode |
| rpytest -n 4 | 0.99s | **20% faster than xdist** |
| rpytest -n auto | 1.12s | Warm worker pool |

**Key insight:** For this test suite, rpytest's default hybrid execution (warm workers + direct execution) outperforms explicit parallel modes because the warm daemon eliminates startup overhead.

## Detailed Results

### Execution Time (3 runs each)

| Configuration | Run 1 | Run 2 | Run 3 | Average |
|---------------|-------|-------|-------|---------|
| pytest | 0.56s | 0.48s | 0.48s | 0.51s |
| pytest -n 4 | 1.32s | 1.18s | 1.19s | 1.23s |
| pytest -n auto | 2.79s | 2.95s | 3.44s | 3.06s |
| rpytest | 0.41s | 0.67s | 0.37s | 0.48s |
| rpytest -n 1 | 1.58s | 1.49s | 1.41s | 1.49s |
| rpytest -n 4 | 1.15s | 0.78s | 1.04s | 0.99s |
| rpytest -n auto | 1.12s | 1.18s | 1.06s | 1.12s |

### Memory Usage

| Component | pytest | rpytest |
|-----------|--------|---------|
| CLI process | 35.8 MB | 6.2 MB |
| Daemon (shared) | N/A | ~80 MB |

The rpytest daemon is a shared process that serves multiple CLI invocations.
The CLI itself is a lightweight Rust binary.

### Throughput

| Metric | pytest | rpytest |
|--------|--------|---------|
| Tests/second | 941 | 1,000 |
| ms/test | 1.06 | 1.00 |

## Why pytest-xdist Can Be Slower

For small-to-medium test suites (under ~1000 tests), pytest-xdist's parallel execution often provides no benefit or is even slower because:

1. **Worker startup overhead**: Each worker spawns a new Python process
2. **Collection per worker**: Each worker re-collects the test suite
3. **IPC overhead**: Results must be serialized and sent back to master

rpytest avoids these issues with:
- **Warm daemon**: Python and pytest are already loaded
- **Cached inventory**: Tests are only collected once
- **Direct execution**: Simple tests bypass pytest entirely

## When Parallel Helps

Use `-n` with rpytest when:
- Running very long test suites (>1000 tests)
- Tests have significant individual runtime (>100ms each)
- You need to saturate CPU cores with compute-heavy tests

For most unit test suites, rpytest's default mode is optimal.

## Architecture Optimizations

### 1. Daemon Architecture
- Persistent Python daemon eliminates interpreter startup overhead
- Warm pytest workers with pre-loaded test framework
- Context caching across CLI invocations

### 2. Hybrid Execution
Tests are classified as "simple" or "complex":
- **Simple tests** (~91%): Executed directly via function calls
- **Complex tests** (~9%): Run through pytest warm workers

### 3. Native AST Collection
- Pure Python AST parsing instead of pytest collection
- 6x faster than pytest's collection phase
- Cached to disk with mtime-based invalidation

### 4. Duration-Aware Scheduling
- LPT (Longest Processing Time) algorithm for load balancing
- Historical duration tracking for better predictions
- Optimal test ordering for parallel execution

## Benchmark Commands

Run these commands to reproduce the benchmarks:

```bash
# Setup
source .venv/bin/activate
pip install pytest-xdist

# pytest benchmarks
time python -m pytest benchmark_suite/ -q
time python -m pytest benchmark_suite/ -n 4 -q
time python -m pytest benchmark_suite/ -n auto -q

# rpytest benchmarks
time ./target/release/rpytest benchmark_suite/ -q
time ./target/release/rpytest benchmark_suite/ -n 1 -q
time ./target/release/rpytest benchmark_suite/ -n 4 -q
time ./target/release/rpytest benchmark_suite/ -n auto -q

# Memory comparison
/usr/bin/time -v python -m pytest benchmark_suite/ -q 2>&1 | grep "Maximum resident"
/usr/bin/time -v ./target/release/rpytest benchmark_suite/ -q 2>&1 | grep "Maximum resident"

# Drop-in compatibility verification
./target/release/rpytest --verify-dropin benchmark_suite/
```

## Test Suite Composition

The `benchmark_suite/` contains 480 tests across 10 test files:
- Simple assertion tests
- Parameterized tests (10 params each)
- Tests with fixtures
- Class-based test methods

## CI/CD Recommendations

For CI pipelines, use rpytest's native sharding instead of xdist for distributed testing:

```yaml
# GitHub Actions example
jobs:
  test:
    strategy:
      matrix:
        shard: [0, 1, 2, 3]
    steps:
      - run: rpytest --shard ${{ matrix.shard }} --total-shards 4 --shard-strategy duration_balanced
```

This approach:
- Avoids xdist worker startup overhead
- Uses duration-balanced sharding for even distribution
- Scales horizontally across CI runners
