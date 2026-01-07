//! Flakiness detection and tracking.

use crate::error::Result;
use crate::models::{FlakinessRecord, TestOutcome};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::debug;

/// Tracks test flakiness and manages auto-reruns.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlakinessTracker {
    /// Flakiness records by node_id
    records: HashMap<String, FlakinessRecord>,
    /// Maximum number of outcomes to keep per test
    max_outcomes: usize,
    /// Path to persist data
    storage_path: Option<PathBuf>,
}

impl FlakinessTracker {
    /// Create a new tracker.
    pub fn new(storage_path: Option<PathBuf>) -> Self {
        let mut tracker = FlakinessTracker {
            records: HashMap::new(),
            max_outcomes: 20,
            storage_path,
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
            });

        let prev_outcome = record.outcomes.last().map(|o| o.clone());

        record.outcomes.push(outcome.clone().into());

        // Keep only last N outcomes
        if record.outcomes.len() > self.max_outcomes {
            record.outcomes = record.outcomes[record.outcomes.len() - self.max_outcomes..].to_vec();
        }

        record.total_runs += 1;

        // Update consecutive counters
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

        // Track flaky streaks (outcome flips)
        if let Some(prev) = prev_outcome {
            let prev_str = prev.as_str();
            let prev_is_fail = prev_str == "failed" || prev_str == "error";
            let curr_is_fail = matches!(outcome, TestOutcome::Failed | TestOutcome::Error);
            if prev_is_fail != curr_is_fail {
                record.flaky_streak += 1;
            }
        }

        // Auto-save
        if let Some(ref path) = self.storage_path {
            let _ = self.save();
        }
    }

    /// Get flakiness record for a test.
    pub fn get_record(&self, node_id: &str) -> Option<&FlakinessRecord> {
        self.records.get(node_id)
    }

    /// Check if a test is considered flaky.
    pub fn is_flaky(&self, node_id: &str) -> bool {
        if let Some(record) = self.records.get(node_id) {
            Self::check_is_flaky(record)
        } else {
            false
        }
    }

    /// Internal check for flakiness based on record.
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
            .filter(|r| Self::check_is_flaky(r))
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
                has_fail && !Self::check_is_flaky(r)
            })
            .collect()
    }

    /// Get count of stable tests.
    pub fn stable_count(&self) -> usize {
        self.records
            .values()
            .filter(|r| {
                !Self::check_is_flaky(r)
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
    fn test_record_outcome() {
        let dir = TempDir::new().unwrap();
        let storage_path = dir.path().join("flakiness.json");
        let mut tracker = FlakinessTracker::new(Some(storage_path));

        tracker.record_outcome("test_a", TestOutcome::Passed, None);
        tracker.record_outcome("test_a", TestOutcome::Failed, Some("error"));

        assert_eq!(tracker.total_tracked(), 1);
        assert!(!tracker.is_flaky("test_a")); // Need more outcomes
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
    }
}
