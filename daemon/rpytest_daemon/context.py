"""Repository context management."""

import hashlib
import json
import logging
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Tuple, Any

from .worker import WorkerPool, TestResult as WorkerTestResult
from .scheduler import TestScheduler
from .streaming import StreamingRun, StreamingRunManager, execute_streaming_run
from .flakiness import FlakinessTracker, determine_final_outcome, RerunResult
from .fixtures import SessionFixtureManager, FixtureConfig
from .sharding import TestSharder, ShardConfig
from .executor import BatchExecutor, InProcessExecutor, TestResult as ExecutorTestResult
from .warm_worker import get_warm_pool, WarmWorkerPool, TestResult as WarmTestResult
from .native_collector import NativeCollector, NativeTestNode
from .direct_executor import DirectExecutor, DirectTestResult

logger = logging.getLogger(__name__)


@dataclass
class TestNode:
    """Represents a single test node."""
    node_id: str
    file_path: str
    name: str = ""
    class_name: Optional[str] = None
    line_number: Optional[int] = None
    markers: List[str] = field(default_factory=list)
    keywords: List[str] = field(default_factory=list)
    skip: bool = False
    xfail: bool = False

    def to_node_info(self) -> dict:
        """Convert to TestNodeInfo dict format."""
        return {
            "node_id": self.node_id,
            "file_path": self.file_path,
            "lineno": self.line_number,
            "name": self.name,
            "class_name": self.class_name,
            "markers": self.markers,
            "skip": self.skip,
            "xfail": self.xfail,
        }


@dataclass
class TestResult:
    """Result of a single test execution."""
    node_id: str
    outcome: str  # passed, failed, skipped, error, xfail, xpass
    duration_ms: int
    message: Optional[str] = None
    stdout: Optional[str] = None
    stderr: Optional[str] = None


@dataclass
class RunSummary:
    """Summary of a test run."""
    total: int
    passed: int
    failed: int
    skipped: int
    errors: int
    duration_ms: int
    results: List[TestResult] = field(default_factory=list)


@dataclass
class RerunConfig:
    """Configuration for auto-rerun behavior."""
    enabled: bool = False
    max_reruns: int = 2
    only_flaky: bool = False  # Only rerun known flaky tests
    delay_ms: int = 0  # Delay between reruns

    def to_dict(self) -> Dict:
        """Serialize to dict."""
        return {
            "enabled": self.enabled,
            "max_reruns": self.max_reruns,
            "only_flaky": self.only_flaky,
            "delay_ms": self.delay_ms,
        }

    @classmethod
    def from_dict(cls, data: Dict) -> "RerunConfig":
        """Deserialize from dict."""
        return cls(
            enabled=data.get("enabled", False),
            max_reruns=data.get("max_reruns", 2),
            only_flaky=data.get("only_flaky", False),
            delay_ms=data.get("delay_ms", 0),
        )


class RepoContext:
    """Execution context for a single repository."""

    def __init__(self, context_id: str, repo_path: Path, python_path: Optional[Path] = None):
        self.context_id = context_id
        self.repo_path = repo_path
        self.python_path = python_path or Path(sys.executable)
        self.inventory: Dict[str, TestNode] = {}
        self.inventory_hash: str = ""
        self.last_collection_time: float = 0
        self.duration_history: Dict[str, List[int]] = {}  # node_id -> list of durations
        self.outcome_history: Dict[str, List[str]] = {}   # node_id -> list of outcomes
        self._worker_pool: Optional[WorkerPool] = None
        self._use_parallel: bool = True  # Enable parallel execution by default
        self._scheduler: TestScheduler = TestScheduler()
        self._streaming_manager: StreamingRunManager = StreamingRunManager()

        # Phase 5: Advanced features
        storage_dir = repo_path / ".rpytest"
        self._flakiness_tracker = FlakinessTracker(
            storage_path=storage_dir / "flakiness.json"
        )
        self._fixture_manager = SessionFixtureManager()
        self._fixture_config = FixtureConfig()
        self._sharder = TestSharder()
        self._rerun_config = RerunConfig()  # Auto-rerun settings

        # Native collection and direct execution
        self._native_collector: Optional[NativeCollector] = None
        self._direct_executor: Optional[DirectExecutor] = None
        self._native_tests: Dict[str, NativeTestNode] = {}  # Cache of native test info
        self._use_native: bool = True  # Enable native collection/execution

    def collect(self, force: bool = False) -> Tuple[int, int]:
        """
        Collect tests using cached inventory or in-process pytest collection.

        Optimization priority:
        1. If cache is valid AND inventory loaded -> return immediately
        2. If cache is valid but inventory not loaded -> load from disk
        3. Otherwise -> run pytest collection

        Returns (node_count, duration_ms).
        """
        start_time = time.time()

        # Fast path: cache valid and inventory already in memory
        if not force and self.inventory and self._is_cache_valid():
            duration_ms = int((time.time() - start_time) * 1000)
            logger.debug(f"Using in-memory inventory: {len(self.inventory)} tests ({duration_ms}ms)")
            return len(self.inventory), duration_ms

        # Medium path: load from disk cache
        if not force and self._is_cache_valid():
            if self._load_inventory_cache():
                # Also populate native test info for hybrid execution
                if self._use_native and not self._native_tests:
                    self._populate_native_test_info()
                duration_ms = int((time.time() - start_time) * 1000)
                logger.info(f"Loaded inventory from cache: {len(self.inventory)} tests ({duration_ms}ms)")
                return len(self.inventory), duration_ms

        logger.info(f"Collecting tests in {self.repo_path}")

        # Try native collection first (fastest)
        if self._use_native:
            try:
                count, duration = self._collect_native()
                self._update_cache_state()
                return count, duration
            except Exception as e:
                logger.warning(f"Native collection failed, falling back to pytest: {e}")

        # Fall back to pytest collection
        try:
            count, duration = self._collect_inprocess()
            self._update_cache_state()
            return count, duration
        except Exception as e:
            logger.warning(f"In-process collection failed, falling back to subprocess: {e}")
            result = self._collect_subprocess()
            self._update_cache_state()
            return result

    def _collect_native(self) -> Tuple[int, int]:
        """Collect tests using native AST parsing (fastest).

        This bypasses pytest entirely for collection, providing ~6x speedup.
        """
        start_time = time.time()

        if self._native_collector is None:
            self._native_collector = NativeCollector(self.repo_path)

        native_tests = self._native_collector.collect()
        self._native_tests = native_tests

        # Convert to standard inventory format
        self.inventory.clear()
        for node_id, native_node in native_tests.items():
            self.inventory[node_id] = TestNode(
                node_id=native_node.node_id,
                file_path=native_node.file_path,
                name=native_node.name,
                class_name=native_node.class_name,
                line_number=native_node.line_number,
                markers=native_node.markers,
            )

        # Compute inventory hash
        inventory_str = json.dumps(sorted(self.inventory.keys()))
        self.inventory_hash = hashlib.sha256(inventory_str.encode()).hexdigest()[:16]
        self.last_collection_time = time.time()

        duration_ms = int((time.time() - start_time) * 1000)
        logger.info(f"Native collected {len(self.inventory)} tests in {duration_ms}ms")

        return len(self.inventory), duration_ms

    def _populate_native_test_info(self):
        """Populate native test info for hybrid execution.

        Called when loading from cache to enable direct execution of simple tests.
        """
        if self._native_collector is None:
            self._native_collector = NativeCollector(self.repo_path)

        # Run native collection to get is_simple classification
        native_tests = self._native_collector.collect()
        self._native_tests = native_tests

        simple_count = sum(1 for t in native_tests.values() if t.is_simple)
        logger.debug(f"Native test info: {simple_count} simple, {len(native_tests) - simple_count} complex")

    def _is_cache_valid(self) -> bool:
        """Check if the cached inventory is still valid.

        Uses fast validation by checking directory mtime first, then
        only checking individual files if needed.
        """
        cache_file = self.repo_path / ".rpytest" / "cache_state.json"
        if not cache_file.exists():
            return False

        try:
            cache_mtime = cache_file.stat().st_mtime

            # Fast check: if test directories haven't been modified since cache,
            # we can skip individual file checks
            test_dirs = set()
            for pattern in ["**/test_*.py", "**/*_test.py"]:
                for path in self.repo_path.glob(pattern):
                    test_dirs.add(path.parent)

            dirs_unchanged = True
            for test_dir in test_dirs:
                if test_dir.stat().st_mtime > cache_mtime:
                    dirs_unchanged = False
                    break

            if dirs_unchanged and self.inventory:
                # Directories unchanged and we have inventory - cache is valid
                return True

            # Need to do detailed check
            with open(cache_file) as f:
                cache_state = json.load(f)

            # Check file modification times
            for file_path, mtime in cache_state.get("file_mtimes", {}).items():
                full_path = self.repo_path / file_path
                if full_path.exists():
                    if full_path.stat().st_mtime > mtime:
                        logger.debug(f"File changed: {file_path}")
                        return False
                else:
                    # File was deleted
                    return False

            # Check for new test files
            current_files = set(self._find_test_files())
            cached_files = set(cache_state.get("file_mtimes", {}).keys())
            if current_files != cached_files:
                logger.debug("Test file set changed")
                return False

            return True

        except Exception as e:
            logger.debug(f"Cache validation failed: {e}")
            return False

    def _update_cache_state(self):
        """Update the cache state after collection."""
        cache_dir = self.repo_path / ".rpytest"
        cache_dir.mkdir(exist_ok=True)

        # Record file modification times
        file_mtimes = {}
        for file_path in self._find_test_files():
            full_path = self.repo_path / file_path
            if full_path.exists():
                file_mtimes[file_path] = full_path.stat().st_mtime

        cache_state = {
            "inventory_hash": self.inventory_hash,
            "file_mtimes": file_mtimes,
            "timestamp": time.time(),
        }

        cache_file = cache_dir / "cache_state.json"
        with open(cache_file, "w") as f:
            json.dump(cache_state, f)

        # Also save inventory to disk for fast loading
        self._save_inventory_cache()

    def _save_inventory_cache(self):
        """Save inventory to disk for fast loading."""
        cache_dir = self.repo_path / ".rpytest"
        cache_dir.mkdir(exist_ok=True)

        inventory_cache = {
            "inventory_hash": self.inventory_hash,
            "last_collection_time": self.last_collection_time,
            "nodes": {
                node_id: {
                    "node_id": node.node_id,
                    "file_path": node.file_path,
                    "name": node.name,
                    "class_name": node.class_name,
                    "line_number": node.line_number,
                    "markers": node.markers,
                    "keywords": node.keywords,
                    "skip": node.skip,
                    "xfail": node.xfail,
                }
                for node_id, node in self.inventory.items()
            }
        }

        cache_file = cache_dir / "inventory_cache.json"
        with open(cache_file, "w") as f:
            json.dump(inventory_cache, f)

    def _load_inventory_cache(self) -> bool:
        """Load inventory from disk cache. Returns True if successful."""
        cache_file = self.repo_path / ".rpytest" / "inventory_cache.json"
        if not cache_file.exists():
            return False

        try:
            with open(cache_file) as f:
                cache = json.load(f)

            self.inventory_hash = cache.get("inventory_hash", "")
            self.last_collection_time = cache.get("last_collection_time", 0)

            self.inventory.clear()
            for node_id, node_data in cache.get("nodes", {}).items():
                self.inventory[node_id] = TestNode(
                    node_id=node_data["node_id"],
                    file_path=node_data["file_path"],
                    name=node_data.get("name", ""),
                    class_name=node_data.get("class_name"),
                    line_number=node_data.get("line_number"),
                    markers=node_data.get("markers", []),
                    keywords=node_data.get("keywords", []),
                    skip=node_data.get("skip", False),
                    xfail=node_data.get("xfail", False),
                )

            logger.debug(f"Loaded {len(self.inventory)} tests from cache")
            return True

        except Exception as e:
            logger.debug(f"Failed to load inventory cache: {e}")
            return False

    def _find_test_files(self) -> List[str]:
        """Find all test files in the repo."""
        test_files = []
        for pattern in ["**/test_*.py", "**/*_test.py"]:
            for path in self.repo_path.glob(pattern):
                # Skip venv and other common exclusions
                path_str = str(path.relative_to(self.repo_path))
                if not any(excl in path_str for excl in [".venv", "venv", "__pycache__", ".git", "node_modules"]):
                    test_files.append(path_str)
        return sorted(test_files)

    def _collect_inprocess(self) -> Tuple[int, int]:
        """Collect tests using in-process pytest (fast)."""
        import pytest
        import os

        start_time = time.time()

        class CollectorPlugin:
            """Plugin to collect test items."""
            def __init__(self):
                self.items = []

            def pytest_collection_modifyitems(self, items):
                self.items = items

        collector = CollectorPlugin()

        # Change to repo directory
        old_cwd = os.getcwd()
        try:
            os.chdir(self.repo_path)

            # Run pytest in collection-only mode
            pytest.main(
                ["--collect-only", "-q", "--ignore=daemon", "--ignore=.venv"],
                plugins=[collector],
            )

        finally:
            os.chdir(old_cwd)

        # Parse collected items
        self.inventory.clear()
        for item in collector.items:
            node_id = item.nodeid
            node = self._parse_node_id(node_id)

            # Extract markers
            markers = [m.name for m in item.iter_markers()]
            node.markers = markers

            # Extract line number if available
            if hasattr(item, 'location') and item.location:
                node.line_number = item.location[1]

            self.inventory[node_id] = node

        # Compute inventory hash
        inventory_str = json.dumps(sorted(self.inventory.keys()))
        self.inventory_hash = hashlib.sha256(inventory_str.encode()).hexdigest()[:16]
        self.last_collection_time = time.time()

        duration_ms = int((time.time() - start_time) * 1000)
        logger.info(f"Collected {len(self.inventory)} tests in {duration_ms}ms (in-process)")

        return len(self.inventory), duration_ms

    def _collect_subprocess(self) -> Tuple[int, int]:
        """Collect tests using subprocess (fallback)."""
        start_time = time.time()

        # Run pytest --collect-only with quiet output
        cmd = [
            str(self.python_path),
            "-m", "pytest",
            "--collect-only",
            "-q",
            "--no-header",
        ]

        try:
            result = subprocess.run(
                cmd,
                cwd=self.repo_path,
                capture_output=True,
                text=True,
                timeout=60,
            )
        except subprocess.TimeoutExpired:
            logger.error("Collection timed out")
            raise RuntimeError("Collection timed out after 60 seconds")
        except Exception as e:
            logger.error(f"Collection failed: {e}")
            raise

        # Parse collected tests from stdout
        # Format: path/to/test.py::test_name
        self.inventory.clear()

        for line in result.stdout.strip().split("\n"):
            line = line.strip()
            if not line or line.startswith("=") or line.startswith("-"):
                continue
            if "::" in line:
                # This is a node ID
                node_id = line.split()[0] if " " in line else line
                node = self._parse_node_id(node_id)
                self.inventory[node_id] = node

        # Compute inventory hash
        inventory_str = json.dumps(sorted(self.inventory.keys()))
        self.inventory_hash = hashlib.sha256(inventory_str.encode()).hexdigest()[:16]
        self.last_collection_time = time.time()

        duration_ms = int((time.time() - start_time) * 1000)
        logger.info(f"Collected {len(self.inventory)} tests in {duration_ms}ms (subprocess)")

        return len(self.inventory), duration_ms

    def _parse_node_id(self, node_id: str) -> TestNode:
        """Parse a node ID into a TestNode with metadata."""
        # Handle parametrized tests: test_foo[param] -> test_foo
        base_id = node_id
        if "[" in node_id:
            base_id = node_id[:node_id.rfind("[")]

        parts = base_id.split("::")
        file_path = parts[0] if parts else ""

        # Extract name and class
        name = ""
        class_name = None

        if len(parts) >= 2:
            if len(parts) == 2:
                # file.py::test_name
                name = parts[1]
            elif len(parts) >= 3:
                # file.py::Class::test_name
                class_name = parts[1]
                name = parts[2]

        # Build keywords from components
        keywords = [name]
        if class_name:
            keywords.append(class_name)
        if file_path:
            # Add file stem without extension
            stem = file_path.rsplit("/", 1)[-1]
            if stem.endswith(".py"):
                stem = stem[:-3]
            keywords.append(stem)

        return TestNode(
            node_id=node_id,
            file_path=file_path,
            name=name,
            class_name=class_name,
            keywords=keywords,
        )

    def get_inventory_nodes(self) -> List[dict]:
        """Get all inventory nodes as TestNodeInfo dicts."""
        return [node.to_node_info() for node in self.inventory.values()]

    def _ensure_worker_pool(self, num_workers: int = 0) -> WorkerPool:
        """Ensure worker pool is initialized and running."""
        if self._worker_pool is None:
            self._worker_pool = WorkerPool(num_workers)
            self._worker_pool.start()
            logger.info(f"Started worker pool with {self._worker_pool.num_workers} workers")
        return self._worker_pool

    def configure_workers(self, num_workers: int) -> int:
        """Configure the number of workers in the pool."""
        if self._worker_pool is None:
            self._worker_pool = WorkerPool(num_workers)
            self._worker_pool.start()
        else:
            self._worker_pool.resize(num_workers)
        logger.info(f"Worker pool configured to {self._worker_pool.num_workers} workers")
        return self._worker_pool.num_workers

    def get_worker_status(self) -> Dict[str, Any]:
        """Get worker pool status."""
        if self._worker_pool is None:
            return {
                "active_workers": 0,
                "idle_workers": 0,
                "tests_executed": 0,
                "avg_test_duration_ms": 0,
            }
        return self._worker_pool.get_status()

    def shutdown_workers(self):
        """Shutdown the worker pool."""
        if self._worker_pool is not None:
            self._worker_pool.stop()
            self._worker_pool = None
            logger.info("Worker pool shutdown")

    def list_tests(
        self,
        keyword: Optional[str] = None,
        marker: Optional[str] = None,
    ) -> List[str]:
        """
        List test node IDs, optionally filtered by keyword or marker.

        Note: Full keyword/marker filtering requires parsing expressions.
        For Phase 1, we do simple substring matching.
        """
        node_ids = list(self.inventory.keys())

        if keyword:
            # Simple substring match for now
            keyword_lower = keyword.lower()
            node_ids = [
                nid for nid in node_ids
                if keyword_lower in nid.lower()
            ]

        # Marker filtering would require collecting marker info
        # which needs more complex pytest integration

        return node_ids

    def run_tests(
        self,
        node_ids: List[str],
        maxfail: Optional[int] = None,
        num_workers: Optional[int] = None,
        failed_first: bool = False,
    ) -> RunSummary:
        """
        Run specified tests using pytest.

        Args:
            node_ids: List of test node IDs to run.
            maxfail: Stop after N failures.
            num_workers: Number of parallel workers (None = auto, 1 = sequential).
            failed_first: If True, run recently failed tests first.

        Returns a RunSummary with results.
        """
        start_time = time.time()

        if not node_ids:
            return RunSummary(
                total=0, passed=0, failed=0, skipped=0, errors=0,
                duration_ms=0,
            )

        # Get recent failures if needed
        recent_failures = []
        if failed_first:
            recent_failures = [
                nid for nid, outcomes in self.outcome_history.items()
                if outcomes and outcomes[-1] in ("failed", "error")
            ]

        # Schedule tests for optimal execution order
        scheduled_ids = self._scheduler.schedule(
            node_ids,
            failed_first=failed_first,
            recent_failures=recent_failures,
        )

        # Decide execution mode
        use_parallel = self._use_parallel and (num_workers is None or num_workers != 1)

        if use_parallel and len(scheduled_ids) > 1:
            return self._run_parallel(scheduled_ids, maxfail, num_workers, start_time)
        else:
            return self._run_sequential(scheduled_ids, maxfail, start_time)

    def _run_parallel(
        self,
        node_ids: List[str],
        maxfail: Optional[int],
        num_workers: Optional[int],
        start_time: float,
    ) -> RunSummary:
        """Run tests using hybrid execution: direct for simple tests, pytest for complex.

        This provides maximum performance by:
        1. Running simple tests directly (no pytest overhead)
        2. Running complex tests (fixtures, params) via warm workers
        """
        # Partition tests into simple (direct execution) and complex (pytest)
        simple_tests = []
        complex_tests = []

        if self._use_native and self._native_tests:
            for node_id in node_ids:
                native_node = self._native_tests.get(node_id)
                if native_node and native_node.is_simple:
                    simple_tests.append(node_id)
                else:
                    complex_tests.append(node_id)
        else:
            # No native test info - run all via pytest
            complex_tests = node_ids

        logger.info(
            f"Hybrid execution: {len(simple_tests)} simple (direct), "
            f"{len(complex_tests)} complex (pytest)"
        )

        passed = 0
        failed = 0
        skipped = 0
        errors = 0
        results = []

        # Execute simple tests directly (ultra-fast path)
        if simple_tests:
            if self._direct_executor is None:
                self._direct_executor = DirectExecutor(self.repo_path)

            direct_results = self._direct_executor.execute_tests(simple_tests)

            for dr in direct_results:
                if dr.outcome == "passed":
                    passed += 1
                elif dr.outcome == "failed":
                    failed += 1
                else:
                    errors += 1

                results.append(TestResult(
                    node_id=dr.node_id,
                    outcome=dr.outcome,
                    duration_ms=dr.duration_ms,
                    message=dr.message,
                ))
                self._update_history(dr.node_id, dr.outcome, dr.duration_ms)

        # Execute complex tests via warm workers
        if complex_tests:
            pool = get_warm_pool(num_workers or 0)
            warm_results = pool.run_tests(complex_tests, self.repo_path, batch_size=50)

            for wr in warm_results:
                outcome = wr.outcome
                if outcome == "passed":
                    passed += 1
                elif outcome == "failed":
                    failed += 1
                elif outcome == "skipped":
                    skipped += 1
                elif outcome in ("xfail", "xpass"):
                    if outcome == "xfail":
                        skipped += 1
                    else:
                        failed += 1
                else:
                    errors += 1

                results.append(TestResult(
                    node_id=wr.node_id,
                    outcome=outcome,
                    duration_ms=wr.duration_ms,
                    message=wr.message,
                ))
                self._update_history(wr.node_id, outcome, wr.duration_ms)

        duration_ms = int((time.time() - start_time) * 1000)
        total = passed + failed + skipped + errors

        logger.info(
            f"Hybrid execution complete: {total} tests, "
            f"{passed} passed, {failed} failed, "
            f"{skipped} skipped, {errors} errors "
            f"in {duration_ms}ms"
        )

        return RunSummary(
            total=total,
            passed=passed,
            failed=failed,
            skipped=skipped,
            errors=errors,
            duration_ms=duration_ms,
            results=results,
        )

    def _run_sequential(
        self,
        node_ids: List[str],
        maxfail: Optional[int],
        start_time: float,
    ) -> RunSummary:
        """Run tests sequentially using subprocess."""
        # Build pytest command
        cmd = [
            str(self.python_path),
            "-m", "pytest",
            "-v",
            "--tb=short",
        ]

        if maxfail:
            cmd.extend(["--maxfail", str(maxfail)])

        # Add node IDs
        cmd.extend(node_ids)

        logger.info(f"Running {len(node_ids)} tests sequentially")

        try:
            result = subprocess.run(
                cmd,
                cwd=self.repo_path,
                capture_output=True,
                text=True,
                timeout=300,  # 5 minute timeout
            )
        except subprocess.TimeoutExpired:
            logger.error("Test run timed out")
            raise RuntimeError("Test run timed out after 300 seconds")
        except Exception as e:
            logger.error(f"Test run failed: {e}")
            raise

        # Parse results from output
        passed = 0
        failed = 0
        skipped = 0
        errors = 0
        results = []

        for line in result.stdout.split("\n"):
            line = line.strip()
            if " PASSED" in line:
                passed += 1
                node_id = line.split(" PASSED")[0].strip()
                results.append(TestResult(
                    node_id=node_id,
                    outcome="passed",
                    duration_ms=0,
                ))
            elif " FAILED" in line:
                failed += 1
                node_id = line.split(" FAILED")[0].strip()
                results.append(TestResult(
                    node_id=node_id,
                    outcome="failed",
                    duration_ms=0,
                ))
            elif " SKIPPED" in line:
                skipped += 1
                node_id = line.split(" SKIPPED")[0].strip()
                results.append(TestResult(
                    node_id=node_id,
                    outcome="skipped",
                    duration_ms=0,
                ))
            elif " ERROR" in line:
                errors += 1
                node_id = line.split(" ERROR")[0].strip()
                results.append(TestResult(
                    node_id=node_id,
                    outcome="error",
                    duration_ms=0,
                ))

        duration_ms = int((time.time() - start_time) * 1000)
        total = passed + failed + skipped + errors

        logger.info(
            f"Sequential run complete: {total} tests, "
            f"{passed} passed, {failed} failed, "
            f"{skipped} skipped, {errors} errors "
            f"in {duration_ms}ms"
        )

        return RunSummary(
            total=total,
            passed=passed,
            failed=failed,
            skipped=skipped,
            errors=errors,
            duration_ms=duration_ms,
            results=results,
        )

    def _update_history(self, node_id: str, outcome: str, duration_ms: int):
        """Update outcome and duration history for a test."""
        # Duration history (keep last 10)
        if node_id not in self.duration_history:
            self.duration_history[node_id] = []
        self.duration_history[node_id].append(duration_ms)
        if len(self.duration_history[node_id]) > 10:
            self.duration_history[node_id] = self.duration_history[node_id][-10:]

        # Outcome history (keep last 10)
        if node_id not in self.outcome_history:
            self.outcome_history[node_id] = []
        self.outcome_history[node_id].append(outcome)
        if len(self.outcome_history[node_id]) > 10:
            self.outcome_history[node_id] = self.outcome_history[node_id][-10:]

        # Update scheduler's duration tracking
        self._scheduler.update_duration(node_id, duration_ms)

    def get_estimated_duration(self, node_id: str) -> int:
        """Get estimated duration for a test based on history."""
        return self._scheduler.get_estimated_duration(node_id)

    def estimate_run_time(self, node_ids: List[str], num_workers: int) -> Tuple[int, int]:
        """Estimate total run time for a set of tests.

        Args:
            node_ids: Tests to estimate.
            num_workers: Number of parallel workers.

        Returns:
            Tuple of (estimated_wall_time_ms, estimated_cpu_time_ms)
        """
        return self._scheduler.estimate_total_duration(node_ids, num_workers)

    def start_streaming_run(
        self,
        node_ids: List[str],
        num_workers: Optional[int] = None,
        maxfail: Optional[int] = None,
    ) -> StreamingRun:
        """Start a streaming test run.

        Args:
            node_ids: Tests to run.
            num_workers: Number of parallel workers (None = auto).
            maxfail: Stop after N failures.

        Returns:
            StreamingRun object for polling progress.
        """
        # Schedule tests for optimal order
        scheduled_ids = self._scheduler.schedule(node_ids)

        # Start the run
        run = self._streaming_manager.start_run(
            self.context_id,
            scheduled_ids,
            maxfail,
        )

        # Ensure worker pool is started
        pool = self._ensure_worker_pool(num_workers or 0)

        # Execute in background thread
        thread = threading.Thread(
            target=execute_streaming_run,
            args=(run, pool, self.repo_path, self.python_path, self._update_history),
            daemon=True,
        )
        thread.start()

        return run

    def get_streaming_run(self, run_id: str) -> Optional[StreamingRun]:
        """Get a streaming run by ID."""
        return self._streaming_manager.get_run(run_id)

    # --- Phase 5: Flakiness Detection ---

    def configure_rerun(
        self,
        enabled: bool = True,
        max_reruns: int = 2,
        only_flaky: bool = False,
        delay_ms: int = 0,
    ):
        """Configure auto-rerun behavior."""
        self._rerun_config = RerunConfig(
            enabled=enabled,
            max_reruns=max_reruns,
            only_flaky=only_flaky,
            delay_ms=delay_ms,
        )
        logger.info(f"Configured rerun: {self._rerun_config}")

    def get_rerun_config(self) -> Dict:
        """Get current rerun configuration."""
        return self._rerun_config.to_dict()

    def get_flaky_tests(self) -> List[str]:
        """Get list of tests currently considered flaky."""
        return self._flakiness_tracker.get_flaky_tests()

    def get_flakiness_report(self) -> Dict:
        """Get flakiness report for all tracked tests."""
        return self._flakiness_tracker.get_flakiness_report()

    def get_test_flakiness(self, node_id: str) -> Optional[Dict]:
        """Get flakiness info for a specific test."""
        record = self._flakiness_tracker.get_record(node_id)
        if not record:
            return None
        return {
            "node_id": record.node_id,
            "failure_rate": record.failure_rate,
            "is_flaky": record.is_flaky,
            "flaky_streak": record.flaky_streak,
            "consecutive_failures": record.consecutive_failures,
            "consecutive_passes": record.consecutive_passes,
            "total_runs": record.total_runs,
            "recent_outcomes": record.outcomes[-10:],
        }

    def run_tests_with_rerun(
        self,
        node_ids: List[str],
        maxfail: Optional[int] = None,
        num_workers: Optional[int] = None,
    ) -> RunSummary:
        """Run tests with automatic rerun of failures.

        Uses flakiness tracking to intelligently rerun failed tests.
        """
        if not self._rerun_config.enabled:
            return self.run_tests(node_ids, maxfail, num_workers)

        # First run
        summary = self.run_tests(node_ids, maxfail, num_workers)

        # Collect failures for potential rerun
        failures = [
            r for r in summary.results
            if r.outcome in ("failed", "error")
        ]

        if not failures:
            return summary

        # Determine which tests to rerun
        to_rerun = []
        for result in failures:
            should_rerun, reason = self._flakiness_tracker.should_rerun(
                result.node_id,
                result.outcome,
                self._rerun_config.max_reruns,
            )
            if should_rerun:
                if not self._rerun_config.only_flaky or reason == "known_flaky":
                    to_rerun.append(result.node_id)

        if not to_rerun:
            return summary

        logger.info(f"Rerunning {len(to_rerun)} failed tests")

        # Rerun with delay if configured
        if self._rerun_config.delay_ms > 0:
            time.sleep(self._rerun_config.delay_ms / 1000)

        # Run reruns (single worker for isolation)
        rerun_results: Dict[str, List[str]] = {nid: [] for nid in to_rerun}
        final_outcomes: Dict[str, str] = {}

        for attempt in range(self._rerun_config.max_reruns):
            still_failing = [
                nid for nid in to_rerun
                if nid not in final_outcomes or final_outcomes[nid] != "passed"
            ]

            if not still_failing:
                break

            rerun_summary = self.run_tests(still_failing, maxfail=None, num_workers=1)

            for result in rerun_summary.results:
                rerun_results[result.node_id].append(result.outcome)
                if result.outcome == "passed":
                    final_outcomes[result.node_id] = "passed"

            if self._rerun_config.delay_ms > 0 and attempt < self._rerun_config.max_reruns - 1:
                time.sleep(self._rerun_config.delay_ms / 1000)

        # Update summary with final outcomes
        passed_on_rerun = 0
        for result in summary.results:
            if result.node_id in final_outcomes:
                if result.outcome != "passed" and final_outcomes[result.node_id] == "passed":
                    result.outcome = "passed"
                    result.message = f"[FLAKY] Passed on rerun ({len(rerun_results[result.node_id])} attempts)"
                    passed_on_rerun += 1

        # Adjust counts
        summary.passed += passed_on_rerun
        summary.failed -= passed_on_rerun

        return summary

    # --- Phase 5: Session Fixture Reuse ---

    def configure_fixture_reuse(
        self,
        enabled: bool = True,
        max_age_seconds: float = 600,
        teardown_on_conftest_change: bool = True,
    ):
        """Configure session fixture reuse."""
        self._fixture_config = FixtureConfig(
            enabled=enabled,
            max_fixture_age_seconds=max_age_seconds,
            teardown_on_conftest_change=teardown_on_conftest_change,
        )

        if enabled:
            self._fixture_manager.create_session(
                self.context_id,
                self.repo_path,
                self.python_path,
            )
            self._fixture_manager.enable_reuse(self.context_id)
            logger.info(f"Enabled fixture reuse for {self.context_id}")
        else:
            self._fixture_manager.disable_reuse(self.context_id)
            logger.info(f"Disabled fixture reuse for {self.context_id}")

    def get_fixture_config(self) -> Dict:
        """Get current fixture configuration."""
        return self._fixture_config.to_dict()

    def get_session_status(self) -> Optional[Dict]:
        """Get session fixture status."""
        return self._fixture_manager.get_session_status(self.context_id)

    def invalidate_fixtures(self, changed_files: List[str]) -> List[str]:
        """Invalidate fixtures based on file changes."""
        paths = [Path(f) for f in changed_files]
        return self._fixture_manager.invalidate_on_file_change(
            self.context_id,
            paths,
        )

    # --- Phase 5: Sharding Support ---

    def shard_tests(
        self,
        node_ids: List[str],
        shard_index: int,
        total_shards: int,
        strategy: str = "duration_balanced",
    ) -> List[str]:
        """Get tests for a specific shard.

        Args:
            node_ids: All test node IDs.
            shard_index: This shard's index (0-based).
            total_shards: Total number of shards.
            strategy: Sharding strategy (hash, round_robin, duration_balanced).

        Returns:
            List of node IDs assigned to this shard.
        """
        # Update sharder with current duration estimates
        self._sharder.duration_estimates = {
            nid: self._scheduler.get_estimated_duration(nid)
            for nid in node_ids
        }

        config = ShardConfig(
            shard_index=shard_index,
            total_shards=total_shards,
            strategy=strategy,
        )

        return self._sharder.get_shard(node_ids, config)

    def get_shard_info(
        self,
        node_ids: List[str],
        total_shards: int,
        strategy: str = "duration_balanced",
    ) -> Dict:
        """Get sharding distribution info.

        Returns detailed info about how tests would be distributed.
        """
        self._sharder.duration_estimates = {
            nid: self._scheduler.get_estimated_duration(nid)
            for nid in node_ids
        }

        return self._sharder.get_balance_report(node_ids, total_shards, strategy)


class ContextRegistry:
    """Registry of repository contexts.

    Supports context persistence - reuses existing contexts for the same
    repo path to avoid repeated collection overhead.
    """

    def __init__(self):
        self._contexts: Dict[str, RepoContext] = {}
        self._contexts_by_path: Dict[str, str] = {}  # repo_path -> context_id
        self._counter: int = 0

    def create_context(
        self,
        repo_path: str,
        python_path: Optional[str] = None,
    ) -> RepoContext:
        """Get or create a repository context.

        If a context already exists for this repo path, returns it.
        Otherwise creates a new one.
        """
        path = Path(repo_path).resolve()
        if not path.exists():
            raise ValueError(f"Repository path does not exist: {repo_path}")

        path_str = str(path)

        # Check if we already have a context for this path
        if path_str in self._contexts_by_path:
            existing_id = self._contexts_by_path[path_str]
            if existing_id in self._contexts:
                logger.debug(f"Reusing existing context {existing_id} for {path}")
                return self._contexts[existing_id]

        # Create new context
        self._counter += 1
        context_id = f"ctx-{self._counter:04d}"

        py_path = Path(python_path) if python_path else None

        context = RepoContext(
            context_id=context_id,
            repo_path=path,
            python_path=py_path,
        )

        self._contexts[context_id] = context
        self._contexts_by_path[path_str] = context_id
        logger.info(f"Created context {context_id} for {path}")

        return context

    def get_context(self, context_id: str) -> Optional[RepoContext]:
        """Get a context by ID."""
        return self._contexts.get(context_id)

    def remove_context(self, context_id: str) -> bool:
        """Remove a context by ID."""
        if context_id in self._contexts:
            context = self._contexts[context_id]
            context.shutdown_workers()  # Clean up worker pool
            del self._contexts[context_id]
            logger.info(f"Removed context {context_id}")
            return True
        return False

    def list_contexts(self) -> List[str]:
        """List all context IDs."""
        return list(self._contexts.keys())

    def clear(self):
        """Remove all contexts."""
        for context in self._contexts.values():
            context.shutdown_workers()
        self._contexts.clear()
        logger.info("Cleared all contexts")
