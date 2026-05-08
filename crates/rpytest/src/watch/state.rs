//! Watch mode state machine.
//!
//! Manages the lifecycle of file watching and test execution:
//! - Detects file changes
//! - Debounces rapid successive changes
//! - Computes affected tests via dependency graph
//! - Triggers re-collection when needed
//! - Executes tests and handles results

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::debug;

/// Events that drive the watch mode state machine.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// File changes detected.
    FileChanges(Vec<WatchFileEvent>),
    /// Debounce timer expired.
    Debounced,
    /// User requested a full test run.
    UserTrigger,
    /// Test run completed.
    RunComplete,
    /// Shutdown requested.
    Shutdown,
    /// No changes detected (heartbeat).
    NoOp,
}

/// File change event for watch mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchFileEvent {
    pub path: PathBuf,
    pub kind: WatchEventKind,
}

/// Type of file change event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEventKind {
    Modified,
    Created,
    Deleted,
    Renamed,
}

/// States in the watch mode lifecycle.
///
/// Transitions:
/// - Idle -> Debouncing (file changes detected)
/// - Debouncing -> ComputingAffected (debounce expired)
/// - ComputingAffected -> Recollecting (conftest or test file changed)
/// - ComputingAffected -> Running (affected tests known)
/// - ComputingAffected -> Idle (no affected tests)
/// - Recollecting -> Running (tests ready)
/// - Recollecting -> Idle (no tests found)
/// - Running -> Idle (run complete)
/// - Any -> WaitingForTrigger (if paused)
/// - WaitingForTrigger -> Running (user pressed key)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchState {
    /// Waiting for file changes.
    Idle,
    /// Changes detected, waiting for debounce window to expire.
    Debouncing {
        events: Vec<WatchFileEvent>,
        deadline: Instant,
    },
    /// Computing which tests are affected by the changed files.
    ComputingAffected {
        changed_files: Vec<PathBuf>,
    },
    /// Re-collecting tests due to conftest or test file changes.
    Recollecting {
        reason: RecollectReason,
    },
    /// Tests are currently running.
    Running {
        test_count: usize,
        start_time: Instant,
    },
    /// Paused, waiting for user trigger.
    WaitingForTrigger,
}

/// Reason for re-collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecollectReason {
    ConftestChanged,
    TestFilesChanged { file_names: HashSet<String> },
}

impl WatchState {
    /// Returns true if the watch loop should wait for file changes.
    pub fn is_idle(&self) -> bool {
        matches!(self, WatchState::Idle | WatchState::WaitingForTrigger)
    }

    /// Returns true if tests are currently running.
    pub fn is_running(&self) -> bool {
        matches!(self, WatchState::Running { .. })
    }

    /// Attempt to transition to a new state.
    ///
    /// Returns the new state on success, or an error describing why the transition is invalid.
    pub fn transition(self, event: &WatchEvent) -> Result<Self, String> {
        let new_state = match (&self, event) {
            // Idle: file changes start debouncing
            (WatchState::Idle, WatchEvent::FileChanges(events)) => WatchState::Debouncing {
                events: events.clone(),
                deadline: Instant::now() + Duration::from_millis(300),
            },
            // Idle: user trigger starts running all tests
            (WatchState::Idle, WatchEvent::UserTrigger) => WatchState::WaitingForTrigger,

            // Debouncing: accumulate more changes or expire
            (WatchState::Debouncing { events, deadline }, WatchEvent::FileChanges(new_events)) => {
                let mut merged = events.clone();
                merged.extend(new_events.clone());
                WatchState::Debouncing {
                    events: merged,
                    deadline: *deadline,
                }
            }
            (WatchState::Debouncing { events, .. }, WatchEvent::Debounced) => WatchState::ComputingAffected {
                changed_files: events.iter().map(|e| e.path.clone()).collect(),
            },

            // ComputingAffected: determine next step
            (WatchState::ComputingAffected { changed_files }, WatchEvent::Debounced) => {
                // This is the main decision point - caller should evaluate affected tests
                // and transition to Recollecting, Running, or Idle
                WatchState::ComputingAffected {
                    changed_files: changed_files.clone(),
                }
            }

            // Recollecting: proceed to run
            (WatchState::Recollecting { .. }, WatchEvent::RunComplete) => WatchState::Idle,

            // Running: return to idle when done
            (WatchState::Running { .. }, WatchEvent::RunComplete) => WatchState::Idle,

            // WaitingForTrigger: user trigger starts run
            (WatchState::WaitingForTrigger, WatchEvent::UserTrigger) => {
                WatchState::Running {
                    test_count: 0,
                    start_time: Instant::now(),
                }
            }
            (WatchState::WaitingForTrigger, WatchEvent::FileChanges(events)) => WatchState::Debouncing {
                events: events.clone(),
                deadline: Instant::now() + Duration::from_millis(300),
            },

            // Shutdown from any state
            (_, WatchEvent::Shutdown) => WatchState::Idle,

            // No-op events don't change state
            (current, WatchEvent::NoOp) => current.clone(),

            // Invalid transitions
            _ => {
                return Err(format!(
                    "Invalid watch state transition: {:?} on event {:?}",
                    self, event
                ));
            }
        };

        if new_state != self {
            debug!(
                "Watch state: {:?} -> {:?} (event: {:?})",
                self, new_state, event
            );
        }

        Ok(new_state)
    }
}

/// Result of computing affected tests.
#[derive(Debug, Clone)]
pub struct AffectedTests {
    /// Whether all tests should be run (conftest changed).
    pub run_all: bool,
    /// Specific test node IDs to run.
    pub node_ids: Vec<String>,
    /// Whether test files changed (may need re-collection).
    pub test_files_changed: bool,
    /// Names of changed test files for filtering.
    pub changed_test_file_names: HashSet<String>,
}

impl AffectedTests {
    /// Returns true if any tests should be run.
    pub fn has_tests(&self) -> bool {
        self.run_all || !self.node_ids.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_watch_state_transitions() {
        // Idle -> Debouncing
        let state = WatchState::Idle
            .transition(&WatchEvent::FileChanges(vec![WatchFileEvent {
                path: PathBuf::from("test.py"),
                kind: WatchEventKind::Modified,
            }]))
            .unwrap();
        assert!(matches!(state, WatchState::Debouncing { .. }));

        // Debouncing -> ComputingAffected
        let state = state.transition(&WatchEvent::Debounced).unwrap();
        assert!(matches!(state, WatchState::ComputingAffected { .. }));

        // Running -> Idle
        let state = WatchState::Running {
            test_count: 5,
            start_time: Instant::now(),
        }
        .transition(&WatchEvent::RunComplete)
        .unwrap();
        assert_eq!(state, WatchState::Idle);

        // Shutdown from any state
        let state = WatchState::Running {
            test_count: 5,
            start_time: Instant::now(),
        }
        .transition(&WatchEvent::Shutdown)
        .unwrap();
        assert_eq!(state, WatchState::Idle);
    }

    #[test]
    fn test_watch_state_invalid_transitions() {
        // Cannot transition Idle -> RunComplete
        assert!(WatchState::Idle.transition(&WatchEvent::RunComplete).is_err());

        // Cannot transition Running -> Debouncing
        assert!(
            WatchState::Running {
                test_count: 1,
                start_time: Instant::now(),
            }
            .transition(&WatchEvent::FileChanges(vec![]))
            .is_err()
        );
    }

    #[test]
    fn test_affected_tests() {
        let affected = AffectedTests {
            run_all: false,
            node_ids: vec!["test.py::test_1".to_string()],
            test_files_changed: true,
            changed_test_file_names: ["test.py".to_string()].into(),
        };
        assert!(affected.has_tests());

        let empty = AffectedTests {
            run_all: false,
            node_ids: vec![],
            test_files_changed: false,
            changed_test_file_names: HashSet::new(),
        };
        assert!(!empty.has_tests());

        let all = AffectedTests {
            run_all: true,
            node_ids: vec![],
            test_files_changed: false,
            changed_test_file_names: HashSet::new(),
        };
        assert!(all.has_tests());
    }
}
