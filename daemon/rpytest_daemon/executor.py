"""Efficient test execution using in-process pytest and batching."""

import json
import logging
import re
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Tuple, Any
from concurrent.futures import ProcessPoolExecutor, as_completed

logger = logging.getLogger(__name__)


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
class BatchResult:
    """Result of a batch of tests."""
    results: List[TestResult]
    total_duration_ms: int


def run_batch_subprocess(
    node_ids: List[str],
    repo_path: str,
    python_path: str,
    timeout: int = 300,
) -> BatchResult:
    """Run a batch of tests in a single subprocess.

    This is much more efficient than running one subprocess per test
    because Python/pytest startup cost is amortized across the batch.
    """
    start_time = time.time()

    if not node_ids:
        return BatchResult(results=[], total_duration_ms=0)

    # Run pytest with JSON output for structured results
    cmd = [
        python_path,
        "-m", "pytest",
        "--tb=short",
        "-v",
        # Use pytest's built-in result reporting
    ] + node_ids

    try:
        result = subprocess.run(
            cmd,
            cwd=repo_path,
            capture_output=True,
            text=True,
            timeout=timeout,
        )

        total_duration_ms = int((time.time() - start_time) * 1000)

        # Parse results from verbose output
        results = parse_pytest_verbose_output(result.stdout, node_ids)

        return BatchResult(
            results=results,
            total_duration_ms=total_duration_ms,
        )

    except subprocess.TimeoutExpired:
        total_duration_ms = int((time.time() - start_time) * 1000)
        # Mark all tests as error due to timeout
        results = [
            TestResult(
                node_id=nid,
                outcome="error",
                duration_ms=total_duration_ms // len(node_ids),
                message=f"Batch timed out after {timeout}s",
            )
            for nid in node_ids
        ]
        return BatchResult(results=results, total_duration_ms=total_duration_ms)

    except Exception as e:
        total_duration_ms = int((time.time() - start_time) * 1000)
        results = [
            TestResult(
                node_id=nid,
                outcome="error",
                duration_ms=total_duration_ms // len(node_ids),
                message=str(e),
            )
            for nid in node_ids
        ]
        return BatchResult(results=results, total_duration_ms=total_duration_ms)


def parse_pytest_verbose_output(stdout: str, expected_node_ids: List[str]) -> List[TestResult]:
    """Parse pytest verbose output to extract test results.

    Verbose output format:
    test_file.py::test_name PASSED
    test_file.py::TestClass::test_method FAILED
    """
    results = []
    found_node_ids = set()

    # Pattern to match test result lines
    # Matches: node_id PASSED/FAILED/SKIPPED/ERROR/XFAIL/XPASS
    pattern = re.compile(r'^(.+?)\s+(PASSED|FAILED|SKIPPED|ERROR|XFAIL|XPASS)', re.MULTILINE)

    for match in pattern.finditer(stdout):
        node_id = match.group(1).strip()
        outcome = match.group(2).lower()

        # Handle xfail/xpass outcomes
        if outcome == "xfail":
            outcome = "xfail"
        elif outcome == "xpass":
            outcome = "xpass"

        found_node_ids.add(node_id)
        results.append(TestResult(
            node_id=node_id,
            outcome=outcome,
            duration_ms=0,  # Could parse from --durations output
        ))

    # Add any missing tests as errors
    for nid in expected_node_ids:
        if nid not in found_node_ids:
            # Check if it's a partial match (parameterized tests)
            if not any(nid in fid or fid in nid for fid in found_node_ids):
                results.append(TestResult(
                    node_id=nid,
                    outcome="error",
                    duration_ms=0,
                    message="Test not found in output",
                ))

    return results


def run_inprocess_pytest(
    node_ids: List[str],
    repo_path: Path,
) -> BatchResult:
    """Run tests in-process using pytest.main().

    This is the most efficient method as it eliminates all subprocess overhead.
    However, tests share the same process so there's less isolation.
    """
    import pytest

    start_time = time.time()

    if not node_ids:
        return BatchResult(results=[], total_duration_ms=0)

    # Capture pytest output
    class ResultCollector:
        def __init__(self):
            self.results: List[TestResult] = []

        def pytest_runtest_logreport(self, report):
            if report.when == "call" or (report.when == "setup" and report.outcome == "skipped"):
                outcome = report.outcome
                if hasattr(report, "wasxfail"):
                    if report.outcome == "passed":
                        outcome = "xpass"
                    else:
                        outcome = "xfail"

                self.results.append(TestResult(
                    node_id=report.nodeid,
                    outcome=outcome,
                    duration_ms=int(report.duration * 1000),
                    message=str(report.longrepr) if report.longrepr else None,
                ))

    collector = ResultCollector()

    # Change to repo directory
    import os
    old_cwd = os.getcwd()
    try:
        os.chdir(repo_path)

        # Run pytest with our collector plugin
        pytest.main(
            ["-x", "--tb=short", "-q"] + node_ids,
            plugins=[collector],
        )

    finally:
        os.chdir(old_cwd)

    total_duration_ms = int((time.time() - start_time) * 1000)

    return BatchResult(
        results=collector.results,
        total_duration_ms=total_duration_ms,
    )


class BatchExecutor:
    """Efficient test executor using batching and parallel subprocess execution."""

    def __init__(
        self,
        repo_path: Path,
        python_path: Path,
        num_workers: int = 0,
        batch_size: int = 50,
    ):
        self.repo_path = repo_path
        self.python_path = python_path
        self.num_workers = num_workers if num_workers > 0 else (
            __import__('multiprocessing').cpu_count()
        )
        self.batch_size = batch_size

    def run_tests(
        self,
        node_ids: List[str],
        maxfail: Optional[int] = None,
    ) -> Tuple[List[TestResult], int]:
        """Run tests efficiently using batching.

        Returns (results, total_duration_ms).
        """
        if not node_ids:
            return [], 0

        start_time = time.time()

        # If small number of tests, run in single batch
        if len(node_ids) <= self.batch_size:
            batch_result = run_batch_subprocess(
                node_ids,
                str(self.repo_path),
                str(self.python_path),
            )
            return batch_result.results, batch_result.total_duration_ms

        # Split into batches
        batches = [
            node_ids[i:i + self.batch_size]
            for i in range(0, len(node_ids), self.batch_size)
        ]

        logger.info(f"Running {len(node_ids)} tests in {len(batches)} batches "
                   f"with {self.num_workers} workers")

        all_results = []
        fail_count = 0

        # Run batches in parallel using ProcessPoolExecutor
        with ProcessPoolExecutor(max_workers=self.num_workers) as executor:
            # Submit all batches
            futures = {
                executor.submit(
                    run_batch_subprocess,
                    batch,
                    str(self.repo_path),
                    str(self.python_path),
                ): batch
                for batch in batches
            }

            # Collect results as they complete
            for future in as_completed(futures):
                try:
                    batch_result = future.result()
                    all_results.extend(batch_result.results)

                    # Check for maxfail
                    for result in batch_result.results:
                        if result.outcome in ("failed", "error"):
                            fail_count += 1
                            if maxfail and fail_count >= maxfail:
                                logger.info(f"Stopping after {fail_count} failures")
                                # Cancel remaining futures
                                for f in futures:
                                    f.cancel()
                                break

                except Exception as e:
                    logger.exception(f"Batch execution failed: {e}")
                    # Mark batch tests as error
                    batch = futures[future]
                    for nid in batch:
                        all_results.append(TestResult(
                            node_id=nid,
                            outcome="error",
                            duration_ms=0,
                            message=str(e),
                        ))

        total_duration_ms = int((time.time() - start_time) * 1000)
        return all_results, total_duration_ms


class InProcessExecutor:
    """Test executor using in-process pytest (fastest, less isolation)."""

    def __init__(self, repo_path: Path):
        self.repo_path = repo_path

    def run_tests(
        self,
        node_ids: List[str],
        maxfail: Optional[int] = None,
    ) -> Tuple[List[TestResult], int]:
        """Run tests in-process using pytest.main()."""
        if not node_ids:
            return [], 0

        batch_result = run_inprocess_pytest(node_ids, self.repo_path)
        return batch_result.results, batch_result.total_duration_ms
