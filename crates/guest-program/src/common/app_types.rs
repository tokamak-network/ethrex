use ethrex_common::types::blobs_bundle;
use ethrex_common::types::l2::fee_config::FeeConfig;
use ethrex_common::types::Block;
use ethrex_common::{Address, H256, U256};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, Bytes};

/// Input for app-specific circuits.
///
/// Unlike the full EVM `ProgramInput` which includes an `ExecutionWitness`
/// (the entire state trie subset), this input provides only the specific
/// storage proofs needed for the app's operations. The circuit uses these
/// proofs to verify and update state incrementally, avoiding full EVM execution.
#[serde_as]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppProgramInput {
    /// Blocks to execute.
    pub blocks: Vec<Block>,

    /// Previous state root (already verified on L1).
    pub prev_state_root: H256,

    /// Merkle proofs for storage slots that will be read/modified.
    pub storage_proofs: Vec<StorageProof>,

    /// Account proofs for accounts whose balance/nonce will change.
    pub account_proofs: Vec<AccountProof>,

    /// Elasticity multiplier for base fee calculation.
    pub elasticity_multiplier: u64,

    /// Per-block fee configuration.
    pub fee_configs: Vec<FeeConfig>,

    /// KZG blob commitment (48 bytes).
    #[serde_as(as = "Bytes")]
    pub blob_commitment: blobs_bundle::Commitment,

    /// KZG blob proof (48 bytes).
    #[serde_as(as = "Bytes")]
    pub blob_proof: blobs_bundle::Proof,

    /// Chain ID.
    pub chain_id: u64,
}

/// Merkle proof for a specific storage slot.
///
/// The circuit verifies this proof against `prev_state_root` and uses it to
/// compute the new state root after modifications.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StorageProof {
    /// Account address.
    pub address: Address,

    /// Storage slot key.
    pub slot: H256,

    /// Current value at this slot.
    pub value: U256,

    /// Merkle path from state trie root to this account (RLP-encoded nodes).
    pub account_proof: Vec<Vec<u8>>,

    /// Merkle path from storage trie root to this slot (RLP-encoded nodes).
    pub storage_proof: Vec<Vec<u8>>,
}

/// Merkle proof for an account (balance, nonce, etc.).
///
/// Used for accounts that participate in ETH transfers, gas deduction,
/// or deposit/withdrawal operations.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AccountProof {
    /// Account address.
    pub address: Address,

    /// Account nonce.
    pub nonce: u64,

    /// Account balance.
    pub balance: U256,

    /// Account storage root.
    pub storage_root: H256,

    /// Account code hash.
    pub code_hash: H256,

    /// Merkle path from state trie root to this account (RLP-encoded nodes).
    pub proof: Vec<Vec<u8>>,
}
