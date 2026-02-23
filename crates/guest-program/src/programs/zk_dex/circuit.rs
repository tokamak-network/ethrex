//! DexCircuit — AppCircuit implementation for the ZK-DEX guest program.
//!
//! Currently supports a single operation type: **TokenTransfer** (ERC-20 style
//! token transfer between accounts).  The circuit reads/writes balances from
//! [`AppState`] storage using a Solidity-compatible storage layout:
//!
//! ```text
//! mapping(address token => mapping(address user => uint256)) balances;  // slot 0
//! slot = keccak256(abi.encode(user, keccak256(abi.encode(token, 0))))
//! ```

use bytes::Bytes;
use ethrex_common::types::{Log, Transaction, TxKind};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::keccak::keccak_hash;

use crate::common::app_execution::{AppCircuit, AppCircuitError, AppOperation, OperationResult};
use crate::common::app_state::AppState;

// ── Operation type constants ─────────────────────────────────────

/// ERC-20 style token transfer between two accounts.
pub const OP_TOKEN_TRANSFER: u8 = 0;

/// Fixed gas cost for a token transfer (matches Solidity contract).
pub const TOKEN_TRANSFER_GAS: u64 = 65_000;

/// ABI selector for `transfer(address,address,uint256)`.
///
/// Computed as `keccak256("transfer(address,address,uint256)")[0..4]`.
fn transfer_selector() -> [u8; 4] {
    let hash = keccak_hash(b"transfer(address,address,uint256)");
    [hash[0], hash[1], hash[2], hash[3]]
}

/// Public accessor for the transfer selector bytes (used by input conversion).
pub fn transfer_selector_bytes() -> [u8; 4] {
    transfer_selector()
}

/// ERC-20 `Transfer(address,address,uint256)` event topic.
fn transfer_event_topic() -> H256 {
    H256::from(keccak_hash(b"Transfer(address,address,uint256)"))
}

// ── DexCircuit ───────────────────────────────────────────────────

/// ZK-DEX circuit that implements [`AppCircuit`].
///
/// Processes token transfer operations against verified storage proofs.
pub struct DexCircuit {
    /// DEX contract address on the L2.
    pub contract_address: Address,
}

impl AppCircuit for DexCircuit {
    fn classify_tx(&self, tx: &Transaction) -> Result<AppOperation, AppCircuitError> {
        // Must be a call to the DEX contract.
        let to = match tx.to() {
            TxKind::Call(addr) => addr,
            TxKind::Create => return Err(AppCircuitError::UnknownTransaction),
        };
        if to != self.contract_address {
            return Err(AppCircuitError::UnknownTransaction);
        }

        let data = tx.data();
        if data.len() < 4 {
            return Err(AppCircuitError::InvalidParams(
                "calldata too short for selector".into(),
            ));
        }

        let selector = &data[..4];
        if selector == transfer_selector() {
            // transfer(address to, address token, uint256 amount)
            // Each param is 32-byte ABI-encoded.
            if data.len() < 4 + 96 {
                return Err(AppCircuitError::InvalidParams(
                    "calldata too short for transfer params".into(),
                ));
            }
            // params = to(32) + token(32) + amount(32)
            let params = data[4..4 + 96].to_vec();
            Ok(AppOperation {
                op_type: OP_TOKEN_TRANSFER,
                params,
            })
        } else {
            Err(AppCircuitError::UnknownTransaction)
        }
    }

    fn execute_operation(
        &self,
        state: &mut AppState,
        from: Address,
        op: &AppOperation,
    ) -> Result<OperationResult, AppCircuitError> {
        match op.op_type {
            OP_TOKEN_TRANSFER => self.execute_transfer(state, from, &op.params),
            _ => Err(AppCircuitError::InvalidParams(format!(
                "unknown op_type: {}",
                op.op_type
            ))),
        }
    }

    fn gas_cost(&self, op: &AppOperation) -> u64 {
        match op.op_type {
            OP_TOKEN_TRANSFER => TOKEN_TRANSFER_GAS,
            _ => 0,
        }
    }

    fn generate_logs(&self, from: Address, op: &AppOperation, result: &OperationResult) -> Vec<Log> {
        if !result.success {
            return vec![];
        }
        match op.op_type {
            OP_TOKEN_TRANSFER => {
                // Decode params: to(32) + token(32) + amount(32)
                if op.params.len() < 96 {
                    return vec![];
                }
                let to = address_from_abi_word(&op.params[0..32]);
                let amount_bytes = &op.params[64..96];

                // Transfer event: topic0 = Transfer sig, topic1 = from, topic2 = to
                // data = amount (32 bytes)
                let log = Log {
                    address: self.contract_address,
                    topics: vec![
                        transfer_event_topic(),
                        address_to_h256(from),
                        address_to_h256(to),
                    ],
                    data: Bytes::copy_from_slice(amount_bytes),
                };
                vec![log]
            }
            _ => vec![],
        }
    }
}

impl DexCircuit {
    /// Execute a token transfer: debit sender, credit receiver.
    fn execute_transfer(
        &self,
        state: &mut AppState,
        from: Address,
        params: &[u8],
    ) -> Result<OperationResult, AppCircuitError> {
        if params.len() < 96 {
            return Err(AppCircuitError::InvalidParams(
                "transfer params too short".into(),
            ));
        }

        let to = address_from_abi_word(&params[0..32]);
        let token = address_from_abi_word(&params[32..64]);
        let amount = U256::from_big_endian(&params[64..96]);

        // Zero-amount transfer succeeds as a no-op.
        if amount.is_zero() {
            return Ok(OperationResult {
                success: true,
                data: vec![],
            });
        }

        // Read sender balance.
        let from_slot = balance_storage_slot(token, from);
        let from_balance = state.get_storage(self.contract_address, from_slot)?;

        // Check sufficient balance.
        if from_balance < amount {
            return Ok(OperationResult {
                success: false,
                data: vec![],
            });
        }

        // Update sender balance.
        state.set_storage(self.contract_address, from_slot, from_balance - amount)?;

        // Read and update receiver balance.
        let to_slot = balance_storage_slot(token, to);
        let to_balance = state.get_storage(self.contract_address, to_slot)?;
        state.set_storage(self.contract_address, to_slot, to_balance + amount)?;

        Ok(OperationResult {
            success: true,
            data: vec![],
        })
    }
}

// ── Storage layout helpers ───────────────────────────────────────

/// Compute the storage slot for `balances[token][user]`.
///
/// Solidity layout for `mapping(address => mapping(address => uint256))` at
/// base slot 0:
/// ```text
/// slot = keccak256(abi.encode(user, keccak256(abi.encode(token, 0))))
/// ```
pub fn balance_storage_slot(token: Address, user: Address) -> H256 {
    // Inner: keccak256(abi.encode(token, 0))
    let mut inner_preimage = [0u8; 64];
    inner_preimage[12..32].copy_from_slice(token.as_bytes()); // token left-padded to 32
    // slot 0 is already zero in bytes 32..64

    let inner_hash = keccak_hash(&inner_preimage);

    // Outer: keccak256(abi.encode(user, inner_hash))
    let mut outer_preimage = [0u8; 64];
    outer_preimage[12..32].copy_from_slice(user.as_bytes()); // user left-padded to 32
    outer_preimage[32..64].copy_from_slice(&inner_hash);

    H256::from(keccak_hash(&outer_preimage))
}

// ── ABI helpers ──────────────────────────────────────────────────

/// Extract an address from a 32-byte ABI-encoded word (last 20 bytes).
fn address_from_abi_word(word: &[u8]) -> Address {
    debug_assert!(word.len() >= 32);
    Address::from_slice(&word[12..32])
}

/// Pad an address to a 32-byte H256 (left-padded with zeros).
fn address_to_h256(addr: Address) -> H256 {
    let mut buf = [0u8; 32];
    buf[12..32].copy_from_slice(addr.as_bytes());
    H256::from(buf)
}

/// Build ABI-encoded calldata for `transfer(address,address,uint256)`.
pub fn encode_transfer_calldata(to: Address, token: Address, amount: U256) -> Vec<u8> {
    let mut data = Vec::with_capacity(4 + 96);
    data.extend_from_slice(&transfer_selector());

    // to (address, padded to 32 bytes)
    let mut word = [0u8; 32];
    word[12..32].copy_from_slice(to.as_bytes());
    data.extend_from_slice(&word);

    // token (address, padded to 32 bytes)
    let mut word = [0u8; 32];
    word[12..32].copy_from_slice(token.as_bytes());
    data.extend_from_slice(&word);

    // amount (uint256, 32 bytes big-endian)
    data.extend_from_slice(&amount.to_big_endian());

    data
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::app_types::{AccountProof, StorageProof};
    use ethrex_common::types::EIP1559Transaction;
    use ethrex_common::H160;

    fn dex_address() -> Address {
        H160([0xDE; 20])
    }

    fn token_address() -> Address {
        H160([0xAA; 20])
    }

    fn user_a() -> Address {
        H160([0x01; 20])
    }

    fn user_b() -> Address {
        H160([0x02; 20])
    }

    fn make_circuit() -> DexCircuit {
        DexCircuit {
            contract_address: dex_address(),
        }
    }

    /// Build a minimal EIP-1559 transaction for testing classify_tx.
    /// classify_tx only inspects `tx.to()` and `tx.data()`.
    fn make_test_tx(to: Address, data: Vec<u8>) -> Transaction {
        Transaction::EIP1559Transaction(EIP1559Transaction {
            to: TxKind::Call(to),
            data: Bytes::from(data),
            ..Default::default()
        })
    }

    /// Build an AppState with the DEX contract account and given storage slots.
    fn make_state_with_balances(
        token: Address,
        balances: Vec<(Address, U256)>,
    ) -> AppState {
        let contract = dex_address();

        let account_proofs = vec![AccountProof {
            address: contract,
            nonce: 0,
            balance: U256::zero(),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
            proof: vec![],
        }];

        let storage_proofs: Vec<StorageProof> = balances
            .into_iter()
            .map(|(user, balance)| {
                let slot = balance_storage_slot(token, user);
                StorageProof {
                    address: contract,
                    slot,
                    value: balance,
                    account_proof: vec![],
                    storage_proof: vec![],
                }
            })
            .collect();

        AppState::from_proofs(H256::zero(), account_proofs, storage_proofs)
    }

    // ── classify_tx tests ────────────────────────────────────────

    #[test]
    fn classify_valid_transfer_tx() {
        let circuit = make_circuit();
        let calldata = encode_transfer_calldata(user_b(), token_address(), U256::from(100));
        let tx = make_test_tx(dex_address(), calldata);

        let op = circuit.classify_tx(&tx).expect("should classify");
        assert_eq!(op.op_type, OP_TOKEN_TRANSFER);
        assert_eq!(op.params.len(), 96);
    }

    #[test]
    fn classify_wrong_contract_fails() {
        let circuit = make_circuit();
        let calldata = encode_transfer_calldata(user_b(), token_address(), U256::from(100));
        let wrong_addr = H160([0xFF; 20]);
        let tx = make_test_tx(wrong_addr, calldata);

        assert!(circuit.classify_tx(&tx).is_err());
    }

    #[test]
    fn classify_unknown_selector_fails() {
        let circuit = make_circuit();
        let tx = make_test_tx(dex_address(), vec![0xDE, 0xAD, 0xBE, 0xEF]);

        assert!(circuit.classify_tx(&tx).is_err());
    }

    #[test]
    fn classify_short_calldata_fails() {
        let circuit = make_circuit();
        // Only 2 bytes — too short for a selector.
        let tx = make_test_tx(dex_address(), vec![0x00, 0x01]);

        assert!(circuit.classify_tx(&tx).is_err());
    }

    #[test]
    fn classify_selector_ok_but_params_too_short() {
        let circuit = make_circuit();
        // Valid selector but only 32 bytes of params instead of 96.
        let mut data = transfer_selector().to_vec();
        data.extend_from_slice(&[0u8; 32]);
        let tx = make_test_tx(dex_address(), data);

        assert!(circuit.classify_tx(&tx).is_err());
    }

    // ── execute_operation tests ──────────────────────────────────

    #[test]
    fn execute_successful_transfer() {
        let circuit = make_circuit();
        let token = token_address();
        let mut state = make_state_with_balances(
            token,
            vec![(user_a(), U256::from(1000)), (user_b(), U256::from(500))],
        );

        let calldata = encode_transfer_calldata(user_b(), token, U256::from(300));
        let tx = make_test_tx(dex_address(), calldata);
        let op = circuit.classify_tx(&tx).unwrap();

        let result = circuit
            .execute_operation(&mut state, user_a(), &op)
            .expect("should succeed");
        assert!(result.success);

        // Verify balances updated.
        let a_slot = balance_storage_slot(token, user_a());
        let b_slot = balance_storage_slot(token, user_b());
        assert_eq!(
            state.get_storage(dex_address(), a_slot).unwrap(),
            U256::from(700)
        );
        assert_eq!(
            state.get_storage(dex_address(), b_slot).unwrap(),
            U256::from(800)
        );
    }

    #[test]
    fn execute_insufficient_balance() {
        let circuit = make_circuit();
        let token = token_address();
        let mut state = make_state_with_balances(
            token,
            vec![(user_a(), U256::from(50)), (user_b(), U256::from(0))],
        );

        let calldata = encode_transfer_calldata(user_b(), token, U256::from(100));
        let tx = make_test_tx(dex_address(), calldata);
        let op = circuit.classify_tx(&tx).unwrap();

        let result = circuit
            .execute_operation(&mut state, user_a(), &op)
            .expect("should return failure result, not error");
        assert!(!result.success);

        // Balances unchanged.
        let a_slot = balance_storage_slot(token, user_a());
        assert_eq!(
            state.get_storage(dex_address(), a_slot).unwrap(),
            U256::from(50)
        );
    }

    #[test]
    fn execute_zero_amount_is_noop() {
        let circuit = make_circuit();
        let token = token_address();
        let mut state = make_state_with_balances(
            token,
            vec![(user_a(), U256::from(100)), (user_b(), U256::from(0))],
        );

        let calldata = encode_transfer_calldata(user_b(), token, U256::zero());
        let tx = make_test_tx(dex_address(), calldata);
        let op = circuit.classify_tx(&tx).unwrap();

        let result = circuit
            .execute_operation(&mut state, user_a(), &op)
            .expect("should succeed");
        assert!(result.success);

        // Balances unchanged.
        let a_slot = balance_storage_slot(token, user_a());
        assert_eq!(
            state.get_storage(dex_address(), a_slot).unwrap(),
            U256::from(100)
        );
    }

    #[test]
    fn execute_self_transfer() {
        let circuit = make_circuit();
        let token = token_address();
        let mut state = make_state_with_balances(
            token,
            vec![(user_a(), U256::from(500))],
        );

        let calldata = encode_transfer_calldata(user_a(), token, U256::from(100));
        let tx = make_test_tx(dex_address(), calldata);
        let op = circuit.classify_tx(&tx).unwrap();

        let result = circuit
            .execute_operation(&mut state, user_a(), &op)
            .expect("should succeed");
        assert!(result.success);

        // Balance unchanged (debit then credit same slot).
        let a_slot = balance_storage_slot(token, user_a());
        assert_eq!(
            state.get_storage(dex_address(), a_slot).unwrap(),
            U256::from(500)
        );
    }

    // ── gas_cost tests ───────────────────────────────────────────

    #[test]
    fn gas_cost_token_transfer() {
        let circuit = make_circuit();
        let op = AppOperation {
            op_type: OP_TOKEN_TRANSFER,
            params: vec![0; 96],
        };
        assert_eq!(circuit.gas_cost(&op), TOKEN_TRANSFER_GAS);
        assert_eq!(circuit.gas_cost(&op), 65_000);
    }

    // ── generate_logs tests ──────────────────────────────────────

    #[test]
    fn generate_logs_successful_transfer() {
        let circuit = make_circuit();
        let token = token_address();
        let amount = U256::from(42);
        let calldata = encode_transfer_calldata(user_b(), token, amount);
        // params = calldata[4..]
        let params = calldata[4..].to_vec();

        let op = AppOperation {
            op_type: OP_TOKEN_TRANSFER,
            params,
        };
        let result = OperationResult {
            success: true,
            data: vec![],
        };

        let logs = circuit.generate_logs(user_a(), &op, &result);
        assert_eq!(logs.len(), 1);

        let log = &logs[0];
        assert_eq!(log.address, dex_address());
        assert_eq!(log.topics.len(), 3);
        assert_eq!(log.topics[0], transfer_event_topic());
        assert_eq!(log.topics[1], address_to_h256(user_a()));
        assert_eq!(log.topics[2], address_to_h256(user_b()));

        // Data should be the ABI-encoded amount.
        let expected_amount = amount.to_big_endian();
        assert_eq!(log.data.as_ref(), &expected_amount);
    }

    #[test]
    fn generate_logs_failed_transfer_is_empty() {
        let circuit = make_circuit();
        let op = AppOperation {
            op_type: OP_TOKEN_TRANSFER,
            params: vec![0; 96],
        };
        let result = OperationResult {
            success: false,
            data: vec![],
        };

        let logs = circuit.generate_logs(user_a(), &op, &result);
        assert!(logs.is_empty());
    }

    // ── balance_storage_slot tests ───────────────────────────────

    #[test]
    fn balance_slot_is_deterministic() {
        let slot1 = balance_storage_slot(token_address(), user_a());
        let slot2 = balance_storage_slot(token_address(), user_a());
        assert_eq!(slot1, slot2);
    }

    #[test]
    fn balance_slot_differs_for_different_users() {
        let slot_a = balance_storage_slot(token_address(), user_a());
        let slot_b = balance_storage_slot(token_address(), user_b());
        assert_ne!(slot_a, slot_b);
    }

    #[test]
    fn balance_slot_differs_for_different_tokens() {
        let token1 = H160([0xAA; 20]);
        let token2 = H160([0xBB; 20]);
        let slot1 = balance_storage_slot(token1, user_a());
        let slot2 = balance_storage_slot(token2, user_a());
        assert_ne!(slot1, slot2);
    }
}
