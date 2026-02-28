//! Debugger callback trait for per-opcode recording.
//!
//! Feature-gated behind `tokamak-debugger`. When enabled, the VM calls
//! [`OpcodeRecorder::record_step`] before each opcode dispatch, allowing
//! external consumers (e.g. `tokamak-debugger` crate) to capture full
//! execution traces for time-travel replay.

use crate::call_frame::Stack;
use crate::memory::Memory;
use ethrex_common::Address;

/// Callback trait invoked by the interpreter loop before each opcode.
///
/// Implementors capture whatever state they need from the provided arguments.
/// The `stack` reference allows peeking at top-N values without cloning.
/// The `memory` reference allows reading LOG data regions.
pub trait OpcodeRecorder {
    #[allow(clippy::too_many_arguments)]
    fn record_step(
        &mut self,
        opcode: u8,
        pc: usize,
        gas_remaining: i64,
        depth: usize,
        stack: &Stack,
        memory: &Memory,
        code_address: Address,
    );
}
