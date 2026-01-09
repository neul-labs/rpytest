//! File system watcher for detecting changes.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebouncedEvent, Debouncer};
use thiserror::Error;
use tracing::{debug, info};

/// Errors from file watching.
#[derive(Debug, Error)]
pub enum WatchError {
    #[error("Failed to create watcher: {0}")]
    CreateWatcher(#[from] notify::Error),

    #[error("Watch channel closed")]
    ChannelClosed,
}

/// Kind of file system event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEventKind {
    /// File was created.
    Created,
    /// File was modified.
    Modified,
    /// File was deleted.
    Deleted,
    /// File was renamed.
    Renamed,
}

/// A file system change event.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    /// Path to the changed file.
    pub path: PathBuf,
    /// Kind of change.
    pub kind: WatchEventKind,
}

/// File watcher for a repository.
pub struct FileWatcher {
    /// Root path being watched.
    root: PathBuf,
    /// Debouncer handle.
    _debouncer: Debouncer<RecommendedWatcher>,
    /// Receiver for events.
    receiver: Receiver<Result<Vec<DebouncedEvent>, notify::Error>>,
    /// Patterns to ignore.
    ignore_patterns: Vec<String>,
}

impl FileWatcher {
    /// Create a new file watcher for a repository.
    ///
    /// # Arguments
    /// * `root` - Root directory to watch.
    /// * `debounce_ms` - Debounce interval in milliseconds.
    pub fn new(root: impl AsRef<Path>, debounce_ms: u64) -> Result<Self, WatchError> {
        let root = root.as_ref().to_path_buf();
        let (tx, rx) = channel();

        let debounce_duration = Duration::from_millis(debounce_ms);
        let mut debouncer = new_debouncer(debounce_duration, tx)?;

        // Watch the root directory recursively
        debouncer.watcher().watch(&root, RecursiveMode::Recursive)?;

        info!("Started watching {} for changes", root.display());

        Ok(Self {
            root,
            _debouncer: debouncer,
            receiver: rx,
            ignore_patterns: vec![
                "__pycache__".to_string(),
                ".pytest_cache".to_string(),
                ".rpytest".to_string(),
                ".git".to_string(),
                ".tox".to_string(),
                ".venv".to_string(),
                "venv".to_string(),
                ".mypy_cache".to_string(),
                "*.pyc".to_string(),
                "*.pyo".to_string(),
                ".coverage".to_string(),
                "htmlcov".to_string(),
            ],
        })
    }

    /// Add a pattern to ignore.
    pub fn add_ignore_pattern(&mut self, pattern: impl Into<String>) {
        self.ignore_patterns.push(pattern.into());
    }

    /// Wait for file changes with optional timeout.
    ///
    /// Returns `None` if timeout expires, or `Some(events)` when changes detected.
    pub fn wait_for_changes(&self, timeout: Option<Duration>) -> Option<Vec<WatchEvent>> {
        let result = match timeout {
            Some(duration) => self.receiver.recv_timeout(duration).ok(),
            None => self.receiver.recv().ok(),
        };

        result.and_then(|res| res.ok()).map(|debounced_events| {
            debounced_events
                .into_iter()
                .filter_map(|e| self.convert_event(e))
                .collect()
        })
    }

    /// Poll for changes without blocking.
    pub fn poll_changes(&self) -> Vec<WatchEvent> {
        let mut events = Vec::new();

        while let Ok(Ok(debounced_events)) = self.receiver.try_recv() {
            for e in debounced_events {
                if let Some(event) = self.convert_event(e) {
                    events.push(event);
                }
            }
        }

        events
    }

    /// Convert a debounced event to a watch event.
    fn convert_event(&self, event: DebouncedEvent) -> Option<WatchEvent> {
        let path = event.path;

        // Skip ignored patterns
        if self.should_ignore(&path) {
            debug!("Ignoring change to: {}", path.display());
            return None;
        }

        // Only watch Python files and conftest.py
        if !self.is_relevant_file(&path) {
            debug!("Skipping non-Python file: {}", path.display());
            return None;
        }

        // Determine event kind
        let kind = if path.exists() {
            WatchEventKind::Modified
        } else {
            WatchEventKind::Deleted
        };

        debug!("File change detected: {} ({:?})", path.display(), kind);

        Some(WatchEvent { path, kind })
    }

    /// Check if a path should be ignored.
    fn should_ignore(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        for pattern in &self.ignore_patterns {
            if let Some(suffix) = pattern.strip_prefix('*') {
                // Suffix match
                if path_str.ends_with(suffix) {
                    return true;
                }
            } else {
                // Contains match
                if path_str.contains(pattern) {
                    return true;
                }
            }
        }

        false
    }

    /// Check if a file is relevant for test execution.
    fn is_relevant_file(&self, path: &Path) -> bool {
        let Some(ext) = path.extension() else {
            // Check for files without extension (like conftest)
            return path.file_name().map(|n| n == "conftest").unwrap_or(false);
        };

        ext == "py"
    }

    /// Get the root directory being watched.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Filter events to only Python test files.
pub fn filter_test_files(events: Vec<WatchEvent>) -> Vec<WatchEvent> {
    events
        .into_iter()
        .filter(|e| {
            let name = e.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("test_") || name.ends_with("_test.py") || name == "conftest.py"
        })
        .collect()
}

/// Filter events to only source files (not test files).
pub fn filter_source_files(events: Vec<WatchEvent>) -> Vec<WatchEvent> {
    events
        .into_iter()
        .filter(|e| {
            let name = e.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            !name.starts_with("test_") && !name.ends_with("_test.py") && name != "conftest.py"
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_test_files() {
        let events = vec![
            WatchEvent {
                path: PathBuf::from("test_foo.py"),
                kind: WatchEventKind::Modified,
            },
            WatchEvent {
                path: PathBuf::from("foo.py"),
                kind: WatchEventKind::Modified,
            },
            WatchEvent {
                path: PathBuf::from("bar_test.py"),
                kind: WatchEventKind::Modified,
            },
            WatchEvent {
                path: PathBuf::from("conftest.py"),
                kind: WatchEventKind::Modified,
            },
        ];

        let filtered = filter_test_files(events);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn test_filter_source_files() {
        let events = vec![
            WatchEvent {
                path: PathBuf::from("test_foo.py"),
                kind: WatchEventKind::Modified,
            },
            WatchEvent {
                path: PathBuf::from("foo.py"),
                kind: WatchEventKind::Modified,
            },
            WatchEvent {
                path: PathBuf::from("utils.py"),
                kind: WatchEventKind::Modified,
            },
        ];

        let filtered = filter_source_files(events);
        assert_eq!(filtered.len(), 2);
    }
}
