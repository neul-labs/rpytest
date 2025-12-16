"""Download prebuilt rpytest binaries."""

import hashlib
import os
import platform
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

# GitHub release URL pattern
RELEASE_URL = "https://github.com/user/rpytest/releases/download"
VERSION = "0.1.0"


def get_platform_target() -> str:
    """Get the target triple for the current platform."""
    system = platform.system().lower()
    machine = platform.machine().lower()

    # Normalize architecture
    if machine in ("x86_64", "amd64"):
        arch = "x86_64"
    elif machine in ("arm64", "aarch64"):
        arch = "aarch64"
    elif machine in ("i686", "i386"):
        arch = "i686"
    else:
        raise RuntimeError(f"Unsupported architecture: {machine}")

    # Build target triple
    if system == "darwin":
        return f"{arch}-apple-darwin"
    elif system == "linux":
        # Check for musl vs glibc
        try:
            import subprocess
            result = subprocess.run(["ldd", "--version"], capture_output=True, text=True)
            if "musl" in result.stderr.lower() or "musl" in result.stdout.lower():
                return f"{arch}-unknown-linux-musl"
        except Exception:
            pass
        return f"{arch}-unknown-linux-gnu"
    elif system == "windows":
        return f"{arch}-pc-windows-msvc"
    else:
        raise RuntimeError(f"Unsupported platform: {system}")


def download_binary(target: str | None = None, output_dir: Path | None = None) -> Path:
    """Download the rpytest binary for the given target.

    Args:
        target: Target triple (e.g., "x86_64-unknown-linux-gnu").
                If None, auto-detect the current platform.
        output_dir: Directory to place the binary. If None, uses the package bin/ directory.

    Returns:
        Path to the downloaded binary.
    """
    if target is None:
        target = get_platform_target()

    if output_dir is None:
        output_dir = Path(__file__).parent / "bin"

    output_dir.mkdir(parents=True, exist_ok=True)

    # Construct download URL
    archive_name = f"rpytest-{VERSION}-{target}.tar.gz"
    url = f"{RELEASE_URL}/v{VERSION}/{archive_name}"

    print(f"Downloading rpytest {VERSION} for {target}...")
    print(f"URL: {url}")

    try:
        # Download to temp file
        with tempfile.NamedTemporaryFile(delete=False, suffix=".tar.gz") as tmp:
            tmp_path = tmp.name
            urllib.request.urlretrieve(url, tmp_path)

        # Extract binary
        with tarfile.open(tmp_path, "r:gz") as tar:
            # Find the binary in the archive
            for member in tar.getmembers():
                if member.name.endswith("rpytest") or member.name == "rpytest":
                    # Extract to output directory
                    member.name = f"rpytest-{target}"
                    tar.extract(member, output_dir)
                    binary_path = output_dir / f"rpytest-{target}"
                    os.chmod(binary_path, 0o755)
                    print(f"Installed: {binary_path}")
                    return binary_path

        raise RuntimeError("Binary not found in archive")

    except urllib.error.HTTPError as e:
        if e.code == 404:
            raise RuntimeError(
                f"No prebuilt binary available for {target}.\n"
                f"Please build from source: cargo build --release"
            )
        raise

    finally:
        # Clean up temp file
        if 'tmp_path' in locals():
            try:
                os.unlink(tmp_path)
            except OSError:
                pass


def main():
    """CLI entry point for downloading binaries."""
    import argparse

    parser = argparse.ArgumentParser(description="Download rpytest binary")
    parser.add_argument(
        "--target",
        help="Target triple (auto-detected if not specified)"
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        help="Output directory for the binary"
    )
    parser.add_argument(
        "--list-targets",
        action="store_true",
        help="List available targets"
    )

    args = parser.parse_args()

    if args.list_targets:
        print("Available targets:")
        print("  x86_64-unknown-linux-gnu")
        print("  x86_64-unknown-linux-musl")
        print("  aarch64-unknown-linux-gnu")
        print("  x86_64-apple-darwin")
        print("  aarch64-apple-darwin")
        return

    try:
        binary_path = download_binary(args.target, args.output_dir)
        print(f"\nSuccess! Binary installed at: {binary_path}")
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
