"""CLI entry point for rpytest daemon."""

import argparse
import logging
import os
import sys

from .server import get_default_socket_path, run_daemon


def main():
    """Main entry point."""
    parser = argparse.ArgumentParser(
        description="rpytest daemon - Python test execution service",
    )
    parser.add_argument(
        "--socket",
        "-s",
        default=None,
        help=f"Socket path (default: {get_default_socket_path()})",
    )
    parser.add_argument(
        "--verbose",
        "-v",
        action="count",
        default=0,
        help="Increase verbosity",
    )
    parser.add_argument(
        "--version",
        action="version",
        version="%(prog)s 0.1.0",
    )

    args = parser.parse_args()

    # Set log level based on verbosity
    if args.verbose >= 2:
        log_level = logging.DEBUG
    elif args.verbose >= 1:
        log_level = logging.INFO
    else:
        log_level = logging.WARNING

    logging.basicConfig(
        level=log_level,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
    )

    # Run the daemon
    socket_path = args.socket
    if socket_path and not socket_path.startswith("ipc://"):
        socket_path = f"ipc://{socket_path}"

    try:
        run_daemon(socket_path)
    except KeyboardInterrupt:
        print("\nInterrupted", file=sys.stderr)
        sys.exit(130)
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
