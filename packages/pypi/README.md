# rpytest

> **Run your pytest suite faster. Change nothing.**
>
> A Rust-powered, drop-in replacement for pytest that eliminates startup overhead while keeping your tests, fixtures, and plugins untouched.

[![PyPI Version](https://img.shields.io/pypi/v/rpytest.svg)](https://pypi.org/project/rpytest/)
[![License](https://img.shields.io/pypi/l/rpytest.svg)](https://github.com/neul-labs/rpytest/blob/main/LICENSE)
[![Python Versions](https://img.shields.io/pypi/pyversions/rpytest.svg)](https://pypi.org/project/rpytest/)

**[Website](https://rpytest.neullabs.com)** · **[Documentation](https://docs.neullabs.com/rpytest)** · **[GitHub](https://github.com/neul-labs/rpytest)**

## Why rpytest?

```
pytest  ->  2.91s  (480 tests)
rpytest ->  1.55s  (same 480 tests)
        =  1.9x faster
```

rpytest uses a persistent Rust daemon to keep Python warm between runs. No more interpreter startup costs on every invocation.

## Installation

```bash
pip install rpytest
```

That's it. The correct native binary for your platform (macOS or Linux, Intel or Apple Silicon) is bundled automatically.

## Usage

rpytest mirrors pytest's CLI exactly. If you know pytest, you know rpytest.

```bash
# Run all tests
rpytest

# Run specific tests
rpytest tests/test_api.py::test_login

# Filter by keyword or marker
rpytest -k "auth" -m "not slow"

# Parallel execution — no pytest-xdist needed
rpytest -n auto

# Watch mode for TDD
rpytest --watch
```

## Key Features

| Feature | pytest | rpytest |
|---------|--------|---------|
| Startup time | ~200ms | <10ms |
| Memory usage | 35.8 MB | 6.2 MB |
| Parallel workers | pytest-xdist plugin | Built-in `-n` flag |
| Watch mode | pytest-watch plugin | Built-in `--watch` |
| Flakiness detection | flaky plugin | Built-in `--reruns` |
| Sharding | pytest-shard plugin | Built-in `--shard` |

- **Full pytest compatibility** — plugins, fixtures, conftest.py, pytest.ini all work unchanged
- **Built-in parallelism** — `rpytest -n 4` or `rpytest -n auto`
- **Watch mode** — file changes trigger automatic re-runs of affected tests
- **Flakiness detection** — `rpytest --reruns 3` auto-retries failed tests
- **Session fixture reuse** — `rpytest --reuse-fixtures` persists expensive fixtures
- **CI sharding** — `rpytest --shard 0 --total-shards 4`

## Requirements

- Python 3.9+
- pytest 7.0+
- macOS or Linux

## How It Works

1. **First run**: Spawns a background daemon that collects your test suite
2. **Subsequent runs**: Rust CLI filters tests and dispatches to warm Python workers
3. **Results stream back** in real-time

The daemon persists between runs, so TDD loops and CI retries skip all startup work.

## Documentation

Full docs at [docs.neullabs.com/rpytest](https://docs.neullabs.com/rpytest)

## Part of the Neul Labs toolchain

Explore the rest of the Neul Labs developer tools:

| Project | Description |
| --- | --- |
| [rjest](https://github.com/neul-labs/rjest) | A blazing-fast, Jest-compatible test runner — 100x faster warm runs. |
| [rninja](https://github.com/neul-labs/rninja) | Drop-in Ninja replacement with built-in caching. |
| [gity](https://github.com/neul-labs/gity) | Make large Git repositories feel instant. |
| [stkd](https://github.com/neul-labs/stkd) | Stacked diffs for GitHub and GitLab. |
| [grite](https://github.com/neul-labs/grite) | The issue tracker that lives in your repo. Built for AI agents. |

Learn more at [neullabs.com](https://www.neullabs.com).

## License

MIT
