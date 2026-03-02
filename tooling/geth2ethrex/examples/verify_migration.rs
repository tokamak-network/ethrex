//! Example: verify that block canonical hashes match between Geth and ethrex after migration.
//!
//! Usage (after running `g2r`):
//! ```bash
//! cargo run --example verify_migration -- \
//!   --source /path/to/geth/chaindata \
//!   --target /path/to/ethrex/storage \
//!   --genesis /path/to/genesis.json
//! ```

use ethrex_storage::{EngineType, Store};
use geth2ethrex::readers::open_geth_block_reader;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut source = String::new();
    let mut target = String::new();
    let mut genesis = String::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--source" => {
                i += 1;
                source = args[i].clone();
            }
            "--target" => {
                i += 1;
                target = args[i].clone();
            }
            "--genesis" => {
                i += 1;
                genesis = args[i].clone();
            }
            _ => {}
        }
        i += 1;
    }

    if source.is_empty() || target.is_empty() || genesis.is_empty() {
        eprintln!(
            "Usage: verify_migration --source <chaindata> --target <ethrex_db> --genesis <genesis.json>"
        );
        std::process::exit(1);
    }

    let chaindata = Path::new(&source);
    let target_path = Path::new(&target);

    // Open Geth reader (hot Pebble DB + ancient fallback)
    let geth_reader = open_geth_block_reader(chaindata)
        .map_err(|e| format!("Cannot open Geth chaindata: {e}"))?;

    // Open ethrex Store (existing, no genesis init needed since it was already migrated)
    let store = Store::new_from_genesis(target_path, EngineType::RocksDB, &genesis).await?;

    let head_hash = geth_reader.read_head_block_hash()?;
    let source_head = geth_reader.read_block_number(head_hash)?;

    println!("Geth source head: #{source_head}");
    let target_head = store.get_latest_block_number().await?;
    println!("ethrex target head: #{target_head}");

    // Verify up to the target head (source may have more blocks if it kept
    // producing after the migration snapshot was taken).
    let verify_up_to = target_head.min(source_head);
    if source_head != target_head {
        eprintln!(
            "Note: head mismatch (Geth={source_head}, ethrex={target_head}). \
             Verifying blocks 1..={verify_up_to}."
        );
    }

    if verify_up_to == 0 {
        eprintln!("Nothing to verify (target head is 0).");
        std::process::exit(1);
    }

    println!("\n{:<10} {:<68} {}", "Block", "Hash (Geth)", "Match");
    println!("{}", "-".repeat(85));

    let mut all_ok = true;
    for block_number in 1..=verify_up_to {
        let geth_hash = match geth_reader.read_canonical_hash(block_number)? {
            Some(h) => h,
            None => {
                eprintln!("❌ Block #{block_number}: no canonical hash in Geth");
                all_ok = false;
                continue;
            }
        };

        let ethrex_hash = match store.get_canonical_block_hash(block_number).await? {
            Some(h) => h,
            None => {
                eprintln!("❌ Block #{block_number}: no canonical hash in ethrex");
                all_ok = false;
                continue;
            }
        };

        let matches = geth_hash == ethrex_hash;
        if !matches {
            all_ok = false;
        }
        println!(
            "Block #{:<6} {:<68} {}",
            block_number,
            format!("{geth_hash:?}"),
            if matches { "✅" } else { "❌ MISMATCH" }
        );
    }

    println!("{}", "-".repeat(85));
    if all_ok {
        println!("✅ All {verify_up_to} block hashes match — migration verified!");
        Ok(())
    } else {
        eprintln!("❌ Hash mismatches detected!");
        std::process::exit(1);
    }
}
