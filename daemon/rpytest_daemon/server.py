"""NNG-based RPC server for rpytest daemon."""

import logging
import os
import signal
import struct
import sys
import time
from pathlib import Path
from typing import Optional

import pynng

from .context import ContextRegistry
from .protocol import (
    ErrorCode,
    collection_complete,
    config_ack,
    context_ready,
    decode_request,
    encode_response,
    error,
    flakiness_report,
    fixture_config,
    inventory_data,
    make_test_result_info,
    pong,
    rerun_config,
    run_complete,
    run_progress,
    run_started,
    session_status,
    shard_info,
    sharded_tests,
    shutdown_ack,
    test_flakiness,
    test_list,
    worker_status,
    worker_config_ack,
)

logger = logging.getLogger(__name__)


def get_default_socket_path() -> str:
    """Get the default socket path."""
    runtime_dir = os.environ.get("XDG_RUNTIME_DIR", "/tmp")
    return f"ipc://{runtime_dir}/rpytest.sock"


class DaemonServer:
    """NNG-based RPC server for rpytest."""

    def __init__(self, socket_path: Optional[str] = None, idle_timeout: int = 0):
        self.socket_path = socket_path or get_default_socket_path()
        self.idle_timeout = idle_timeout  # 0 = no timeout
        self.registry = ContextRegistry()
        self.running = False
        self._socket: Optional[pynng.Rep0] = None
        self._last_activity = time.time()

    def _update_activity(self):
        """Update the last activity timestamp."""
        self._last_activity = time.time()

    def _check_idle_timeout(self) -> bool:
        """Check if idle timeout has been exceeded. Returns True if should stop."""
        if self.idle_timeout <= 0:
            return False
        idle_time = time.time() - self._last_activity
        if idle_time >= self.idle_timeout:
            logger.info(f"Idle timeout reached ({self.idle_timeout}s), shutting down")
            return True
        return False

    def start(self):
        """Start the daemon server."""
        logger.info(f"Starting daemon at {self.socket_path}")
        if self.idle_timeout > 0:
            logger.info(f"Idle timeout: {self.idle_timeout}s")

        # Clean up stale socket file if it exists
        socket_file = self.socket_path.replace("ipc://", "")
        if os.path.exists(socket_file):
            os.unlink(socket_file)
            logger.debug(f"Removed stale socket file: {socket_file}")

        # Create REP socket (request-reply pattern)
        self._socket = pynng.Rep0()
        self._socket.listen(self.socket_path)

        # Set socket options
        self._socket.recv_timeout = 1000  # 1 second timeout for checking shutdown

        self.running = True
        self._update_activity()
        logger.info("Daemon started, waiting for connections")

        # Main loop
        while self.running:
            try:
                # Receive request with length prefix
                data = self._socket.recv()
                self._update_activity()
                response = self._handle_request(data)
                self._socket.send(response)
            except pynng.Timeout:
                # Timeout is expected - check idle timeout
                if self._check_idle_timeout():
                    break
                continue
            except pynng.Closed:
                logger.info("Socket closed")
                break
            except Exception as e:
                logger.exception(f"Error handling request: {e}")
                try:
                    err_response = self._encode_error(
                        ErrorCode.INTERNAL_ERROR,
                        str(e),
                    )
                    self._socket.send(err_response)
                except Exception:
                    pass

        self._cleanup()

    def stop(self):
        """Stop the daemon server."""
        logger.info("Stopping daemon")
        self.running = False

    def _cleanup(self):
        """Clean up resources."""
        if self._socket:
            self._socket.close()
            self._socket = None

        # Clean up socket file
        socket_file = self.socket_path.replace("ipc://", "")
        if os.path.exists(socket_file):
            os.unlink(socket_file)

        logger.info("Daemon stopped")

    def _handle_request(self, data: bytes) -> bytes:
        """Handle an incoming request and return a response."""
        # Parse length-prefixed frame
        if len(data) < 4:
            return self._encode_error(ErrorCode.INVALID_REQUEST, "Frame too short")

        length = struct.unpack("<I", data[:4])[0]
        payload = data[4:4 + length]

        try:
            request = decode_request(payload)
        except Exception as e:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                f"Failed to decode request: {e}",
            )

        request_type = request.get("type")
        logger.debug(f"Received request: {request_type}")

        # Dispatch to handler
        handlers = {
            "init_context": self._handle_init_context,
            "collect": self._handle_collect,
            "run": self._handle_run,
            "list": self._handle_list,
            "get_inventory": self._handle_get_inventory,
            "get_worker_status": self._handle_get_worker_status,
            "configure_workers": self._handle_configure_workers,
            "run_stream": self._handle_run_stream,
            "get_run_progress": self._handle_get_run_progress,
            "shutdown": self._handle_shutdown,
            "ping": self._handle_ping,
            # Phase 5: Flakiness
            "get_flakiness_report": self._handle_get_flakiness_report,
            "get_test_flakiness": self._handle_get_test_flakiness,
            "configure_rerun": self._handle_configure_rerun,
            "get_rerun_config": self._handle_get_rerun_config,
            "run_with_rerun": self._handle_run_with_rerun,
            # Phase 5: Fixtures
            "configure_fixture_reuse": self._handle_configure_fixture_reuse,
            "get_fixture_config": self._handle_get_fixture_config,
            "get_session_status": self._handle_get_session_status,
            # Phase 5: Sharding
            "get_shard": self._handle_get_shard,
            "get_shard_info": self._handle_get_shard_info,
        }

        handler = handlers.get(request_type)
        if handler:
            try:
                return handler(request)
            except Exception as e:
                logger.exception(f"Handler error: {e}")
                return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))
        else:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                f"Unknown request type: {request_type}",
            )

    def _encode_response(self, response: dict) -> bytes:
        """Encode a response with length prefix."""
        payload = encode_response(response)
        length = struct.pack("<I", len(payload))
        return length + payload

    def _encode_error(self, code: ErrorCode, message: str) -> bytes:
        """Encode an error response."""
        return self._encode_response(error(code, message))

    def _handle_init_context(self, request: dict) -> bytes:
        """Handle InitContext request."""
        repo_path = request.get("repo_path")
        python_path = request.get("python_path")

        if not repo_path:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing repo_path",
            )

        try:
            context = self.registry.create_context(repo_path, python_path)

            # Auto-collect on init
            context.collect()

            return self._encode_response(
                context_ready(context.context_id, context.inventory_hash)
            )
        except ValueError as e:
            return self._encode_error(ErrorCode.INVALID_REQUEST, str(e))
        except Exception as e:
            return self._encode_error(ErrorCode.COLLECTION_FAILED, str(e))

    def _handle_collect(self, request: dict) -> bytes:
        """Handle Collect request."""
        context_id = request.get("context_id")
        force = request.get("force", False)

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            node_count, duration_ms = context.collect(force)
            return self._encode_response(
                collection_complete(node_count, duration_ms)
            )
        except Exception as e:
            return self._encode_error(ErrorCode.COLLECTION_FAILED, str(e))

    def _handle_run(self, request: dict) -> bytes:
        """Handle Run request."""
        context_id = request.get("context_id")
        node_ids = request.get("node_ids", [])
        workers = request.get("workers")
        maxfail = request.get("maxfail")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            summary = context.run_tests(node_ids, maxfail, workers)
            return self._encode_response(
                run_complete(
                    total=summary.total,
                    passed=summary.passed,
                    failed=summary.failed,
                    skipped=summary.skipped,
                    errors=summary.errors,
                    duration_ms=summary.duration_ms,
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_list(self, request: dict) -> bytes:
        """Handle List request."""
        context_id = request.get("context_id")
        keyword = request.get("keyword")
        marker = request.get("marker")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            node_ids = context.list_tests(keyword, marker)
            return self._encode_response(test_list(node_ids))
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_get_inventory(self, request: dict) -> bytes:
        """Handle GetInventory request."""
        context_id = request.get("context_id")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            nodes = context.get_inventory_nodes()
            collected_at = int(context.last_collection_time * 1000)
            return self._encode_response(
                inventory_data(
                    hash=context.inventory_hash,
                    collected_at=collected_at,
                    nodes=nodes,
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_get_worker_status(self, request: dict) -> bytes:
        """Handle GetWorkerStatus request."""
        context_id = request.get("context_id")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            status = context.get_worker_status()
            return self._encode_response(
                worker_status(
                    active_workers=status["active_workers"],
                    idle_workers=status["idle_workers"],
                    tests_executed=status["tests_executed"],
                    avg_test_duration_ms=status["avg_test_duration_ms"],
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_configure_workers(self, request: dict) -> bytes:
        """Handle ConfigureWorkers request."""
        context_id = request.get("context_id")
        num_workers = request.get("num_workers")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        if num_workers is None:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing num_workers",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            actual_workers = context.configure_workers(num_workers)
            return self._encode_response(worker_config_ack(actual_workers))
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_run_stream(self, request: dict) -> bytes:
        """Handle RunStream request - start a streaming test run."""
        context_id = request.get("context_id")
        node_ids = request.get("node_ids", [])
        workers = request.get("workers")
        maxfail = request.get("maxfail")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            run = context.start_streaming_run(node_ids, workers, maxfail)
            return self._encode_response(
                run_started(run.run_id, run.total)
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_get_run_progress(self, request: dict) -> bytes:
        """Handle GetRunProgress request - poll for streaming run progress."""
        context_id = request.get("context_id")
        run_id = request.get("run_id")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        if not run_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing run_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        run = context.get_streaming_run(run_id)
        if not run:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Run not found: {run_id}",
            )

        try:
            # Get pending results
            pending = run.get_pending_results()
            result_infos = [
                make_test_result_info(
                    node_id=r.node_id,
                    outcome=r.outcome,
                    duration_ms=r.duration_ms,
                    message=r.message,
                )
                for r in pending
            ]

            return self._encode_response(
                run_progress(
                    run_id=run.run_id,
                    total=run.total,
                    completed=run.completed,
                    running=run.running,
                    done=run.done,
                    results=result_infos,
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_shutdown(self, request: dict) -> bytes:
        """Handle Shutdown request."""
        context_id = request.get("context_id")

        if context_id:
            # Shutdown specific context
            if self.registry.remove_context(context_id):
                return self._encode_response(shutdown_ack())
            else:
                return self._encode_error(
                    ErrorCode.CONTEXT_NOT_FOUND,
                    f"Context not found: {context_id}",
                )
        else:
            # Shutdown entire daemon
            self.registry.clear()
            self.stop()
            return self._encode_response(shutdown_ack())

    def _handle_ping(self, request: dict) -> bytes:
        """Handle Ping request."""
        return self._encode_response(pong())

    # --- Phase 5: Flakiness Handlers ---

    def _handle_get_flakiness_report(self, request: dict) -> bytes:
        """Handle GetFlakinessReport request."""
        context_id = request.get("context_id")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            report = context.get_flakiness_report()
            return self._encode_response(
                flakiness_report(
                    flaky_tests=report["flaky_tests"],
                    unstable_tests=report["unstable_tests"],
                    stable_count=report["stable_count"],
                    total_tracked=report["total_tracked"],
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_get_test_flakiness(self, request: dict) -> bytes:
        """Handle GetTestFlakiness request."""
        context_id = request.get("context_id")
        node_id = request.get("node_id")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        if not node_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing node_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            info = context.get_test_flakiness(node_id)
            if not info:
                return self._encode_response(
                    test_flakiness(
                        node_id=node_id,
                        failure_rate=0.0,
                        is_flaky=False,
                        flaky_streak=0,
                        consecutive_failures=0,
                        consecutive_passes=0,
                        total_runs=0,
                        recent_outcomes=[],
                    )
                )
            return self._encode_response(
                test_flakiness(
                    node_id=info["node_id"],
                    failure_rate=info["failure_rate"],
                    is_flaky=info["is_flaky"],
                    flaky_streak=info["flaky_streak"],
                    consecutive_failures=info["consecutive_failures"],
                    consecutive_passes=info["consecutive_passes"],
                    total_runs=info["total_runs"],
                    recent_outcomes=info["recent_outcomes"],
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_configure_rerun(self, request: dict) -> bytes:
        """Handle ConfigureRerun request."""
        context_id = request.get("context_id")
        enabled = request.get("enabled", True)
        max_reruns = request.get("max_reruns", 2)
        only_flaky = request.get("only_flaky", False)
        delay_ms = request.get("delay_ms", 0)

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            context.configure_rerun(enabled, max_reruns, only_flaky, delay_ms)
            cfg = context.get_rerun_config()
            return self._encode_response(
                config_ack("rerun", cfg)
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_get_rerun_config(self, request: dict) -> bytes:
        """Handle GetRerunConfig request."""
        context_id = request.get("context_id")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            cfg = context.get_rerun_config()
            return self._encode_response(
                rerun_config(
                    enabled=cfg["enabled"],
                    max_reruns=cfg["max_reruns"],
                    only_flaky=cfg["only_flaky"],
                    delay_ms=cfg["delay_ms"],
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_run_with_rerun(self, request: dict) -> bytes:
        """Handle RunWithRerun request - run tests with auto-rerun."""
        context_id = request.get("context_id")
        node_ids = request.get("node_ids", [])
        workers = request.get("workers")
        maxfail = request.get("maxfail")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            summary = context.run_tests_with_rerun(node_ids, maxfail, workers)
            return self._encode_response(
                run_complete(
                    total=summary.total,
                    passed=summary.passed,
                    failed=summary.failed,
                    skipped=summary.skipped,
                    errors=summary.errors,
                    duration_ms=summary.duration_ms,
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    # --- Phase 5: Fixture Handlers ---

    def _handle_configure_fixture_reuse(self, request: dict) -> bytes:
        """Handle ConfigureFixtureReuse request."""
        context_id = request.get("context_id")
        enabled = request.get("enabled", True)
        max_age_seconds = request.get("max_age_seconds", 600)
        teardown_on_conftest_change = request.get("teardown_on_conftest_change", True)

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            context.configure_fixture_reuse(
                enabled, max_age_seconds, teardown_on_conftest_change
            )
            cfg = context.get_fixture_config()
            return self._encode_response(
                config_ack("fixture_reuse", cfg)
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_get_fixture_config(self, request: dict) -> bytes:
        """Handle GetFixtureConfig request."""
        context_id = request.get("context_id")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            cfg = context.get_fixture_config()
            return self._encode_response(
                fixture_config(
                    enabled=cfg["enabled"],
                    max_fixture_age_seconds=cfg["max_fixture_age_seconds"],
                    teardown_on_conftest_change=cfg["teardown_on_conftest_change"],
                    teardown_on_test_file_change=cfg["teardown_on_test_file_change"],
                    scopes_to_reuse=cfg["scopes_to_reuse"],
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_get_session_status(self, request: dict) -> bytes:
        """Handle GetSessionStatus request."""
        context_id = request.get("context_id")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            status = context.get_session_status()
            if not status:
                return self._encode_response(
                    session_status(
                        session_id="",
                        repo_path=str(context.repo_path),
                        created_at=0.0,
                        last_run_at=0.0,
                        total_runs=0,
                        enabled=False,
                        fixtures={},
                    )
                )
            return self._encode_response(
                session_status(
                    session_id=status["session_id"],
                    repo_path=status["repo_path"],
                    created_at=status["created_at"],
                    last_run_at=status["last_run_at"],
                    total_runs=status["total_runs"],
                    enabled=status["enabled"],
                    fixtures=status["fixtures"],
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    # --- Phase 5: Sharding Handlers ---

    def _handle_get_shard(self, request: dict) -> bytes:
        """Handle GetShard request - get tests for a specific shard."""
        context_id = request.get("context_id")
        node_ids = request.get("node_ids", [])
        shard_index = request.get("shard_index")
        total_shards = request.get("total_shards")
        strategy = request.get("strategy", "duration_balanced")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        if shard_index is None or total_shards is None:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing shard_index or total_shards",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            # If no node_ids provided, use full inventory
            if not node_ids:
                node_ids = list(context.inventory.keys())

            shard_nodes = context.shard_tests(
                node_ids, shard_index, total_shards, strategy
            )
            return self._encode_response(
                sharded_tests(
                    shard_index=shard_index,
                    total_shards=total_shards,
                    node_ids=shard_nodes,
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))

    def _handle_get_shard_info(self, request: dict) -> bytes:
        """Handle GetShardInfo request - get sharding distribution info."""
        context_id = request.get("context_id")
        node_ids = request.get("node_ids", [])
        total_shards = request.get("total_shards")
        strategy = request.get("strategy", "duration_balanced")

        if not context_id:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing context_id",
            )

        if total_shards is None:
            return self._encode_error(
                ErrorCode.INVALID_REQUEST,
                "Missing total_shards",
            )

        context = self.registry.get_context(context_id)
        if not context:
            return self._encode_error(
                ErrorCode.CONTEXT_NOT_FOUND,
                f"Context not found: {context_id}",
            )

        try:
            # If no node_ids provided, use full inventory
            if not node_ids:
                node_ids = list(context.inventory.keys())

            info = context.get_shard_info(node_ids, total_shards, strategy)
            return self._encode_response(
                shard_info(
                    strategy=info["strategy"],
                    total_shards=info["total_shards"],
                    total_tests=info["total_tests"],
                    shard_test_counts=info["shard_test_counts"],
                    shard_durations_ms=info["shard_durations_ms"],
                    count_imbalance_percent=info["count_imbalance_percent"],
                    duration_imbalance_percent=info["duration_imbalance_percent"],
                    estimated_wall_time_ms=info["estimated_wall_time_ms"],
                )
            )
        except Exception as e:
            return self._encode_error(ErrorCode.INTERNAL_ERROR, str(e))


def run_daemon(socket_path: Optional[str] = None, idle_timeout: int = 0):
    """Run the daemon server.

    Args:
        socket_path: IPC socket path (e.g., "ipc:///tmp/rpytest.sock")
        idle_timeout: Seconds of inactivity before auto-shutdown (0 = disabled)
    """
    # Set up logging
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
    )

    server = DaemonServer(socket_path, idle_timeout=idle_timeout)

    # Handle signals
    def signal_handler(signum, frame):
        logger.info(f"Received signal {signum}")
        server.stop()

    signal.signal(signal.SIGINT, signal_handler)
    signal.signal(signal.SIGTERM, signal_handler)

    server.start()
