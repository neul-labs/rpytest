//! Storage abstraction for rpytest persistence.

mod sled_backend;
mod traits;

pub use sled_backend::SledBackend;
pub use traits::{keys, StorageBackend, StorageError, StorageResult, SCHEMA_VERSION};
