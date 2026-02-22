/// Well-known backend identifiers used with [`GuestProgram::elf`] and
/// [`GuestProgram::vk_bytes`].
///
/// These string constants allow the `GuestProgram` trait to identify zkVM
/// backends without depending on the `BackendType` enum (which lives in the
/// `ethrex-prover` crate and would create a circular dependency).
pub mod backends {
    pub const SP1: &str = "sp1";
    pub const RISC0: &str = "risc0";
    pub const ZISK: &str = "zisk";
    pub const OPENVM: &str = "openvm";
    pub const EXEC: &str = "exec";
}

/// Error type for guest program operations.
#[derive(Debug, thiserror::Error)]
pub enum GuestProgramError {
    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Unsupported backend: {0}")]
    UnsupportedBackend(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Trait that abstracts a guest program running inside a zkVM.
///
/// Each guest program is a self-contained program compiled to a RISC-V ELF
/// binary.  The prover executes this ELF inside a zkVM (SP1, RISC0, …) and
/// produces a proof.  Different guest programs can implement different
/// validation logic (EVM execution, simple transfers, DEX order matching, etc.)
/// while sharing the same prover infrastructure.
///
/// # Design choices
///
/// The trait operates at the **bytes level** to avoid generic type proliferation
/// through `ProverBackend`.  Each guest program keeps its own strongly-typed
/// `Input`/`Output` types internally, but exposes only `&[u8]` through this
/// trait.
///
/// Backend identification uses `&str` constants (see [`backends`]) rather than
/// an enum to avoid a circular dependency between the `guest-program` and
/// `prover` crates.
///
/// # Object safety
///
/// This trait is object-safe so that [`std::sync::Arc<dyn GuestProgram>`] can
/// be stored in a runtime registry.
pub trait GuestProgram: Send + Sync {
    /// Unique identifier for this guest program (e.g. `"evm-l2"`, `"transfer"`).
    fn program_id(&self) -> &str;

    /// Compiled ELF binary for a given zkVM backend.
    ///
    /// Returns `None` when the requested backend is not supported or the ELF
    /// has not been compiled (e.g. the corresponding feature flag is disabled).
    ///
    /// `backend` should be one of the constants in [`backends`].
    fn elf(&self, backend: &str) -> Option<&[u8]>;

    /// Verification key bytes for a given zkVM backend.
    ///
    /// The exact format depends on the backend:
    /// - SP1: 32-byte `vk.bytes32()` hash
    /// - RISC0: hex-encoded image ID
    ///
    /// Returns `None` when the VK is not available (e.g. SP1 generates VKs at
    /// setup time from the ELF, so a compile-time VK may not exist).
    fn vk_bytes(&self, backend: &str) -> Option<Vec<u8>>;

    /// Integer identifier for this program type on L1.
    ///
    /// Used as the `programTypeId` key in the on-chain `verificationKeys`
    /// mapping: `verificationKeys[commitHash][programTypeId][verifierId]`.
    fn program_type_id(&self) -> u8;

    /// Serialize raw input data into the bytes the guest program expects.
    ///
    /// The default implementation is the identity (pass-through), which is
    /// correct when the caller already supplies bytes in the format the guest
    /// program reads from the zkVM stdin.
    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_input.to_vec())
    }

    /// Encode the zkVM's raw public-values output into the byte layout
    /// expected by the L1 verifier contract.
    ///
    /// The default implementation is the identity (pass-through), which is
    /// correct when the zkVM output already matches the L1 encoding.
    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_output.to_vec())
    }
}
