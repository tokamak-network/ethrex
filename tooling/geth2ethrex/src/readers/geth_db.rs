//! Geth block reader using Geth's rawdb key schema
//!
//! Implements the key encoding used by go-ethereum's `core/rawdb/schema.go`
//! to read block headers and bodies from a Geth Pebble/LevelDB chaindata directory.
//!
//! ## Key Format Reference (go-ethereum rawdb/schema.go)
//!
//! | Data          | Key format                         |
//! |---------------|------------------------------------|
//! | Head hash     | `"LastBlock"`                      |
//! | Canonical hash| `"h" + num(8 BE) + "n"`            |
//! | Block header  | `"h" + num(8 BE) + hash(32)`       |
//! | Block body    | `"b" + num(8 BE) + hash(32)`       |
//! | Block number  | `"H" + hash(32)`                   |

use ethrex_common::{
    H256,
    types::{Block, BlockBody, BlockHeader},
};
use ethrex_rlp::decode::RLPDecode;

use super::KeyValueReader;

/// Reads Ethereum blocks from a Geth chaindata directory using Geth's rawdb key schema.
pub struct GethBlockReader {
    reader: Box<dyn KeyValueReader>,
}

impl GethBlockReader {
    /// Creates a new GethBlockReader wrapping any KeyValueReader.
    pub fn new(reader: Box<dyn KeyValueReader>) -> Self {
        Self { reader }
    }

    /// Returns the hash of the head block stored under `"LastBlock"`.
    pub fn read_head_block_hash(&self) -> Result<H256, Box<dyn std::error::Error>> {
        let raw = self
            .reader
            .get(b"LastBlock")?
            .ok_or("Key 'LastBlock' not found in Geth chaindata")?;

        if raw.len() != 32 {
            return Err(format!(
                "'LastBlock' value has unexpected length {} (expected 32)",
                raw.len()
            )
            .into());
        }

        Ok(H256::from_slice(&raw))
    }

    /// Returns the block number for the given hash using the reverse index `"H" + hash`.
    pub fn read_block_number(&self, hash: H256) -> Result<u64, Box<dyn std::error::Error>> {
        let key = header_number_key(hash);
        let raw = self.reader.get(&key)?.ok_or_else(|| {
            format!("Block number not found for hash {:?}", hash)
        })?;

        if raw.len() != 8 {
            return Err(format!(
                "Block number value has unexpected length {} (expected 8)",
                raw.len()
            )
            .into());
        }

        Ok(u64::from_be_bytes(raw.try_into().unwrap()))
    }

    /// Returns the canonical block hash for the given block number.
    ///
    /// Returns `None` if there is no canonical block at that number (e.g. above chain head).
    pub fn read_canonical_hash(
        &self,
        number: u64,
    ) -> Result<Option<H256>, Box<dyn std::error::Error>> {
        let key = canonical_hash_key(number);
        match self.reader.get(&key)? {
            None => Ok(None),
            Some(raw) => {
                if raw.len() != 32 {
                    return Err(format!(
                        "Canonical hash for block #{number} has unexpected length {} (expected 32)",
                        raw.len()
                    )
                    .into());
                }
                Ok(Some(H256::from_slice(&raw)))
            }
        }
    }

    /// Reads and RLP-decodes the block header for a known (number, hash) pair.
    pub fn read_block_header(
        &self,
        number: u64,
        hash: H256,
    ) -> Result<Option<BlockHeader>, Box<dyn std::error::Error>> {
        let key = header_key(number, hash);
        match self.reader.get(&key)? {
            None => Ok(None),
            Some(raw) => {
                let header = BlockHeader::decode(&raw).map_err(|e| {
                    format!("RLP decode error for block header #{number}: {e:?}")
                })?;
                Ok(Some(header))
            }
        }
    }

    /// Reads and RLP-decodes the block body for a known (number, hash) pair.
    pub fn read_block_body(
        &self,
        number: u64,
        hash: H256,
    ) -> Result<Option<BlockBody>, Box<dyn std::error::Error>> {
        let key = body_key(number, hash);
        match self.reader.get(&key)? {
            None => Ok(None),
            Some(raw) => {
                let body = BlockBody::decode(&raw).map_err(|e| {
                    format!("RLP decode error for block body #{number}: {e:?}")
                })?;
                Ok(Some(body))
            }
        }
    }

    /// Reads a complete block (header + body) for a known (number, hash) pair.
    ///
    /// Returns `None` if either header or body is missing.
    pub fn read_block(
        &self,
        number: u64,
        hash: H256,
    ) -> Result<Option<Block>, Box<dyn std::error::Error>> {
        let header = match self.read_block_header(number, hash)? {
            Some(h) => h,
            None => return Ok(None),
        };
        let body = match self.read_block_body(number, hash)? {
            Some(b) => b,
            None => return Ok(None),
        };
        Ok(Some(Block::new(header, body)))
    }
}

// --- Key encoding helpers (mirrors go-ethereum rawdb/schema.go) ---

/// `"LastBlock"` → head block hash
#[allow(dead_code)]
fn last_block_key() -> &'static [u8] {
    b"LastBlock"
}

/// `"h" + num(8 BE) + "n"` → canonical block hash
fn canonical_hash_key(number: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(10);
    key.push(b'h');
    key.extend_from_slice(&number.to_be_bytes());
    key.push(b'n');
    key
}

/// `"h" + num(8 BE) + hash(32)` → RLP-encoded block header
fn header_key(number: u64, hash: H256) -> Vec<u8> {
    let mut key = Vec::with_capacity(41);
    key.push(b'h');
    key.extend_from_slice(&number.to_be_bytes());
    key.extend_from_slice(hash.as_bytes());
    key
}

/// `"b" + num(8 BE) + hash(32)` → RLP-encoded block body
fn body_key(number: u64, hash: H256) -> Vec<u8> {
    let mut key = Vec::with_capacity(41);
    key.push(b'b');
    key.extend_from_slice(&number.to_be_bytes());
    key.extend_from_slice(hash.as_bytes());
    key
}

/// `"H" + hash(32)` → block number (8-byte BE)
fn header_number_key(hash: H256) -> Vec<u8> {
    let mut key = Vec::with_capacity(33);
    key.push(b'H');
    key.extend_from_slice(hash.as_bytes());
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_hash_key_format() {
        let key = canonical_hash_key(1);
        assert_eq!(key, b"h\x00\x00\x00\x00\x00\x00\x00\x01n");
    }

    #[test]
    fn header_key_format() {
        let hash = H256::zero();
        let key = header_key(1, hash);
        assert_eq!(key[0], b'h');
        assert_eq!(&key[1..9], &1u64.to_be_bytes());
        assert_eq!(&key[9..], hash.as_bytes());
    }

    #[test]
    fn body_key_format() {
        let hash = H256::zero();
        let key = body_key(1, hash);
        assert_eq!(key[0], b'b');
        assert_eq!(&key[1..9], &1u64.to_be_bytes());
        assert_eq!(&key[9..], hash.as_bytes());
    }

    #[test]
    fn header_number_key_format() {
        let hash = H256::zero();
        let key = header_number_key(hash);
        assert_eq!(key[0], b'H');
        assert_eq!(&key[1..], hash.as_bytes());
    }
}
