"""Direct test execution - bypasses pytest for simple tests.

For tests without fixtures, this executes them directly by:
1. Importing the test module (with parallel pre-loading)
2. Calling the test function directly
3. Catching AssertionError for failures

This is ~100-1000x faster than pytest for simple tests.
"""

import importlib.util
import sys
import time
import traceback
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path
from typing import List, Dict, Optional, Tuple, Any
import logging

logger = logging.getLogger(__name__)


@dataclass
class DirectTestResult:
    """Result of directly executing a test."""
    node_id: str
    outcome: str  # passed, failed, error
    duration_ms: int
    message: Optional[str] = None


class DirectExecutor:
    """Execute simple tests directly without pytest overhead."""

    def __init__(self, repo_path: Path, max_workers: int = 8):
        self.repo_path = repo_path
        self.max_workers = max_workers
        self._module_cache: Dict[str, Any] = {}
        self._executor: Optional[ThreadPoolExecutor] = None

    def _get_executor(self) -> ThreadPoolExecutor:
        """Get or create the thread pool executor."""
        if self._executor is None:
            self._executor = ThreadPoolExecutor(max_workers=self.max_workers)
        return self._executor

    def execute_tests(self, node_ids: List[str], parallel: bool = True) -> List[DirectTestResult]:
        """Execute a list of tests directly.

        Args:
            node_ids: List of test node IDs to execute.
            parallel: If True, execute tests in parallel using thread pool.

        Returns:
            List of DirectTestResult objects.
        """
        # Group tests by module for efficiency
        tests_by_module: Dict[str, List[Tuple[str, str, Optional[str]]]] = {}
        for node_id in node_ids:
            file_path, class_name, test_name = self._parse_node_id(node_id)
            if file_path not in tests_by_module:
                tests_by_module[file_path] = []
            tests_by_module[file_path].append((node_id, test_name, class_name))

        # Pre-load all modules in parallel (helps with I/O-bound imports)
        modules_to_load = [fp for fp in tests_by_module.keys() if fp not in self._module_cache]
        if modules_to_load:
            self._preload_modules_parallel(modules_to_load)

        if parallel and len(node_ids) > 1:
            return self._execute_parallel(tests_by_module)
        else:
            return self._execute_sequential(tests_by_module)

    def _execute_sequential(self, tests_by_module: Dict[str, List[Tuple[str, str, Optional[str]]]]) -> List[DirectTestResult]:
        """Execute tests sequentially (original behavior)."""
        results = []
        for file_path, tests in tests_by_module.items():
            module = self._load_module(file_path)
            if module is None:
                for node_id, _, _ in tests:
                    results.append(DirectTestResult(
                        node_id=node_id,
                        outcome='error',
                        duration_ms=0,
                        message=f"Failed to load module: {file_path}",
                    ))
                continue

            for node_id, test_name, class_name in tests:
                result = self._execute_single_test(module, node_id, test_name, class_name)
                results.append(result)
        return results

    def _execute_parallel(self, tests_by_module: Dict[str, List[Tuple[str, str, Optional[str]]]]) -> List[DirectTestResult]:
        """Execute tests in parallel using thread pool."""
        results: List[DirectTestResult] = []
        futures_map: Dict[Any, str] = {}  # future -> node_id

        executor = self._get_executor()

        # Submit all tests to the thread pool
        for file_path, tests in tests_by_module.items():
            module = self._load_module(file_path)
            if module is None:
                for node_id, _, _ in tests:
                    results.append(DirectTestResult(
                        node_id=node_id,
                        outcome='error',
                        duration_ms=0,
                        message=f"Failed to load module: {file_path}",
                    ))
                continue

            for node_id, test_name, class_name in tests:
                future = executor.submit(
                    self._execute_single_test,
                    module, node_id, test_name, class_name
                )
                futures_map[future] = node_id

        # Collect results as they complete
        for future in as_completed(futures_map):
            try:
                result = future.result()
                results.append(result)
            except Exception as e:
                node_id = futures_map[future]
                results.append(DirectTestResult(
                    node_id=node_id,
                    outcome='error',
                    duration_ms=0,
                    message=f"Execution error: {e}",
                ))

        return results

    def _preload_modules_parallel(self, file_paths: List[str], max_workers: int = 4):
        """Pre-load multiple modules in parallel.

        This helps speed up initial module loading by parallelizing I/O.
        """
        if not file_paths:
            return

        # Use ThreadPoolExecutor for I/O-bound module loading
        with ThreadPoolExecutor(max_workers=max_workers) as executor:
            futures = {
                executor.submit(self._load_module, fp): fp
                for fp in file_paths
            }
            for future in as_completed(futures):
                file_path = futures[future]
                try:
                    future.result()
                except Exception as e:
                    logger.debug(f"Parallel load failed for {file_path}: {e}")

    def _parse_node_id(self, node_id: str) -> Tuple[str, Optional[str], str]:
        """Parse node ID into (file_path, class_name, test_name)."""
        parts = node_id.split('::')
        file_path = parts[0]

        if len(parts) == 2:
            # file.py::test_name
            return file_path, None, parts[1]
        elif len(parts) == 3:
            # file.py::ClassName::test_name
            return file_path, parts[1], parts[2]
        else:
            raise ValueError(f"Invalid node ID: {node_id}")

    def _load_module(self, file_path: str) -> Optional[Any]:
        """Load a Python module from file path."""
        if file_path in self._module_cache:
            return self._module_cache[file_path]

        full_path = self.repo_path / file_path
        if not full_path.exists():
            logger.error(f"Test file not found: {full_path}")
            return None

        try:
            # Generate a unique module name
            module_name = f"rpytest_direct_{file_path.replace('/', '_').replace('.py', '')}"

            # Load the module
            spec = importlib.util.spec_from_file_location(module_name, full_path)
            if spec is None or spec.loader is None:
                return None

            module = importlib.util.module_from_spec(spec)

            # Add repo path to sys.path temporarily if needed
            if str(self.repo_path) not in sys.path:
                sys.path.insert(0, str(self.repo_path))

            spec.loader.exec_module(module)
            self._module_cache[file_path] = module
            return module

        except Exception as e:
            logger.error(f"Failed to load module {file_path}: {e}")
            return None

    def _execute_single_test(
        self,
        module: Any,
        node_id: str,
        test_name: str,
        class_name: Optional[str],
    ) -> DirectTestResult:
        """Execute a single test function."""
        start_time = time.perf_counter()

        try:
            if class_name:
                # Get the class and instantiate it
                test_class = getattr(module, class_name, None)
                if test_class is None:
                    return DirectTestResult(
                        node_id=node_id,
                        outcome='error',
                        duration_ms=0,
                        message=f"Class not found: {class_name}",
                    )

                # Create instance and get method
                instance = test_class()
                test_func = getattr(instance, test_name, None)
            else:
                # Get the function directly
                test_func = getattr(module, test_name, None)

            if test_func is None:
                return DirectTestResult(
                    node_id=node_id,
                    outcome='error',
                    duration_ms=0,
                    message=f"Test function not found: {test_name}",
                )

            # Execute the test
            test_func()

            duration_ms = int((time.perf_counter() - start_time) * 1000)
            return DirectTestResult(
                node_id=node_id,
                outcome='passed',
                duration_ms=duration_ms,
            )

        except AssertionError as e:
            duration_ms = int((time.perf_counter() - start_time) * 1000)
            return DirectTestResult(
                node_id=node_id,
                outcome='failed',
                duration_ms=duration_ms,
                message=str(e) or "Assertion failed",
            )

        except Exception as e:
            duration_ms = int((time.perf_counter() - start_time) * 1000)
            return DirectTestResult(
                node_id=node_id,
                outcome='error',
                duration_ms=duration_ms,
                message=f"{type(e).__name__}: {e}",
            )

    def clear_cache(self):
        """Clear the module cache."""
        self._module_cache.clear()

    def shutdown(self):
        """Shutdown the thread pool executor."""
        if self._executor is not None:
            self._executor.shutdown(wait=False)
            self._executor = None


class HybridExecutor:
    """Hybrid executor that uses direct execution for simple tests, pytest for complex ones."""

    def __init__(self, repo_path: Path):
        self.repo_path = repo_path
        self._direct_executor = DirectExecutor(repo_path)

    def execute_tests(
        self,
        simple_tests: List[str],
        complex_tests: List[str],
    ) -> Tuple[List[DirectTestResult], List[str]]:
        """Execute tests using hybrid approach.

        Args:
            simple_tests: Tests to run directly (no fixtures).
            complex_tests: Tests to run via pytest (have fixtures).

        Returns:
            Tuple of (direct_results, complex_tests_to_run_via_pytest).
        """
        # Execute simple tests directly
        direct_results = []
        if simple_tests:
            logger.info(f"Executing {len(simple_tests)} simple tests directly")
            direct_results = self._direct_executor.execute_tests(simple_tests)

        # Return complex tests for pytest execution
        return direct_results, complex_tests
