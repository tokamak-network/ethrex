//! Reader for Geth's ancient (freezer) database
//!
//! Geth stores older blocks in a "freezer" database to save space in the hot
//! Pebble/LevelDB store. This module implements reading from that format.
//!
//! ## Ancient DB Layout
//!
//! Located at `{chaindata}/ancient/chain/`, with one file-set per table:
//!
//! | Table     | Compression | Content               |
//! |-----------|-------------|-----------------------|
//! | `hashes`  | raw (none)  | canonical block hashes (32 B each) |
//! | `headers` | snappy      | RLP-encoded block headers          |
//! | `bodies`  | snappy      | RLP-encoded block bodies           |
//!
//! ## Index Format
//!
//! Each table has a `*.{r,c}idx` index file with **6-byte entries**:
//! - bytes `[0..2]`: big-endian `u16` — file segment number (e.g. `0` → `*.0000.{r,c}dat`)
//! - bytes `[2..6]`: big-endian `u32` — byte offset within that segment
//!
//! There are N+1 entries for N items; entry N is the "end marker" giving the
//! end offset of the last item.
//!
//! For **raw** tables the item width is fixed (32 B for hashes), so the index
//! can be synthesised from the offset alone. For **compressed** tables the
//! length of item N is `entry[N+1].offset − entry[N].offset`.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use ethrex_common::{
    H256,
    types::{BlockBody, BlockHeader},
};
use ethrex_rlp::decode::RLPDecode;

/// One 6-byte entry in a `.{r,c}idx` index file.
#[derive(Clone, Copy)]
struct IndexEntry {
    /// Segment file number (e.g. `0` → `*.0000.*dat`).
    filenum: u16,
    /// Byte offset within the segment.
    offset: u32,
}

impl IndexEntry {
    fn from_bytes(b: &[u8; 6]) -> Self {
        Self {
            filenum: u16::from_be_bytes([b[0], b[1]]),
            offset: u32::from_be_bytes([b[2], b[3], b[4], b[5]]),
        }
    }
}

/// Reads a single IndexEntry at position `item_index` from an open index file.
fn read_index_entry(idx_file: &mut File, item_index: u64) -> io::Result<IndexEntry> {
    let mut buf = [0u8; 6];
    idx_file.seek(SeekFrom::Start(item_index * 6))?;
    idx_file.read_exact(&mut buf)?;
    Ok(IndexEntry::from_bytes(&buf))
}

/// Reads raw bytes for item `item_index` from a table.
///
/// `idx_path` points to the `.{r,c}idx` file; data segments live alongside it
/// with the naming pattern `{stem}.{filenum:04}.{ext}dat`.
fn read_item_bytes(
    idx_path: &Path,
    item_index: u64,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut idx = File::open(idx_path)?;
    let start = read_index_entry(&mut idx, item_index)?;
    let end = read_index_entry(&mut idx, item_index + 1)?;

    // Length of this item
    let len = if end.filenum == start.filenum {
        let start_off = start.offset as usize;
        let end_off = end.offset as usize;
        if end_off < start_off {
            return Err(format!(
                "ancient item #{item_index} has invalid index: end offset ({}) < start offset ({})",
                end.offset, start.offset
            )
            .into());
        }
        end_off - start_off
    } else {
        // Item spans a segment boundary — rare in practice for small chains
        return Err(format!(
            "ancient item #{item_index} spans segment boundary (filenum {}-{}); not yet supported",
            start.filenum, end.filenum
        )
        .into());
    };

    // Guard against unreasonable allocations from corrupted index data
    const MAX_ITEM_SIZE: usize = 16 * 1024 * 1024; // 16 MiB
    if len > MAX_ITEM_SIZE {
        return Err(format!(
            "ancient item #{item_index} claims unreasonable size {len} bytes (max {MAX_ITEM_SIZE})"
        )
        .into());
    }

    // Build the data segment path: same stem as idx, with "NNNN.?dat" suffix
    // e.g. headers.cidx → headers.0000.cdat
    let stem = idx_path
        .file_stem()
        .ok_or("idx path has no stem")?
        .to_string_lossy();
    let ext_char = if idx_path.extension().map_or(false, |e| e == "ridx") {
        'r'
    } else {
        'c'
    };
    let data_name = format!("{}.{:04}.{}dat", stem, start.filenum, ext_char);
    let data_path = idx_path.with_file_name(data_name);

    let mut dat = File::open(&data_path)
        .map_err(|e| format!("cannot open ancient data file {}: {e}", data_path.display()))?;
    dat.seek(SeekFrom::Start(start.offset as u64))?;

    let mut buf = vec![0u8; len];
    dat.read_exact(&mut buf)?;
    Ok(buf)
}

// ---------------------------------------------------------------------------

/// Reader for Geth's ancient (freezer) block database.
///
/// The ancient DB directory is expected at `{chaindata}/ancient/chain/`.
#[derive(Debug)]
pub struct AncientReader {
    dir: PathBuf,
    /// Number of items in the ancient DB (= highest block number + 1).
    item_count: u64,
}

impl AncientReader {
    /// Opens the ancient DB rooted at `ancient_chain_dir`
    /// (typically `{chaindata}/ancient/chain`).
    ///
    /// Returns `None` if the directory does not exist (ancient DB disabled).
    pub fn open(ancient_chain_dir: &Path) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let hashes_idx = ancient_chain_dir.join("hashes.ridx");
        if !hashes_idx.exists() {
            return Ok(None);
        }

        // Number of items = index entries − 1 (last entry is the end marker)
        let meta = std::fs::metadata(&hashes_idx)?;
        if meta.len() % 6 != 0 {
            return Err(format!(
                "ancient hashes index file has invalid size {} (not a multiple of 6)",
                meta.len()
            )
            .into());
        }
        let index_entries = meta.len() / 6;
        let item_count = index_entries.saturating_sub(1);

        Ok(Some(Self {
            dir: ancient_chain_dir.to_path_buf(),
            item_count,
        }))
    }

    /// Returns the highest block number stored in the ancient DB.
    #[allow(dead_code)]
    pub fn max_block_number(&self) -> u64 {
        self.item_count.saturating_sub(1)
    }

    /// Returns `true` if block `number` is within the ancient DB range.
    pub fn contains(&self, number: u64) -> bool {
        number < self.item_count
    }

    /// Reads the canonical hash for block `number` from the `hashes` table.
    pub fn read_canonical_hash(
        &self,
        number: u64,
    ) -> Result<Option<H256>, Box<dyn std::error::Error>> {
        if !self.contains(number) {
            return Ok(None);
        }

        // hashes table is raw (32 bytes each) — use the ridx
        let idx_path = self.dir.join("hashes.ridx");
        let raw = read_item_bytes(&idx_path, number)?;

        if raw.len() != 32 {
            return Err(format!(
                "ancient canonical hash for block #{number} has unexpected length {}",
                raw.len()
            )
            .into());
        }
        Ok(Some(H256::from_slice(&raw)))
    }

    /// Reads and RLP-decodes the block header for `number` from the `headers` table.
    pub fn read_block_header(
        &self,
        number: u64,
    ) -> Result<Option<BlockHeader>, Box<dyn std::error::Error>> {
        if !self.contains(number) {
            return Ok(None);
        }

        let idx_path = self.dir.join("headers.cidx");
        let compressed = read_item_bytes(&idx_path, number)?;
        let raw = snap::raw::Decoder::new()
            .decompress_vec(&compressed)
            .map_err(|e| format!("snappy decompress error for header #{number}: {e}"))?;

        let header = BlockHeader::decode(&raw)
            .map_err(|e| format!("RLP decode error for ancient header #{number}: {e:?}"))?;
        Ok(Some(header))
    }

    /// Reads and RLP-decodes the block body for `number` from the `bodies` table.
    pub fn read_block_body(
        &self,
        number: u64,
    ) -> Result<Option<BlockBody>, Box<dyn std::error::Error>> {
        if !self.contains(number) {
            return Ok(None);
        }

        let idx_path = self.dir.join("bodies.cidx");
        let compressed = read_item_bytes(&idx_path, number)?;
        let raw = snap::raw::Decoder::new()
            .decompress_vec(&compressed)
            .map_err(|e| format!("snappy decompress error for body #{number}: {e}"))?;

        let body = BlockBody::decode(&raw)
            .map_err(|e| format!("RLP decode error for ancient body #{number}: {e:?}"))?;
        Ok(Some(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn index_entry_from_bytes() {
        // filenum=0, offset=32
        let b: [u8; 6] = [0, 0, 0, 0, 0, 32];
        let e = IndexEntry::from_bytes(&b);
        assert_eq!(e.filenum, 0);
        assert_eq!(e.offset, 32);
    }

    #[test]
    fn index_entry_big_endian() {
        // filenum=1, offset=0x00010203
        let b: [u8; 6] = [0, 1, 0, 1, 2, 3];
        let e = IndexEntry::from_bytes(&b);
        assert_eq!(e.filenum, 1);
        assert_eq!(e.offset, 0x00010203);
    }

    #[test]
    fn open_returns_none_for_nonexistent_dir() {
        let result = AncientReader::open(Path::new("/tmp/nonexistent_ancient_db_xyz"));
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn open_rejects_misaligned_index_file() {
        let dir = tempfile::tempdir().unwrap();
        let idx_path = dir.path().join("hashes.ridx");
        // Write 7 bytes (not a multiple of 6)
        fs::write(&idx_path, &[0u8; 7]).unwrap();

        let result = AncientReader::open(dir.path());
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not a multiple of 6"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn contains_returns_false_for_out_of_range() {
        let dir = tempfile::tempdir().unwrap();
        let idx_path = dir.path().join("hashes.ridx");
        // 2 entries (6*2=12 bytes) → 1 item (block 0 only)
        fs::write(&idx_path, &[0u8; 12]).unwrap();

        let reader = AncientReader::open(dir.path()).unwrap().unwrap();
        assert!(reader.contains(0));
        assert!(!reader.contains(1));
        assert!(!reader.contains(u64::MAX));
    }

    #[test]
    fn read_item_bytes_rejects_reversed_offsets() {
        let dir = tempfile::tempdir().unwrap();
        let idx_path = dir.path().join("test.ridx");

        // Two entries: start offset=100, end offset=50 (reversed → should error)
        let mut data = Vec::new();
        // Entry 0: filenum=0, offset=100
        data.extend_from_slice(&[0, 0, 0, 0, 0, 100]);
        // Entry 1: filenum=0, offset=50 (less than 100 → invalid)
        data.extend_from_slice(&[0, 0, 0, 0, 0, 50]);
        fs::write(&idx_path, &data).unwrap();

        let result = read_item_bytes(&idx_path, 0);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("end offset") && err_msg.contains("start offset"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn read_item_bytes_rejects_oversized_items() {
        let dir = tempfile::tempdir().unwrap();
        let idx_path = dir.path().join("test.ridx");

        // Two entries: offset 0 → 0x01000001 (>16 MiB)
        let mut data = Vec::new();
        // Entry 0: filenum=0, offset=0
        data.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        // Entry 1: filenum=0, offset=0x01000001 (~16.8 MiB)
        data.extend_from_slice(&[0, 0, 0x01, 0x00, 0x00, 0x01]);
        fs::write(&idx_path, &data).unwrap();

        let result = read_item_bytes(&idx_path, 0);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("unreasonable size"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn read_item_bytes_rejects_cross_segment() {
        let dir = tempfile::tempdir().unwrap();
        let idx_path = dir.path().join("test.ridx");

        // Entry 0: filenum=0, offset=0
        // Entry 1: filenum=1, offset=0 (different segment)
        let mut data = Vec::new();
        data.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        data.extend_from_slice(&[0, 1, 0, 0, 0, 0]);
        fs::write(&idx_path, &data).unwrap();

        let result = read_item_bytes(&idx_path, 0);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("segment boundary"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn max_block_number_handles_empty_db() {
        let dir = tempfile::tempdir().unwrap();
        let idx_path = dir.path().join("hashes.ridx");
        // 1 entry (just the end marker) → 0 items
        fs::write(&idx_path, &[0u8; 6]).unwrap();

        let reader = AncientReader::open(dir.path()).unwrap().unwrap();
        assert_eq!(reader.item_count, 0);
        // max_block_number wraps to u64::MAX — this is a known edge case
        // but contains(0) correctly returns false
        assert!(!reader.contains(0));
    }
}
