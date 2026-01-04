# Sharding

Sharding distributes tests across multiple CI runners for faster parallel execution.

## Basic Usage

```bash
# On runner 1
rpytest tests/ --shard=0 --total-shards=4

# On runner 2
rpytest tests/ --shard=1 --total-shards=4

# On runner 3
rpytest tests/ --shard=2 --total-shards=4

# On runner 4
rpytest tests/ --shard=3 --total-shards=4
```

## Sharding Strategies

### Duration Balanced (Default)

Distributes tests to balance total execution time per shard:

```bash
rpytest --shard=0 --total-shards=4 --shard-strategy=duration_balanced
```

Uses historical test durations to ensure each shard takes roughly the same time.

**Best for**: Uneven test durations, maximizing parallelism efficiency.

### Hash-Based

Deterministic distribution based on test name hash:

```bash
rpytest --shard=0 --total-shards=4 --shard-strategy=hash
```

Same test always goes to same shard, regardless of other tests.

**Best for**: Consistent sharding across runs, caching scenarios.

### Round Robin

Sequential distribution across shards:

```bash
rpytest --shard=0 --total-shards=4 --shard-strategy=round_robin
```

Tests distributed in order: 0→1→2→3→0→1→2→3...

**Best for**: Simple distribution when durations are similar.

## Strategy Comparison

| Strategy | Pros | Cons |
|----------|------|------|
| `duration_balanced` | Optimal runtime balance | Requires duration history |
| `hash` | Consistent, cacheable | May be imbalanced |
| `round_robin` | Simple, predictable | Ignores durations |

## CI/CD Integration

### GitHub Actions

```yaml
name: Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        shard: [0, 1, 2, 3]
      fail-fast: false

    steps:
      - uses: actions/checkout@v4

      - name: Set up Python
        uses: actions/setup-python@v5
        with:
          python-version: '3.11'

      - name: Install dependencies
        run: pip install -e . rpytest-daemon

      - name: Run tests
        run: |
          rpytest tests/ \
            --shard=${{ matrix.shard }} \
            --total-shards=4 \
            --shard-strategy=duration_balanced \
            --junitxml=report-${{ matrix.shard }}.xml

      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: test-results-${{ matrix.shard }}
          path: report-${{ matrix.shard }}.xml
```

### GitLab CI

```yaml
test:
  stage: test
  parallel: 4
  script:
    - rpytest tests/
        --shard=$((CI_NODE_INDEX - 1))
        --total-shards=$CI_NODE_TOTAL
        --shard-strategy=duration_balanced
```

### CircleCI

```yaml
jobs:
  test:
    parallelism: 4
    steps:
      - run:
          command: |
            rpytest tests/ \
              --shard=$CIRCLE_NODE_INDEX \
              --total-shards=$CIRCLE_NODE_TOTAL \
              --shard-strategy=duration_balanced
```

### Jenkins

```groovy
pipeline {
    agent any
    stages {
        stage('Test') {
            matrix {
                axes {
                    axis {
                        name 'SHARD'
                        values '0', '1', '2', '3'
                    }
                }
                stages {
                    stage('Run Shard') {
                        steps {
                            sh """
                                rpytest tests/ \
                                    --shard=${SHARD} \
                                    --total-shards=4 \
                                    --shard-strategy=duration_balanced
                            """
                        }
                    }
                }
            }
        }
    }
}
```

## Shard Info

Get information about shard distribution:

```bash
rpytest tests/ --total-shards=4 --shard-info
```

Output:

```
Sharding Strategy: duration_balanced
Total Shards: 4
Total Tests: 500

Shard Distribution:
  Shard 0: 125 tests, estimated 30.5s
  Shard 1: 124 tests, estimated 30.2s
  Shard 2: 126 tests, estimated 30.8s
  Shard 3: 125 tests, estimated 30.1s

Balance Metrics:
  Count Imbalance: 1.6%
  Duration Imbalance: 2.3%
  Estimated Wall Time: 30.8s
```

## Duration History

Duration-balanced sharding uses historical test durations:

### Building History

First run uses round-robin (no history):

```bash
rpytest tests/ --shard=0 --total-shards=4
# Durations recorded
```

Subsequent runs use duration data:

```bash
rpytest tests/ --shard=0 --total-shards=4
# Better balanced
```

### Storage Location

Duration data stored in:

```
.rpytest/
└── durations/
    └── context-xxx/
        └── test_durations.json
```

### Sharing Across Runners

For consistent sharding across CI runners, share duration data:

```yaml
# GitHub Actions example
- name: Download duration cache
  uses: actions/cache@v4
  with:
    path: .rpytest/durations
    key: rpytest-durations-${{ github.ref }}
    restore-keys: |
      rpytest-durations-main
      rpytest-durations-
```

## Combining with Parallel Execution

Use sharding between runners and parallelism within:

```bash
# Each runner
rpytest tests/ \
    --shard=$SHARD \
    --total-shards=4 \
    -n auto  # Parallel within shard
```

## Dynamic Shard Count

Adjust shards based on test count:

```bash
#!/bin/bash
TEST_COUNT=$(rpytest --collect-only -q | tail -1 | cut -d' ' -f1)

if [ "$TEST_COUNT" -gt 1000 ]; then
    SHARDS=8
elif [ "$TEST_COUNT" -gt 500 ]; then
    SHARDS=4
else
    SHARDS=2
fi

rpytest tests/ --shard=$MY_SHARD --total-shards=$SHARDS
```

## Troubleshooting

### Imbalanced Shards

If one shard takes much longer:

1. Check for new tests without duration history
2. Use `--shard-info` to analyze distribution
3. Consider different strategy:

```bash
# Try hash-based for consistency
rpytest --shard-strategy=hash
```

### Missing Tests

If some tests don't run on any shard:

```bash
# Verify all tests are collected
rpytest --collect-only --total-shards=4 --shard-all
```

### Duplicate Tests

If tests run on multiple shards:

```bash
# Hash strategy guarantees no duplicates
rpytest --shard-strategy=hash
```
