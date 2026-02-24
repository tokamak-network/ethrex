use std::{
    fs::OpenOptions,
    future::Future,
    io::Write,
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
const REPORT_SCHEMA_VERSION: u32 = 1;
const DEFAULT_RETRY_BASE_DELAY_MS: u64 = 1_000;
const MAX_RETRY_BASE_DELAY_MS: u64 = 60_000;

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
        #[arg(long = "report-file")]
        /// Optional path to append emitted reports (JSON lines in --json mode)
        report_file: Option<PathBuf>,
        #[arg(long = "retry-attempts", default_value_t = MAX_RETRY_ATTEMPTS, value_parser = clap::value_parser!(u32).range(1..=10))]
        /// Retry budget for retryable operations (1-10, inclusive)
        retry_attempts: u32,
        #[arg(long = "retry-base-delay-ms", default_value_t = DEFAULT_RETRY_BASE_DELAY_MS, value_parser = clap::value_parser!(u64).range(0..=MAX_RETRY_BASE_DELAY_MS))]
        /// Initial retry backoff delay in milliseconds (0-60000)
        retry_base_delay_ms: u64,
        #[arg(long = "continue-on-error", default_value_t = false)]
        /// Continue migrating subsequent blocks when a block-level import fails
        continue_on_error: bool,
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
    schema_version: u32,
    status: &'static str,
    phase: &'static str,
    source_head: u64,
    target_head: u64,
    plan: Option<MigrationPlan>,
    dry_run: bool,
    imported_blocks: u64,
    skipped_blocks: u64,
    elapsed_ms: u64,
    retry_attempts: u32,
    retries_performed: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorKind {
    Transient,
    Fatal,
}

#[derive(Debug)]
struct RetryFailure {
    attempts_used: u32,
    max_attempts: u32,
    kind: ErrorKind,
    message: String,
}

impl std::fmt::Display for RetryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (retry_attempts_used={} max_attempts={})",
            self.message, self.attempts_used, self.max_attempts
        )
    }
}

impl std::error::Error for RetryFailure {}

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

fn classify_error_from_message(message: &str) -> (ErrorKind, &'static str) {
    let msg = message.to_ascii_lowercase();
    let transient_markers = ["eagain", "etimedout", "timed out", "enospc", "temporar"];
    if transient_markers.iter().any(|marker| msg.contains(marker)) {
        return (ErrorKind::Transient, "message_marker");
    }

    (ErrorKind::Fatal, "default_fatal")
}

fn classify_io_error_kind(kind: std::io::ErrorKind) -> ErrorKind {
    use std::io::ErrorKind as IoErrorKind;

    match kind {
        IoErrorKind::WouldBlock
        | IoErrorKind::TimedOut
        | IoErrorKind::Interrupted
        | IoErrorKind::OutOfMemory
        | IoErrorKind::ConnectionReset
        | IoErrorKind::ConnectionAborted
        | IoErrorKind::NotConnected
        | IoErrorKind::BrokenPipe => ErrorKind::Transient,
        _ => ErrorKind::Fatal,
    }
}

fn classify_error_from_report(error: &eyre::Report) -> (ErrorKind, &'static str) {
    if let Some(retry_failure) = error.downcast_ref::<RetryFailure>() {
        return (retry_failure.kind, "retry_failure");
    }

    if let Some(io_error) = error.downcast_ref::<std::io::Error>() {
        return (classify_io_error_kind(io_error.kind()), "io_kind");
    }

    classify_error_from_message(&format!("{error:#}"))
}

fn compute_backoff_delay(base_delay: Duration, attempts_used: u32) -> Duration {
    let multiplier = 2u32.saturating_pow(attempts_used.saturating_sub(1));
    base_delay.checked_mul(multiplier).unwrap_or(Duration::MAX)
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
                let message = format!("{error:#}");
                let (kind, _) = classify_error_from_report(&error);
                if !kind.retryable() || attempts >= max_attempts {
                    return Err(eyre::Report::new(RetryFailure {
                        attempts_used: attempts,
                        max_attempts,
                        kind,
                        message,
                    }));
                }

                tokio::time::sleep(compute_backoff_delay(base_delay, attempts)).await;
            }
        }
    }
}

fn retry_sync<T, O>(mut operation: O, max_attempts: u32, base_delay: Duration) -> Result<(T, u32)>
where
    O: FnMut() -> Result<T>,
{
    let mut attempts = 0u32;

    loop {
        attempts += 1;
        match operation() {
            Ok(value) => return Ok((value, attempts)),
            Err(error) => {
                let message = format!("{error:#}");
                let (kind, _) = classify_error_from_report(&error);
                if !kind.retryable() || attempts >= max_attempts {
                    return Err(eyre::Report::new(RetryFailure {
                        attempts_used: attempts,
                        max_attempts,
                        kind,
                        message,
                    }));
                }

                std::thread::sleep(compute_backoff_delay(base_delay, attempts));
            }
        }
    }
}

#[derive(Serialize)]
struct MigrationErrorReport {
    schema_version: u32,
    status: &'static str,
    phase: &'static str,
    error_type: &'static str,
    error_classification: &'static str,
    retryable: bool,
    retry_attempts: u32,
    retry_attempts_used: Option<u32>,
    error: String,
    elapsed_ms: u64,
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis() as u64
}

fn build_migration_error_report(
    error: &eyre::Report,
    started_at: Instant,
    retry_attempts: u32,
) -> MigrationErrorReport {
    let retry_failure = error.downcast_ref::<RetryFailure>();
    let error_message = format!("{error:#}");
    let (error_kind, error_classification) = classify_error_from_report(error);

    MigrationErrorReport {
        schema_version: REPORT_SCHEMA_VERSION,
        status: "failed",
        phase: "execution",
        error_type: error_kind.as_str(),
        error_classification,
        retryable: error_kind.retryable(),
        retry_attempts,
        retry_attempts_used: retry_failure.map(|failure| failure.attempts_used),
        error: error_message,
        elapsed_ms: elapsed_ms(started_at),
    }
}

pub fn emit_error_report(
    json: bool,
    retry_attempts: u32,
    started_at: Instant,
    error: &eyre::Report,
    report_file: Option<&Path>,
) {
    if json {
        let report = build_migration_error_report(error, started_at, retry_attempts);

        match serde_json::to_string(&report) {
            Ok(encoded) => {
                println!("{encoded}");
                if let Err(write_error) = append_report_line(report_file, &encoded) {
                    eprintln!(
                        "Migration failed: {error:#}\nCannot write report file: {write_error:#}"
                    );
                }
            }
            Err(ser_error) => {
                eprintln!("Migration failed: {error:#}\nReport encoding failed: {ser_error}")
            }
        }
        return;
    }

    let line = format!(
        "Migration failed after {}ms: {error:#}",
        elapsed_ms(started_at)
    );
    eprintln!("{line}");
    if let Err(write_error) = append_report_line(report_file, &line) {
        eprintln!("Cannot write report file: {write_error:#}");
    }
}

fn append_report_line(report_file: Option<&Path>, line: &str) -> Result<()> {
    let Some(path) = report_file else {
        return Ok(());
    };

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("Cannot create report directory {parent:?}"))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .wrap_err_with(|| format!("Cannot open report file {path:?}"))?;
    writeln!(file, "{line}").wrap_err_with(|| format!("Cannot write report file {path:?}"))?;
    Ok(())
}

fn emit_report(report: &MigrationReport, json: bool, report_file: Option<&Path>) -> Result<()> {
    if json {
        let encoded =
            serde_json::to_string(report).wrap_err("Cannot serialize migration report")?;
        println!("{encoded}");
        append_report_line(report_file, &encoded)?;
        return Ok(());
    }

    let summary = match report.plan {
        Some(plan) => format!(
            "Migration plan: {} block(s), from #{}, to #{}",
            plan.block_count(),
            plan.start_block,
            plan.end_block
        ),
        None => format!(
            "Rocksdb store is already up to date (target head: {}, source head: {})",
            report.target_head, report.source_head
        ),
    };
    println!("{summary}");
    append_report_line(report_file, &summary)?;

    if report.dry_run {
        let line = "Dry-run complete: no data was written.";
        println!("{line}");
        append_report_line(report_file, line)?;
    } else if report.imported_blocks > 0 {
        let line = format!(
            "Migration completed successfully: imported {} block(s).",
            report.imported_blocks
        );
        println!("{line}");
        append_report_line(report_file, &line)?;

        if report.skipped_blocks > 0 {
            let skipped_line = format!(
                "Migration skipped {} block(s) due to --continue-on-error.",
                report.skipped_blocks
            );
            println!("{skipped_line}");
            append_report_line(report_file, &skipped_line)?;
        }
    }

    Ok(())
}

impl Subcommand {
    pub fn json_output(&self) -> bool {
        match self {
            Self::Libmdbx2Rocksdb { json, .. } => *json,
        }
    }

    pub fn retry_attempts(&self) -> u32 {
        match self {
            Self::Libmdbx2Rocksdb { retry_attempts, .. } => *retry_attempts,
        }
    }

    pub fn report_file(&self) -> Option<&Path> {
        match self {
            Self::Libmdbx2Rocksdb { report_file, .. } => report_file.as_deref(),
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
                retry_attempts,
                retry_base_delay_ms,
                continue_on_error,
                report_file,
            } => {
                migrate_libmdbx_to_rocksdb(
                    genesis_path,
                    old_storage_path,
                    new_storage_path,
                    *dry_run,
                    *json,
                    *retry_attempts,
                    Duration::from_millis(*retry_base_delay_ms),
                    *continue_on_error,
                    report_file.as_deref(),
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
    retry_attempts: u32,
    retry_base_delay: Duration,
    continue_on_error: bool,
    report_file: Option<&Path>,
) -> Result<()> {
    let started_at = Instant::now();
    let mut retries_performed = 0u32;

    let old_path = old_storage_path
        .to_str()
        .wrap_err("Invalid UTF-8 in old storage path")?;
    let (old_store, attempts) = retry_sync(
        || {
            ethrex_storage_libmdbx::Store::new(
                old_path,
                ethrex_storage_libmdbx::EngineType::Libmdbx,
            )
            .wrap_err_with(|| format!("Cannot open libmdbx store at {old_storage_path:?}"))
        },
        retry_attempts,
        retry_base_delay,
    )?;
    retries_performed += attempts.saturating_sub(1);

    let (_, attempts) = retry_async(
        || async {
            old_store
                .load_initial_state()
                .await
                .wrap_err("Cannot load libmdbx store state")
        },
        retry_attempts,
        retry_base_delay,
    )
    .await?;
    retries_performed += attempts.saturating_sub(1);

    let genesis = genesis_path
        .to_str()
        .wrap_err("Invalid UTF-8 in genesis path")?;
    let (new_store, attempts) = retry_async(
        || async {
            ethrex_storage::Store::new_from_genesis(
                new_storage_path,
                ethrex_storage::EngineType::RocksDB,
                genesis,
            )
            .await
            .wrap_err_with(|| format!("Cannot create/open rocksdb store at {new_storage_path:?}"))
        },
        retry_attempts,
        retry_base_delay,
    )
    .await?;
    retries_performed += attempts.saturating_sub(1);

    let (last_block_number, attempts) = retry_async(
        || async {
            old_store
                .get_latest_block_number()
                .await
                .wrap_err("Cannot get latest block from libmdbx store")
        },
        retry_attempts,
        retry_base_delay,
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
        retry_attempts,
        retry_base_delay,
    )
    .await?;
    retries_performed += attempts.saturating_sub(1);

    let Some(plan) = build_migration_plan(last_known_block, last_block_number) else {
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "up_to_date",
            phase: "planning",
            source_head: last_block_number,
            target_head: last_known_block,
            plan: None,
            dry_run,
            imported_blocks: 0,
            skipped_blocks: 0,
            elapsed_ms: elapsed_ms(started_at),
            retry_attempts,
            retries_performed,
        };
        emit_report(&report, json, report_file)?;
        return Ok(());
    };

    if dry_run {
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "planned",
            phase: "planning",
            source_head: last_block_number,
            target_head: last_known_block,
            plan: Some(plan),
            dry_run: true,
            imported_blocks: 0,
            skipped_blocks: 0,
            elapsed_ms: elapsed_ms(started_at),
            retry_attempts,
            retries_performed,
        };
        emit_report(&report, json, report_file)?;
        return Ok(());
    }

    emit_report(
        &MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "in_progress",
            phase: "execution",
            source_head: last_block_number,
            target_head: last_known_block,
            plan: Some(plan),
            dry_run: false,
            imported_blocks: 0,
            skipped_blocks: 0,
            elapsed_ms: elapsed_ms(started_at),
            retry_attempts,
            retries_performed,
        },
        json,
        report_file,
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
        retry_attempts,
        retry_base_delay,
    )
    .await?;
    retries_performed += attempts.saturating_sub(1);

    let mut added_blocks = Vec::new();
    let mut skipped_blocks = 0u64;
    for (block_number, body) in (plan.start_block..=plan.end_block).zip(block_bodies) {
        let header_result = retry_sync(
            || {
                old_store
                    .get_block_header(block_number)
                    .wrap_err_with(|| {
                        format!("Cannot fetch block header #{block_number} from libmdbx store")
                    })?
                    .ok_or_else(|| {
                        eyre::eyre!("Missing block header #{block_number} in libmdbx store")
                    })
            },
            retry_attempts,
            retry_base_delay,
        );

        let (header, attempts) = match header_result {
            Ok(value) => value,
            Err(error) if continue_on_error => {
                skipped_blocks += 1;
                eprintln!(
                    "Warning: skipping block #{block_number} after header read failure: {error:#}"
                );
                continue;
            }
            Err(error) => return Err(error),
        };
        retries_performed += attempts.saturating_sub(1);

        let header = migrate_block_header(header);
        let body = migrate_block_body(body);
        let block = Block::new(header, body);

        let block_hash = block.hash();
        let add_result = retry_sync(
            || {
                blockchain
                    .add_block_pipeline(block.clone())
                    .wrap_err_with(|| format!("Cannot add block {block_number} to rocksdb store"))
            },
            retry_attempts,
            retry_base_delay,
        );

        let (_, attempts) = match add_result {
            Ok(value) => value,
            Err(error) if continue_on_error => {
                skipped_blocks += 1;
                eprintln!(
                    "Warning: skipping block #{block_number} after import failure: {error:#}"
                );
                continue;
            }
            Err(error) => return Err(error),
        };
        retries_performed += attempts.saturating_sub(1);

        added_blocks.push((block_number, block_hash));
    }

    if added_blocks.is_empty() {
        return Err(eyre::eyre!(
            "Migration could not import any block in range #{}..=#{} (continue_on_error={continue_on_error})",
            plan.start_block,
            plan.end_block
        ));
    }

    let (head_block_number, head_block_hash) = *added_blocks
        .last()
        .ok_or_else(|| eyre::eyre!("Cannot determine migrated chain head"))?;

    let (_, attempts) = retry_async(
        || async {
            new_store
                .forkchoice_update(
                    added_blocks.clone(),
                    head_block_number,
                    head_block_hash,
                    None,
                    None,
                )
                .await
                .wrap_err("Cannot apply forkchoice update")
        },
        retry_attempts,
        retry_base_delay,
    )
    .await?;
    retries_performed += attempts.saturating_sub(1);

    if skipped_blocks > 0 {
        eprintln!(
            "Warning: migration completed with {skipped_blocks} skipped block(s) due to --continue-on-error"
        );
    }

    let report = MigrationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        status: "completed",
        phase: "execution",
        source_head: last_block_number,
        target_head: head_block_number,
        plan: Some(plan),
        dry_run: false,
        imported_blocks: added_blocks.len() as u64,
        skipped_blocks,
        elapsed_ms: elapsed_ms(started_at),
        retry_attempts,
        retries_performed,
    };
    emit_report(&report, json, report_file)?;

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
    use std::{
        fs,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Instant, SystemTime, UNIX_EPOCH},
    };

    use super::{
        CLI, DEFAULT_RETRY_BASE_DELAY_MS, MAX_RETRY_ATTEMPTS, MigrationErrorReport, MigrationPlan,
        MigrationReport, REPORT_SCHEMA_VERSION, RetryFailure, Subcommand, append_report_line,
        build_migration_error_report, build_migration_plan, classify_error_from_message,
        classify_error_from_report, classify_io_error_kind, compute_backoff_delay,
        emit_error_report, emit_report, retry_async, retry_sync,
    };
    use clap::Parser;
    use serde_json::{Value, json};

    fn unique_test_path(suffix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("migrations-cli-unit-{suffix}-{nanos}"))
    }

    #[test]
    fn emit_report_writes_json_line_to_report_file() {
        let report_path = unique_test_path("json-report").join("report.jsonl");
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "planned",
            phase: "planning",
            source_head: 12,
            target_head: 8,
            plan: Some(MigrationPlan {
                start_block: 9,
                end_block: 12,
            }),
            dry_run: true,
            imported_blocks: 0,
            skipped_blocks: 0,
            elapsed_ms: 3,
            retry_attempts: 3,
            retries_performed: 0,
        };

        emit_report(&report, true, Some(&report_path))
            .expect("json report emission should succeed");

        let file_content =
            fs::read_to_string(&report_path).expect("report file should be readable");
        let line = file_content
            .lines()
            .next()
            .expect("report file should contain one line");
        let parsed: Value = serde_json::from_str(line).expect("line should be valid json");
        assert_eq!(parsed["status"], "planned");
        assert_eq!(parsed["schema_version"], 1);

        if let Some(parent) = report_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn emit_report_writes_up_to_date_json_line_with_null_plan() {
        let report_path = unique_test_path("json-up-to-date").join("report.jsonl");
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "up_to_date",
            phase: "planning",
            source_head: 42,
            target_head: 42,
            plan: None,
            dry_run: false,
            imported_blocks: 0,
            skipped_blocks: 0,
            elapsed_ms: 4,
            retry_attempts: 3,
            retries_performed: 0,
        };

        emit_report(&report, true, Some(&report_path))
            .expect("json report emission should succeed");

        let file_content =
            fs::read_to_string(&report_path).expect("report file should be readable");
        let line = file_content
            .lines()
            .next()
            .expect("report file should contain one line");
        let parsed: Value = serde_json::from_str(line).expect("line should be valid json");

        assert_eq!(parsed["status"], "up_to_date");
        assert!(parsed["plan"].is_null());

        if let Some(parent) = report_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn emit_report_json_mode_appends_single_line_for_dry_run() {
        let report_path = unique_test_path("json-dry-run-single-line").join("report.jsonl");
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "planned",
            phase: "planning",
            source_head: 12,
            target_head: 8,
            plan: Some(MigrationPlan {
                start_block: 9,
                end_block: 12,
            }),
            dry_run: true,
            imported_blocks: 0,
            skipped_blocks: 0,
            elapsed_ms: 3,
            retry_attempts: 3,
            retries_performed: 0,
        };

        emit_report(&report, true, Some(&report_path))
            .expect("json report emission should succeed");

        let file_content =
            fs::read_to_string(&report_path).expect("report file should be readable");
        let lines: Vec<&str> = file_content.lines().collect();
        assert_eq!(lines.len(), 1, "json mode should append exactly one line");

        let parsed: Value = serde_json::from_str(lines[0]).expect("line should be valid json");
        assert_eq!(parsed["status"], "planned");
        assert_eq!(parsed["dry_run"], true);

        if let Some(parent) = report_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn emit_report_writes_human_lines_to_report_file() {
        let report_path = unique_test_path("human-report").join("report.log");
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "completed",
            phase: "execution",
            source_head: 20,
            target_head: 20,
            plan: Some(MigrationPlan {
                start_block: 13,
                end_block: 20,
            }),
            dry_run: false,
            imported_blocks: 8,
            skipped_blocks: 0,
            elapsed_ms: 10,
            retry_attempts: 3,
            retries_performed: 1,
        };

        emit_report(&report, false, Some(&report_path))
            .expect("human report emission should succeed");

        let file_content =
            fs::read_to_string(&report_path).expect("report file should be readable");
        assert!(file_content.contains("Migration plan: 8 block(s), from #13, to #20"));
        assert!(file_content.contains("Migration completed successfully: imported 8 block(s)."));

        if let Some(parent) = report_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn emit_report_writes_skipped_blocks_marker_to_report_file() {
        let report_path = unique_test_path("human-report-skipped").join("report.log");
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "completed",
            phase: "execution",
            source_head: 20,
            target_head: 18,
            plan: Some(MigrationPlan {
                start_block: 13,
                end_block: 20,
            }),
            dry_run: false,
            imported_blocks: 6,
            skipped_blocks: 2,
            elapsed_ms: 10,
            retry_attempts: 3,
            retries_performed: 1,
        };

        emit_report(&report, false, Some(&report_path))
            .expect("human report emission should succeed");

        let file_content =
            fs::read_to_string(&report_path).expect("report file should be readable");
        assert!(file_content.contains("Migration completed successfully: imported 6 block(s)."));
        assert!(file_content.contains("Migration skipped 2 block(s) due to --continue-on-error."));

        if let Some(parent) = report_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn emit_error_report_writes_json_line_to_report_file() {
        let report_path = unique_test_path("json-error-report").join("error.jsonl");
        let error = eyre::eyre!("temporary EAGAIN failure");

        emit_error_report(
            true,
            MAX_RETRY_ATTEMPTS,
            Instant::now(),
            &error,
            Some(&report_path),
        );

        let file_content =
            fs::read_to_string(&report_path).expect("report file should be readable");
        let line = file_content
            .lines()
            .next()
            .expect("report file should contain one line");
        let parsed: Value = serde_json::from_str(line).expect("line should be valid json");
        assert_eq!(parsed["status"], "failed");
        assert_eq!(parsed["retryable"], true);

        if let Some(parent) = report_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn emit_error_report_writes_human_line_to_report_file() {
        let report_path = unique_test_path("human-error-report").join("error.log");
        let error = eyre::eyre!("fatal corruption");

        emit_error_report(
            false,
            MAX_RETRY_ATTEMPTS,
            Instant::now(),
            &error,
            Some(&report_path),
        );

        let file_content =
            fs::read_to_string(&report_path).expect("report file should be readable");
        assert!(file_content.contains("Migration failed after"));
        assert!(file_content.contains("fatal corruption"));

        if let Some(parent) = report_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn append_report_line_creates_parent_dirs_and_appends() {
        let report_path = unique_test_path("append-lines").join("nested/reports/output.log");

        append_report_line(Some(&report_path), "first line").expect("first write should succeed");
        append_report_line(Some(&report_path), "second line").expect("second write should succeed");

        let file_content =
            fs::read_to_string(&report_path).expect("report file should be readable");
        let lines: Vec<&str> = file_content.lines().collect();
        assert_eq!(lines, vec!["first line", "second line"]);

        if let Some(parent) = report_path.parent() {
            let root = parent.parent().and_then(|p| p.parent()).unwrap_or(parent);
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn append_report_line_is_noop_without_file_path() {
        append_report_line(None, "ignored line").expect("none path should be no-op success");
    }

    #[test]
    fn emit_report_writes_up_to_date_summary_to_report_file() {
        let report_path = unique_test_path("up-to-date-report").join("report.log");
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "up_to_date",
            phase: "planning",
            source_head: 100,
            target_head: 100,
            plan: None,
            dry_run: false,
            imported_blocks: 0,
            skipped_blocks: 0,
            elapsed_ms: 2,
            retry_attempts: 3,
            retries_performed: 0,
        };

        emit_report(&report, false, Some(&report_path))
            .expect("up_to_date report emission should succeed");

        let file_content =
            fs::read_to_string(&report_path).expect("report file should be readable");
        assert!(
            file_content.contains(
                "Rocksdb store is already up to date (target head: 100, source head: 100)"
            )
        );
        assert!(!file_content.contains("Dry-run complete: no data was written."));

        if let Some(parent) = report_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn emit_report_writes_dry_run_marker_to_report_file() {
        let report_path = unique_test_path("dry-run-report").join("report.log");
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "planned",
            phase: "planning",
            source_head: 20,
            target_head: 10,
            plan: Some(MigrationPlan {
                start_block: 11,
                end_block: 20,
            }),
            dry_run: true,
            imported_blocks: 0,
            skipped_blocks: 0,
            elapsed_ms: 5,
            retry_attempts: 3,
            retries_performed: 0,
        };

        emit_report(&report, false, Some(&report_path))
            .expect("dry-run report emission should succeed");

        let file_content =
            fs::read_to_string(&report_path).expect("report file should be readable");
        assert!(file_content.contains("Migration plan: 10 block(s), from #11, to #20"));
        assert!(file_content.contains("Dry-run complete: no data was written."));

        if let Some(parent) = report_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

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
    fn parses_libmdbx2rocksdb_flags() {
        let cli = CLI::parse_from([
            "migrations",
            "libmdbx2rocksdb",
            "--genesis",
            "genesis.json",
            "--store.old",
            "old-db",
            "--store.new",
            "new-db",
            "--dry-run",
            "--json",
        ]);

        match cli.command {
            Subcommand::Libmdbx2Rocksdb {
                genesis_path,
                old_storage_path,
                new_storage_path,
                dry_run,
                json,
                report_file,
                retry_attempts,
                retry_base_delay_ms,
                continue_on_error,
            } => {
                assert_eq!(genesis_path, std::path::PathBuf::from("genesis.json"));
                assert_eq!(old_storage_path, std::path::PathBuf::from("old-db"));
                assert_eq!(new_storage_path, std::path::PathBuf::from("new-db"));
                assert!(dry_run);
                assert!(json);
                assert!(report_file.is_none());
                assert_eq!(retry_attempts, MAX_RETRY_ATTEMPTS);
                assert_eq!(retry_base_delay_ms, DEFAULT_RETRY_BASE_DELAY_MS);
                assert!(!continue_on_error);
            }
        }
    }

    #[test]
    fn parses_alias_l2r() {
        let cli = CLI::parse_from([
            "migrations",
            "l2r",
            "--genesis",
            "g.json",
            "--store.old",
            "a",
            "--store.new",
            "b",
        ]);

        match cli.command {
            Subcommand::Libmdbx2Rocksdb { dry_run, json, .. } => {
                assert!(!dry_run);
                assert!(!json);
            }
        }
    }

    #[test]
    fn rejects_missing_required_args() {
        let parsed = CLI::try_parse_from(["migrations", "libmdbx2rocksdb", "--genesis", "g.json"]);
        assert!(
            parsed.is_err(),
            "cli should fail when required store paths are missing"
        );
        let rendered = parsed.err().expect("must be clap error").to_string();

        assert!(rendered.contains("--store.old"));
        assert!(rendered.contains("--store.new"));
    }

    #[test]
    fn rejects_retry_attempts_out_of_range() {
        let parsed = CLI::try_parse_from([
            "migrations",
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
            "--retry-attempts",
            "0",
        ]);

        assert!(parsed.is_err());
    }

    #[test]
    fn parses_custom_retry_config() {
        let cli = CLI::parse_from([
            "migrations",
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
            "--retry-attempts",
            "5",
            "--retry-base-delay-ms",
            "250",
        ]);

        match cli.command {
            Subcommand::Libmdbx2Rocksdb {
                retry_attempts,
                retry_base_delay_ms,
                ..
            } => {
                assert_eq!(retry_attempts, 5);
                assert_eq!(retry_base_delay_ms, 250);
            }
        }
    }

    #[test]
    fn accepts_zero_retry_base_delay() {
        let cli = CLI::parse_from([
            "migrations",
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
            "--retry-base-delay-ms",
            "0",
        ]);

        match cli.command {
            Subcommand::Libmdbx2Rocksdb {
                retry_base_delay_ms,
                ..
            } => {
                assert_eq!(retry_base_delay_ms, 0);
            }
        }
    }

    #[test]
    fn parses_continue_on_error_flag() {
        let cli = CLI::parse_from([
            "migrations",
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
            "--continue-on-error",
        ]);

        match cli.command {
            Subcommand::Libmdbx2Rocksdb {
                continue_on_error, ..
            } => {
                assert!(continue_on_error);
            }
        }
    }

    #[test]
    fn rejects_retry_base_delay_out_of_range() {
        let parsed = CLI::try_parse_from([
            "migrations",
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
            "--retry-base-delay-ms",
            "60001",
        ]);

        assert!(parsed.is_err());
    }

    #[test]
    fn report_file_reflects_flag_value() {
        let with_report_file = CLI::parse_from([
            "migrations",
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
            "--report-file",
            "reports/migration.jsonl",
        ]);
        assert_eq!(
            with_report_file.command.report_file(),
            Some(std::path::Path::new("reports/migration.jsonl"))
        );

        let without_report_file = CLI::parse_from([
            "migrations",
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
        ]);
        assert!(without_report_file.command.report_file().is_none());
    }

    #[test]
    fn json_output_reflects_flag_value() {
        let with_json = CLI::parse_from([
            "migrations",
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
            "--json",
        ]);
        assert!(with_json.command.json_output());

        let without_json = CLI::parse_from([
            "migrations",
            "libmdbx2rocksdb",
            "--genesis",
            "g.json",
            "--store.old",
            "old",
            "--store.new",
            "new",
        ]);
        assert!(!without_json.command.json_output());
    }

    #[test]
    fn serializes_migration_report() {
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
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
            skipped_blocks: 0,
            elapsed_ms: 7,
            retry_attempts: 3,
            retries_performed: 1,
        };

        let encoded = serde_json::to_value(&report).expect("report should serialize");
        let expected = json!({
            "schema_version": 1,
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
            "skipped_blocks": 0,
            "elapsed_ms": 7,
            "retry_attempts": 3,
            "retries_performed": 1
        });
        assert_eq!(encoded, expected);
    }

    #[test]
    fn serializes_up_to_date_report_with_null_plan() {
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "up_to_date",
            phase: "planning",
            source_head: 100,
            target_head: 100,
            plan: None,
            dry_run: false,
            imported_blocks: 0,
            skipped_blocks: 0,
            elapsed_ms: 3,
            retry_attempts: 3,
            retries_performed: 0,
        };

        let encoded = serde_json::to_value(&report).expect("report should serialize");
        let expected = json!({
            "schema_version": 1,
            "status": "up_to_date",
            "phase": "planning",
            "source_head": 100,
            "target_head": 100,
            "plan": Value::Null,
            "dry_run": false,
            "imported_blocks": 0,
            "skipped_blocks": 0,
            "elapsed_ms": 3,
            "retry_attempts": 3,
            "retries_performed": 0
        });
        assert_eq!(encoded, expected);
    }

    #[test]
    fn success_report_json_contract_keys_are_stable() {
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "completed",
            phase: "execution",
            source_head: 10,
            target_head: 10,
            plan: Some(MigrationPlan {
                start_block: 1,
                end_block: 10,
            }),
            dry_run: false,
            imported_blocks: 10,
            skipped_blocks: 0,
            elapsed_ms: 55,
            retry_attempts: 3,
            retries_performed: 1,
        };

        let encoded = serde_json::to_value(&report).expect("report should serialize");
        let object = encoded.as_object().expect("must be json object");
        let expected_keys = [
            "schema_version",
            "status",
            "phase",
            "source_head",
            "target_head",
            "plan",
            "dry_run",
            "imported_blocks",
            "skipped_blocks",
            "elapsed_ms",
            "retry_attempts",
            "retries_performed",
        ];

        assert_eq!(object.len(), expected_keys.len());
        for key in expected_keys {
            assert!(object.contains_key(key), "missing key: {key}");
        }
    }

    #[test]
    fn serializes_completed_report_with_skipped_blocks() {
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "completed",
            phase: "execution",
            source_head: 42,
            target_head: 40,
            plan: Some(MigrationPlan {
                start_block: 11,
                end_block: 42,
            }),
            dry_run: false,
            imported_blocks: 30,
            skipped_blocks: 2,
            elapsed_ms: 12,
            retry_attempts: 3,
            retries_performed: 1,
        };

        let encoded = serde_json::to_value(&report).expect("report should serialize");
        assert_eq!(encoded["status"], "completed");
        assert_eq!(encoded["imported_blocks"], 30);
        assert_eq!(encoded["skipped_blocks"], 2);
    }

    #[test]
    fn serializes_error_report() {
        let report = MigrationErrorReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "failed",
            phase: "execution",
            error_type: "fatal",
            error_classification: "retry_failure",
            retryable: false,
            retry_attempts: 3,
            retry_attempts_used: Some(2),
            error: "boom".to_owned(),
            elapsed_ms: 11,
        };

        let encoded = serde_json::to_value(&report).expect("error report should serialize");
        let expected = json!({
            "schema_version": 1,
            "status": "failed",
            "phase": "execution",
            "error_type": "fatal",
            "error_classification": "retry_failure",
            "retryable": false,
            "retry_attempts": 3,
            "retry_attempts_used": 2,
            "error": "boom",
            "elapsed_ms": 11
        });
        assert_eq!(encoded, expected);
    }

    #[test]
    fn serializes_error_report_without_retry_attempts_used() {
        let report = MigrationErrorReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "failed",
            phase: "execution",
            error_type: "fatal",
            error_classification: "default_fatal",
            retryable: false,
            retry_attempts: 3,
            retry_attempts_used: None,
            error: "direct fatal failure".to_owned(),
            elapsed_ms: 9,
        };

        let encoded = serde_json::to_value(&report).expect("error report should serialize");
        let expected = json!({
            "schema_version": 1,
            "status": "failed",
            "phase": "execution",
            "error_type": "fatal",
            "error_classification": "default_fatal",
            "retryable": false,
            "retry_attempts": 3,
            "retry_attempts_used": Value::Null,
            "error": "direct fatal failure",
            "elapsed_ms": 9
        });
        assert_eq!(encoded, expected);
    }

    #[test]
    fn failure_report_json_contract_keys_are_stable() {
        let report = MigrationErrorReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "failed",
            phase: "execution",
            error_type: "transient",
            error_classification: "retry_failure",
            retryable: true,
            retry_attempts: 3,
            retry_attempts_used: Some(3),
            error: "temporary timeout".to_owned(),
            elapsed_ms: 77,
        };

        let encoded = serde_json::to_value(&report).expect("error report should serialize");
        let object = encoded.as_object().expect("must be json object");
        let expected_keys = [
            "schema_version",
            "status",
            "phase",
            "error_type",
            "error_classification",
            "retryable",
            "retry_attempts",
            "retry_attempts_used",
            "error",
            "elapsed_ms",
        ];

        assert_eq!(object.len(), expected_keys.len());
        for key in expected_keys {
            assert!(object.contains_key(key), "missing key: {key}");
        }
    }

    #[test]
    fn classifies_transient_error_markers() {
        assert_eq!(
            classify_error_from_message("read failed: EAGAIN").0,
            super::ErrorKind::Transient
        );
        assert_eq!(
            classify_error_from_message("operation timed out while reading").0,
            super::ErrorKind::Transient
        );
    }

    #[test]
    fn classifies_fatal_errors_by_default() {
        assert_eq!(
            classify_error_from_message("leveldb corrupted block").0,
            super::ErrorKind::Fatal
        );
    }

    #[test]
    fn classify_error_from_report_prefers_io_error_kind() {
        let io_error = std::io::Error::new(std::io::ErrorKind::TimedOut, "network timeout");
        let report = eyre::Report::new(io_error);
        let (kind, source) = classify_error_from_report(&report);

        assert_eq!(kind, super::ErrorKind::Transient);
        assert_eq!(source, "io_kind");
    }

    #[test]
    fn classify_io_error_kind_maps_expected_transient_cases() {
        assert_eq!(
            classify_io_error_kind(std::io::ErrorKind::ConnectionReset),
            super::ErrorKind::Transient
        );
        assert_eq!(
            classify_io_error_kind(std::io::ErrorKind::BrokenPipe),
            super::ErrorKind::Transient
        );
    }

    #[test]
    fn classify_io_error_kind_maps_expected_fatal_cases() {
        assert_eq!(
            classify_io_error_kind(std::io::ErrorKind::InvalidData),
            super::ErrorKind::Fatal
        );
        assert_eq!(
            classify_io_error_kind(std::io::ErrorKind::PermissionDenied),
            super::ErrorKind::Fatal
        );
    }

    #[test]
    fn classify_error_from_report_marks_permission_denied_as_fatal_io_kind() {
        let io_error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no permission");
        let report = eyre::Report::new(io_error);
        let (kind, source) = classify_error_from_report(&report);

        assert_eq!(kind, super::ErrorKind::Fatal);
        assert_eq!(source, "io_kind");
    }

    #[test]
    fn classify_error_from_report_falls_back_to_message_marker() {
        let report = eyre::eyre!("temporary EAGAIN read failure");
        let (kind, source) = classify_error_from_report(&report);

        assert_eq!(kind, super::ErrorKind::Transient);
        assert_eq!(source, "message_marker");
    }

    #[test]
    fn classify_error_from_report_falls_back_to_default_fatal() {
        let report = eyre::eyre!("unknown unrecoverable migration failure");
        let (kind, source) = classify_error_from_report(&report);

        assert_eq!(kind, super::ErrorKind::Fatal);
        assert_eq!(source, "default_fatal");
    }

    #[test]
    fn build_error_report_uses_io_classification() {
        let io_error = std::io::Error::new(std::io::ErrorKind::TimedOut, "network timeout");
        let report = eyre::Report::new(io_error);
        let error_report =
            build_migration_error_report(&report, Instant::now(), MAX_RETRY_ATTEMPTS);

        assert_eq!(error_report.error_type, "transient");
        assert_eq!(error_report.error_classification, "io_kind");
        assert!(error_report.retryable);
        assert_eq!(error_report.retry_attempts_used, None);
    }

    #[test]
    fn build_error_report_uses_retry_failure_metadata() {
        let report = eyre::Report::new(RetryFailure {
            attempts_used: 3,
            max_attempts: 3,
            kind: super::ErrorKind::Transient,
            message: "temporary timeout".to_owned(),
        });
        let error_report =
            build_migration_error_report(&report, Instant::now(), MAX_RETRY_ATTEMPTS);

        assert_eq!(error_report.error_type, "transient");
        assert_eq!(error_report.error_classification, "retry_failure");
        assert_eq!(error_report.retry_attempts_used, Some(3));
    }

    #[test]
    fn classify_error_from_report_prefers_retry_failure_over_wrapped_io() {
        let wrapped = eyre::Report::new(RetryFailure {
            attempts_used: 2,
            max_attempts: 3,
            kind: super::ErrorKind::Fatal,
            message: std::io::Error::new(std::io::ErrorKind::TimedOut, "socket timed out")
                .to_string(),
        });
        let (kind, source) = classify_error_from_report(&wrapped);

        assert_eq!(kind, super::ErrorKind::Fatal);
        assert_eq!(source, "retry_failure");
    }

    #[test]
    fn build_error_report_uses_message_marker_fallback() {
        let report = eyre::eyre!("temporary EAGAIN read failure");
        let error_report =
            build_migration_error_report(&report, Instant::now(), MAX_RETRY_ATTEMPTS);

        assert_eq!(error_report.error_type, "transient");
        assert_eq!(error_report.error_classification, "message_marker");
        assert_eq!(error_report.retry_attempts_used, None);
    }

    #[test]
    fn build_error_report_uses_default_fatal_fallback() {
        let report = eyre::eyre!("unexpected migration corruption");
        let error_report =
            build_migration_error_report(&report, Instant::now(), MAX_RETRY_ATTEMPTS);

        assert_eq!(error_report.error_type, "fatal");
        assert_eq!(error_report.error_classification, "default_fatal");
        assert_eq!(error_report.retry_attempts_used, None);
    }

    #[test]
    fn build_error_report_value_matrix_is_consistent() {
        let io_timeout = eyre::Report::new(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "network timeout",
        ));
        let io_report =
            build_migration_error_report(&io_timeout, Instant::now(), MAX_RETRY_ATTEMPTS);
        assert_eq!(io_report.error_type, "transient");
        assert_eq!(io_report.error_classification, "io_kind");
        assert!(io_report.retryable);
        assert_eq!(io_report.retry_attempts_used, None);

        let retry_failure = eyre::Report::new(RetryFailure {
            attempts_used: 3,
            max_attempts: 3,
            kind: super::ErrorKind::Transient,
            message: "temporary timeout".to_owned(),
        });
        let retry_report =
            build_migration_error_report(&retry_failure, Instant::now(), MAX_RETRY_ATTEMPTS);
        assert_eq!(retry_report.error_type, "transient");
        assert_eq!(retry_report.error_classification, "retry_failure");
        assert!(retry_report.retryable);
        assert_eq!(retry_report.retry_attempts_used, Some(3));

        let fatal = eyre::eyre!("corrupted leveldb block");
        let fatal_report = build_migration_error_report(&fatal, Instant::now(), MAX_RETRY_ATTEMPTS);
        assert_eq!(fatal_report.error_type, "fatal");
        assert_eq!(fatal_report.error_classification, "default_fatal");
        assert!(!fatal_report.retryable);
        assert_eq!(fatal_report.retry_attempts_used, None);
    }

    #[test]
    fn retry_failure_display_includes_attempt_metadata() {
        let failure = RetryFailure {
            attempts_used: 2,
            max_attempts: 3,
            kind: super::ErrorKind::Transient,
            message: "temporary timeout".to_owned(),
        };

        let rendered = failure.to_string();
        assert!(rendered.contains("retry_attempts_used=2"));
        assert!(rendered.contains("max_attempts=3"));
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
    async fn retry_async_does_not_retry_fatal_io_error() {
        let result = retry_async(
            || async {
                Err::<u64, _>(eyre::Report::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "permission denied",
                )))
            },
            3,
            std::time::Duration::from_millis(0),
        )
        .await;

        let error = result.expect_err("fatal io error should not be retried");
        let retry_failure = error
            .downcast_ref::<RetryFailure>()
            .expect("retry metadata should be attached");
        assert_eq!(retry_failure.attempts_used, 1);
        assert_eq!(retry_failure.max_attempts, 3);
        assert_eq!(retry_failure.kind, super::ErrorKind::Fatal);
    }

    #[tokio::test]
    async fn retry_async_retries_io_timeout_error_until_success() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_op = Arc::clone(&attempts);

        let (value, total_attempts) = retry_async(
            move || {
                let attempts_for_op = Arc::clone(&attempts_for_op);
                async move {
                    let current = attempts_for_op.fetch_add(1, Ordering::SeqCst);
                    if current == 0 {
                        Err(eyre::Report::new(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "socket timed out",
                        )))
                    } else {
                        Ok(7u64)
                    }
                }
            },
            3,
            std::time::Duration::from_millis(0),
        )
        .await
        .expect("io timeout should be retried and eventually succeed");

        assert_eq!(value, 7);
        assert_eq!(total_attempts, 2);
    }

    #[tokio::test]
    async fn retry_async_with_single_attempt_stops_immediately() {
        let result = retry_async(
            || async { Err::<u64, _>(eyre::eyre!("temporary EAGAIN failure")) },
            1,
            std::time::Duration::from_millis(0),
        )
        .await;

        let error = result.expect_err("single-attempt retry should fail immediately");
        let retry_failure = error
            .downcast_ref::<RetryFailure>()
            .expect("retry metadata should be attached");
        assert_eq!(retry_failure.attempts_used, 1);
        assert_eq!(retry_failure.max_attempts, 1);
        assert_eq!(retry_failure.kind, super::ErrorKind::Transient);
    }

    #[tokio::test]
    async fn retry_async_failure_carries_typed_retry_metadata() {
        let result = retry_async(
            || async { Err::<u64, _>(eyre::eyre!("temporary EAGAIN failure")) },
            3,
            std::time::Duration::from_millis(0),
        )
        .await;

        let error = result.expect_err("expected retry exhaustion failure");
        let retry_failure = error
            .downcast_ref::<RetryFailure>()
            .expect("retry failure metadata should be attached");

        assert_eq!(retry_failure.attempts_used, 3);
        assert_eq!(retry_failure.max_attempts, 3);
        assert_eq!(retry_failure.kind, super::ErrorKind::Transient);

        let (kind, source) = classify_error_from_report(&error);
        assert_eq!(kind, super::ErrorKind::Transient);
        assert_eq!(source, "retry_failure");
    }

    #[tokio::test]
    async fn retry_async_honors_custom_max_attempts() {
        let result = retry_async(
            || async { Err::<u64, _>(eyre::eyre!("temporary ETIMEDOUT failure")) },
            5,
            std::time::Duration::from_millis(0),
        )
        .await;

        let error = result.expect_err("expected retry exhaustion at configured max attempts");
        let retry_failure = error
            .downcast_ref::<RetryFailure>()
            .expect("retry failure metadata should be attached");

        assert_eq!(retry_failure.attempts_used, 5);
        assert_eq!(retry_failure.max_attempts, 5);
        assert_eq!(retry_failure.kind, super::ErrorKind::Transient);
    }

    #[test]
    fn retry_sync_retries_transient_error_until_success() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_op = Arc::clone(&attempts);

        let (value, total_attempts) = retry_sync(
            move || {
                let current = attempts_for_op.fetch_add(1, Ordering::SeqCst);
                if current == 0 {
                    Err(eyre::eyre!("temporary EAGAIN failure"))
                } else {
                    Ok(42u64)
                }
            },
            3,
            std::time::Duration::from_millis(0),
        )
        .expect("retry should eventually succeed");

        assert_eq!(value, 42);
        assert_eq!(total_attempts, 2);
    }

    #[test]
    fn retry_sync_does_not_retry_fatal_error() {
        let result = retry_sync(
            || Err::<u64, _>(eyre::eyre!("corrupted leveldb block")),
            3,
            std::time::Duration::from_millis(0),
        );

        assert!(result.is_err());
    }

    #[test]
    fn retry_sync_does_not_retry_fatal_io_error() {
        let result = retry_sync(
            || {
                Err::<u64, _>(eyre::Report::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "permission denied",
                )))
            },
            3,
            std::time::Duration::from_millis(0),
        );

        let error = result.expect_err("fatal io error should not be retried");
        let retry_failure = error
            .downcast_ref::<RetryFailure>()
            .expect("retry metadata should be attached");
        assert_eq!(retry_failure.attempts_used, 1);
        assert_eq!(retry_failure.max_attempts, 3);
        assert_eq!(retry_failure.kind, super::ErrorKind::Fatal);
    }

    #[test]
    fn retry_sync_retries_io_timeout_error_until_success() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_op = Arc::clone(&attempts);

        let (value, total_attempts) = retry_sync(
            move || {
                let current = attempts_for_op.fetch_add(1, Ordering::SeqCst);
                if current == 0 {
                    Err(eyre::Report::new(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "socket timed out",
                    )))
                } else {
                    Ok(11u64)
                }
            },
            3,
            std::time::Duration::from_millis(0),
        )
        .expect("io timeout should be retried and eventually succeed");

        assert_eq!(value, 11);
        assert_eq!(total_attempts, 2);
    }

    #[test]
    fn retry_sync_failure_carries_typed_retry_metadata() {
        let result = retry_sync(
            || {
                Err::<u64, _>(eyre::Report::new(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "socket timed out",
                )))
            },
            3,
            std::time::Duration::from_millis(0),
        );

        let error = result.expect_err("expected retry exhaustion failure");
        let retry_failure = error
            .downcast_ref::<RetryFailure>()
            .expect("retry failure metadata should be attached");

        assert_eq!(retry_failure.attempts_used, 3);
        assert_eq!(retry_failure.max_attempts, 3);
        assert_eq!(retry_failure.kind, super::ErrorKind::Transient);

        let (kind, source) = classify_error_from_report(&error);
        assert_eq!(kind, super::ErrorKind::Transient);
        assert_eq!(source, "retry_failure");
    }

    #[test]
    fn retry_sync_honors_custom_max_attempts() {
        let result = retry_sync(
            || {
                Err::<u64, _>(eyre::Report::new(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "socket timed out",
                )))
            },
            5,
            std::time::Duration::from_millis(0),
        );

        let error = result.expect_err("expected retry exhaustion at configured max attempts");
        let retry_failure = error
            .downcast_ref::<RetryFailure>()
            .expect("retry failure metadata should be attached");

        assert_eq!(retry_failure.attempts_used, 5);
        assert_eq!(retry_failure.max_attempts, 5);
        assert_eq!(retry_failure.kind, super::ErrorKind::Transient);
    }

    #[test]
    fn compute_backoff_delay_doubles_per_attempt() {
        let base = std::time::Duration::from_millis(100);
        assert_eq!(
            compute_backoff_delay(base, 1),
            std::time::Duration::from_millis(100)
        );
        assert_eq!(
            compute_backoff_delay(base, 2),
            std::time::Duration::from_millis(200)
        );
        assert_eq!(
            compute_backoff_delay(base, 3),
            std::time::Duration::from_millis(400)
        );
    }

    #[test]
    fn compute_backoff_delay_saturates_on_overflow() {
        let delay = compute_backoff_delay(std::time::Duration::MAX, 2);
        assert_eq!(delay, std::time::Duration::MAX);
    }
}
