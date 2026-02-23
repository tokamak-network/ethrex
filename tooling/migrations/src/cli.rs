use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use clap::{Parser as ClapParser, Subcommand as ClapSubcommand};
use ethrex_blockchain::{Blockchain, BlockchainOptions, BlockchainType, L2Config};
use ethrex_common::types::Block;
use eyre::{ContextCompat, Result, WrapErr};
use serde::Serialize;

use crate::utils::{migrate_block_body, migrate_block_header};

#[allow(clippy::upper_case_acronyms)]
#[derive(ClapParser)]
#[command(
    name = "migrations",
    author = "Lambdaclass",
    about = "ethrex migration tools"
)]
pub struct CLI {
    #[command(subcommand)]
    pub command: Subcommand,
}

#[derive(ClapSubcommand)]
pub enum Subcommand {
    #[command(
        name = "libmdbx2rocksdb",
        visible_alias = "l2r",
        about = "Migrate a libmdbx database to rocksdb"
    )]
    Libmdbx2Rocksdb {
        #[arg(long = "genesis")]
        /// Path to the genesis file for the old database
        genesis_path: PathBuf,
        #[arg(long = "store.old")]
        /// Path to the target Libmbdx database to migrate
        old_storage_path: PathBuf,
        #[arg(long = "store.new")]
        /// Path for the new RocksDB database
        new_storage_path: PathBuf,
        #[arg(long = "dry-run", default_value_t = false)]
        /// Validate source/target stores and print migration plan without writing blocks
        dry_run: bool,
        #[arg(long = "json", default_value_t = false)]
        /// Emit machine-readable JSON output
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
struct MigrationPlan {
    start_block: u64,
    end_block: u64,
}

impl MigrationPlan {
    fn block_count(&self) -> u64 {
        self.end_block - self.start_block + 1
    }
}

#[derive(Serialize)]
struct MigrationReport {
    status: &'static str,
    phase: &'static str,
    source_head: u64,
    target_head: u64,
    plan: Option<MigrationPlan>,
    dry_run: bool,
    imported_blocks: u64,
    elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorKind {
    Transient,
    Fatal,
}

impl ErrorKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Fatal => "fatal",
        }
    }

    fn retryable(self) -> bool {
        matches!(self, Self::Transient)
    }
}

fn classify_error(message: &str) -> ErrorKind {
    let msg = message.to_ascii_lowercase();
    let transient_markers = ["eagain", "etimedout", "timed out", "enospc", "temporar"];
    if transient_markers.iter().any(|marker| msg.contains(marker)) {
        return ErrorKind::Transient;
    }

    ErrorKind::Fatal
}

#[derive(Serialize)]
struct MigrationErrorReport {
    status: &'static str,
    phase: &'static str,
    error_type: &'static str,
    retryable: bool,
    error: String,
    elapsed_ms: u64,
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis() as u64
}

pub fn emit_error_report(json: bool, started_at: Instant, error: &eyre::Report) {
    if json {
        let error_message = format!("{error:#}");
        let error_kind = classify_error(&error_message);
        let report = MigrationErrorReport {
            status: "failed",
            phase: "execution",
            error_type: error_kind.as_str(),
            retryable: error_kind.retryable(),
            error: error_message,
            elapsed_ms: elapsed_ms(started_at),
        };

        match serde_json::to_string(&report) {
            Ok(encoded) => println!("{encoded}"),
            Err(ser_error) => {
                eprintln!("Migration failed: {error:#}\nReport encoding failed: {ser_error}")
            }
        }
        return;
    }

    eprintln!(
        "Migration failed after {}ms: {error:#}",
        elapsed_ms(started_at)
    );
}

fn emit_report(report: &MigrationReport, json: bool) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string(report).wrap_err("Cannot serialize migration report")?
        );
        return Ok(());
    }

    match report.plan {
        Some(plan) => println!(
            "Migration plan: {} block(s), from #{}, to #{}",
            plan.block_count(),
            plan.start_block,
            plan.end_block
        ),
        None => println!(
            "Rocksdb store is already up to date (target head: {}, source head: {})",
            report.target_head, report.source_head
        ),
    }

    if report.dry_run {
        println!("Dry-run complete: no data was written.");
    } else if report.imported_blocks > 0 {
        println!(
            "Migration completed successfully: imported {} block(s).",
            report.imported_blocks
        );
    }

    Ok(())
}

impl Subcommand {
    pub fn json_output(&self) -> bool {
        match self {
            Self::Libmdbx2Rocksdb { json, .. } => *json,
        }
    }

    pub async fn run(&self) -> Result<()> {
        match self {
            Self::Libmdbx2Rocksdb {
                genesis_path,
                old_storage_path,
                new_storage_path,
                dry_run,
                json,
            } => {
                migrate_libmdbx_to_rocksdb(
                    genesis_path,
                    old_storage_path,
                    new_storage_path,
                    *dry_run,
                    *json,
                )
                .await
            }
        }
    }
}

async fn migrate_libmdbx_to_rocksdb(
    genesis_path: &Path,
    old_storage_path: &Path,
    new_storage_path: &Path,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let started_at = Instant::now();

    let old_path = old_storage_path
        .to_str()
        .wrap_err("Invalid UTF-8 in old storage path")?;
    let old_store =
        ethrex_storage_libmdbx::Store::new(old_path, ethrex_storage_libmdbx::EngineType::Libmdbx)
            .wrap_err_with(|| format!("Cannot open libmdbx store at {old_storage_path:?}"))?;
    old_store
        .load_initial_state()
        .await
        .wrap_err("Cannot load libmdbx store state")?;

    let genesis = genesis_path
        .to_str()
        .wrap_err("Invalid UTF-8 in genesis path")?;
    let new_store = ethrex_storage::Store::new_from_genesis(
        new_storage_path,
        ethrex_storage::EngineType::RocksDB,
        genesis,
    )
    .await
    .wrap_err_with(|| format!("Cannot create/open rocksdb store at {new_storage_path:?}"))?;

    let last_block_number = old_store
        .get_latest_block_number()
        .await
        .wrap_err("Cannot get latest block from libmdbx store")?;
    let last_known_block = new_store
        .get_latest_block_number()
        .await
        .wrap_err("Cannot get latest block from rocksdb store")?;

    let Some(plan) = build_migration_plan(last_known_block, last_block_number) else {
        let report = MigrationReport {
            status: "up_to_date",
            phase: "planning",
            source_head: last_block_number,
            target_head: last_known_block,
            plan: None,
            dry_run,
            imported_blocks: 0,
            elapsed_ms: elapsed_ms(started_at),
        };
        emit_report(&report, json)?;
        return Ok(());
    };

    if dry_run {
        let report = MigrationReport {
            status: "planned",
            phase: "planning",
            source_head: last_block_number,
            target_head: last_known_block,
            plan: Some(plan),
            dry_run: true,
            imported_blocks: 0,
            elapsed_ms: elapsed_ms(started_at),
        };
        emit_report(&report, json)?;
        return Ok(());
    }

    emit_report(
        &MigrationReport {
            status: "in_progress",
            phase: "execution",
            source_head: last_block_number,
            target_head: last_known_block,
            plan: Some(plan),
            dry_run: false,
            imported_blocks: 0,
            elapsed_ms: elapsed_ms(started_at),
        },
        json,
    )?;

    let blockchain_opts = BlockchainOptions {
        // TODO: we may want to migrate using a specified fee config
        r#type: BlockchainType::L2(L2Config::default()),
        ..Default::default()
    };
    let blockchain = Blockchain::new(new_store.clone(), blockchain_opts);

    let block_bodies = old_store
        .get_block_bodies(plan.start_block, plan.end_block)
        .await
        .wrap_err("Cannot get block bodies from libmdbx store")?;

    let block_headers = (plan.start_block..=plan.end_block).map(|i| {
        old_store
            .get_block_header(i)
            .wrap_err_with(|| format!("Cannot fetch block header #{i} from libmdbx store"))?
            .ok_or_else(|| eyre::eyre!("Missing block header #{i} in libmdbx store"))
    });

    let blocks = block_headers.zip(block_bodies);
    let mut added_blocks = Vec::new();
    for (header, body) in blocks {
        let header = migrate_block_header(header?);
        let body = migrate_block_body(body);
        let block_number = header.number;
        let block = Block::new(header, body);

        let block_hash = block.hash();
        blockchain
            .add_block_pipeline(block)
            .wrap_err_with(|| format!("Cannot add block {block_number} to rocksdb store"))?;
        added_blocks.push((block_number, block_hash));
    }

    let last_block = old_store
        .get_block_header(plan.end_block)
        .wrap_err_with(|| format!("Cannot fetch last block header #{}", plan.end_block))?
        .ok_or_else(|| eyre::eyre!("Missing block header #{}", plan.end_block))?;
    new_store
        .forkchoice_update(
            added_blocks,
            last_block.number,
            last_block.hash(),
            None,
            None,
        )
        .await
        .wrap_err("Cannot apply forkchoice update")?;

    let report = MigrationReport {
        status: "completed",
        phase: "execution",
        source_head: last_block_number,
        target_head: plan.end_block,
        plan: Some(plan),
        dry_run: false,
        imported_blocks: plan.block_count(),
        elapsed_ms: elapsed_ms(started_at),
    };
    emit_report(&report, json)?;

    Ok(())
}

fn build_migration_plan(last_known_block: u64, last_source_block: u64) -> Option<MigrationPlan> {
    (last_known_block < last_source_block).then_some(MigrationPlan {
        start_block: last_known_block + 1,
        end_block: last_source_block,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        MigrationErrorReport, MigrationPlan, MigrationReport, build_migration_plan, classify_error,
    };
    use serde_json::{Value, json};

    #[test]
    fn no_plan_when_target_is_up_to_date() {
        let plan = build_migration_plan(100, 100);
        assert!(plan.is_none());
    }

    #[test]
    fn no_plan_when_target_is_ahead() {
        let plan = build_migration_plan(101, 100);
        assert!(plan.is_none());
    }

    #[test]
    fn builds_plan_when_source_is_ahead() {
        let plan = build_migration_plan(12, 20).expect("plan should exist");
        assert_eq!(plan.start_block, 13);
        assert_eq!(plan.end_block, 20);
        assert_eq!(plan.block_count(), 8);
    }

    #[test]
    fn serializes_migration_report() {
        let report = MigrationReport {
            status: "planned",
            phase: "planning",
            source_head: 42,
            target_head: 40,
            plan: Some(MigrationPlan {
                start_block: 41,
                end_block: 42,
            }),
            dry_run: true,
            imported_blocks: 0,
            elapsed_ms: 7,
        };

        let encoded = serde_json::to_value(&report).expect("report should serialize");
        let expected = json!({
            "status": "planned",
            "phase": "planning",
            "source_head": 42,
            "target_head": 40,
            "plan": {
                "start_block": 41,
                "end_block": 42
            },
            "dry_run": true,
            "imported_blocks": 0,
            "elapsed_ms": 7
        });
        assert_eq!(encoded, expected);
    }

    #[test]
    fn serializes_up_to_date_report_with_null_plan() {
        let report = MigrationReport {
            status: "up_to_date",
            phase: "planning",
            source_head: 100,
            target_head: 100,
            plan: None,
            dry_run: false,
            imported_blocks: 0,
            elapsed_ms: 3,
        };

        let encoded = serde_json::to_value(&report).expect("report should serialize");
        let expected = json!({
            "status": "up_to_date",
            "phase": "planning",
            "source_head": 100,
            "target_head": 100,
            "plan": Value::Null,
            "dry_run": false,
            "imported_blocks": 0,
            "elapsed_ms": 3
        });
        assert_eq!(encoded, expected);
    }

    #[test]
    fn serializes_error_report() {
        let report = MigrationErrorReport {
            status: "failed",
            phase: "execution",
            error_type: "fatal",
            retryable: false,
            error: "boom".to_owned(),
            elapsed_ms: 11,
        };

        let encoded = serde_json::to_value(&report).expect("error report should serialize");
        let expected = json!({
            "status": "failed",
            "phase": "execution",
            "error_type": "fatal",
            "retryable": false,
            "error": "boom",
            "elapsed_ms": 11
        });
        assert_eq!(encoded, expected);
    }

    #[test]
    fn classifies_transient_error_markers() {
        assert_eq!(
            classify_error("read failed: EAGAIN"),
            super::ErrorKind::Transient
        );
        assert_eq!(
            classify_error("operation timed out while reading"),
            super::ErrorKind::Transient
        );
    }

    #[test]
    fn classifies_fatal_errors_by_default() {
        assert_eq!(
            classify_error("leveldb corrupted block"),
            super::ErrorKind::Fatal
        );
    }
}
