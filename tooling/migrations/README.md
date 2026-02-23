# Ethrex migration tools

This tool provides a way to migrate ethrex databases created with Libmdbx to RocksDB.

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

Usage: migrations libmdbx2rocksdb --genesis <GENESIS_PATH> --store.old <OLD_STORAGE_PATH> --store.new <NEW_STORAGE_PATH> [--dry-run] [--json] [--retry-attempts <RETRY_ATTEMPTS>] [--retry-base-delay-ms <RETRY_BASE_DELAY_MS>]

Options:
      --genesis <GENESIS_PATH>                      Path to the genesis file for the genesis block of store.old
      --store.old <OLD_STORAGE_PATH>                Path to the target database to migrate
      --store.new <NEW_STORAGE_PATH>                Path to use for the migrated database
      --dry-run                                     Validate source/target stores and print migration plan without writing blocks
      --json                                        Emit machine-readable JSON output
      --retry-attempts <RETRY_ATTEMPTS>             Retry budget for retryable operations (1-10, inclusive) [default: 3]
      --retry-base-delay-ms <RETRY_BASE_DELAY_MS>   Initial retry backoff delay in milliseconds (0-60000) [default: 1000]
  -h, --help                                        Print help
```

`--dry-run` can be used in automation to verify source and target DB readability and to preview how many blocks would be imported before doing a real migration run.

`--retry-attempts` and `--retry-base-delay-ms` tune retry policy for retryable operations.

`--json` prints a structured migration report (`status`, `phase`, source/target heads, plan, dry-run flag, imported blocks, elapsed runtime) suitable for scripting and CI logs.
When execution fails with `--json`, the CLI emits a structured failure object including `error_type` and `retryable` for automation parsing.

## JSON output contract (stable)

Success/progress shape:

```json
{
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
  "elapsed_ms": 15,
  "retry_attempts": 3,
  "retries_performed": 0
}
```

Notes:
- `phase` is `planning` for `planned`/`up_to_date` and `execution` for `in_progress`/`completed`.
- `plan` is `null` only when `status = "up_to_date"`.
- `imported_blocks` is `0` for `planned`, `in_progress`, and `up_to_date`.
- `imported_blocks > 0` only for `completed` runs.
- `elapsed_ms` is the runtime elapsed at the moment the report is emitted.
- `retry_attempts` is the configured max attempts for retryable operations.
- `retries_performed` is the number of retries actually used in successful/planned runs.
- Failure reports include `retry_attempts` (policy budget).
- `retry_attempts_used` is populated when a retry-managed operation exhausts attempts; otherwise it is `null` for direct/non-retried failures.
- `error_classification` explains how retryability was derived (`retry_failure`, `io_kind`, `message_marker`, `default_fatal`).
- For `retry_failure`, the underlying error text includes attempt metadata (`retry_attempts_used`, `max_attempts`) for debugging.

Failure shape:

```json
{
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
