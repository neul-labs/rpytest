"""Test scheduler for load balancing across workers."""

import logging
from dataclasses import dataclass
from typing import Dict, List, Optional, Tuple

logger = logging.getLogger(__name__)


@dataclass
class ScheduledTest:
    """A test with scheduling metadata."""
    node_id: str
    estimated_duration_ms: int
    priority: int = 0  # Higher = scheduled earlier


class TestScheduler:
    """Scheduler for ordering tests based on duration estimates.

    Strategy: Schedule longest tests first (LPT - Longest Processing Time).
    This helps minimize total execution time by ensuring slow tests
    start early while faster tests fill in gaps.
    """

    def __init__(self):
        self._duration_history: Dict[str, List[int]] = {}
        self._default_duration_ms: int = 1000  # Default 1 second estimate

    def update_duration(self, node_id: str, duration_ms: int):
        """Update duration history for a test."""
        if node_id not in self._duration_history:
            self._duration_history[node_id] = []

        self._duration_history[node_id].append(duration_ms)

        # Keep only last 10 runs
        if len(self._duration_history[node_id]) > 10:
            self._duration_history[node_id] = self._duration_history[node_id][-10:]

    def get_estimated_duration(self, node_id: str) -> int:
        """Get estimated duration for a test."""
        if node_id in self._duration_history and self._duration_history[node_id]:
            # Use exponential moving average favoring recent runs
            durations = self._duration_history[node_id]
            if len(durations) == 1:
                return durations[0]

            # Weight recent runs more heavily
            weights = [0.5 ** i for i in range(len(durations))]
            weights.reverse()  # Most recent gets highest weight
            total_weight = sum(weights)
            weighted_sum = sum(d * w for d, w in zip(durations, weights))
            return int(weighted_sum / total_weight)

        return self._default_duration_ms

    def schedule(
        self,
        node_ids: List[str],
        failed_first: bool = False,
        recent_failures: Optional[List[str]] = None,
    ) -> List[str]:
        """Schedule tests for optimal execution order.

        Args:
            node_ids: Tests to schedule.
            failed_first: If True, prioritize recently failed tests.
            recent_failures: List of node IDs that failed recently.

        Returns:
            Ordered list of node IDs optimized for parallel execution.
        """
        if not node_ids:
            return []

        recent_failures = recent_failures or []

        # Create scheduled test objects
        scheduled: List[ScheduledTest] = []
        for node_id in node_ids:
            est_duration = self.get_estimated_duration(node_id)

            # Calculate priority
            priority = est_duration  # Longer tests get higher priority

            if failed_first and node_id in recent_failures:
                # Boost priority for recently failed tests
                priority += 1_000_000

            scheduled.append(ScheduledTest(
                node_id=node_id,
                estimated_duration_ms=est_duration,
                priority=priority,
            ))

        # Sort by priority descending (highest first)
        scheduled.sort(key=lambda x: x.priority, reverse=True)

        return [s.node_id for s in scheduled]

    def estimate_total_duration(
        self,
        node_ids: List[str],
        num_workers: int,
    ) -> Tuple[int, int]:
        """Estimate total execution time given number of workers.

        Returns:
            Tuple of (estimated_wall_time_ms, estimated_cpu_time_ms)
        """
        if not node_ids or num_workers == 0:
            return (0, 0)

        # Sum up all durations for CPU time
        total_cpu_ms = sum(self.get_estimated_duration(nid) for nid in node_ids)

        # Estimate wall time using LPT scheduling
        # This is an approximation - actual time depends on task arrival
        worker_loads = [0] * num_workers

        # Schedule longest first (LPT)
        durations = [(nid, self.get_estimated_duration(nid)) for nid in node_ids]
        durations.sort(key=lambda x: x[1], reverse=True)

        for _, duration in durations:
            # Assign to worker with least load
            min_idx = worker_loads.index(min(worker_loads))
            worker_loads[min_idx] += duration

        estimated_wall_ms = max(worker_loads) if worker_loads else 0

        return (estimated_wall_ms, total_cpu_ms)

    def get_stats(self) -> Dict[str, int]:
        """Get scheduler statistics."""
        total_tests = len(self._duration_history)
        total_runs = sum(len(v) for v in self._duration_history.values())
        avg_duration = 0

        if total_runs > 0:
            all_durations = [d for durations in self._duration_history.values() for d in durations]
            avg_duration = sum(all_durations) // len(all_durations)

        return {
            "tracked_tests": total_tests,
            "total_runs": total_runs,
            "avg_duration_ms": avg_duration,
        }

    def clear_history(self):
        """Clear all duration history."""
        self._duration_history.clear()
        logger.info("Scheduler history cleared")


def create_balanced_batches(
    node_ids: List[str],
    scheduler: TestScheduler,
    num_batches: int,
) -> List[List[str]]:
    """Create balanced batches of tests for parallel execution.

    Distributes tests across batches trying to balance total duration.

    Args:
        node_ids: Tests to distribute.
        scheduler: Scheduler with duration estimates.
        num_batches: Number of batches to create.

    Returns:
        List of batches, each containing test node IDs.
    """
    if not node_ids or num_batches == 0:
        return []

    if num_batches >= len(node_ids):
        # Each test gets its own batch
        return [[nid] for nid in node_ids]

    # Get estimated durations
    tests = [(nid, scheduler.get_estimated_duration(nid)) for nid in node_ids]

    # Sort by duration descending (longest first)
    tests.sort(key=lambda x: x[1], reverse=True)

    # Initialize batches
    batches: List[List[str]] = [[] for _ in range(num_batches)]
    batch_loads = [0] * num_batches

    # LPT assignment
    for node_id, duration in tests:
        # Find batch with minimum load
        min_idx = batch_loads.index(min(batch_loads))
        batches[min_idx].append(node_id)
        batch_loads[min_idx] += duration

    return batches
