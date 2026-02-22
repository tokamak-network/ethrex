use crate::traits::{GuestProgram, GuestProgramError};

/// ZK-DEX Guest Program — privacy-preserving decentralized exchange.
///
/// This program proves note-based state transitions for private trading:
/// mint, spend, liquidate (note management) and makeOrder, takeOrder,
/// settleOrder, convertNote (order matching).
///
/// Reference: <https://github.com/tokamak-network/zk-dex/tree/circom>
///
/// Currently a stub — no ELF binaries are compiled yet.  The program is
/// registered in the [`GuestProgramRegistry`] so that the proof coordinator
/// can assign batches with `program_id = "zk-dex"` and provers can
/// advertise support for it.
pub struct ZkDexGuestProgram;

impl GuestProgram for ZkDexGuestProgram {
    fn program_id(&self) -> &str {
        "zk-dex"
    }

    fn elf(&self, _backend: &str) -> Option<&[u8]> {
        // No ELF compiled yet — will be populated when the zkVM
        // entrypoint crate is built (Phase 2.4).
        None
    }

    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
        None
    }

    fn program_type_id(&self) -> u8 {
        2 // ZK-DEX
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // Will be replaced with ZK-DEX-specific input serialization
        // once the input types (note tree, order list, tx batch) are defined.
        Ok(raw_input.to_vec())
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // Will be replaced with ZK-DEX-specific output encoding
        // (initial/final state root, processed tx count) once defined.
        Ok(raw_output.to_vec())
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
}
