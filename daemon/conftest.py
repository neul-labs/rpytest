"""Root conftest.py for daemon tests.

This file ensures pytest only collects tests from the tests/ directory
and not from the rpytest_daemon/ package.
"""

collect_ignore = ["rpytest_daemon"]
