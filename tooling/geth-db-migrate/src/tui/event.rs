use std::time::Duration;

/// Migration progress events sent from the migration loop to the TUI renderer.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// Emitted once before the first batch starts.
    Init {
        source_path: String,
        target_path: String,
        db_type: String,
        start_block: u64,
        end_block: u64,
    },
    /// Emitted after each batch is successfully written.
    BatchCompleted {
        batch_number: u64,
        total_batches: u64,
        current_block: u64,
        blocks_in_batch: u64,
        elapsed: Duration,
    },
    /// Emitted when a block is skipped due to --continue-on-error.
    BlockSkipped { block_number: u64, reason: String },
    /// Emitted when migration finishes successfully.
    Completed {
        imported_blocks: u64,
        skipped_blocks: u64,
        elapsed: Duration,
        retries_performed: u32,
    },
    /// Emitted when migration terminates with a fatal error.
    Error { message: String },
    /// Emitted when state migration phase starts.
    StatePhaseStarted { total_accounts: u64 },
    /// Emitted after a batch of accounts is processed.
    AccountBatchCompleted {
        processed: u64,
        total: u64,
        elapsed: Duration,
    },
    /// Emitted when state migration phase completes.
    StatePhaseCompleted {
        accounts: u64,
        storage_slots: u64,
        code_entries: u64,
        accounts_without_preimage: u64,
        slots_without_preimage: u64,
        elapsed: Duration,
    },
    /// Emitted when offline verification phase starts.
    VerificationStarted {
        start_block: u64,
        end_block: u64,
        total_blocks: u64,
        state_trie_check: bool,
    },
    /// Emitted as offline verification progresses.
    VerificationProgress {
        checked: u64,
        total: u64,
        mismatches: u64,
        elapsed: Duration,
    },
    /// Emitted when a mismatch is found during verification.
    VerificationMismatch { block_number: u64, reason: String },
    /// Emitted when offline verification finishes.
    VerificationCompleted {
        checked: u64,
        mismatches: u64,
        elapsed: Duration,
    },
}
