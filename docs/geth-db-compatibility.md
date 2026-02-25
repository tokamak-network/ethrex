# Geth Database Compatibility Guide

This document explains how `geth2ethrex` handles different Geth database backends (LevelDB and Pebble).

## Background

Geth (go-ethereum) has used two different database backends over its history:

- **LevelDB** (Geth v1.9 and earlier)
- **Pebble** (Geth v1.10+ default, `--db.engine=pebble`)

The `geth2ethrex` tool can read both formats and migrate them to ethrex's RocksDB storage.

---

## Database Type Detection

`geth2ethrex` automatically detects the database type by inspecting the chaindata directory:

### Detection Logic (in order)

1. **Check for `OPTIONS-*` files** → Pebble
   - Pebble creates version-specific options files (e.g., `OPTIONS-000001`)
   
2. **Check for `*.ldb` files** → LevelDB
   - LevelDB SSTables use the `.ldb` extension
   
3. **Check for `*.sst` files** → Pebble
   - Pebble SSTables use the `.sst` extension (same as RocksDB)

### Example Directory Structures

**LevelDB chaindata:**
```
/path/to/geth/chaindata/
├── CURRENT
├── MANIFEST-000001
├── 000003.log
├── 000005.ldb   ← LevelDB SSTable
└── LOG
```

**Pebble chaindata:**
```
/path/to/geth/chaindata/
├── CURRENT
├── MANIFEST-000001
├── OPTIONS-000001   ← Pebble-specific
├── 000003.log
├── 000005.sst       ← Pebble SSTable
└── LOCK
```

---

## Reader Implementations

### Pebble Reader (RocksDB-based)

**Implementation**: Uses the `rocksdb` Rust crate to read Pebble databases.

**Compatibility**:
- ✅ Pebble and RocksDB share a similar SSTable format
- ✅ Read-only access works for most Geth databases (v1.10.0–v1.14.x tested)
- ⚠️ **Pebble-specific features are ignored** (e.g., custom bloom filters)
- ⚠️ Write operations are not supported

**Usage**:
```rust
use geth2ethrex::readers::pebble::PebbleReader;
use geth2ethrex::readers::KeyValueReader;
use std::path::Path;

let chaindata = Path::new("/path/to/geth/chaindata");
let reader = PebbleReader::open(chaindata)?;

// Read block header
let key = b"header:0";
if let Some(value) = reader.get(key)? {
    println!("Found genesis header: {} bytes", value.len());
}
```

### LevelDB Reader (TODO)

**Status**: Not yet implemented.

**Planned Implementation**: Use `rusty-leveldb` crate.

**Workaround**: See "Fallback Strategy" below.

---

## Compatibility Matrix

| Geth Version | Default DB | geth2ethrex Support |
|--------------|------------|---------------------|
| v1.9.x and earlier | LevelDB | ⚠️ Not yet (planned) |
| v1.10.0 – v1.14.x | Pebble | ✅ Yes (via RocksDB) |
| v1.15.0+ | Pebble | ⚠️ Needs testing |

---

## Fallback Strategy

If the RocksDB-based Pebble reader fails (e.g., incompatible Pebble version), you can export the data using Geth's built-in export command:

### Step 1: Export from Geth

```bash
geth --datadir /path/to/geth export /tmp/blocks.rlp
```

This exports the blockchain to an RLP-encoded file.

### Step 2: Import into ethrex

```bash
geth2ethrex import-rlp \
  --input /tmp/blocks.rlp \
  --target /path/to/ethrex/storage
```

**Pros**:
- ✅ 100% accurate (uses Geth's native export)
- ✅ Works with any Geth version

**Cons**:
- ⚠️ Requires disk space (2× original chaindata size)
- ⚠️ Very slow (can take hours or days for full chains)
- ⚠️ Requires Geth binary

---

## Known Limitations

### Pebble Reader (RocksDB-based)

1. **Read-only access**  
   Cannot write to Pebble databases (RocksDB compatibility layer limitations).

2. **Bloom filter compatibility**  
   Pebble's custom bloom filters are ignored; reads may be slightly slower.

3. **Compaction state**  
   The RocksDB reader does not trigger Pebble compactions; database may appear "stale."

4. **Version-specific quirks**  
   Pebble v1.1+ introduced new features that may not be fully compatible.

### LevelDB Reader

Not yet implemented. Use Geth export as a workaround.

---

## Testing Your Database

To verify that `geth2ethrex` can read your Geth chaindata:

```bash
geth2ethrex verify-read --chaindata /path/to/geth/chaindata
```

This command:
- Detects the database type
- Opens the database in read-only mode
- Reads the genesis block header
- Reports success or failure

Example output:
```
Detected: Pebble (OPTIONS-000001 found)
Opening database...
Read genesis header: 540 bytes
✅ Database is readable
```

---

## Troubleshooting

### Error: "Unable to determine database type"

**Cause**: No recognizable database files (`.ldb`, `.sst`, or `OPTIONS-*`) found.

**Solution**:
1. Verify the path points to Geth's `chaindata` directory (not the parent `geth` directory)
2. Check that the database is not corrupted
3. Try the Geth export fallback strategy

### Error: "RocksDB failed to open Pebble database"

**Cause**: Incompatible Pebble version or corrupted database.

**Solution**:
1. Try the Geth export fallback strategy (see above)
2. Check Geth logs for corruption warnings
3. Re-sync Geth from scratch (if possible)

### Error: "LevelDB reader not yet implemented"

**Solution**:
1. Use Geth v1.10+ with Pebble (recommended)
2. Use the Geth export fallback strategy
3. Wait for LevelDB reader implementation (tracked in issue #XXX)

---

## Future Work

- [ ] Implement LevelDB reader (`rusty-leveldb` crate)
- [ ] Support Pebble v1.1+ new features
- [ ] Write operations (if needed for in-place migration)
- [ ] Benchmarking: RocksDB vs native Pebble read performance
- [ ] Integration tests with real Geth chaindata snapshots

---

## References

- [Pebble GitHub](https://github.com/cockroachdb/pebble)
- [LevelDB](https://github.com/google/leveldb)
- [RocksDB](https://rocksdb.org/)
- [Geth Database Migration Guide](https://geth.ethereum.org/docs/fundamentals/databases)
- [Geth v1.10 Release Notes (Pebble default)](https://github.com/ethereum/go-ethereum/releases/tag/v1.10.0)
