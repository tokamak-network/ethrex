pub mod execution;
pub mod types;

use crate::traits::{GuestProgram, GuestProgramError};

/// ZK-DEX Guest Program — privacy-preserving decentralized exchange.
///
/// This program proves batch token transfer state transitions.  Each batch
/// contains a list of [`types::DexTransfer`] items; the execution function
/// ([`execution::execution_program`]) validates every transfer and computes
/// a deterministic `final_state_root` via chained Keccak-256 hashing.
///
/// Reference: <https://github.com/tokamak-network/zk-dex/tree/circom>
///
/// ## Serialization
///
/// The ZK-DEX guest binary reads rkyv-serialized [`types::DexProgramInput`]
/// from the zkVM stdin.  [`serialize_input`](GuestProgram::serialize_input)
/// is a pass-through because the prover already supplies the correct bytes.
///
/// [`encode_output`](GuestProgram::encode_output) is also a pass-through;
/// the guest binary calls [`types::DexProgramOutput::encode`] internally.
pub struct ZkDexGuestProgram;

impl GuestProgram for ZkDexGuestProgram {
    fn program_id(&self) -> &str {
        "zk-dex"
    }

    fn elf(&self, _backend: &str) -> Option<&[u8]> {
        // ELF binaries will be compiled separately for each zkVM backend
        // and uploaded via the Guest Program Store.
        None
    }

    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
        None
    }

    fn program_type_id(&self) -> u8 {
        2 // ZK-DEX
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // The prover serializes DexProgramInput to rkyv bytes before calling
        // this method.  Pass-through is correct.
        Ok(raw_input.to_vec())
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // The zkVM guest binary calls DexProgramOutput::encode() internally
        // and commits the result as public values.  Pass-through is correct.
        Ok(raw_output.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::execution::execution_program;
    use super::types::{DexProgramInput, DexTransfer};

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
    fn elf_returns_none() {
        let gp = ZkDexGuestProgram;
        assert!(gp.elf("sp1").is_none());
        assert!(gp.elf("risc0").is_none());
    }

    #[test]
    fn serialize_input_is_identity() {
        let gp = ZkDexGuestProgram;
        let data = b"test data";
        assert_eq!(gp.serialize_input(data).unwrap(), data);
    }

    // ── Execution tests ────────────────────────────────────────────

    fn sample_transfer(nonce: u64) -> DexTransfer {
        DexTransfer {
            from: [1u8; 20],
            to: [2u8; 20],
            token: [3u8; 20],
            amount: 100,
            nonce,
        }
    }

    #[test]
    fn execution_produces_deterministic_output() {
        let input = DexProgramInput {
            initial_state_root: [0xAA; 32],
            transfers: vec![sample_transfer(0), sample_transfer(1)],
        };
        let output = execution_program(input.clone()).expect("should succeed");

        assert_eq!(output.initial_state_root, [0xAA; 32]);
        assert_eq!(output.transfer_count, 2);
        assert_ne!(output.final_state_root, output.initial_state_root);

        // Same input must produce the same output (deterministic).
        let output2 = execution_program(input).expect("should succeed");
        assert_eq!(output.final_state_root, output2.final_state_root);
    }

    #[test]
    fn execution_rejects_empty_batch() {
        let input = DexProgramInput {
            initial_state_root: [0; 32],
            transfers: vec![],
        };
        assert!(execution_program(input).is_err());
    }

    #[test]
    fn execution_rejects_zero_amount() {
        let input = DexProgramInput {
            initial_state_root: [0; 32],
            transfers: vec![DexTransfer {
                from: [1; 20],
                to: [2; 20],
                token: [3; 20],
                amount: 0,
                nonce: 0,
            }],
        };
        assert!(execution_program(input).is_err());
    }

    #[test]
    fn execution_rejects_self_transfer() {
        let input = DexProgramInput {
            initial_state_root: [0; 32],
            transfers: vec![DexTransfer {
                from: [1; 20],
                to: [1; 20], // same as from
                token: [3; 20],
                amount: 100,
                nonce: 0,
            }],
        };
        assert!(execution_program(input).is_err());
    }

    #[test]
    fn output_encode_roundtrip() {
        let input = DexProgramInput {
            initial_state_root: [0xBB; 32],
            transfers: vec![sample_transfer(0)],
        };
        let output = execution_program(input).expect("should succeed");
        let encoded = output.encode();
        // 32 (initial) + 32 (final) + 8 (count) = 72 bytes
        assert_eq!(encoded.len(), 72);
        assert_eq!(&encoded[..32], &output.initial_state_root);
        assert_eq!(&encoded[32..64], &output.final_state_root);
    }

    #[test]
    fn rkyv_roundtrip() {
        let input = DexProgramInput {
            initial_state_root: [0xCC; 32],
            transfers: vec![sample_transfer(0), sample_transfer(1)],
        };
        let bytes =
            rkyv::to_bytes::<rkyv::rancor::Error>(&input).expect("rkyv serialize");
        let restored: DexProgramInput =
            rkyv::from_bytes::<DexProgramInput, rkyv::rancor::Error>(&bytes)
                .expect("rkyv deserialize");
        assert_eq!(restored.initial_state_root, input.initial_state_root);
        assert_eq!(restored.transfers.len(), 2);
    }
}
