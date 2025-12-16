"""pytest configuration for example tests."""

import pytest


@pytest.fixture
def sample_data():
    """Provide sample data for tests."""
    return {"name": "test", "value": 42}
