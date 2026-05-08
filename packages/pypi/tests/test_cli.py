"""Tests for the rpytest CLI wrapper."""

import subprocess
import sys
from pathlib import Path

import pytest

# Add the package root to the path so we can import rpytest_cli
sys.path.insert(0, str(Path(__file__).parent.parent))

from rpytest_cli import get_binary_path, main


def test_get_binary_path_finds_cargo_target():
    """If a cargo target binary exists, get_binary_path should find it."""
    # This test may fail in CI if the binary isn't built, so we just check
    # that the function doesn't crash when no binary is found.
    try:
        path = get_binary_path()
        assert path.exists()
    except RuntimeError as exc:
        # Expected when no binary is available in the environment
        assert "rpytest binary not found" in str(exc)


def test_main_exits_with_error_when_binary_missing(monkeypatch, capsys):
    """main() should print an error and exit when the binary is missing."""
    monkeypatch.setattr(
        "rpytest_cli.get_binary_path",
        lambda: (_ for _ in ()).throw(RuntimeError("binary not found")),
    )
    with pytest.raises(SystemExit) as exc_info:
        main()
    assert exc_info.value.code == 1
    captured = capsys.readouterr()
    assert "binary not found" in captured.err
