//! Sentinel background service for block monitoring.
//!
//! `SentinelService` runs a dedicated background thread that receives committed
//! blocks via a channel, applies the pre-filter heuristics, and deep-analyzes
//! any suspicious transactions using the Autopsy Lab pipeline.
//!
//! The service implements `ethrex_blockchain::BlockObserver` so it can be plugged
//! directly into the `Blockchain` struct without creating a circular dependency.

use std::sync::Mutex;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use ethrex_blockchain::BlockObserver;
use ethrex_common::types::{Block, Receipt};
use ethrex_storage::Store;

use super::analyzer::DeepAnalyzer;
use super::pre_filter::PreFilter;
use super::types::{AnalysisConfig, SentinelAlert, SentinelConfig};

/// Message sent from the block processing pipeline to the sentinel worker.
enum SentinelMessage {
    /// A new block has been committed to the store.
    BlockCommitted {
        block: Box<Block>,
        receipts: Vec<Receipt>,
    },
    /// Graceful shutdown request.
    Shutdown,
}

/// Callback trait for consuming alerts produced by the sentinel.
///
/// Implementations might log to stderr, write to a JSONL file, or POST to a webhook.
pub trait AlertHandler: Send + 'static {
    fn on_alert(&self, alert: SentinelAlert);
}

/// Default alert handler that logs to stderr.
pub struct LogAlertHandler;

impl AlertHandler for LogAlertHandler {
    fn on_alert(&self, alert: SentinelAlert) {
        eprintln!(
            "[SENTINEL ALERT] block={} tx_index={} priority={:?} summary={}",
            alert.block_number, alert.tx_index, alert.alert_priority, alert.summary
        );
    }
}

/// Background sentinel service that monitors committed blocks for suspicious activity.
///
/// The service uses a single background thread connected via an `mpsc` channel.
/// `on_block_committed()` is non-blocking: it sends block data to the channel
/// and returns immediately, ensuring zero overhead on the block processing hot path.
///
/// The worker thread runs the two-stage pipeline:
/// 1. **Pre-filter** (receipt-based heuristics, ~10-50μs per TX)
/// 2. **Deep analysis** (opcode replay + attack classification, only for suspicious TXs)
pub struct SentinelService {
    sender: Mutex<mpsc::Sender<SentinelMessage>>,
    worker_handle: Mutex<Option<JoinHandle<()>>>,
}

impl SentinelService {
    /// Create a new sentinel service with a background worker thread.
    ///
    /// The `store` is used by the deep analyzer to replay suspicious transactions.
    /// The `alert_handler` receives confirmed alerts.
    pub fn new(
        store: Store,
        config: SentinelConfig,
        analysis_config: AnalysisConfig,
        alert_handler: Box<dyn AlertHandler>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();

        let worker_handle = thread::Builder::new()
            .name("sentinel-worker".to_string())
            .spawn(move || {
                Self::worker_loop(receiver, store, config, analysis_config, alert_handler);
            })
            .expect("Failed to spawn sentinel worker thread");

        Self {
            sender: Mutex::new(sender),
            worker_handle: Mutex::new(Some(worker_handle)),
        }
    }

    /// Request graceful shutdown of the background worker.
    pub fn shutdown(&self) {
        if let Ok(sender) = self.sender.lock() {
            let _ = sender.send(SentinelMessage::Shutdown);
        }
    }

    /// Returns true if the background worker thread is still alive.
    pub fn is_running(&self) -> bool {
        self.worker_handle
            .lock()
            .map(|h| h.as_ref().is_some_and(|jh| !jh.is_finished()))
            .unwrap_or(false)
    }

    fn worker_loop(
        receiver: mpsc::Receiver<SentinelMessage>,
        store: Store,
        config: SentinelConfig,
        analysis_config: AnalysisConfig,
        alert_handler: Box<dyn AlertHandler>,
    ) {
        let pre_filter = PreFilter::new(config);

        while let Ok(msg) = receiver.recv() {
            match msg {
                SentinelMessage::BlockCommitted { block, receipts } => {
                    Self::process_block(
                        &store,
                        &block,
                        &receipts,
                        &pre_filter,
                        &analysis_config,
                        &*alert_handler,
                    );
                }
                SentinelMessage::Shutdown => break,
            }
        }
    }

    fn process_block(
        store: &Store,
        block: &Block,
        receipts: &[Receipt],
        pre_filter: &PreFilter,
        analysis_config: &AnalysisConfig,
        alert_handler: &dyn AlertHandler,
    ) {
        // Stage 1: Pre-filter with lightweight receipt-based heuristics
        let suspicious_txs =
            pre_filter.scan_block(&block.body.transactions, receipts, &block.header);

        if suspicious_txs.is_empty() {
            return;
        }

        // Stage 2: Deep analysis for each suspicious TX
        for suspicion in &suspicious_txs {
            match DeepAnalyzer::analyze(store, block, suspicion, analysis_config) {
                Ok(Some(alert)) => {
                    alert_handler.on_alert(alert);
                }
                Ok(None) => {
                    // Deep analysis dismissed the suspicion — benign
                }
                Err(_e) => {
                    // In production this would use tracing::warn!
                    // For now, silently skip to avoid crashing the worker
                }
            }
        }
    }
}

impl BlockObserver for SentinelService {
    fn on_block_committed(&self, block: Block, receipts: Vec<Receipt>) {
        if let Ok(sender) = self.sender.lock() {
            // Non-blocking send — if channel is disconnected, silently drop
            let _ = sender.send(SentinelMessage::BlockCommitted {
                block: Box::new(block),
                receipts,
            });
        }
    }
}

impl Drop for SentinelService {
    fn drop(&mut self) {
        self.shutdown();
        if let Ok(mut handle) = self.worker_handle.lock()
            && let Some(h) = handle.take()
        {
            let _ = h.join();
        }
    }
}
