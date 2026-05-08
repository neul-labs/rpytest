# rpytest (npm)

Rust-powered, drop-in replacement for pytest.

## Installation

```bash
npm install -g rpytest
```

The postinstall script automatically downloads the correct prebuilt binary for your platform (macOS or Linux, x64 or arm64).

## Usage

```bash
rpytest
rpytest -v
rpytest tests/test_foo.py
rpytest -n auto
rpytest --watch
```

## Requirements

- Node.js 14+
- macOS or Linux

## License

MIT
