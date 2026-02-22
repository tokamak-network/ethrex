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

Usage: migrations libmdbx2rocksdb --genesis <GENESIS_PATH> --store.old <OLD_STORAGE_PATH> --store.new <NEW_STORAGE_PATH> [--dry-run] [--json]

Options:
      --genesis <GENESIS_PATH>        Path to the genesis file for the genesis block of store.old
      --store.old <OLD_STORAGE_PATH>  Path to the target database to migrate
      --store.new <NEW_STORAGE_PATH>  Path to use for the migrated database
      --dry-run                       Validate source/target stores and print migration plan without writing blocks
      --json                          Emit machine-readable JSON output
  -h, --help                          Print help
```

`--dry-run` can be used in automation to verify source and target DB readability and to preview how many blocks would be imported before doing a real migration run.

`--json` prints a structured migration report (`status`, source/target heads, plan, dry-run flag, imported blocks) suitable for scripting and CI logs.
