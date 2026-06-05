# Installation

## Requirements

- **Python**: 3.9 or higher
- **Operating System**: Linux, macOS (Windows support coming soon)
- **Rust**: 1.75+ (only for building from source)

## Install via pip (recommended)

The PyPI package bundles a prebuilt platform binary, so it's the
zero-config option:

```bash
pip install rpytest
```

Or with [uv](https://github.com/astral-sh/uv):

```bash
uv pip install rpytest
```

## Install via Homebrew

For macOS and Linux:

```bash
brew tap neul-labs/tap
brew install rpytest
```

## Install via npm

For workflows that already use npm/pnpm/yarn:

```bash
npm install -g rpytest
```

## Install via cargo

`cargo install` installs the CLI **and** the daemon - both are required:

```bash
cargo install rpytest rpytest-daemon
```

!!! warning "Both binaries required"
    rpytest delegates execution to a separate `rpytest-daemon` binary.
    When installing via `cargo install`, you must install both packages.
    `pip`, `brew`, and `npm` distributions bundle everything together.

## Install from source

Clone and build:

```bash
git clone https://github.com/neul-labs/rpytest.git
cd rpytest
cargo build --release

# Add to PATH
export PATH="$PWD/target/release:$PATH"
```

Or install both crates from the workspace:

```bash
cargo install --path crates/rpytest
cargo install --path crates/rpytest-daemon
```

## Verify Installation

```bash
# Check version
rpytest --version

# Run a simple test
echo 'def test_example(): assert True' > test_example.py
rpytest test_example.py
```

## Virtual Environment Detection

rpytest automatically detects your Python environment in this order:

1. `VIRTUAL_ENV` environment variable
2. `PYTHON` environment variable
3. `.venv/` directory in current or parent directories
4. System `python3` or `python`

!!! tip "Using with uv"
    rpytest works seamlessly with [uv](https://github.com/astral-sh/uv):

    ```bash
    uv venv
    uv pip install rpytest pytest
    rpytest tests/
    ```

## IDE Integration

### VS Code

Add to your `.vscode/settings.json`:

```json
{
  "python.testing.pytestEnabled": false,
  "python.testing.unittestEnabled": false,
  "rpytest.enable": true
}
```

### PyCharm

rpytest can be configured as an external tool:

1. Go to Settings → Tools → External Tools
2. Add new tool with:
   - Program: `rpytest`
   - Arguments: `$FilePath$`
   - Working directory: `$ProjectFileDir$`

## Troubleshooting

### Daemon Connection Failed

If you see "Failed to connect to daemon":

```bash
# Check if daemon is running
rpytest --daemon-status

# Stop any stale daemon
rpytest --daemon-stop

# Clean up stale files
rpytest --cleanup
```

### Python Not Found

If rpytest can't find Python:

```bash
# Set explicitly
export PYTHON=/path/to/python3

# Or use virtual environment
source .venv/bin/activate
rpytest tests/
```

### Permission Denied on Socket

```bash
# Check XDG_RUNTIME_DIR
echo $XDG_RUNTIME_DIR

# If not set, rpytest uses /tmp
ls -la /tmp/rpytest.sock
```
