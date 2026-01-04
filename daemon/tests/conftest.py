"""Shared fixtures for daemon tests."""

import tempfile
from pathlib import Path
from typing import Generator

import pytest


@pytest.fixture
def temp_dir() -> Generator[Path, None, None]:
    """Create a temporary directory for tests."""
    with tempfile.TemporaryDirectory() as tmpdir:
        yield Path(tmpdir)


@pytest.fixture
def sample_test_file(temp_dir: Path) -> Path:
    """Create a sample test file for collection tests."""
    test_file = temp_dir / "test_sample.py"
    test_file.write_text('''
def test_simple():
    """A simple test."""
    assert True

def test_with_assertion():
    """Test with an assertion."""
    assert 1 + 1 == 2

class TestClass:
    """Test class."""

    def test_method(self):
        """Test method."""
        assert True
''')
    return test_file


@pytest.fixture
def sample_conftest(temp_dir: Path) -> Path:
    """Create a sample conftest.py with fixtures."""
    conftest = temp_dir / "conftest.py"
    conftest.write_text('''
import pytest

@pytest.fixture
def sample_fixture():
    """Sample fixture."""
    return {"key": "value"}

@pytest.fixture(scope="session")
def session_fixture():
    """Session-scoped fixture."""
    return "session_data"
''')
    return conftest


@pytest.fixture
def parametrized_test_file(temp_dir: Path) -> Path:
    """Create a test file with parametrized tests."""
    test_file = temp_dir / "test_parametrized.py"
    test_file.write_text('''
import pytest

@pytest.mark.parametrize("value", [1, 2, 3])
def test_single_param(value):
    assert value > 0

@pytest.mark.parametrize("a,b", [(1, 2), (3, 4)])
def test_multiple_params(a, b):
    assert a < b

@pytest.mark.parametrize("x", [1, 2])
@pytest.mark.parametrize("y", ["a", "b"])
def test_stacked_params(x, y):
    assert x > 0
    assert y in ("a", "b")
''')
    return test_file


@pytest.fixture
def marked_test_file(temp_dir: Path) -> Path:
    """Create a test file with various markers."""
    test_file = temp_dir / "test_markers.py"
    test_file.write_text('''
import pytest

def test_plain():
    assert True

@pytest.mark.skip(reason="testing skip")
def test_skipped():
    assert False

@pytest.mark.xfail
def test_xfail():
    assert False

@pytest.mark.slow
def test_custom_marker():
    assert True
''')
    return test_file
