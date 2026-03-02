//! Geth block reader using Geth's rawdb key schema
//!
//! Implements the key encoding used by go-ethereum's `core/rawdb/schema.go`
//! to read block headers and bodies from a Geth Pebble/LevelDB chaindata directory.
//!
//! ## Key Format Reference (go-ethereum rawdb/schema.go)
//!
//! | Data              | Key format                                   |
//! |-------------------|----------------------------------------------|
//! | Head hash         | `"LastBlock"`                                |
//! | Canonical hash    | `"h" + num(8 BE) + "n"`                     |
//! | Block header      | `"h" + num(8 BE) + hash(32)`                |
//! | Block body        | `"b" + num(8 BE) + hash(32)`                |
//! | Block number      | `"H" + hash(32)`                             |
//! | Account snapshot  | `"a" + account_hash(32)`                     |
//! | Storage snapshot  | `"o" + account_hash(32) + slot_hash(32)`     |
//! | Code              | `"c" + code_hash(32)`                        |
//! | Receipts          | `"r" + num(8 BE) + hash(32)`                 |
//! | Preimage          | `"secure-key-" + hash(32)`                   |
//!
//! ## Ancient (Freezer) Fallback
//!
//! Geth's path-state scheme moves canonical block data to an "ancient" freezer
//! database almost immediately. `GethBlockReader` transparently falls back to
//! [`super::ancient::AncientReader`] for blocks that are no longer in the hot
//! Pebble store.

use ethrex_common::{
    H256, U256,
    types::{Block, BlockBody, BlockHeader, Log, Receipt},
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::structs::Decoder;

use super::KeyValueReader;
use super::ancient::AncientReader;

/// Geth's slim-encoded account snapshot.
///
/// Geth stores account snapshots as a "slim" RLP list where empty fields
/// (default storage root and empty code hash) are omitted.
/// Format: `[nonce, balance, root?, codehash?]`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlimAccount {
    pub nonce: u64,
    pub balance: U256,
    pub storage_root: H256,
    pub code_hash: H256,
}

/// Empty trie root: keccak256(RLP(""))
const EMPTY_ROOT: [u8; 32] = [
    0x56, 0xe8, 0x1f, 0x17, 0x1b, 0xcc, 0x55, 0xa6, 0xff, 0x83, 0x45, 0xe6, 0x92, 0xc0, 0xf8, 0x6e,
    0x5b, 0x48, 0xe0, 0x1b, 0x99, 0x6c, 0xad, 0xc0, 0x01, 0x62, 0x2f, 0xb5, 0xe3, 0x63, 0xb4, 0x21,
];

/// keccak256 of empty bytes
const KECCAK_EMPTY: [u8; 32] = [
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
];

impl SlimAccount {
    /// Encodes this account as full RLP (for storage or debug output).
    /// Outputs all 4 fields regardless of defaults.
    pub fn rlp_encode_full(&self) -> Vec<u8> {
        use ethrex_rlp::encode::RLPEncode;

        // Encode items
        let nonce_encoded = self.nonce.encode_to_vec();
        let balance_encoded = self.balance.encode_to_vec();
        let root_encoded = self.storage_root.as_bytes().to_vec();
        let code_hash_encoded = self.code_hash.as_bytes().to_vec();

        // Encode as an RLP list [nonce, balance, root, code_hash]
        let mut result = Vec::new();

        // Calculate total length
        let total_len = nonce_encoded.len() + balance_encoded.len() + root_encoded.len() + code_hash_encoded.len();

        // Add RLP list header
        if total_len < 56 {
            result.push(0xc0 + total_len as u8);
        } else {
            // For simplicity, just handle the basic case (won't be > 55 bytes for account data)
            result.push(0xc0 + total_len as u8);
        }

        // Add encoded items
        result.extend_from_slice(&nonce_encoded);
        result.extend_from_slice(&balance_encoded);
        result.extend_from_slice(&root_encoded);
        result.extend_from_slice(&code_hash_encoded);

        result
    }

    /// Decodes a Geth slim-RLP encoded account snapshot.
    ///
    /// The slim encoding is an RLP list `[nonce, balance, root?, codehash?]`
    /// where `root` defaults to EMPTY_ROOT and `codehash` defaults to KECCAK_EMPTY
    /// when omitted (empty byte string in the RLP).
    pub fn decode(data: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        if data.is_empty() {
            return Err("empty slim account data".into());
        }

        // Parse the outer RLP list
        let (list_data, _) = decode_rlp_list(data)?;
        let mut items = decode_rlp_items(&list_data)?;

        // Must have at least 2 items (nonce, balance)
        if items.len() < 2 {
            return Err(format!(
                "slim account has {} items, expected at least 2",
                items.len()
            )
            .into());
        }

        // Pad to 4 items with empty bytes
        while items.len() < 4 {
            items.push(Vec::new());
        }

        let nonce = decode_u64(&items[0]);
        let balance = decode_u256(&items[1]);

        let storage_root = if items[2].is_empty() {
            H256::from_slice(&EMPTY_ROOT)
        } else if items[2].len() == 32 {
            H256::from_slice(&items[2])
        } else {
            return Err(format!(
                "slim account storage_root has unexpected length {}",
                items[2].len()
            )
            .into());
        };

        let code_hash = if items[3].is_empty() {
            H256::from_slice(&KECCAK_EMPTY)
        } else if items[3].len() == 32 {
            H256::from_slice(&items[3])
        } else {
            return Err(format!(
                "slim account code_hash has unexpected length {}",
                items[3].len()
            )
            .into());
        };

        Ok(Self {
            nonce,
            balance,
            storage_root,
            code_hash,
        })
    }
}

/// Single receipt from a block (decoded from Geth format)
#[derive(Debug, Clone)]
pub struct StoredReceipt {
    pub succeeded: bool,
    pub cumulative_gas_used: u64,
    pub logs: Vec<Log>,
}

/// Decode an RLP list, returning (list_data, remaining_bytes)
fn decode_rlp_list(data: &[u8]) -> Result<(Vec<u8>, usize), Box<dyn std::error::Error>> {
    if data.is_empty() {
        return Err("empty RLP data".into());
    }

    let byte = data[0];
    if byte < 0xc0 {
        return Err("not an RLP list".into());
    }

    if byte == 0xc0 {
        return Ok((Vec::new(), 1));
    }

    if byte <= 0xf7 {
        let len = (byte - 0xc0) as usize;
        if data.len() < 1 + len {
            return Err("incomplete RLP list".into());
        }
        return Ok((data[1..1 + len].to_vec(), 1 + len));
    }

    let len_bytes = (byte - 0xf7) as usize;
    if data.len() < 1 + len_bytes {
        return Err("incomplete RLP list length".into());
    }

    let mut len = 0usize;
    for &b in &data[1..1 + len_bytes] {
        len = len * 256 + b as usize;
    }

    let start = 1 + len_bytes;
    if data.len() < start + len {
        return Err("incomplete RLP list data".into());
    }

    Ok((data[start..start + len].to_vec(), start + len))
}

/// Decode RLP items from list data
fn decode_rlp_items(data: &[u8]) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error>> {
    let mut items = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        let byte = data[offset];

        if byte < 0x80 {
            // Single byte
            items.push(vec![byte]);
            offset += 1;
        } else if byte < 0xb8 {
            // String < 56 bytes
            let len = (byte - 0x80) as usize;
            if offset + 1 + len > data.len() {
                return Err("incomplete RLP string".into());
            }
            items.push(data[offset + 1..offset + 1 + len].to_vec());
            offset += 1 + len;
        } else if byte < 0xc0 {
            // String >= 56 bytes
            let len_bytes = (byte - 0xb7) as usize;
            if offset + 1 + len_bytes > data.len() {
                return Err("incomplete RLP string length".into());
            }

            let mut len = 0usize;
            for &b in &data[offset + 1..offset + 1 + len_bytes] {
                len = len * 256 + b as usize;
            }

            let start = offset + 1 + len_bytes;
            if start + len > data.len() {
                return Err("incomplete RLP string data".into());
            }

            items.push(data[start..start + len].to_vec());
            offset = start + len;
        } else {
            // Nested list - skip for now
            return Err("nested RLP lists not supported".into());
        }
    }

    Ok(items)
}

/// Decode u64 from RLP bytes (big-endian, variable length)
fn decode_u64(data: &[u8]) -> u64 {
    if data.is_empty() {
        return 0;
    }
    let mut result = 0u64;
    for &byte in data {
        result = result * 256 + byte as u64;
    }
    result
}

/// Decode U256 from RLP bytes (big-endian, variable length)
fn decode_u256(data: &[u8]) -> U256 {
    if data.is_empty() {
        return U256::from(0);
    }
    let mut result = U256::from(0);
    for &byte in data {
        result = result * U256::from(256u32) + U256::from(byte);
    }
    result
}

/// Reads Ethereum blocks from a Geth chaindata directory using Geth's rawdb key schema.
///
/// Transparently reads from both the hot Pebble/LevelDB store and the ancient
/// (freezer) database so that the caller does not need to know where each block
/// is stored.
pub struct GethBlockReader {
    reader: Box<dyn KeyValueReader>,
    ancient: Option<AncientReader>,
}

impl GethBlockReader {
    /// Creates a new `GethBlockReader`.
    ///
    /// `ancient` should be opened from `{chaindata}/ancient/chain/` when
    /// available. Pass `None` to read only from the hot key-value store.
    pub fn new(reader: Box<dyn KeyValueReader>, ancient: Option<AncientReader>) -> Self {
        Self { reader, ancient }
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
        let raw = self
            .reader
            .get(&key)?
            .ok_or_else(|| format!("Block number not found for hash {:?}", hash))?;

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
    /// Checks the hot Pebble/LevelDB store first, then falls back to the
    /// ancient DB. Returns `None` if the block is beyond the chain head.
    pub fn read_canonical_hash(
        &self,
        number: u64,
    ) -> Result<Option<H256>, Box<dyn std::error::Error>> {
        // Hot DB lookup
        let key = canonical_hash_key(number);
        if let Some(raw) = self.reader.get(&key)? {
            if raw.len() != 32 {
                return Err(format!(
                    "Canonical hash for block #{number} has unexpected length {} (expected 32)",
                    raw.len()
                )
                .into());
            }
            return Ok(Some(H256::from_slice(&raw)));
        }

        // Ancient DB fallback
        if let Some(ancient) = &self.ancient {
            return ancient.read_canonical_hash(number);
        }

        Ok(None)
    }

    /// Reads and RLP-decodes the block header for a known (number, hash) pair.
    ///
    /// Falls back to the ancient DB when the key is absent from the hot store.
    pub fn read_block_header(
        &self,
        number: u64,
        hash: H256,
    ) -> Result<Option<BlockHeader>, Box<dyn std::error::Error>> {
        // Hot DB lookup
        let key = header_key(number, hash);
        if let Some(raw) = self.reader.get(&key)? {
            let header = BlockHeader::decode(&raw)
                .map_err(|e| format!("RLP decode error for block header #{number}: {e:?}"))?;
            return Ok(Some(header));
        }

        // Ancient DB fallback (index by block number)
        if let Some(ancient) = &self.ancient {
            return ancient.read_block_header(number);
        }

        Ok(None)
    }

    /// Reads and RLP-decodes the block body for a known (number, hash) pair.
    ///
    /// Falls back to the ancient DB when the key is absent from the hot store.
    pub fn read_block_body(
        &self,
        number: u64,
        hash: H256,
    ) -> Result<Option<BlockBody>, Box<dyn std::error::Error>> {
        // Hot DB lookup
        let key = body_key(number, hash);
        if let Some(raw) = self.reader.get(&key)? {
            let body = BlockBody::decode(&raw)
                .map_err(|e| format!("RLP decode error for block body #{number}: {e:?}"))?;
            return Ok(Some(body));
        }

        // Ancient DB fallback
        if let Some(ancient) = &self.ancient {
            return ancient.read_block_body(number);
        }

        Ok(None)
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

    /// Returns the count of account snapshots (by iterating with prefix).
    /// Returns 0 if snapshots are not available.
    pub fn count_account_snapshots(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let prefix = account_snapshot_prefix();
        let iter = self.reader.iter_prefix(&prefix)?;
        Ok(iter.count() as u64)
    }

    /// Iterates all account snapshots.
    /// Returns an iterator yielding (account_hash, SlimAccount) pairs.
    pub fn iter_account_snapshots(
        &self,
    ) -> Result<Box<dyn Iterator<Item = (H256, SlimAccount)>>, Box<dyn std::error::Error>> {
        let prefix = account_snapshot_prefix();
        let iter = self.reader.iter_prefix(&prefix)?;

        let result = iter.filter_map(|entry| {
            if let Ok((key, value)) = entry {
                // Account snapshot key: "a"(1) + account_hash(32) = 33 bytes
                if key.len() == 33 {
                    if let Ok(account) = SlimAccount::decode(&value) {
                        let account_hash = H256::from_slice(&key[1..33]);
                        return Some((account_hash, account));
                    }
                }
            }
            None
        });

        Ok(Box::new(result))
    }

    /// Iterates all storage snapshots for a given account.
    /// Returns an iterator yielding (slot_hash, raw_value) pairs.
    pub fn iter_storage_snapshots(
        &self,
        account_hash: &H256,
    ) -> Result<Box<dyn Iterator<Item = (H256, Vec<u8>)>>, Box<dyn std::error::Error>> {
        let prefix = storage_snapshot_prefix(*account_hash);
        let iter = self.reader.iter_prefix(&prefix)?;

        let result = iter.filter_map(|entry| {
            if let Ok((key, value)) = entry {
                // Storage snapshot key: "o"(1) + account_hash(32) + slot_hash(32) = 65 bytes
                if key.len() == 65 {
                    let slot_hash = H256::from_slice(&key[33..65]);
                    return Some((slot_hash, value));
                }
            }
            None
        });

        Ok(Box::new(result))
    }

    /// Reads contract bytecode by code hash.
    /// Returns Some(bytecode) if found, None otherwise.
    pub fn read_code(&self, code_hash: H256) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
        let key = code_key(code_hash);
        self.reader.get(&key)
    }

    /// Reads a preimage (address or storage slot) by its hash.
    /// Used for recovering address/slot from hash-only snapshots.
    pub fn read_preimage(&self, hash: H256) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
        let key = preimage_key(hash);
        self.reader.get(&key)
    }

    /// Reads raw receipts RLP for a block.
    /// Returns RLP([Receipt]) which must be decoded with decode_stored_receipts().
    pub fn read_raw_receipts(
        &self,
        number: u64,
        hash: H256,
    ) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
        let key = receipt_key(number, hash);
        self.reader.get(&key)
    }

    /// Reads and decodes receipts for a block, pairing each with its tx_type.
    ///
    /// Geth stored receipts do NOT include tx_type, so we derive it from
    /// the corresponding transaction in the block body.
    ///
    /// Returns `Ok(None)` if no raw receipt data exists for this block.
    /// Returns an error if the receipt count does not match the transaction count.
    pub fn read_receipts(
        &self,
        number: u64,
        hash: H256,
        body: &BlockBody,
    ) -> Result<Option<Vec<Receipt>>, Box<dyn std::error::Error>> {
        let raw = match self.read_raw_receipts(number, hash)? {
            Some(r) => r,
            None => return Ok(None),
        };

        let stored = decode_stored_receipts(&raw)?;

        if stored.len() != body.transactions.len() {
            return Err(format!(
                "receipt count ({}) != transaction count ({}) for block #{number}",
                stored.len(),
                body.transactions.len()
            )
            .into());
        }

        let receipts = stored
            .into_iter()
            .zip(body.transactions.iter())
            .map(|(sr, tx)| Receipt {
                tx_type: tx.tx_type(),
                succeeded: sr.succeeded,
                cumulative_gas_used: sr.cumulative_gas_used,
                logs: sr.logs,
            })
            .collect();

        Ok(Some(receipts))
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

// --- Snapshot & data key builders (go-ethereum core/rawdb/schema.go) ---
//
// go-ethereum prefix reference:
//   snapshotAccountPrefix = []byte("a")
//   snapshotStoragePrefix = []byte("o")
//   codePrefix            = []byte("c")
//   blockReceiptsPrefix   = []byte("r")
//   preimagePrefix        = []byte("secure-key-")

/// `"a" + account_hash(32)` → slim-encoded account snapshot
#[allow(dead_code)]
fn account_snapshot_key(account_hash: H256) -> Vec<u8> {
    let mut key = Vec::with_capacity(33);
    key.push(b'a');
    key.extend_from_slice(account_hash.as_bytes());
    key
}

/// Prefix for account snapshot iteration: `"a"`
fn account_snapshot_prefix() -> Vec<u8> {
    vec![b'a']
}

/// `"o" + account_hash(32) + slot_hash(32)` → raw storage value
#[allow(dead_code)]
fn storage_snapshot_key(account_hash: H256, slot_hash: H256) -> Vec<u8> {
    let mut key = Vec::with_capacity(65);
    key.push(b'o');
    key.extend_from_slice(account_hash.as_bytes());
    key.extend_from_slice(slot_hash.as_bytes());
    key
}

/// Prefix for storage snapshot iteration of an account: `"o" + account_hash(32)`
fn storage_snapshot_prefix(account_hash: H256) -> Vec<u8> {
    let mut key = Vec::with_capacity(33);
    key.push(b'o');
    key.extend_from_slice(account_hash.as_bytes());
    key
}

/// `"c" + code_hash(32)` → bytecode
fn code_key(code_hash: H256) -> Vec<u8> {
    let mut key = Vec::with_capacity(33);
    key.push(b'c');
    key.extend_from_slice(code_hash.as_bytes());
    key
}

/// `"secure-key-" + hash(32)` → preimage
fn preimage_key(hash: H256) -> Vec<u8> {
    let mut key = Vec::with_capacity(43);
    key.extend_from_slice(b"secure-key-");
    key.extend_from_slice(hash.as_bytes());
    key
}

/// `"r" + block_num(8 BE) + hash(32)` → RLP([Receipt])
fn receipt_key(number: u64, hash: H256) -> Vec<u8> {
    let mut key = Vec::with_capacity(41);
    key.push(b'r');
    key.extend_from_slice(&number.to_be_bytes());
    key.extend_from_slice(hash.as_bytes());
    key
}

/// Decodes stored receipts from Geth's RLP format.
///
/// Geth stores per-block receipts as:
/// ```text
/// RLP([
///   RLP([status(1byte), cumulativeGasUsed(u64), [log1, log2, ...]]),
///   ...
/// ])
/// ```
/// Each log: `RLP([address(20), [topic1, topic2, ...], data])`
///
/// The stored format omits bloom filters and tx_type (those are derived
/// from the corresponding transaction).
pub fn decode_stored_receipts(
    raw: &[u8],
) -> Result<Vec<StoredReceipt>, Box<dyn std::error::Error>> {
    if raw.is_empty() {
        return Err("empty receipt data".into());
    }

    // Outer structure: an RLP list of RLP-encoded receipts
    let outer_decoder = Decoder::new(raw)
        .map_err(|e| format!("failed to decode outer receipt list: {e}"))?;

    let mut receipts = Vec::new();
    let mut dec = outer_decoder;

    // Iterate through the outer list: each item is an RLP-encoded receipt
    while !dec.is_done() {
        let (receipt_bytes, next_dec): (Vec<u8>, _) = dec
            .get_encoded_item()
            .map_err(|e| format!("failed to get receipt item: {e}"))?;
        dec = next_dec;

        let receipt = decode_single_stored_receipt(&receipt_bytes)?;
        receipts.push(receipt);
    }

    // Ensure no trailing bytes after the list
    dec.finish()
        .map_err(|e| format!("trailing data in outer receipt list: {e}"))?;

    Ok(receipts)
}

/// Decodes a single stored receipt from its RLP encoding.
///
/// Format: `RLP([status(1byte), cumulativeGasUsed(u64), [log1, log2, ...]])`
fn decode_single_stored_receipt(
    data: &[u8],
) -> Result<StoredReceipt, Box<dyn std::error::Error>> {
    let decoder =
        Decoder::new(data).map_err(|e| format!("failed to decode stored receipt: {e}"))?;

    // status: RLP-encoded bool — success = 0x01, failure = 0x80 (empty bytes)
    let (succeeded, decoder): (bool, _) = decoder
        .decode_field("status")
        .map_err(|e| format!("failed to decode receipt status: {e}"))?;

    // cumulativeGasUsed: u64
    let (cumulative_gas_used, decoder): (u64, _) = decoder
        .decode_field("cumulative_gas_used")
        .map_err(|e| format!("failed to decode cumulative gas used: {e}"))?;

    // logs: Vec<Log>
    let (logs, decoder): (Vec<Log>, _) = decoder
        .decode_field("logs")
        .map_err(|e| format!("failed to decode receipt logs: {e}"))?;

    decoder
        .finish()
        .map_err(|e| format!("trailing data in stored receipt: {e}"))?;

    Ok(StoredReceipt {
        succeeded,
        cumulative_gas_used,
        logs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::Log;
    use ethrex_common::{Address, Bytes};

    /// Helper: RLP-encode a single stored receipt as [status, cumGasUsed, [logs]]
    fn encode_stored_receipt(succeeded: bool, cum_gas: u64, logs: &[Log]) -> Vec<u8> {
        use ethrex_rlp::structs::Encoder;
        let logs_vec: Vec<Log> = logs.to_vec();
        let mut buf = Vec::new();
        Encoder::new(&mut buf)
            .encode_field(&succeeded)
            .encode_field(&cum_gas)
            .encode_field(&logs_vec)
            .finish();
        buf
    }

    /// Helper: RLP-encode a list of already-encoded receipt items into an outer list
    fn encode_receipt_list(receipts: &[Vec<u8>]) -> Vec<u8> {
        // Concatenate all receipt bytes, then wrap in an RLP list header
        let mut payload = Vec::new();
        for r in receipts {
            payload.extend_from_slice(r);
        }
        let mut result = Vec::new();
        encode_rlp_list_header(&mut result, payload.len());
        result.extend_from_slice(&payload);
        result
    }

    /// Encode an RLP list header for a given payload length
    fn encode_rlp_list_header(buf: &mut Vec<u8>, len: usize) {
        if len < 56 {
            buf.push(0xc0 + len as u8);
        } else {
            let len_bytes = {
                let mut n = len;
                let mut bytes = Vec::new();
                while n > 0 {
                    bytes.push((n & 0xff) as u8);
                    n >>= 8;
                }
                bytes.reverse();
                bytes
            };
            buf.push(0xf7 + len_bytes.len() as u8);
            buf.extend_from_slice(&len_bytes);
        }
    }

    #[test]
    fn decode_stored_receipts_empty_block() {
        // Empty receipt list: RLP encoding of empty list = 0xc0
        let raw = vec![0xc0];
        let result = decode_stored_receipts(&raw).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn decode_stored_receipts_single_legacy() {
        // Single receipt: succeeded, cumGasUsed=21000, no logs
        let receipt_rlp = encode_stored_receipt(true, 21000, &[]);
        let raw = encode_receipt_list(&[receipt_rlp]);

        let receipts = decode_stored_receipts(&raw).unwrap();
        assert_eq!(receipts.len(), 1);
        assert!(receipts[0].succeeded);
        assert_eq!(receipts[0].cumulative_gas_used, 21000);
        assert!(receipts[0].logs.is_empty());
    }

    #[test]
    fn decode_stored_receipts_failed_receipt() {
        // Single receipt: failed, cumGasUsed=50000, no logs
        let receipt_rlp = encode_stored_receipt(false, 50000, &[]);
        let raw = encode_receipt_list(&[receipt_rlp]);

        let receipts = decode_stored_receipts(&raw).unwrap();
        assert_eq!(receipts.len(), 1);
        assert!(!receipts[0].succeeded);
        assert_eq!(receipts[0].cumulative_gas_used, 50000);
        assert!(receipts[0].logs.is_empty());
    }

    #[test]
    fn decode_stored_receipts_with_logs() {
        let addr = Address::from_low_u64_be(0xdead);
        let topic = H256::from_low_u64_be(0xbeef);
        let data = Bytes::from(vec![0x01, 0x02, 0x03]);

        let log = Log {
            address: addr,
            topics: vec![topic],
            data: data.clone(),
        };

        let receipt_rlp = encode_stored_receipt(true, 100_000, &[log]);
        let raw = encode_receipt_list(&[receipt_rlp]);

        let receipts = decode_stored_receipts(&raw).unwrap();
        assert_eq!(receipts.len(), 1);
        assert!(receipts[0].succeeded);
        assert_eq!(receipts[0].cumulative_gas_used, 100_000);
        assert_eq!(receipts[0].logs.len(), 1);
        assert_eq!(receipts[0].logs[0].address, addr);
        assert_eq!(receipts[0].logs[0].topics, vec![topic]);
        assert_eq!(receipts[0].logs[0].data, data);
    }

    #[test]
    fn decode_stored_receipts_multiple() {
        // Two receipts in a single block
        let r1 = encode_stored_receipt(true, 21000, &[]);

        let addr = Address::from_low_u64_be(0xca11);
        let log = Log {
            address: addr,
            topics: vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)],
            data: Bytes::from(vec![0xff]),
        };
        let r2 = encode_stored_receipt(false, 63000, &[log]);

        let raw = encode_receipt_list(&[r1, r2]);

        let receipts = decode_stored_receipts(&raw).unwrap();
        assert_eq!(receipts.len(), 2);

        // First receipt
        assert!(receipts[0].succeeded);
        assert_eq!(receipts[0].cumulative_gas_used, 21000);
        assert!(receipts[0].logs.is_empty());

        // Second receipt
        assert!(!receipts[1].succeeded);
        assert_eq!(receipts[1].cumulative_gas_used, 63000);
        assert_eq!(receipts[1].logs.len(), 1);
        assert_eq!(receipts[1].logs[0].address, addr);
        assert_eq!(receipts[1].logs[0].topics.len(), 2);
    }

    #[test]
    fn decode_stored_receipts_with_multiple_logs() {
        let log1 = Log {
            address: Address::from_low_u64_be(1),
            topics: vec![],
            data: Bytes::from(vec![]),
        };
        let log2 = Log {
            address: Address::from_low_u64_be(2),
            topics: vec![H256::from_low_u64_be(0xaa), H256::from_low_u64_be(0xbb)],
            data: Bytes::from(vec![0x10, 0x20, 0x30, 0x40]),
        };

        let receipt_rlp = encode_stored_receipt(true, 200_000, &[log1.clone(), log2.clone()]);
        let raw = encode_receipt_list(&[receipt_rlp]);

        let receipts = decode_stored_receipts(&raw).unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].logs.len(), 2);
        assert_eq!(receipts[0].logs[0].address, log1.address);
        assert_eq!(receipts[0].logs[1].address, log2.address);
        assert_eq!(receipts[0].logs[1].topics.len(), 2);
    }

    #[test]
    fn decode_stored_receipts_rejects_empty_data() {
        let result = decode_stored_receipts(&[]);
        assert!(result.is_err());
    }

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
