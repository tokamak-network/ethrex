use std::{
    future::Future,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use clap::{Parser as ClapParser, Subcommand as ClapSubcommand};
use ethrex_blockchain::{Blockchain, BlockchainOptions, BlockchainType, L2Config};
use ethrex_common::types::Block;
use eyre::{ContextCompat, Result, WrapErr};
use serde::Serialize;

use crate::utils::{migrate_block_body, migrate_block_header};

const MAX_RETRY_ATTEMPTS: u32 = 3;
const RETRY_BASE_DELAY: Duration = Duration::from_secs(1);

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
    retry_attempts: u32,
    retries_performed: u32,
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

async fn retry_async<T, O, Fut>(
    mut operation: O,
    max_attempts: u32,
    base_delay: Duration,
) -> Result<(T, u32)>
where
    O: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempts = 0u32;

    loop {
        attempts += 1;
        match operation().await {
            Ok(value) => return Ok((value, attempts)),
            Err(error) => {
                let is_retryable = classify_error(&format!("{error:#}")).retryable();
                if !is_retryable || attempts >= max_attempts {
                    return Err(eyre::eyre!("{error:#} (retry_attempts_used={attempts})"));
                }

                let backoff = base_delay * 2u32.pow(attempts - 1);
                tokio::time::sleep(backoff).await;
            }
        }
    }
}

#[derive(Serialize)]
struct MigrationErrorReport {
    status: &'static str,
    phase: &'static str,
    error_type: &'static str,
    retryable: bool,
    retry_attempts: u32,
    retry_attempts_used: Option<u32>,
    error: String,
    elapsed_ms: u64,
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis() as u64
}

fn extract_retry_attempts_used(message: &str) -> Option<u32> {
    let marker = "retry_attempts_used=";
    let start = message.find(marker)? + marker.len();
    let digits: String = message[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();

    if digits.is_empty() {
        return None;
    }

    digits.parse().ok()
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
            retry_attempts: MAX_RETRY_ATTEMPTS,
            retry_attempts_used: extract_retry_attempts_used(&error_message),
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
    let mut retries_performed = 0u32;

    let old_path = old_storage_path
        .to_str()
        .wrap_err("Invalid UTF-8 in old storage path")?;
    let old_store =
        ethrex_storage_libmdbx::Store::new(old_path, ethrex_storage_libmdbx::EngineType::Libmdbx)
            .wrap_err_with(|| format!("Cannot open libmdbx store at {old_storage_path:?}"))?;
    let (_, attempts) = retry_async(
        || async {
            old_store
                .load_initial_state()
                .await
                .wrap_err("Cannot load libmdbx store state")
        },
        MAX_RETRY_ATTEMPTS,
        RETRY_BASE_DELAY,
    )
    .await?;
    retries_performed += attempts.saturating_sub(1);

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

    let (last_block_number, attempts) = retry_async(
        || async {
            old_store
                .get_latest_block_number()
                .await
                .wrap_err("Cannot get latest block from libmdbx store")
        },
        MAX_RETRY_ATTEMPTS,
        RETRY_BASE_DELAY,
    )
    .await?;
    retries_performed += attempts.saturating_sub(1);

    let (last_known_block, attempts) = retry_async(
        || async {
            new_store
                .get_latest_block_number()
                .await
                .wrap_err("Cannot get latest block from rocksdb store")
        },
        MAX_RETRY_ATTEMPTS,
        RETRY_BASE_DELAY,
    )
    .await?;
    retries_performed += attempts.saturating_sub(1);

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
            retry_attempts: MAX_RETRY_ATTEMPTS,
            retries_performed,
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
            retry_attempts: MAX_RETRY_ATTEMPTS,
            retries_performed,
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
            retry_attempts: MAX_RETRY_ATTEMPTS,
            retries_performed,
        },
        json,
    )?;

    let blockchain_opts = BlockchainOptions {
        // TODO: we may want to migrate using a specified fee config
        r#type: BlockchainType::L2(L2Config::default()),
        ..Default::default()
    };
    let blockchain = Blockchain::new(new_store.clone(), blockchain_opts);

    let (block_bodies, attempts) = retry_async(
        || async {
            old_store
                .get_block_bodies(plan.start_block, plan.end_block)
                .await
                .wrap_err("Cannot get block bodies from libmdbx store")
        },
        MAX_RETRY_ATTEMPTS,
        RETRY_BASE_DELAY,
    )
    .await?;
    retries_performed += attempts.saturating_sub(1);

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
    let (_, attempts) = retry_async(
        || async {
            new_store
                .forkchoice_update(
                    added_blocks.clone(),
                    last_block.number,
                    last_block.hash(),
                    None,
                    None,
                )
                .await
                .wrap_err("Cannot apply forkchoice update")
        },
        MAX_RETRY_ATTEMPTS,
        RETRY_BASE_DELAY,
    )
    .await?;
    retries_performed += attempts.saturating_sub(1);

    let report = MigrationReport {
        status: "completed",
        phase: "execution",
        source_head: last_block_number,
        target_head: plan.end_block,
        plan: Some(plan),
        dry_run: false,
        imported_blocks: plan.block_count(),
        elapsed_ms: elapsed_ms(started_at),
        retry_attempts: MAX_RETRY_ATTEMPTS,
        retries_performed,
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
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{
        MigrationErrorReport, MigrationPlan, MigrationReport, build_migration_plan, classify_error,
        extract_retry_attempts_used, retry_async,
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
            retry_attempts: 3,
            retries_performed: 1,
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
            "elapsed_ms": 7,
            "retry_attempts": 3,
            "retries_performed": 1
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
            retry_attempts: 3,
            retries_performed: 0,
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
            "elapsed_ms": 3,
            "retry_attempts": 3,
            "retries_performed": 0
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
            retry_attempts: 3,
            retry_attempts_used: Some(2),
            error: "boom".to_owned(),
            elapsed_ms: 11,
        };

        let encoded = serde_json::to_value(&report).expect("error report should serialize");
        let expected = json!({
            "status": "failed",
            "phase": "execution",
            "error_type": "fatal",
            "retryable": false,
            "retry_attempts": 3,
            "retry_attempts_used": 2,
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

    #[tokio::test]
    async fn retry_async_retries_transient_error_until_success() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_op = Arc::clone(&attempts);

        let (value, total_attempts) = retry_async(
            move || {
                let attempts_for_op = Arc::clone(&attempts_for_op);
                async move {
                    let current = attempts_for_op.fetch_add(1, Ordering::SeqCst);
                    if current == 0 {
                        Err(eyre::eyre!("temporary EAGAIN failure"))
                    } else {
                        Ok(42u64)
                    }
                }
            },
            3,
            std::time::Duration::from_millis(0),
        )
        .await
        .expect("retry should eventually succeed");

        assert_eq!(value, 42);
        assert_eq!(total_attempts, 2);
    }

    #[tokio::test]
    async fn retry_async_does_not_retry_fatal_error() {
        let result = retry_async(
            || async { Err::<u64, _>(eyre::eyre!("corrupted leveldb block")) },
            3,
            std::time::Duration::from_millis(0),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn retry_async_failure_includes_attempts_used_marker() {
        let result = retry_async(
            || async { Err::<u64, _>(eyre::eyre!("temporary EAGAIN failure")) },
            3,
            std::time::Duration::from_millis(0),
        )
        .await;

        let error = result.expect_err("expected retry exhaustion failure");
        let message = format!("{error:#}");
        assert_eq!(extract_retry_attempts_used(&message), Some(3));
    }

    #[test]
    fn extracts_retry_attempts_used_from_message() {
        let message = "db timeout (retry_attempts_used=2)";
        assert_eq!(extract_retry_attempts_used(message), Some(2));
        assert_eq!(extract_retry_attempts_used("no marker"), None);
    }
}
