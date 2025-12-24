"""Native AST-based test collection - bypasses pytest entirely.

This is 6x faster than pytest collection because it:
1. Doesn't import pytest or any test modules
2. Uses pure Python AST parsing (fast)
3. Doesn't run conftest.py files
4. Caches is_simple classification to disk
"""

import ast
import json
import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Optional, Set, Dict, Any, Tuple
import logging

logger = logging.getLogger(__name__)


@dataclass
class NativeTestNode:
    """Test node discovered via AST parsing."""
    node_id: str
    file_path: str
    name: str
    class_name: Optional[str] = None
    line_number: int = 0
    markers: List[str] = field(default_factory=list)
    is_simple: bool = True  # True if test has no fixtures/complex features
    parameters: List[str] = field(default_factory=list)  # For parameterized tests


class NativeCollector:
    """Fast test collector using AST parsing instead of pytest.

    Supports caching of is_simple classification to avoid repeated AST parsing.
    """

    def __init__(self, repo_path: Path):
        self.repo_path = repo_path
        self.tests: Dict[str, NativeTestNode] = {}
        self._conftest_fixtures: Set[str] = set()
        self._cache_dir = repo_path / ".rpytest"

    def collect(self, use_cache: bool = True) -> Dict[str, NativeTestNode]:
        """Collect all tests using AST parsing.

        Args:
            use_cache: If True, try to load from cache first.

        Returns dict of node_id -> NativeTestNode.
        """
        # Try to load from cache first
        if use_cache and self._load_cache():
            return self.tests

        self.tests.clear()

        # Find all test files
        test_files = self._find_test_files()

        # Parse conftest files first to know about fixtures
        self._parse_conftest_files()

        # Parse each test file
        for test_file in test_files:
            try:
                self._parse_test_file(test_file)
            except Exception as e:
                logger.warning(f"Failed to parse {test_file}: {e}")

        # Save to cache
        self._save_cache()

        return self.tests

    def _get_cache_path(self) -> Path:
        """Get path to the native test info cache file."""
        return self._cache_dir / "native_tests.json"

    def _is_cache_valid(self) -> bool:
        """Check if native test cache is valid."""
        cache_path = self._get_cache_path()
        if not cache_path.exists():
            return False

        try:
            cache_mtime = cache_path.stat().st_mtime

            # Check if any test files or conftest files are newer than cache
            for test_file in self._find_test_files():
                if test_file.stat().st_mtime > cache_mtime:
                    return False

            # Check conftest files
            for conftest in self.repo_path.glob('**/conftest.py'):
                if '.venv' not in str(conftest) and 'venv' not in str(conftest):
                    if conftest.stat().st_mtime > cache_mtime:
                        return False

            return True
        except Exception:
            return False

    def _load_cache(self) -> bool:
        """Load native test info from cache. Returns True if successful."""
        if not self._is_cache_valid():
            return False

        cache_path = self._get_cache_path()
        try:
            with open(cache_path) as f:
                cache_data = json.load(f)

            self.tests.clear()
            for node_id, data in cache_data.get("tests", {}).items():
                self.tests[node_id] = NativeTestNode(
                    node_id=data["node_id"],
                    file_path=data["file_path"],
                    name=data["name"],
                    class_name=data.get("class_name"),
                    line_number=data.get("line_number", 0),
                    markers=data.get("markers", []),
                    is_simple=data.get("is_simple", True),
                    parameters=data.get("parameters", []),
                )

            logger.debug(f"Loaded {len(self.tests)} native tests from cache")
            return True

        except Exception as e:
            logger.debug(f"Failed to load native cache: {e}")
            return False

    def _save_cache(self):
        """Save native test info to cache."""
        self._cache_dir.mkdir(exist_ok=True)
        cache_path = self._get_cache_path()

        cache_data = {
            "timestamp": time.time(),
            "tests": {
                node_id: {
                    "node_id": node.node_id,
                    "file_path": node.file_path,
                    "name": node.name,
                    "class_name": node.class_name,
                    "line_number": node.line_number,
                    "markers": node.markers,
                    "is_simple": node.is_simple,
                    "parameters": node.parameters,
                }
                for node_id, node in self.tests.items()
            }
        }

        try:
            with open(cache_path, "w") as f:
                json.dump(cache_data, f)
            logger.debug(f"Saved {len(self.tests)} native tests to cache")
        except Exception as e:
            logger.warning(f"Failed to save native cache: {e}")

    def _find_test_files(self) -> List[Path]:
        """Find all test files in the repo using os.walk (faster than glob)."""
        test_files = []
        exclude_dirs = {'.venv', 'venv', '__pycache__', '.git', 'node_modules', '.tox', '.pytest_cache', 'daemon'}

        import os
        for root, dirs, files in os.walk(self.repo_path):
            # Prune excluded directories
            dirs[:] = [d for d in dirs if d not in exclude_dirs]

            for filename in files:
                if (filename.startswith('test_') or filename.endswith('_test.py')) and filename.endswith('.py'):
                    test_files.append(Path(root) / filename)

        return sorted(test_files)

    def _parse_conftest_files(self):
        """Parse conftest.py files to find fixture names."""
        self._conftest_fixtures.clear()

        for conftest in self.repo_path.glob('**/conftest.py'):
            if '.venv' in str(conftest) or 'venv' in str(conftest):
                continue

            try:
                tree = ast.parse(conftest.read_text())
                for node in ast.walk(tree):
                    if isinstance(node, ast.FunctionDef):
                        # Check for @pytest.fixture decorator
                        for decorator in node.decorator_list:
                            if self._is_fixture_decorator(decorator):
                                self._conftest_fixtures.add(node.name)
                                break
            except Exception as e:
                logger.debug(f"Failed to parse conftest {conftest}: {e}")

    def _is_fixture_decorator(self, decorator: ast.expr) -> bool:
        """Check if a decorator is @pytest.fixture or @fixture."""
        if isinstance(decorator, ast.Name):
            return decorator.id == 'fixture'
        elif isinstance(decorator, ast.Attribute):
            return decorator.attr == 'fixture'
        elif isinstance(decorator, ast.Call):
            return self._is_fixture_decorator(decorator.func)
        return False

    def _parse_test_file(self, file_path: Path):
        """Parse a single test file and extract test nodes."""
        try:
            source = file_path.read_text()
            tree = ast.parse(source)
        except SyntaxError as e:
            logger.warning(f"Syntax error in {file_path}: {e}")
            return

        rel_path = str(file_path.relative_to(self.repo_path))

        # Track local fixtures
        local_fixtures: Set[str] = set()

        # First pass: find local fixtures
        for node in ast.walk(tree):
            if isinstance(node, ast.FunctionDef):
                for decorator in node.decorator_list:
                    if self._is_fixture_decorator(decorator):
                        local_fixtures.add(node.name)
                        break

        all_fixtures = self._conftest_fixtures | local_fixtures

        # Second pass: find test functions and classes
        for node in ast.iter_child_nodes(tree):
            if isinstance(node, ast.FunctionDef) and node.name.startswith('test_'):
                # Top-level test function
                test_node = self._create_test_node(
                    rel_path, node, None, all_fixtures
                )
                self.tests[test_node.node_id] = test_node

            elif isinstance(node, ast.ClassDef) and node.name.startswith('Test'):
                # Test class
                for item in node.body:
                    if isinstance(item, ast.FunctionDef) and item.name.startswith('test_'):
                        test_node = self._create_test_node(
                            rel_path, item, node.name, all_fixtures
                        )
                        self.tests[test_node.node_id] = test_node

    def _create_test_node(
        self,
        file_path: str,
        func: ast.FunctionDef,
        class_name: Optional[str],
        available_fixtures: Set[str],
    ) -> NativeTestNode:
        """Create a NativeTestNode from an AST function definition."""
        # Build node ID
        if class_name:
            node_id = f"{file_path}::{class_name}::{func.name}"
        else:
            node_id = f"{file_path}::{func.name}"

        # Extract markers from decorators
        markers = []
        is_parameterized = False
        parameters = []

        for decorator in func.decorator_list:
            marker_name = self._extract_marker_name(decorator)
            if marker_name:
                markers.append(marker_name)
                if marker_name == 'parametrize':
                    is_parameterized = True
                    # TODO: Extract parameter values

        # Determine if test is "simple" (can use fast execution path)
        # A test is simple if it:
        # 1. Has no parameters that are fixtures (except 'self')
        # 2. Is not parameterized
        # 3. Has no special markers (skip, xfail, etc.)

        func_params = [arg.arg for arg in func.args.args if arg.arg != 'self']
        uses_fixtures = any(p in available_fixtures for p in func_params)

        special_markers = {'skip', 'skipif', 'xfail', 'parametrize'}
        has_special_markers = any(m in special_markers for m in markers)

        is_simple = not uses_fixtures and not is_parameterized and not has_special_markers

        return NativeTestNode(
            node_id=node_id,
            file_path=file_path,
            name=func.name,
            class_name=class_name,
            line_number=func.lineno,
            markers=markers,
            is_simple=is_simple,
            parameters=parameters,
        )

    def _extract_marker_name(self, decorator: ast.expr) -> Optional[str]:
        """Extract marker name from a decorator."""
        if isinstance(decorator, ast.Name):
            return decorator.id

        elif isinstance(decorator, ast.Attribute):
            # @pytest.mark.skip -> 'skip'
            if isinstance(decorator.value, ast.Attribute):
                if decorator.value.attr == 'mark':
                    return decorator.attr
            return decorator.attr

        elif isinstance(decorator, ast.Call):
            return self._extract_marker_name(decorator.func)

        return None


def collect_tests_native(repo_path: Path) -> Dict[str, NativeTestNode]:
    """Convenience function to collect tests using native AST parsing."""
    collector = NativeCollector(repo_path)
    return collector.collect()
