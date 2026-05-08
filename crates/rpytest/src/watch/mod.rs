//! File watching and incremental test execution.

mod dependency;
mod state;
mod watcher;

#[allow(unused_imports)]
pub use dependency::{AffectedTests, DependencyGraph};
pub use state::{RecollectReason, WatchEvent, WatchEventKind, WatchFileEvent, WatchState};
#[allow(unused_imports)]
pub use watcher::{
    filter_test_files, FileWatcher, WatchEvent as WatcherEvent, WatchEventKind as WatcherEventKind,
};
