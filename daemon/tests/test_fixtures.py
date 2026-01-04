"""Tests for the session fixture reuse module."""

import tempfile
import time
from pathlib import Path

import pytest

from rpytest_daemon.fixtures import (
    FixtureState,
    SessionState,
    SessionFixtureManager,
    FixtureConfig,
)


class TestFixtureState:
    """Tests for FixtureState dataclass."""

    def test_basic_creation(self):
        state = FixtureState(name="db_connection", scope="session")
        assert state.name == "db_connection"
        assert state.scope == "session"
        assert state.created_at == 0.0
        assert state.last_used == 0.0
        assert state.use_count == 0
        assert state.teardown_pending is False

    def test_with_values(self):
        now = time.time()
        state = FixtureState(
            name="cache",
            scope="module",
            created_at=now,
            last_used=now,
            use_count=5,
            teardown_pending=True,
        )
        assert state.use_count == 5
        assert state.teardown_pending is True


class TestSessionState:
    """Tests for SessionState dataclass."""

    def test_basic_creation(self):
        state = SessionState(
            session_id="sess-123",
            repo_path=Path("/path/to/repo"),
            python_path=Path("/usr/bin/python3"),
        )
        assert state.session_id == "sess-123"
        assert state.repo_path == Path("/path/to/repo")
        assert state.fixtures == {}
        assert state.total_runs == 0
        assert state.enabled is False

    def test_mark_fixture_used_new(self):
        state = SessionState(
            session_id="sess-123",
            repo_path=Path("/path/to/repo"),
            python_path=Path("/usr/bin/python3"),
        )

        state.mark_fixture_used("db", scope="session")

        assert "db" in state.fixtures
        assert state.fixtures["db"].use_count == 1
        assert state.fixtures["db"].scope == "session"

    def test_mark_fixture_used_existing(self):
        state = SessionState(
            session_id="sess-123",
            repo_path=Path("/path/to/repo"),
            python_path=Path("/usr/bin/python3"),
        )

        state.mark_fixture_used("db")
        state.mark_fixture_used("db")
        state.mark_fixture_used("db")

        assert state.fixtures["db"].use_count == 3

    def test_mark_run_complete(self):
        state = SessionState(
            session_id="sess-123",
            repo_path=Path("/path/to/repo"),
            python_path=Path("/usr/bin/python3"),
        )

        assert state.total_runs == 0
        state.mark_run_complete()
        assert state.total_runs == 1
        assert state.last_run_at > 0

    def test_get_stale_fixtures(self):
        state = SessionState(
            session_id="sess-123",
            repo_path=Path("/path/to/repo"),
            python_path=Path("/usr/bin/python3"),
        )

        # Add a fixture with old timestamp
        old_time = time.time() - 400  # 400 seconds ago
        state.fixtures["old_fixture"] = FixtureState(
            name="old_fixture",
            scope="session",
            last_used=old_time,
        )

        # Add a fixture with recent timestamp
        state.mark_fixture_used("recent_fixture")

        stale = state.get_stale_fixtures(max_age_seconds=300)

        assert "old_fixture" in stale
        assert "recent_fixture" not in stale

    def test_to_dict(self):
        state = SessionState(
            session_id="sess-123",
            repo_path=Path("/path/to/repo"),
            python_path=Path("/usr/bin/python3"),
        )
        state.mark_fixture_used("db")
        state.enabled = True

        result = state.to_dict()

        assert result["session_id"] == "sess-123"
        assert result["repo_path"] == "/path/to/repo"
        assert result["enabled"] is True
        assert "db" in result["fixtures"]


class TestSessionFixtureManager:
    """Tests for SessionFixtureManager class."""

    def test_create_session(self):
        manager = SessionFixtureManager()
        session = manager.create_session(
            context_id="ctx-1",
            repo_path=Path("/repo"),
            python_path=Path("/python"),
        )

        assert session.repo_path == Path("/repo")
        assert "ctx-1" in session.session_id

    def test_get_session_existing(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))

        session = manager.get_session("ctx-1")
        assert session is not None

    def test_get_session_nonexistent(self):
        manager = SessionFixtureManager()
        session = manager.get_session("ctx-unknown")
        assert session is None

    def test_enable_reuse(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))

        result = manager.enable_reuse("ctx-1")
        assert result is True
        assert manager.is_reuse_enabled("ctx-1") is True

    def test_enable_reuse_nonexistent(self):
        manager = SessionFixtureManager()
        result = manager.enable_reuse("ctx-unknown")
        assert result is False

    def test_disable_reuse(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))
        manager.enable_reuse("ctx-1")

        result = manager.disable_reuse("ctx-1")
        assert result is True
        assert manager.is_reuse_enabled("ctx-1") is False

    def test_disable_reuse_clears_fixtures(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))
        manager.enable_reuse("ctx-1")

        session = manager.get_session("ctx-1")
        session.mark_fixture_used("db")
        assert len(session.fixtures) == 1

        manager.disable_reuse("ctx-1")
        assert len(session.fixtures) == 0

    def test_is_reuse_enabled_nonexistent(self):
        manager = SessionFixtureManager()
        result = manager.is_reuse_enabled("ctx-unknown")
        assert result is False

    def test_invalidate_on_file_change_conftest(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))
        manager.enable_reuse("ctx-1")

        session = manager.get_session("ctx-1")
        session.mark_fixture_used("db")
        session.mark_fixture_used("cache")

        invalidated = manager.invalidate_on_file_change(
            "ctx-1",
            [Path("/repo/conftest.py")],
        )

        assert "db" in invalidated
        assert "cache" in invalidated
        assert len(session.fixtures) == 0

    def test_invalidate_on_file_change_test_file(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))
        manager.enable_reuse("ctx-1")

        session = manager.get_session("ctx-1")
        session.mark_fixture_used("db")

        # Test file changes don't invalidate by default
        invalidated = manager.invalidate_on_file_change(
            "ctx-1",
            [Path("/repo/test_something.py")],
        )

        assert invalidated == []
        assert len(session.fixtures) == 1

    def test_invalidate_on_file_change_not_enabled(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))
        # Not enabled

        invalidated = manager.invalidate_on_file_change(
            "ctx-1",
            [Path("/repo/conftest.py")],
        )

        assert invalidated == []

    def test_cleanup_stale(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))

        session = manager.get_session("ctx-1")

        # Add old fixture
        old_time = time.time() - 700
        session.fixtures["old"] = FixtureState(
            name="old", scope="session", last_used=old_time
        )

        # Add recent fixture
        session.mark_fixture_used("recent")

        cleaned = manager.cleanup_stale("ctx-1", max_age=600)

        assert "old" in cleaned
        assert "old" not in session.fixtures
        assert "recent" in session.fixtures

    def test_get_session_status(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))
        manager.enable_reuse("ctx-1")

        status = manager.get_session_status("ctx-1")

        assert status is not None
        assert status["enabled"] is True
        assert status["repo_path"] == "/repo"

    def test_get_session_status_nonexistent(self):
        manager = SessionFixtureManager()
        status = manager.get_session_status("ctx-unknown")
        assert status is None

    def test_teardown_session(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))

        manager.teardown_session("ctx-1")

        assert manager.get_session("ctx-1") is None

    def test_build_pytest_args_not_enabled(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))

        args = manager.build_pytest_args("ctx-1", ["-v", "--tb=short"])

        assert args == ["-v", "--tb=short"]

    def test_build_pytest_args_enabled(self):
        manager = SessionFixtureManager()
        manager.create_session("ctx-1", Path("/repo"), Path("/python"))
        manager.enable_reuse("ctx-1")

        args = manager.build_pytest_args("ctx-1", ["-v"])

        # Currently returns same args, but structure is in place for extension
        assert "-v" in args


class TestFixtureConfig:
    """Tests for FixtureConfig dataclass."""

    def test_defaults(self):
        config = FixtureConfig()
        assert config.enabled is False
        assert config.max_fixture_age_seconds == 600
        assert config.teardown_on_conftest_change is True
        assert config.teardown_on_test_file_change is False
        assert "session" in config.scopes_to_reuse
        assert "package" in config.scopes_to_reuse

    def test_custom_values(self):
        config = FixtureConfig(
            enabled=True,
            max_fixture_age_seconds=1800,
            teardown_on_conftest_change=False,
            teardown_on_test_file_change=True,
            scopes_to_reuse={"session"},
        )
        assert config.enabled is True
        assert config.max_fixture_age_seconds == 1800
        assert config.scopes_to_reuse == {"session"}

    def test_to_dict(self):
        config = FixtureConfig(
            enabled=True,
            max_fixture_age_seconds=300,
            scopes_to_reuse={"session", "module"},
        )
        result = config.to_dict()

        assert result["enabled"] is True
        assert result["max_fixture_age_seconds"] == 300
        assert set(result["scopes_to_reuse"]) == {"session", "module"}

    def test_from_dict(self):
        data = {
            "enabled": True,
            "max_fixture_age_seconds": 900,
            "teardown_on_conftest_change": False,
            "teardown_on_test_file_change": True,
            "scopes_to_reuse": ["session"],
        }
        config = FixtureConfig.from_dict(data)

        assert config.enabled is True
        assert config.max_fixture_age_seconds == 900
        assert config.teardown_on_conftest_change is False
        assert config.scopes_to_reuse == {"session"}

    def test_from_dict_defaults(self):
        data = {}
        config = FixtureConfig.from_dict(data)

        assert config.enabled is False
        assert config.max_fixture_age_seconds == 600
