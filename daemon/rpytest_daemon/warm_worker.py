"""Persistent warm workers for efficient test execution.

Workers stay alive between test runs, eliminating Python/pytest startup overhead.
Each worker maintains a warm pytest environment and receives tests via queue.
"""

import logging
import multiprocessing
import os
import queue
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional, Any

logger = logging.getLogger(__name__)


@dataclass
class TestTask:
    """A batch of tests to execute."""
    batch_id: str
    node_ids: List[str]
    repo_path: str


@dataclass
class TestResult:
    """Result of a single test."""
    node_id: str
    outcome: str
    duration_ms: int
    message: Optional[str] = None


@dataclass
class BatchResult:
    """Result of a batch execution."""
    batch_id: str
    results: List[TestResult]
    duration_ms: int


def warm_worker_loop(
    worker_id: int,
    task_queue: multiprocessing.Queue,
    result_queue: multiprocessing.Queue,
    ready_event: multiprocessing.Event,
):
    """Main loop for a warm worker process.

    The worker loads pytest once and reuses it for all test batches.
    """
    import pytest

    logger.info(f"Warm worker {worker_id} starting, loading pytest...")

    # Signal that we're ready
    ready_event.set()

    while True:
        try:
            # Wait for a task
            try:
                task_data = task_queue.get(timeout=1.0)
            except queue.Empty:
                continue

            if task_data is None:
                # Shutdown signal
                logger.info(f"Warm worker {worker_id} shutting down")
                break

            task = TestTask(**task_data)

            # Execute tests
            start_time = time.time()
            results = execute_tests_inprocess(task.node_ids, task.repo_path)
            duration_ms = int((time.time() - start_time) * 1000)

            # Send results back (convert to dicts for serialization)
            results_dicts = [
                {
                    "node_id": r.node_id,
                    "outcome": r.outcome,
                    "duration_ms": r.duration_ms,
                    "message": r.message,
                }
                for r in results
            ]
            batch_result = {
                "batch_id": task.batch_id,
                "results": results_dicts,
                "duration_ms": duration_ms,
            }
            result_queue.put(batch_result)

        except Exception as e:
            logger.exception(f"Warm worker {worker_id} error: {e}")


def execute_tests_inprocess(node_ids: List[str], repo_path: str) -> List[TestResult]:
    """Execute tests in-process using pytest."""
    import pytest

    results = []

    class ResultCollector:
        """Pytest plugin to collect results."""
        def __init__(self):
            self.results: List[TestResult] = []
            self.start_times: Dict[str, float] = {}

        def pytest_runtest_logstart(self, nodeid, location):
            self.start_times[nodeid] = time.time()

        def pytest_runtest_logreport(self, report):
            if report.when == "call" or (report.when == "setup" and report.outcome == "skipped"):
                start = self.start_times.get(report.nodeid, time.time())
                duration_ms = int((time.time() - start) * 1000)

                outcome = report.outcome
                if hasattr(report, "wasxfail"):
                    outcome = "xpass" if report.outcome == "passed" else "xfail"

                message = None
                if report.longrepr:
                    message = str(report.longrepr)[:500]  # Truncate long messages

                self.results.append(TestResult(
                    node_id=report.nodeid,
                    outcome=outcome,
                    duration_ms=duration_ms,
                    message=message,
                ))

    collector = ResultCollector()

    # Change to repo directory
    old_cwd = os.getcwd()
    try:
        os.chdir(repo_path)

        # Run pytest with minimal output
        pytest.main(
            ["--tb=short", "-q", "-x"] + node_ids,
            plugins=[collector],
        )

    finally:
        os.chdir(old_cwd)

    return collector.results


class WarmWorkerPool:
    """Pool of persistent warm workers."""

    # Class-level limits for resource protection
    MAX_QUEUE_SIZE = 1000  # Maximum tasks that can be queued
    MAX_BATCH_SIZE = 100   # Maximum tests per batch
    DEFAULT_BATCH_SIZE = 50
    RESULT_TIMEOUT_SECS = 300  # 5 minute timeout for results

    def __init__(
        self,
        num_workers: int = 0,
        max_queue_size: Optional[int] = None,
        batch_size: int = DEFAULT_BATCH_SIZE,
    ):
        if num_workers <= 0:
            num_workers = max(1, multiprocessing.cpu_count())

        self.num_workers = num_workers
        self.max_queue_size = max_queue_size or self.MAX_QUEUE_SIZE
        self.batch_size = min(batch_size, self.MAX_BATCH_SIZE)
        self.task_queue: multiprocessing.Queue = multiprocessing.Queue(maxsize=self.max_queue_size)
        self.result_queue: multiprocessing.Queue = multiprocessing.Queue()
        self.workers: List[multiprocessing.Process] = []
        self.ready_events: List[multiprocessing.Event] = []
        self.running = False
        self._batch_counter = 0
        self._pending_batches: Dict[str, bool] = {}

    def start(self):
        """Start all workers."""
        if self.running:
            return

        logger.info(f"Starting warm worker pool with {self.num_workers} workers")
        self.running = True

        for i in range(self.num_workers):
            ready_event = multiprocessing.Event()
            self.ready_events.append(ready_event)

            process = multiprocessing.Process(
                target=warm_worker_loop,
                args=(i, self.task_queue, self.result_queue, ready_event),
                daemon=True,
            )
            process.start()
            self.workers.append(process)

        # Wait for all workers to be ready
        logger.info("Waiting for workers to initialize...")
        for i, event in enumerate(self.ready_events):
            if not event.wait(timeout=30):
                logger.warning(f"Worker {i} failed to initialize in time")

        logger.info("All workers ready")

    def stop(self):
        """Stop all workers."""
        if not self.running:
            return

        logger.info("Stopping warm worker pool")
        self.running = False

        # Clear pending batches
        self._pending_batches.clear()

        # Send shutdown signals
        for _ in self.workers:
            self.task_queue.put(None)

        # Wait for workers to exit
        for process in self.workers:
            process.join(timeout=5)
            if process.is_alive():
                process.terminate()

        self.workers.clear()
        self.ready_events.clear()

        # Close queues
        self.task_queue.close()
        self.result_queue.close()

    def run_tests(
        self,
        node_ids: List[str],
        repo_path: Path,
        batch_size: Optional[int] = None,
    ) -> List[TestResult]:
        """Run tests using warm workers.

        Tests are distributed across workers in batches.

        Args:
            node_ids: Test node IDs to execute
            repo_path: Repository root path
            batch_size: Override default batch size (capped at MAX_BATCH_SIZE)

        Returns:
            List of test results
        """
        if not node_ids:
            return []

        if not self.running:
            self.start()

        # Use instance batch_size as default, allow override
        effective_batch_size = self.batch_size
        if batch_size is not None:
            effective_batch_size = min(batch_size, self.MAX_BATCH_SIZE)

        # Split into batches
        batches = [
            node_ids[i:i + effective_batch_size]
            for i in range(0, len(node_ids), effective_batch_size)
        ]

        # Submit batches and track pending
        batch_ids = []
        for batch in batches:
            self._batch_counter += 1
            batch_id = f"batch-{self._batch_counter}"
            batch_ids.append(batch_id)
            self._pending_batches[batch_id] = True

            task = TestTask(
                batch_id=batch_id,
                node_ids=batch,
                repo_path=str(repo_path),
            )
            self.task_queue.put(task.__dict__)

        # Collect results
        all_results = []
        results_received = 0
        start_time = time.time()

        while results_received < len(batches):
            # Check for timeout periodically
            remaining_timeout = self.RESULT_TIMEOUT_SECS - (time.time() - start_time)
            if remaining_timeout <= 0:
                logger.warning("Timeout waiting for batch results")
                break

            try:
                result_data = self.result_queue.get(timeout=remaining_timeout)

                # Handle both dict and already-parsed results
                if isinstance(result_data, dict):
                    batch_id = result_data.get("batch_id")
                    if batch_id in self._pending_batches:
                        del self._pending_batches[batch_id]

                    results_list = result_data.get("results", [])
                    for r in results_list:
                        if isinstance(r, dict):
                            all_results.append(TestResult(**r))
                        elif isinstance(r, TestResult):
                            all_results.append(r)
                        else:
                            # r might be a dataclass with __dict__
                            all_results.append(TestResult(**r.__dict__))
                results_received += 1
            except queue.Empty:
                logger.warning("Timeout waiting for batch results")
                break

        # Clean up any pending batches on timeout
        if self._pending_batches:
            pending_count = len(self._pending_batches)
            logger.warning(f"Cleanup: {pending_count} batches timed out and were discarded")
            self._pending_batches.clear()

        return all_results

    def get_status(self) -> Dict[str, Any]:
        """Get pool status."""
        alive = sum(1 for w in self.workers if w.is_alive())
        return {
            "num_workers": self.num_workers,
            "alive_workers": alive,
            "running": self.running,
        }

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, *args):
        self.stop()


# Global warm worker pool for reuse
_global_pool: Optional[WarmWorkerPool] = None


def get_warm_pool(num_workers: int = 0) -> WarmWorkerPool:
    """Get or create the global warm worker pool."""
    global _global_pool

    if _global_pool is None:
        _global_pool = WarmWorkerPool(num_workers)
        _global_pool.start()

    return _global_pool


def shutdown_warm_pool():
    """Shutdown the global warm worker pool."""
    global _global_pool

    if _global_pool is not None:
        _global_pool.stop()
        _global_pool = None
