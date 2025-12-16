"""Example test file for rpytest verification."""

import pytest


def test_addition():
    """Simple passing test."""
    assert 1 + 1 == 2


def test_subtraction():
    """Another passing test."""
    assert 5 - 3 == 2


def test_multiplication():
    """Yet another passing test."""
    assert 3 * 4 == 12


@pytest.mark.skip(reason="Demonstrating skip")
def test_skipped():
    """This test should be skipped."""
    assert False


class TestMathOperations:
    """Test class with multiple methods."""

    def test_division(self):
        """Test division."""
        assert 10 / 2 == 5

    def test_modulo(self):
        """Test modulo operator."""
        assert 10 % 3 == 1
