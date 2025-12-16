"""Repository context management."""

import hashlib
import logging
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Tuple
import json

logger = logging.getLogger(__name__)


@dataclass
class TestNode:
    """Represents a single test node."""
    node_id: str
    file_path: str
    line_number: Optional[int] = None
    markers: List[str] = field(default_factory=list)
    keywords: List[str] = field(default_factory=list)


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

    def collect(self, force: bool = False) -> Tuple[int, int]:
        """
        Collect tests using pytest --collect-only.

        Returns (node_count, duration_ms).
        """
        start_time = time.time()

        # Run pytest --collect-only with JSON output
        cmd = [
            str(self.python_path),
            "-m", "pytest",
            "--collect-only",
            "-q",
            "--no-header",
        ]

        logger.info(f"Collecting tests in {self.repo_path}")

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

                # Parse file path and line number
                file_path = node_id.split("::")[0]

                self.inventory[node_id] = TestNode(
                    node_id=node_id,
                    file_path=file_path,
                )

        # Compute inventory hash
        inventory_str = json.dumps(sorted(self.inventory.keys()))
        self.inventory_hash = hashlib.sha256(inventory_str.encode()).hexdigest()[:16]
        self.last_collection_time = time.time()

        duration_ms = int((time.time() - start_time) * 1000)
        logger.info(f"Collected {len(self.inventory)} tests in {duration_ms}ms")

        return len(self.inventory), duration_ms

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
    ) -> RunSummary:
        """
        Run specified tests using pytest.

        Returns a RunSummary with results.
        """
        start_time = time.time()

        if not node_ids:
            return RunSummary(
                total=0, passed=0, failed=0, skipped=0, errors=0,
                duration_ms=0,
            )

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

        logger.info(f"Running {len(node_ids)} tests")

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
        # This is a simplified parser - a real implementation would use
        # pytest's JSON report or hook into pytest directly
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
            f"Run complete: {total} tests, "
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


class ContextRegistry:
    """Registry of repository contexts."""

    def __init__(self):
        self._contexts: Dict[str, RepoContext] = {}
        self._counter: int = 0

    def create_context(
        self,
        repo_path: str,
        python_path: Optional[str] = None,
    ) -> RepoContext:
        """Create a new repository context."""
        self._counter += 1
        context_id = f"ctx-{self._counter:04d}"

        path = Path(repo_path).resolve()
        if not path.exists():
            raise ValueError(f"Repository path does not exist: {repo_path}")

        py_path = Path(python_path) if python_path else None

        context = RepoContext(
            context_id=context_id,
            repo_path=path,
            python_path=py_path,
        )

        self._contexts[context_id] = context
        logger.info(f"Created context {context_id} for {path}")

        return context

    def get_context(self, context_id: str) -> Optional[RepoContext]:
        """Get a context by ID."""
        return self._contexts.get(context_id)

    def remove_context(self, context_id: str) -> bool:
        """Remove a context by ID."""
        if context_id in self._contexts:
            del self._contexts[context_id]
            logger.info(f"Removed context {context_id}")
            return True
        return False

    def list_contexts(self) -> List[str]:
        """List all context IDs."""
        return list(self._contexts.keys())

    def clear(self):
        """Remove all contexts."""
        self._contexts.clear()
        logger.info("Cleared all contexts")
