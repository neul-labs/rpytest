"""Fuzzy tests for output capture compatibility."""

import pytest
import sys
import warnings
import logging


# =============================================================================
# Basic Output Capture
# =============================================================================

def test_print_stdout():
    """Test stdout print."""
    print("Hello stdout")
    assert True


def test_print_stderr():
    """Test stderr print."""
    print("Hello stderr", file=sys.stderr)
    assert True


def test_print_both():
    """Test printing to both stdout and stderr."""
    print("stdout message")
    print("stderr message", file=sys.stderr)
    assert True


# =============================================================================
# Capsys Fixture
# =============================================================================

def test_capsys_stdout(capsys):
    """Capture stdout with capsys."""
    print("captured stdout")
    captured = capsys.readouterr()
    assert "captured stdout" in captured.out


def test_capsys_stderr(capsys):
    """Capture stderr with capsys."""
    print("captured stderr", file=sys.stderr)
    captured = capsys.readouterr()
    assert "captured stderr" in captured.err


def test_capsys_multiple_reads(capsys):
    """Multiple readouterr calls."""
    print("first")
    cap1 = capsys.readouterr()
    print("second")
    cap2 = capsys.readouterr()
    assert "first" in cap1.out
    assert "second" in cap2.out
    assert "first" not in cap2.out


def test_capsys_empty(capsys):
    """Empty capture."""
    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""


# =============================================================================
# Capfd Fixture
# =============================================================================

def test_capfd_stdout(capfd):
    """Capture stdout with capfd."""
    print("fd captured")
    captured = capfd.readouterr()
    assert "fd captured" in captured.out


def test_capfd_stderr(capfd):
    """Capture stderr with capfd."""
    print("fd error", file=sys.stderr)
    captured = capfd.readouterr()
    assert "fd error" in captured.err


# =============================================================================
# Disable Capture
# =============================================================================

def test_capsys_disabled(capsys):
    """Test with capture disabled temporarily."""
    print("before disable")
    with capsys.disabled():
        print("during disable - not captured")
    print("after disable")
    captured = capsys.readouterr()
    assert "before disable" in captured.out
    assert "after disable" in captured.out
    # "during disable" goes to real stdout


# =============================================================================
# Warnings Capture
# =============================================================================

def test_warning_basic():
    """Test basic warning."""
    warnings.warn("test warning", UserWarning)
    assert True


def test_recwarn_fixture(recwarn):
    """Capture warnings with recwarn."""
    warnings.warn("captured warning", UserWarning)
    assert len(recwarn) == 1
    assert "captured" in str(recwarn[0].message)


def test_warns_context():
    """Use pytest.warns context manager."""
    with pytest.warns(UserWarning):
        warnings.warn("expected warning", UserWarning)


def test_warns_match():
    """pytest.warns with message matching."""
    with pytest.warns(UserWarning, match="expected"):
        warnings.warn("expected warning message", UserWarning)


def test_deprecated_call():
    """Test deprecated_call context manager."""
    def deprecated_func():
        warnings.warn("deprecated", DeprecationWarning)
        return 42

    with pytest.deprecated_call():
        result = deprecated_func()
    assert result == 42


# =============================================================================
# Logging Capture
# =============================================================================

def test_caplog_basic(caplog):
    """Basic logging capture."""
    logging.warning("test warning log")
    assert "test warning" in caplog.text


def test_caplog_level(caplog):
    """Capture at specific level."""
    with caplog.at_level(logging.DEBUG):
        logging.debug("debug message")
        logging.info("info message")
    assert "debug message" in caplog.text
    assert "info message" in caplog.text


def test_caplog_records(caplog):
    """Access log records."""
    logging.error("error message")
    assert len(caplog.records) >= 1
    assert any(r.levelname == "ERROR" for r in caplog.records)


def test_caplog_clear(caplog):
    """Clear captured logs."""
    with caplog.at_level(logging.INFO):
        logging.info("first")
        caplog.clear()
        logging.info("second")
        assert "first" not in caplog.text
        assert "second" in caplog.text


# =============================================================================
# Multiline Output
# =============================================================================

def test_multiline_stdout(capsys):
    """Multiline stdout."""
    print("line 1")
    print("line 2")
    print("line 3")
    captured = capsys.readouterr()
    assert captured.out.count("\n") >= 3


def test_multiline_no_newline(capsys):
    """Print without newline."""
    print("no newline", end="")
    captured = capsys.readouterr()
    assert captured.out == "no newline"


# =============================================================================
# Unicode Output
# =============================================================================

def test_unicode_stdout(capsys):
    """Unicode in stdout."""
    print("Unicode: cafe")
    captured = capsys.readouterr()
    assert "cafe" in captured.out


def test_unicode_emoji(capsys):
    """Emoji in output."""
    print("Status: OK")
    captured = capsys.readouterr()
    assert "OK" in captured.out


# =============================================================================
# Large Output
# =============================================================================

def test_large_stdout(capsys):
    """Large stdout output."""
    for i in range(100):
        print(f"line {i}")
    captured = capsys.readouterr()
    assert "line 0" in captured.out
    assert "line 99" in captured.out


# =============================================================================
# Output with Exceptions
# =============================================================================

def test_output_before_exception(capsys):
    """Output captured even when test fails."""
    print("before exception")
    # Don't actually raise, just verify capture works
    captured = capsys.readouterr()
    assert "before exception" in captured.out


# =============================================================================
# Fixture Output
# =============================================================================

@pytest.fixture
def fixture_with_output(capsys):
    """Fixture that produces output."""
    print("fixture setup")
    yield "value"
    print("fixture teardown")


def test_fixture_output(fixture_with_output, capsys):
    """Test with fixture that outputs."""
    print("test body")
    assert fixture_with_output == "value"
    captured = capsys.readouterr()
    assert "test body" in captured.out
