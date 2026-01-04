# Performance

rpytest is designed for speed at every layer. This document explains the performance optimizations and how to get the best results.

## Performance Gains

### Where Time is Saved

| Phase | pytest | rpytest | Savings |
|-------|--------|---------|---------|
| CLI startup | ~200ms | <10ms | 95% |
| Test collection | ~500ms | ~50ms | 90% |
| Per-test overhead | ~10ms | ~2ms | 80% |
| Result aggregation | ~50ms | ~5ms | 90% |

### Real-World Impact

For a 500-test suite:

```
pytest:   500ms startup + 500ms collect + 500*10ms overhead = 6.0s overhead
rpytest:  10ms startup + 50ms collect + 500*2ms overhead = 1.06s overhead

Savings: ~5 seconds (83% less overhead)
```

## Optimization Techniques

### 1. Rust CLI

The command-line interface is written in Rust for instant startup:

```
$ time rpytest --help
real    0m0.008s  # 8 milliseconds

$ time pytest --help
real    0m0.234s  # 234 milliseconds
```

**Why it matters**: Every invocation pays startup cost. In watch mode with frequent re-runs, this adds up.

### 2. Native Collection

AST-based test discovery without imports:

```python
# pytest: imports every file to collect
import tests.test_api     # executes module-level code
import tests.test_db      # loads all dependencies
import tests.test_utils   # slow!

# rpytest: parses AST without execution
ast.parse(open("tests/test_api.py").read())  # fast!
```

**Benchmark**:
```
1000 test files:
  pytest collection: 8.5s
  rpytest collection: 0.3s (28x faster)
```

### 3. Daemon Model

Persistent daemon avoids repeated Python startup:

```
Cold start (no daemon):
  rpytest tests/ → start daemon (100ms) → run tests

Warm start (daemon running):
  rpytest tests/ → connect (<1ms) → run tests
```

### 4. Worker Pool

Pre-spawned worker processes ready to execute:

```
First run:  spawn workers (50ms each)
Next runs:  workers already running (0ms spawn)
```

### 5. Parallel Execution

LPT scheduling for optimal load balancing:

```
Sequential: ████████████████████████ 24s
Parallel-4: ██████                    6s (4x speedup)
```

### 6. Efficient IPC

Binary MessagePack instead of JSON:

```
JSON:      {"node_id": "tests/test.py::test_func", "outcome": "passed"}
MessagePack: \x82\xa7node_id\xbetests/test.py::test_func\xa7outcome\xa6passed

Size: 30% smaller
Parse: 5x faster
```

## Configuration for Speed

### Optimal Settings

```toml
# pyproject.toml
[tool.rpytest]
# Use all CPU cores
parallel = "auto"

# Keep daemon running
daemon_idle_timeout = 600

# Reuse session fixtures
enable_fixture_reuse = true

# Skip slow tests in development
default_markers = "not slow"
```

### Watch Mode

For fastest feedback during development:

```bash
# Fast tests only
rpytest --watch -m "not slow"

# Single file focus
rpytest tests/test_current.py --watch
```

### CI Optimization

```yaml
# Parallel jobs with duration-balanced sharding
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
            --shard-strategy=duration_balanced \
            -n auto
```

## Profiling

### Measure Collection Time

```bash
$ rpytest --collect-only --benchmark
Collection: 0.05s (native)
Tests found: 500
```

### Measure Execution Breakdown

```bash
$ rpytest tests/ --timing
Setup:      0.02s
Collection: 0.05s
Execution:  2.30s
Teardown:   0.01s
Reporting:  0.01s
Total:      2.39s
```

### Compare with pytest

```bash
$ ./benchmark.sh
Running pytest...
pytest: 3.42s

Running rpytest...
rpytest: 1.85s

Speedup: 1.85x
```

## Memory Efficiency

### CLI Memory

```
pytest:   ~40MB (loads Python runtime + plugins)
rpytest:  ~5MB (Rust binary only)
```

### Daemon Memory

```
Baseline:    ~30MB
Per worker:  ~20MB
With fixtures: varies
```

### Optimization Tips

```bash
# Limit workers to reduce memory
rpytest -n 2

# Reduce fixture max age
rpytest --fixture-max-age=300

# Restart daemon periodically
rpytest --daemon-stop && rpytest tests/
```

## Bottleneck Analysis

### Slow Collection?

Check for:
- Module-level imports in test files
- Complex conftest.py fixtures
- Dynamic test generation

Solution:
```bash
# Force native collector
rpytest --native-collect-only
```

### Slow Tests?

Identify slowest tests:
```bash
rpytest --durations=10
```

Output:
```
Slowest 10 tests:
  2.50s tests/test_db.py::test_complex_query
  1.20s tests/test_api.py::test_file_upload
  0.80s tests/test_auth.py::test_oauth_flow
```

### Slow Fixtures?

Check fixture setup time:
```bash
rpytest --setup-show
```

### IPC Latency?

Debug communication:
```bash
RUST_LOG=debug rpytest tests/ 2>&1 | grep -i ipc
```

## Scaling

### Small Suite (<100 tests)

```bash
# Sequential is fine
rpytest tests/
```

### Medium Suite (100-1000 tests)

```bash
# Parallel execution
rpytest tests/ -n auto
```

### Large Suite (1000+ tests)

```bash
# Sharding across CI runners
rpytest tests/ --shard=0 --total-shards=8 -n auto
```

### Huge Suite (10000+ tests)

```bash
# Maximum parallelism + duration balancing
rpytest tests/ \
  --shard=$SHARD --total-shards=16 \
  --shard-strategy=duration_balanced \
  -n auto \
  --reuse-fixtures
```

## Comparison Table

| Metric | pytest | rpytest | Improvement |
|--------|--------|---------|-------------|
| CLI startup | 200ms | 8ms | 25x |
| Collection (1k files) | 8.5s | 0.3s | 28x |
| Per-test overhead | 10ms | 2ms | 5x |
| Memory (CLI) | 40MB | 5MB | 8x |
| Watch mode latency | 500ms | 50ms | 10x |
| Parallel efficiency | 70% | 90% | 1.3x |

## Future Optimizations

Planned improvements:

1. **Incremental collection**: Only re-parse changed files
2. **Result caching**: Skip unchanged tests
3. **Distributed execution**: Run across multiple machines
4. **WASM workers**: Faster isolation than subprocess
