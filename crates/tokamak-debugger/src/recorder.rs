//! [`OpcodeRecorder`] implementation that captures [`StepRecord`]s.

use crate::types::{ReplayConfig, StepRecord, StorageWrite};
use ethrex_common::{Address, H256, U256};
use ethrex_levm::call_frame::Stack;
use ethrex_levm::debugger_hook::OpcodeRecorder;

// Opcode constants for enrichment
const OP_SSTORE: u8 = 0x55;
const OP_CALL: u8 = 0xF1;
const OP_CALLCODE: u8 = 0xF2;
const OP_CREATE: u8 = 0xF0;
const OP_CREATE2: u8 = 0xF5;
const OP_LOG0: u8 = 0xA0;
const OP_LOG4: u8 = 0xA4;

/// Records each opcode step into a `Vec<StepRecord>`.
pub struct DebugRecorder {
    pub steps: Vec<StepRecord>,
    config: ReplayConfig,
}

impl DebugRecorder {
    pub fn new(config: ReplayConfig) -> Self {
        Self {
            steps: Vec::new(),
            config,
        }
    }

    fn capture_stack_top(&self, stack: &Stack) -> Vec<U256> {
        let depth = stack.len();
        let n = self.config.stack_top_capture.min(depth);
        let mut top = Vec::with_capacity(n);
        for i in 0..n {
            if let Some(val) = stack.peek(i) {
                top.push(val);
            }
        }
        top
    }

    /// Extract call_value for CALL/CREATE opcodes from pre-execution stack.
    fn extract_call_value(opcode: u8, stack: &Stack) -> Option<U256> {
        match opcode {
            // CALL: stack[0]=gas, stack[1]=to, stack[2]=value
            OP_CALL | OP_CALLCODE => stack.peek(2),
            // CREATE/CREATE2: stack[0]=value
            OP_CREATE | OP_CREATE2 => stack.peek(0),
            // DELEGATECALL/STATICCALL don't transfer value
            _ => None,
        }
    }

    /// Extract log topics for LOG0-LOG4 opcodes from pre-execution stack.
    fn extract_log_topics(opcode: u8, stack: &Stack) -> Option<Vec<H256>> {
        if !(OP_LOG0..=OP_LOG4).contains(&opcode) {
            return None;
        }
        let topic_count = (opcode - OP_LOG0) as usize;
        if topic_count == 0 {
            return Some(Vec::new());
        }
        // LOG stack: [offset, size, topic0, topic1, ...]
        let mut topics = Vec::with_capacity(topic_count);
        for i in 0..topic_count {
            if let Some(val) = stack.peek(2 + i) {
                let bytes = val.to_big_endian();
                topics.push(H256::from(bytes));
            }
        }
        Some(topics)
    }

    /// Extract storage write info for SSTORE from pre-execution stack.
    fn extract_sstore(
        opcode: u8,
        stack: &Stack,
        code_address: Address,
    ) -> Option<Vec<StorageWrite>> {
        if opcode != OP_SSTORE {
            return None;
        }
        // SSTORE stack: [key, value]
        let key = stack.peek(0)?;
        let new_value = stack.peek(1)?;
        let slot = H256::from(key.to_big_endian());
        Some(vec![StorageWrite {
            address: code_address,
            slot,
            old_value: U256::zero(), // Filled post-hoc by enrichment
            new_value,
        }])
    }
}

impl OpcodeRecorder for DebugRecorder {
    #[allow(clippy::too_many_arguments)]
    fn record_step(
        &mut self,
        opcode: u8,
        pc: usize,
        gas_remaining: i64,
        depth: usize,
        stack: &Stack,
        memory_size: usize,
        code_address: Address,
    ) {
        let step_index = self.steps.len();
        let stack_top = self.capture_stack_top(stack);
        let stack_depth = stack.len();

        let call_value = Self::extract_call_value(opcode, stack);
        let log_topics = Self::extract_log_topics(opcode, stack);
        let storage_writes = Self::extract_sstore(opcode, stack, code_address);

        self.steps.push(StepRecord {
            step_index,
            pc,
            opcode,
            depth,
            gas_remaining,
            stack_top,
            stack_depth,
            memory_size,
            code_address,
            call_value,
            storage_writes,
            log_topics,
        });
    }
}
