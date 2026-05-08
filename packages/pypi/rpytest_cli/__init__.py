"""rpytest - Rust-powered, drop-in replacement for pytest.

This package provides the rpytest CLI tool and pytest plugin.
"""

__version__ = "0.1.2"

import os
import platform
import subprocess
import sys
from pathlib import Path


def get_binary_path() -> Path:
    """Get the path to the rpytest binary."""
    # Check if we're in development mode (source checkout)
    pkg_dir = Path(__file__).parent

    # Try package-bundled binary first
    bin_dir = pkg_dir / "bin"
    if bin_dir.exists():
        system = platform.system().lower()
        machine = platform.machine().lower()

        # Normalize architecture names
        if machine in ("x86_64", "amd64"):
            machine = "x86_64"
        elif machine in ("arm64", "aarch64"):
            machine = "aarch64"

        if system == "darwin":
            binary_name = f"rpytest-{machine}-apple-darwin"
        elif system == "linux":
            binary_name = f"rpytest-{machine}-unknown-linux-gnu"
        else:
            binary_name = "rpytest"

        binary_path = bin_dir / binary_name
        if binary_path.exists():
            return binary_path

    # Try system PATH, skipping any Python wrapper scripts to avoid
    # infinite fork loops when the pip package entry point shadows
    # the Rust binary.
    this_script = Path(sys.argv[0]).resolve() if sys.argv[0] else None
    path_dirs = os.get_exec_path()
    for directory in path_dirs:
        candidate = Path(directory) / "rpytest"
        if not candidate.is_file() or not os.access(candidate, os.X_OK):
            continue
        resolved = candidate.resolve()
        # Skip if it resolves to the same file as the current wrapper
        if this_script and resolved == this_script:
            continue
        # Skip Python scripts (they have a #! shebang pointing to python)
        try:
            with open(resolved, "rb") as f:
                header = f.read(64)
            if header.startswith(b"#!") and b"python" in header.split(b"\n")[0]:
                continue
        except OSError:
            continue
        return resolved

    # Try cargo target directory (development)
    workspace_root = pkg_dir.parent.parent
    for build_type in ("release", "debug"):
        target_binary = workspace_root / "target" / build_type / "rpytest"
        if target_binary.exists():
            return target_binary

    raise RuntimeError(
        "rpytest binary not found. Please install via:\n"
        "  cargo install --path crates/rpytest\n"
        "or download prebuilt binaries from releases."
    )


def main():
    """Main entry point for rpytest CLI."""
    try:
        binary_path = get_binary_path()
    except RuntimeError as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)

    # Make sure the binary is executable
    if not os.access(binary_path, os.X_OK):
        os.chmod(binary_path, 0o755)

    # Execute the Rust binary with all arguments
    try:
        result = subprocess.run(
            [str(binary_path)] + sys.argv[1:],
            check=False
        )
        sys.exit(result.returncode)
    except KeyboardInterrupt:
        sys.exit(130)
    except Exception as e:
        print(f"Error executing rpytest: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
