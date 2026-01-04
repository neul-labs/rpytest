"""Tests for the flakiness detection module."""

import json
import tempfile
from pathlib import Path

import pytest

from rpytest_daemon.flakiness import (
    FlakinessRecord,
    RerunResult,
    FlakinessTracker,
    determine_final_outcome,
    annotate_flaky_output,
)


class TestFlakinessRecord:
    """Tests for FlakinessRecord dataclass."""

    def test_basic_creation(self):
        record = FlakinessRecord(node_id="test.py::test_1")
        assert record.node_id == "test.py::test_1"
        assert record.outcomes == []
        assert record.consecutive_failures == 0
        assert record.consecutive_passes == 0
        assert record.flaky_streak == 0
        assert record.total_runs == 0
        assert record.last_failure_message is None

    def test_failure_rate_empty(self):
        record = FlakinessRecord(node_id="test.py::test_1")
        assert record.failure_rate == 0.0

    def test_failure_rate_all_passed(self):
        record = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["passed", "passed", "passed"],
        )
        assert record.failure_rate == 0.0

    def test_failure_rate_all_failed(self):
        record = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["failed", "failed", "failed"],
        )
        assert record.failure_rate == 1.0

    def test_failure_rate_mixed(self):
        record = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["passed", "failed", "passed", "error"],
        )
        # 2 failures out of 4
        assert record.failure_rate == 0.5

    def test_is_flaky_not_enough_history(self):
        record = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["passed", "failed"],
            flaky_streak=2,
        )
        # Need at least 3 outcomes
        assert record.is_flaky is False

    def test_is_flaky_all_same_outcome(self):
        record = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["passed", "passed", "passed"],
            flaky_streak=0,
        )
        # No mix of pass/fail
        assert record.is_flaky is False

    def test_is_flaky_insufficient_streak(self):
        record = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["passed", "failed", "passed"],
            flaky_streak=1,  # Need at least 2
        )
        assert record.is_flaky is False

    def test_is_flaky_true(self):
        record = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["passed", "failed", "passed"],
            flaky_streak=2,
        )
        assert record.is_flaky is True

    def test_record_outcome_first_pass(self):
        record = FlakinessRecord(node_id="test.py::test_1")
        record.record_outcome("passed")

        assert record.outcomes == ["passed"]
        assert record.total_runs == 1
        assert record.consecutive_passes == 1
        assert record.consecutive_failures == 0

    def test_record_outcome_first_failure(self):
        record = FlakinessRecord(node_id="test.py::test_1")
        record.record_outcome("failed", message="AssertionError")

        assert record.outcomes == ["failed"]
        assert record.total_runs == 1
        assert record.consecutive_failures == 1
        assert record.consecutive_passes == 0
        assert record.last_failure_message == "AssertionError"

    def test_record_outcome_consecutive_tracking(self):
        record = FlakinessRecord(node_id="test.py::test_1")
        record.record_outcome("passed")
        record.record_outcome("passed")
        record.record_outcome("failed")
        record.record_outcome("failed")
        record.record_outcome("passed")

        assert record.consecutive_passes == 1
        assert record.consecutive_failures == 0

    def test_record_outcome_flaky_streak_incremented(self):
        record = FlakinessRecord(node_id="test.py::test_1")
        record.record_outcome("passed")
        record.record_outcome("failed")  # Flip 1
        record.record_outcome("passed")  # Flip 2

        assert record.flaky_streak == 2

    def test_record_outcome_max_history(self):
        record = FlakinessRecord(node_id="test.py::test_1")
        for i in range(25):
            record.record_outcome("passed")

        # Should keep only last 20
        assert len(record.outcomes) == 20
        assert record.total_runs == 25

    def test_record_outcome_skipped_resets_counters(self):
        record = FlakinessRecord(node_id="test.py::test_1")
        record.record_outcome("passed")
        record.record_outcome("passed")
        record.record_outcome("skipped")

        assert record.consecutive_passes == 0
        assert record.consecutive_failures == 0

    def test_to_dict(self):
        record = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["passed", "failed"],
            consecutive_failures=1,
            consecutive_passes=0,
            flaky_streak=1,
            total_runs=2,
            last_failure_message="Error!",
        )
        result = record.to_dict()

        assert result == {
            "node_id": "test.py::test_1",
            "outcomes": ["passed", "failed"],
            "consecutive_failures": 1,
            "consecutive_passes": 0,
            "flaky_streak": 1,
            "total_runs": 2,
            "last_failure_message": "Error!",
        }

    def test_from_dict(self):
        data = {
            "node_id": "test.py::test_1",
            "outcomes": ["passed", "failed", "passed"],
            "consecutive_failures": 0,
            "consecutive_passes": 1,
            "flaky_streak": 2,
            "total_runs": 3,
            "last_failure_message": "Previous error",
        }
        record = FlakinessRecord.from_dict(data)

        assert record.node_id == "test.py::test_1"
        assert record.outcomes == ["passed", "failed", "passed"]
        assert record.flaky_streak == 2


class TestRerunResult:
    """Tests for RerunResult dataclass."""

    def test_basic_creation(self):
        result = RerunResult(
            node_id="test.py::test_1",
            original_outcome="failed",
            rerun_outcomes=["passed"],
            final_outcome="passed",
            is_flaky=True,
        )
        assert result.node_id == "test.py::test_1"
        assert result.passed_on_rerun is True

    def test_passed_on_rerun_false_when_original_passed(self):
        result = RerunResult(
            node_id="test.py::test_1",
            original_outcome="passed",
            rerun_outcomes=[],
            final_outcome="passed",
            is_flaky=False,
        )
        assert result.passed_on_rerun is False

    def test_passed_on_rerun_false_when_still_failed(self):
        result = RerunResult(
            node_id="test.py::test_1",
            original_outcome="failed",
            rerun_outcomes=["failed"],
            final_outcome="failed",
            is_flaky=False,
        )
        assert result.passed_on_rerun is False


class TestFlakinessTracker:
    """Tests for FlakinessTracker class."""

    def test_init_no_storage(self):
        tracker = FlakinessTracker()
        assert tracker.records == {}
        assert tracker.storage_path is None

    def test_init_with_storage(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            storage_path = Path(tmpdir) / "flakiness.json"
            tracker = FlakinessTracker(storage_path=storage_path)
            assert tracker.storage_path == storage_path

    def test_record_outcome_new_test(self):
        tracker = FlakinessTracker()
        tracker.record_outcome("test.py::test_1", "passed")

        assert "test.py::test_1" in tracker.records
        record = tracker.records["test.py::test_1"]
        assert record.outcomes == ["passed"]

    def test_record_outcome_existing_test(self):
        tracker = FlakinessTracker()
        tracker.record_outcome("test.py::test_1", "passed")
        tracker.record_outcome("test.py::test_1", "failed", message="Error!")

        record = tracker.records["test.py::test_1"]
        assert record.outcomes == ["passed", "failed"]
        assert record.last_failure_message == "Error!"

    def test_get_record_existing(self):
        tracker = FlakinessTracker()
        tracker.record_outcome("test.py::test_1", "passed")

        record = tracker.get_record("test.py::test_1")
        assert record is not None
        assert record.node_id == "test.py::test_1"

    def test_get_record_nonexistent(self):
        tracker = FlakinessTracker()
        record = tracker.get_record("test.py::test_unknown")
        assert record is None

    def test_get_flaky_tests(self):
        tracker = FlakinessTracker()

        # Create a flaky test
        tracker.records["test.py::test_flaky"] = FlakinessRecord(
            node_id="test.py::test_flaky",
            outcomes=["passed", "failed", "passed"],
            flaky_streak=2,
        )

        # Create a stable test
        tracker.records["test.py::test_stable"] = FlakinessRecord(
            node_id="test.py::test_stable",
            outcomes=["passed", "passed", "passed"],
            flaky_streak=0,
        )

        flaky = tracker.get_flaky_tests()
        assert "test.py::test_flaky" in flaky
        assert "test.py::test_stable" not in flaky

    def test_get_failure_rate_existing(self):
        tracker = FlakinessTracker()
        tracker.records["test.py::test_1"] = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["passed", "failed", "passed", "failed"],
        )

        rate = tracker.get_failure_rate("test.py::test_1")
        assert rate == 0.5

    def test_get_failure_rate_nonexistent(self):
        tracker = FlakinessTracker()
        rate = tracker.get_failure_rate("test.py::test_unknown")
        assert rate == 0.0

    def test_should_rerun_passed_test(self):
        tracker = FlakinessTracker()
        should_rerun, reason = tracker.should_rerun("test.py::test_1", "passed")
        assert should_rerun is False
        assert reason == "not_failed"

    def test_should_rerun_first_failure(self):
        tracker = FlakinessTracker()
        should_rerun, reason = tracker.should_rerun("test.py::test_1", "failed")
        assert should_rerun is True
        assert reason == "first_failure"

    def test_should_rerun_known_flaky(self):
        tracker = FlakinessTracker()
        tracker.records["test.py::test_1"] = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["passed", "failed", "passed"],
            flaky_streak=2,
        )

        should_rerun, reason = tracker.should_rerun("test.py::test_1", "failed")
        assert should_rerun is True
        assert reason == "known_flaky"

    def test_should_rerun_regression_check(self):
        tracker = FlakinessTracker()
        tracker.records["test.py::test_1"] = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["passed", "passed", "passed", "passed", "passed"],
            consecutive_passes=5,
            flaky_streak=0,
        )

        should_rerun, reason = tracker.should_rerun("test.py::test_1", "failed")
        assert should_rerun is True
        assert reason == "regression_check"

    def test_should_rerun_high_failure_rate(self):
        tracker = FlakinessTracker()
        tracker.records["test.py::test_1"] = FlakinessRecord(
            node_id="test.py::test_1",
            outcomes=["failed", "failed", "failed", "passed"],
            consecutive_passes=1,
            flaky_streak=1,
        )

        should_rerun, reason = tracker.should_rerun("test.py::test_1", "failed")
        assert should_rerun is False
        assert reason == "high_failure_rate"

    def test_get_flakiness_report(self):
        tracker = FlakinessTracker()

        # Add various test records
        tracker.records["test_flaky"] = FlakinessRecord(
            node_id="test_flaky",
            outcomes=["passed", "failed", "passed"],
            flaky_streak=2,
        )
        tracker.records["test_unstable"] = FlakinessRecord(
            node_id="test_unstable",
            outcomes=["passed", "passed", "passed", "passed", "passed",
                      "passed", "passed", "failed", "passed", "failed"],
        )
        tracker.records["test_stable"] = FlakinessRecord(
            node_id="test_stable",
            outcomes=["passed", "passed", "passed"],
        )

        report = tracker.get_flakiness_report()

        assert report["total_tracked"] == 3
        assert len(report["flaky_tests"]) == 1
        assert len(report["unstable_tests"]) == 1
        assert report["stable_count"] == 1

    def test_persistence_save_and_load(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            storage_path = Path(tmpdir) / "flakiness.json"

            # Create tracker and add records
            tracker1 = FlakinessTracker(storage_path=storage_path)
            tracker1.record_outcome("test.py::test_1", "passed")
            tracker1.record_outcome("test.py::test_1", "failed")

            # Create new tracker loading from same file
            tracker2 = FlakinessTracker(storage_path=storage_path)

            assert "test.py::test_1" in tracker2.records
            assert tracker2.records["test.py::test_1"].outcomes == ["passed", "failed"]


class TestDetermineFinalOutcome:
    """Tests for determine_final_outcome function."""

    def test_empty_outcomes(self):
        assert determine_final_outcome([]) == "unknown"

    def test_any_passed_wins(self):
        assert determine_final_outcome(["failed", "passed", "failed"]) == "passed"

    def test_skipped_returned(self):
        assert determine_final_outcome(["failed", "skipped"]) == "skipped"

    def test_xfail_uses_last(self):
        assert determine_final_outcome(["failed", "xfail"]) == "xfail"

    def test_all_failures_uses_last(self):
        assert determine_final_outcome(["failed", "error"]) == "error"

    def test_single_passed(self):
        assert determine_final_outcome(["passed"]) == "passed"

    def test_single_failed(self):
        assert determine_final_outcome(["failed"]) == "failed"


class TestAnnotateFlakyOutput:
    """Tests for annotate_flaky_output function."""

    def test_no_annotation_needed(self):
        output = "Test output"
        result = annotate_flaky_output(output, is_flaky=False, rerun_count=0)
        assert result == output

    def test_flaky_annotation(self):
        output = "Test output"
        result = annotate_flaky_output(output, is_flaky=True, rerun_count=0)
        assert result == "[FLAKY TEST]\nTest output"

    def test_rerun_annotation(self):
        output = "Test output"
        result = annotate_flaky_output(output, is_flaky=False, rerun_count=2)
        assert result == "[RERUN 2x]\nTest output"

    def test_both_annotations(self):
        output = "Test output"
        result = annotate_flaky_output(output, is_flaky=True, rerun_count=3)
        assert result == "[FLAKY TEST] [RERUN 3x]\nTest output"

    def test_empty_output_flaky(self):
        result = annotate_flaky_output("", is_flaky=True, rerun_count=0)
        assert result == "[FLAKY TEST]"

    def test_empty_output_with_rerun(self):
        result = annotate_flaky_output("", is_flaky=False, rerun_count=1)
        assert result == "[RERUN 1x]"
