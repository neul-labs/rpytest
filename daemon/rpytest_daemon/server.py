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
    context_ready,
    decode_request,
    encode_response,
    error,
    pong,
    run_complete,
    shutdown_ack,
    test_list,
)

logger = logging.getLogger(__name__)


def get_default_socket_path() -> str:
    """Get the default socket path."""
    runtime_dir = os.environ.get("XDG_RUNTIME_DIR", "/tmp")
    return f"ipc://{runtime_dir}/rpytest.sock"


class DaemonServer:
    """NNG-based RPC server for rpytest."""

    def __init__(self, socket_path: Optional[str] = None):
        self.socket_path = socket_path or get_default_socket_path()
        self.registry = ContextRegistry()
        self.running = False
        self._socket: Optional[pynng.Rep0] = None

    def start(self):
        """Start the daemon server."""
        logger.info(f"Starting daemon at {self.socket_path}")

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
        logger.info("Daemon started, waiting for connections")

        # Main loop
        while self.running:
            try:
                # Receive request with length prefix
                data = self._socket.recv()
                response = self._handle_request(data)
                self._socket.send(response)
            except pynng.Timeout:
                # Timeout is expected, continue loop
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
            "shutdown": self._handle_shutdown,
            "ping": self._handle_ping,
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
            summary = context.run_tests(node_ids, maxfail)
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


def run_daemon(socket_path: Optional[str] = None):
    """Run the daemon server."""
    # Set up logging
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
    )

    server = DaemonServer(socket_path)

    # Handle signals
    def signal_handler(signum, frame):
        logger.info(f"Received signal {signum}")
        server.stop()

    signal.signal(signal.SIGINT, signal_handler)
    signal.signal(signal.SIGTERM, signal_handler)

    server.start()
