# g2r ethrex-Ready Output — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `geth-db-migrate g2r` migration output directly loadable by `ethrex --datadir`, enabling P2P sync resume and RPC queries on Sepolia.

**Architecture:** Three layers of changes — (1) receipt migration to fill the critical data gap, (2) finalized/safe block metadata, (3) enhanced verification and ethrex-ready smoke test. All changes are in `tooling/geth-db-migrate/` except reading ethrex Store types.

**Tech Stack:** Rust, ethrex-rlp, ethrex-storage Store API, ethrex-common types (Receipt, TxType, Log)

**Design Doc:** `docs/plans/2026-03-02-g2r-ethrex-ready-design.md`

---

## Pre-Implementation Notes

### Corrected Gap Analysis (from code inspection)

| Item | Originally Thought | Actual Status |
|------|-------------------|---------------|
| `metadata.json` | Missing | Already created by `Store::new_from_genesis()` |
| Transaction Locations | Unclear | Already written by `add_blocks()` (store.rs:284-292) |
| **Receipts** | Unclear | **NOT written** — `add_blocks()` does not call `add_receipts()` |
| Finalized/Safe Block Number | Missing | Confirmed: `forkchoice_update()` receives `None, None` (cli.rs:1279-1280) |

### Geth Stored Receipt Format (go-ethereum core/types/receipt.go)

Geth stores receipts per-block as:
```
RLP([
  RLP([status(1byte), cumulativeGasUsed, [log1, log2, ...]]),
  RLP([status(1byte), cumulativeGasUsed, [log1, log2, ...]]),
  ...
])
```
- Each log: `RLP([address(20), [topic1, topic2, ...], data])`
- **No bloom** in stored format (reconstructed on read)
- **No tx_type** in stored receipt (must be derived from block body transactions)
- Status: `0x00` = failed, `0x01` = succeeded (post-Byzantium)

### ethrex Receipt (crates/common/types/receipt.rs:17-25)
```rust
pub struct Receipt {
    pub tx_type: TxType,     // ← must come from block body transaction
    pub succeeded: bool,
    pub cumulative_gas_used: u64,
    pub logs: Vec<Log>,
}
```

### Key Store API
```rust
// crates/storage/store.rs:628
pub async fn add_receipts(&self, block_hash: BlockHash, receipts: Vec<Receipt>) -> Result<(), StoreError>
// Key: (block_hash, index).encode_to_vec() → Value: receipt.encode_to_vec()

// crates/storage/store.rs:977 — already called by g2r
pub async fn forkchoice_update(&self, ..., safe: Option<BlockNumber>, finalized: Option<BlockNumber>)
```

---

## Task 1: Implement Geth Receipt Decoding

**Files:**
- Modify: `tooling/geth-db-migrate/src/readers/geth_db.rs:625-629` (decode_stored_receipts stub)
- Test: `tooling/geth-db-migrate/src/readers/geth_db.rs` (tests module at bottom)

### Step 1: Write the failing test for receipt decoding

Add to the `tests` module in `geth_db.rs`:

```rust
#[test]
fn decode_stored_receipts_single_legacy() {
    use ethrex_rlp::encode::RLPEncode;
    // Build a stored receipt: [status=1, cumGasUsed=21000, logs=[]]
    let mut receipt_inner = Vec::new();
    ethrex_rlp::structs::Encoder::new(&mut receipt_inner)
        .encode_field(&1u8)           // status = success
        .encode_field(&21000u64)      // cumulative gas used
        .encode_field(&Vec::<Vec<u8>>::new()) // empty logs
        .finish();

    // Wrap in outer list
    let mut encoded = Vec::new();
    ethrex_rlp::structs::Encoder::new(&mut encoded)
        .encode_field(&vec![receipt_inner.clone()])
        .finish();

    // This should fail because decode_stored_receipts is a stub
    let receipts = decode_stored_receipts(&encoded).unwrap();
    assert_eq!(receipts.len(), 1);
    assert!(receipts[0].succeeded);
    assert_eq!(receipts[0].cumulative_gas_used, 21000);
    assert!(receipts[0].logs.is_empty());
}
```

### Step 2: Run test to verify it fails

Run: `cargo test -p geth-db-migrate decode_stored_receipts_single_legacy`
Expected: FAIL — `assert_eq!(receipts.len(), 1)` fails because stub returns empty Vec

### Step 3: Implement decode_stored_receipts

Replace the TODO stub at `geth_db.rs:625-629` with a real implementation using `ethrex_rlp::decode::RLPDecode`:

```rust
/// Decodes stored receipts from Geth's RLP format.
///
/// Geth stores receipts per-block as `RLP([receipt1, receipt2, ...])` where each
/// receipt is `RLP([status(u8), cumulative_gas_used(u64), [logs]])`.
/// Bloom is NOT stored. tx_type is NOT stored (derive from block body).
pub fn decode_stored_receipts(
    raw: &[u8],
) -> Result<Vec<StoredReceipt>, Box<dyn std::error::Error>> {
    use ethrex_rlp::decode::RLPDecode;

    // Outer RLP list: [receipt1_bytes, receipt2_bytes, ...]
    let receipt_list: Vec<Vec<u8>> = Vec::<Vec<u8>>::decode(raw)
        .map_err(|e| format!("Failed to decode receipt list: {e:?}"))?;

    let mut receipts = Vec::with_capacity(receipt_list.len());
    for (i, receipt_bytes) in receipt_list.iter().enumerate() {
        let decoder = ethrex_rlp::structs::Decoder::new(receipt_bytes)
            .map_err(|e| format!("Receipt #{i} decoder init: {e:?}"))?;

        let (status, decoder): (u8, _) = decoder
            .decode_field("status")
            .map_err(|e| format!("Receipt #{i} status: {e:?}"))?;
        let (cumulative_gas_used, decoder): (u64, _) = decoder
            .decode_field("cumulative_gas_used")
            .map_err(|e| format!("Receipt #{i} cumulative_gas_used: {e:?}"))?;
        let (logs, decoder): (Vec<Log>, _) = decoder
            .decode_field("logs")
            .map_err(|e| format!("Receipt #{i} logs: {e:?}"))?;
        let _ = decoder
            .finish()
            .map_err(|e| format!("Receipt #{i} trailing data: {e:?}"))?;

        receipts.push(StoredReceipt {
            succeeded: status == 1,
            cumulative_gas_used,
            logs,
        });
    }

    Ok(receipts)
}
```

### Step 4: Run test to verify it passes

Run: `cargo test -p geth-db-migrate decode_stored_receipts_single_legacy`
Expected: PASS

### Step 5: Add more receipt test cases

```rust
#[test]
fn decode_stored_receipts_with_logs() {
    use bytes::Bytes;
    use ethereum_types::Address;
    use ethrex_rlp::encode::RLPEncode;

    let log = Log {
        address: Address::from_low_u64_be(0x42),
        topics: vec![H256::from_low_u64_be(0xdead)],
        data: Bytes::from_static(b"hello"),
    };

    // Build receipt: [status=0, cumGasUsed=50000, [log]]
    let mut receipt_inner = Vec::new();
    ethrex_rlp::structs::Encoder::new(&mut receipt_inner)
        .encode_field(&0u8)
        .encode_field(&50000u64)
        .encode_field(&vec![log.clone()])
        .finish();

    let mut encoded = Vec::new();
    ethrex_rlp::structs::Encoder::new(&mut encoded)
        .encode_field(&vec![receipt_inner])
        .finish();

    let receipts = decode_stored_receipts(&encoded).unwrap();
    assert_eq!(receipts.len(), 1);
    assert!(!receipts[0].succeeded);
    assert_eq!(receipts[0].cumulative_gas_used, 50000);
    assert_eq!(receipts[0].logs.len(), 1);
    assert_eq!(receipts[0].logs[0].address, Address::from_low_u64_be(0x42));
}

#[test]
fn decode_stored_receipts_multiple() {
    use ethrex_rlp::encode::RLPEncode;

    let mut r1 = Vec::new();
    ethrex_rlp::structs::Encoder::new(&mut r1)
        .encode_field(&1u8).encode_field(&21000u64).encode_field(&Vec::<Log>::new()).finish();
    let mut r2 = Vec::new();
    ethrex_rlp::structs::Encoder::new(&mut r2)
        .encode_field(&1u8).encode_field(&42000u64).encode_field(&Vec::<Log>::new()).finish();

    let mut encoded = Vec::new();
    ethrex_rlp::structs::Encoder::new(&mut encoded)
        .encode_field(&vec![r1, r2])
        .finish();

    let receipts = decode_stored_receipts(&encoded).unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0].cumulative_gas_used, 21000);
    assert_eq!(receipts[1].cumulative_gas_used, 42000);
}

#[test]
fn decode_stored_receipts_empty_block() {
    use ethrex_rlp::encode::RLPEncode;
    // Empty receipt list
    let encoded = Vec::<Vec<u8>>::new().encode_to_vec();
    let receipts = decode_stored_receipts(&encoded).unwrap();
    assert!(receipts.is_empty());
}
```

### Step 6: Run all receipt tests

Run: `cargo test -p geth-db-migrate decode_stored_receipts`
Expected: All PASS

### Step 7: Add `read_receipts` method to GethBlockReader

Add to `GethBlockReader` impl block (after `read_raw_receipts` at line 512):

```rust
/// Reads and decodes receipts for a block, pairing each with its tx_type.
///
/// Geth stored receipts do NOT include `tx_type`, so we derive it from
/// the corresponding transaction in the block body. The caller must
/// provide the block body (already read during migration).
pub fn read_receipts(
    &self,
    number: u64,
    hash: H256,
    body: &BlockBody,
) -> Result<Option<Vec<ethrex_common::types::Receipt>>, Box<dyn std::error::Error>> {
    let raw = match self.read_raw_receipts(number, hash)? {
        Some(r) => r,
        None => return Ok(None),
    };

    let stored = decode_stored_receipts(&raw)?;

    if stored.len() != body.transactions.len() {
        return Err(format!(
            "Receipt count mismatch for block #{number}: {} receipts vs {} txs",
            stored.len(),
            body.transactions.len()
        ).into());
    }

    let receipts = stored
        .into_iter()
        .zip(body.transactions.iter())
        .map(|(sr, tx)| ethrex_common::types::Receipt {
            tx_type: tx.tx_type(),
            succeeded: sr.succeeded,
            cumulative_gas_used: sr.cumulative_gas_used,
            logs: sr.logs,
        })
        .collect();

    Ok(Some(receipts))
}
```

### Step 8: Commit

```bash
git add tooling/geth-db-migrate/src/readers/geth_db.rs
git commit -m "feat(geth-db-migrate): implement Geth stored receipt decoding and reading"
```

---

## Task 2: Add Receipt Writing to g2r Migration

**Files:**
- Modify: `tooling/geth-db-migrate/src/cli.rs:1240-1315` (block migration loop)

### Step 1: Read and store receipts alongside blocks

In `cli.rs`, inside the block migration loop (around line 1240-1310), after each block is read and before the batch is written, collect receipts too:

```rust
// After line 1244 (batch.push(block)), add receipt collection:
// Read receipts for this block
if let Some(receipts) = geth_reader
    .read_receipts(block_number, block_hash, &block.body)
    .map_err(|e| eyre::eyre!("Cannot read receipts for block #{block_number}: {e}"))?
{
    batch_receipts.push((block_hash, receipts));
}
```

Initialize `batch_receipts` before the inner loop (alongside `batch` and `batch_canonical`):
```rust
let mut batch_receipts: Vec<(H256, Vec<ethrex_common::types::Receipt>)> = Vec::new();
```

### Step 2: Write receipts after add_blocks succeeds

After the `add_blocks()` retry block (around line 1265), add receipt writing:

```rust
// Write receipts for this batch
for (block_hash, receipts) in &batch_receipts {
    new_store
        .add_receipts(*block_hash, receipts.clone())
        .await
        .wrap_err_with(|| format!("Cannot write receipts for block {block_hash:?}"))?;
}
```

### Step 3: Clear batch_receipts at batch boundary

At the batch boundary reset (alongside `batch.clear()` and `batch_canonical.clear()`):
```rust
batch_receipts.clear();
```

### Step 4: Add import for Receipt type

At the top of `cli.rs`, ensure `ethrex_common::types::Receipt` is importable (likely already accessible via `Block` import).

### Step 5: Build and verify compilation

Run: `cargo build -p geth-db-migrate`
Expected: Compiles without errors

### Step 6: Commit

```bash
git add tooling/geth-db-migrate/src/cli.rs
git commit -m "feat(geth-db-migrate): add receipt migration to g2r block import loop"
```

---

## Task 3: Set Finalized/Safe Block Number

**Files:**
- Modify: `tooling/geth-db-migrate/src/cli.rs:1272-1289` (forkchoice_update call)

### Step 1: Change forkchoice_update to pass safe/finalized

At `cli.rs:1275-1281`, change the `forkchoice_update()` call:

```rust
// Before (line 1279-1280):
//  None,
//  None,

// After:
Some(last_num),
Some(last_num),
```

This sets both `SafeBlockNumber` and `FinalizedBlockNumber` to the last block in each batch. The final batch will set them to the migration head.

### Step 2: Build and verify

Run: `cargo build -p geth-db-migrate`
Expected: Compiles

### Step 3: Commit

```bash
git add tooling/geth-db-migrate/src/cli.rs
git commit -m "fix(geth-db-migrate): set finalized and safe block numbers during g2r migration"
```

---

## Task 4: Enhance Verification — Body Check

**Files:**
- Modify: `tooling/geth-db-migrate/src/cli.rs:612-766` (verify_geth_to_rocksdb_offline function)

### Step 1: Add body hash verification to existing verification loop

Inside the verification loop (after the state root check at line 702), add body comparison:

```rust
// After state root check, add body verification:
let geth_body = geth_reader
    .read_block_body(block_number, geth_hash)
    .map_err(|e| eyre::eyre!("verify #{block_number}: cannot read geth body: {e}"))?;
let ethrex_body = store
    .get_block_body(block_number)
    .await?;

match (geth_body, ethrex_body) {
    (Some(gb), Some(eb)) => {
        if gb.transactions.len() != eb.transactions.len() {
            mismatches += 1;
            block_mismatch = true;
            #[cfg(feature = "tui")]
            if let Some(tx) = tui_tx {
                let _ = tx.try_send(crate::tui::event::ProgressEvent::VerificationMismatch {
                    block_number,
                    reason: format!(
                        "tx count mismatch geth={} ethrex={}",
                        gb.transactions.len(),
                        eb.transactions.len()
                    ),
                });
            }
        }
    }
    (Some(_), None) => {
        mismatches += 1;
        block_mismatch = true;
    }
    _ => {} // geth body missing = ancient not available, skip
}
```

### Step 2: Update OfflineVerificationSummary

Add a `body_checks_passed` field to `OfflineVerificationSummary` (line 576-581):

```rust
struct OfflineVerificationSummary {
    start_block: u64,
    end_block: u64,
    checked_blocks: u64,
    mismatches: u64,
    body_checks_passed: u64,
}
```

Track `body_checks_passed` in the loop and include in the summary.

### Step 3: Build and run existing tests

Run: `cargo build -p geth-db-migrate && cargo test -p geth-db-migrate`
Expected: Compiles, all unit tests pass

### Step 4: Commit

```bash
git add tooling/geth-db-migrate/src/cli.rs
git commit -m "feat(geth-db-migrate): add body verification to offline verification"
```

---

## Task 5: Add `--verify-deep` Flag

**Files:**
- Modify: `tooling/geth-db-migrate/src/cli.rs` (CLI struct + verification function)

### Step 1: Add --verify-deep CLI argument

In the `Geth2Rocksdb` variant (around line 76), add:

```rust
#[arg(long = "verify-deep", default_value_t = false)]
/// Run deep verification: receipts root, code hash sampling, tx location checks
verify_deep: bool,
```

### Step 2: Pass verify_deep to verification function

Update `verify_geth_to_rocksdb_offline` signature to accept `verify_deep: bool`.

### Step 3: Add receipt root verification (when verify_deep=true)

Inside the verification loop, when `verify_deep`:

```rust
if verify_deep {
    // Check receipts exist for blocks with transactions
    if let Some(ethrex_body) = &ethrex_body {
        if !ethrex_body.transactions.is_empty() {
            let mut has_all_receipts = true;
            for idx in 0..ethrex_body.transactions.len() {
                if store.get_receipt(block_number, idx as u64).await?.is_none() {
                    has_all_receipts = false;
                    break;
                }
            }
            if !has_all_receipts {
                mismatches += 1;
                block_mismatch = true;
                // TUI event...
            }
        }
    }
}
```

### Step 4: Build, test, commit

Run: `cargo build -p geth-db-migrate && cargo test -p geth-db-migrate`

```bash
git add tooling/geth-db-migrate/src/cli.rs
git commit -m "feat(geth-db-migrate): add --verify-deep flag for receipt and code verification"
```

---

## Task 6: Add ethrex-Ready Smoke Test

**Files:**
- Modify: `tooling/geth-db-migrate/src/cli.rs` (new function + CLI flag + integration)

### Step 1: Add --ethrex-ready / --no-ethrex-ready CLI flags

In the `Geth2Rocksdb` variant:

```rust
#[arg(long = "ethrex-ready", default_value_t = true, action = clap::ArgAction::Set)]
/// Run ethrex startup compatibility check after migration
ethrex_ready: bool,
```

### Step 2: Implement ethrex_ready_check function

```rust
#[derive(Debug, Serialize)]
struct EthrexReadyReport {
    ethrex_ready: bool,
    checks: EthrexReadyChecks,
}

#[derive(Debug, Serialize)]
struct EthrexReadyChecks {
    metadata_json: String,
    latest_block_number: String,
    latest_header: String,
    chain_config: String,
    genesis_block: String,
    state_root_valid: String,
}

async fn check_ethrex_ready(
    store: &ethrex_storage::Store,
    target_path: &Path,
) -> EthrexReadyReport {
    let mut all_pass = true;
    let mut checks = EthrexReadyChecks {
        metadata_json: String::new(),
        latest_block_number: String::new(),
        latest_header: String::new(),
        chain_config: String::new(),
        genesis_block: String::new(),
        state_root_valid: String::new(),
    };

    // 1. metadata.json
    let metadata_path = target_path.join("metadata.json");
    if metadata_path.exists() {
        checks.metadata_json = "pass".into();
    } else {
        checks.metadata_json = "FAIL: metadata.json not found".into();
        all_pass = false;
    }

    // 2. LatestBlockNumber
    match store.get_latest_block_number().await {
        Ok(Some(n)) => checks.latest_block_number = format!("pass (block {n})"),
        Ok(None) => {
            checks.latest_block_number = "FAIL: LatestBlockNumber not set".into();
            all_pass = false;
        }
        Err(e) => {
            checks.latest_block_number = format!("FAIL: {e}");
            all_pass = false;
        }
    }

    // 3. Latest header loadable
    match store.get_block_header_by_latest() {
        Ok(Some(_)) => checks.latest_header = "pass".into(),
        Ok(None) => {
            checks.latest_header = "FAIL: latest header not found in HEADERS table".into();
            all_pass = false;
        }
        Err(e) => {
            checks.latest_header = format!("FAIL: {e}");
            all_pass = false;
        }
    }

    // 4. ChainConfig parseable
    let config = store.get_chain_config();
    checks.chain_config = format!("pass (chain_id={})", config.chain_id);

    // 5. Genesis block exists
    match store.get_canonical_block_hash(0).await {
        Ok(Some(genesis_hash)) => {
            let header_ok = store.get_block_header(0).map(|h| h.is_some()).unwrap_or(false);
            let body_ok = store.get_block_body(0).await.map(|b| b.is_some()).unwrap_or(false);
            if header_ok && body_ok {
                checks.genesis_block = format!("pass ({genesis_hash:?})");
            } else {
                checks.genesis_block = "FAIL: genesis hash exists but header/body missing".into();
                all_pass = false;
            }
        }
        _ => {
            checks.genesis_block = "FAIL: no canonical hash for block 0".into();
            all_pass = false;
        }
    }

    // 6. State root valid for latest block
    match store.get_block_header_by_latest() {
        Ok(Some(header)) => {
            match store.has_state_root(header.state_root) {
                Ok(true) => checks.state_root_valid = "pass".into(),
                Ok(false) => {
                    checks.state_root_valid =
                        format!("FAIL: state root {:?} not found in trie", header.state_root);
                    all_pass = false;
                }
                Err(e) => {
                    checks.state_root_valid = format!("FAIL: {e}");
                    all_pass = false;
                }
            }
        }
        _ => {
            checks.state_root_valid = "SKIP: no latest header".into();
        }
    }

    EthrexReadyReport {
        ethrex_ready: all_pass,
        checks,
    }
}
```

### Step 3: Integrate into g2r execution flow

After verification (around line 1374), add:

```rust
if ethrex_ready {
    let report = check_ethrex_ready(&new_store, &target_storage).await;
    if json {
        let json_str = serde_json::to_string_pretty(&report).unwrap_or_default();
        eprintln!("{json_str}");
    } else {
        eprintln!("[ethrex-ready] {}", if report.ethrex_ready { "PASS" } else { "FAIL" });
        // Print individual checks
    }
    if !report.ethrex_ready {
        return Err(eyre::eyre!("ethrex-ready check failed"));
    }
}
```

### Step 4: Build, test, commit

Run: `cargo build -p geth-db-migrate && cargo test -p geth-db-migrate`

```bash
git add tooling/geth-db-migrate/src/cli.rs
git commit -m "feat(geth-db-migrate): add ethrex-ready startup compatibility check"
```

---

## Task 7: Update Design Doc with Corrected Findings

**Files:**
- Modify: `docs/plans/2026-03-02-g2r-ethrex-ready-design.md`

### Step 1: Update the gap table

Fix metadata.json and tx locations entries to reflect "already handled" status.

### Step 2: Commit

```bash
git add docs/plans/2026-03-02-g2r-ethrex-ready-design.md
git commit -m "docs(geth-db-migrate): update design doc with corrected gap analysis"
```

---

## Task 8: Run Full Test Suite and Verify

### Step 1: Run all unit tests

Run: `cargo test -p geth-db-migrate`
Expected: All tests pass (existing 42+ new receipt tests)

### Step 2: Build release binary

Run: `cargo build --release --manifest-path tooling/geth-db-migrate/Cargo.toml`
Expected: Builds successfully

### Step 3: Final commit (if any fixups needed)

---

## Dependency Order

```
Task 1 (receipt decode) → Task 2 (receipt write in g2r)
Task 3 (finalized/safe) — independent
Task 4 (body verify) — independent
Task 5 (--verify-deep) → depends on Task 2 (receipts available)
Task 6 (ethrex-ready) — independent (but best done after Tasks 2-3)
Task 7 (doc update) — independent
Task 8 (final verify) → depends on all above
```

**Parallelizable:** Tasks 3, 4, 7 can run in parallel with Task 1→2.
