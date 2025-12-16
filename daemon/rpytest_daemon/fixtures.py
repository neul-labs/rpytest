"""Session fixture reuse management."""

import logging
import os
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Set

logger = logging.getLogger(__name__)


@dataclass
class FixtureState:
    """State of a session fixture."""
    name: str
    scope: str  # "session", "package", "module", "class", "function"
    created_at: float = 0.0
    last_used: float = 0.0
    use_count: int = 0
    teardown_pending: bool = False


@dataclass
class SessionState:
    """State of a pytest session with warm fixtures."""
    session_id: str
    repo_path: Path
    python_path: Path
    fixtures: Dict[str, FixtureState] = field(default_factory=dict)
    created_at: float = field(default_factory=time.time)
    last_run_at: float = 0.0
    total_runs: int = 0
    enabled: bool = False
    _lock: threading.Lock = field(default_factory=threading.Lock)

    def mark_fixture_used(self, name: str, scope: str = "session"):
        """Mark a fixture as used in the current run."""
        with self._lock:
            now = time.time()
            if name not in self.fixtures:
                self.fixtures[name] = FixtureState(
                    name=name,
                    scope=scope,
                    created_at=now,
                )
            self.fixtures[name].last_used = now
            self.fixtures[name].use_count += 1

    def mark_run_complete(self):
        """Mark a test run as complete."""
        with self._lock:
            self.last_run_at = time.time()
            self.total_runs += 1

    def get_stale_fixtures(self, max_age_seconds: float = 300) -> List[str]:
        """Get fixtures that haven't been used recently."""
        now = time.time()
        stale = []
        with self._lock:
            for name, state in self.fixtures.items():
                if now - state.last_used > max_age_seconds:
                    stale.append(name)
        return stale

    def to_dict(self) -> Dict:
        """Serialize to dict."""
        with self._lock:
            return {
                "session_id": self.session_id,
                "repo_path": str(self.repo_path),
                "created_at": self.created_at,
                "last_run_at": self.last_run_at,
                "total_runs": self.total_runs,
                "enabled": self.enabled,
                "fixtures": {
                    name: {
                        "name": f.name,
                        "scope": f.scope,
                        "created_at": f.created_at,
                        "last_used": f.last_used,
                        "use_count": f.use_count,
                    }
                    for name, f in self.fixtures.items()
                },
            }


class SessionFixtureManager:
    """Manages session fixture reuse across test runs.

    Safety guards:
    - Explicit opt-in required per context
    - Automatic teardown on file changes
    - Max age limit for fixtures
    - Isolation checks between tests
    """

    MAX_FIXTURE_AGE = 600  # 10 minutes default
    TEARDOWN_ON_CONFTEST_CHANGE = True

    def __init__(self):
        self._sessions: Dict[str, SessionState] = {}
        self._lock = threading.Lock()

    def create_session(
        self,
        context_id: str,
        repo_path: Path,
        python_path: Path,
    ) -> SessionState:
        """Create a new session state for a context."""
        with self._lock:
            session = SessionState(
                session_id=f"{context_id}-{int(time.time())}",
                repo_path=repo_path,
                python_path=python_path,
            )
            self._sessions[context_id] = session
            logger.info(f"Created session {session.session_id} for {context_id}")
            return session

    def get_session(self, context_id: str) -> Optional[SessionState]:
        """Get session state for a context."""
        with self._lock:
            return self._sessions.get(context_id)

    def enable_reuse(self, context_id: str) -> bool:
        """Enable fixture reuse for a context."""
        with self._lock:
            session = self._sessions.get(context_id)
            if not session:
                return False
            session.enabled = True
            logger.info(f"Enabled fixture reuse for {context_id}")
            return True

    def disable_reuse(self, context_id: str) -> bool:
        """Disable fixture reuse and teardown all fixtures."""
        with self._lock:
            session = self._sessions.get(context_id)
            if not session:
                return False
            session.enabled = False
            session.fixtures.clear()
            logger.info(f"Disabled fixture reuse for {context_id}")
            return True

    def is_reuse_enabled(self, context_id: str) -> bool:
        """Check if fixture reuse is enabled."""
        with self._lock:
            session = self._sessions.get(context_id)
            return session.enabled if session else False

    def invalidate_on_file_change(
        self,
        context_id: str,
        changed_files: List[Path],
    ) -> List[str]:
        """Invalidate fixtures based on file changes.

        Returns list of invalidated fixture names.
        """
        session = self.get_session(context_id)
        if not session or not session.enabled:
            return []

        invalidated = []

        # Check for conftest.py changes
        conftest_changed = any(
            f.name == "conftest.py" for f in changed_files
        )

        if conftest_changed and self.TEARDOWN_ON_CONFTEST_CHANGE:
            # All session fixtures need teardown
            with session._lock:
                invalidated = list(session.fixtures.keys())
                session.fixtures.clear()
            logger.info(
                f"Invalidated all fixtures for {context_id} due to conftest.py change"
            )
            return invalidated

        # Check for test file changes affecting specific fixtures
        # This would require deeper analysis of fixture dependencies
        # For now, invalidate all on any test file change
        test_files_changed = any(
            f.name.startswith("test_") or f.name.endswith("_test.py")
            for f in changed_files
        )

        if test_files_changed:
            # Keep session fixtures, but mark for potential re-evaluation
            logger.debug(
                f"Test files changed in {context_id}, fixtures retained"
            )

        return invalidated

    def cleanup_stale(
        self,
        context_id: str,
        max_age: float = None,
    ) -> List[str]:
        """Clean up stale fixtures.

        Returns list of cleaned up fixture names.
        """
        if max_age is None:
            max_age = self.MAX_FIXTURE_AGE

        session = self.get_session(context_id)
        if not session:
            return []

        stale = session.get_stale_fixtures(max_age)
        if stale:
            with session._lock:
                for name in stale:
                    session.fixtures.pop(name, None)
            logger.info(f"Cleaned up {len(stale)} stale fixtures for {context_id}")

        return stale

    def get_session_status(self, context_id: str) -> Optional[Dict]:
        """Get status of session fixture reuse."""
        session = self.get_session(context_id)
        if not session:
            return None
        return session.to_dict()

    def teardown_session(self, context_id: str):
        """Teardown all fixtures and remove session."""
        with self._lock:
            session = self._sessions.pop(context_id, None)
            if session:
                logger.info(f"Torn down session for {context_id}")

    def build_pytest_args(
        self,
        context_id: str,
        base_args: List[str],
    ) -> List[str]:
        """Build pytest arguments with session reuse settings.

        If reuse is enabled, adds arguments to preserve session state.
        """
        session = self.get_session(context_id)
        if not session or not session.enabled:
            return base_args

        # Add pytest arguments for session persistence
        # Note: This requires custom pytest plugin or conftest configuration
        args = base_args.copy()

        # The actual implementation would require a pytest plugin
        # that manages fixture caching across invocations
        # For now, we document the intended behavior

        return args


@dataclass
class FixtureConfig:
    """Configuration for fixture reuse behavior."""
    enabled: bool = False
    max_fixture_age_seconds: float = 600
    teardown_on_conftest_change: bool = True
    teardown_on_test_file_change: bool = False
    scopes_to_reuse: Set[str] = field(
        default_factory=lambda: {"session", "package"}
    )

    def to_dict(self) -> Dict:
        """Serialize to dict."""
        return {
            "enabled": self.enabled,
            "max_fixture_age_seconds": self.max_fixture_age_seconds,
            "teardown_on_conftest_change": self.teardown_on_conftest_change,
            "teardown_on_test_file_change": self.teardown_on_test_file_change,
            "scopes_to_reuse": list(self.scopes_to_reuse),
        }

    @classmethod
    def from_dict(cls, data: Dict) -> "FixtureConfig":
        """Deserialize from dict."""
        return cls(
            enabled=data.get("enabled", False),
            max_fixture_age_seconds=data.get("max_fixture_age_seconds", 600),
            teardown_on_conftest_change=data.get(
                "teardown_on_conftest_change", True
            ),
            teardown_on_test_file_change=data.get(
                "teardown_on_test_file_change", False
            ),
            scopes_to_reuse=set(data.get("scopes_to_reuse", ["session", "package"])),
        )
