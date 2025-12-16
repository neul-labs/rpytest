"""Worker pool for parallel test execution."""

import logging
import multiprocessing
import os
import queue
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Tuple, Any

logger = logging.getLogger(__name__)


@dataclass
class TestTask:
    """A test to be executed by a worker."""
    task_id: str
    node_id: str
    repo_path: Path
    python_path: Path


@dataclass
class TestResult:
    """Result of a test execution."""
    task_id: str
    node_id: str
    outcome: str  # passed, failed, skipped, error
    duration_ms: int
    message: Optional[str] = None
    stdout: Optional[str] = None
    stderr: Optional[str] = None


class Worker:
    """A worker process that executes tests."""

    def __init__(
        self,
        worker_id: int,
        task_queue: multiprocessing.Queue,
        result_queue: multiprocessing.Queue,
    ):
        self.worker_id = worker_id
        self.task_queue = task_queue
        self.result_queue = result_queue
        self.process: Optional[multiprocessing.Process] = None
        self.running = False

    def start(self):
        """Start the worker process."""
        self.running = True
        self.process = multiprocessing.Process(
            target=self._run_loop,
            args=(self.worker_id, self.task_queue, self.result_queue),
            daemon=True,
        )
        self.process.start()
        logger.info(f"Worker {self.worker_id} started (PID: {self.process.pid})")

    def stop(self):
        """Stop the worker process."""
        self.running = False
        if self.process and self.process.is_alive():
            self.process.terminate()
            self.process.join(timeout=2)
            if self.process.is_alive():
                self.process.kill()
        logger.info(f"Worker {self.worker_id} stopped")

    def is_alive(self) -> bool:
        """Check if the worker process is alive."""
        return self.process is not None and self.process.is_alive()

    @staticmethod
    def _run_loop(
        worker_id: int,
        task_queue: multiprocessing.Queue,
        result_queue: multiprocessing.Queue,
    ):
        """Main loop for the worker process."""
        logger.info(f"Worker {worker_id} loop started")

        while True:
            try:
                # Get task with timeout to allow checking for shutdown
                try:
                    task_data = task_queue.get(timeout=1.0)
                except queue.Empty:
                    continue

                if task_data is None:
                    # Poison pill - shutdown signal
                    logger.info(f"Worker {worker_id} received shutdown signal")
                    break

                task = TestTask(**task_data)
                result = Worker._execute_test(worker_id, task)
                result_queue.put(result.__dict__)

            except Exception as e:
                logger.exception(f"Worker {worker_id} error: {e}")

        logger.info(f"Worker {worker_id} loop ended")

    @staticmethod
    def _execute_test(worker_id: int, task: TestTask) -> TestResult:
        """Execute a single test and return the result."""
        start_time = time.time()

        try:
            # Run pytest for this single test
            cmd = [
                str(task.python_path),
                "-m", "pytest",
                task.node_id,
                "-v",
                "--tb=short",
                "-q",
            ]

            result = subprocess.run(
                cmd,
                cwd=task.repo_path,
                capture_output=True,
                text=True,
                timeout=60,
            )

            duration_ms = int((time.time() - start_time) * 1000)

            # Parse outcome from output
            outcome = "error"
            if result.returncode == 0:
                outcome = "passed"
            elif "PASSED" in result.stdout:
                outcome = "passed"
            elif "FAILED" in result.stdout:
                outcome = "failed"
            elif "SKIPPED" in result.stdout:
                outcome = "skipped"
            elif "ERROR" in result.stdout:
                outcome = "error"

            return TestResult(
                task_id=task.task_id,
                node_id=task.node_id,
                outcome=outcome,
                duration_ms=duration_ms,
                stdout=result.stdout,
                stderr=result.stderr,
            )

        except subprocess.TimeoutExpired:
            duration_ms = int((time.time() - start_time) * 1000)
            return TestResult(
                task_id=task.task_id,
                node_id=task.node_id,
                outcome="error",
                duration_ms=duration_ms,
                message="Test timed out after 60 seconds",
            )

        except Exception as e:
            duration_ms = int((time.time() - start_time) * 1000)
            return TestResult(
                task_id=task.task_id,
                node_id=task.node_id,
                outcome="error",
                duration_ms=duration_ms,
                message=str(e),
            )


class WorkerPool:
    """Pool of worker processes for parallel test execution."""

    def __init__(self, num_workers: int = 0):
        """Initialize the worker pool.

        Args:
            num_workers: Number of workers. 0 means auto-detect (CPU count).
        """
        if num_workers <= 0:
            num_workers = max(1, multiprocessing.cpu_count())

        self.num_workers = num_workers
        self.workers: List[Worker] = []
        self.task_queue: multiprocessing.Queue = multiprocessing.Queue()
        self.result_queue: multiprocessing.Queue = multiprocessing.Queue()
        self.running = False
        self.task_counter = 0
        self.tests_executed = 0
        self.total_duration_ms = 0
        self._lock = threading.Lock()

    def start(self):
        """Start all workers in the pool."""
        if self.running:
            return

        logger.info(f"Starting worker pool with {self.num_workers} workers")
        self.running = True

        for i in range(self.num_workers):
            worker = Worker(i, self.task_queue, self.result_queue)
            worker.start()
            self.workers.append(worker)

    def stop(self):
        """Stop all workers in the pool."""
        if not self.running:
            return

        logger.info("Stopping worker pool")
        self.running = False

        # Send poison pills
        for _ in self.workers:
            self.task_queue.put(None)

        # Stop all workers
        for worker in self.workers:
            worker.stop()

        self.workers.clear()

    def resize(self, num_workers: int):
        """Resize the worker pool."""
        if num_workers <= 0:
            num_workers = max(1, multiprocessing.cpu_count())

        if num_workers == self.num_workers:
            return

        logger.info(f"Resizing worker pool from {self.num_workers} to {num_workers}")

        # Stop and restart with new size
        self.stop()
        self.num_workers = num_workers
        self.start()

    def submit_tests(
        self,
        node_ids: List[str],
        repo_path: Path,
        python_path: Path,
    ) -> str:
        """Submit tests to the pool for execution.

        Returns a batch ID for tracking.
        """
        batch_id = f"batch-{time.time():.0f}"

        for node_id in node_ids:
            with self._lock:
                self.task_counter += 1
                task_id = f"{batch_id}-{self.task_counter}"

            task = TestTask(
                task_id=task_id,
                node_id=node_id,
                repo_path=repo_path,
                python_path=python_path,
            )
            self.task_queue.put(task.__dict__)

        return batch_id

    def collect_results(self, count: int, timeout: float = 300.0) -> List[TestResult]:
        """Collect results from workers.

        Args:
            count: Number of results to collect.
            timeout: Maximum time to wait in seconds.

        Returns:
            List of test results.
        """
        results = []
        deadline = time.time() + timeout

        while len(results) < count and time.time() < deadline:
            try:
                remaining = deadline - time.time()
                result_data = self.result_queue.get(timeout=min(1.0, remaining))
                result = TestResult(**result_data)
                results.append(result)

                with self._lock:
                    self.tests_executed += 1
                    self.total_duration_ms += result.duration_ms

            except queue.Empty:
                continue

        return results

    def get_status(self) -> Dict[str, Any]:
        """Get pool status."""
        active = sum(1 for w in self.workers if w.is_alive())
        idle = self.num_workers - active if self.running else 0

        avg_duration = 0
        if self.tests_executed > 0:
            avg_duration = self.total_duration_ms // self.tests_executed

        return {
            "active_workers": active,
            "idle_workers": idle,
            "tests_executed": self.tests_executed,
            "avg_test_duration_ms": avg_duration,
        }

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.stop()
