//! Sentinel-specific types for the pre-filter, deep analysis, and alert system.

use ethrex_common::{Address, H256, U256};
use serde::{Deserialize, Serialize};

#[cfg(feature = "autopsy")]
use crate::autopsy::types::{AttackPattern, DetectedPattern, FundFlow};

/// Configuration for the sentinel pre-filter.
#[derive(Debug, Clone)]
pub struct SentinelConfig {
    /// Minimum combined score to flag a TX as suspicious (default: 0.5).
    pub suspicion_threshold: f64,
    /// Minimum ETH value for high-value transfer heuristic (default: 1 ETH).
    pub min_value_wei: U256,
    /// Minimum gas for gas-related heuristics (default: 500_000).
    pub min_gas_used: u64,
    /// Minimum ERC-20 transfer count to flag (default: 5).
    pub min_erc20_transfers: usize,
    /// Gas ratio threshold for unusual-gas heuristic (default: 0.95).
    pub gas_ratio_threshold: f64,
}

impl Default for SentinelConfig {
    fn default() -> Self {
        Self {
            suspicion_threshold: 0.5,
            // 1 ETH = 10^18 wei
            min_value_wei: U256::from(1_000_000_000_000_000_000_u64),
            min_gas_used: 500_000,
            min_erc20_transfers: 5,
            gas_ratio_threshold: 0.95,
        }
    }
}

/// A transaction flagged as suspicious by the pre-filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspiciousTx {
    pub tx_hash: H256,
    pub tx_index: usize,
    pub reasons: Vec<SuspicionReason>,
    pub score: f64,
    pub priority: AlertPriority,
}

/// Reason why a transaction was flagged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuspicionReason {
    /// Flash loan event signature detected in logs.
    FlashLoanSignature { provider_address: Address },
    /// High-value TX that reverted with significant gas usage.
    HighValueWithRevert { value_wei: U256, gas_used: u64 },
    /// Unusually many ERC-20 Transfer events in a single TX.
    MultipleErc20Transfers { count: usize },
    /// TX interacts with a known high-value DeFi contract.
    KnownContractInteraction {
        address: Address,
        label: String,
    },
    /// Gas usage suspiciously close to gas limit (automated exploit script).
    UnusualGasPattern { gas_used: u64, gas_limit: u64 },
    /// Self-destruct indicators detected.
    SelfDestructDetected,
    /// Both price oracle and DEX interaction in same TX.
    PriceOracleWithSwap { oracle: Address },
}

impl SuspicionReason {
    /// Fixed score contribution for this reason.
    pub fn score(&self) -> f64 {
        match self {
            Self::FlashLoanSignature { .. } => 0.4,
            Self::HighValueWithRevert { .. } => 0.3,
            Self::MultipleErc20Transfers { count } => {
                if *count > 10 {
                    0.4
                } else {
                    0.2
                }
            }
            Self::KnownContractInteraction { .. } => 0.1,
            Self::UnusualGasPattern { .. } => 0.15,
            Self::SelfDestructDetected => 0.3,
            Self::PriceOracleWithSwap { .. } => 0.2,
        }
    }
}

/// Alert priority derived from combined suspicion score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertPriority {
    /// Score >= 0.3 but < 0.5
    Medium,
    /// Score >= 0.5 but < 0.8
    High,
    /// Score >= 0.8
    Critical,
}

impl AlertPriority {
    pub fn from_score(score: f64) -> Self {
        if score >= 0.8 {
            Self::Critical
        } else if score >= 0.5 {
            Self::High
        } else {
            Self::Medium
        }
    }
}

/// Configuration for the deep analysis engine.
#[derive(Debug, Clone)]
pub struct AnalysisConfig {
    /// Maximum opcode steps to record before aborting (default: 1_000_000).
    pub max_steps: usize,
    /// Minimum confidence to emit a SentinelAlert (default: 0.4).
    pub min_alert_confidence: f64,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            max_steps: 1_000_000,
            min_alert_confidence: 0.4,
        }
    }
}

/// Alert emitted after deep analysis confirms suspicious activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentinelAlert {
    pub block_number: u64,
    pub block_hash: H256,
    pub tx_hash: H256,
    pub tx_index: usize,
    pub alert_priority: AlertPriority,
    /// Pre-filter suspicion reasons that triggered deep analysis.
    pub suspicion_reasons: Vec<SuspicionReason>,
    /// Pre-filter score.
    pub suspicion_score: f64,
    /// Attack patterns confirmed by deep analysis.
    #[cfg(feature = "autopsy")]
    pub detected_patterns: Vec<DetectedPattern>,
    /// Fund flows extracted by deep analysis.
    #[cfg(feature = "autopsy")]
    pub fund_flows: Vec<FundFlow>,
    /// Total value at risk across all fund flows.
    pub total_value_at_risk: U256,
    /// Human-readable summary.
    pub summary: String,
    /// Number of opcode steps recorded during replay.
    pub total_steps: usize,
}

#[cfg(feature = "autopsy")]
impl SentinelAlert {
    /// Highest confidence among all detected patterns.
    pub fn max_confidence(&self) -> f64 {
        self.detected_patterns
            .iter()
            .map(|p| p.confidence)
            .fold(0.0_f64, f64::max)
    }

    /// Names of all detected attack patterns.
    pub fn pattern_names(&self) -> Vec<&'static str> {
        self.detected_patterns
            .iter()
            .map(|p| match &p.pattern {
                AttackPattern::Reentrancy { .. } => "Reentrancy",
                AttackPattern::FlashLoan { .. } => "FlashLoan",
                AttackPattern::PriceManipulation { .. } => "PriceManipulation",
                AttackPattern::AccessControlBypass { .. } => "AccessControlBypass",
            })
            .collect()
    }
}

/// Errors specific to the sentinel deep analysis engine.
#[derive(Debug, thiserror::Error)]
pub enum SentinelError {
    #[error("VM execution error: {0}")]
    Vm(String),

    #[error("Database error: {0}")]
    Db(String),

    #[error("Block {block_number} not found in store")]
    BlockNotFound { block_number: u64 },

    #[error("Transaction at index {tx_index} not found in block {block_number}")]
    TxNotFound { block_number: u64, tx_index: usize },

    #[error("Parent block header not found for block {block_number}")]
    ParentNotFound { block_number: u64 },

    #[error("State root missing for block {block_number}")]
    StateRootMissing { block_number: u64 },

    #[error("Sender recovery failed for tx at index {tx_index}: {cause}")]
    SenderRecovery { tx_index: usize, cause: String },

    #[error("Step limit exceeded: {steps} > {max_steps}")]
    StepLimitExceeded { steps: usize, max_steps: usize },
}
