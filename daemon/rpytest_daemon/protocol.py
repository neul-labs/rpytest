"""Protocol message types matching the Rust definitions."""

from dataclasses import dataclass, field
from enum import Enum
from typing import Optional, List, Dict, Any
import msgpack


class ErrorCode(str, Enum):
    """Error codes for categorizing failures."""
    CONTEXT_NOT_FOUND = "context_not_found"
    COLLECTION_FAILED = "collection_failed"
    INVALID_REQUEST = "invalid_request"
    INTERNAL_ERROR = "internal_error"
    TIMEOUT = "timeout"
    PYTHON_NOT_FOUND = "python_not_found"


class OutcomeStatus(str, Enum):
    """Test outcome status."""
    PASSED = "passed"
    FAILED = "failed"
    SKIPPED = "skipped"
    ERROR = "error"
    XFAIL = "xfail"
    XPASS = "xpass"


@dataclass
class Outcome:
    """Test outcome with optional details."""
    status: OutcomeStatus
    message: Optional[str] = None
    reason: Optional[str] = None

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for serialization."""
        result = {"status": self.status.value}
        if self.message:
            result["message"] = self.message
        if self.reason:
            result["reason"] = self.reason
        return result


def decode_request(data: bytes) -> Dict[str, Any]:
    """Decode a MessagePack request."""
    return msgpack.unpackb(data, raw=False)


def encode_response(response: Dict[str, Any]) -> bytes:
    """Encode a response to MessagePack."""
    return msgpack.packb(response, use_bin_type=True)


# Response builders
def context_ready(context_id: str, inventory_hash: str) -> Dict[str, Any]:
    """Build a ContextReady response."""
    return {
        "type": "context_ready",
        "context_id": context_id,
        "inventory_hash": inventory_hash,
    }


def collection_complete(node_count: int, duration_ms: int) -> Dict[str, Any]:
    """Build a CollectionComplete response."""
    return {
        "type": "collection_complete",
        "node_count": node_count,
        "duration_ms": duration_ms,
    }


def test_list(node_ids: List[str]) -> Dict[str, Any]:
    """Build a TestList response."""
    return {
        "type": "test_list",
        "node_ids": node_ids,
    }


def inventory_data(
    hash: str,
    collected_at: int,
    nodes: List[Dict[str, Any]],
) -> Dict[str, Any]:
    """Build an InventoryData response."""
    return {
        "type": "inventory_data",
        "hash": hash,
        "collected_at": collected_at,
        "nodes": nodes,
    }


def make_test_node_info(
    node_id: str,
    file_path: str,
    lineno: Optional[int],
    name: str,
    class_name: Optional[str],
    markers: List[str],
    skip: bool,
    xfail: bool,
) -> Dict[str, Any]:
    """Build a TestNodeInfo dict for InventoryData response."""
    return {
        "node_id": node_id,
        "file_path": file_path,
        "lineno": lineno,
        "name": name,
        "class_name": class_name,
        "markers": markers,
        "skip": skip,
        "xfail": xfail,
    }


def run_complete(
    total: int,
    passed: int,
    failed: int,
    skipped: int,
    errors: int,
    duration_ms: int,
) -> Dict[str, Any]:
    """Build a RunComplete response."""
    return {
        "type": "run_complete",
        "total": total,
        "passed": passed,
        "failed": failed,
        "skipped": skipped,
        "errors": errors,
        "duration_ms": duration_ms,
    }


def shutdown_ack() -> Dict[str, Any]:
    """Build a ShutdownAck response."""
    return {"type": "shutdown_ack"}


def pong() -> Dict[str, Any]:
    """Build a Pong response."""
    return {"type": "pong"}


def error(code: ErrorCode, message: str) -> Dict[str, Any]:
    """Build an Error response."""
    return {
        "type": "error",
        "code": code.value,
        "message": message,
    }


def worker_status(
    active_workers: int,
    idle_workers: int,
    tests_executed: int,
    avg_test_duration_ms: int,
) -> Dict[str, Any]:
    """Build a WorkerStatus response."""
    return {
        "type": "worker_status",
        "active_workers": active_workers,
        "idle_workers": idle_workers,
        "tests_executed": tests_executed,
        "avg_test_duration_ms": avg_test_duration_ms,
    }


def worker_config_ack(num_workers: int) -> Dict[str, Any]:
    """Build a WorkerConfigAck response."""
    return {
        "type": "worker_config_ack",
        "num_workers": num_workers,
    }


def run_started(run_id: str, total_tests: int) -> Dict[str, Any]:
    """Build a RunStarted response."""
    return {
        "type": "run_started",
        "run_id": run_id,
        "total_tests": total_tests,
    }


def run_progress(
    run_id: str,
    total: int,
    completed: int,
    running: int,
    done: bool,
    results: List[Dict[str, Any]],
) -> Dict[str, Any]:
    """Build a RunProgress response."""
    return {
        "type": "run_progress",
        "run_id": run_id,
        "total": total,
        "completed": completed,
        "running": running,
        "done": done,
        "results": results,
    }


def make_test_result_info(
    node_id: str,
    outcome: str,
    duration_ms: int,
    message: Optional[str] = None,
) -> Dict[str, Any]:
    """Build a TestResultInfo dict."""
    return {
        "node_id": node_id,
        "outcome": outcome,
        "duration_ms": duration_ms,
        "message": message,
    }


# Phase 5: Flakiness, Fixtures, and Sharding responses

def flakiness_report(
    flaky_tests: List[Dict[str, Any]],
    unstable_tests: List[Dict[str, Any]],
    stable_count: int,
    total_tracked: int,
) -> Dict[str, Any]:
    """Build a FlakinessReport response."""
    return {
        "type": "flakiness_report",
        "flaky_tests": flaky_tests,
        "unstable_tests": unstable_tests,
        "stable_count": stable_count,
        "total_tracked": total_tracked,
    }


def test_flakiness(
    node_id: str,
    failure_rate: float,
    is_flaky: bool,
    flaky_streak: int,
    consecutive_failures: int,
    consecutive_passes: int,
    total_runs: int,
    recent_outcomes: List[str],
) -> Dict[str, Any]:
    """Build a TestFlakiness response."""
    return {
        "type": "test_flakiness",
        "node_id": node_id,
        "failure_rate": failure_rate,
        "is_flaky": is_flaky,
        "flaky_streak": flaky_streak,
        "consecutive_failures": consecutive_failures,
        "consecutive_passes": consecutive_passes,
        "total_runs": total_runs,
        "recent_outcomes": recent_outcomes,
    }


def rerun_config(
    enabled: bool,
    max_reruns: int,
    only_flaky: bool,
    delay_ms: int,
) -> Dict[str, Any]:
    """Build a RerunConfig response."""
    return {
        "type": "rerun_config",
        "enabled": enabled,
        "max_reruns": max_reruns,
        "only_flaky": only_flaky,
        "delay_ms": delay_ms,
    }


def fixture_config(
    enabled: bool,
    max_fixture_age_seconds: float,
    teardown_on_conftest_change: bool,
    teardown_on_test_file_change: bool,
    scopes_to_reuse: List[str],
) -> Dict[str, Any]:
    """Build a FixtureConfig response."""
    return {
        "type": "fixture_config",
        "enabled": enabled,
        "max_fixture_age_seconds": max_fixture_age_seconds,
        "teardown_on_conftest_change": teardown_on_conftest_change,
        "teardown_on_test_file_change": teardown_on_test_file_change,
        "scopes_to_reuse": scopes_to_reuse,
    }


def session_status(
    session_id: str,
    repo_path: str,
    created_at: float,
    last_run_at: float,
    total_runs: int,
    enabled: bool,
    fixtures: Dict[str, Any],
) -> Dict[str, Any]:
    """Build a SessionStatus response."""
    return {
        "type": "session_status",
        "session_id": session_id,
        "repo_path": repo_path,
        "created_at": created_at,
        "last_run_at": last_run_at,
        "total_runs": total_runs,
        "enabled": enabled,
        "fixtures": fixtures,
    }


def shard_info(
    strategy: str,
    total_shards: int,
    total_tests: int,
    shard_test_counts: List[int],
    shard_durations_ms: List[int],
    count_imbalance_percent: float,
    duration_imbalance_percent: float,
    estimated_wall_time_ms: int,
) -> Dict[str, Any]:
    """Build a ShardInfo response."""
    return {
        "type": "shard_info",
        "strategy": strategy,
        "total_shards": total_shards,
        "total_tests": total_tests,
        "shard_test_counts": shard_test_counts,
        "shard_durations_ms": shard_durations_ms,
        "count_imbalance_percent": count_imbalance_percent,
        "duration_imbalance_percent": duration_imbalance_percent,
        "estimated_wall_time_ms": estimated_wall_time_ms,
    }


def sharded_tests(
    shard_index: int,
    total_shards: int,
    node_ids: List[str],
) -> Dict[str, Any]:
    """Build a ShardedTests response."""
    return {
        "type": "sharded_tests",
        "shard_index": shard_index,
        "total_shards": total_shards,
        "node_ids": node_ids,
    }


def config_ack(config_type: str, config: Dict[str, Any]) -> Dict[str, Any]:
    """Build a ConfigAck response."""
    return {
        "type": "config_ack",
        "config_type": config_type,
        "config": config,
    }
