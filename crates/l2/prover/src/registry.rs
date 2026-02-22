use std::collections::HashMap;
use std::sync::Arc;

use ethrex_guest_program::traits::GuestProgram;

/// Registry mapping `program_id` → [`GuestProgram`] implementations.
///
/// The registry is created once at prover startup and is immutable during
/// the prover's lifetime.  Each registered [`GuestProgram`] provides ELF
/// binaries and serialization logic for a specific guest program type
/// (e.g. `"evm-l2"`, `"transfer"`).
pub struct GuestProgramRegistry {
    programs: HashMap<String, Arc<dyn GuestProgram>>,
    default_program_id: String,
}

impl GuestProgramRegistry {
    /// Create a new empty registry with the given default program id.
    ///
    /// The default is used when a batch does not specify a `program_id`
    /// (backward compatibility with pre-modularization protocol).
    pub fn new(default_program_id: &str) -> Self {
        Self {
            programs: HashMap::new(),
            default_program_id: default_program_id.to_string(),
        }
    }

    /// Register a guest program.  The program's [`GuestProgram::program_id`]
    /// is used as the key; registering a program with a duplicate id replaces
    /// the previous entry.
    pub fn register(&mut self, program: Arc<dyn GuestProgram>) {
        self.programs
            .insert(program.program_id().to_string(), program);
    }

    /// Look up a guest program by id.
    pub fn get(&self, program_id: &str) -> Option<&Arc<dyn GuestProgram>> {
        self.programs.get(program_id)
    }

    /// Return the default guest program, if registered.
    pub fn default_program(&self) -> Option<&Arc<dyn GuestProgram>> {
        self.programs.get(&self.default_program_id)
    }

    /// Return the default program id.
    pub fn default_program_id(&self) -> &str {
        &self.default_program_id
    }

    /// Return all registered program ids.
    pub fn program_ids(&self) -> Vec<&str> {
        self.programs.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_guest_program::traits::GuestProgramError;

    /// Minimal stub for testing the registry.
    struct StubProgram {
        id: &'static str,
    }

    impl GuestProgram for StubProgram {
        fn program_id(&self) -> &str {
            self.id
        }
        fn elf(&self, _backend: &str) -> Option<&[u8]> {
            None
        }
        fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
            None
        }
        fn program_type_id(&self) -> u8 {
            99
        }
        fn serialize_input(&self, raw: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
            Ok(raw.to_vec())
        }
        fn encode_output(&self, raw: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
            Ok(raw.to_vec())
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = GuestProgramRegistry::new("stub-a");
        reg.register(Arc::new(StubProgram { id: "stub-a" }));
        reg.register(Arc::new(StubProgram { id: "stub-b" }));

        assert!(reg.get("stub-a").is_some());
        assert!(reg.get("stub-b").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn default_program() {
        let mut reg = GuestProgramRegistry::new("stub-a");
        reg.register(Arc::new(StubProgram { id: "stub-a" }));

        let default = reg.default_program().expect("default should exist");
        assert_eq!(default.program_id(), "stub-a");
    }

    #[test]
    fn default_program_missing() {
        let reg = GuestProgramRegistry::new("nonexistent");
        assert!(reg.default_program().is_none());
    }

    #[test]
    fn program_ids() {
        let mut reg = GuestProgramRegistry::new("a");
        reg.register(Arc::new(StubProgram { id: "a" }));
        reg.register(Arc::new(StubProgram { id: "b" }));

        let mut ids = reg.program_ids();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn duplicate_registration_replaces() {
        let mut reg = GuestProgramRegistry::new("x");
        reg.register(Arc::new(StubProgram { id: "x" }));
        reg.register(Arc::new(StubProgram { id: "x" }));
        assert_eq!(reg.program_ids().len(), 1);
    }
}
