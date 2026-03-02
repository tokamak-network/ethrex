use std::{
    fs::OpenOptions,
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use clap::{Parser as ClapParser, Subcommand as ClapSubcommand};
use eyre::{ContextCompat, Result, WrapErr};
use serde::Serialize;

const MAX_RETRY_ATTEMPTS: u32 = 3;
const REPORT_SCHEMA_VERSION: u32 = 1;
const DEFAULT_RETRY_BASE_DELAY_MS: u64 = 1_000;
const MAX_RETRY_BASE_DELAY_MS: u64 = 60_000;
const VERIFICATION_PROGRESS_INTERVAL: u64 = 10;

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
        name = "geth2rocksdb",
        visible_alias = "g2r",
        about = "Migrate Geth chaindata (LevelDB/Pebble) to ethrex RocksDB"
    )]
    Geth2Rocksdb {
        #[arg(long = "source")]
        /// Path to Geth chaindata directory (LevelDB or Pebble)
        geth_chaindata: PathBuf,
        #[arg(long = "target")]
        /// Path for the new ethrex RocksDB database
        target_storage: PathBuf,
        #[arg(long = "genesis")]
        /// Path to the genesis file for ethrex initialization
        genesis_path: PathBuf,
        #[arg(long = "dry-run", default_value_t = false)]
        /// Detect Geth DB type and print migration plan without writing blocks
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
        #[arg(long = "verify-offline", default_value_t = true, action = clap::ArgAction::Set)]
        /// Run offline DB-to-DB verification after migration
        verify_offline: bool,
        #[arg(long = "verify-start-block")]
        /// Optional start block override for offline verification
        verify_start_block: Option<u64>,
        #[arg(long = "verify-end-block")]
        /// Optional end block override for offline verification
        verify_end_block: Option<u64>,
        #[arg(long = "skip-state-trie-check", default_value_t = false)]
        /// Skip ethrex has_state_root check during offline verification
        skip_state_trie_check: bool,
        #[arg(long = "verify-deep", default_value_t = false)]
        /// Run deep verification: receipt existence check for blocks with transactions
        verify_deep: bool,
        #[arg(long = "blocks-only", default_value_t = false)]
        /// Only migrate block data (skip state: accounts, storage, code)
        blocks_only: bool,
        #[arg(long = "from-block")]
        /// Start block migration from this block number (auto-detects merge block if unset)
        from_block: Option<u64>,
        #[arg(long = "ethrex-ready", default_value_t = true, action = clap::ArgAction::Set)]
        /// Run ethrex startup compatibility check after migration
        ethrex_ready: bool,
    },
    #[command(
        name = "geth2lmdb",
        visible_alias = "g2l",
        about = "Migrate Geth chaindata (Pebble) to py-ethclient LMDB format"
    )]
    Geth2Lmdb {
        #[arg(long = "source")]
        /// Path to Geth chaindata directory (Pebble)
        geth_chaindata: PathBuf,
        #[arg(long = "target")]
        /// Path for the output LMDB database
        lmdb_path: PathBuf,
        #[arg(long = "dry-run", default_value_t = false)]
        /// Detect Geth DB type and print migration plan without writing
        dry_run: bool,
        #[arg(long = "json", default_value_t = false)]
        /// Emit machine-readable JSON output
        json: bool,
        #[arg(long = "report-file")]
        /// Optional path to append emitted reports
        report_file: Option<PathBuf>,
        #[arg(long = "blocks-only", default_value_t = false)]
        /// Only migrate block data (skip state: accounts, storage, code)
        blocks_only: bool,
        #[arg(long = "map-size-gb", default_value_t = 4)]
        /// LMDB map size in GB (default: 4)
        map_size_gb: u32,
        #[arg(long = "continue-on-error", default_value_t = false)]
        /// Continue migrating when individual items fail
        continue_on_error: bool,
        #[arg(long = "verify-offline", default_value_t = true, action = clap::ArgAction::Set)]
        /// Run offline DB-to-DB verification after migration
        verify_offline: bool,
        #[arg(long = "verify-start-block")]
        /// Optional start block override for offline verification
        verify_start_block: Option<u64>,
        #[arg(long = "verify-end-block")]
        /// Optional end block override for offline verification
        verify_end_block: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
struct MigrationPlan {
    start_block: u64,
    end_block: u64,
}

impl MigrationPlan {
    fn block_count(&self) -> u64 {
        self.end_block.saturating_sub(self.start_block) + 1
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
            Self::Geth2Rocksdb { json, .. } => *json,
            Self::Geth2Lmdb { json, .. } => *json,
        }
    }

    pub fn retry_attempts(&self) -> u32 {
        match self {
            Self::Geth2Rocksdb { retry_attempts, .. } => *retry_attempts,
            Self::Geth2Lmdb { .. } => MAX_RETRY_ATTEMPTS,
        }
    }

    pub fn report_file(&self) -> Option<&Path> {
        match self {
            Self::Geth2Rocksdb { report_file, .. } => report_file.as_deref(),
            Self::Geth2Lmdb { report_file, .. } => report_file.as_deref(),
        }
    }

    pub async fn run(&self) -> Result<()> {
        match self {
            Self::Geth2Rocksdb {
                geth_chaindata,
                target_storage,
                genesis_path,
                dry_run,
                json,
                retry_attempts,
                retry_base_delay_ms,
                continue_on_error,
                verify_offline,
                verify_start_block,
                verify_end_block,
                skip_state_trie_check,
                verify_deep,
                report_file,
                blocks_only,
                from_block,
                ethrex_ready,
            } => {
                // TUI is on by default when compiled with the `tui` feature,
                // unless JSON output is requested.
                let use_tui = cfg!(feature = "tui") && !json;
                migrate_geth_to_rocksdb(
                    geth_chaindata,
                    target_storage,
                    genesis_path,
                    *dry_run,
                    *json,
                    *retry_attempts,
                    Duration::from_millis(*retry_base_delay_ms),
                    *continue_on_error,
                    *verify_offline,
                    *verify_start_block,
                    *verify_end_block,
                    *skip_state_trie_check,
                    *verify_deep,
                    *ethrex_ready,
                    report_file.as_deref(),
                    use_tui,
                    *blocks_only,
                    *from_block,
                )
                .await
            }
            Self::Geth2Lmdb {
                geth_chaindata,
                lmdb_path,
                dry_run,
                json,
                report_file,
                blocks_only,
                map_size_gb,
                continue_on_error,
                verify_offline,
                verify_start_block,
                verify_end_block,
            } => {
                let use_tui = cfg!(feature = "tui") && !json;
                migrate_geth_to_lmdb(
                    geth_chaindata,
                    lmdb_path,
                    *dry_run,
                    *json,
                    *blocks_only,
                    *map_size_gb,
                    *continue_on_error,
                    *verify_offline,
                    *verify_start_block,
                    *verify_end_block,
                    report_file.as_deref(),
                    use_tui,
                )
                .await
            }
        }
    }
}

fn build_migration_plan(
    last_known_block: u64,
    last_source_block: u64,
    resume_from_block: Option<u64>,
) -> Result<Option<MigrationPlan>> {
    if let Some(resume_start) = resume_from_block {
        if resume_start <= last_known_block {
            return Err(eyre::eyre!(
                "Invalid --resume-from-block={resume_start}: must be greater than target head #{last_known_block}"
            ));
        }

        if resume_start > last_source_block {
            return Err(eyre::eyre!(
                "Invalid --resume-from-block={resume_start}: must be <= source head #{last_source_block}"
            ));
        }

        return Ok(Some(MigrationPlan {
            start_block: resume_start,
            end_block: last_source_block,
        }));
    }

    Ok(
        (last_known_block < last_source_block).then_some(MigrationPlan {
            start_block: last_known_block + 1,
            end_block: last_source_block,
        }),
    )
}

#[derive(Debug, Clone, Copy)]
struct OfflineVerificationSummary {
    start_block: u64,
    end_block: u64,
    checked_blocks: u64,
    mismatches: u64,
    body_checks_passed: u64,
}

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
    target_path: &std::path::Path,
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

    // 1. metadata.json existence check
    let metadata_path = target_path.join("metadata.json");
    if metadata_path.exists() {
        checks.metadata_json = "pass".into();
    } else {
        checks.metadata_json = "FAIL: metadata.json not found".into();
        all_pass = false;
    }

    // 2. Latest block number — use get_latest_block_number
    let latest_block_result = store.get_latest_block_number().await;
    let latest_block_number = match &latest_block_result {
        Ok(num) => {
            checks.latest_block_number = format!("pass (block {num})");
            Some(*num)
        }
        Err(e) => {
            checks.latest_block_number = format!("FAIL: {e}");
            all_pass = false;
            None
        }
    };

    // 3. Latest header loadable
    let latest_header = latest_block_number.and_then(|num| store.get_block_header(num).ok());
    match &latest_header {
        Some(Some(_)) => checks.latest_header = "pass".into(),
        Some(None) => {
            checks.latest_header = "FAIL: latest header not found".into();
            all_pass = false;
        }
        None => {
            checks.latest_header = "FAIL: could not load latest header".into();
            all_pass = false;
        }
    }
    // Flatten for later use
    let latest_header_inner = latest_header.and_then(|h| h);

    // 4. ChainConfig
    let config = store.get_chain_config();
    checks.chain_config = format!("pass (chain_id={})", config.chain_id);

    // 5. Genesis block
    match store.get_canonical_block_hash(0).await {
        Ok(Some(genesis_hash)) => {
            let header_ok = store
                .get_block_header(0)
                .map(|h| h.is_some())
                .unwrap_or(false);
            let body_ok = store
                .get_block_body(0)
                .await
                .map(|b| b.is_some())
                .unwrap_or(false);
            if header_ok && body_ok {
                checks.genesis_block = format!("pass ({genesis_hash:?})");
            } else {
                checks.genesis_block =
                    "FAIL: genesis hash exists but header/body missing".into();
                all_pass = false;
            }
        }
        Ok(None) => {
            checks.genesis_block = "FAIL: no canonical hash for block 0".into();
            all_pass = false;
        }
        Err(e) => {
            checks.genesis_block = format!("FAIL: {e}");
            all_pass = false;
        }
    }

    // 6. State root valid
    if let Some(header) = &latest_header_inner {
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
    } else {
        checks.state_root_valid = "SKIP: no latest header".into();
    }

    EthrexReadyReport {
        ethrex_ready: all_pass,
        checks,
    }
}

fn resolve_verify_range(
    default_start: u64,
    default_end: u64,
    verify_start: Option<u64>,
    verify_end: Option<u64>,
) -> Result<(u64, u64)> {
    let start = verify_start.unwrap_or(default_start);
    let end = verify_end.unwrap_or(default_end);

    if start > end {
        return Err(eyre::eyre!(
            "Invalid verification range: start #{start} is greater than end #{end}"
        ));
    }

    if start < default_start || end > default_end {
        return Err(eyre::eyre!(
            "Invalid verification range #{}..=#{} (allowed #{}..=#{}).",
            start,
            end,
            default_start,
            default_end
        ));
    }

    Ok((start, end))
}

#[allow(clippy::too_many_arguments)]
async fn verify_geth_to_rocksdb_offline(
    geth_reader: &crate::readers::geth_db::GethBlockReader,
    store: &ethrex_storage::Store,
    start_block: u64,
    end_block: u64,
    skip_state_trie_check: bool,
    verify_deep: bool,
    tui: bool,
    #[cfg(feature = "tui")] tui_tx: &Option<
        tokio::sync::mpsc::Sender<crate::tui::event::ProgressEvent>,
    >,
) -> Result<OfflineVerificationSummary> {
    let total_blocks = end_block.saturating_sub(start_block) + 1;
    let started = Instant::now();
    let mut mismatches = 0u64;
    let mut checked = 0u64;
    let mut body_checks_passed = 0u64;

    #[cfg(feature = "tui")]
    if let Some(tx) = tui_tx {
        let _ = tx
            .send(crate::tui::event::ProgressEvent::VerificationStarted {
                start_block,
                end_block,
                total_blocks,
                state_trie_check: !skip_state_trie_check,
            })
            .await;
    }

    for block_number in start_block..=end_block {
        let mut block_mismatch = false;

        let geth_hash = geth_reader
            .read_canonical_hash(block_number)
            .map_err(|e| {
                eyre::eyre!("verify #{block_number}: cannot read geth canonical hash: {e}")
            })?
            .ok_or_else(|| eyre::eyre!("verify #{block_number}: missing geth canonical hash"))?;

        let ethrex_hash = store
            .get_canonical_block_hash(block_number)
            .await?
            .ok_or_else(|| eyre::eyre!("verify #{block_number}: missing ethrex canonical hash"))?;

        if geth_hash != ethrex_hash {
            mismatches += 1;
            block_mismatch = true;
            #[cfg(feature = "tui")]
            if let Some(tx) = tui_tx {
                let _ = tx.try_send(crate::tui::event::ProgressEvent::VerificationMismatch {
                    block_number,
                    reason: format!(
                        "canonical hash mismatch geth={geth_hash:?} ethrex={ethrex_hash:?}"
                    ),
                });
            }
        } else {
            let geth_header = geth_reader
                .read_block_header(block_number, geth_hash)
                .map_err(|e| eyre::eyre!("verify #{block_number}: cannot read geth header: {e}"))?
                .ok_or_else(|| eyre::eyre!("verify #{block_number}: missing geth header"))?;

            let ethrex_header = store
                .get_block_header(block_number)?
                .ok_or_else(|| eyre::eyre!("verify #{block_number}: missing ethrex header"))?;

            if geth_header.hash() != ethrex_header.hash() {
                mismatches += 1;
                block_mismatch = true;
                #[cfg(feature = "tui")]
                if let Some(tx) = tui_tx {
                    let _ = tx.try_send(crate::tui::event::ProgressEvent::VerificationMismatch {
                        block_number,
                        reason: "header hash mismatch".into(),
                    });
                }
            }

            if geth_header.state_root != ethrex_header.state_root {
                mismatches += 1;
                block_mismatch = true;
                #[cfg(feature = "tui")]
                if let Some(tx) = tui_tx {
                    let _ = tx.try_send(crate::tui::event::ProgressEvent::VerificationMismatch {
                        block_number,
                        reason: format!(
                            "state root mismatch geth={:?} ethrex={:?}",
                            geth_header.state_root, ethrex_header.state_root
                        ),
                    });
                }
            }

            if !skip_state_trie_check && !store.has_state_root(ethrex_header.state_root)? {
                mismatches += 1;
                block_mismatch = true;
                #[cfg(feature = "tui")]
                if let Some(tx) = tui_tx {
                    let _ = tx.try_send(crate::tui::event::ProgressEvent::VerificationMismatch {
                        block_number,
                        reason: format!("missing state trie root {:?}", ethrex_header.state_root),
                    });
                }
            }

            // Body verification: compare transaction count between Geth and ethrex
            let geth_body = geth_reader
                .read_block_body(block_number, geth_hash)
                .map_err(|e| {
                    eyre::eyre!("verify #{block_number}: cannot read geth body: {e}")
                })?;
            let ethrex_body = store.get_block_body(block_number).await?;

            match (&geth_body, &ethrex_body) {
                (Some(gb), Some(eb)) => {
                    if gb.transactions.len() != eb.transactions.len() {
                        mismatches += 1;
                        block_mismatch = true;
                        #[cfg(feature = "tui")]
                        if let Some(tx) = tui_tx {
                            let _ = tx.try_send(
                                crate::tui::event::ProgressEvent::VerificationMismatch {
                                    block_number,
                                    reason: format!(
                                        "body tx count mismatch geth={} ethrex={}",
                                        gb.transactions.len(),
                                        eb.transactions.len()
                                    ),
                                },
                            );
                        }
                    } else {
                        body_checks_passed += 1;
                    }
                }
                (Some(_), None) => {
                    mismatches += 1;
                    block_mismatch = true;
                    #[cfg(feature = "tui")]
                    if let Some(tx) = tui_tx {
                        let _ = tx.try_send(
                            crate::tui::event::ProgressEvent::VerificationMismatch {
                                block_number,
                                reason: "ethrex body missing".into(),
                            },
                        );
                    }
                }
                (None, Some(_)) => {
                    // Geth body not available (e.g. ancient not accessible), skip
                }
                (None, None) => {
                    // Both missing, skip
                }
            }

            // Deep verification: check receipt existence for blocks with transactions
            if verify_deep {
                if let Some(eb) = &ethrex_body {
                    if !eb.transactions.is_empty() {
                        let mut has_all_receipts = true;
                        for idx in 0..eb.transactions.len() {
                            if store
                                .get_receipt(block_number, idx as u64)
                                .await?
                                .is_none()
                            {
                                has_all_receipts = false;
                                break;
                            }
                        }
                        if !has_all_receipts {
                            mismatches += 1;
                            block_mismatch = true;
                            #[cfg(feature = "tui")]
                            if let Some(tx) = tui_tx {
                                let _ = tx.try_send(
                                    crate::tui::event::ProgressEvent::VerificationMismatch {
                                        block_number,
                                        reason: "missing receipts for block with transactions"
                                            .into(),
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }

        checked += 1;

        #[cfg(feature = "tui")]
        if let Some(tx) = tui_tx
            && (checked.is_multiple_of(VERIFICATION_PROGRESS_INTERVAL) || checked == total_blocks)
        {
            let _ = tx.try_send(crate::tui::event::ProgressEvent::VerificationProgress {
                checked,
                total: total_blocks,
                mismatches,
                elapsed: started.elapsed(),
            });
        }

        if !tui
            && (checked.is_multiple_of(VERIFICATION_PROGRESS_INTERVAL) || checked == total_blocks)
        {
            let elapsed = started.elapsed().as_secs_f64();
            let rate = if elapsed > 0.0 {
                checked as f64 / elapsed
            } else {
                0.0
            };
            let pct = (checked as f64 * 100.0) / total_blocks as f64;
            println!(
                "[verify] {checked}/{total_blocks} ({pct:5.1}%) mismatches={mismatches} elapsed={elapsed:7.1}s rate={rate:7.2}/s"
            );
        }

        if !tui && block_mismatch {
            eprintln!("[verify] mismatch at block #{block_number}");
        }
    }

    #[cfg(feature = "tui")]
    if let Some(tx) = tui_tx {
        let _ = tx.try_send(crate::tui::event::ProgressEvent::VerificationCompleted {
            checked,
            mismatches,
            elapsed: started.elapsed(),
        });
    }

    Ok(OfflineVerificationSummary {
        start_block,
        end_block,
        checked_blocks: checked,
        mismatches,
        body_checks_passed,
    })
}

#[allow(clippy::too_many_arguments)]
fn verify_geth_to_lmdb_offline(
    geth_reader: &crate::readers::geth_db::GethBlockReader,
    lmdb: &crate::writers::lmdb::LmdbWriter,
    start_block: u64,
    end_block: u64,
    tui: bool,
    #[cfg(feature = "tui")] tui_tx: &Option<
        tokio::sync::mpsc::Sender<crate::tui::event::ProgressEvent>,
    >,
) -> Result<OfflineVerificationSummary> {
    use ethrex_common::types::BlockHeader;
    use ethrex_rlp::decode::RLPDecode;

    let total_blocks = end_block.saturating_sub(start_block) + 1;
    let started = Instant::now();
    let mut checked = 0u64;
    let mut mismatches = 0u64;
    let rtxn = lmdb
        .read_txn()
        .map_err(|e| eyre::eyre!("LMDB read transaction failed: {e}"))?;

    for block_number in start_block..=end_block {
        let mut block_mismatch = false;
        let geth_hash = geth_reader
            .read_canonical_hash(block_number)
            .map_err(|e| {
                eyre::eyre!("verify #{block_number}: cannot read geth canonical hash: {e}")
            })?
            .ok_or_else(|| eyre::eyre!("verify #{block_number}: missing geth canonical hash"))?;

        let lmdb_hash_raw = lmdb
            .get_canonical(&rtxn, block_number)
            .map_err(|e| eyre::eyre!("verify #{block_number}: cannot read LMDB canonical: {e}"))?
            .ok_or_else(|| eyre::eyre!("verify #{block_number}: missing LMDB canonical hash"))?;

        if lmdb_hash_raw.len() != 32 {
            return Err(eyre::eyre!(
                "verify #{block_number}: LMDB canonical hash length is {}, expected 32",
                lmdb_hash_raw.len()
            ));
        }

        let mut lmdb_hash_bytes = [0u8; 32];
        lmdb_hash_bytes.copy_from_slice(&lmdb_hash_raw);
        let lmdb_hash = ethrex_common::H256(lmdb_hash_bytes);

        if geth_hash != lmdb_hash {
            mismatches += 1;
            block_mismatch = true;
            #[cfg(feature = "tui")]
            if let Some(tx) = tui_tx {
                let _ = tx.try_send(crate::tui::event::ProgressEvent::VerificationMismatch {
                    block_number,
                    reason: format!("canonical hash mismatch geth={geth_hash:?} lmdb={lmdb_hash:?}"),
                });
            }
        } else {
            let geth_header = geth_reader
                .read_block_header(block_number, geth_hash)
                .map_err(|e| eyre::eyre!("verify #{block_number}: cannot read geth header: {e}"))?
                .ok_or_else(|| eyre::eyre!("verify #{block_number}: missing geth header"))?;

            let lmdb_header_rlp = lmdb
                .get_header(&rtxn, &lmdb_hash_bytes)
                .map_err(|e| eyre::eyre!("verify #{block_number}: cannot read LMDB header: {e}"))?
                .ok_or_else(|| eyre::eyre!("verify #{block_number}: missing LMDB header"))?;

            let lmdb_header = BlockHeader::decode(&lmdb_header_rlp).map_err(|e| {
                eyre::eyre!("verify #{block_number}: cannot decode LMDB header: {e:?}")
            })?;

            if geth_header.hash() != lmdb_header.hash() {
                mismatches += 1;
                block_mismatch = true;
                #[cfg(feature = "tui")]
                if let Some(tx) = tui_tx {
                    let _ = tx.try_send(crate::tui::event::ProgressEvent::VerificationMismatch {
                        block_number,
                        reason: "header hash mismatch".into(),
                    });
                }
            }
            if geth_header.state_root != lmdb_header.state_root {
                mismatches += 1;
                block_mismatch = true;
                #[cfg(feature = "tui")]
                if let Some(tx) = tui_tx {
                    let _ = tx.try_send(crate::tui::event::ProgressEvent::VerificationMismatch {
                        block_number,
                        reason: format!(
                            "state root mismatch geth={:?} lmdb={:?}",
                            geth_header.state_root, lmdb_header.state_root
                        ),
                    });
                }
            }
        }

        checked += 1;
        #[cfg(feature = "tui")]
        if let Some(tx) = tui_tx
            && (checked.is_multiple_of(VERIFICATION_PROGRESS_INTERVAL) || checked == total_blocks)
        {
            let _ = tx.try_send(crate::tui::event::ProgressEvent::VerificationProgress {
                checked,
                total: total_blocks,
                mismatches,
                elapsed: started.elapsed(),
            });
        }

        if !tui
            && (checked.is_multiple_of(VERIFICATION_PROGRESS_INTERVAL) || checked == total_blocks)
        {
            let elapsed = started.elapsed().as_secs_f64();
            let rate = if elapsed > 0.0 {
                checked as f64 / elapsed
            } else {
                0.0
            };
            let pct = (checked as f64 * 100.0) / total_blocks as f64;
            println!(
                "[verify] {checked}/{total_blocks} ({pct:5.1}%) mismatches={mismatches} elapsed={elapsed:7.1}s rate={rate:7.2}/s"
            );
        }

        if !tui && block_mismatch {
            eprintln!("[verify] mismatch at block #{block_number}");
        }
    }

    Ok(OfflineVerificationSummary {
        start_block,
        end_block,
        checked_blocks: checked,
        mismatches,
        body_checks_passed: 0, // LMDB verification does not check bodies yet
    })
}

/// Migrates Geth chaindata (LevelDB or Pebble) to ethrex RocksDB storage
#[allow(clippy::too_many_arguments)]
async fn migrate_geth_to_rocksdb(
    geth_chaindata: &Path,
    target_storage: &Path,
    genesis_path: &Path,
    dry_run: bool,
    json: bool,
    retry_attempts: u32,
    retry_base_delay: Duration,
    continue_on_error: bool,
    verify_offline: bool,
    verify_start_block: Option<u64>,
    verify_end_block: Option<u64>,
    skip_state_trie_check: bool,
    verify_deep: bool,
    ethrex_ready: bool,
    report_file: Option<&Path>,
    tui: bool,
    blocks_only: bool,
    from_block: Option<u64>,
) -> Result<()> {
    use crate::detect::{GethDbType, detect_geth_db_type};
    use crate::readers::open_geth_block_reader;

    const BATCH_SIZE: u64 = 1_000;

    let started_at = Instant::now();
    let mut retries_performed = 0u32;

    // Phase 1: Detect and open Geth reader
    let db_type =
        detect_geth_db_type(geth_chaindata).wrap_err("Failed to detect Geth database type")?;

    if json {
        eprintln!(
            r#"{{"phase":"detect","db_type":"{}","chaindata_path":"{}"}}"#,
            match db_type {
                GethDbType::LevelDB => "leveldb",
                GethDbType::Pebble => "pebble",
                GethDbType::Unknown => "unknown",
            },
            geth_chaindata.display()
        );
    }

    let geth_reader = open_geth_block_reader(geth_chaindata)
        .map_err(|e| eyre::eyre!("Failed to open Geth chaindata reader: {}", e))?;

    // Phase 2: Read Geth head block number
    let head_hash = retry_sync(
        || {
            geth_reader
                .read_head_block_hash()
                .map_err(|e| eyre::eyre!("Cannot read Geth head block hash: {}", e))
        },
        retry_attempts,
        retry_base_delay,
    )
    .map(|(v, a)| {
        retries_performed += a.saturating_sub(1);
        v
    })?;

    let last_source_block = retry_sync(
        || {
            geth_reader
                .read_block_number(head_hash)
                .map_err(|e| eyre::eyre!("Cannot read Geth head block number: {}", e))
        },
        retry_attempts,
        retry_base_delay,
    )
    .map(|(v, a)| {
        retries_performed += a.saturating_sub(1);
        v
    })?;

    // Phase 3: Open (or create) target ethrex RocksDB store
    let genesis = genesis_path
        .to_str()
        .wrap_err("Invalid UTF-8 in genesis path")?;

    let (new_store, attempts) = retry_async(
        || async {
            ethrex_storage::Store::new_from_genesis(
                target_storage,
                ethrex_storage::EngineType::RocksDB,
                genesis,
            )
            .await
            .wrap_err_with(|| format!("Cannot create/open rocksdb store at {target_storage:?}"))
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

    // Phase 3b: Determine effective start block (auto-detect merge block if needed)
    let resume_from_block = if let Some(explicit_from_block) = from_block {
        // User explicitly specified --from-block, use it
        Some(explicit_from_block)
    } else {
        // Auto-detect merge block from chain config
        let chain_config = new_store.get_chain_config();
        if let Some(merge_block) = chain_config.merge_netsplit_block {
            // Only use merge block if:
            // 1. We haven't reached it yet in migration (merge_block > last_known_block)
            // 2. It actually exists in source data (merge_block <= last_source_block)
            if merge_block > last_known_block && merge_block <= last_source_block {
                Some(merge_block)
            } else {
                None
            }
        } else {
            None
        }
    };

    // Phase 4: Build migration plan
    let Some(plan) = build_migration_plan(last_known_block, last_source_block, resume_from_block)? else {
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "up_to_date",
            phase: "planning",
            source_head: last_source_block,
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
            source_head: last_source_block,
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

    // Phase 5: Block-by-block migration in batches
    // Optionally start TUI dashboard
    #[cfg(feature = "tui")]
    let (mut tui_tx, mut tui_handle): (
        Option<tokio::sync::mpsc::Sender<crate::tui::event::ProgressEvent>>,
        Option<tokio::task::JoinHandle<()>>,
    ) = if tui {
        let (tx, rx) = tokio::sync::mpsc::channel::<crate::tui::event::ProgressEvent>(256);
        let handle = tokio::spawn(crate::tui::run_tui(rx));
        (Some(tx), Some(handle))
    } else {
        (None, None)
    };

    #[cfg(not(feature = "tui"))]
    {
        if tui {
            eprintln!("--tui 플래그를 사용하려면 --features tui 로 빌드하세요.");
        }
    }

    if !tui {
        emit_report(
            &MigrationReport {
                schema_version: REPORT_SCHEMA_VERSION,
                status: "in_progress",
                phase: "execution",
                source_head: last_source_block,
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
    }

    #[cfg(feature = "tui")]
    if let Some(tx) = &tui_tx {
        let db_type_str = match db_type {
            crate::detect::GethDbType::Pebble => "Pebble",
            crate::detect::GethDbType::LevelDB => "LevelDB",
            crate::detect::GethDbType::Unknown => "Unknown",
        };
        let _ = tx
            .send(crate::tui::event::ProgressEvent::Init {
                source_path: geth_chaindata.display().to_string(),
                target_path: target_storage.display().to_string(),
                db_type: db_type_str.to_string(),
                start_block: plan.start_block,
                end_block: plan.end_block,
            })
            .await;
    }

    // Track migration progress without accumulating all block references.
    // Only keep the last head and aggregate counts to avoid O(N) memory.
    let mut total_imported: u64 = 0;
    let mut last_head: Option<(u64, ethrex_common::H256)> = None;
    let mut skipped_blocks = 0u64;

    // Run the batch migration loop, capturing any error for TUI cleanup.
    let migration_result: Result<()> = async {
        let mut batch_start = plan.start_block;
        while batch_start <= plan.end_block {
            let batch_end = (batch_start + BATCH_SIZE - 1).min(plan.end_block);
            let mut batch: Vec<ethrex_common::types::Block> = Vec::new();
            let mut batch_canonical: Vec<(u64, ethrex_common::H256)> = Vec::new();
            let mut batch_receipts: Vec<(ethrex_common::H256, Vec<ethrex_common::types::Receipt>)> =
                Vec::new();

            for block_number in batch_start..=batch_end {
                // Read canonical hash for this block number
                let canonical_hash_result = retry_sync(
                    || {
                        geth_reader
                            .read_canonical_hash(block_number)
                            .map_err(|e| {
                                eyre::eyre!(
                                    "Cannot read canonical hash for block #{block_number}: {e}"
                                )
                            })?
                            .ok_or_else(|| {
                                eyre::eyre!("No canonical block found at #{block_number}")
                            })
                    },
                    retry_attempts,
                    retry_base_delay,
                );

                let (block_hash, attempts) = match canonical_hash_result {
                    Ok(value) => value,
                    Err(error) if continue_on_error => {
                        skipped_blocks += 1;
                        #[cfg(feature = "tui")]
                        let reason = format!("canonical hash 없음: {error:#}");
                        if !tui {
                            eprintln!(
                                "Warning: skipping block #{block_number} (no canonical hash): {error:#}"
                            );
                        }
                        #[cfg(feature = "tui")]
                        if let Some(tx) = &tui_tx {
                            let _ = tx
                                .send(crate::tui::event::ProgressEvent::BlockSkipped {
                                    block_number,
                                    reason,
                                })
                                .await;
                        }
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                retries_performed += attempts.saturating_sub(1);

                // Read full block (header + body)
                let block_result = retry_sync(
                    || {
                        geth_reader
                            .read_block(block_number, block_hash)
                            .map_err(|e| {
                                eyre::eyre!(
                                    "Cannot read block #{block_number} ({block_hash:?}): {e}"
                                )
                            })?
                            .ok_or_else(|| {
                                eyre::eyre!(
                                    "Block #{block_number} ({block_hash:?}) missing header or body"
                                )
                            })
                    },
                    retry_attempts,
                    retry_base_delay,
                );

                let (block, attempts) = match block_result {
                    Ok(value) => value,
                    Err(error) if continue_on_error => {
                        skipped_blocks += 1;
                        #[cfg(feature = "tui")]
                        let reason = format!("블록 읽기 실패: {error:#}");
                        if !tui {
                            eprintln!(
                                "Warning: skipping block #{block_number} after read failure: {error:#}"
                            );
                        }
                        #[cfg(feature = "tui")]
                        if let Some(tx) = &tui_tx {
                            let _ = tx
                                .send(crate::tui::event::ProgressEvent::BlockSkipped {
                                    block_number,
                                    reason,
                                })
                                .await;
                        }
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                retries_performed += attempts.saturating_sub(1);

                batch_canonical.push((block_number, block_hash));
                batch.push(block);

                // Read receipts for this block (derive tx_type from block body)
                let body_ref = &batch.last().unwrap().body;
                let receipt_result = retry_sync(
                    || {
                        geth_reader
                            .read_receipts(block_number, block_hash, body_ref)
                            .map_err(|e| {
                                eyre::eyre!(
                                    "Cannot read receipts for block #{block_number}: {e}"
                                )
                            })
                    },
                    retry_attempts,
                    retry_base_delay,
                );

                match receipt_result {
                    Ok((Some(receipts), attempts)) => {
                        retries_performed += attempts.saturating_sub(1);
                        batch_receipts.push((block_hash, receipts));
                    }
                    Ok((None, attempts)) => {
                        retries_performed += attempts.saturating_sub(1);
                        // Receipts not available (e.g., ancient DB without receipt support).
                        // Not fatal — skip receipt writing for this block.
                    }
                    Err(error) if continue_on_error => {
                        if !tui {
                            eprintln!(
                                "Warning: cannot read receipts for block #{block_number}: {error:#}"
                            );
                        }
                    }
                    Err(error) => return Err(error),
                }
            }

            if batch.is_empty() {
                batch_start = batch_end + 1;
                continue;
            }

            // Write batch to RocksDB
            let (_, attempts) = retry_async(
                || async {
                    new_store.add_blocks(batch.clone()).await.wrap_err_with(|| {
                        format!(
                            "Cannot write block batch #{batch_start}..=#{batch_end} to rocksdb"
                        )
                    })
                },
                retry_attempts,
                retry_base_delay,
            )
            .await?;
            retries_performed += attempts.saturating_sub(1);

            // Write receipts for this batch
            let receipts_to_write: Vec<_> = batch_receipts.drain(..).collect();
            if !receipts_to_write.is_empty() {
                let (_, attempts) = retry_async(
                    || {
                        let receipts = receipts_to_write.clone();
                        async {
                            for (block_hash, receipts) in receipts {
                                new_store
                                    .add_receipts(block_hash, receipts)
                                    .await
                                    .wrap_err_with(|| {
                                        format!("Cannot write receipts for block {block_hash:?}")
                                    })?;
                            }
                            Ok(())
                        }
                    },
                    retry_attempts,
                    retry_base_delay,
                )
                .await?;
                retries_performed += attempts.saturating_sub(1);
            }

            // Update canonical chain for this batch
            let (last_num, last_hash) = *batch_canonical
                .last()
                .ok_or_else(|| eyre::eyre!("Empty canonical batch"))?;

            let (_, attempts) = retry_async(
                || async {
                    new_store
                        .forkchoice_update(
                            batch_canonical.clone(),
                            last_num,
                            last_hash,
                            Some(last_num),
                            Some(last_num),
                        )
                        .await
                        .wrap_err("Cannot apply forkchoice update for batch")
                },
                retry_attempts,
                retry_base_delay,
            )
            .await?;
            retries_performed += attempts.saturating_sub(1);

            let blocks_in_batch = batch_canonical.len() as u64;
            total_imported += blocks_in_batch;
            last_head = batch_canonical.last().copied();

            #[cfg(feature = "tui")]
            if let Some(tx) = &tui_tx {
                let batch_number = (batch_start - plan.start_block) / BATCH_SIZE + 1;
                let total_batches = plan.block_count().div_ceil(BATCH_SIZE);
                let _ = tx
                    .send(crate::tui::event::ProgressEvent::BatchCompleted {
                        batch_number,
                        total_batches,
                        current_block: batch_end,
                        blocks_in_batch,
                        elapsed: started_at.elapsed(),
                    })
                    .await;
            }

            batch_start = batch_end + 1;
        }

        Ok(())
    }
    .await;

    let mut verification_summary: Option<OfflineVerificationSummary> = None;
    let final_result: Result<()> = if let Err(err) = migration_result {
        Err(err)
    } else if total_imported == 0 {
        Err(eyre::eyre!(
            "Migration could not import any block in range #{}..=#{} (continue_on_error={continue_on_error})",
            plan.start_block,
            plan.end_block
        ))
    } else {
        // Phase 6: State migration (accounts, storage, code)
        if !blocks_only {
            migrate_state_to_rocksdb(
                &geth_reader,
                &new_store,
                last_source_block,
                tui,
                &started_at,
                #[cfg(feature = "tui")]
                &tui_tx,
            )
            .await?;
        }

        if verify_offline {
            let (verify_start, verify_end) = resolve_verify_range(
                plan.start_block,
                plan.end_block,
                verify_start_block,
                verify_end_block,
            )?;

            let summary = verify_geth_to_rocksdb_offline(
                &geth_reader,
                &new_store,
                verify_start,
                verify_end,
                skip_state_trie_check,
                verify_deep,
                tui,
                #[cfg(feature = "tui")]
                &tui_tx,
            )
            .await?;

            if summary.mismatches > 0 {
                Err(eyre::eyre!(
                    "Offline verification failed with {} mismatch(es) in range #{}..=#{}",
                    summary.mismatches,
                    summary.start_block,
                    summary.end_block
                ))
            } else {
                verification_summary = Some(summary);
                Ok(())
            }?
        }
        Ok(())
    };

    // Always cleanup TUI before propagating errors — ensures terminal is restored.
    #[cfg(feature = "tui")]
    {
        if let Some(tx) = tui_tx.take() {
            match &final_result {
                Ok(()) if total_imported > 0 => {
                    let _ = tx
                        .send(crate::tui::event::ProgressEvent::Completed {
                            imported_blocks: total_imported,
                            skipped_blocks,
                            elapsed: started_at.elapsed(),
                            retries_performed,
                        })
                        .await;
                }
                Ok(()) => {
                    let _ = tx
                        .send(crate::tui::event::ProgressEvent::Error {
                            message: format!(
                                "Migration could not import any block in range #{}..=#{} (continue_on_error={continue_on_error})",
                                plan.start_block, plan.end_block
                            ),
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(crate::tui::event::ProgressEvent::Error {
                            message: format!("{e:#}"),
                        })
                        .await;
                }
            }
            // Drop tx — TUI will detect channel close and wait for 'q'.
            drop(tx);
        }
        if let Some(handle) = tui_handle.take()
            && let Err(join_error) = handle.await
        {
            eprintln!("TUI task join failed: {join_error}");
        }
    }

    // Propagate migration/verification errors after TUI is cleaned up
    final_result?;

    let (head_block_number, _head_block_hash) =
        last_head.ok_or_else(|| eyre::eyre!("Cannot determine migrated chain head"))?;

    if skipped_blocks > 0 && !tui {
        eprintln!(
            "Warning: migration completed with {skipped_blocks} skipped block(s) due to --continue-on-error"
        );
    }

    if !tui {
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "completed",
            phase: "execution",
            source_head: last_source_block,
            target_head: head_block_number,
            plan: Some(plan),
            dry_run: false,
            imported_blocks: total_imported,
            skipped_blocks,
            elapsed_ms: elapsed_ms(started_at),
            retry_attempts,
            retries_performed,
        };
        emit_report(&report, json, report_file)?;
        if let Some(summary) = verification_summary {
            println!(
                "Offline verification passed: checked {} block(s) in #{}..=#{} (body checks passed: {}).",
                summary.checked_blocks, summary.start_block, summary.end_block,
                summary.body_checks_passed
            );
        }
    }

    // ethrex-ready startup compatibility check
    if ethrex_ready {
        let ready_report = check_ethrex_ready(&new_store, target_storage).await;
        if json {
            let json_str = serde_json::to_string_pretty(&ready_report).unwrap_or_default();
            eprintln!("{json_str}");
        } else {
            eprintln!(
                "[ethrex-ready] {}",
                if ready_report.ethrex_ready {
                    "PASS"
                } else {
                    "FAIL"
                }
            );
            for (name, result) in [
                ("metadata_json", &ready_report.checks.metadata_json),
                (
                    "latest_block_number",
                    &ready_report.checks.latest_block_number,
                ),
                ("latest_header", &ready_report.checks.latest_header),
                ("chain_config", &ready_report.checks.chain_config),
                ("genesis_block", &ready_report.checks.genesis_block),
                ("state_root_valid", &ready_report.checks.state_root_valid),
            ] {
                eprintln!("  {name}: {result}");
            }
        }
        if !ready_report.ethrex_ready {
            return Err(eyre::eyre!("ethrex-ready check failed"));
        }
    }

    Ok(())
}

/// Migrates Geth account/storage/code snapshots into ethrex's Merkle Patricia Trie.
///
/// Reads all accounts from Geth's snapshot layer (`"a" + hash` prefix),
/// builds per-account storage tries and a global state trie, then commits
/// all trie nodes to ethrex's RocksDB trie tables.
///
/// The computed state root is verified against the head block header.
#[allow(clippy::too_many_arguments)]
async fn migrate_state_to_rocksdb(
    geth_reader: &crate::readers::geth_db::GethBlockReader,
    store: &ethrex_storage::Store,
    head_block_number: u64,
    tui: bool,
    started_at: &Instant,
    #[cfg(feature = "tui")] tui_tx: &Option<
        tokio::sync::mpsc::Sender<crate::tui::event::ProgressEvent>,
    >,
) -> Result<()> {
    use ethrex_common::types::{AccountState, Code};
    use ethrex_common::U256;
    use ethrex_rlp::encode::RLPEncode;
    use ethrex_trie::EMPTY_TRIE_HASH;

    const STATE_BATCH_SIZE: u64 = 10_000;
    const KECCAK_EMPTY_BYTES: [u8; 32] = [
        0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7,
        0x03, 0xc0, 0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04,
        0x5d, 0x85, 0xa4, 0x70,
    ];

    // Read head block header for state_root verification
    let head_header = store
        .get_block_header(head_block_number)?
        .ok_or_else(|| eyre::eyre!("Head block #{head_block_number} header not found"))?;
    let expected_state_root = head_header.state_root;

    // Count accounts for progress tracking
    let total_accounts = geth_reader.count_account_snapshots().unwrap_or(0);

    if !tui {
        eprintln!(
            "[state] Starting state migration ({total_accounts} accounts, expected root={expected_state_root:?})"
        );
    }

    #[cfg(feature = "tui")]
    if let Some(tx) = tui_tx {
        let _ = tx
            .send(crate::tui::event::ProgressEvent::StatePhaseStarted { total_accounts })
            .await;
    }

    // Open a fresh state trie (BackendTrieDB handles DB writes on commit)
    let mut state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;

    let mut processed_accounts: u64 = 0;
    let mut total_storage_slots: u64 = 0;
    let mut total_code_entries: u64 = 0;
    let mut code_hashes_written: std::collections::HashSet<ethrex_common::H256> =
        std::collections::HashSet::new();

    let account_iter = geth_reader
        .iter_account_snapshots()
        .map_err(|e| eyre::eyre!("Account snapshot iteration failed: {e}"))?;

    for (account_hash, slim_account) in account_iter {
        // 1. Build storage trie for this account
        let storage_root = if slim_account.storage_root != *EMPTY_TRIE_HASH {
            // open_direct_storage_trie uses BackendTrieDB with account_hash prefix
            // so commit() writes to STORAGE_TRIE_NODES with correct prefix
            let mut storage_trie =
                store.open_direct_storage_trie(account_hash, *EMPTY_TRIE_HASH)?;

            if let Ok(storage_iter) = geth_reader.iter_storage_snapshots(&account_hash) {
                for (slot_hash, raw_value) in storage_iter {
                    let value = U256::from_big_endian(&raw_value);
                    if !value.is_zero() {
                        storage_trie.insert(
                            slot_hash.as_bytes().to_vec(),
                            value.encode_to_vec(),
                        )?;
                        total_storage_slots += 1;
                    }
                }
            }

            // Commit writes storage trie nodes to DB via BackendTrieDB
            storage_trie.commit()?;
            storage_trie.hash_no_commit()
        } else {
            *EMPTY_TRIE_HASH
        };

        // 2. Write contract code
        if slim_account.code_hash.0 != KECCAK_EMPTY_BYTES
            && !code_hashes_written.contains(&slim_account.code_hash)
            && let Ok(Some(bytecode)) = geth_reader.read_code(slim_account.code_hash)
        {
            let code = Code::from_bytecode_unchecked(
                bytecode.into(),
                slim_account.code_hash,
            );
            store.add_account_code(code).await?;
            code_hashes_written.insert(slim_account.code_hash);
            total_code_entries += 1;
        }

        // 3. Insert account into state trie
        let account_state = AccountState {
            nonce: slim_account.nonce,
            balance: slim_account.balance,
            storage_root,
            code_hash: slim_account.code_hash,
        };
        state_trie.insert(
            account_hash.as_bytes().to_vec(),
            account_state.encode_to_vec(),
        )?;

        processed_accounts += 1;

        // 4. Progress reporting every STATE_BATCH_SIZE accounts
        if processed_accounts.is_multiple_of(STATE_BATCH_SIZE) {
            if !tui {
                let pct = (processed_accounts as f64 * 100.0) / total_accounts.max(1) as f64;
                eprintln!(
                    "[state] {processed_accounts}/{total_accounts} ({pct:.1}%) accounts, {total_storage_slots} slots, {total_code_entries} codes"
                );
            }

            #[cfg(feature = "tui")]
            if let Some(tx) = tui_tx {
                let _ = tx
                    .send(crate::tui::event::ProgressEvent::AccountBatchCompleted {
                        processed: processed_accounts,
                        total: total_accounts,
                        elapsed: started_at.elapsed(),
                    })
                    .await;
            }
        }
    }

    // Commit the entire state trie to DB
    state_trie.commit()?;
    let computed_state_root = state_trie.hash_no_commit();

    // Verify state root
    if computed_state_root != expected_state_root {
        eprintln!(
            "[state] WARNING: State root mismatch! computed={computed_state_root:?} expected={expected_state_root:?}"
        );
        eprintln!("[state] The migration completed but the state trie may be inconsistent.");
    } else if !tui {
        eprintln!("[state] State root verified: {computed_state_root:?}");
    }

    if !tui {
        eprintln!(
            "[state] Completed: {processed_accounts} accounts, {total_storage_slots} storage slots, {total_code_entries} code entries"
        );
    }

    #[cfg(feature = "tui")]
    if let Some(tx) = tui_tx {
        let _ = tx
            .send(crate::tui::event::ProgressEvent::StatePhaseCompleted {
                accounts: processed_accounts,
                storage_slots: total_storage_slots,
                code_entries: total_code_entries,
                accounts_without_preimage: 0,
                slots_without_preimage: 0,
                elapsed: started_at.elapsed(),
            })
            .await;
    }

    Ok(())
}

/// Migrates Geth chaindata (Pebble) to py-ethclient LMDB format.
///
/// Two-phase migration:
/// - Phase 1: Block data (headers, bodies, canonical, tx_index)
/// - Phase 2: State data (accounts, storage, code) — skipped if `blocks_only` is true
#[allow(clippy::too_many_arguments)]
async fn migrate_geth_to_lmdb(
    geth_chaindata: &Path,
    lmdb_path: &Path,
    dry_run: bool,
    json: bool,
    blocks_only: bool,
    map_size_gb: u32,
    continue_on_error: bool,
    verify_offline: bool,
    verify_start_block: Option<u64>,
    verify_end_block: Option<u64>,
    report_file: Option<&Path>,
    tui: bool,
) -> Result<()> {
    use crate::detect::{GethDbType, detect_geth_db_type};
    use crate::readers::geth_db::decode_stored_receipts;
    use crate::readers::open_geth_block_reader;
    use crate::writers::lmdb::LmdbWriter;
    use ethrex_common::types::{ReceiptWithBloom, TxType, bloom_from_logs};
    use ethrex_rlp::encode::RLPEncode;
    use tiny_keccak::{Hasher, Keccak};

    const BATCH_SIZE: u64 = 1_000;
    const STATE_BATCH_SIZE: usize = 10_000;

    let started_at = Instant::now();

    // Phase 1: Detect and open Geth reader
    let db_type = detect_geth_db_type(geth_chaindata).wrap_err("Geth DB 타입 감지 실패")?;

    let db_type_str = match db_type {
        GethDbType::Pebble => "Pebble",
        GethDbType::LevelDB => "LevelDB",
        GethDbType::Unknown => "Unknown",
    };

    if json {
        eprintln!(
            r#"{{"phase":"detect","db_type":"{db_type_str}","chaindata_path":"{}"}}"#,
            geth_chaindata.display()
        );
    }

    let geth_reader = open_geth_block_reader(geth_chaindata)
        .map_err(|e| eyre::eyre!("Geth chaindata 열기 실패: {e}"))?;

    // Read Geth head block number
    let head_hash = geth_reader
        .read_head_block_hash()
        .map_err(|e| eyre::eyre!("Geth head 블록 해시 읽기 실패: {e}"))?;

    let last_source_block = geth_reader
        .read_block_number(head_hash)
        .map_err(|e| eyre::eyre!("Geth head 블록 번호 읽기 실패: {e}"))?;

    let plan = MigrationPlan {
        start_block: 0,
        end_block: last_source_block,
    };

    if json {
        eprintln!(
            r#"{{"phase":"plan","source_head":{},"block_count":{},"blocks_only":{}}}"#,
            last_source_block,
            plan.block_count(),
            blocks_only
        );
    }

    if dry_run {
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "planned",
            phase: "planning",
            source_head: last_source_block,
            target_head: 0,
            plan: Some(plan),
            dry_run: true,
            imported_blocks: 0,
            skipped_blocks: 0,
            elapsed_ms: elapsed_ms(started_at),
            retry_attempts: MAX_RETRY_ATTEMPTS,
            retries_performed: 0,
        };
        emit_report(&report, json, report_file)?;
        return Ok(());
    }

    // Create LMDB writer
    let map_size_bytes = (map_size_gb as usize) * 1024 * 1024 * 1024;
    let lmdb = LmdbWriter::create(lmdb_path, map_size_bytes)
        .map_err(|e| eyre::eyre!("LMDB 생성 실패 ({}): {e}", lmdb_path.display()))?;

    // Optionally start TUI dashboard
    #[cfg(feature = "tui")]
    let (mut tui_tx, mut tui_handle): (
        Option<tokio::sync::mpsc::Sender<crate::tui::event::ProgressEvent>>,
        Option<tokio::task::JoinHandle<()>>,
    ) = if tui {
        let (tx, rx) = tokio::sync::mpsc::channel::<crate::tui::event::ProgressEvent>(256);
        let handle = tokio::spawn(crate::tui::run_tui(rx));
        (Some(tx), Some(handle))
    } else {
        (None, None)
    };

    #[cfg(not(feature = "tui"))]
    {
        let _ = tui;
    }

    #[cfg(feature = "tui")]
    if let Some(tx) = &tui_tx {
        let _ = tx
            .send(crate::tui::event::ProgressEvent::Init {
                source_path: geth_chaindata.display().to_string(),
                target_path: lmdb_path.display().to_string(),
                db_type: db_type_str.to_string(),
                start_block: plan.start_block,
                end_block: plan.end_block,
            })
            .await;
    }

    // --- Phase 1: Block migration ---
    let mut total_imported: u64 = 0;
    let mut skipped_blocks = 0u64;

    let migration_result: Result<()> = async {
        let mut batch_start = plan.start_block;
        while batch_start <= plan.end_block {
            let batch_end = (batch_start + BATCH_SIZE - 1).min(plan.end_block);

            let mut wtxn = lmdb
                .write_txn()
                .map_err(|e| eyre::eyre!("LMDB 쓰기 트랜잭션 시작 실패: {e}"))?;

            let mut blocks_in_batch: u64 = 0;

            for block_number in batch_start..=batch_end {
                let canonical_hash = match geth_reader.read_canonical_hash(block_number) {
                    Ok(Some(h)) => h,
                    Ok(None) if continue_on_error => {
                        skipped_blocks += 1;
                        continue;
                    }
                    Ok(None) => {
                        return Err(eyre::eyre!("블록 #{block_number}의 canonical 해시 없음"));
                    }
                    Err(e) if continue_on_error => {
                        skipped_blocks += 1;
                        eprintln!("경고: 블록 #{block_number} canonical 해시 읽기 실패: {e}");
                        continue;
                    }
                    Err(e) => {
                        return Err(eyre::eyre!(
                            "블록 #{block_number} canonical 해시 읽기 실패: {e}"
                        ));
                    }
                };

                let block_hash_bytes: [u8; 32] = canonical_hash.0;

                // Read header and body
                let header = match geth_reader.read_block_header(block_number, canonical_hash) {
                    Ok(Some(h)) => h,
                    Ok(None) if continue_on_error => {
                        skipped_blocks += 1;
                        continue;
                    }
                    Ok(None) => {
                        return Err(eyre::eyre!("블록 #{block_number} 헤더 없음"));
                    }
                    Err(e) if continue_on_error => {
                        skipped_blocks += 1;
                        eprintln!("경고: 블록 #{block_number} 헤더 읽기 실패: {e}");
                        continue;
                    }
                    Err(e) => {
                        return Err(eyre::eyre!("블록 #{block_number} 헤더 읽기 실패: {e}"));
                    }
                };

                let body = match geth_reader.read_block_body(block_number, canonical_hash) {
                    Ok(Some(b)) => b,
                    Ok(None) if continue_on_error => {
                        skipped_blocks += 1;
                        continue;
                    }
                    Ok(None) => {
                        return Err(eyre::eyre!("블록 #{block_number} 바디 없음"));
                    }
                    Err(e) if continue_on_error => {
                        skipped_blocks += 1;
                        eprintln!("경고: 블록 #{block_number} 바디 읽기 실패: {e}");
                        continue;
                    }
                    Err(e) => {
                        return Err(eyre::eyre!("블록 #{block_number} 바디 읽기 실패: {e}"));
                    }
                };

                // Encode header and body as RLP
                let header_rlp = header.encode_to_vec();
                let body_rlp = body.encode_to_vec();

                // Write to LMDB
                lmdb.put_header(&mut wtxn, &block_hash_bytes, &header_rlp)
                    .map_err(|e| eyre::eyre!("LMDB header 쓰기 실패 #{block_number}: {e}"))?;
                lmdb.put_body(&mut wtxn, &block_hash_bytes, &body_rlp)
                    .map_err(|e| eyre::eyre!("LMDB body 쓰기 실패 #{block_number}: {e}"))?;
                lmdb.put_canonical(&mut wtxn, block_number, &block_hash_bytes)
                    .map_err(|e| eyre::eyre!("LMDB canonical 쓰기 실패 #{block_number}: {e}"))?;
                lmdb.put_header_number(&mut wtxn, block_number, &block_hash_bytes)
                    .map_err(|e| {
                        eyre::eyre!("LMDB header_numbers 쓰기 실패 #{block_number}: {e}")
                    })?;

                // Write tx_index entries
                for (tx_idx, tx) in body.transactions.iter().enumerate() {
                    let tx_hash_h256 = tx.hash();
                    let tx_hash_bytes: [u8; 32] = tx_hash_h256.0;
                    lmdb.put_tx_index(&mut wtxn, &tx_hash_bytes, &block_hash_bytes, tx_idx as u32)
                        .map_err(|e| {
                            eyre::eyre!("LMDB tx_index 쓰기 실패 #{block_number} tx {tx_idx}: {e}")
                        })?;
                }

                // Write receipts (P0): read from Geth, recompute bloom, encode for py-ethclient
                match geth_reader.read_raw_receipts(block_number, canonical_hash) {
                    Ok(Some(raw_receipts)) => match decode_stored_receipts(&raw_receipts) {
                        Ok(stored_receipts) => {
                            let mut receipt_inner_encodings = Vec::new();
                            for (i, sr) in stored_receipts.iter().enumerate() {
                                let tx_type = body
                                    .transactions
                                    .get(i)
                                    .map(|tx| tx.tx_type())
                                    .unwrap_or(TxType::Legacy);
                                let bloom = bloom_from_logs(&sr.logs);
                                let rwb = ReceiptWithBloom {
                                    tx_type,
                                    succeeded: sr.succeeded,
                                    cumulative_gas_used: sr.cumulative_gas_used,
                                    bloom,
                                    logs: sr.logs.clone(),
                                };
                                receipt_inner_encodings.push(rwb.encode_inner());
                            }
                            let mut receipts_rlp = Vec::new();
                            let items: Vec<&[u8]> = receipt_inner_encodings
                                .iter()
                                .map(|v| v.as_slice())
                                .collect();
                            encode_rlp_receipt_list(&items, &mut receipts_rlp);
                            lmdb.put_receipts(&mut wtxn, &block_hash_bytes, &receipts_rlp)
                                .map_err(|e| {
                                    eyre::eyre!("LMDB receipts 쓰기 실패 #{block_number}: {e}")
                                })?;
                        }
                        Err(e) if continue_on_error => {
                            eprintln!("경고: 블록 #{block_number} receipt 디코딩 실패: {e}");
                        }
                        Err(e) => {
                            return Err(eyre::eyre!(
                                "블록 #{block_number} receipt 디코딩 실패: {e}"
                            ));
                        }
                    },
                    Ok(None) => {} // No receipts (e.g., genesis block)
                    Err(e) if continue_on_error => {
                        eprintln!("경고: 블록 #{block_number} receipt 읽기 실패: {e}");
                    }
                    Err(e) => {
                        return Err(eyre::eyre!("블록 #{block_number} receipt 읽기 실패: {e}"));
                    }
                }

                blocks_in_batch += 1;
            }

            // Set latest_block in meta
            lmdb.set_latest_block(&mut wtxn, batch_end)
                .map_err(|e| eyre::eyre!("LMDB meta 쓰기 실패: {e}"))?;

            wtxn.commit()
                .map_err(|e| eyre::eyre!("LMDB 트랜잭션 커밋 실패: {e}"))?;

            total_imported += blocks_in_batch;

            #[cfg(feature = "tui")]
            if let Some(tx) = &tui_tx {
                let batch_number = (batch_start - plan.start_block) / BATCH_SIZE + 1;
                let total_batches = plan.block_count().div_ceil(BATCH_SIZE);
                let _ = tx
                    .send(crate::tui::event::ProgressEvent::BatchCompleted {
                        batch_number,
                        total_batches,
                        current_block: batch_end,
                        blocks_in_batch,
                        elapsed: started_at.elapsed(),
                    })
                    .await;
            }

            batch_start = batch_end + 1;
        }

        // --- Phase 2: State migration ---
        if !blocks_only {
            // Count accounts for progress tracking
            #[cfg(feature = "tui")]
            let total_accounts = geth_reader.count_account_snapshots().unwrap_or(0);

            #[cfg(feature = "tui")]
            if let Some(tx) = &tui_tx {
                let _ = tx
                    .send(crate::tui::event::ProgressEvent::StatePhaseStarted { total_accounts })
                    .await;
            }

            let mut processed_accounts: u64 = 0;
            let mut total_storage_slots: u64 = 0;
            let mut total_code_entries: u64 = 0;
            let mut accounts_without_preimage: u64 = 0;
            let mut slots_without_preimage: u64 = 0;
            let mut code_hashes_written: std::collections::HashSet<[u8; 32]> =
                std::collections::HashSet::new();

            // Helper: compute keccak256
            let keccak256 = |data: &[u8]| -> [u8; 32] {
                let mut hasher = Keccak::v256();
                let mut output = [0u8; 32];
                hasher.update(data);
                hasher.finalize(&mut output);
                output
            };

            let account_iter = geth_reader
                .iter_account_snapshots()
                .map_err(|e| eyre::eyre!("account 스냅샷 순회 실패: {e}"))?;

            let mut wtxn = lmdb
                .write_txn()
                .map_err(|e| eyre::eyre!("LMDB 상태 트랜잭션 시작 실패: {e}"))?;

            for (account_hash, slim_account) in account_iter {
                let account_hash_bytes: [u8; 32] = account_hash.0;
                let full_rlp = slim_account.rlp_encode_full();

                // Always write to snap_accounts (hash-keyed)
                lmdb.put_snap_account(&mut wtxn, &account_hash_bytes, &full_rlp)
                    .map_err(|e| eyre::eyre!("LMDB snap_accounts 쓰기 실패: {e}"))?;

                // Try to resolve preimage for address-keyed accounts DB
                if let Ok(Some(preimage)) = geth_reader.read_preimage(account_hash) {
                    if preimage.len() == 20 {
                        let address: [u8; 20] = preimage.try_into().unwrap();
                        // Verify hash matches
                        let computed_hash = keccak256(&address);
                        if computed_hash == account_hash_bytes {
                            lmdb.put_account(&mut wtxn, &address, &full_rlp)
                                .map_err(|e| eyre::eyre!("LMDB accounts 쓰기 실패: {e}"))?;

                            // Migrate storage for this account (address-keyed)
                            if let Ok(storage_iter) =
                                geth_reader.iter_storage_snapshots(&account_hash)
                            {
                                for (slot_hash, raw_value) in storage_iter {
                                    let slot_hash_bytes: [u8; 32] = slot_hash.0;

                                    // Write to snap_storage (hash-keyed)
                                    lmdb.put_snap_storage(
                                        &mut wtxn,
                                        &account_hash_bytes,
                                        &slot_hash_bytes,
                                        &raw_value,
                                    )
                                    .map_err(|e| eyre::eyre!("LMDB snap_storage 쓰기 실패: {e}"))?;

                                    // Try to resolve slot preimage for address-keyed storage
                                    if let Ok(Some(slot_preimage)) =
                                        geth_reader.read_preimage(slot_hash)
                                        && slot_preimage.len() == 32
                                    {
                                        let slot: [u8; 32] = slot_preimage.try_into().unwrap();
                                        lmdb.put_storage(&mut wtxn, &address, &slot, &raw_value)
                                            .map_err(|e| {
                                                eyre::eyre!("LMDB storage 쓰기 실패: {e}")
                                            })?;
                                        // P1: original_storage (same data at migration time)
                                        lmdb.put_original_storage(&mut wtxn, &address, &slot, &raw_value)
                                            .map_err(|e| {
                                                eyre::eyre!("LMDB original_storage 쓰기 실패: {e}")
                                            })?;
                                    } else {
                                        slots_without_preimage += 1;
                                    }

                                    total_storage_slots += 1;
                                }
                            }
                        }
                    } else {
                        accounts_without_preimage += 1;
                    }
                } else {
                    accounts_without_preimage += 1;
                    // No preimage — only write snap_storage (hash-keyed)
                    if let Ok(storage_iter) = geth_reader.iter_storage_snapshots(&account_hash) {
                        for (slot_hash, raw_value) in storage_iter {
                            let slot_hash_bytes: [u8; 32] = slot_hash.0;
                            lmdb.put_snap_storage(
                                &mut wtxn,
                                &account_hash_bytes,
                                &slot_hash_bytes,
                                &raw_value,
                            )
                            .map_err(|e| eyre::eyre!("LMDB snap_storage 쓰기 실패: {e}"))?;
                            slots_without_preimage += 1;
                            total_storage_slots += 1;
                        }
                    }
                }

                // Write code if not already written
                let code_hash_bytes: [u8; 32] = slim_account.code_hash.0;
                if code_hash_bytes != KECCAK_EMPTY_BYTES
                    && !code_hashes_written.contains(&code_hash_bytes)
                    && let Ok(Some(bytecode)) = geth_reader.read_code(slim_account.code_hash)
                {
                    lmdb.put_code(&mut wtxn, &code_hash_bytes, &bytecode)
                        .map_err(|e| eyre::eyre!("LMDB code 쓰기 실패: {e}"))?;
                    code_hashes_written.insert(code_hash_bytes);
                    total_code_entries += 1;
                }

                processed_accounts += 1;

                // Commit in batches to avoid oversized transactions
                if processed_accounts.is_multiple_of(STATE_BATCH_SIZE as u64) {
                    wtxn.commit()
                        .map_err(|e| eyre::eyre!("LMDB 상태 커밋 실패: {e}"))?;

                    #[cfg(feature = "tui")]
                    if let Some(tx) = &tui_tx {
                        let _ = tx
                            .send(crate::tui::event::ProgressEvent::AccountBatchCompleted {
                                processed: processed_accounts,
                                total: total_accounts,
                                elapsed: started_at.elapsed(),
                            })
                            .await;
                    }

                    wtxn = lmdb
                        .write_txn()
                        .map_err(|e| eyre::eyre!("LMDB 상태 트랜잭션 재시작 실패: {e}"))?;
                }
            }

            // P3: Write snap_progress metadata
            let snap_progress = format!(
                r#"{{"done":true,"synced_accounts":{},"synced_storage_slots":{},"synced_bytecodes":{}}}"#,
                processed_accounts, total_storage_slots, total_code_entries
            );
            lmdb.set_snap_progress(&mut wtxn, snap_progress.as_bytes())
                .map_err(|e| eyre::eyre!("LMDB snap_progress 쓰기 실패: {e}"))?;

            // Final commit for remaining state data
            wtxn.commit()
                .map_err(|e| eyre::eyre!("LMDB 최종 상태 커밋 실패: {e}"))?;

            // P2: Warn about preimage misses
            if accounts_without_preimage > 0 || slots_without_preimage > 0 {
                eprintln!(
                    "경고: preimage 누락 — 계정: {accounts_without_preimage}, 슬롯: {slots_without_preimage}"
                );
            }

            #[cfg(feature = "tui")]
            if let Some(tx) = &tui_tx {
                let _ = tx
                    .send(crate::tui::event::ProgressEvent::StatePhaseCompleted {
                        accounts: processed_accounts,
                        storage_slots: total_storage_slots,
                        code_entries: total_code_entries,
                        accounts_without_preimage,
                        slots_without_preimage,
                        elapsed: started_at.elapsed(),
                    })
                    .await;
            }
        }

        Ok(())
    }
    .await;

    let mut verification_summary: Option<OfflineVerificationSummary> = None;
    let final_result: Result<()> = if let Err(err) = migration_result {
        Err(err)
    } else if total_imported == 0 {
        Err(eyre::eyre!("마이그레이션에서 블록을 가져오지 못했습니다."))
    } else {
        if verify_offline {
            let (verify_start, verify_end) = resolve_verify_range(
                plan.start_block,
                plan.end_block,
                verify_start_block,
                verify_end_block,
            )?;
            #[cfg(feature = "tui")]
            if let Some(tx) = &tui_tx {
                let verify_total = verify_end.saturating_sub(verify_start) + 1;
                let _ = tx
                    .send(crate::tui::event::ProgressEvent::VerificationStarted {
                        start_block: verify_start,
                        end_block: verify_end,
                        total_blocks: verify_total,
                        state_trie_check: false,
                    })
                    .await;
            }

            let summary = verify_geth_to_lmdb_offline(
                &geth_reader,
                &lmdb,
                verify_start,
                verify_end,
                tui,
                #[cfg(feature = "tui")]
                &tui_tx,
            )?;

            #[cfg(feature = "tui")]
            if let Some(tx) = &tui_tx {
                let _ = tx
                    .send(crate::tui::event::ProgressEvent::VerificationCompleted {
                        checked: summary.checked_blocks,
                        mismatches: summary.mismatches,
                        elapsed: started_at.elapsed(),
                    })
                    .await;
            }

            if summary.mismatches > 0 {
                Err(eyre::eyre!(
                    "Offline verification failed with {} mismatch(es) in range #{}..=#{}",
                    summary.mismatches,
                    summary.start_block,
                    summary.end_block
                ))
            } else {
                verification_summary = Some(summary);
                Ok(())
            }?
        }
        Ok(())
    };

    // Cleanup TUI
    #[cfg(feature = "tui")]
    {
        if let Some(tx) = tui_tx.take() {
            match &final_result {
                Ok(()) if total_imported > 0 => {
                    let _ = tx
                        .send(crate::tui::event::ProgressEvent::Completed {
                            imported_blocks: total_imported,
                            skipped_blocks,
                            elapsed: started_at.elapsed(),
                            retries_performed: 0,
                        })
                        .await;
                }
                Ok(()) => {
                    let _ = tx
                        .send(crate::tui::event::ProgressEvent::Error {
                            message: "마이그레이션에서 블록을 가져오지 못했습니다.".into(),
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(crate::tui::event::ProgressEvent::Error {
                            message: format!("{e:#}"),
                        })
                        .await;
                }
            }
            drop(tx);
        }
        if let Some(handle) = tui_handle.take() {
            let _ = handle.await;
        }
    }

    final_result?;

    if !tui {
        let report = MigrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            status: "completed",
            phase: "execution",
            source_head: last_source_block,
            target_head: last_source_block,
            plan: Some(plan),
            dry_run: false,
            imported_blocks: total_imported,
            skipped_blocks,
            elapsed_ms: elapsed_ms(started_at),
            retry_attempts: MAX_RETRY_ATTEMPTS,
            retries_performed: 0,
        };
        emit_report(&report, json, report_file)?;
        if let Some(summary) = verification_summary {
            println!(
                "Offline verification passed: checked {} block(s) in #{}..=#{} (body checks passed: {}).",
                summary.checked_blocks, summary.start_block, summary.end_block,
                summary.body_checks_passed
            );
        }
    }

    Ok(())
}

/// Encodes a list of pre-encoded receipt items into an RLP list.
///
/// Each item is already encoded (e.g., via `ReceiptWithBloom::encode_inner()`).
/// This wraps them in an outer RLP list: `0xc0+len | items_concat` or
/// `0xf7+len_bytes | len | items_concat`.
fn encode_rlp_receipt_list(items: &[&[u8]], out: &mut Vec<u8>) {
    let total_len: usize = items.iter().map(|i| i.len()).sum();
    if total_len <= 55 {
        out.push(0xc0 + total_len as u8);
    } else {
        let len_bytes = total_len.to_be_bytes();
        let start = len_bytes.iter().position(|&b| b != 0).unwrap_or(7);
        let len_payload = &len_bytes[start..];
        out.push(0xf7 + len_payload.len() as u8);
        out.extend_from_slice(len_payload);
    }
    for item in items {
        out.extend_from_slice(item);
    }
}

/// keccak256 of empty bytes (constant for comparison)
const KECCAK_EMPTY_BYTES: [u8; 32] = [
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
];

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
        MAX_RETRY_ATTEMPTS, MigrationErrorReport, MigrationPlan, MigrationReport,
        REPORT_SCHEMA_VERSION, RetryFailure, append_report_line, build_migration_error_report,
        build_migration_plan, classify_error_from_message, classify_error_from_report,
        classify_io_error_kind, compute_backoff_delay, emit_error_report, emit_report, retry_async,
        retry_sync,
    };
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
        let plan = build_migration_plan(100, 100, None).expect("planning should succeed");
        assert!(plan.is_none());
    }

    #[test]
    fn no_plan_when_target_is_ahead() {
        let plan = build_migration_plan(101, 100, None).expect("planning should succeed");
        assert!(plan.is_none());
    }

    #[test]
    fn builds_plan_when_source_is_ahead() {
        let plan = build_migration_plan(12, 20, None)
            .expect("planning should succeed")
            .expect("plan should exist");
        assert_eq!(plan.start_block, 13);
        assert_eq!(plan.end_block, 20);
        assert_eq!(plan.block_count(), 8);
    }

    #[test]
    fn builds_plan_from_resume_from_block_override() {
        let plan = build_migration_plan(12, 20, Some(15))
            .expect("planning should succeed")
            .expect("plan should exist");
        assert_eq!(plan.start_block, 15);
        assert_eq!(plan.end_block, 20);
        assert_eq!(plan.block_count(), 6);
    }

    #[test]
    fn rejects_resume_from_block_not_greater_than_target_head() {
        let result = build_migration_plan(12, 20, Some(12));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_resume_from_block_above_source_head() {
        let result = build_migration_plan(12, 20, Some(21));
        assert!(result.is_err());
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
