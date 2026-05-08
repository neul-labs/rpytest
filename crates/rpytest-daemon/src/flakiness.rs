//! Flakiness detection and tracking with explicit stability state machine.

use crate::error::Result;
use crate::models::{FlakinessRecord, StabilityState, TestOutcome};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::debug;

impl StabilityState {
    /// Apply an outcome and return the new state.
    ///
    /// The `prev_outcome` is the most recent previous outcome (if any).
    pub fn transition(self, outcome: &TestOutcome, prev_outcome: Option<&TestOutcome>) -> Self {
        match self {
            StabilityState::Unknown => match outcome {
                TestOutcome::Passed => StabilityState::Stable {
                    consecutive_passes: 1,
                },
                TestOutcome::Failed | TestOutcome::Error => StabilityState::Unstable {
                    consecutive_failures: 1,
                },
                // Skipped, xfail, xpass don't contribute to stability tracking
                _ => StabilityState::Unknown,
            },
            StabilityState::Stable { consecutive_passes } => match outcome {
                TestOutcome::Passed => StabilityState::Stable {
                    consecutive_passes: consecutive_passes + 1,
                },
                TestOutcome::Failed | TestOutcome::Error => {
                    // First failure after stable streak: enter flaky
                    StabilityState::Flaky { streak_count: 1 }
                }
                _ => StabilityState::Unknown,
            },
            StabilityState::Unstable {
                consecutive_failures,
            } => match outcome {
                TestOutcome::Passed => {
                    // First pass after unstable streak: enter flaky
                    StabilityState::Flaky { streak_count: 1 }
                }
                TestOutcome::Failed | TestOutcome::Error => StabilityState::Unstable {
                    consecutive_failures: consecutive_failures + 1,
                },
                _ => StabilityState::Unknown,
            },
            StabilityState::Flaky { streak_count } => {
                // Determine if this outcome is a flip from the previous
                let is_flip = prev_outcome.map_or(false, |prev| {
                    let prev_is_fail = matches!(prev, TestOutcome::Failed | TestOutcome::Error);
                    let curr_is_fail = matches!(outcome, TestOutcome::Failed | TestOutcome::Error);
                    prev_is_fail != curr_is_fail
                });

                if is_flip {
                    let new_streak = streak_count + 1;
                    if new_streak >= 3 {
                        // Promote to confirmed flaky after 3+ flips
                        StabilityState::ConfirmedFlaky
                    } else {
                        StabilityState::Flaky {
                            streak_count: new_streak,
                        }
                    }
                } else {
                    // No flip: transition to stable or unstable based on current outcome
                    match outcome {
                        TestOutcome::Passed => StabilityState::Stable {
                            consecutive_passes: 1,
                        },
                        TestOutcome::Failed | TestOutcome::Error => StabilityState::Unstable {
                            consecutive_failures: 1,
                        },
                        _ => StabilityState::Unknown,
                    }
                }
            }
            StabilityState::ConfirmedFlaky => {
                // Once confirmed flaky, stay confirmed unless we see a long stable streak
                match outcome {
                    TestOutcome::Passed => {
                        // Check if previous was also passed (no flip)
                        let prev_was_pass =
                            prev_outcome.map_or(false, |prev| matches!(prev, TestOutcome::Passed));
                        if prev_was_pass {
                            // Start counting stable streak
                            StabilityState::Stable {
                                consecutive_passes: 2, // this pass + previous pass
                            }
                        } else {
                            StabilityState::ConfirmedFlaky
                        }
                    }
                    TestOutcome::Failed | TestOutcome::Error => {
                        let prev_was_fail = prev_outcome.map_or(false, |prev| {
                            matches!(prev, TestOutcome::Failed | TestOutcome::Error)
                        });
                        if prev_was_fail {
                            StabilityState::Unstable {
                                consecutive_failures: 2,
                            }
                        } else {
                            StabilityState::ConfirmedFlaky
                        }
                    }
                    _ => StabilityState::ConfirmedFlaky,
                }
            }
        }
    }
}

/// Tracks test flakiness and manages auto-reruns.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlakinessTracker {
    /// Flakiness records by node_id
    records: HashMap<String, FlakinessRecord>,
    /// Maximum number of outcomes to keep per test
    max_outcomes: usize,
    /// Path to persist data
    storage_path: Option<PathBuf>,
    /// Dirty flag for buffered writes
    #[serde(skip)]
    dirty: bool,
}

impl FlakinessTracker {
    /// Create a new tracker.
    pub fn new(storage_path: Option<PathBuf>) -> Self {
        let mut tracker = FlakinessTracker {
            records: HashMap::new(),
            max_outcomes: 20,
            storage_path,
            dirty: false,
        };

        // Load from disk if path provided
        if let Some(ref path) = tracker.storage_path {
            if path.exists() {
                let _ = tracker.load();
            }
        }

        tracker
    }

    /// Record a test outcome and update statistics.
    ///
    /// Updates the stability state machine for the test.
    pub fn record_outcome(&mut self, node_id: &str, outcome: TestOutcome, message: Option<&str>) {
        let record = self
            .records
            .entry(node_id.to_string())
            .or_insert_with(|| FlakinessRecord {
                node_id: node_id.to_string(),
                outcomes: Vec::new(),
                consecutive_failures: 0,
                consecutive_passes: 0,
                flaky_streak: 0,
                total_runs: 0,
                last_failure_message: None,
                stability: StabilityState::Unknown,
            });

        let prev_outcome = record.outcomes.last().cloned();

        record.outcomes.push(outcome.clone().into());

        // Keep only last N outcomes
        if record.outcomes.len() > self.max_outcomes {
            record.outcomes = record.outcomes[record.outcomes.len() - self.max_outcomes..].to_vec();
        }

        record.total_runs += 1;

        // Update stability state machine
        let prev_for_transition = prev_outcome.as_ref().and_then(|s| match s.as_str() {
            "passed" => Some(TestOutcome::Passed),
            "failed" => Some(TestOutcome::Failed),
            "error" => Some(TestOutcome::Error),
            "skipped" => Some(TestOutcome::Skipped),
            "xfail" => Some(TestOutcome::Xfail),
            "xpass" => Some(TestOutcome::Xpass),
            _ => None,
        });

        let old_stability = std::mem::replace(
            &mut record.stability,
            StabilityState::Unknown, // temporary placeholder
        );
        record.stability = old_stability.transition(&outcome, prev_for_transition.as_ref());

        // Update legacy counters for backward compatibility with serialization
        match outcome {
            TestOutcome::Failed | TestOutcome::Error => {
                record.consecutive_failures += 1;
                record.consecutive_passes = 0;
                record.last_failure_message = message.map(|s| s.to_string());
            }
            TestOutcome::Passed => {
                record.consecutive_passes += 1;
                record.consecutive_failures = 0;
            }
            _ => {
                // skipped, xfail, xpass - reset both
                record.consecutive_failures = 0;
                record.consecutive_passes = 0;
            }
        }

        // Track flaky streaks (outcome flips) for backward compatibility
        if let Some(ref prev) = prev_outcome {
            let prev_str = prev.as_str();
            let prev_is_fail = prev_str == "failed" || prev_str == "error";
            let curr_is_fail = matches!(outcome, TestOutcome::Failed | TestOutcome::Error);
            if prev_is_fail != curr_is_fail {
                record.flaky_streak += 1;
            }
        }

        // Mark as dirty - caller should call flush_if_dirty() periodically
        self.dirty = true;
    }

    /// Get flakiness record for a test.
    pub fn get_record(&self, node_id: &str) -> Option<&FlakinessRecord> {
        self.records.get(node_id)
    }

    /// Check if a test is considered flaky.
    pub fn is_flaky(&self, node_id: &str) -> bool {
        self.records
            .get(node_id)
            .map_or(false, |r| r.stability.is_flaky())
    }

    /// Get the stability state for a test.
    pub fn stability_state(&self, node_id: &str) -> StabilityState {
        self.records
            .get(node_id)
            .map_or(StabilityState::Unknown, |r| r.stability.clone())
    }

    /// Internal check for flakiness based on record (legacy heuristic).
    fn check_is_flaky(record: &FlakinessRecord) -> bool {
        if record.outcomes.len() < 3 {
            return false;
        }
        let has_pass = record.outcomes.iter().any(|o| o == "passed");
        let has_fail = record
            .outcomes
            .iter()
            .any(|o| o == "failed" || o == "error");
        has_pass && has_fail && record.flaky_streak >= 2
    }

    /// Get failure rate for a test (0.0 - 1.0).
    pub fn get_failure_rate(&self, node_id: &str) -> f64 {
        if let Some(record) = self.records.get(node_id) {
            if record.outcomes.is_empty() {
                return 0.0;
            }
            let failures: usize = record
                .outcomes
                .iter()
                .filter(|o| o.as_str() == "failed" || o.as_str() == "error")
                .count();
            failures as f64 / record.outcomes.len() as f64
        } else {
            0.0
        }
    }

    /// Get all flaky tests.
    pub fn get_flaky_tests(&self) -> Vec<&FlakinessRecord> {
        self.records
            .values()
            .filter(|r| r.stability.is_flaky())
            .collect()
    }

    /// Get unstable tests (some failures but not flaky).
    pub fn get_unstable_tests(&self) -> Vec<&FlakinessRecord> {
        self.records
            .values()
            .filter(|r| {
                let has_fail = r
                    .outcomes
                    .iter()
                    .any(|o| o.as_str() == "failed" || o.as_str() == "error");
                has_fail && !r.stability.is_flaky()
            })
            .collect()
    }

    /// Get count of stable tests.
    pub fn stable_count(&self) -> usize {
        self.records
            .values()
            .filter(|r| {
                !r.stability.is_flaky()
                    && !r
                        .outcomes
                        .iter()
                        .any(|o| o.as_str() == "failed" || o.as_str() == "error")
            })
            .count()
    }

    /// Get total tracked tests.
    pub fn total_tracked(&self) -> usize {
        self.records.len()
    }

    /// Save to disk.
    pub fn save(&self) -> Result<()> {
        if let Some(ref path) = self.storage_path {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let data: Vec<_> = self.records.values().collect();
            let json = serde_json::to_string_pretty(&data)?;
            std::fs::write(path, json)?;
            debug!("Saved {} flakiness records", self.records.len());
        }
        Ok(())
    }

    /// Flush to disk only if there are pending changes.
    /// This is more efficient than calling save() after every record_outcome().
    pub fn flush_if_dirty(&mut self) -> Result<()> {
        if self.dirty {
            self.save()?;
            self.dirty = false;
        }
        Ok(())
    }

    /// Check if there are unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Load from disk.
    pub fn load(&mut self) -> Result<()> {
        if let Some(ref path) = self.storage_path {
            if !path.exists() {
                return Ok(());
            }
            let json = std::fs::read_to_string(path)?;
            let data: Vec<FlakinessRecord> = serde_json::from_str(&json)?;
            self.records.clear();
            for record in data {
                self.records.insert(record.node_id.clone(), record);
            }
            debug!("Loaded {} flakiness records", self.records.len());
        }
        Ok(())
    }

    /// Clear all records.
    pub fn clear(&mut self) {
        self.records.clear();
        if let Some(ref path) = self.storage_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_stability_state_transitions() {
        // Unknown -> Stable
        let state = StabilityState::Unknown.transition(&TestOutcome::Passed, None);
        assert!(matches!(
            state,
            StabilityState::Stable {
                consecutive_passes: 1
            }
        ));

        // Unknown -> Unstable
        let state = StabilityState::Unknown.transition(&TestOutcome::Failed, None);
        assert!(matches!(
            state,
            StabilityState::Unstable {
                consecutive_failures: 1
            }
        ));

        // Stable -> Flaky (first failure)
        let state = StabilityState::Stable {
            consecutive_passes: 3,
        }
        .transition(&TestOutcome::Failed, Some(&TestOutcome::Passed));
        assert!(matches!(state, StabilityState::Flaky { streak_count: 1 }));

        // Unstable -> Flaky (first pass)
        let state = StabilityState::Unstable {
            consecutive_failures: 2,
        }
        .transition(&TestOutcome::Passed, Some(&TestOutcome::Failed));
        assert!(matches!(state, StabilityState::Flaky { streak_count: 1 }));

        // Flaky with flip -> Flaky (streak increases)
        let state = StabilityState::Flaky { streak_count: 1 }
            .transition(&TestOutcome::Failed, Some(&TestOutcome::Passed));
        assert!(matches!(state, StabilityState::Flaky { streak_count: 2 }));

        // Flaky with enough flips -> ConfirmedFlaky
        let state = StabilityState::Flaky { streak_count: 2 }
            .transition(&TestOutcome::Passed, Some(&TestOutcome::Failed));
        assert!(matches!(state, StabilityState::ConfirmedFlaky));

        // Flaky without flip -> Stable
        let state = StabilityState::Flaky { streak_count: 1 }
            .transition(&TestOutcome::Passed, Some(&TestOutcome::Passed));
        assert!(matches!(
            state,
            StabilityState::Stable {
                consecutive_passes: 1
            }
        ));

        // Flaky without flip -> Unstable
        let state = StabilityState::Flaky { streak_count: 1 }
            .transition(&TestOutcome::Failed, Some(&TestOutcome::Failed));
        assert!(matches!(
            state,
            StabilityState::Unstable {
                consecutive_failures: 1
            }
        ));

        // Skipped resets to Unknown
        let state = StabilityState::Stable {
            consecutive_passes: 5,
        }
        .transition(&TestOutcome::Skipped, Some(&TestOutcome::Passed));
        assert!(matches!(state, StabilityState::Unknown));
    }

    #[test]
    fn test_record_outcome() {
        let dir = TempDir::new().unwrap();
        let storage_path = dir.path().join("flakiness.json");
        let mut tracker = FlakinessTracker::new(Some(storage_path));

        tracker.record_outcome("test_a", TestOutcome::Passed, None);
        tracker.record_outcome("test_a", TestOutcome::Failed, Some("error"));

        assert_eq!(tracker.total_tracked(), 1);
        assert!(!tracker.is_flaky("test_a")); // Need more outcomes for flaky state

        let state = tracker.stability_state("test_a");
        assert!(matches!(state, StabilityState::Flaky { streak_count: 1 }));
    }

    #[test]
    fn test_flaky_detection() {
        let dir = TempDir::new().unwrap();
        let storage_path = dir.path().join("flakiness.json");
        let mut tracker = FlakinessTracker::new(Some(storage_path));

        // Record alternating outcomes
        tracker.record_outcome("test_a", TestOutcome::Passed, None);
        tracker.record_outcome("test_a", TestOutcome::Failed, None);
        tracker.record_outcome("test_a", TestOutcome::Passed, None);
        tracker.record_outcome("test_a", TestOutcome::Failed, None);
        tracker.record_outcome("test_a", TestOutcome::Passed, None);

        assert!(tracker.is_flaky("test_a"));
        assert_eq!(tracker.get_failure_rate("test_a"), 0.4);

        let state = tracker.stability_state("test_a");
        assert!(matches!(state, StabilityState::ConfirmedFlaky));
    }

    #[test]
    fn test_get_flaky_tests() {
        let dir = TempDir::new().unwrap();
        let storage_path = dir.path().join("flakiness.json");
        let mut tracker = FlakinessTracker::new(Some(storage_path));

        tracker.record_outcome("test_flaky", TestOutcome::Passed, None);
        tracker.record_outcome("test_flaky", TestOutcome::Failed, None);
        tracker.record_outcome("test_flaky", TestOutcome::Passed, None);

        tracker.record_outcome("test_stable", TestOutcome::Passed, None);
        tracker.record_outcome("test_stable", TestOutcome::Passed, None);

        let flaky = tracker.get_flaky_tests();
        assert_eq!(flaky.len(), 1);
        assert_eq!(flaky[0].node_id, "test_flaky");
        assert!(flaky[0].stability.is_flaky());
    }

    #[test]
    fn test_confirmed_flaky_recovery() {
        let dir = TempDir::new().unwrap();
        let storage_path = dir.path().join("flakiness.json");
        let mut tracker = FlakinessTracker::new(Some(storage_path));

        // Make test confirmed flaky
        tracker.record_outcome("test_recover", TestOutcome::Passed, None);
        tracker.record_outcome("test_recover", TestOutcome::Failed, None);
        tracker.record_outcome("test_recover", TestOutcome::Passed, None);
        tracker.record_outcome("test_recover", TestOutcome::Failed, None);
        tracker.record_outcome("test_recover", TestOutcome::Passed, None);

        assert!(tracker.is_flaky("test_recover"));
        let state = tracker.stability_state("test_recover");
        assert!(matches!(state, StabilityState::ConfirmedFlaky));

        // Now pass consistently - should eventually become stable
        tracker.record_outcome("test_recover", TestOutcome::Passed, None);
        tracker.record_outcome("test_recover", TestOutcome::Passed, None);
        tracker.record_outcome("test_recover", TestOutcome::Passed, None);

        let state = tracker.stability_state("test_recover");
        assert!(matches!(
            state,
            StabilityState::Stable {
                consecutive_passes: 4
            }
        ));
        assert!(!tracker.is_flaky("test_recover"));
    }

    #[test]
    fn test_stable_count() {
        let dir = TempDir::new().unwrap();
        let storage_path = dir.path().join("flakiness.json");
        let mut tracker = FlakinessTracker::new(Some(storage_path));

        tracker.record_outcome("stable1", TestOutcome::Passed, None);
        tracker.record_outcome("stable1", TestOutcome::Passed, None);

        tracker.record_outcome("flaky1", TestOutcome::Passed, None);
        tracker.record_outcome("flaky1", TestOutcome::Failed, None);
        tracker.record_outcome("flaky1", TestOutcome::Passed, None);

        tracker.record_outcome("unstable1", TestOutcome::Failed, None);
        tracker.record_outcome("unstable1", TestOutcome::Failed, None);

        assert_eq!(tracker.stable_count(), 1);
        assert_eq!(tracker.get_flaky_tests().len(), 1);
        assert_eq!(tracker.get_unstable_tests().len(), 1);
    }
}
