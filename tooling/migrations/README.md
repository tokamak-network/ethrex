# Ethrex migration tools

This tool provides a way to migrate ethrex databases created with Libmdbx to RocksDB.

## Development prerequisites

- Rust toolchain matching the workspace (`rust-toolchain.toml`)
- `libclang` available on the system for `bindgen` (required by `mdbx-sys`/`librocksdb-sys` during build/test)

Quick check:

```bash
tooling/migrations/scripts/check-prereqs.sh
```

If `cargo test --manifest-path tooling/migrations/Cargo.toml` fails with
`Unable to find libclang`, install your distro `libclang` package and/or set:

```bash
export LIBCLANG_PATH=/path/to/libclang
```

Common install examples:

```bash
# Ubuntu/Debian
sudo apt-get update && sudo apt-get install -y libclang-dev clang

# Fedora
sudo dnf install -y clang clang-devel

# Arch Linux
sudo pacman -S --needed clang
```

## Instructions

> [!IMPORTANT]
> If you are migrating a db from an ethrex L2 rollup you should also copy the `rollup_store`, `rollup_store-shm` and `rollup_store-wal` files to `<NEW_STORAGE_PATH>`.

It is recomended to backup the original database before migration even if this process does not erase the old data. To migrate a database run:

```
cargo run --release -- l2r --genesis <GENESIS_PATH> --store.old <OLD_STORAGE_PATH> --store.new <NEW_STORAGE_PATH>
```

This will output the migrated database to `<NEW_STORAGE_PATH>`.
Finally restart your ethrex node pointing `--datadir` to the path of the migrated database

## CLI Reference

```
Migrate a libmdbx database to rocksdb

Usage: migrations libmdbx2rocksdb --genesis <GENESIS_PATH> --store.old <OLD_STORAGE_PATH> --store.new <NEW_STORAGE_PATH> [--dry-run] [--json] [--report-file <REPORT_FILE>] [--retry-attempts <RETRY_ATTEMPTS>] [--retry-base-delay-ms <RETRY_BASE_DELAY_MS>] [--continue-on-error] [--resume-from-block <RESUME_FROM_BLOCK>] [--checkpoint-file <CHECKPOINT_FILE>] [--resume-from-checkpoint <RESUME_FROM_CHECKPOINT>]

Options:
      --genesis <GENESIS_PATH>                      Path to the genesis file for the genesis block of store.old
      --store.old <OLD_STORAGE_PATH>                Path to the target database to migrate
      --store.new <NEW_STORAGE_PATH>                Path to use for the migrated database
      --dry-run                                     Validate source/target stores and print migration plan without writing blocks
      --json                                        Emit machine-readable JSON output
      --report-file <REPORT_FILE>                   Optional path to append emitted reports (JSON lines in --json mode)
      --retry-attempts <RETRY_ATTEMPTS>             Retry budget for retryable operations (1-10, inclusive) [default: 3]
      --retry-base-delay-ms <RETRY_BASE_DELAY_MS>   Initial retry backoff delay in milliseconds (0-60000) [default: 1000]
      --continue-on-error                           Continue migrating subsequent blocks when a block-level import fails
      --resume-from-block <RESUME_FROM_BLOCK>       Force migration start block (must be > current target head and <= source head)
      --checkpoint-file <CHECKPOINT_FILE>           Optional path to write migration checkpoint metadata after successful completion
      --resume-from-checkpoint <RESUME_FROM_CHECKPOINT>
                                                    Optional path to a checkpoint file whose target_head is used as migration start
  -h, --help                                        Print help
```

`--dry-run` can be used in automation to verify source and target DB readability and to preview how many blocks would be imported before doing a real migration run.

`--retry-attempts` and `--retry-base-delay-ms` tune retry policy for retryable operations.
Retry handling is applied during source LibMDBX store open, source state reads, source/target store head discovery, target RocksDB store open/creation, per-block header fetches, per-block imports, and final forkchoice update.

`--continue-on-error` enables degraded execution for block-level failures during migration execution: failed block header reads/imports are skipped with warnings, and migration proceeds using successfully imported blocks.

`--resume-from-block` overrides the computed start block and is validated against discovered heads (`target_head < resume_from_block <= source_head`). This is useful for operator-driven resume after a partial migration.

`--resume-from-checkpoint` derives resume start from a checkpoint file (`resume_start = checkpoint.target_head + 1`) and applies the same head validation. `--resume-from-block` and `--resume-from-checkpoint` are mutually exclusive.

`--checkpoint-file` writes a JSON checkpoint after successful completion with migration head/volume metadata (`source_head`, `target_head`, `imported_blocks`, `skipped_blocks`, retry counters, elapsed time).

`--json` prints a structured migration report (`status`, `phase`, source/target heads, plan, dry-run flag, imported blocks, elapsed runtime) suitable for scripting and CI logs.
When execution fails with `--json`, the CLI emits a structured failure object including `error_type` and `retryable` for automation parsing.
`--report-file` appends emitted reports to a file (JSONL in `--json` mode; human-readable lines otherwise), including failure reports.
Parent directories are created automatically when needed, and each emitted report is written as a single appended line.

## JSON output contract (stable)

Success/progress shape:

```json
{
  "schema_version": 1,
  "status": "planned|in_progress|completed|up_to_date",
  "phase": "planning|execution",
  "source_head": 42,
  "target_head": 40,
  "plan": {
    "start_block": 41,
    "end_block": 42
  },
  "dry_run": true,
  "imported_blocks": 0,
  "skipped_blocks": 0,
  "elapsed_ms": 15,
  "retry_attempts": 3,
  "retries_performed": 0
}
```

Notes:
- `schema_version` is the JSON contract version for downstream compatibility handling.
- `phase` is `planning` for `planned`/`up_to_date` and `execution` for `in_progress`/`completed`.
- `plan` is `null` only when `status = "up_to_date"`.
- `imported_blocks` is `0` for `planned`, `in_progress`, and `up_to_date`.
- `imported_blocks > 0` only for `completed` runs.
- `skipped_blocks` counts block-level failures skipped via `--continue-on-error` (always `0` when not enabled).
- `elapsed_ms` is the runtime elapsed at the moment the report is emitted.
- `retry_attempts` is the configured max attempts for retryable operations.
- `retries_performed` is the number of retries actually used in successful/planned runs.
- Failure reports include `retry_attempts` (policy budget).
- `retry_attempts_used` is populated when a retry-managed operation exhausts attempts; otherwise it is `null` for direct/non-retried failures.
- `error_classification` explains how retryability was derived (`retry_failure`, `io_kind`, `message_marker`, `default_fatal`).
- For `retry_failure`, the underlying error text includes attempt metadata (`retry_attempts_used`, `max_attempts`) for debugging.

### Retryability policy notes

The current IO classification policy treats these as **transient** (retryable), and this classification is used directly by both async and sync retry paths:
- `WouldBlock`
- `TimedOut`
- `Interrupted`
- `OutOfMemory`
- `ConnectionReset`
- `ConnectionAborted`
- `NotConnected`
- `BrokenPipe`

Other `std::io::ErrorKind` values are treated as **fatal** by default (no retries; e.g. `PermissionDenied`).

Failure shape:

```json
{
  "schema_version": 1,
  "status": "failed",
  "phase": "execution",
  "error_type": "transient|fatal",
  "error_classification": "retry_failure|io_kind|message_marker|default_fatal",
  "retryable": true,
  "retry_attempts": 3,
  "retry_attempts_used": 2,
  "error": "human-readable error with context",
  "elapsed_ms": 27
}
```
