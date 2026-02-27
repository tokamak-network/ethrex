//! Attack pattern classifier.
//!
//! Analyzes execution traces to detect common DeFi attack patterns:
//! reentrancy, flash loans, price manipulation, and access control bypasses.

use ethrex_common::{Address, U256};
use rustc_hash::FxHashMap;

use crate::types::StepRecord;

use super::types::AttackPattern;

// Opcode constants
const OP_SSTORE: u8 = 0x55;
const OP_CALL: u8 = 0xF1;
const OP_CALLCODE: u8 = 0xF2;
const OP_DELEGATECALL: u8 = 0xF4;
const OP_STATICCALL: u8 = 0xFA;
const OP_CALLER: u8 = 0x33;
const OP_LOG3: u8 = 0xA3;

/// Stateless attack pattern classifier.
pub struct AttackClassifier;

impl AttackClassifier {
    /// Analyze a trace and return all detected attack patterns.
    pub fn classify(steps: &[StepRecord]) -> Vec<AttackPattern> {
        let mut patterns = Vec::new();
        patterns.extend(Self::detect_reentrancy(steps));
        patterns.extend(Self::detect_flash_loan(steps));
        patterns.extend(Self::detect_price_manipulation(steps));
        patterns.extend(Self::detect_access_control_bypass(steps));
        patterns
    }

    /// Detect reentrancy: CALL at depth D to address A, then steps at depth > D
    /// with same code_address A, followed by SSTORE after re-entry.
    fn detect_reentrancy(steps: &[StepRecord]) -> Vec<AttackPattern> {
        let mut patterns = Vec::new();

        // Build a list of external calls with their targets and depths
        let calls: Vec<(usize, Address, usize)> = steps
            .iter()
            .filter(|s| is_call_opcode(s.opcode))
            .filter_map(|s| {
                // For CALL/CALLCODE, target address is stack[1] (to)
                let target = extract_call_target(s)?;
                Some((s.step_index, target, s.depth))
            })
            .collect();

        for &(call_idx, _target, call_depth) in &calls {
            // The caller (potential victim) is the contract that made this CALL
            let caller_address = steps
                .get(call_idx)
                .map(|s| s.code_address)
                .unwrap_or(Address::zero());

            // Look for re-entry: a subsequent CALL at deeper depth that
            // targets the original caller (victim) address
            let reentry_step = calls.iter().find(|&&(idx, tgt, depth)| {
                idx > call_idx && depth > call_depth && tgt == caller_address
            });

            if let Some(&(reentry_idx, _, _)) = reentry_step {
                // Look for SSTORE after re-entry in the victim contract
                let sstore_after = steps[reentry_idx..]
                    .iter()
                    .find(|s| s.opcode == OP_SSTORE && s.code_address == caller_address);

                if let Some(sstore) = sstore_after {
                    patterns.push(AttackPattern::Reentrancy {
                        target_contract: caller_address,
                        reentrant_call_step: reentry_idx,
                        state_modified_step: sstore.step_index,
                        call_depth_at_entry: call_depth,
                    });
                }
            }
        }

        patterns
    }

    /// Detect flash loan: large ETH CALL value early → matching reverse transfer near end.
    fn detect_flash_loan(steps: &[StepRecord]) -> Vec<AttackPattern> {
        let mut patterns = Vec::new();
        let total = steps.len();
        if total < 4 {
            return patterns;
        }

        // Find large value transfers in first quarter of trace
        let first_quarter = total / 4;
        let last_quarter_start = total - (total / 4);

        let borrows: Vec<(usize, U256)> = steps[..first_quarter.min(steps.len())]
            .iter()
            .filter_map(|s| {
                let value = s.call_value.as_ref()?;
                if *value > U256::zero() {
                    Some((s.step_index, *value))
                } else {
                    None
                }
            })
            .collect();

        for &(borrow_idx, borrow_amount) in &borrows {
            // Look for repayment of similar size in last quarter
            let repay = steps[last_quarter_start..].iter().find(|s| {
                if let Some(value) = &s.call_value {
                    // Repay amount should be >= borrow (borrow + fee)
                    *value >= borrow_amount && s.step_index > borrow_idx
                } else {
                    false
                }
            });

            if let Some(repay_step) = repay {
                patterns.push(AttackPattern::FlashLoan {
                    borrow_step: borrow_idx,
                    borrow_amount,
                    repay_step: repay_step.step_index,
                    repay_amount: repay_step.call_value.unwrap_or(U256::zero()),
                });
            }
        }

        patterns
    }

    /// Detect price manipulation: STATICCALL to same address twice with
    /// a LOG3 Transfer event between them (indicating a swap).
    fn detect_price_manipulation(steps: &[StepRecord]) -> Vec<AttackPattern> {
        let mut patterns = Vec::new();

        // Find pairs of STATICCALL to same address with a Transfer event between
        let static_calls: Vec<(usize, Address)> = steps
            .iter()
            .filter(|s| s.opcode == OP_STATICCALL)
            .filter_map(|s| {
                let target = extract_call_target_static(s)?;
                Some((s.step_index, target))
            })
            .collect();

        // Find LOG3 Transfer events (ERC-20 Transfer topic)
        let transfers: Vec<usize> = steps
            .iter()
            .filter(|s| s.opcode == OP_LOG3 && has_transfer_topic(s))
            .map(|s| s.step_index)
            .collect();

        for i in 0..static_calls.len() {
            let (read1_idx, oracle_addr) = static_calls[i];

            // Find a transfer event after this read
            let swap_idx = transfers.iter().find(|&&t| t > read1_idx);
            let Some(&swap_step) = swap_idx else {
                continue;
            };

            // Find second read to same oracle after the swap
            let read2 = static_calls[i + 1..]
                .iter()
                .find(|&&(idx, addr)| idx > swap_step && addr == oracle_addr);

            if let Some(&(read2_idx, _)) = read2 {
                patterns.push(AttackPattern::PriceManipulation {
                    oracle_read_before: read1_idx,
                    swap_step,
                    oracle_read_after: read2_idx,
                    price_delta_percent: 0.0, // Would need storage reads to calculate
                });
            }
        }

        patterns
    }

    /// Detect access control bypass: SSTORE without CALLER (0x33) check
    /// in the same call frame depth.
    fn detect_access_control_bypass(steps: &[StepRecord]) -> Vec<AttackPattern> {
        let mut patterns = Vec::new();

        // Group steps by (code_address, depth) to represent call frames
        let mut frames: FxHashMap<(Address, usize), FrameInfo> = FxHashMap::default();

        for step in steps {
            let key = (step.code_address, step.depth);
            let frame = frames.entry(key).or_insert_with(|| FrameInfo {
                has_caller_check: false,
                sstore_steps: Vec::new(),
            });

            if step.opcode == OP_CALLER {
                frame.has_caller_check = true;
            }
            if step.opcode == OP_SSTORE {
                frame.sstore_steps.push(step.step_index);
            }
        }

        // Flag frames with SSTORE but no CALLER check
        for ((contract, _depth), frame) in &frames {
            if !frame.has_caller_check && !frame.sstore_steps.is_empty() {
                for &sstore_step in &frame.sstore_steps {
                    patterns.push(AttackPattern::AccessControlBypass {
                        sstore_step,
                        contract: *contract,
                    });
                }
            }
        }

        patterns
    }
}

struct FrameInfo {
    has_caller_check: bool,
    sstore_steps: Vec<usize>,
}

fn is_call_opcode(op: u8) -> bool {
    matches!(op, OP_CALL | OP_CALLCODE | OP_DELEGATECALL | OP_STATICCALL)
}

/// Extract target address from CALL/CALLCODE stack: stack[1] = to address.
fn extract_call_target(step: &StepRecord) -> Option<Address> {
    let val = step.stack_top.get(1)?;
    let bytes = val.to_big_endian();
    Some(Address::from_slice(&bytes[12..]))
}

/// Extract target address from STATICCALL/DELEGATECALL stack: stack[1] = to address.
fn extract_call_target_static(step: &StepRecord) -> Option<Address> {
    let val = step.stack_top.get(1)?;
    let bytes = val.to_big_endian();
    Some(Address::from_slice(&bytes[12..]))
}

/// Check if a LOG step has the ERC-20 Transfer event topic.
fn has_transfer_topic(step: &StepRecord) -> bool {
    if let Some(topics) = &step.log_topics {
        // Transfer(address,address,uint256) = 0xddf252ad...
        topics.first().is_some_and(|t| {
            t.as_bytes()[0] == 0xdd
                && t.as_bytes()[1] == 0xf2
                && t.as_bytes()[2] == 0x52
                && t.as_bytes()[3] == 0xad
        })
    } else {
        false
    }
}
