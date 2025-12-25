"""Fixtures for benchmark tests."""
import pytest


@pytest.fixture
def simple_data():
    """Simple function-scoped fixture."""
    return {
        "key": "value",
        "numbers": list(range(100)),
        "nested": {"a": 1, "b": 2},
    }


@pytest.fixture(scope="session")
def session_data():
    """Session-scoped fixture."""
    return {"session_id": 12345, "config": {"debug": True}}
