//! Test scheduler for load balancing across workers.

use crate::models::ScheduledTest;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Scheduler for ordering tests based on duration estimates.
///
/// Strategy: Schedule longest tests first (LPT - Longest Processing Time).
/// This helps minimize total execution time by ensuring slow tests
/// start early while faster tests fill in gaps.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TestScheduler {
    /// Duration history for each test (node_id -> list of durations in ms)
    pub duration_history: HashMap<String, Vec<u64>>,
    /// Default duration estimate in ms when no history exists
    pub default_duration_ms: u64,
}

impl TestScheduler {
    /// Create a new scheduler.
    pub fn new() -> Self {
        TestScheduler {
            duration_history: HashMap::new(),
            default_duration_ms: 1000, // 1 second default
        }
    }

    /// Update duration history for a test.
    pub fn update_duration(&mut self, node_id: &str, duration_ms: u64) {
        let history = self
            .duration_history
            .entry(node_id.to_string())
            .or_default();
        history.push(duration_ms);

        // Keep only last 10 runs
        if history.len() > 10 {
            *history = history[history.len() - 10..].to_vec();
        }
    }

    /// Get estimated duration for a test.
    pub fn get_estimated_duration(&self, node_id: &str) -> u64 {
        if let Some(durations) = self.duration_history.get(node_id) {
            if durations.is_empty() {
                return self.default_duration_ms;
            }

            if durations.len() == 1 {
                return durations[0];
            }

            // Use exponential moving average favoring recent runs
            let len = durations.len();
            let mut weighted_sum: f64 = 0.0;
            let mut total_weight: f64 = 0.0;

            for (i, &duration) in durations.iter().enumerate() {
                // Most recent gets highest weight (reversed index)
                let weight = 0.5_f64.powi((len - 1 - i) as i32);
                weighted_sum += duration as f64 * weight;
                total_weight += weight;
            }

            (weighted_sum / total_weight) as u64
        } else {
            self.default_duration_ms
        }
    }

    /// Schedule tests for optimal execution order.
    ///
    /// Args:
    ///   node_ids: Tests to schedule.
    ///   failed_first: If true, prioritize recently failed tests.
    ///   recent_failures: List of node IDs that failed recently.
    ///
    /// Returns:
    ///   Ordered list of node IDs optimized for parallel execution.
    pub fn schedule(
        &self,
        node_ids: &[String],
        failed_first: bool,
        recent_failures: &[String],
    ) -> Vec<String> {
        if node_ids.is_empty() {
            return Vec::new();
        }

        let recent_failures_set: std::collections::HashSet<&str> =
            recent_failures.iter().map(|s| s.as_str()).collect();

        // Create scheduled test objects
        let mut scheduled: Vec<ScheduledTest> = Vec::with_capacity(node_ids.len());

        for node_id in node_ids {
            let est_duration = self.get_estimated_duration(node_id);

            // Calculate priority
            let mut priority = est_duration;

            if failed_first && recent_failures_set.contains(node_id.as_str()) {
                // Boost priority for recently failed tests
                priority += 1_000_000;
            }

            scheduled.push(ScheduledTest {
                node_id: node_id.clone(),
                estimated_duration_ms: est_duration,
                priority,
            });
        }

        // Sort by priority descending (highest first)
        scheduled.sort_by(|a, b| b.priority.cmp(&a.priority));

        scheduled.into_iter().map(|s| s.node_id).collect()
    }

    /// Clear duration history.
    pub fn clear_history(&mut self) {
        self.duration_history.clear();
    }

    /// Get history for a specific test.
    pub fn get_history(&self, node_id: &str) -> Option<&Vec<u64>> {
        self.duration_history.get(node_id)
    }

    /// Get total number of tracked tests.
    pub fn tracked_count(&self) -> usize {
        self.duration_history.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schedule_ordering() {
        let scheduler = TestScheduler::new();
        let node_ids = vec![
            "test_a".to_string(),
            "test_b".to_string(),
            "test_c".to_string(),
        ];

        // Without history, should maintain order
        let result = scheduler.schedule(&node_ids, false, &[]);
        assert_eq!(result, node_ids);
    }

    #[test]
    fn test_failed_first_priority() {
        let scheduler = TestScheduler::new();
        let node_ids = vec![
            "test_a".to_string(),
            "test_b".to_string(),
            "test_c".to_string(),
        ];
        let recent_failures = vec!["test_b".to_string()];

        let result = scheduler.schedule(&node_ids, true, &recent_failures);
        // test_b should be first
        assert_eq!(result[0], "test_b");
    }

    #[test]
    fn test_duration_update() {
        let mut scheduler = TestScheduler::new();
        scheduler.update_duration("test_a", 100);
        scheduler.update_duration("test_a", 200);

        let duration = scheduler.get_estimated_duration("test_a");
        // Should be around 175 (weighted average favoring recent)
        assert!(duration >= 150 && duration <= 200);
    }
}
