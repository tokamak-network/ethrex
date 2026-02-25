//! Geth database type detection (LevelDB vs Pebble)
//!
//! This module provides logic to automatically detect whether a Geth chaindata
//! directory uses LevelDB or Pebble as its storage backend.

use std::fs;
use std::path::Path;

/// Detected Geth database backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GethDbType {
    /// LevelDB (Geth v1.9 and earlier, or --db.engine=leveldb)
    LevelDB,
    /// Pebble (Geth v1.10+ default, --db.engine=pebble)
    Pebble,
    /// Unable to determine the database type
    Unknown,
}

/// Error type for database detection
#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    #[error("Failed to read directory: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("Chaindata directory not found: {0}")]
    DirectoryNotFound(String),
    
    #[error("No database files found in directory")]
    NoDbFiles,
}

/// Detects the Geth database type by inspecting the chaindata directory
///
/// Detection logic (in order):
/// 1. Check for OPTIONS-* files (Pebble-specific)
/// 2. Check for *.ldb files (LevelDB SSTables)
/// 3. Check for *.sst files (Pebble SSTables)
/// 4. If none found, return Unknown
///
/// # Arguments
/// * `chaindata_path` - Path to the Geth chaindata directory
///
/// # Returns
/// * `Ok(GethDbType)` - Detected database type
/// * `Err(DetectError)` - If detection fails
///
/// # Example
/// ```no_run
/// use std::path::Path;
/// use geth2ethrex::detect::{detect_geth_db_type, GethDbType};
///
/// let chaindata = Path::new("/path/to/geth/chaindata");
/// match detect_geth_db_type(chaindata) {
///     Ok(GethDbType::LevelDB) => println!("Detected LevelDB"),
///     Ok(GethDbType::Pebble) => println!("Detected Pebble"),
///     Ok(GethDbType::Unknown) => println!("Unknown database type"),
///     Err(e) => eprintln!("Detection failed: {}", e),
/// }
/// ```
pub fn detect_geth_db_type(chaindata_path: &Path) -> Result<GethDbType, DetectError> {
    // Verify directory exists
    if !chaindata_path.exists() {
        return Err(DetectError::DirectoryNotFound(
            chaindata_path.display().to_string(),
        ));
    }

    // 1. Check for OPTIONS-* files (Pebble-specific)
    if has_pebble_options_files(chaindata_path)? {
        return Ok(GethDbType::Pebble);
    }

    // 2. Check for .ldb files (LevelDB SSTables)
    if has_leveldb_files(chaindata_path)? {
        return Ok(GethDbType::LevelDB);
    }

    // 3. Check for .sst files (Pebble SSTables, also used by RocksDB)
    if has_sst_files(chaindata_path)? {
        return Ok(GethDbType::Pebble);
    }

    // 4. No recognizable database files found
    Ok(GethDbType::Unknown)
}

/// Checks if the directory contains Pebble-specific OPTIONS files
fn has_pebble_options_files(path: &Path) -> Result<bool, DetectError> {
    let entries = fs::read_dir(path)?;
    
    for entry in entries {
        let entry = entry?;
        let filename = entry.file_name();
        let name = filename.to_string_lossy();
        
        // Pebble creates OPTIONS-NNNNNN files
        if name.starts_with("OPTIONS-") {
            return Ok(true);
        }
    }
    
    Ok(false)
}

/// Checks if the directory contains LevelDB SSTable files (.ldb)
fn has_leveldb_files(path: &Path) -> Result<bool, DetectError> {
    let entries = fs::read_dir(path)?;
    
    for entry in entries {
        let entry = entry?;
        let filename = entry.file_name();
        let name = filename.to_string_lossy();
        
        if name.ends_with(".ldb") {
            return Ok(true);
        }
    }
    
    Ok(false)
}

/// Checks if the directory contains SSTable files (.sst)
fn has_sst_files(path: &Path) -> Result<bool, DetectError> {
    let entries = fs::read_dir(path)?;
    
    for entry in entries {
        let entry = entry?;
        let filename = entry.file_name();
        let name = filename.to_string_lossy();
        
        if name.ends_with(".sst") {
            return Ok(true);
        }
    }
    
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::TempDir;

    #[test]
    fn detects_pebble_via_options_file() {
        let temp_dir = TempDir::new().unwrap();
        let chaindata = temp_dir.path();

        // Create Pebble-specific OPTIONS file
        File::create(chaindata.join("OPTIONS-000001")).unwrap();

        let db_type = detect_geth_db_type(chaindata).unwrap();
        assert_eq!(db_type, GethDbType::Pebble);
    }

    #[test]
    fn detects_leveldb_via_ldb_files() {
        let temp_dir = TempDir::new().unwrap();
        let chaindata = temp_dir.path();

        // Create LevelDB SSTable files
        File::create(chaindata.join("000005.ldb")).unwrap();

        let db_type = detect_geth_db_type(chaindata).unwrap();
        assert_eq!(db_type, GethDbType::LevelDB);
    }

    #[test]
    fn detects_pebble_via_sst_files() {
        let temp_dir = TempDir::new().unwrap();
        let chaindata = temp_dir.path();

        // Create Pebble SSTable files
        File::create(chaindata.join("000005.sst")).unwrap();

        let db_type = detect_geth_db_type(chaindata).unwrap();
        assert_eq!(db_type, GethDbType::Pebble);
    }

    #[test]
    fn returns_unknown_for_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let chaindata = temp_dir.path();

        let db_type = detect_geth_db_type(chaindata).unwrap();
        assert_eq!(db_type, GethDbType::Unknown);
    }

    #[test]
    fn returns_error_for_missing_directory() {
        let nonexistent = Path::new("/tmp/nonexistent_geth_chaindata_dir_xyz");
        
        let result = detect_geth_db_type(nonexistent);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DetectError::DirectoryNotFound(_)));
    }

    #[test]
    fn pebble_takes_precedence_over_leveldb() {
        let temp_dir = TempDir::new().unwrap();
        let chaindata = temp_dir.path();

        // Create both LevelDB and Pebble files
        File::create(chaindata.join("000005.ldb")).unwrap();
        File::create(chaindata.join("OPTIONS-000001")).unwrap();

        // OPTIONS file should be checked first
        let db_type = detect_geth_db_type(chaindata).unwrap();
        assert_eq!(db_type, GethDbType::Pebble);
    }
}
