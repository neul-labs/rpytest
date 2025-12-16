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
