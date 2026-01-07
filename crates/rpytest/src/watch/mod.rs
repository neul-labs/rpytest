//! File watching and incremental test execution.

mod dependency;
mod watcher;

pub use dependency::{AffectedTests, DependencyGraph};
pub use watcher::{filter_test_files, FileWatcher, WatchEvent, WatchEventKind};
