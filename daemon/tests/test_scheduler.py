"""Tests for the test scheduler module."""

import pytest

from rpytest_daemon.scheduler import (
    ScheduledTest,
    TestScheduler,
    create_balanced_batches,
)


class TestScheduledTest:
    """Tests for ScheduledTest dataclass."""

    def test_basic_creation(self):
        test = ScheduledTest(
            node_id="test.py::test_func",
            estimated_duration_ms=1000,
        )
        assert test.node_id == "test.py::test_func"
        assert test.estimated_duration_ms == 1000
        assert test.priority == 0

    def test_with_priority(self):
        test = ScheduledTest(
            node_id="test.py::test_func",
            estimated_duration_ms=500,
            priority=10,
        )
        assert test.priority == 10


class TestTestScheduler:
    """Tests for TestScheduler class."""

    def test_default_duration(self):
        scheduler = TestScheduler()
        duration = scheduler.get_estimated_duration("unknown_test")
        assert duration == 1000  # Default 1 second

    def test_update_duration_single(self):
        scheduler = TestScheduler()
        scheduler.update_duration("test.py::test_1", 500)

        duration = scheduler.get_estimated_duration("test.py::test_1")
        assert duration == 500

    def test_update_duration_multiple(self):
        scheduler = TestScheduler()
        scheduler.update_duration("test.py::test_1", 100)
        scheduler.update_duration("test.py::test_1", 200)
        scheduler.update_duration("test.py::test_1", 300)

        # Uses exponential moving average, recent values weighted more
        duration = scheduler.get_estimated_duration("test.py::test_1")
        # Should be somewhere between 100 and 300, closer to 300
        assert 100 < duration < 300

    def test_update_duration_keeps_last_10(self):
        scheduler = TestScheduler()
        for i in range(15):
            scheduler.update_duration("test.py::test_1", 100)

        # History should be capped at 10
        assert len(scheduler._duration_history["test.py::test_1"]) == 10

    def test_schedule_empty(self):
        scheduler = TestScheduler()
        result = scheduler.schedule([])
        assert result == []

    def test_schedule_single_test(self):
        scheduler = TestScheduler()
        result = scheduler.schedule(["test.py::test_1"])
        assert result == ["test.py::test_1"]

    def test_schedule_longest_first(self):
        scheduler = TestScheduler()
        scheduler.update_duration("test.py::test_short", 100)
        scheduler.update_duration("test.py::test_long", 1000)
        scheduler.update_duration("test.py::test_medium", 500)

        result = scheduler.schedule([
            "test.py::test_short",
            "test.py::test_long",
            "test.py::test_medium",
        ])

        # Longest first (LPT algorithm)
        assert result[0] == "test.py::test_long"
        assert result[1] == "test.py::test_medium"
        assert result[2] == "test.py::test_short"

    def test_schedule_failed_first(self):
        scheduler = TestScheduler()
        scheduler.update_duration("test.py::test_a", 100)
        scheduler.update_duration("test.py::test_b", 100)
        scheduler.update_duration("test.py::test_c", 100)

        result = scheduler.schedule(
            ["test.py::test_a", "test.py::test_b", "test.py::test_c"],
            failed_first=True,
            recent_failures=["test.py::test_b"],
        )

        # Failed test should come first
        assert result[0] == "test.py::test_b"

    def test_schedule_failed_first_with_duration(self):
        scheduler = TestScheduler()
        scheduler.update_duration("test.py::test_short", 100)
        scheduler.update_duration("test.py::test_long", 1000)

        result = scheduler.schedule(
            ["test.py::test_short", "test.py::test_long"],
            failed_first=True,
            recent_failures=["test.py::test_short"],
        )

        # Failed test takes priority even though it's shorter
        assert result[0] == "test.py::test_short"
        assert result[1] == "test.py::test_long"

    def test_estimate_total_duration_empty(self):
        scheduler = TestScheduler()
        wall, cpu = scheduler.estimate_total_duration([], 4)
        assert wall == 0
        assert cpu == 0

    def test_estimate_total_duration_zero_workers(self):
        scheduler = TestScheduler()
        wall, cpu = scheduler.estimate_total_duration(["test.py::test_1"], 0)
        assert wall == 0
        assert cpu == 0

    def test_estimate_total_duration_single_worker(self):
        scheduler = TestScheduler()
        scheduler.update_duration("test.py::test_1", 100)
        scheduler.update_duration("test.py::test_2", 200)
        scheduler.update_duration("test.py::test_3", 300)

        wall, cpu = scheduler.estimate_total_duration(
            ["test.py::test_1", "test.py::test_2", "test.py::test_3"],
            num_workers=1,
        )

        # With 1 worker, wall time = cpu time = sum of all
        assert wall == 600
        assert cpu == 600

    def test_estimate_total_duration_multiple_workers(self):
        scheduler = TestScheduler()
        scheduler.update_duration("test.py::test_1", 100)
        scheduler.update_duration("test.py::test_2", 100)
        scheduler.update_duration("test.py::test_3", 100)
        scheduler.update_duration("test.py::test_4", 100)

        wall, cpu = scheduler.estimate_total_duration(
            ["test.py::test_1", "test.py::test_2",
             "test.py::test_3", "test.py::test_4"],
            num_workers=2,
        )

        # CPU time is sum
        assert cpu == 400
        # Wall time should be ~200 with 2 workers
        assert wall == 200

    def test_get_stats_empty(self):
        scheduler = TestScheduler()
        stats = scheduler.get_stats()

        assert stats["tracked_tests"] == 0
        assert stats["total_runs"] == 0
        assert stats["avg_duration_ms"] == 0

    def test_get_stats_with_data(self):
        scheduler = TestScheduler()
        scheduler.update_duration("test.py::test_1", 100)
        scheduler.update_duration("test.py::test_1", 200)
        scheduler.update_duration("test.py::test_2", 300)

        stats = scheduler.get_stats()

        assert stats["tracked_tests"] == 2
        assert stats["total_runs"] == 3
        assert stats["avg_duration_ms"] == 200  # (100+200+300)/3

    def test_clear_history(self):
        scheduler = TestScheduler()
        scheduler.update_duration("test.py::test_1", 100)
        scheduler.update_duration("test.py::test_2", 200)

        scheduler.clear_history()

        assert scheduler.get_estimated_duration("test.py::test_1") == 1000  # Default
        assert len(scheduler._duration_history) == 0


class TestCreateBalancedBatches:
    """Tests for create_balanced_batches function."""

    def test_empty_input(self):
        scheduler = TestScheduler()
        result = create_balanced_batches([], scheduler, 4)
        assert result == []

    def test_zero_batches(self):
        scheduler = TestScheduler()
        result = create_balanced_batches(["test.py::test_1"], scheduler, 0)
        assert result == []

    def test_more_batches_than_tests(self):
        scheduler = TestScheduler()
        result = create_balanced_batches(
            ["test.py::test_1", "test.py::test_2"],
            scheduler,
            num_batches=5,
        )

        # Each test gets its own batch
        assert len(result) == 2
        assert result[0] == ["test.py::test_1"]
        assert result[1] == ["test.py::test_2"]

    def test_equal_distribution(self):
        scheduler = TestScheduler()
        # All same duration
        for i in range(4):
            scheduler.update_duration(f"test.py::test_{i}", 100)

        result = create_balanced_batches(
            [f"test.py::test_{i}" for i in range(4)],
            scheduler,
            num_batches=2,
        )

        assert len(result) == 2
        assert len(result[0]) == 2
        assert len(result[1]) == 2

    def test_balanced_by_duration(self):
        scheduler = TestScheduler()
        scheduler.update_duration("test.py::test_long", 300)
        scheduler.update_duration("test.py::test_short_1", 100)
        scheduler.update_duration("test.py::test_short_2", 100)
        scheduler.update_duration("test.py::test_short_3", 100)

        result = create_balanced_batches(
            ["test.py::test_long", "test.py::test_short_1",
             "test.py::test_short_2", "test.py::test_short_3"],
            scheduler,
            num_batches=2,
        )

        # LPT should put long test in one batch, short tests distributed
        assert len(result) == 2

        # Calculate batch durations
        batch_durations = []
        for batch in result:
            duration = sum(scheduler.get_estimated_duration(t) for t in batch)
            batch_durations.append(duration)

        # Batches should be relatively balanced (300 vs 300 ideally)
        assert max(batch_durations) <= 400  # Not perfectly balanced but reasonable
