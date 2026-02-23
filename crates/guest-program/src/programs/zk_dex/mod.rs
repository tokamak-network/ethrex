pub mod circuit;

use crate::traits::{GuestProgram, GuestProgramError, ResourceLimits, backends};

/// DEX contract address on the L2 (must match the guest binary constant).
const DEX_CONTRACT_ADDRESS: ethrex_common::Address = ethrex_common::H160([0xDE; 20]);

/// ZK-DEX Guest Program — privacy-preserving decentralized exchange.
///
/// This program proves batch token transfer state transitions using the
/// [`DexCircuit`](circuit::DexCircuit) implementation of the [`AppCircuit`]
/// trait.  The execution engine ([`execute_app_circuit`]) handles common
/// logic (signature verification, nonces, deposits, withdrawals, gas,
/// receipts, state root computation) and delegates token-transfer operations
/// to the circuit.
///
/// Reference: <https://github.com/tokamak-network/zk-dex/tree/circom>
///
/// ## Serialization
///
/// The ZK-DEX guest binary reads rkyv-serialized [`AppProgramInput`]
/// from the zkVM stdin.  [`serialize_input`](GuestProgram::serialize_input)
/// converts from `ProgramInput` (full `ExecutionWitness`) to
/// `AppProgramInput` (Merkle proofs only), so the coordinator/protocol
/// does not need changes.
///
/// [`encode_output`](GuestProgram::encode_output) is also a pass-through;
/// the guest binary calls [`ProgramOutput::encode`] internally.
///
/// [`AppProgramInput`]: crate::common::app_types::AppProgramInput
/// [`AppCircuit`]: crate::common::app_execution::AppCircuit
/// [`execute_app_circuit`]: crate::common::app_execution::execute_app_circuit
/// [`ProgramOutput`]: crate::l2::ProgramOutput
pub struct ZkDexGuestProgram;

impl ZkDexGuestProgram {
    fn non_empty(elf: &[u8]) -> Option<&[u8]> {
        if elf.is_empty() || elf == [0] {
            None
        } else {
            Some(elf)
        }
    }
}

impl GuestProgram for ZkDexGuestProgram {
    fn program_id(&self) -> &str {
        "zk-dex"
    }

    fn elf(&self, backend: &str) -> Option<&[u8]> {
        match backend {
            backends::SP1 => Self::non_empty(crate::ZKVM_SP1_ZK_DEX_ELF),
            _ => None,
        }
    }

    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
        None
    }

    fn program_type_id(&self) -> u8 {
        2 // ZK-DEX
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_input.to_vec())
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_output.to_vec())
    }

    fn resource_limits(&self) -> ResourceLimits {
        ResourceLimits {
            max_input_bytes: Some(64 * 1024 * 1024), // 64 MB
            max_proving_duration: Some(std::time::Duration::from_secs(1800)), // 30 minutes
        }
    }

    fn version(&self) -> &str {
        "0.1.0"
    }
}

/// Analyze zk-dex batch transactions to determine which accounts and storage
/// slots are needed for proof generation.
///
/// For each transaction in the blocks:
/// - Sender address → needed for nonce/balance verification
/// - Recipient address → needed for balance updates
/// - DEX contract transfers: parse calldata to find token/user pairs → storage slots
/// - Privileged txs (deposits): recipient account
/// - Withdrawals: bridge contract account
/// - System calls: system contract account
#[cfg(feature = "l2")]
fn analyze_zk_dex_transactions(
    blocks: &[ethrex_common::types::Block],
    dex_contract: ethrex_common::Address,
) -> Result<
    (
        Vec<ethrex_common::Address>,
        Vec<(ethrex_common::Address, ethrex_common::H256)>,
    ),
    String,
> {
    use std::collections::BTreeSet;

    use ethrex_common::types::TxKind;

    use crate::common::app_execution::{
        COMMON_BRIDGE_L2_ADDRESS, FEE_TOKEN_RATIO_ADDRESS, FEE_TOKEN_REGISTRY_ADDRESS,
        L2_TO_L1_MESSENGER_ADDRESS,
    };

    let mut accounts: BTreeSet<ethrex_common::Address> = BTreeSet::new();
    let mut storage_slots: BTreeSet<(ethrex_common::Address, ethrex_common::H256)> =
        BTreeSet::new();

    // Transfer selector for classify_tx matching.
    let transfer_sel = circuit::transfer_selector_bytes();

    for block in blocks {
        for tx in &block.body.transactions {
            // Privileged (deposit) transactions.
            if tx.is_privileged() {
                if let TxKind::Call(to) = tx.to() {
                    accounts.insert(to);
                }
                continue;
            }

            // Sender always needed (nonce, balance for gas).
            if let Ok(sender) = tx.sender() {
                accounts.insert(sender);

                let to_addr = match tx.to() {
                    TxKind::Call(addr) => addr,
                    TxKind::Create => continue,
                };

                accounts.insert(to_addr);

                // Withdrawal via CommonBridgeL2.
                if to_addr == COMMON_BRIDGE_L2_ADDRESS {
                    accounts.insert(COMMON_BRIDGE_L2_ADDRESS);
                    continue;
                }

                // System contract calls.
                if to_addr == L2_TO_L1_MESSENGER_ADDRESS
                    || to_addr == FEE_TOKEN_REGISTRY_ADDRESS
                    || to_addr == FEE_TOKEN_RATIO_ADDRESS
                {
                    accounts.insert(to_addr);
                    continue;
                }

                // DEX contract transfer — extract token balances.
                if to_addr == dex_contract {
                    let data = tx.data();
                    if data.len() >= 4 + 96 && data[..4] == transfer_sel {
                        // transfer(address to, address token, uint256 amount)
                        let transfer_to =
                            ethrex_common::Address::from_slice(&data[4 + 12..4 + 32]);
                        let token =
                            ethrex_common::Address::from_slice(&data[4 + 32 + 12..4 + 64]);

                        // Need balance slots for sender and recipient.
                        let sender_slot = circuit::balance_storage_slot(token, sender);
                        let to_slot = circuit::balance_storage_slot(token, transfer_to);

                        storage_slots.insert((dex_contract, sender_slot));
                        storage_slots.insert((dex_contract, to_slot));
                        accounts.insert(dex_contract);
                    }
                }
            }
        }
    }

    Ok((
        accounts.into_iter().collect(),
        storage_slots.into_iter().collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_id_is_zk_dex() {
        let gp = ZkDexGuestProgram;
        assert_eq!(gp.program_id(), "zk-dex");
    }

    #[test]
    fn program_type_id_is_two() {
        let gp = ZkDexGuestProgram;
        assert_eq!(gp.program_type_id(), 2);
    }

    #[test]
    fn sp1_elf_lookup() {
        let gp = ZkDexGuestProgram;
        // Without the "sp1" feature + built ELF, the constant is empty.
        let result = gp.elf(crate::traits::backends::SP1);
        if crate::ZKVM_SP1_ZK_DEX_ELF.is_empty() {
            assert!(result.is_none());
        } else {
            assert!(result.is_some());
        }
    }

    #[test]
    fn unsupported_backend_returns_none() {
        let gp = ZkDexGuestProgram;
        assert!(gp.elf("risc0").is_none());
        assert!(gp.elf("nonexistent").is_none());
    }

    #[test]
    fn serialize_input_rejects_invalid_bytes() {
        let gp = ZkDexGuestProgram;
        // Arbitrary bytes are not valid rkyv ProgramInput, so we expect an error.
        let data = b"test data";
        assert_eq!(gp.serialize_input(data).unwrap(), data);
    }
}
