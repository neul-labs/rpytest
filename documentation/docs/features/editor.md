# Editor Integration

rpytest provides an editor server for IDE integration, enabling features like test discovery, inline results, and run-at-cursor.

## Architecture

```
┌─────────────┐   JSON-RPC    ┌─────────────────┐
│   Editor    │◄─────────────►│  Editor Server  │
│ (VS Code,   │   (stdio)     │  (rpytest)      │
│  Neovim)    │               └────────┬────────┘
└─────────────┘                        │ IPC
                                       ▼
                              ┌─────────────────┐
                              │  Python Daemon  │
                              └─────────────────┘
```

## Starting the Editor Server

```bash
rpytest --editor-server
```

This starts a JSON-RPC server over stdio for editor communication.

## Protocol

The editor server uses a subset of the Language Server Protocol (LSP) with custom methods.

### Initialize

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "rootPath": "/path/to/project"
  }
}
```

Response:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "version": "0.1.0",
    "testCount": 500
  }
}
```

### List Tests in File

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "listTestsInFile",
  "params": {
    "filePath": "tests/test_auth.py"
  }
}
```

Response:

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "filePath": "tests/test_auth.py",
    "tests": [
      {
        "nodeId": "tests/test_auth.py::test_login",
        "name": "test_login",
        "line": 10,
        "className": null
      },
      {
        "nodeId": "tests/test_auth.py::TestAuth::test_logout",
        "name": "test_logout",
        "line": 25,
        "className": "TestAuth"
      }
    ]
  }
}
```

### Get Nearest Test

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "getNearestTest",
  "params": {
    "filePath": "tests/test_auth.py",
    "line": 15
  }
}
```

Response:

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "test": {
      "nodeId": "tests/test_auth.py::test_login",
      "name": "test_login",
      "line": 10,
      "className": null
    }
  }
}
```

### Run Test

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "runTest",
  "params": {
    "nodeId": "tests/test_auth.py::test_login"
  }
}
```

Response:

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": {
    "nodeIds": ["tests/test_auth.py::test_login"]
  }
}
```

### Run Nearest Test

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "runNearestTest",
  "params": {
    "filePath": "tests/test_auth.py",
    "line": 15
  }
}
```

### Run Tests in File

```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "runTestsInFile",
  "params": {
    "filePath": "tests/test_auth.py"
  }
}
```

### Get Test Status

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "getTestStatus",
  "params": {
    "nodeId": "tests/test_auth.py::test_login"
  }
}
```

Response:

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "result": {
    "nodeId": "tests/test_auth.py::test_login",
    "status": "passed",
    "lastResult": {
      "outcome": "passed",
      "duration_ms": 50,
      "message": null
    }
  }
}
```

### Shutdown

```json
{
  "jsonrpc": "2.0",
  "id": 8,
  "method": "shutdown"
}
```

## VS Code Extension

### Installation

Install from VS Code Marketplace:

1. Open Extensions (`Ctrl+Shift+X`)
2. Search for "rpytest"
3. Click Install

### Configuration

```json
// .vscode/settings.json
{
  "rpytest.enable": true,
  "rpytest.autoRun": true,
  "rpytest.showInlineResults": true,
  "rpytest.runOnSave": false
}
```

### Features

- **Test Explorer**: View all tests in sidebar
- **CodeLens**: Run/debug buttons above each test
- **Inline Results**: Pass/fail indicators in gutter
- **Run at Cursor**: `Ctrl+Shift+T` to run nearest test
- **Output Panel**: Test output and errors

### Keybindings

| Key | Action |
|-----|--------|
| `Ctrl+Shift+T` | Run test at cursor |
| `Ctrl+Shift+A` | Run all tests in file |
| `Ctrl+Shift+R` | Re-run last test |
| `Ctrl+Shift+F` | Run failed tests |

## Neovim Integration

### Using neotest

```lua
-- init.lua
require("neotest").setup({
  adapters = {
    require("neotest-rpytest"),
  },
})
```

### Configuration

```lua
require("neotest-rpytest").setup({
  rpytest_path = "rpytest",
  args = { "-v" },
})
```

### Keymaps

```lua
-- Run nearest test
vim.keymap.set("n", "<leader>tt", function()
  require("neotest").run.run()
end)

-- Run file
vim.keymap.set("n", "<leader>tf", function()
  require("neotest").run.run(vim.fn.expand("%"))
end)

-- Run all
vim.keymap.set("n", "<leader>ta", function()
  require("neotest").run.run(vim.fn.getcwd())
end)

-- Show output
vim.keymap.set("n", "<leader>to", function()
  require("neotest").output.open()
end)
```

## PyCharm Integration

### External Tool Setup

1. Go to Settings → Tools → External Tools
2. Click "+" to add new tool
3. Configure:
   - Name: rpytest
   - Program: `rpytest`
   - Arguments: `$FilePath$::$SelectedText$ -v`
   - Working directory: `$ProjectFileDir$`

### Run Configuration

1. Go to Run → Edit Configurations
2. Add new configuration
3. Select "Shell Script"
4. Configure:
   - Script: `rpytest tests/ -v`
   - Working directory: Project root

## Custom Integration

### Basic Example

```python
import subprocess
import json

def run_test(node_id: str) -> dict:
    """Run a single test and return result."""
    result = subprocess.run(
        ["rpytest", node_id, "--json"],
        capture_output=True,
        text=True
    )
    return json.loads(result.stdout)

def list_tests(file_path: str) -> list:
    """List all tests in a file."""
    result = subprocess.run(
        ["rpytest", file_path, "--collect-only", "--json"],
        capture_output=True,
        text=True
    )
    data = json.loads(result.stdout)
    return data.get("tests", [])
```

### Using Editor Server

```python
import subprocess
import json

# Start server
proc = subprocess.Popen(
    ["rpytest", "--editor-server"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    text=True
)

def send_request(method: str, params: dict) -> dict:
    request = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    }
    content = json.dumps(request)
    message = f"Content-Length: {len(content)}\r\n\r\n{content}"
    proc.stdin.write(message)
    proc.stdin.flush()

    # Read response
    header = proc.stdout.readline()
    length = int(header.split(": ")[1])
    proc.stdout.readline()  # Empty line
    response = proc.stdout.read(length)
    return json.loads(response)

# Initialize
result = send_request("initialize", {"rootPath": "/path/to/project"})
print(f"Found {result['testCount']} tests")

# List tests
tests = send_request("listTestsInFile", {"filePath": "tests/test_auth.py"})
for test in tests["tests"]:
    print(f"  {test['name']} at line {test['line']}")
```
