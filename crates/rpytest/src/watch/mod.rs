//! File watching and incremental test execution.

mod dependency;
mod watcher;

#[allow(unused_imports)]
pub use dependency::{AffectedTests, DependencyGraph};
#[allow(unused_imports)]
pub use watcher::{filter_test_files, FileWatcher, WatchEvent, WatchEventKind};
