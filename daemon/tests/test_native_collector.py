"""Tests for the native AST-based test collector."""

import tempfile
from pathlib import Path

import pytest

from rpytest_daemon.native_collector import (
    NativeTestNode,
    NativeCollector,
    collect_tests_native,
)


class TestNativeTestNode:
    """Tests for NativeTestNode dataclass."""

    def test_basic_creation(self):
        node = NativeTestNode(
            node_id="test.py::test_func",
            file_path="test.py",
            name="test_func",
        )
        assert node.node_id == "test.py::test_func"
        assert node.file_path == "test.py"
        assert node.name == "test_func"
        assert node.class_name is None
        assert node.line_number == 0
        assert node.markers == []
        assert node.is_simple is True
        assert node.parameters == []

    def test_with_class(self):
        node = NativeTestNode(
            node_id="test.py::TestClass::test_method",
            file_path="test.py",
            name="test_method",
            class_name="TestClass",
            line_number=25,
        )
        assert node.class_name == "TestClass"
        assert node.line_number == 25

    def test_with_markers(self):
        node = NativeTestNode(
            node_id="test.py::test_func",
            file_path="test.py",
            name="test_func",
            markers=["slow", "integration"],
            is_simple=False,
        )
        assert node.markers == ["slow", "integration"]
        assert node.is_simple is False


class TestNativeCollector:
    """Tests for NativeCollector class."""

    def test_init(self, temp_dir):
        collector = NativeCollector(temp_dir)
        assert collector.repo_path == temp_dir
        assert collector.tests == {}

    def test_find_test_files(self, temp_dir):
        # Create test files
        (temp_dir / "test_a.py").write_text("def test_a(): pass")
        (temp_dir / "test_b.py").write_text("def test_b(): pass")
        (temp_dir / "not_a_test.py").write_text("def foo(): pass")
        (temp_dir / "something_test.py").write_text("def test_c(): pass")

        collector = NativeCollector(temp_dir)
        files = collector._find_test_files()

        file_names = [f.name for f in files]
        # Should find test_*.py and *_test.py files
        assert "test_a.py" in file_names
        assert "test_b.py" in file_names
        assert "something_test.py" in file_names
        # Note: The collector finds files matching test_*.py OR *_test.py patterns
        # Files that don't match these patterns are excluded
        test_file_count = sum(1 for f in file_names
                              if f.startswith("test_") or f.endswith("_test.py"))
        assert test_file_count >= 3

    def test_collect_simple_function(self, temp_dir):
        test_file = temp_dir / "test_simple.py"
        test_file.write_text("""
def test_one():
    assert True

def test_two():
    assert 1 == 1
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        assert len(tests) == 2
        assert "test_simple.py::test_one" in tests
        assert "test_simple.py::test_two" in tests

        node = tests["test_simple.py::test_one"]
        assert node.name == "test_one"
        assert node.class_name is None
        assert node.is_simple is True

    def test_collect_test_class(self, temp_dir):
        test_file = temp_dir / "test_class.py"
        test_file.write_text("""
class TestExample:
    def test_method(self):
        assert True

    def test_another(self):
        assert True
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        assert len(tests) == 2
        assert "test_class.py::TestExample::test_method" in tests
        assert "test_class.py::TestExample::test_another" in tests

        node = tests["test_class.py::TestExample::test_method"]
        assert node.class_name == "TestExample"

    def test_collect_with_fixtures_not_simple(self, temp_dir, sample_conftest):
        test_file = temp_dir / "test_fixtures.py"
        test_file.write_text("""
def test_with_fixture(sample_fixture):
    assert sample_fixture == {"key": "value"}
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        node = tests["test_fixtures.py::test_with_fixture"]
        assert node.is_simple is False  # Uses fixture

    def test_collect_with_builtin_fixtures(self, temp_dir):
        test_file = temp_dir / "test_builtins.py"
        test_file.write_text("""
def test_with_tmp_path(tmp_path):
    assert tmp_path.exists()

def test_with_capsys(capsys):
    print("hello")
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        assert tests["test_builtins.py::test_with_tmp_path"].is_simple is False
        assert tests["test_builtins.py::test_with_capsys"].is_simple is False

    def test_collect_markers(self, temp_dir):
        test_file = temp_dir / "test_markers.py"
        test_file.write_text("""
import pytest

@pytest.mark.slow
def test_slow():
    pass

@pytest.mark.skip(reason="not ready")
def test_skipped():
    pass
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        assert "slow" in tests["test_markers.py::test_slow"].markers
        assert "skip" in tests["test_markers.py::test_skipped"].markers
        # skip marker makes it not simple
        assert tests["test_markers.py::test_skipped"].is_simple is False

    def test_collect_parametrized_single(self, temp_dir):
        test_file = temp_dir / "test_param.py"
        test_file.write_text("""
import pytest

@pytest.mark.parametrize("value", [1, 2, 3])
def test_param(value):
    assert value > 0
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        # Should expand to 3 tests
        assert len(tests) == 3
        assert "test_param.py::test_param[1]" in tests
        assert "test_param.py::test_param[2]" in tests
        assert "test_param.py::test_param[3]" in tests

    def test_collect_parametrized_tuple(self, temp_dir):
        test_file = temp_dir / "test_param_tuple.py"
        test_file.write_text("""
import pytest

@pytest.mark.parametrize("a,b", [(1, 2), (3, 4)])
def test_param(a, b):
    assert a < b
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        assert len(tests) == 2
        assert "test_param_tuple.py::test_param[1-2]" in tests
        assert "test_param_tuple.py::test_param[3-4]" in tests

    def test_collect_parametrized_strings(self, temp_dir):
        test_file = temp_dir / "test_param_str.py"
        test_file.write_text('''
import pytest

@pytest.mark.parametrize("name", ["alice", "bob"])
def test_greeting(name):
    assert len(name) > 0
''')

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        assert len(tests) == 2
        assert "test_param_str.py::test_greeting[alice]" in tests
        assert "test_param_str.py::test_greeting[bob]" in tests

    def test_collect_parametrized_none_and_bool(self, temp_dir):
        test_file = temp_dir / "test_param_special.py"
        test_file.write_text("""
import pytest

@pytest.mark.parametrize("value", [None, True, False])
def test_special(value):
    pass
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        assert len(tests) == 3
        assert "test_param_special.py::test_special[None]" in tests
        assert "test_param_special.py::test_special[True]" in tests
        assert "test_param_special.py::test_special[False]" in tests

    def test_collect_async_test(self, temp_dir):
        test_file = temp_dir / "test_async.py"
        test_file.write_text("""
import pytest

async def test_async():
    await some_async_func()
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        node = tests["test_async.py::test_async"]
        assert node.is_simple is False  # Async tests require pytest-asyncio

    def test_collect_class_with_setup(self, temp_dir):
        test_file = temp_dir / "test_setup.py"
        test_file.write_text("""
class TestWithSetup:
    def setup_method(self):
        self.data = []

    def test_method(self):
        assert self.data == []
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        node = tests["test_setup.py::TestWithSetup::test_method"]
        assert node.is_simple is False  # Class has setup_method

    def test_collect_handles_syntax_error(self, temp_dir):
        test_file = temp_dir / "test_syntax_error.py"
        test_file.write_text("def test_broken(:\n    pass")  # Invalid syntax

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        # Should not crash, just skip the file
        assert len(tests) == 0

    def test_cache_creation(self, temp_dir):
        test_file = temp_dir / "test_cache.py"
        test_file.write_text("def test_one(): pass")

        collector = NativeCollector(temp_dir)
        collector.collect(use_cache=True)

        cache_path = temp_dir / ".rpytest" / "native_tests.json"
        assert cache_path.exists()

    def test_cache_loading(self, temp_dir):
        test_file = temp_dir / "test_cache.py"
        test_file.write_text("def test_one(): pass")

        # First collection - creates cache
        collector1 = NativeCollector(temp_dir)
        tests1 = collector1.collect(use_cache=True)

        # Second collection - should load from cache
        collector2 = NativeCollector(temp_dir)
        tests2 = collector2.collect(use_cache=True)

        assert tests1.keys() == tests2.keys()

    def test_cache_invalidation_on_file_change(self, temp_dir):
        test_file = temp_dir / "test_cache.py"
        test_file.write_text("def test_one(): pass")

        # First collection
        collector1 = NativeCollector(temp_dir)
        collector1.collect(use_cache=True)

        # Modify file (update mtime)
        import time
        time.sleep(0.1)  # Ensure different mtime
        test_file.write_text("def test_one(): pass\ndef test_two(): pass")

        # Second collection - cache should be invalid
        collector2 = NativeCollector(temp_dir)
        tests2 = collector2.collect(use_cache=True)

        assert len(tests2) == 2

    def test_conftest_fixture_detection(self, temp_dir):
        conftest = temp_dir / "conftest.py"
        conftest.write_text("""
import pytest

@pytest.fixture
def custom_data():
    return {"key": "value"}
""")

        test_file = temp_dir / "test_conftest_fixture.py"
        test_file.write_text("""
def test_uses_custom(custom_data):
    assert custom_data["key"] == "value"
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        node = tests["test_conftest_fixture.py::test_uses_custom"]
        assert node.is_simple is False  # Uses conftest fixture

    def test_local_fixture_detection(self, temp_dir):
        test_file = temp_dir / "test_local_fixture.py"
        test_file.write_text("""
import pytest

@pytest.fixture
def local_data():
    return [1, 2, 3]

def test_uses_local(local_data):
    assert len(local_data) == 3

def test_no_fixture():
    assert True
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        assert tests["test_local_fixture.py::test_uses_local"].is_simple is False
        assert tests["test_local_fixture.py::test_no_fixture"].is_simple is True

    def test_excludes_venv_directories(self, temp_dir):
        # Create a .venv directory with test files (should be excluded)
        venv_dir = temp_dir / ".venv" / "lib" / "site-packages"
        venv_dir.mkdir(parents=True)
        (venv_dir / "test_in_venv.py").write_text("def test_venv(): pass")

        # Create a regular test file
        (temp_dir / "test_regular.py").write_text("def test_regular(): pass")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        assert len(tests) == 1
        assert "test_regular.py::test_regular" in tests


class TestCollectTestsNative:
    """Tests for the collect_tests_native convenience function."""

    def test_basic_collection(self, temp_dir):
        test_file = temp_dir / "test_example.py"
        test_file.write_text("""
def test_one():
    assert True

def test_two():
    assert True
""")

        tests = collect_tests_native(temp_dir)

        assert len(tests) == 2
        assert all(isinstance(node, NativeTestNode) for node in tests.values())


class TestParametrizationEdgeCases:
    """Tests for edge cases in parametrized test collection."""

    def test_parametrize_with_negative_numbers(self, temp_dir):
        test_file = temp_dir / "test_negative.py"
        test_file.write_text("""
import pytest

@pytest.mark.parametrize("n", [-1, 0, 1])
def test_negative(n):
    pass
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        assert len(tests) == 3
        assert "test_negative.py::test_negative[-1]" in tests
        assert "test_negative.py::test_negative[0]" in tests
        assert "test_negative.py::test_negative[1]" in tests

    def test_parametrize_with_floats(self, temp_dir):
        test_file = temp_dir / "test_floats.py"
        test_file.write_text("""
import pytest

@pytest.mark.parametrize("f", [0.0, 1.5, -2.5])
def test_floats(f):
    pass
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        assert len(tests) == 3

    def test_parametrize_stacked(self, temp_dir):
        test_file = temp_dir / "test_stacked.py"
        test_file.write_text("""
import pytest

@pytest.mark.parametrize("x", [1, 2])
@pytest.mark.parametrize("y", ["a", "b"])
def test_stacked(x, y):
    pass
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        # Should produce 2x2 = 4 tests
        assert len(tests) == 4

    def test_parametrize_duplicate_values(self, temp_dir):
        test_file = temp_dir / "test_duplicate.py"
        test_file.write_text("""
import pytest

@pytest.mark.parametrize("val", [1, 1, 2])
def test_duplicate(val):
    pass
""")

        collector = NativeCollector(temp_dir)
        tests = collector.collect(use_cache=False)

        # Duplicates should get _0, _1 suffix
        assert len(tests) == 3
        node_ids = list(tests.keys())
        assert any("1_0" in nid for nid in node_ids)
        assert any("1_1" in nid for nid in node_ids)
