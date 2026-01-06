"""Tests for protocol message types and serialization."""

import msgpack
import pytest

from rpytest_daemon.protocol import (
    ErrorCode,
    OutcomeStatus,
    Outcome,
    decode_request,
    encode_response,
    context_ready,
    collection_complete,
    test_list as make_test_list_response,
    inventory_data,
    make_test_node_info,
    run_complete,
    shutdown_ack,
    pong,
    error,
    worker_status,
    run_started,
    run_progress,
    make_test_result_info,
    flakiness_report,
    test_flakiness as make_test_flakiness_response,
    shard_info,
    sharded_tests,
)


class TestErrorCode:
    """Tests for ErrorCode enum."""

    def test_error_codes_are_strings(self):
        assert ErrorCode.CONTEXT_NOT_FOUND.value == "context_not_found"
        assert ErrorCode.COLLECTION_FAILED.value == "collection_failed"
        assert ErrorCode.INVALID_REQUEST.value == "invalid_request"
        assert ErrorCode.INTERNAL_ERROR.value == "internal_error"
        assert ErrorCode.TIMEOUT.value == "timeout"
        assert ErrorCode.PYTHON_NOT_FOUND.value == "python_not_found"


class TestOutcomeStatus:
    """Tests for OutcomeStatus enum."""

    def test_outcome_statuses(self):
        assert OutcomeStatus.PASSED.value == "passed"
        assert OutcomeStatus.FAILED.value == "failed"
        assert OutcomeStatus.SKIPPED.value == "skipped"
        assert OutcomeStatus.ERROR.value == "error"
        assert OutcomeStatus.XFAIL.value == "xfail"
        assert OutcomeStatus.XPASS.value == "xpass"


class TestOutcome:
    """Tests for Outcome dataclass."""

    def test_outcome_basic(self):
        outcome = Outcome(status=OutcomeStatus.PASSED)
        assert outcome.status == OutcomeStatus.PASSED
        assert outcome.message is None
        assert outcome.reason is None

    def test_outcome_with_message(self):
        outcome = Outcome(
            status=OutcomeStatus.FAILED,
            message="Assertion error",
            reason="Expected True",
        )
        assert outcome.status == OutcomeStatus.FAILED
        assert outcome.message == "Assertion error"
        assert outcome.reason == "Expected True"

    def test_outcome_to_dict_minimal(self):
        outcome = Outcome(status=OutcomeStatus.PASSED)
        result = outcome.to_dict()
        assert result == {"status": "passed"}

    def test_outcome_to_dict_full(self):
        outcome = Outcome(
            status=OutcomeStatus.FAILED,
            message="error msg",
            reason="test reason",
        )
        result = outcome.to_dict()
        assert result == {
            "status": "failed",
            "message": "error msg",
            "reason": "test reason",
        }


class TestMessagePackRoundtrip:
    """Tests for msgpack encoding/decoding."""

    def test_decode_request_simple(self):
        data = msgpack.packb({"type": "ping"}, use_bin_type=True)
        result = decode_request(data)
        assert result == {"type": "ping"}

    def test_decode_request_with_payload(self):
        request = {
            "type": "init_context",
            "repo_path": "/path/to/repo",
            "python_path": "/usr/bin/python3",
        }
        data = msgpack.packb(request, use_bin_type=True)
        result = decode_request(data)
        assert result == request

    def test_encode_response_simple(self):
        response = {"type": "pong"}
        encoded = encode_response(response)
        decoded = msgpack.unpackb(encoded, raw=False)
        assert decoded == response

    def test_roundtrip_complex_payload(self):
        response = {
            "type": "run_progress",
            "run_id": "abc123",
            "total": 100,
            "completed": 50,
            "running": 5,
            "done": False,
            "results": [
                {"node_id": "test.py::test_1", "outcome": "passed", "duration_ms": 100},
                {"node_id": "test.py::test_2", "outcome": "failed", "duration_ms": 200},
            ],
        }
        encoded = encode_response(response)
        decoded = msgpack.unpackb(encoded, raw=False)
        assert decoded == response


class TestResponseBuilders:
    """Tests for response builder functions."""

    def test_context_ready(self):
        from rpytest_daemon.protocol import PROTOCOL_VERSION
        result = context_ready("ctx-123", "hash-abc")
        assert result == {
            "type": "context_ready",
            "protocol_version": PROTOCOL_VERSION,
            "context_id": "ctx-123",
            "inventory_hash": "hash-abc",
        }

    def test_collection_complete(self):
        result = collection_complete(node_count=42, duration_ms=1500)
        assert result == {
            "type": "collection_complete",
            "node_count": 42,
            "duration_ms": 1500,
        }

    def test_test_list(self):
        node_ids = ["test.py::test_1", "test.py::test_2"]
        result = make_test_list_response(node_ids)
        assert result == {
            "type": "test_list",
            "node_ids": node_ids,
        }

    def test_inventory_data(self):
        nodes = [
            make_test_node_info(
                node_id="test.py::test_1",
                file_path="test.py",
                lineno=10,
                name="test_1",
                class_name=None,
                markers=["slow"],
                skip=False,
                xfail=False,
            )
        ]
        result = inventory_data(
            hash="abc123",
            collected_at=1000000,
            nodes=nodes,
        )
        assert result["type"] == "inventory_data"
        assert result["hash"] == "abc123"
        assert result["collected_at"] == 1000000
        assert len(result["nodes"]) == 1

    def test_make_test_node_info(self):
        result = make_test_node_info(
            node_id="test.py::TestClass::test_method",
            file_path="test.py",
            lineno=25,
            name="test_method",
            class_name="TestClass",
            markers=["slow", "integration"],
            skip=True,
            xfail=False,
        )
        assert result == {
            "node_id": "test.py::TestClass::test_method",
            "file_path": "test.py",
            "lineno": 25,
            "name": "test_method",
            "class_name": "TestClass",
            "markers": ["slow", "integration"],
            "skip": True,
            "xfail": False,
        }

    def test_run_complete(self):
        result = run_complete(
            total=100,
            passed=90,
            failed=5,
            skipped=3,
            errors=2,
            duration_ms=5000,
        )
        assert result == {
            "type": "run_complete",
            "total": 100,
            "passed": 90,
            "failed": 5,
            "skipped": 3,
            "errors": 2,
            "duration_ms": 5000,
        }

    def test_shutdown_ack(self):
        result = shutdown_ack()
        assert result == {"type": "shutdown_ack"}

    def test_pong(self):
        result = pong()
        assert result == {"type": "pong"}

    def test_error(self):
        result = error(ErrorCode.TIMEOUT, "Request timed out")
        assert result == {
            "type": "error",
            "code": "timeout",
            "message": "Request timed out",
        }

    def test_worker_status(self):
        result = worker_status(
            active_workers=4,
            idle_workers=2,
            tests_executed=100,
            avg_test_duration_ms=150,
        )
        assert result == {
            "type": "worker_status",
            "active_workers": 4,
            "idle_workers": 2,
            "tests_executed": 100,
            "avg_test_duration_ms": 150,
        }

    def test_run_started(self):
        result = run_started("run-456", 50)
        assert result == {
            "type": "run_started",
            "run_id": "run-456",
            "total_tests": 50,
        }

    def test_run_progress(self):
        results = [make_test_result_info("test.py::test_1", "passed", 100)]
        result = run_progress(
            run_id="run-789",
            total=10,
            completed=5,
            running=2,
            done=False,
            results=results,
        )
        assert result["type"] == "run_progress"
        assert result["run_id"] == "run-789"
        assert result["total"] == 10
        assert result["completed"] == 5
        assert result["done"] is False
        assert len(result["results"]) == 1

    def test_make_test_result_info(self):
        result = make_test_result_info(
            node_id="test.py::test_1",
            outcome="failed",
            duration_ms=250,
            message="AssertionError: expected True",
        )
        assert result == {
            "node_id": "test.py::test_1",
            "outcome": "failed",
            "duration_ms": 250,
            "message": "AssertionError: expected True",
        }

    def test_flakiness_report(self):
        result = flakiness_report(
            flaky_tests=[{"node_id": "test.py::test_flaky", "failure_rate": 0.3}],
            unstable_tests=[],
            stable_count=95,
            total_tracked=100,
        )
        assert result["type"] == "flakiness_report"
        assert len(result["flaky_tests"]) == 1
        assert result["stable_count"] == 95

    def test_test_flakiness(self):
        result = make_test_flakiness_response(
            node_id="test.py::test_flaky",
            failure_rate=0.25,
            is_flaky=True,
            flaky_streak=3,
            consecutive_failures=0,
            consecutive_passes=2,
            total_runs=20,
            recent_outcomes=["passed", "failed", "passed"],
        )
        assert result["type"] == "test_flakiness"
        assert result["is_flaky"] is True
        assert result["flaky_streak"] == 3

    def test_shard_info(self):
        result = shard_info(
            strategy="duration_balanced",
            total_shards=4,
            total_tests=100,
            shard_test_counts=[25, 25, 25, 25],
            shard_durations_ms=[5000, 5100, 4900, 5000],
            count_imbalance_percent=0.0,
            duration_imbalance_percent=4.0,
            estimated_wall_time_ms=5100,
        )
        assert result["type"] == "shard_info"
        assert result["strategy"] == "duration_balanced"
        assert result["total_shards"] == 4

    def test_sharded_tests(self):
        result = sharded_tests(
            shard_index=1,
            total_shards=4,
            node_ids=["test.py::test_1", "test.py::test_2"],
        )
        assert result == {
            "type": "sharded_tests",
            "shard_index": 1,
            "total_shards": 4,
            "node_ids": ["test.py::test_1", "test.py::test_2"],
        }


class TestEdgeCases:
    """Tests for edge cases in protocol handling."""

    def test_empty_node_ids(self):
        result = make_test_list_response([])
        assert result["node_ids"] == []

    def test_empty_string_values(self):
        result = make_test_result_info(
            node_id="",
            outcome="passed",
            duration_ms=0,
            message="",
        )
        assert result["node_id"] == ""
        assert result["message"] == ""

    def test_large_payload(self):
        node_ids = [f"test_{i}.py::test_func_{i}" for i in range(1000)]
        result = make_test_list_response(node_ids)
        assert len(result["node_ids"]) == 1000

        # Verify it can be encoded/decoded
        encoded = encode_response(result)
        decoded = msgpack.unpackb(encoded, raw=False)
        assert len(decoded["node_ids"]) == 1000

    def test_unicode_in_messages(self):
        result = error(ErrorCode.INTERNAL_ERROR, "Error with unicode: \u4e2d\u6587")
        encoded = encode_response(result)
        decoded = msgpack.unpackb(encoded, raw=False)
        assert "\u4e2d\u6587" in decoded["message"]

    def test_special_characters_in_node_id(self):
        node_id = "test.py::test_param[value-with-special!@#$%]"
        result = make_test_result_info(node_id, "passed", 100)
        assert result["node_id"] == node_id
