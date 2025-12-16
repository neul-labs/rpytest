//! Storage backend trait for abstracting persistence layer.

use std::path::Path;

/// Error type for storage operations.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Key not found")]
    NotFound,

    #[error("Storage corrupted: {0}")]
    Corrupted(String),

    #[error("Storage backend error: {0}")]
    Backend(String),
}

/// Result type for storage operations.
pub type StorageResult<T> = Result<T, StorageError>;

/// Abstraction over key-value storage backends.
///
/// This trait allows rpytest to use different storage implementations
/// (e.g., sled, redb) without changing the core logic.
pub trait StorageBackend: Send + Sync {
    /// Open or create a storage instance at the given path.
    fn open(path: &Path) -> StorageResult<Self>
    where
        Self: Sized;

    /// Get a value by key.
    fn get(&self, key: &[u8]) -> StorageResult<Option<Vec<u8>>>;

    /// Set a key-value pair.
    fn set(&self, key: &[u8], value: &[u8]) -> StorageResult<()>;

    /// Delete a key.
    fn delete(&self, key: &[u8]) -> StorageResult<()>;

    /// Check if a key exists.
    fn contains(&self, key: &[u8]) -> StorageResult<bool> {
        Ok(self.get(key)?.is_some())
    }

    /// Flush pending writes to disk.
    fn flush(&self) -> StorageResult<()>;

    /// Iterate over all keys with a given prefix.
    fn scan_prefix(&self, prefix: &[u8]) -> StorageResult<Vec<(Vec<u8>, Vec<u8>)>>;

    /// Clear all data (use with caution).
    fn clear(&self) -> StorageResult<()>;
}

/// Schema version for cache format compatibility.
pub const SCHEMA_VERSION: u32 = 1;

/// Key prefixes for namespacing data in storage.
pub mod keys {
    /// Prefix for inventory data.
    pub const INVENTORY: &[u8] = b"inv:";

    /// Prefix for test duration history.
    pub const DURATIONS: &[u8] = b"dur:";

    /// Prefix for test outcome history.
    pub const OUTCOMES: &[u8] = b"out:";

    /// Prefix for context metadata.
    pub const CONTEXT: &[u8] = b"ctx:";

    /// Key for schema version.
    pub const SCHEMA: &[u8] = b"_schema_version";
}
