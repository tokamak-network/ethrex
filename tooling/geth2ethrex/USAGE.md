# geth2ethrex Usage Guide

## Overview

`geth2ethrex` is a migration tool that converts Geth chaindata (LevelDB or Pebble format) to ethrex's RocksDB-based storage format. This enables seamless migration from Geth to ethrex without re-syncing the entire blockchain.

## Features

- **Automatic DB Detection**: Identifies Pebble or LevelDB format automatically
- **Block-by-block Migration**: Preserves chain integrity with canonical hash verification
- **Retry Policy**: Handles transient I/O errors with configurable retry attempts
- **Dry-run Mode**: Validates migration without writing data
- **JSON Output**: Machine-readable progress and error reporting

## Prerequisites

- **Rust**: 1.82.0 or later
- **Geth Chaindata**: A synced Geth datadir (Pebble or LevelDB)
- **Genesis File**: ethrex-compatible genesis JSON
- **Disk Space**: At least 1.5x the source chaindata size

## Installation

### Build from Source

```bash
cd tooling/geth2ethrex
cargo build --release
```

The binary will be available at `../../target/release/geth2ethrex`.

## Usage

### Basic Command

```bash
geth2ethrex geth2rocksdb \
  --source /path/to/geth/chaindata \
  --target /path/to/ethrex/storage \
  --genesis /path/to/genesis.json
```

### Dry-run Mode (Recommended First)

```bash
geth2ethrex geth2rocksdb \
  --source /path/to/geth/chaindata \
  --target /path/to/ethrex/storage \
  --genesis /path/to/genesis.json \
  --dry-run
```

### JSON Output

```bash
geth2ethrex geth2rocksdb \
  --source /path/to/geth/chaindata \
  --target /path/to/ethrex/storage \
  --genesis /path/to/genesis.json \
  --json > migration.json
```

## Command Options

### Required Arguments

- `--source <GETH_CHAINDATA>`: Path to Geth's chaindata directory
- `--target <TARGET_STORAGE>`: Path where ethrex RocksDB will be created
- `--genesis <GENESIS_PATH>`: Path to genesis JSON file

### Optional Flags

- `--dry-run`: Validate without writing data
- `--json`: Output progress in JSON format
- `--continue-on-error`: Continue migration even if some blocks fail
- `--retry-attempts <N>`: Maximum retry attempts for I/O operations (default: 3)
- `--retry-base-delay-ms <MS>`: Initial retry backoff delay in milliseconds (default: 1000)

### Aliases

- `geth2rocksdb` (alias: `g2r`)

## Genesis File Format

The genesis file must include these fields:

```json
{
  "config": {
    "chainId": 11155111,
    "homesteadBlock": 0,
    "eip150Block": 0,
    "eip155Block": 0,
    "eip158Block": 0,
    "byzantiumBlock": 0,
    "constantinopleBlock": 0,
    "petersburgBlock": 0,
    "istanbulBlock": 0,
    "berlinBlock": 0,
    "londonBlock": 0,
    "terminalTotalDifficulty": 17000000000000000,
    "terminalTotalDifficultyPassed": true,
    "shanghaiTime": 1677557088,
    "cancunTime": 1706655072,
    "depositContractAddress": "0x7f02C3E3c98b133055B8B348B2Ac625669Ed295D"
  },
  "nonce": "0x0",
  "timestamp": "0x6159af19",
  "extraData": "0x...",
  "gasLimit": "0x1c9c380",
  "difficulty": "0x20000",
  "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
  "coinbase": "0x0000000000000000000000000000000000000000",
  "alloc": {},
  "number": "0x0",
  "gasUsed": "0x0",
  "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000"
}
```

## Examples

### Migrate Sepolia Testnet

```bash
# 1. Dry-run to validate
geth2ethrex g2r \
  --source ~/.ethereum/sepolia/geth/chaindata \
  --target ~/ethrex-data/sepolia \
  --genesis sepolia-genesis.json \
  --dry-run

# 2. Actual migration with JSON output
geth2ethrex g2r \
  --source ~/.ethereum/sepolia/geth/chaindata \
  --target ~/ethrex-data/sepolia \
  --genesis sepolia-genesis.json \
  --json | tee migration-log.json
```

### Migrate Mainnet

```bash
geth2ethrex g2r \
  --source ~/.ethereum/mainnet/geth/chaindata \
  --target ~/ethrex-data/mainnet \
  --genesis mainnet-genesis.json \
  --retry-attempts 5 \
  --retry-base-delay-ms 2000
```

### Continue on Errors (Advanced)

```bash
# Useful for partial migrations or debugging
geth2ethrex g2r \
  --source /data/geth/chaindata \
  --target /data/ethrex \
  --genesis genesis.json \
  --continue-on-error \
  --json > partial-migration.json
```

## Important Notes

### Before Migration

1. **Stop Geth**: Ensure Geth is not running to prevent data corruption
2. **Backup**: Create a backup of your Geth chaindata
3. **Disk Space**: Verify sufficient space (1.5x source size recommended)
4. **Dry-run First**: Always run with `--dry-run` to validate

### During Migration

- Migration time depends on chaindata size (estimate: 1-2 hours per 100GB)
- CPU and I/O intensive process
- Use `--json` for progress monitoring
- Safe to interrupt with Ctrl+C (no partial writes)

### After Migration

- Verify migration with ethrex's validation tools
- Check target RocksDB integrity
- Test block retrieval and state access
- Keep original Geth chaindata until verification complete

## Supported Database Formats

| Format   | Read Support | Notes                                    |
|----------|--------------|------------------------------------------|
| Pebble   | ✅ Full      | Via RocksDB crate (SST format compatible) |
| LevelDB  | 🚧 Planned   | Requires leveldb crate integration       |

## Troubleshooting

### Error: "Detected Geth database type: Unknown"

**Cause**: Source directory is not a valid Geth chaindata.

**Solution**:
- Verify path points to `geth/chaindata` directory
- Check for `CURRENT` and `MANIFEST-*` files
- Ensure Geth has synced at least genesis block

### Error: "Cannot determine source (geth) head block"

**Cause**: Chaindata has no blocks (genesis-only state).

**Solution**:
- Sync at least a few blocks with Geth before migration
- For post-merge chains, ensure beacon client connection

### Error: "Failed to deserialize genesis file"

**Cause**: Missing required fields in genesis JSON.

**Solution**:
- Add `depositContractAddress` to `config` section
- Verify all EIP activation blocks/timestamps present
- Use ethrex genesis format (not Geth format)

### Error: "Cannot create/open rocksdb store"

**Cause**: Target directory permissions or existing data.

**Solutions**:
- Check write permissions on target directory
- Remove existing RocksDB data at target path
- Ensure target is not mounted read-only

### Migration Hangs or Slow

**Causes**:
- Large chaindata size
- I/O bottleneck (HDD vs SSD)
- Insufficient memory

**Solutions**:
- Use SSD for both source and target
- Close other I/O-intensive applications
- Monitor with `--json` output
- Consider `--retry-attempts 1` to fail fast on errors

## Performance Tips

1. **Use SSD**: 10-50x faster than HDD
2. **Dry-run First**: Catches errors early without writing data
3. **JSON Logging**: Redirect to file for post-mortem analysis
4. **Dedicated Server**: Avoid running on production nodes
5. **Network Storage**: Avoid NFS/CIFS for target path

## Output Format

### JSON Output Schema

```json
{
  "status": "success",
  "source_db_type": "Pebble",
  "source_path": "/path/to/geth/chaindata",
  "target_path": "/path/to/ethrex/storage",
  "blocks_migrated": 12345,
  "duration_ms": 3600000,
  "timestamp": "2026-02-25T15:00:00Z"
}
```

### Error Output

```json
{
  "status": "error",
  "error": "Migration error description",
  "retry_attempts_used": 3,
  "max_attempts": 3,
  "duration_ms": 1500
}
```

## See Also

- [README.md](README.md) - Project overview and architecture
- [docs/geth-db-compatibility.md](../../docs/geth-db-compatibility.md) - Database format details
- [ethrex documentation](https://github.com/lambdaclass/ethrex) - Main ethrex project

## Contributing

Found a bug or want to improve this guide? Please open an issue or PR on the ethrex repository.

## License

This tool is part of the ethrex project and follows the same license (MIT/Apache-2.0).
