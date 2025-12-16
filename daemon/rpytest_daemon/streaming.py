"""Streaming run management for real-time test result delivery."""

import logging
import threading
import time
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Any
from queue import Queue, Empty

from .worker import WorkerPool, TestResult

logger = logging.getLogger(__name__)


@dataclass
class StreamingRun:
    """A streaming test run that can be polled for progress."""
    run_id: str
    context_id: str
    node_ids: List[str]
    total: int
    completed: int = 0
    running: int = 0
    done: bool = False
    failed_count: int = 0
    maxfail: Optional[int] = None

    # Results that haven't been delivered yet
    pending_results: List[TestResult] = field(default_factory=list)

    # All results for summary
    all_results: List[TestResult] = field(default_factory=list)

    # Lock for thread safety
    _lock: threading.Lock = field(default_factory=threading.Lock)

    def add_result(self, result: TestResult):
        """Add a completed test result."""
        with self._lock:
            self.completed += 1
            self.running = max(0, self.running - 1)
            self.pending_results.append(result)
            self.all_results.append(result)

            if result.outcome in ("failed", "error"):
                self.failed_count += 1

                # Check maxfail
                if self.maxfail and self.failed_count >= self.maxfail:
                    self.done = True

            # Check if all tests complete
            if self.completed >= self.total:
                self.done = True

    def get_pending_results(self) -> List[TestResult]:
        """Get and clear pending results."""
        with self._lock:
            results = self.pending_results
            self.pending_results = []
            return results

    def mark_running(self, count: int):
        """Mark tests as currently running."""
        with self._lock:
            self.running = count


class StreamingRunManager:
    """Manages streaming test runs."""

    def __init__(self):
        self._runs: Dict[str, StreamingRun] = {}
        self._lock = threading.Lock()

    def start_run(
        self,
        context_id: str,
        node_ids: List[str],
        maxfail: Optional[int] = None,
    ) -> StreamingRun:
        """Start a new streaming run."""
        run_id = f"run-{uuid.uuid4().hex[:8]}"

        run = StreamingRun(
            run_id=run_id,
            context_id=context_id,
            node_ids=node_ids,
            total=len(node_ids),
            maxfail=maxfail,
        )

        with self._lock:
            self._runs[run_id] = run

        logger.info(f"Started streaming run {run_id} with {len(node_ids)} tests")
        return run

    def get_run(self, run_id: str) -> Optional[StreamingRun]:
        """Get a run by ID."""
        with self._lock:
            return self._runs.get(run_id)

    def remove_run(self, run_id: str):
        """Remove a completed run."""
        with self._lock:
            if run_id in self._runs:
                del self._runs[run_id]
                logger.info(f"Removed streaming run {run_id}")

    def cleanup_old_runs(self, max_age_seconds: float = 3600):
        """Clean up old completed runs."""
        # For now, just remove completed runs
        # In production, would track timestamps
        with self._lock:
            to_remove = [
                run_id for run_id, run in self._runs.items()
                if run.done
            ]
            for run_id in to_remove:
                del self._runs[run_id]
            if to_remove:
                logger.info(f"Cleaned up {len(to_remove)} completed runs")


def execute_streaming_run(
    run: StreamingRun,
    pool: WorkerPool,
    repo_path: Path,
    python_path: Path,
    update_history_callback: Optional[callable] = None,
):
    """Execute a streaming run using the worker pool.

    This runs in a background thread and updates the run state as
    results come in.
    """
    try:
        # Submit all tests
        pool.submit_tests(run.node_ids, repo_path, python_path)
        run.mark_running(min(pool.num_workers, len(run.node_ids)))

        # Collect results as they complete
        remaining = len(run.node_ids)
        while remaining > 0 and not run.done:
            results = pool.collect_results(1, timeout=1.0)

            for result in results:
                run.add_result(result)
                remaining -= 1

                # Update history if callback provided
                if update_history_callback:
                    update_history_callback(
                        result.node_id,
                        result.outcome,
                        result.duration_ms,
                    )

                # Update running count
                run.mark_running(min(pool.num_workers, remaining))

        # Mark as done
        run.done = True
        logger.info(f"Streaming run {run.run_id} complete: {run.completed} tests")

    except Exception as e:
        logger.exception(f"Error in streaming run {run.run_id}: {e}")
        run.done = True
