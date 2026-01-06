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
from typing import List, Optional, Set, Dict, Any, Tuple, Union
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

    # Built-in pytest fixtures that require pytest to execute
    BUILTIN_FIXTURES = {
        # Capture fixtures
        'capsys', 'capfd', 'capsysbinary', 'capfdbinary', 'caplog',
        # Temp/path fixtures
        'tmp_path', 'tmp_path_factory', 'tmpdir', 'tmpdir_factory',
        # Request/config fixtures
        'request', 'pytestconfig', 'cache',
        # Warning fixtures
        'recwarn',
        # Monkeypatch
        'monkeypatch',
        # Doctest
        'doctest_namespace',
    }

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
        except Exception as e:
            logger.debug(f"Cache validation failed (will rebuild): {e}")
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

        # First pass: find local fixtures and build class hierarchy
        class_methods: Dict[str, List[Union[ast.FunctionDef, ast.AsyncFunctionDef]]] = {}
        class_bases: Dict[str, List[str]] = {}
        # Track classes that have setup/teardown methods (can't use direct execution)
        classes_with_setup: Set[str] = set()
        setup_teardown_names = {'setup_method', 'teardown_method', 'setup_class', 'teardown_class', 'setup', 'teardown'}

        for node in ast.iter_child_nodes(tree):
            if isinstance(node, ast.FunctionDef):
                for decorator in node.decorator_list:
                    if self._is_fixture_decorator(decorator):
                        local_fixtures.add(node.name)
                        break
            elif isinstance(node, ast.ClassDef):
                # Track class methods
                class_methods[node.name] = []
                for item in node.body:
                    if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) and item.name.startswith('test_'):
                        class_methods[node.name].append(item)
                    elif isinstance(item, ast.FunctionDef):
                        # Check for fixtures in class
                        for decorator in item.decorator_list:
                            if self._is_fixture_decorator(decorator):
                                local_fixtures.add(item.name)
                                break
                        # Check for setup/teardown methods
                        if item.name in setup_teardown_names:
                            classes_with_setup.add(node.name)

                # Track base classes (only simple names within same file)
                class_bases[node.name] = []
                for base in node.bases:
                    if isinstance(base, ast.Name):
                        class_bases[node.name].append(base.id)

        all_fixtures = self._conftest_fixtures | local_fixtures | self.BUILTIN_FIXTURES

        # Second pass: find test functions and classes with inherited methods
        for node in ast.iter_child_nodes(tree):
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith('test_'):
                # Top-level test function (sync or async)
                test_nodes = self._create_test_nodes(
                    rel_path, node, None, all_fixtures, False
                )
                for test_node in test_nodes:
                    self.tests[test_node.node_id] = test_node

            elif isinstance(node, ast.ClassDef) and node.name.startswith('Test'):
                # Test class - collect direct and inherited methods
                seen_methods: Set[str] = set()
                has_setup = node.name in classes_with_setup

                # Get inherited methods from base classes
                inherited_methods = self._get_inherited_methods(
                    node.name, class_methods, class_bases, seen_methods
                )

                # Add inherited methods
                for method in inherited_methods:
                    if method.name not in seen_methods:
                        seen_methods.add(method.name)
                        test_nodes = self._create_test_nodes(
                            rel_path, method, node.name, all_fixtures, has_setup
                        )
                        for test_node in test_nodes:
                            self.tests[test_node.node_id] = test_node

                # Add direct methods (may override inherited)
                for item in node.body:
                    if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) and item.name.startswith('test_'):
                        if item.name not in seen_methods:
                            seen_methods.add(item.name)
                            test_nodes = self._create_test_nodes(
                                rel_path, item, node.name, all_fixtures, has_setup
                            )
                            for test_node in test_nodes:
                                self.tests[test_node.node_id] = test_node

    def _get_inherited_methods(
        self,
        class_name: str,
        class_methods: Dict[str, List[Union[ast.FunctionDef, ast.AsyncFunctionDef]]],
        class_bases: Dict[str, List[str]],
        seen_methods: Set[str],
        visited: Optional[Set[str]] = None,
    ) -> List[Union[ast.FunctionDef, ast.AsyncFunctionDef]]:
        """Get test methods inherited from base classes."""
        if visited is None:
            visited = set()

        if class_name in visited:
            return []  # Prevent infinite recursion
        visited.add(class_name)

        inherited = []
        bases = class_bases.get(class_name, [])

        for base_name in bases:
            # Recursively get methods from parent classes
            inherited.extend(
                self._get_inherited_methods(base_name, class_methods, class_bases, seen_methods, visited)
            )
            # Add methods from this base class
            if base_name in class_methods:
                for method in class_methods[base_name]:
                    if method.name not in seen_methods:
                        inherited.append(method)

        return inherited

    def _create_test_nodes(
        self,
        file_path: str,
        func: Union[ast.FunctionDef, ast.AsyncFunctionDef],
        class_name: Optional[str],
        available_fixtures: Set[str],
        class_has_setup: bool = False,
    ) -> List[NativeTestNode]:
        """Create NativeTestNode(s) from an AST function definition.

        Returns a list because parametrized tests expand to multiple nodes.

        Args:
            file_path: Relative path to the test file.
            func: AST node for the test function.
            class_name: Name of the test class, if any.
            available_fixtures: Set of known fixture names.
            class_has_setup: True if the class has setup/teardown methods.
        """
        # Build base node ID
        if class_name:
            base_node_id = f"{file_path}::{class_name}::{func.name}"
        else:
            base_node_id = f"{file_path}::{func.name}"

        # Extract markers and parametrize info from decorators
        markers = []
        param_sets = []  # List of lists - each parametrize decorator adds param values

        for decorator in func.decorator_list:
            marker_name = self._extract_marker_name(decorator)
            if marker_name:
                markers.append(marker_name)
                if marker_name == 'parametrize':
                    # Extract parameter values
                    param_values = self._extract_parametrize_values(decorator)
                    if param_values:
                        param_sets.append(param_values)

        # Determine if test is "simple" (can use fast execution path)
        func_params = [arg.arg for arg in func.args.args if arg.arg != 'self']
        uses_fixtures = any(p in available_fixtures for p in func_params)

        special_markers = {'skip', 'skipif', 'xfail', 'parametrize', 'asyncio'}
        has_special_markers = any(m in special_markers for m in markers)

        # Async tests are never simple - they require pytest-asyncio
        is_async = isinstance(func, ast.AsyncFunctionDef)

        # Tests in classes with setup/teardown require pytest to run those methods
        is_simple = (
            not uses_fixtures
            and not param_sets
            and not has_special_markers
            and not is_async
            and not class_has_setup
        )

        # If no parametrization, return single node
        if not param_sets:
            return [NativeTestNode(
                node_id=base_node_id,
                file_path=file_path,
                name=func.name,
                class_name=class_name,
                line_number=func.lineno,
                markers=markers,
                is_simple=is_simple,
                parameters=[],
            )]

        # Expand parametrized tests
        # For stacked @parametrize decorators, compute cartesian product
        # NOTE: pytest processes decorators bottom-to-top, so we need to reverse
        from itertools import product
        if len(param_sets) == 1:
            all_combos = [(p,) for p in param_sets[0]]
        else:
            # Reverse param_sets to match pytest's bottom-to-top order
            reversed_sets = list(reversed(param_sets))
            all_combos = list(product(*reversed_sets))

        nodes = []

        # Detect if all values in single param_set are lists or dicts
        # This affects whether we use lst/dct prefix or value prefix
        all_lists = False
        all_dicts = False
        if len(param_sets) == 1:
            values = param_sets[0]
            all_lists = all(isinstance(v, list) for v in values)
            all_dicts = all(isinstance(v, dict) for v in values)

        # For non-stacked parametrize, pre-compute IDs to handle duplicates properly
        # pytest uses _0, _1 suffixes from the FIRST occurrence if there are duplicates
        if len(param_sets) == 1:
            # First pass: generate all IDs and detect duplicates
            raw_ids = []
            for idx, val in enumerate(param_sets[0]):
                raw_id = self._format_single_param_value(val, idx, all_lists, all_dicts)
                raw_ids.append(raw_id)

            # Count occurrences
            from collections import Counter
            id_occurrences = Counter(raw_ids)

            # Second pass: add _0, _1 etc. suffix for duplicates
            id_seen: Dict[str, int] = {}
            final_ids = []
            for raw_id in raw_ids:
                if id_occurrences[raw_id] > 1:
                    # Has duplicates - add suffix
                    suffix = id_seen.get(raw_id, 0)
                    id_seen[raw_id] = suffix + 1
                    final_ids.append(f"{raw_id}_{suffix}")
                else:
                    final_ids.append(raw_id)

            # Create nodes
            for idx, param_id in enumerate(final_ids):
                node_id = f"{base_node_id}[{param_id}]"
                nodes.append(NativeTestNode(
                    node_id=node_id,
                    file_path=file_path,
                    name=func.name,
                    class_name=class_name,
                    line_number=func.lineno,
                    markers=markers,
                    is_simple=False,
                    parameters=[param_sets[0][idx]],
                ))
        else:
            # Stacked parametrize - use cartesian product
            # Track string IDs to handle duplicates
            id_counts: Dict[str, int] = {}

            for combo in all_combos:
                param_strs = []
                for val in combo:
                    param_strs.append(self._format_param_value_simple(val))
                param_id = "-".join(param_strs)

                # Handle duplicate IDs
                if param_id in id_counts:
                    id_counts[param_id] += 1
                    unique_id = f"{param_id}_{id_counts[param_id]}"
                else:
                    id_counts[param_id] = 0
                    unique_id = param_id

                node_id = f"{base_node_id}[{unique_id}]"

                nodes.append(NativeTestNode(
                    node_id=node_id,
                    file_path=file_path,
                    name=func.name,
                    class_name=class_name,
                    line_number=func.lineno,
                    markers=markers,
                    is_simple=False,
                    parameters=list(combo),
                ))

        return nodes

    def _format_single_param_value(
        self, value: Any, index: int, all_lists: bool, all_dicts: bool
    ) -> str:
        """Format a single parameter value for test ID.

        Args:
            value: The parameter value.
            index: Position in the parameter list (for valueN naming).
            all_lists: True if all params are lists (use lstN).
            all_dicts: True if all params are dicts (use dctN).
        """
        if value is None:
            return "None"
        elif isinstance(value, bool):
            return str(value)
        elif isinstance(value, (int, float)):
            if value == 0:
                if isinstance(value, float):
                    if str(value).startswith('-'):
                        return "-0.0"
                    return "0.0"
                return "0"
            return str(value)
        elif isinstance(value, str):
            if value == '\t':
                return '\\t'
            elif value == '\n':
                return '\\n'
            elif value == '\r':
                return '\\r'
            return value
        elif isinstance(value, tuple):
            # Tuples are formatted as their contents joined with dashes
            parts = [self._format_param_value_simple(v) for v in value]
            return "-".join(parts)
        elif isinstance(value, list):
            if all_lists:
                return f"lst{index}"
            else:
                return f"value{index}"
        elif isinstance(value, dict):
            if all_dicts:
                return f"dct{index}"
            else:
                return f"value{index}"
        else:
            return f"value{index}"

    def _extract_parametrize_values(self, decorator: ast.expr) -> List[Any]:
        """Extract parameter values from @pytest.mark.parametrize decorator."""
        if not isinstance(decorator, ast.Call):
            return []

        # parametrize(argnames, argvalues, ...)
        # We need the second argument (argvalues)
        if len(decorator.args) < 2:
            return []

        argvalues = decorator.args[1]

        # Handle list of values: [1, 2, 3]
        if isinstance(argvalues, ast.List):
            return [self._ast_to_value(elt) for elt in argvalues.elts]

        # Handle tuple of values: (1, 2, 3)
        if isinstance(argvalues, ast.Tuple):
            return [self._ast_to_value(elt) for elt in argvalues.elts]

        return []

    def _ast_to_value(self, node: ast.expr) -> Any:
        """Convert an AST node to a Python value for use in test IDs."""
        if isinstance(node, ast.Constant):
            return node.value
        elif isinstance(node, ast.Num):  # Python 3.7 compat
            return node.n
        elif isinstance(node, ast.Str):  # Python 3.7 compat
            return node.s
        elif isinstance(node, ast.NameConstant):  # Python 3.7 compat
            return node.value
        elif isinstance(node, ast.Tuple):
            return tuple(self._ast_to_value(elt) for elt in node.elts)
        elif isinstance(node, ast.List):
            return [self._ast_to_value(elt) for elt in node.elts]
        elif isinstance(node, ast.Dict):
            # Return a dict representation
            keys = [self._ast_to_value(k) if k else None for k in node.keys]
            values = [self._ast_to_value(v) for v in node.values]
            return dict(zip(keys, values))
        elif isinstance(node, ast.Name):
            # Handle special names
            if node.id == 'True':
                return True
            elif node.id == 'False':
                return False
            elif node.id == 'None':
                return None
            return node.id  # Variable reference, use name as string
        elif isinstance(node, ast.Call):
            # Handle pytest.param() calls
            func = node.func
            if isinstance(func, ast.Attribute) and func.attr == 'param':
                # Check for id= keyword argument first
                for kw in node.keywords:
                    if kw.arg == 'id':
                        id_value = self._ast_to_value(kw.value)
                        if isinstance(id_value, str):
                            return id_value
                # Fall back to using the value
                if node.args:
                    return self._ast_to_value(node.args[0])
            # For other calls, just use a placeholder
            return "call"
        elif isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
            # Handle negative numbers: -1, -0.5, etc.
            val = self._ast_to_value(node.operand)
            if isinstance(val, (int, float)):
                return -val
            return f"-{val}"
        else:
            # For complex expressions, use a generic representation
            return "..."

    def _format_param_value_with_index(
        self, value: Any, type_counters: Dict[str, int]
    ) -> str:
        """Format a parameter value for use in test ID, using indices for complex types.

        Args:
            value: The parameter value to format.
            type_counters: Dict tracking next index for each type (lst, dct, value).

        Returns:
            Formatted string suitable for use in pytest test ID.
        """
        if value is None:
            return "None"
        elif isinstance(value, bool):
            # Must check bool before int since bool is subclass of int
            return str(value)
        elif isinstance(value, (int, float)):
            # Handle negative zero
            if value == 0:
                if isinstance(value, float):
                    # -0.0 vs 0.0
                    if str(value).startswith('-'):
                        return "-0.0"
                    return "0.0"
                return "0"
            return str(value)
        elif isinstance(value, str):
            # Escape special whitespace characters like pytest does
            if value == '\t':
                return '\\t'
            elif value == '\n':
                return '\\n'
            elif value == '\r':
                return '\\r'
            # Empty string and other strings are kept as-is
            return value
        elif isinstance(value, tuple):
            # Tuples are formatted as their contents joined with dashes
            parts = [self._format_param_value_simple(v) for v in value]
            return "-".join(parts)
        elif isinstance(value, list):
            # Check if this is a standalone list parametrize (all lists)
            # vs mixed types. For mixed types, use valueN.
            # We use 'lst' prefix only when counters show it's a list-only param
            if type_counters.get('_all_lists', False):
                idx = type_counters['lst']
                type_counters['lst'] += 1
                return f"lst{idx}"
            else:
                # Use valueN for mixed type scenarios
                idx = type_counters['value']
                type_counters['value'] += 1
                return f"value{idx}"
        elif isinstance(value, dict):
            # Check if this is a dict-only parametrize
            if type_counters.get('_all_dicts', False):
                idx = type_counters['dct']
                type_counters['dct'] += 1
                return f"dct{idx}"
            else:
                idx = type_counters['value']
                type_counters['value'] += 1
                return f"value{idx}"
        else:
            # Other complex objects use valueN format
            idx = type_counters['value']
            type_counters['value'] += 1
            return f"value{idx}"

    def _format_param_value_simple(self, value: Any) -> str:
        """Format a parameter value simply (for tuple contents, stacked params, etc.)."""
        if value is None:
            return "None"
        elif isinstance(value, bool):
            return str(value)
        elif isinstance(value, (int, float)):
            if value == 0:
                if isinstance(value, float):
                    if str(value).startswith('-'):
                        return "-0.0"
                    return "0.0"
                return "0"
            return str(value)
        elif isinstance(value, str):
            if value == '\t':
                return '\\t'
            elif value == '\n':
                return '\\n'
            elif value == '\r':
                return '\\r'
            return value
        else:
            return str(value)

    def _format_param_value(self, value: Any) -> str:
        """Format a parameter value for use in test ID (legacy, non-indexed)."""
        if value is None:
            return "None"
        elif isinstance(value, bool):
            return str(value)
        elif isinstance(value, (int, float)):
            return str(value)
        elif isinstance(value, str):
            return value
        elif isinstance(value, tuple):
            return "-".join(self._format_param_value(v) for v in value)
        elif isinstance(value, list):
            return "-".join(self._format_param_value(v) for v in value)
        else:
            return str(value)

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
