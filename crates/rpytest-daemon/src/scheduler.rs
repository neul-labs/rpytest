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

    /// Split tests into N balanced batches using LPT (Longest Processing Time) algorithm.
    ///
    /// This distributes tests across workers to minimize total execution time by:
    /// 1. Sorting tests by estimated duration (longest first)
    /// 2. Greedily assigning each test to the batch with the lowest total duration
    ///
    /// Returns a vector of batches, where each batch is a vector of node IDs.
    pub fn split_balanced(&self, node_ids: &[String], worker_count: usize) -> Vec<Vec<String>> {
        if worker_count <= 1 || node_ids.is_empty() {
            return vec![node_ids.to_vec()];
        }

        // Sort by duration descending
        let mut sorted: Vec<_> = node_ids
            .iter()
            .map(|id| (id.clone(), self.get_estimated_duration(id)))
            .collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        // Greedy assignment to batch with lowest total duration
        let mut batches: Vec<Vec<String>> = vec![Vec::new(); worker_count];
        let mut batch_durations = vec![0u64; worker_count];

        for (test, duration) in sorted {
            // Find the batch with the minimum total duration
            let min_idx = batch_durations
                .iter()
                .enumerate()
                .min_by_key(|(_, d)| *d)
                .map(|(i, _)| i)
                .unwrap_or(0);

            batches[min_idx].push(test);
            batch_durations[min_idx] += duration;
        }

        // Filter out empty batches
        batches.into_iter().filter(|b| !b.is_empty()).collect()
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

    #[test]
    fn test_split_balanced_empty() {
        let scheduler = TestScheduler::new();
        let result = scheduler.split_balanced(&[], 4);
        assert_eq!(result.len(), 1);
        assert!(result[0].is_empty());
    }

    #[test]
    fn test_split_balanced_single_worker() {
        let scheduler = TestScheduler::new();
        let node_ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let result = scheduler.split_balanced(&node_ids, 1);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 3);
    }

    #[test]
    fn test_split_balanced_even_distribution() {
        let mut scheduler = TestScheduler::new();
        // Create tests with known durations
        scheduler.update_duration("slow", 1000);
        scheduler.update_duration("medium1", 500);
        scheduler.update_duration("medium2", 500);
        scheduler.update_duration("fast1", 100);
        scheduler.update_duration("fast2", 100);

        let node_ids = vec![
            "slow".to_string(),
            "medium1".to_string(),
            "medium2".to_string(),
            "fast1".to_string(),
            "fast2".to_string(),
        ];

        let batches = scheduler.split_balanced(&node_ids, 2);
        assert_eq!(batches.len(), 2);

        // Calculate total duration per batch
        let batch1_duration: u64 = batches[0]
            .iter()
            .map(|id| scheduler.get_estimated_duration(id))
            .sum();
        let batch2_duration: u64 = batches[1]
            .iter()
            .map(|id| scheduler.get_estimated_duration(id))
            .sum();

        // Batches should be reasonably balanced (within 50% of each other)
        let max_dur = batch1_duration.max(batch2_duration);
        let min_dur = batch1_duration.min(batch2_duration);
        assert!(
            max_dur <= min_dur * 2,
            "Batches not balanced: {} vs {}",
            batch1_duration,
            batch2_duration
        );
    }
}
