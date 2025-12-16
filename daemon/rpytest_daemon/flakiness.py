"""Flakiness detection and auto-rerun logic."""

import json
import logging
import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Tuple

logger = logging.getLogger(__name__)


@dataclass
class FlakinessRecord:
    """Record of test flakiness over time."""
    node_id: str
    outcomes: List[str] = field(default_factory=list)  # Last N outcomes
    consecutive_failures: int = 0
    consecutive_passes: int = 0
    flaky_streak: int = 0  # Times outcome flipped
    total_runs: int = 0
    last_failure_message: Optional[str] = None

    @property
    def failure_rate(self) -> float:
        """Calculate failure rate from recent history."""
        if not self.outcomes:
            return 0.0
        failures = sum(1 for o in self.outcomes if o in ("failed", "error"))
        return failures / len(self.outcomes)

    @property
    def is_flaky(self) -> bool:
        """Determine if test is considered flaky.

        A test is flaky if it has both passed and failed in recent history
        with at least 2 outcome flips.
        """
        if len(self.outcomes) < 3:
            return False
        has_pass = any(o == "passed" for o in self.outcomes)
        has_fail = any(o in ("failed", "error") for o in self.outcomes)
        return has_pass and has_fail and self.flaky_streak >= 2

    def record_outcome(self, outcome: str, message: Optional[str] = None):
        """Record a new outcome and update statistics."""
        prev_outcome = self.outcomes[-1] if self.outcomes else None

        self.outcomes.append(outcome)
        if len(self.outcomes) > 20:  # Keep last 20
            self.outcomes = self.outcomes[-20:]

        self.total_runs += 1

        # Update consecutive counters
        if outcome in ("failed", "error"):
            self.consecutive_failures += 1
            self.consecutive_passes = 0
            self.last_failure_message = message
        elif outcome == "passed":
            self.consecutive_passes += 1
            self.consecutive_failures = 0
        else:
            # skipped, xfail, xpass - reset both
            self.consecutive_failures = 0
            self.consecutive_passes = 0

        # Track flaky streaks (outcome flips)
        if prev_outcome is not None:
            prev_is_fail = prev_outcome in ("failed", "error")
            curr_is_fail = outcome in ("failed", "error")
            if prev_is_fail != curr_is_fail:
                self.flaky_streak += 1

    def to_dict(self) -> Dict:
        """Serialize to dict for persistence."""
        return {
            "node_id": self.node_id,
            "outcomes": self.outcomes,
            "consecutive_failures": self.consecutive_failures,
            "consecutive_passes": self.consecutive_passes,
            "flaky_streak": self.flaky_streak,
            "total_runs": self.total_runs,
            "last_failure_message": self.last_failure_message,
        }

    @classmethod
    def from_dict(cls, data: Dict) -> "FlakinessRecord":
        """Deserialize from dict."""
        return cls(
            node_id=data["node_id"],
            outcomes=data.get("outcomes", []),
            consecutive_failures=data.get("consecutive_failures", 0),
            consecutive_passes=data.get("consecutive_passes", 0),
            flaky_streak=data.get("flaky_streak", 0),
            total_runs=data.get("total_runs", 0),
            last_failure_message=data.get("last_failure_message"),
        )


@dataclass
class RerunResult:
    """Result of a rerun attempt for flaky test detection."""
    node_id: str
    original_outcome: str
    rerun_outcomes: List[str]
    final_outcome: str  # "passed" if any pass, else original
    is_flaky: bool
    message: Optional[str] = None

    @property
    def passed_on_rerun(self) -> bool:
        """Whether test passed after reruns."""
        return self.original_outcome != "passed" and self.final_outcome == "passed"


class FlakinessTracker:
    """Tracks test flakiness and manages auto-reruns."""

    def __init__(self, storage_path: Optional[Path] = None):
        self.records: Dict[str, FlakinessRecord] = {}
        self.storage_path = storage_path
        if storage_path:
            self._load()

    def _load(self):
        """Load flakiness records from disk."""
        if not self.storage_path or not self.storage_path.exists():
            return
        try:
            data = json.loads(self.storage_path.read_text())
            for item in data:
                record = FlakinessRecord.from_dict(item)
                self.records[record.node_id] = record
            logger.debug(f"Loaded {len(self.records)} flakiness records")
        except Exception as e:
            logger.warning(f"Failed to load flakiness data: {e}")

    def _save(self):
        """Save flakiness records to disk."""
        if not self.storage_path:
            return
        try:
            self.storage_path.parent.mkdir(parents=True, exist_ok=True)
            data = [r.to_dict() for r in self.records.values()]
            self.storage_path.write_text(json.dumps(data, indent=2))
            logger.debug(f"Saved {len(self.records)} flakiness records")
        except Exception as e:
            logger.warning(f"Failed to save flakiness data: {e}")

    def record_outcome(
        self,
        node_id: str,
        outcome: str,
        message: Optional[str] = None,
    ):
        """Record a test outcome."""
        if node_id not in self.records:
            self.records[node_id] = FlakinessRecord(node_id=node_id)
        self.records[node_id].record_outcome(outcome, message)
        self._save()

    def get_record(self, node_id: str) -> Optional[FlakinessRecord]:
        """Get flakiness record for a test."""
        return self.records.get(node_id)

    def get_flaky_tests(self) -> List[str]:
        """Get list of tests currently considered flaky."""
        return [nid for nid, r in self.records.items() if r.is_flaky]

    def get_failure_rate(self, node_id: str) -> float:
        """Get failure rate for a test."""
        record = self.records.get(node_id)
        return record.failure_rate if record else 0.0

    def should_rerun(
        self,
        node_id: str,
        outcome: str,
        max_reruns: int = 2,
    ) -> Tuple[bool, str]:
        """Determine if a failed test should be rerun.

        Args:
            node_id: Test node ID.
            outcome: Current outcome.
            max_reruns: Maximum reruns allowed.

        Returns:
            Tuple of (should_rerun, reason).
        """
        if outcome not in ("failed", "error"):
            return False, "not_failed"

        record = self.records.get(node_id)
        if not record:
            # New test, rerun on first failure
            return True, "first_failure"

        if record.is_flaky:
            # Known flaky test, always rerun
            return True, "known_flaky"

        if record.consecutive_passes >= 5:
            # Stable test that just failed, might be a real issue
            # Still rerun once to confirm
            return True, "regression_check"

        if record.failure_rate > 0.5:
            # High failure rate, probably a real bug
            return False, "high_failure_rate"

        return True, "default_rerun"

    def get_flakiness_report(self) -> Dict:
        """Get a summary report of flakiness data."""
        flaky = []
        unstable = []
        stable = []

        for nid, record in self.records.items():
            summary = {
                "node_id": nid,
                "failure_rate": record.failure_rate,
                "flaky_streak": record.flaky_streak,
                "total_runs": record.total_runs,
                "consecutive_failures": record.consecutive_failures,
                "consecutive_passes": record.consecutive_passes,
            }

            if record.is_flaky:
                flaky.append(summary)
            elif record.failure_rate > 0.1:
                unstable.append(summary)
            else:
                stable.append(summary)

        return {
            "flaky_tests": flaky,
            "unstable_tests": unstable,
            "stable_count": len(stable),
            "total_tracked": len(self.records),
        }


def determine_final_outcome(outcomes: List[str]) -> str:
    """Determine final outcome from multiple run attempts.

    Logic:
    - If any run passed, final is "passed" (flaky pass)
    - If all runs failed/errored, final is the majority
    - Skips/xfails are passed through
    """
    if not outcomes:
        return "unknown"

    if "passed" in outcomes:
        return "passed"

    if "skipped" in outcomes:
        return "skipped"

    if "xfail" in outcomes or "xpass" in outcomes:
        return outcomes[-1]  # Use last outcome

    # All failures/errors
    return outcomes[-1]


def annotate_flaky_output(output: str, is_flaky: bool, rerun_count: int) -> str:
    """Annotate test output with flakiness information."""
    if not is_flaky and rerun_count == 0:
        return output

    annotation = []
    if is_flaky:
        annotation.append("[FLAKY TEST]")
    if rerun_count > 0:
        annotation.append(f"[RERUN {rerun_count}x]")

    prefix = " ".join(annotation)
    return f"{prefix}\n{output}" if output else prefix
