# geth-db-migrate

English documentation for `geth-db-migrate`.

- Korean guide: [README_kor.md](./README_kor.md)

`geth-db-migrate` migrates Geth chaindata into other execution-client database formats.

## Supported Paths

| Command | Source | Target | Status |
|---|---|---|---|
| `g2r` (`to-rocksdb`) | Geth Pebble | ethrex RocksDB | Blocks + State + Verification |
| `g2l` (`to-lmdb`) | Geth Pebble | py-ethclient LMDB | Blocks + State + Verification |

> LevelDB can be detected but full read support is currently limited.

## Quick Start

### g2r: Geth -> ethrex RocksDB

```bash
cargo build --release --manifest-path tooling/geth-db-migrate/Cargo.toml

geth-db-migrate g2r \
  --source /path/to/geth/chaindata \
  --target /path/to/ethrex/storage \
  --genesis genesis.json \
  --dry-run

geth-db-migrate g2r \
  --source /path/to/geth/chaindata \
  --target /path/to/ethrex/storage \
  --genesis genesis.json
```

### g2l: Geth -> py-ethclient LMDB

```bash
geth-db-migrate g2l \
  --source /path/to/geth/chaindata \
  --target /path/to/lmdb/output \
  --dry-run

geth-db-migrate g2l \
  --source /path/to/geth/chaindata \
  --target /path/to/lmdb/output
```

## Migration Flow

Both commands run these phases:

1. **Block migration** (header/body/receipts in batches) - **always runs**
2. **State migration** (accounts/storage/code) - **default, runs unless `--blocks-only`**
3. **State catch-up** (g2r only, re-executes blocks to materialize head state) - **default, use `--state-catch-up false` to skip**
4. **Offline verification** (canonical hash, header hash, state root)
5. **ethrex-ready check** (g2r only, validates startup compatibility) - **default, use `--ethrex-ready false` to skip**

### Default Behavior

By default, **both block and state migration** run (`--blocks-only=false`).
For `g2r`, **state catch-up to head** is also enabled by default (`--state-catch-up=true`).

Use `--blocks-only` when you want faster block-only migration and intentionally skip account/storage/code.

## Offline Verification

After migration completes, the tool automatically validates data consistency between source (Geth) and target database. Use `--verify-offline false` to skip.

### Common Validation (g2r, g2l)

Each block is checked for:

1. **Canonical Hash Match** — Source and target must have same hash for each block number
2. **Header Hash Match** — BlockHeader RLP encoding must produce identical hash
3. **State Root Match** — Block header's state_root field must match between source and target

### Additional g2r Validation

4. **Block Body (Transaction Count)** — Transaction count per block must match
5. **Receipt Validation** (with `--verify-deep`) — For blocks with transactions, verify all receipts exist

### Customize Verification

```bash
# Verify specific block range only
geth-db-migrate g2r \
  --source /path/to/geth/chaindata \
  --target /path/to/rocksdb \
  --genesis genesis.json \
  --verify-start-block 1000000 \
  --verify-end-block 1001000

# Skip verification for speed
geth-db-migrate g2r \
  --source /path/to/geth/chaindata \
  --target /path/to/rocksdb \
  --genesis genesis.json \
  --verify-offline false
```

## TUI

TUI is enabled by default unless JSON mode is used.

- Build with default features (includes TUI)
- Run without `--json` to get the live dashboard
- Use `Ctrl+C` to stop and `q` to quit when finished

## JSON Mode

Use `--json` for machine-readable output. Add `--report-file` to append JSONL reports.

## Common Options

### g2r

```text
--source --target --genesis [required]
--dry-run
--blocks-only (default: false)
--include-state (default: false, experimental)
--state-catch-up (default: true)
--from-block
--ethrex-ready (default: true)
--verify-offline
--verify-start-block
--verify-end-block
--verify-deep
--json
--report-file
--retry-attempts
--retry-base-delay-ms
--continue-on-error
```

### g2l

```text
--source --target [required]
--dry-run
--blocks-only (default: false)
--include-state (default: false, experimental)
--map-size-gb
--skip-receipts
--verify-offline
--verify-start-block
--verify-end-block
--json
--report-file
--continue-on-error
```

## Build & Test

```bash
cargo build --release --manifest-path tooling/geth-db-migrate/Cargo.toml
cargo test --manifest-path tooling/geth-db-migrate/Cargo.toml
```

## Full Documentation

For the full detailed guide (currently Korean), see [README_kor.md](./README_kor.md).

