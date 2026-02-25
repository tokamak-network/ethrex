//! Database readers for different backend types
//!
//! This module provides abstraction over different key-value stores
//! (LevelDB, Pebble) to enable reading Geth chaindata.

pub mod pebble;

use std::path::Path;

/// Generic key-value store reader interface
pub trait KeyValueReader {
    /// Reads a value for the given key
    ///
    /// # Returns
    /// - `Ok(Some(value))` if the key exists
    /// - `Ok(None)` if the key does not exist
    /// - `Err(_)` on I/O or database errors
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>>;

    /// Checks if a key exists in the database
    fn contains(&self, key: &[u8]) -> Result<bool, Box<dyn std::error::Error>> {
        Ok(self.get(key)?.is_some())
    }
}

/// Opens a Geth database reader based on the detected type
///
/// This is the main entry point for reading Geth chaindata.
/// It automatically detects whether the database is LevelDB or Pebble
/// and returns the appropriate reader.
///
/// # Arguments
/// * `chaindata_path` - Path to the Geth chaindata directory
///
/// # Example
/// ```no_run
/// use std::path::Path;
/// use geth2ethrex::readers::open_geth_reader;
///
/// let chaindata = Path::new("/path/to/geth/chaindata");
/// let reader = open_geth_reader(chaindata).unwrap();
///
/// // Read a block header
/// let key = b"some_key";
/// if let Some(value) = reader.get(key).unwrap() {
///     println!("Found value: {} bytes", value.len());
/// }
/// ```
pub fn open_geth_reader(
    chaindata_path: &Path,
) -> Result<Box<dyn KeyValueReader>, Box<dyn std::error::Error>> {
    use crate::detect::{detect_geth_db_type, GethDbType};

    let db_type = detect_geth_db_type(chaindata_path)?;

    match db_type {
        GethDbType::LevelDB => {
            // TODO: Implement LevelDB reader using rusty-leveldb crate
            Err(format!("LevelDB reader not yet implemented").into())
        }
        GethDbType::Pebble => {
            let reader = pebble::PebbleReader::open(chaindata_path)?;
            Ok(Box::new(reader))
        }
        GethDbType::Unknown => Err(format!(
            "Unable to determine database type for: {}",
            chaindata_path.display()
        )
        .into()),
    }
}
