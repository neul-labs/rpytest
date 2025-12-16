"""pytest plugin for rpytest integration.

This plugin allows pytest to communicate with the rpytest daemon
for enhanced performance and caching.
"""

import os
import pytest


def pytest_configure(config):
    """Configure pytest to use rpytest if available."""
    # Register custom markers
    config.addinivalue_line(
        "markers", "rpytest_skip: Skip this test when running under rpytest"
    )
    config.addinivalue_line(
        "markers", "rpytest_only: Only run this test when running under rpytest"
    )


def pytest_collection_modifyitems(config, items):
    """Modify collected items based on rpytest markers."""
    running_under_rpytest = os.environ.get("RPYTEST") == "1"

    skip_rpytest = pytest.mark.skip(reason="Skipped when running under rpytest")
    skip_pytest = pytest.mark.skip(reason="Only runs under rpytest")

    for item in items:
        if running_under_rpytest:
            # Skip tests marked with rpytest_skip
            if "rpytest_skip" in item.keywords:
                item.add_marker(skip_rpytest)
        else:
            # Skip tests marked with rpytest_only when not using rpytest
            if "rpytest_only" in item.keywords:
                item.add_marker(skip_pytest)


def pytest_report_header(config):
    """Add rpytest info to the pytest header."""
    if os.environ.get("RPYTEST") == "1":
        return ["rpytest: enabled (daemon mode)"]
    return []
