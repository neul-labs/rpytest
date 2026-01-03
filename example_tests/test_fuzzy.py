"""Fuzzy tests for rpytest compatibility verification.

These tests exercise edge cases and various pytest features to ensure
rpytest handles them identically to pytest.
"""

import pytest
import sys
import re


# =============================================================================
# Basic Tests with Various Assertions
# =============================================================================

def test_basic_equality():
    """Basic equality assertion."""
    assert 1 + 1 == 2


def test_basic_inequality():
    """Basic inequality assertion."""
    assert 1 != 2


def test_membership():
    """Membership assertion."""
    assert "a" in "abc"


def test_non_membership():
    """Non-membership assertion."""
    assert "d" not in "abc"


def test_identity_none():
    """Identity assertion with None."""
    x = None
    assert x is None


def test_identity_objects():
    """Identity assertion with objects."""
    a = []
    b = []
    assert a is not b


def test_truthiness():
    """Truthiness assertion."""
    assert bool(1)
    assert bool([1])
    assert bool("hello")


def test_falsiness():
    """Falsiness assertion."""
    assert not bool(0)
    assert not bool([])
    assert not bool("")


def test_comparison_greater():
    """Greater than assertion."""
    assert 5 > 3


def test_comparison_less():
    """Less than assertion."""
    assert 3 < 5


def test_comparison_greater_equal():
    """Greater than or equal assertion."""
    assert 5 >= 5
    assert 5 >= 3


def test_comparison_less_equal():
    """Less than or equal assertion."""
    assert 3 <= 3
    assert 3 <= 5


def test_float_almost_equal():
    """Floating point comparison."""
    assert abs(0.1 + 0.2 - 0.3) < 1e-9


def test_list_equality():
    """List equality assertion."""
    assert [1, 2, 3] == [1, 2, 3]


def test_dict_equality():
    """Dict equality assertion."""
    assert {"a": 1, "b": 2} == {"a": 1, "b": 2}


def test_set_equality():
    """Set equality assertion."""
    assert {1, 2, 3} == {3, 2, 1}


def test_tuple_equality():
    """Tuple equality assertion."""
    assert (1, 2, 3) == (1, 2, 3)


# =============================================================================
# String Edge Cases
# =============================================================================

def test_unicode_string():
    """Test unicode strings."""
    assert "cafe" == "cafe"


def test_empty_string():
    """Test empty string."""
    assert "" == ""
    assert len("") == 0


def test_multiline_string():
    """Test multiline strings."""
    text = """line1
line2
line3"""
    assert "line2" in text


def test_string_with_quotes():
    """Test string with quotes."""
    assert 'He said "hello"' == 'He said "hello"'


def test_string_with_backslash():
    """Test string with backslash."""
    assert "path\\to\\file" == "path\\to\\file"


def test_string_with_tab():
    """Test string with tab."""
    assert "col1\tcol2" == "col1\tcol2"


def test_string_with_newline():
    """Test string with newline."""
    assert "line1\nline2" == "line1\nline2"


# =============================================================================
# Data Structure Edge Cases
# =============================================================================

def test_empty_list():
    """Test empty list."""
    assert [] == []
    assert len([]) == 0


def test_empty_dict():
    """Test empty dict."""
    assert {} == {}
    assert len({}) == 0


def test_empty_set():
    """Test empty set."""
    assert set() == set()


def test_nested_data():
    """Test nested data structures."""
    data = {"a": {"b": {"c": [1, 2, 3]}}}
    assert data["a"]["b"]["c"] == [1, 2, 3]


def test_large_list():
    """Test large list comparison."""
    large = list(range(1000))
    assert large == list(range(1000))


def test_none_value():
    """Test None value."""
    x = None
    assert x is None
    assert x == None  # noqa: E711


# =============================================================================
# Numeric Edge Cases
# =============================================================================

def test_zero():
    """Test zero."""
    assert 0 == 0
    assert -0 == 0


def test_negative_numbers():
    """Test negative numbers."""
    assert -1 < 0
    assert -100 + 100 == 0


def test_large_numbers():
    """Test large numbers."""
    assert 10**100 > 10**99


def test_float_zero():
    """Test float zero."""
    assert 0.0 == 0
    assert -0.0 == 0.0


# =============================================================================
# Exception Testing
# =============================================================================

def test_raises_value_error():
    """Test ValueError is raised."""
    with pytest.raises(ValueError):
        raise ValueError("test error")


def test_raises_type_error():
    """Test TypeError is raised."""
    with pytest.raises(TypeError):
        "string" + 123  # type: ignore


def test_raises_key_error():
    """Test KeyError is raised."""
    with pytest.raises(KeyError):
        {}["missing"]


def test_raises_index_error():
    """Test IndexError is raised."""
    with pytest.raises(IndexError):
        [][0]


def test_raises_zero_division():
    """Test ZeroDivisionError is raised."""
    with pytest.raises(ZeroDivisionError):
        1 / 0


def test_raises_with_message():
    """Test exception with message match."""
    with pytest.raises(ValueError, match=r"test.*error"):
        raise ValueError("test error message")


# =============================================================================
# Skip and XFail
# =============================================================================

@pytest.mark.skip(reason="Intentionally skipped")
def test_unconditional_skip():
    """This test is always skipped."""
    assert False


@pytest.mark.skipif(sys.platform != "nonexistent_platform", reason="Platform check")
def test_conditional_skip_runs():
    """Skip condition is false, test runs."""
    assert True


@pytest.mark.skipif(True, reason="Always skip")
def test_conditional_skip_skips():
    """Skip condition is true, test skips."""
    assert False


@pytest.mark.xfail(reason="Expected failure")
def test_xfail_fails():
    """Expected failure that fails."""
    assert False


@pytest.mark.xfail(reason="Expected failure but passes")
def test_xfail_passes():
    """Expected failure that passes (XPASS)."""
    assert True


# =============================================================================
# Fixtures
# =============================================================================

@pytest.fixture
def simple_value():
    """Simple fixture returning a value."""
    return 42


@pytest.fixture
def list_value():
    """Fixture returning a list."""
    return [1, 2, 3]


@pytest.fixture
def dict_value():
    """Fixture returning a dict."""
    return {"key": "value"}


def test_with_simple_fixture(simple_value):
    """Test using simple fixture."""
    assert simple_value == 42


def test_with_list_fixture(list_value):
    """Test using list fixture."""
    assert len(list_value) == 3


def test_with_dict_fixture(dict_value):
    """Test using dict fixture."""
    assert dict_value["key"] == "value"


def test_with_multiple_fixtures(simple_value, list_value, dict_value):
    """Test using multiple fixtures."""
    assert simple_value == 42
    assert len(list_value) == 3
    assert "key" in dict_value


@pytest.fixture
def setup_teardown_fixture():
    """Fixture with setup and teardown."""
    # Setup
    data = {"created": True}
    yield data
    # Teardown happens here
    data["cleaned"] = True


def test_with_setup_teardown(setup_teardown_fixture):
    """Test using fixture with setup/teardown."""
    assert setup_teardown_fixture["created"] is True


# =============================================================================
# Built-in Fixtures
# =============================================================================

def test_with_tmp_path(tmp_path):
    """Test using tmp_path fixture."""
    test_file = tmp_path / "test.txt"
    test_file.write_text("hello")
    assert test_file.read_text() == "hello"


def test_with_capsys(capsys):
    """Test using capsys fixture."""
    print("hello stdout")
    captured = capsys.readouterr()
    assert "hello" in captured.out


# =============================================================================
# Test Classes
# =============================================================================

class TestBasicClass:
    """Basic test class."""

    def test_method_one(self):
        """First test method."""
        assert True

    def test_method_two(self):
        """Second test method."""
        assert 1 + 1 == 2


class TestClassWithFixture:
    """Test class using fixtures."""

    @pytest.fixture
    def class_fixture(self):
        return "class_value"

    def test_uses_fixture(self, class_fixture):
        """Test using class fixture."""
        assert class_fixture == "class_value"


# =============================================================================
# Parametrized Tests (Simple Cases)
# =============================================================================

@pytest.mark.parametrize("value", [1, 2, 3])
def test_parametrize_simple(value):
    """Simple parametrized test."""
    assert value > 0


@pytest.mark.parametrize("a,b,expected", [
    (1, 2, 3),
    (0, 0, 0),
    (5, 5, 10),
])
def test_parametrize_multiple_args(a, b, expected):
    """Parametrized test with multiple args."""
    assert a + b == expected


@pytest.mark.parametrize("text", ["hello", "world", "test"])
def test_parametrize_strings(text):
    """Parametrized test with strings."""
    assert len(text) > 0


# =============================================================================
# Regex Tests
# =============================================================================

def test_regex_match():
    """Test regex matching."""
    pattern = r"\d{3}-\d{4}"
    assert re.match(pattern, "123-4567")


def test_regex_search():
    """Test regex searching."""
    pattern = r"error"
    assert re.search(pattern, "an error occurred")


def test_regex_findall():
    """Test regex findall."""
    pattern = r"\d+"
    matches = re.findall(pattern, "a1b2c3")
    assert matches == ["1", "2", "3"]


# =============================================================================
# Boolean Edge Cases
# =============================================================================

def test_bool_true():
    """Test True."""
    assert True is True


def test_bool_false():
    """Test False."""
    assert False is False


def test_bool_not():
    """Test not operator."""
    assert not False
    assert not None
    assert not 0
    assert not ""


def test_bool_and():
    """Test and operator."""
    assert True and True
    assert not (True and False)


def test_bool_or():
    """Test or operator."""
    assert True or False
    assert False or True
