use crate::traits::{GuestProgram, GuestProgramError};

/// Tokamon Guest Program — location-based reward/stamp game.
///
/// This program proves state transitions for the Tokamon game:
/// spot management (createSpotSelf, redepositSelf, updateSpot) and
/// claim processing (claimToTelegram, claimByDevice, claimTelegramToWallet,
/// claimDeviceToWallet) with stamp bonus mechanics.
///
/// Reference: <https://github.com/tokamak-network/tokamon/tree/deploy/thanos-sepolia>
///
/// Currently a stub — no ELF binaries are compiled yet.  The program is
/// registered in the [`GuestProgramRegistry`] so that the proof coordinator
/// can assign batches with `program_id = "tokamon"` and provers can
/// advertise support for it.
pub struct TokammonGuestProgram;

impl GuestProgram for TokammonGuestProgram {
    fn program_id(&self) -> &str {
        "tokamon"
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
        3 // Tokamon
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // Will be replaced with Tokamon-specific input serialization
        // once the input types (spot storage, claim tx batch) are defined.
        Ok(raw_input.to_vec())
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        // Will be replaced with Tokamon-specific output encoding
        // (initial/final state root, processed claim count) once defined.
        Ok(raw_output.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_id_is_tokamon() {
        let gp = TokammonGuestProgram;
        assert_eq!(gp.program_id(), "tokamon");
    }

    #[test]
    fn program_type_id_is_three() {
        let gp = TokammonGuestProgram;
        assert_eq!(gp.program_type_id(), 3);
    }

    #[test]
    fn elf_returns_none() {
        let gp = TokammonGuestProgram;
        assert!(gp.elf("sp1").is_none());
        assert!(gp.elf("risc0").is_none());
    }

    #[test]
    fn serialize_input_is_identity() {
        let gp = TokammonGuestProgram;
        let data = b"test data";
        assert_eq!(gp.serialize_input(data).unwrap(), data);
    }
}
