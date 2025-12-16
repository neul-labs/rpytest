//! Sled-based storage backend implementation.

use std::path::Path;

use sled::Db;

use super::traits::{StorageBackend, StorageError, StorageResult};

/// Sled-based storage backend.
pub struct SledBackend {
    db: Db,
}

impl StorageBackend for SledBackend {
    fn open(path: &Path) -> StorageResult<Self>
    where
        Self: Sized,
    {
        let db = sled::open(path).map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(Self { db })
    }

    fn get(&self, key: &[u8]) -> StorageResult<Option<Vec<u8>>> {
        self.db
            .get(key)
            .map(|opt| opt.map(|v| v.to_vec()))
            .map_err(|e| StorageError::Backend(e.to_string()))
    }

    fn set(&self, key: &[u8], value: &[u8]) -> StorageResult<()> {
        self.db
            .insert(key, value)
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }

    fn delete(&self, key: &[u8]) -> StorageResult<()> {
        self.db
            .remove(key)
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }

    fn flush(&self) -> StorageResult<()> {
        self.db
            .flush()
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }

    fn scan_prefix(&self, prefix: &[u8]) -> StorageResult<Vec<(Vec<u8>, Vec<u8>)>> {
        let results: Result<Vec<_>, _> = self
            .db
            .scan_prefix(prefix)
            .map(|res| res.map(|(k, v)| (k.to_vec(), v.to_vec())))
            .collect();

        results.map_err(|e| StorageError::Backend(e.to_string()))
    }

    fn clear(&self) -> StorageResult<()> {
        self.db
            .clear()
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }
}

impl SledBackend {
    /// Get the underlying sled database for advanced operations.
    pub fn inner(&self) -> &Db {
        &self.db
    }

    /// Check if the database is empty.
    pub fn is_empty(&self) -> StorageResult<bool> {
        Ok(self.db.is_empty())
    }

    /// Get the number of entries in the database.
    pub fn len(&self) -> StorageResult<usize> {
        Ok(self.db.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_db() -> (SledBackend, TempDir) {
        let tmp = TempDir::new().unwrap();
        let backend = SledBackend::open(tmp.path()).unwrap();
        (backend, tmp)
    }

    #[test]
    fn test_basic_operations() {
        let (db, _tmp) = create_test_db();

        // Set and get
        db.set(b"key1", b"value1").unwrap();
        assert_eq!(db.get(b"key1").unwrap(), Some(b"value1".to_vec()));

        // Contains
        assert!(db.contains(b"key1").unwrap());
        assert!(!db.contains(b"nonexistent").unwrap());

        // Delete
        db.delete(b"key1").unwrap();
        assert_eq!(db.get(b"key1").unwrap(), None);
    }

    #[test]
    fn test_scan_prefix() {
        let (db, _tmp) = create_test_db();

        db.set(b"inv:test1", b"data1").unwrap();
        db.set(b"inv:test2", b"data2").unwrap();
        db.set(b"dur:test1", b"100").unwrap();

        let inv_results = db.scan_prefix(b"inv:").unwrap();
        assert_eq!(inv_results.len(), 2);

        let dur_results = db.scan_prefix(b"dur:").unwrap();
        assert_eq!(dur_results.len(), 1);
    }

    #[test]
    fn test_clear() {
        let (db, _tmp) = create_test_db();

        db.set(b"key1", b"value1").unwrap();
        db.set(b"key2", b"value2").unwrap();
        assert!(!db.is_empty().unwrap());

        db.clear().unwrap();
        assert!(db.is_empty().unwrap());
    }

    #[test]
    fn test_persistence() {
        let tmp = TempDir::new().unwrap();

        // Write data
        {
            let db = SledBackend::open(tmp.path()).unwrap();
            db.set(b"persistent", b"data").unwrap();
            db.flush().unwrap();
        }

        // Read data in new instance
        {
            let db = SledBackend::open(tmp.path()).unwrap();
            assert_eq!(db.get(b"persistent").unwrap(), Some(b"data".to_vec()));
        }
    }
}
