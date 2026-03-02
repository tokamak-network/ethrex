//! LMDB writer for py-ethclient compatible database
//!
//! Creates an LMDB database matching the schema used by
//! [py-ethclient](https://github.com/tokamak-network/py-ethclient).
//!
//! ## Sub-databases
//!
//! | DB Name          | Key                          | Value                        |
//! |------------------|------------------------------|------------------------------|
//! | `headers`        | block_hash (32B)             | RLP BlockHeader              |
//! | `bodies`         | block_hash (32B)             | RLP block body               |
//! | `canonical`      | block_number (8B BE)         | block_hash (32B)             |
//! | `header_numbers` | block_number (8B BE)         | block_hash (32B)             |
//! | `tx_index`       | tx_hash (32B)                | block_hash (32B) + idx (4B)  |
//! | `accounts`       | address (20B)                | RLP Account                  |
//! | `code`           | code_hash (32B)              | raw bytecode                 |
//! | `storage`        | address (20B) + slot (32B)   | minimal BE int               |
//! | `snap_accounts`  | account_hash (32B)           | RLP Account                  |
//! | `snap_storage`   | acct_hash (32B) + slot_hash (32B) | raw value               |
//! | `meta`           | string key                   | varies                       |

use std::fs;
use std::path::Path;

use heed::types::Bytes;
use heed::{Database, Env, EnvOpenOptions};

/// LMDB writer compatible with py-ethclient's database schema.
pub struct LmdbWriter {
    env: Env,
    headers: Database<Bytes, Bytes>,
    bodies: Database<Bytes, Bytes>,
    canonical: Database<Bytes, Bytes>,
    header_numbers: Database<Bytes, Bytes>,
    tx_index: Database<Bytes, Bytes>,
    accounts: Database<Bytes, Bytes>,
    code: Database<Bytes, Bytes>,
    storage: Database<Bytes, Bytes>,
    original_storage: Database<Bytes, Bytes>,
    receipts: Database<Bytes, Bytes>,
    snap_accounts: Database<Bytes, Bytes>,
    snap_storage: Database<Bytes, Bytes>,
    meta: Database<Bytes, Bytes>,
}

impl LmdbWriter {
    /// Creates a new LMDB environment at `path` with the given map size.
    ///
    /// The directory is created if it doesn't exist.
    /// All 11 named sub-databases are opened/created.
    pub fn create(path: &Path, map_size_bytes: usize) -> Result<Self, Box<dyn std::error::Error>> {
        fs::create_dir_all(path)?;

        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(map_size_bytes)
                .max_dbs(16) // py-ethclient uses up to 13 named DBs
                .open(path)?
        };

        // Create all named sub-databases
        let mut wtxn = env.write_txn()?;
        let headers = env.create_database(&mut wtxn, Some("headers"))?;
        let bodies = env.create_database(&mut wtxn, Some("bodies"))?;
        let canonical = env.create_database(&mut wtxn, Some("canonical"))?;
        let header_numbers = env.create_database(&mut wtxn, Some("header_numbers"))?;
        let tx_index = env.create_database(&mut wtxn, Some("tx_index"))?;
        let accounts = env.create_database(&mut wtxn, Some("accounts"))?;
        let code = env.create_database(&mut wtxn, Some("code"))?;
        let storage = env.create_database(&mut wtxn, Some("storage"))?;
        let original_storage = env.create_database(&mut wtxn, Some("original_storage"))?;
        let receipts = env.create_database(&mut wtxn, Some("receipts"))?;
        let snap_accounts = env.create_database(&mut wtxn, Some("snap_accounts"))?;
        let snap_storage = env.create_database(&mut wtxn, Some("snap_storage"))?;
        let meta = env.create_database(&mut wtxn, Some("meta"))?;
        wtxn.commit()?;

        Ok(Self {
            env,
            headers,
            bodies,
            canonical,
            header_numbers,
            tx_index,
            accounts,
            code,
            storage,
            original_storage,
            receipts,
            snap_accounts,
            snap_storage,
            meta,
        })
    }

    /// Returns a new write transaction.
    pub fn write_txn(&self) -> Result<heed::RwTxn<'_>, heed::Error> {
        self.env.write_txn()
    }

    /// Returns a new read transaction.
    #[allow(dead_code)]
    pub fn read_txn(&self) -> Result<heed::RoTxn<'_>, heed::Error> {
        self.env.read_txn()
    }

    /// Writes a block header to the `headers` DB.
    pub fn put_header(
        &self,
        txn: &mut heed::RwTxn,
        block_hash: &[u8; 32],
        header_rlp: &[u8],
    ) -> Result<(), heed::Error> {
        self.headers.put(txn, block_hash, header_rlp)?;
        Ok(())
    }

    /// Writes a block body to the `bodies` DB.
    pub fn put_body(
        &self,
        txn: &mut heed::RwTxn,
        block_hash: &[u8; 32],
        body_rlp: &[u8],
    ) -> Result<(), heed::Error> {
        self.bodies.put(txn, block_hash, body_rlp)?;
        Ok(())
    }

    /// Writes a canonical block mapping.
    pub fn put_canonical(
        &self,
        txn: &mut heed::RwTxn,
        block_number: u64,
        block_hash: &[u8; 32],
    ) -> Result<(), heed::Error> {
        let key = block_number.to_be_bytes();
        self.canonical.put(txn, &key, block_hash)?;
        Ok(())
    }

    /// Writes a header_numbers mapping (same key/value as canonical).
    pub fn put_header_number(
        &self,
        txn: &mut heed::RwTxn,
        block_number: u64,
        block_hash: &[u8; 32],
    ) -> Result<(), heed::Error> {
        let key = block_number.to_be_bytes();
        self.header_numbers.put(txn, &key, block_hash)?;
        Ok(())
    }

    /// Writes a transaction index entry.
    ///
    /// Value: block_hash (32 bytes) + tx_index (4 bytes BE).
    pub fn put_tx_index(
        &self,
        txn: &mut heed::RwTxn,
        tx_hash: &[u8; 32],
        block_hash: &[u8; 32],
        tx_idx: u32,
    ) -> Result<(), heed::Error> {
        let mut value = Vec::with_capacity(36);
        value.extend_from_slice(block_hash);
        value.extend_from_slice(&tx_idx.to_be_bytes());
        self.tx_index.put(txn, tx_hash, &value)?;
        Ok(())
    }

    /// Writes an account entry (address-keyed).
    pub fn put_account(
        &self,
        txn: &mut heed::RwTxn,
        address: &[u8; 20],
        account_rlp: &[u8],
    ) -> Result<(), heed::Error> {
        self.accounts.put(txn, address.as_slice(), account_rlp)?;
        Ok(())
    }

    /// Writes contract code.
    pub fn put_code(
        &self,
        txn: &mut heed::RwTxn,
        code_hash: &[u8; 32],
        bytecode: &[u8],
    ) -> Result<(), heed::Error> {
        self.code.put(txn, code_hash, bytecode)?;
        Ok(())
    }

    /// Writes block receipts to the `receipts` DB.
    ///
    /// Key: block_hash (32B), Value: RLP-encoded receipt list.
    pub fn put_receipts(
        &self,
        txn: &mut heed::RwTxn,
        block_hash: &[u8; 32],
        receipts_rlp: &[u8],
    ) -> Result<(), heed::Error> {
        self.receipts.put(txn, block_hash, receipts_rlp)?;
        Ok(())
    }

    /// Writes a storage slot (address-keyed).
    ///
    /// Key: address (20B) + slot (32B) = 52 bytes, Value: minimal BE int.
    pub fn put_storage(
        &self,
        txn: &mut heed::RwTxn,
        address: &[u8; 20],
        slot: &[u8; 32],
        value: &[u8],
    ) -> Result<(), heed::Error> {
        let mut key = Vec::with_capacity(52);
        key.extend_from_slice(address);
        key.extend_from_slice(slot);
        self.storage.put(txn, &key, value)?;
        Ok(())
    }

    /// Writes an original_storage slot (address-keyed).
    ///
    /// Key: address (20B) + slot (32B) = 52 bytes, Value: minimal BE int.
    /// At migration time current storage equals original storage.
    pub fn put_original_storage(
        &self,
        txn: &mut heed::RwTxn,
        address: &[u8; 20],
        slot: &[u8; 32],
        value: &[u8],
    ) -> Result<(), heed::Error> {
        let mut key = Vec::with_capacity(52);
        key.extend_from_slice(address);
        key.extend_from_slice(slot);
        self.original_storage.put(txn, &key, value)?;
        Ok(())
    }

    /// Writes a snap_accounts entry (hash-keyed).
    pub fn put_snap_account(
        &self,
        txn: &mut heed::RwTxn,
        account_hash: &[u8; 32],
        account_rlp: &[u8],
    ) -> Result<(), heed::Error> {
        self.snap_accounts.put(txn, account_hash, account_rlp)?;
        Ok(())
    }

    /// Writes a snap_storage entry (hash-keyed).
    ///
    /// Key: account_hash (32B) + slot_hash (32B) = 64 bytes.
    pub fn put_snap_storage(
        &self,
        txn: &mut heed::RwTxn,
        account_hash: &[u8; 32],
        slot_hash: &[u8; 32],
        value: &[u8],
    ) -> Result<(), heed::Error> {
        let mut key = Vec::with_capacity(64);
        key.extend_from_slice(account_hash);
        key.extend_from_slice(slot_hash);
        self.snap_storage.put(txn, &key, value)?;
        Ok(())
    }

    /// Sets the `latest_block` entry in the `meta` DB.
    ///
    /// Value: 8-byte signed big-endian (matching py-ethclient's encoding).
    pub fn set_latest_block(
        &self,
        txn: &mut heed::RwTxn,
        block_number: u64,
    ) -> Result<(), heed::Error> {
        let value = (block_number as i64).to_be_bytes();
        self.meta.put(txn, b"latest_block", &value)?;
        Ok(())
    }

    /// Sets the `snap_progress` entry in the `meta` DB.
    ///
    /// Value: JSON bytes describing sync progress.
    pub fn set_snap_progress(
        &self,
        txn: &mut heed::RwTxn,
        json_bytes: &[u8],
    ) -> Result<(), heed::Error> {
        self.meta.put(txn, b"snap_progress", json_bytes)?;
        Ok(())
    }

    // --- Read helpers (for verification) ---

    /// Reads a header from the DB.
    #[allow(dead_code)]
    pub fn get_header(
        &self,
        txn: &heed::RoTxn,
        block_hash: &[u8; 32],
    ) -> Result<Option<Vec<u8>>, heed::Error> {
        Ok(self.headers.get(txn, block_hash)?.map(|v| v.to_vec()))
    }

    /// Reads a canonical hash for a block number.
    #[allow(dead_code)]
    pub fn get_canonical(
        &self,
        txn: &heed::RoTxn,
        block_number: u64,
    ) -> Result<Option<Vec<u8>>, heed::Error> {
        let key = block_number.to_be_bytes();
        Ok(self.canonical.get(txn, &key)?.map(|v| v.to_vec()))
    }

    /// Reads the latest_block from meta DB.
    #[allow(dead_code)]
    pub fn get_latest_block(&self, txn: &heed::RoTxn) -> Result<Option<i64>, heed::Error> {
        match self.meta.get(txn, b"latest_block")? {
            Some(val) if val.len() == 8 => Ok(Some(i64::from_be_bytes(val.try_into().unwrap()))),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_lmdb_writer_creates_all_dbs() {
        let dir = tempfile::tempdir().unwrap();
        let writer = LmdbWriter::create(dir.path(), 10 * 1024 * 1024).unwrap();

        // Write to all DBs
        let mut txn = writer.write_txn().unwrap();

        let hash = [0xaau8; 32];
        writer.put_header(&mut txn, &hash, b"header_rlp").unwrap();
        writer.put_body(&mut txn, &hash, b"body_rlp").unwrap();
        writer.put_canonical(&mut txn, 42, &hash).unwrap();
        writer.put_header_number(&mut txn, 42, &hash).unwrap();

        let tx_hash = [0xbbu8; 32];
        writer.put_tx_index(&mut txn, &tx_hash, &hash, 0).unwrap();

        let addr = [0xccu8; 20];
        writer.put_account(&mut txn, &addr, b"account_rlp").unwrap();
        writer.put_code(&mut txn, &hash, b"bytecode").unwrap();

        let slot = [0xddu8; 32];
        writer.put_storage(&mut txn, &addr, &slot, b"\x01").unwrap();

        writer
            .put_snap_account(&mut txn, &hash, b"snap_rlp")
            .unwrap();

        let slot_hash = [0xeeu8; 32];
        writer
            .put_snap_storage(&mut txn, &hash, &slot_hash, b"\x42")
            .unwrap();

        writer.set_latest_block(&mut txn, 42).unwrap();
        txn.commit().unwrap();

        // Read back and verify
        let rtxn = writer.read_txn().unwrap();

        let val = writer.get_header(&rtxn, &hash).unwrap();
        assert_eq!(val.as_deref(), Some(b"header_rlp".as_slice()));

        let val = writer.get_canonical(&rtxn, 42).unwrap();
        assert_eq!(val.as_deref(), Some(hash.as_slice()));

        let val = writer.get_latest_block(&rtxn).unwrap();
        assert_eq!(val, Some(42));
    }

    #[test]
    fn receipts_db_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let writer = LmdbWriter::create(dir.path(), 10 * 1024 * 1024).unwrap();

        let mut txn = writer.write_txn().unwrap();
        let hash = [0xaau8; 32];
        let receipt_data = b"some_receipt_rlp";
        writer.put_receipts(&mut txn, &hash, receipt_data).unwrap();
        txn.commit().unwrap();

        let rtxn = writer.read_txn().unwrap();
        let val = writer.receipts.get(&rtxn, &hash).unwrap().unwrap();
        assert_eq!(val, receipt_data);
    }

    #[test]
    fn original_storage_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let writer = LmdbWriter::create(dir.path(), 10 * 1024 * 1024).unwrap();

        let mut txn = writer.write_txn().unwrap();
        let addr = [0xaau8; 20];
        let slot = [0xbbu8; 32];
        writer
            .put_original_storage(&mut txn, &addr, &slot, b"\x42")
            .unwrap();
        txn.commit().unwrap();

        let rtxn = writer.read_txn().unwrap();
        let mut key = Vec::with_capacity(52);
        key.extend_from_slice(&addr);
        key.extend_from_slice(&slot);
        let val = writer.original_storage.get(&rtxn, &key).unwrap().unwrap();
        assert_eq!(val, b"\x42");
    }

    #[test]
    fn snap_progress_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let writer = LmdbWriter::create(dir.path(), 10 * 1024 * 1024).unwrap();

        let mut txn = writer.write_txn().unwrap();
        let json = br#"{"done":true,"synced_accounts":100}"#;
        writer.set_snap_progress(&mut txn, json).unwrap();
        txn.commit().unwrap();

        let rtxn = writer.read_txn().unwrap();
        let val = writer.meta.get(&rtxn, b"snap_progress").unwrap().unwrap();
        assert_eq!(val, json.as_slice());
    }

    #[test]
    fn tx_index_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let writer = LmdbWriter::create(dir.path(), 10 * 1024 * 1024).unwrap();

        let mut txn = writer.write_txn().unwrap();
        let tx_hash = [0x11u8; 32];
        let block_hash = [0x22u8; 32];
        writer
            .put_tx_index(&mut txn, &tx_hash, &block_hash, 5)
            .unwrap();
        txn.commit().unwrap();

        let rtxn = writer.read_txn().unwrap();
        let val = writer.tx_index.get(&rtxn, &tx_hash).unwrap().unwrap();
        assert_eq!(val.len(), 36);
        assert_eq!(&val[..32], &block_hash);
        assert_eq!(&val[32..], &5u32.to_be_bytes());
    }

    #[test]
    fn storage_key_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let writer = LmdbWriter::create(dir.path(), 10 * 1024 * 1024).unwrap();

        let mut txn = writer.write_txn().unwrap();
        let addr = [0xaau8; 20];
        let slot = [0xbbu8; 32];
        writer
            .put_storage(&mut txn, &addr, &slot, b"\x01\x00")
            .unwrap();
        txn.commit().unwrap();

        let rtxn = writer.read_txn().unwrap();
        let mut key = Vec::with_capacity(52);
        key.extend_from_slice(&addr);
        key.extend_from_slice(&slot);
        let val = writer.storage.get(&rtxn, &key).unwrap().unwrap();
        assert_eq!(val, b"\x01\x00");
    }

    #[test]
    fn latest_block_signed_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let writer = LmdbWriter::create(dir.path(), 10 * 1024 * 1024).unwrap();

        let mut txn = writer.write_txn().unwrap();
        writer.set_latest_block(&mut txn, 1_000_000).unwrap();
        txn.commit().unwrap();

        let rtxn = writer.read_txn().unwrap();
        let num = writer.get_latest_block(&rtxn).unwrap().unwrap();
        assert_eq!(num, 1_000_000);
    }
}
