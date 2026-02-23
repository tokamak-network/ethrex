pub mod circuit;

use crate::traits::{GuestProgram, GuestProgramError, ResourceLimits, backends};

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
/// is a pass-through because the prover already supplies the correct bytes.
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
    fn serialize_input_is_identity() {
        let gp = ZkDexGuestProgram;
        let data = b"test data";
        assert_eq!(gp.serialize_input(data).unwrap(), data);
    }
}
