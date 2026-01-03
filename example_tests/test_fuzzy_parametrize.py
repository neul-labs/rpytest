"""Fuzzy tests for parametrize compatibility."""

import pytest


# =============================================================================
# Basic Parametrize
# =============================================================================

@pytest.mark.parametrize("x", [1, 2, 3, 4, 5])
def test_single_param(x):
    """Single parameter."""
    assert x > 0


@pytest.mark.parametrize("x,y", [(1, 2), (3, 4), (5, 6)])
def test_two_params(x, y):
    """Two parameters."""
    assert x < y


@pytest.mark.parametrize("a,b,c", [
    (1, 2, 3),
    (4, 5, 9),
    (10, 20, 30),
])
def test_three_params(a, b, c):
    """Three parameters."""
    assert a + b == c


# =============================================================================
# Parametrize with Different Types
# =============================================================================

@pytest.mark.parametrize("value", [1, 1.5, "string", None, True, False])
def test_mixed_types(value):
    """Mixed type parameters."""
    # Just check it doesn't crash
    assert value is not ...


@pytest.mark.parametrize("lst", [[], [1], [1, 2], [1, 2, 3]])
def test_list_params(lst):
    """List parameters."""
    assert isinstance(lst, list)


@pytest.mark.parametrize("dct", [{}, {"a": 1}, {"a": 1, "b": 2}])
def test_dict_params(dct):
    """Dict parameters."""
    assert isinstance(dct, dict)


# =============================================================================
# Parametrize with IDs
# =============================================================================

@pytest.mark.parametrize("n", [
    pytest.param(1, id="one"),
    pytest.param(2, id="two"),
    pytest.param(3, id="three"),
])
def test_with_ids(n):
    """Test with custom IDs."""
    assert n in [1, 2, 3]


@pytest.mark.parametrize("x,expected", [
    pytest.param(2, 4, id="square-of-2"),
    pytest.param(3, 9, id="square-of-3"),
    pytest.param(4, 16, id="square-of-4"),
])
def test_with_ids_multiple_params(x, expected):
    """Multiple params with IDs."""
    assert x * x == expected


# =============================================================================
# Parametrize with Marks
# =============================================================================

@pytest.mark.parametrize("n", [
    pytest.param(1),
    pytest.param(2, marks=pytest.mark.skip(reason="skip this")),
    pytest.param(3),
])
def test_param_with_skip(n):
    """Parameter with skip mark."""
    assert n in [1, 3]  # 2 is skipped


@pytest.mark.parametrize("n", [
    pytest.param(1),
    pytest.param(0, marks=pytest.mark.xfail(reason="zero fails")),
    pytest.param(3),
])
def test_param_with_xfail(n):
    """Parameter with xfail mark."""
    assert n > 0  # 0 will xfail


# =============================================================================
# Stacked Parametrize (Cartesian Product)
# =============================================================================

@pytest.mark.parametrize("x", [1, 2])
@pytest.mark.parametrize("y", [10, 20])
def test_stacked_parametrize(x, y):
    """Stacked parametrize creates cartesian product."""
    assert x in [1, 2]
    assert y in [10, 20]


@pytest.mark.parametrize("a", ["x", "y"])
@pytest.mark.parametrize("b", ["1", "2"])
@pytest.mark.parametrize("c", ["!", "?"])
def test_triple_stacked(a, b, c):
    """Triple stacked parametrize."""
    result = a + b + c
    assert len(result) == 3


# =============================================================================
# Parametrize with Fixtures
# =============================================================================

@pytest.fixture
def base_value():
    return 100


@pytest.mark.parametrize("multiplier", [1, 2, 3])
def test_param_with_fixture(base_value, multiplier):
    """Parametrize combined with fixture."""
    result = base_value * multiplier
    assert result in [100, 200, 300]


# =============================================================================
# Edge Case Values
# =============================================================================

@pytest.mark.parametrize("value", [0, -0, 0.0, -0.0])
def test_zero_variants(value):
    """Different representations of zero."""
    assert value == 0


@pytest.mark.parametrize("value", ["", " ", "  ", "\t", "\n"])
def test_whitespace_variants(value):
    """Whitespace variations."""
    assert isinstance(value, str)


@pytest.mark.parametrize("value", [True, False, 1, 0, "", "text", [], [1]])
def test_truthy_falsy(value):
    """Truthy and falsy values."""
    # Just verify no crashes
    _ = bool(value)


# =============================================================================
# Parametrize Class Methods
# =============================================================================

class TestParametrizedClass:
    """Class with parametrized methods."""

    @pytest.mark.parametrize("n", [1, 2, 3])
    def test_method_param(self, n):
        """Parametrized method."""
        assert n > 0

    @pytest.mark.parametrize("x,y", [(1, 1), (2, 4), (3, 9)])
    def test_method_multi_param(self, x, y):
        """Multi-param method."""
        assert x * x == y


# =============================================================================
# Indirect Parametrize
# =============================================================================

@pytest.fixture
def double_value(request):
    """Fixture that doubles the param."""
    return request.param * 2


@pytest.mark.parametrize("double_value", [1, 2, 3], indirect=True)
def test_indirect_param(double_value):
    """Test with indirect parametrize."""
    assert double_value in [2, 4, 6]


# =============================================================================
# Empty and Single Parametrize
# =============================================================================

@pytest.mark.parametrize("x", [42])
def test_single_value_param(x):
    """Single value parametrize."""
    assert x == 42


# =============================================================================
# Parametrize with None
# =============================================================================

@pytest.mark.parametrize("value", [None, "not none"])
def test_none_param(value):
    """Parametrize including None."""
    if value is None:
        assert value is None
    else:
        assert value == "not none"


# =============================================================================
# Parametrize with Boolean
# =============================================================================

@pytest.mark.parametrize("flag", [True, False])
def test_boolean_param(flag):
    """Boolean parameter."""
    assert isinstance(flag, bool)


@pytest.mark.parametrize("a,b,expected", [
    (True, True, True),
    (True, False, False),
    (False, True, False),
    (False, False, False),
])
def test_boolean_and(a, b, expected):
    """Boolean AND truth table."""
    assert (a and b) == expected


@pytest.mark.parametrize("a,b,expected", [
    (True, True, True),
    (True, False, True),
    (False, True, True),
    (False, False, False),
])
def test_boolean_or(a, b, expected):
    """Boolean OR truth table."""
    assert (a or b) == expected
