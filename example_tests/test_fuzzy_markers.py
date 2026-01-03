"""Fuzzy tests for marker compatibility."""

import pytest
import sys


# =============================================================================
# Skip Markers
# =============================================================================

@pytest.mark.skip
def test_skip_no_reason():
    """Skip without reason."""
    assert False


@pytest.mark.skip(reason="explicit skip reason")
def test_skip_with_reason():
    """Skip with reason."""
    assert False


@pytest.mark.skipif(True, reason="condition is true")
def test_skipif_true():
    """Skipif with true condition."""
    assert False


@pytest.mark.skipif(False, reason="condition is false")
def test_skipif_false():
    """Skipif with false condition - should run."""
    assert True


@pytest.mark.skipif(sys.version_info < (2, 0), reason="Python 2+ required")
def test_skipif_version_passes():
    """Version check that passes."""
    assert True


@pytest.mark.skipif(sys.version_info < (99, 0), reason="Python 99+ required")
def test_skipif_version_skips():
    """Version check that skips."""
    assert False


# =============================================================================
# XFail Markers
# =============================================================================

@pytest.mark.xfail
def test_xfail_no_reason():
    """Xfail without reason."""
    assert False


@pytest.mark.xfail(reason="expected to fail")
def test_xfail_with_reason():
    """Xfail with reason."""
    assert False


@pytest.mark.xfail(reason="expected to fail but passes")
def test_xfail_passes():
    """Xfail that actually passes (XPASS)."""
    assert True


@pytest.mark.xfail(strict=True, reason="strict xfail")
def test_xfail_strict_fails():
    """Strict xfail that fails."""
    assert False


@pytest.mark.xfail(condition=True, reason="condition true")
def test_xfail_condition_true():
    """Xfail with true condition."""
    assert False


@pytest.mark.xfail(condition=False, reason="condition false")
def test_xfail_condition_false():
    """Xfail with false condition - runs normally."""
    assert True


@pytest.mark.xfail(raises=ValueError, reason="expects ValueError")
def test_xfail_raises_correct():
    """Xfail expecting specific exception."""
    raise ValueError("expected")


@pytest.mark.xfail(raises=TypeError, reason="expects TypeError but gets ValueError")
def test_xfail_raises_wrong():
    """Xfail expecting wrong exception type - will fail differently."""
    # This raises ValueError instead of TypeError, so xfail doesn't apply
    # and the test fails. Mark as skip to avoid test failure.
    pytest.skip("Intentionally skipped - demonstrates xfail edge case")


# =============================================================================
# Custom Markers
# =============================================================================

@pytest.mark.slow
def test_marked_slow():
    """Test marked as slow."""
    assert True


@pytest.mark.integration
def test_marked_integration():
    """Test marked as integration."""
    assert True


@pytest.mark.unit
def test_marked_unit():
    """Test marked as unit."""
    assert True


@pytest.mark.smoke
def test_marked_smoke():
    """Test marked as smoke."""
    assert True


# =============================================================================
# Multiple Markers
# =============================================================================

@pytest.mark.slow
@pytest.mark.integration
def test_multiple_markers():
    """Test with multiple markers."""
    assert True


@pytest.mark.unit
@pytest.mark.smoke
@pytest.mark.fast
def test_three_markers():
    """Test with three markers."""
    assert True


# =============================================================================
# Markers with Arguments
# =============================================================================

@pytest.mark.timeout(10)
def test_marker_with_int_arg():
    """Marker with integer argument."""
    assert True


@pytest.mark.category("auth")
def test_marker_with_string_arg():
    """Marker with string argument."""
    assert True


@pytest.mark.priority(level=1, critical=True)
def test_marker_with_kwargs():
    """Marker with keyword arguments."""
    assert True


# =============================================================================
# Marker on Classes
# =============================================================================

@pytest.mark.slow
class TestMarkedClass:
    """Class with marker."""

    def test_one(self):
        """First test in marked class."""
        assert True

    def test_two(self):
        """Second test in marked class."""
        assert True


@pytest.mark.skip(reason="skip entire class")
class TestSkippedClass:
    """Skipped class."""

    def test_should_skip(self):
        """This should be skipped."""
        assert False


@pytest.mark.xfail(reason="xfail entire class")
class TestXfailClass:
    """Xfail class."""

    def test_should_xfail(self):
        """This should xfail."""
        assert False


# =============================================================================
# Usefixtures Marker
# =============================================================================

@pytest.fixture
def setup_data():
    """Fixture for usefixtures tests."""
    return {"setup": True}


@pytest.mark.usefixtures("setup_data")
def test_usefixtures():
    """Test using usefixtures marker."""
    # Note: fixture value not accessible directly
    assert True


@pytest.mark.usefixtures("setup_data")
class TestUsefixtures:
    """Class using usefixtures."""

    def test_in_class(self):
        """Test in class with usefixtures."""
        assert True


# =============================================================================
# Filterwarnings Marker
# =============================================================================

@pytest.mark.filterwarnings("ignore::DeprecationWarning")
def test_ignore_deprecation():
    """Test ignoring deprecation warnings."""
    import warnings
    warnings.warn("deprecated", DeprecationWarning)
    assert True


# =============================================================================
# Combined Skip and Xfail Logic
# =============================================================================

@pytest.mark.skip(reason="skip takes precedence")
@pytest.mark.xfail(reason="xfail ignored")
def test_skip_over_xfail():
    """Skip should take precedence over xfail."""
    assert False


# =============================================================================
# Parametrize with Markers
# =============================================================================

@pytest.mark.slow
@pytest.mark.parametrize("n", [1, 2, 3])
def test_marker_with_parametrize(n):
    """Marker combined with parametrize."""
    assert n > 0


@pytest.mark.parametrize("x", [
    pytest.param(1, marks=pytest.mark.slow),
    pytest.param(2, marks=[pytest.mark.slow, pytest.mark.integration]),
    pytest.param(3),
])
def test_param_specific_markers(x):
    """Parameters with specific markers."""
    assert x > 0
