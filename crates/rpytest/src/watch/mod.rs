//! File watching and incremental test execution.

mod watcher;
mod dependency;

pub use watcher::{FileWatcher, WatchEvent, WatchEventKind, filter_test_files};
pub use dependency::{DependencyGraph, AffectedTests};
