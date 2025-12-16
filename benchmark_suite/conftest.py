"""Fixtures for benchmark test suite."""
import pytest
import time


@pytest.fixture
def simple_data():
    """Simple fixture returning data."""
    return {"key": "value", "numbers": list(range(100))}


@pytest.fixture
def computed_fixture():
    """Fixture with some computation."""
    result = sum(i * i for i in range(1000))
    return result


@pytest.fixture(scope="module")
def module_fixture():
    """Module-scoped fixture."""
    return {"module": "data", "items": list(range(50))}


@pytest.fixture(scope="session")
def session_fixture():
    """Session-scoped fixture (expensive to create)."""
    # Simulate expensive setup
    data = {f"key_{i}": i * 2 for i in range(100)}
    return data
