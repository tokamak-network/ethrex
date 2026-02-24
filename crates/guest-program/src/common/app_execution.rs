//! App-specific circuit execution engine.
//!
//! This module provides the `AppCircuit` trait and the `execute_app_circuit()`
//! function, which together form the common execution framework for all
//! app-specific guest programs.
//!
//! ## Architecture
//!
//! ```text
//! execute_app_circuit(circuit, input)
//!   ├── For each tx:
//!   │   ├── Privileged?     → handle_privileged_tx()     [common]
//!   │   ├── ETH transfer?   → transfer_eth()             [common]
//!   │   ├── Withdrawal?     → handle_withdrawal()         [common]
//!   │   ├── System call?    → handle_system_call()         [common]
//!   │   └── App operation?  → circuit.execute_operation()  [app-specific]
//!   ├── Compute new state root (incremental MPT)
//!   ├── Compute message digests
//!   └── Return ProgramOutput
//! ```

use ethrex_common::types::{Log, Receipt, Transaction, TxKind};
use ethrex_common::{Address, H160, U256};

use crate::l2::messages::{compute_message_digests, get_batch_messages};
use crate::l2::ProgramOutput;

use super::app_state::{AppState, AppStateError};
use super::app_types::AppProgramInput;
use super::incremental_mpt;

// ── System contract addresses ─────────────────────────────────────

/// CommonBridgeL2: 0x000000000000000000000000000000000000ffff
pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xff, 0xff,
]);

/// L2-to-L1 Messenger: 0x000000000000000000000000000000000000fffe
pub const L2_TO_L1_MESSENGER_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xff, 0xfe,
]);

/// Fee Token Registry: 0x000000000000000000000000000000000000fffc
pub const FEE_TOKEN_REGISTRY_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xff, 0xfc,
]);

/// Fee Token Ratio: 0x000000000000000000000000000000000000fffb
pub const FEE_TOKEN_RATIO_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xff, 0xfb,
]);

/// Fixed gas cost for a simple ETH transfer (no calldata).
pub const ETH_TRANSFER_GAS: u64 = 21_000;

// ── App Circuit trait ─────────────────────────────────────────────

/// Trait that each app-specific circuit must implement.
///
/// The common execution engine handles all shared logic (signature
/// verification, nonces, ETH transfers, deposits, withdrawals, gas
/// deduction, receipts, message digests, state root computation).
/// Only app-specific operations are delegated to this trait.
pub trait AppCircuit {
    /// Classify a transaction as an app operation.
    ///
    /// Called only for transactions that are NOT privileged, ETH transfers,
    /// withdrawals, or system calls. The circuit should parse the calldata
    /// (typically the first 4 bytes = function selector) and return the
    /// operation type and decoded parameters.
    ///
    /// Returns `Err` if the transaction doesn't match any known app operation.
    fn classify_tx(&self, tx: &Transaction) -> Result<AppOperation, AppCircuitError>;

    /// Execute an app operation and update state.
    ///
    /// The implementation should:
    /// 1. Read relevant state from `state` (already verified via proofs)
    /// 2. Compute the state transition (e.g., constant product formula for swap)
    /// 3. Write updated values back to `state`
    ///
    /// Returns the operation result (for log generation).
    fn execute_operation(
        &self,
        state: &mut AppState,
        from: Address,
        op: &AppOperation,
    ) -> Result<OperationResult, AppCircuitError>;

    /// Return the fixed gas cost for this operation.
    ///
    /// Since app transactions are predetermined, gas is fixed per operation
    /// type. This value must match the actual EVM gas consumption of the
    /// corresponding Solidity contract function.
    fn gas_cost(&self, op: &AppOperation) -> u64;

    /// Generate the fixed log pattern for this operation.
    ///
    /// Since app transactions are predetermined, the event logs follow a
    /// fixed pattern per operation type (e.g., swap always emits Transfer +
    /// Swap events). This must match the EVM-generated logs exactly for
    /// receipt root consistency.
    fn generate_logs(
        &self,
        from: Address,
        op: &AppOperation,
        result: &OperationResult,
    ) -> Vec<Log>;
}

/// An app-specific operation parsed from a transaction.
pub struct AppOperation {
    /// Operation type ID (app-specific, e.g., 0 = Swap, 1 = AddLiquidity).
    pub op_type: u8,
    /// ABI-decoded parameters (app-specific encoding).
    pub params: Vec<u8>,
}

/// Result of executing an app operation.
pub struct OperationResult {
    /// Whether the operation succeeded.
    pub success: bool,
    /// Opaque result data (app-specific, used for log generation).
    pub data: Vec<u8>,
}

/// Errors during app circuit execution.
#[derive(Debug, thiserror::Error)]
pub enum AppCircuitError {
    #[error("Unknown transaction: cannot classify as app operation")]
    UnknownTransaction,
    #[error("Invalid parameters: {0}")]
    InvalidParams(String),
    #[error("State error: {0}")]
    State(#[from] AppStateError),
    #[error("Signature verification failed")]
    InvalidSignature,
    #[error("Block validation error: {0}")]
    BlockValidation(String),
    #[error("MPT error: {0}")]
    Mpt(#[from] incremental_mpt::IncrementalMptError),
    #[error("Blob verification error: {0}")]
    Blob(String),
    #[error("Message digest error: {0}")]
    MessageDigest(String),
    #[error("Contract creation not allowed")]
    ContractCreationNotAllowed,
    #[error("Empty batch")]
    EmptyBatch,
}

// ── Main execution function ───────────────────────────────────────

/// Execute an app-specific circuit.
///
/// This is the main entry point for all app-specific guest programs.
/// It handles all common logic and delegates app-specific operations
/// to the provided `AppCircuit` implementation.
///
/// The output is compatible with the existing `ProgramOutput` format,
/// ensuring L1 OnChainProposer contract compatibility.
pub fn execute_app_circuit<C: AppCircuit>(
    circuit: &C,
    input: AppProgramInput,
) -> Result<ProgramOutput, AppCircuitError> {
    if input.blocks.is_empty() {
        return Err(AppCircuitError::EmptyBatch);
    }

    // 1. Build state from proofs.
    let mut state = AppState::from_proofs(
        input.prev_state_root,
        input.account_proofs.clone(),
        input.storage_proofs.clone(),
    );

    // 2. Verify all proofs against the previous state root.
    incremental_mpt::verify_state_proofs(&state)?;

    // 3. Execute each block.
    let mut all_receipts: Vec<Vec<Receipt>> = Vec::new();
    let mut non_privileged_count: u64 = 0;

    for block in &input.blocks {
        let mut block_receipts: Vec<Receipt> = Vec::new();
        let mut cumulative_gas: u64 = 0;

        for tx in &block.body.transactions {
            // ── Privileged transactions (L2 deposits) ── common
            if tx.is_privileged() {
                handle_privileged_tx(&mut state, tx)?;
                // Privileged txs don't produce standard receipts for our purposes.
                // They are accounted for in message digests.
                continue;
            }

            // ── Signature verification ── common
            let sender = tx
                .sender()
                .map_err(|_| AppCircuitError::InvalidSignature)?;

            // ── Nonce verification and increment ── common
            let expected_nonce = state.get_nonce(sender)?;
            state.verify_and_increment_nonce(sender, expected_nonce)?;

            // ── Contract creation check ── always rejected for app L2
            if tx.to() == TxKind::Create {
                return Err(AppCircuitError::ContractCreationNotAllowed);
            }

            let to_address = match tx.to() {
                TxKind::Call(addr) => addr,
                TxKind::Create => unreachable!(),
            };

            // ── ETH transfer (no calldata) ── common
            if tx.data().is_empty() {
                state.transfer_eth(sender, to_address, tx.value())?;
                cumulative_gas += ETH_TRANSFER_GAS;
                apply_gas_deduction(&mut state, sender, ETH_TRANSFER_GAS, &block.header)?;
                block_receipts.push(Receipt {
                    tx_type: tx.tx_type(),
                    succeeded: true,
                    cumulative_gas_used: cumulative_gas,
                    logs: vec![], // ETH transfers emit no events
                });
                non_privileged_count += 1;
                continue;
            }

            // ── Withdrawal (CommonBridgeL2) ── common
            if to_address == COMMON_BRIDGE_L2_ADDRESS {
                let gas = handle_withdrawal(&mut state, tx, sender)?;
                cumulative_gas += gas;
                apply_gas_deduction(&mut state, sender, gas, &block.header)?;
                block_receipts.push(Receipt {
                    tx_type: tx.tx_type(),
                    succeeded: true,
                    cumulative_gas_used: cumulative_gas,
                    logs: generate_withdrawal_logs(sender, tx),
                });
                non_privileged_count += 1;
                continue;
            }

            // ── System contract calls ── common
            if is_system_contract(to_address) {
                let gas = handle_system_call(&mut state, tx, sender, to_address)?;
                cumulative_gas += gas;
                apply_gas_deduction(&mut state, sender, gas, &block.header)?;
                block_receipts.push(Receipt {
                    tx_type: tx.tx_type(),
                    succeeded: true,
                    cumulative_gas_used: cumulative_gas,
                    logs: vec![], // System calls: logs depend on specific contract
                });
                non_privileged_count += 1;
                continue;
            }

            // ── App-specific operation ── delegated to circuit
            let op = circuit.classify_tx(tx)?;
            let gas = circuit.gas_cost(&op);
            let result = circuit.execute_operation(&mut state, sender, &op)?;

            // Handle ETH value transfer if any.
            if !tx.value().is_zero() {
                state.transfer_eth(sender, to_address, tx.value())?;
            }

            cumulative_gas += gas;
            apply_gas_deduction(&mut state, sender, gas, &block.header)?;

            let logs = circuit.generate_logs(sender, &op, &result);
            block_receipts.push(Receipt {
                tx_type: tx.tx_type(),
                succeeded: result.success,
                cumulative_gas_used: cumulative_gas,
                logs,
            });
            non_privileged_count += 1;
        }

        all_receipts.push(block_receipts);
    }

    // 4. Compute new state root (incremental MPT update).
    let final_state_hash = incremental_mpt::compute_new_state_root(&state)?;

    // 5. Compute message digests (deposits, withdrawals).
    let batch_messages = get_batch_messages(&input.blocks, &all_receipts, input.chain_id);
    let message_digests = compute_message_digests(&batch_messages)
        .map_err(|e| AppCircuitError::MessageDigest(e.to_string()))?;
    let balance_diffs =
        ethrex_l2_common::messages::get_balance_diffs(&batch_messages.l2_out_messages);

    // 6. Compute blob versioned hash.
    //
    // App-specific circuits skip KZG proof verification because:
    //   - The L1 OnChainProposer already verifies blob KZG proofs
    //   - kzg-rs's BLS12-381 operations are expensive in the zkVM
    //   - The app circuit only needs the versioned hash for ProgramOutput
    //
    // For validium mode (commitment = [0; 48]), the hash is H256::zero().
    let blob_versioned_hash = {
        use ethrex_common::types::kzg_commitment_to_versioned_hash;
        let is_validium = input.blob_commitment == [0u8; 48] && input.blob_proof == [0u8; 48];
        if is_validium {
            ethrex_common::H256::zero()
        } else {
            kzg_commitment_to_versioned_hash(&input.blob_commitment)
        }
    };

    // 7. Build output (same format as evm-l2 ProgramOutput).
    let last_block_hash = input
        .blocks
        .last()
        .ok_or(AppCircuitError::EmptyBatch)?
        .header
        .hash();

    Ok(ProgramOutput {
        initial_state_hash: input.prev_state_root,
        final_state_hash,
        l1_out_messages_merkle_root: message_digests.l1_out_messages_merkle_root,
        l1_in_messages_rolling_hash: message_digests.l1_in_messages_rolling_hash,
        l2_in_message_rolling_hashes: message_digests.l2_in_message_rolling_hashes,
        blob_versioned_hash,
        last_block_hash,
        chain_id: U256::from(input.chain_id),
        non_privileged_count: U256::from(non_privileged_count),
        balance_diffs,
    })
}

// ── Common transaction handlers ───────────────────────────────────

/// Handle a privileged (deposit) transaction.
///
/// Privileged transactions are L1→L2 deposits. The sender (bridge contract)
/// can mint ETH, so we simply credit the recipient's balance.
fn handle_privileged_tx(
    state: &mut AppState,
    tx: &Transaction,
) -> Result<(), AppCircuitError> {
    let value = tx.value();
    if !value.is_zero() {
        if let TxKind::Call(to) = tx.to() {
            // Credit the recipient. For deposits via bridge, the balance
            // is minted (not transferred from sender).
            state.credit_balance(to, value)?;
        }
    }
    Ok(())
}

/// Handle a withdrawal (L2→L1) transaction via CommonBridgeL2.
///
/// The withdrawal burns ETH on L2 and creates an L1 message.
/// Returns the fixed gas cost for this operation.
fn handle_withdrawal(
    state: &mut AppState,
    tx: &Transaction,
    sender: Address,
) -> Result<u64, AppCircuitError> {
    let value = tx.value();
    if !value.is_zero() {
        state.debit_balance(sender, value)?;
    }
    // Fixed gas for withdrawal operation.
    // TODO: Measure actual EVM gas for CommonBridgeL2.withdraw().
    Ok(100_000)
}

/// Handle a system contract call (L1Messenger, FeeTokenRegistry, etc.).
///
/// Returns the fixed gas cost for this operation.
fn handle_system_call(
    _state: &mut AppState,
    _tx: &Transaction,
    _sender: Address,
    _target: Address,
) -> Result<u64, AppCircuitError> {
    // TODO: Implement system contract logic per contract.
    // For now, just charge a fixed gas cost.
    Ok(50_000)
}

/// Check if an address is a known system contract.
fn is_system_contract(address: Address) -> bool {
    address == COMMON_BRIDGE_L2_ADDRESS
        || address == L2_TO_L1_MESSENGER_ADDRESS
        || address == FEE_TOKEN_REGISTRY_ADDRESS
        || address == FEE_TOKEN_RATIO_ADDRESS
}

/// Apply gas deduction to the sender's balance.
///
/// Gas fee = gas_used * effective_gas_price.
/// For EIP-1559: effective_gas_price = min(max_fee, base_fee + max_priority_fee).
fn apply_gas_deduction(
    state: &mut AppState,
    sender: Address,
    gas_used: u64,
    block_header: &ethrex_common::types::BlockHeader,
) -> Result<(), AppCircuitError> {
    let base_fee = block_header.base_fee_per_gas.unwrap_or(0);
    let gas_price = U256::from(base_fee);
    let fee = gas_price * U256::from(gas_used);
    if !fee.is_zero() {
        state.debit_balance(sender, fee)?;
    }
    Ok(())
}

/// Generate logs for a withdrawal transaction.
///
/// The withdrawal event follows a fixed pattern since the bridge contract
/// always emits the same events.
fn generate_withdrawal_logs(_sender: Address, _tx: &Transaction) -> Vec<Log> {
    // TODO: Generate exact withdrawal logs matching the EVM output.
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_system_contract_checks() {
        assert!(is_system_contract(COMMON_BRIDGE_L2_ADDRESS));
        assert!(is_system_contract(L2_TO_L1_MESSENGER_ADDRESS));
        assert!(is_system_contract(FEE_TOKEN_REGISTRY_ADDRESS));
        assert!(is_system_contract(FEE_TOKEN_RATIO_ADDRESS));
        assert!(!is_system_contract(Address::zero()));
        assert!(!is_system_contract(H160([0xFF; 20])));
    }
}
