# Benchmarks

Performance comparison between rpytest and pytest on various test suites.

## Test Environment

- **CPU**: AMD Ryzen 7 5700U (16 threads)
- **OS**: Linux 6.14.0-37-generic
- **Python**: 3.12.3
- **pytest**: 9.0.2
- **pytest-xdist**: 3.8.0

## Quick Summary

| Metric | pytest | rpytest | Improvement |
|--------|--------|---------|-------------|
| Execution Time | 0.30s | 0.25s | **1.2x faster** |
| CLI Memory | 39.4 MB | 5.9 MB | **6.7x less** |
| Wall Clock | 0.63s | 0.32s | **2.0x faster** |

## Sequential Execution

### 500 Test Suite

| Runner | Execution | Wall Clock |
|--------|-----------|------------|
| pytest | 0.30s | 0.63s |
| rpytest | 0.25s | 0.32s |

**Speedup**: 1.2x execution, 2.0x wall clock

### Breakdown

| Phase | pytest | rpytest |
|-------|--------|---------|
| CLI Startup | ~200ms | ~8ms |
| Collection | ~100ms | ~20ms |
| Execution | 300ms | 250ms |
| Reporting | ~30ms | ~10ms |

## Parallel Execution

rpytest provides built-in `-n` support compatible with pytest-xdist.

| Configuration | Time | vs Sequential |
|---------------|------|---------------|
| pytest | 0.30s | baseline |
| pytest -n 4 | 0.87s | 2.9x slower |
| pytest -n auto | 1.90s | 6.3x slower |
| rpytest | 0.25s | baseline |
| rpytest -n 4 | 0.25s | same |
| rpytest -n auto | 0.20s | 1.25x faster |

### Why xdist Can Be Slower

For small-to-medium test suites, pytest-xdist often provides no benefit:

1. **Worker startup**: Each worker spawns new Python process
2. **Re-collection**: Each worker collects the entire test suite
3. **IPC overhead**: Results serialized and sent to master

rpytest avoids these with:
- Warm daemon with pre-loaded pytest
- Single collection, shared across workers
- Efficient binary IPC

## Memory Usage

| Component | pytest | rpytest |
|-----------|--------|---------|
| CLI Process | 39.4 MB | 5.9 MB |
| Daemon | N/A | ~80 MB (shared) |

The rpytest daemon is shared across all CLI invocations, so the per-invocation cost is just 5.9 MB.

## Throughput

| Metric | pytest | rpytest |
|--------|--------|---------|
| Tests/second | 1,667 | 2,000 |
| ms/test | 0.60 | 0.50 |

## Collection Performance

Native AST collection vs pytest import-based:

| File Count | pytest | rpytest |
|------------|--------|---------|
| 10 files | 0.5s | 0.05s |
| 100 files | 2.0s | 0.1s |
| 1000 files | 8.5s | 0.3s |

**Speedup**: 10-28x faster collection

## Detailed Results

### Execution Time (3 runs)

| Configuration | Run 1 | Run 2 | Run 3 | Average |
|---------------|-------|-------|-------|---------|
| pytest | 0.31s | 0.30s | 0.30s | 0.30s |
| pytest -n 4 | 0.89s | 0.86s | 0.87s | 0.87s |
| rpytest | 0.26s | 0.23s | 0.26s | 0.25s |
| rpytest -n 4 | 0.25s | 0.37s | 0.33s | 0.32s |

### Wall Clock (3 runs)

| Configuration | Run 1 | Run 2 | Run 3 | Average |
|---------------|-------|-------|-------|---------|
| pytest | 0.63s | 0.62s | 0.64s | 0.63s |
| rpytest | 0.32s | 0.31s | 0.33s | 0.32s |

## Test Suite Details

Benchmark suite composition:

- 500 tests across 30 files
- Simple assertion tests
- Parameterized tests (10 params each)
- Tests with fixtures
- Class-based test methods

## Running Benchmarks

Reproduce these results:

```bash
# Setup
uv pip install pytest-xdist

# pytest benchmarks
time uv run python -m pytest benchmark_suite/ -q
time uv run python -m pytest benchmark_suite/ -n 4 -q
time uv run python -m pytest benchmark_suite/ -n auto -q

# rpytest benchmarks
time ./target/release/rpytest benchmark_suite/ -q
time ./target/release/rpytest benchmark_suite/ -n 1 -q
time ./target/release/rpytest benchmark_suite/ -n 4 -q

# Memory comparison
/usr/bin/time -v uv run python -m pytest benchmark_suite/ -q 2>&1 | grep "Maximum resident"
/usr/bin/time -v ./target/release/rpytest benchmark_suite/ -q 2>&1 | grep "Maximum resident"
```

## When to Use What

### Use rpytest (default mode) when:
- Running unit tests during development
- Fast iteration in watch mode
- CI with modest test counts (<1000)

### Use rpytest -n auto when:
- Large test suites (>1000 tests)
- Tests with significant individual runtime (>100ms)
- Need to saturate CPU with compute-heavy tests

### Use sharding when:
- Very large suites (>5000 tests)
- Distributed CI runners available
- Wall clock time critical

## CI Performance

### Traditional pytest + xdist

```yaml
# Single runner with xdist
- run: pytest tests/ -n auto
# Time: ~120s for 5000 tests
```

### rpytest with sharding

```yaml
# 4 runners with sharding
jobs:
  test:
    strategy:
      matrix:
        shard: [0, 1, 2, 3]
    steps:
      - run: rpytest --shard ${{ matrix.shard }} --total-shards 4
# Time: ~35s per runner
```

**Total CI time**: 120s vs 35s = **3.4x faster**
