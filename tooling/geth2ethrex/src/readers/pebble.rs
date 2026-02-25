//! Pebble database reader using RocksDB
//!
//! This module provides read-only access to Geth's Pebble databases
//! by leveraging the RocksDB crate, which can read Pebble's SSTable format.
//!
//! ## Compatibility Notes
//!
//! Pebble is a Go-native key-value store that uses a similar SSTable format
//! to RocksDB. While not 100% compatible, the RocksDB crate can successfully
//! read most Pebble databases in read-only mode.
//!
//! ### Known Limitations
//! - Pebble-specific optimizations (e.g., sstable lazy writes) are ignored
//! - Some advanced Pebble features (like bloom filters) may not work
//! - Read-only access only (write operations are not supported)
//!
//! ### Tested Geth Versions
//! - Geth v1.10.0 through v1.14.x (Pebble as default)
//!
//! ## Fallback Strategy
//!
//! If RocksDB fails to open a Pebble database, users can export the data
//! using Geth's built-in export command:
//!
//! ```bash
//! geth --datadir /path/to/geth export /tmp/blocks.rlp
//! ```

use super::KeyValueReader;
use rocksdb::{DBWithThreadMode, MultiThreaded, Options};
use std::path::Path;
use std::sync::Arc;

/// Pebble database reader (backed by RocksDB)
pub struct PebbleReader {
    db: Arc<DBWithThreadMode<MultiThreaded>>,
}

impl PebbleReader {
    /// Opens a Pebble database in read-only mode
    ///
    /// # Arguments
    /// * `path` - Path to the Pebble chaindata directory
    ///
    /// # Errors
    /// - If the database cannot be opened (corrupted, incompatible version, etc.)
    ///
    /// # Example
    /// ```no_run
    /// use std::path::Path;
    /// use geth2ethrex::readers::pebble::PebbleReader;
    ///
    /// let chaindata = Path::new("/path/to/geth/chaindata");
    /// let reader = PebbleReader::open(chaindata).unwrap();
    /// ```
    pub fn open(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut opts = Options::default();

        // Read-only mode: prevent any modifications
        opts.set_disable_auto_compactions(true);
        opts.set_allow_mmap_reads(true);

        // Increase parallelism for faster reads
        opts.set_max_background_jobs(4);

        // Open database in read-only mode (no_create=false is the default)
        let db = DBWithThreadMode::<MultiThreaded>::open_for_read_only(&opts, path, false)?;

        Ok(Self { db: Arc::new(db) })
    }

    /// Returns the underlying RocksDB instance (for advanced usage)
    pub fn db(&self) -> &DBWithThreadMode<MultiThreaded> {
        &self.db
    }
}

impl KeyValueReader for PebbleReader {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
        match self.db.get(key)? {
            Some(value) => Ok(Some(value.to_vec())),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocksdb::DB;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: creates a mock Pebble database (using RocksDB)
    fn create_mock_pebble_db(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let mut opts = Options::default();
        opts.create_if_missing(true);

        let db = DB::open(&opts, path)?;

        // Insert test data
        db.put(b"header:0", b"genesis_header_data")?;
        db.put(b"header:1", b"block_1_header_data")?;
        db.put(b"body:1", b"block_1_body_data")?;

        // Explicitly drop to close the database
        drop(db);

        // Create Pebble-specific marker file
        fs::write(path.join("OPTIONS-000001"), b"mock pebble options")?;

        Ok(())
    }

    #[test]
    fn opens_pebble_database_readonly() {
        let temp_dir = TempDir::new().unwrap();
        let chaindata = temp_dir.path();

        create_mock_pebble_db(chaindata).unwrap();

        let reader = PebbleReader::open(chaindata).unwrap();
        assert!(reader.db().path().exists());
    }

    #[test]
    fn reads_existing_keys() {
        let temp_dir = TempDir::new().unwrap();
        let chaindata = temp_dir.path();

        create_mock_pebble_db(chaindata).unwrap();
        let reader = PebbleReader::open(chaindata).unwrap();

        let value = reader.get(b"header:0").unwrap();
        assert_eq!(value, Some(b"genesis_header_data".to_vec()));

        let value = reader.get(b"header:1").unwrap();
        assert_eq!(value, Some(b"block_1_header_data".to_vec()));
    }

    #[test]
    fn returns_none_for_missing_keys() {
        let temp_dir = TempDir::new().unwrap();
        let chaindata = temp_dir.path();

        create_mock_pebble_db(chaindata).unwrap();
        let reader = PebbleReader::open(chaindata).unwrap();

        let value = reader.get(b"nonexistent_key").unwrap();
        assert_eq!(value, None);
    }

    #[test]
    fn contains_check_works() {
        let temp_dir = TempDir::new().unwrap();
        let chaindata = temp_dir.path();

        create_mock_pebble_db(chaindata).unwrap();
        let reader = PebbleReader::open(chaindata).unwrap();

        assert!(reader.contains(b"header:0").unwrap());
        assert!(reader.contains(b"body:1").unwrap());
        assert!(!reader.contains(b"missing").unwrap());
    }

    #[test]
    fn fails_on_nonexistent_directory() {
        let nonexistent = Path::new("/tmp/nonexistent_pebble_db_xyz");
        let result = PebbleReader::open(nonexistent);
        assert!(result.is_err());
    }
}
