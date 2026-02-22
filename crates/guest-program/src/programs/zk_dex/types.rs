use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

/// Input for the ZK-DEX guest program.
///
/// Represents a batch of token transfers to be proven.  The prover must
/// demonstrate that every transfer is valid (sufficient balance, correct
/// nonce) and that the `initial_state_root` transitions deterministically
/// to the `final_state_root` computed by the execution function.
#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone, Debug)]
pub struct DexProgramInput {
    /// Merkle root of the token-balance state before this batch.
    pub initial_state_root: [u8; 32],
    /// Ordered list of transfers in this batch.
    pub transfers: Vec<DexTransfer>,
}

/// A single token transfer inside a ZK-DEX batch.
#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone, Debug)]
pub struct DexTransfer {
    /// Sender address (20 bytes).
    pub from: [u8; 20],
    /// Recipient address (20 bytes).
    pub to: [u8; 20],
    /// Token contract address (20 bytes).
    pub token: [u8; 20],
    /// Transfer amount (simplified to u64 for the demo).
    pub amount: u64,
    /// Sender's nonce for replay protection.
    pub nonce: u64,
}

/// Output of the ZK-DEX guest program.
///
/// Committed as public values by the zkVM so the L1 verifier can
/// check the state transition.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DexProgramOutput {
    /// State root before execution.
    pub initial_state_root: [u8; 32],
    /// State root after executing all transfers.
    pub final_state_root: [u8; 32],
    /// Number of transfers successfully processed.
    pub transfer_count: u64,
}

impl DexProgramOutput {
    /// Encode the output to bytes for L1 commitment verification.
    ///
    /// Layout: `initial_state_root (32) || final_state_root (32) || transfer_count (8 BE)`
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(72);
        buf.extend_from_slice(&self.initial_state_root);
        buf.extend_from_slice(&self.final_state_root);
        buf.extend_from_slice(&self.transfer_count.to_be_bytes());
        buf
    }
}
