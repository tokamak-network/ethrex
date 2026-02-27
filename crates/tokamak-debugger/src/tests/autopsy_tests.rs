//! Tests for the Smart Contract Autopsy Lab.
//!
//! These tests use synthetic traces (no network calls required).

use bytes::Bytes;
use ethrex_common::{Address, H256, U256};

use crate::types::{ReplayConfig, ReplayTrace, StepRecord, StorageWrite};

use crate::autopsy::{
    classifier::AttackClassifier,
    enrichment::{collect_sstore_slots, enrich_storage_writes},
    fund_flow::FundFlowTracer,
    report::AutopsyReport,
    types::{AttackPattern, Severity},
};

// ============================================================
// Helpers to build synthetic StepRecords
// ============================================================

fn make_step(index: usize, opcode: u8, depth: usize, code_address: Address) -> StepRecord {
    StepRecord {
        step_index: index,
        pc: index * 2,
        opcode,
        depth,
        gas_remaining: 1_000_000 - (index as i64 * 100),
        stack_top: vec![],
        stack_depth: 0,
        memory_size: 0,
        code_address,
        call_value: None,
        storage_writes: None,
        log_topics: None,
    }
}

fn make_call_step(
    index: usize,
    depth: usize,
    from: Address,
    to: Address,
    value: U256,
) -> StepRecord {
    // CALL: stack = [gas, to, value, ...]
    let to_u256 = U256::from_big_endian(to.as_bytes());
    StepRecord {
        step_index: index,
        pc: index * 2,
        opcode: 0xF1, // CALL
        depth,
        gas_remaining: 1_000_000,
        stack_top: vec![U256::from(100_000), to_u256, value],
        stack_depth: 7,
        memory_size: 0,
        code_address: from,
        call_value: Some(value),
        storage_writes: None,
        log_topics: None,
    }
}

fn make_sstore_step(
    index: usize,
    depth: usize,
    address: Address,
    slot: H256,
    new_value: U256,
) -> StepRecord {
    StepRecord {
        step_index: index,
        pc: index * 2,
        opcode: 0x55, // SSTORE
        depth,
        gas_remaining: 1_000_000,
        stack_top: vec![],
        stack_depth: 2,
        memory_size: 0,
        code_address: address,
        call_value: None,
        storage_writes: Some(vec![StorageWrite {
            address,
            slot,
            old_value: U256::zero(),
            new_value,
        }]),
        log_topics: None,
    }
}

fn make_staticcall_step(index: usize, depth: usize, from: Address, to: Address) -> StepRecord {
    let to_u256 = U256::from_big_endian(to.as_bytes());
    StepRecord {
        step_index: index,
        pc: index * 2,
        opcode: 0xFA, // STATICCALL
        depth,
        gas_remaining: 1_000_000,
        stack_top: vec![U256::from(100_000), to_u256],
        stack_depth: 6,
        memory_size: 0,
        code_address: from,
        call_value: None,
        storage_writes: None,
        log_topics: None,
    }
}

fn transfer_topic() -> H256 {
    // keccak256("Transfer(address,address,uint256)")
    // = 0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef
    let mut bytes = [0u8; 32];
    bytes[0] = 0xdd;
    bytes[1] = 0xf2;
    bytes[2] = 0x52;
    bytes[3] = 0xad;
    H256::from(bytes)
}

fn make_log3_transfer(
    index: usize,
    depth: usize,
    token: Address,
    from: Address,
    to: Address,
) -> StepRecord {
    let mut from_bytes = [0u8; 32];
    from_bytes[12..].copy_from_slice(from.as_bytes());
    let mut to_bytes = [0u8; 32];
    to_bytes[12..].copy_from_slice(to.as_bytes());

    StepRecord {
        step_index: index,
        pc: index * 2,
        opcode: 0xA3, // LOG3
        depth,
        gas_remaining: 1_000_000,
        stack_top: vec![],
        stack_depth: 5,
        memory_size: 64,
        code_address: token,
        call_value: None,
        storage_writes: None,
        log_topics: Some(vec![
            transfer_topic(),
            H256::from(from_bytes),
            H256::from(to_bytes),
        ]),
    }
}

fn make_caller_step(index: usize, depth: usize, address: Address) -> StepRecord {
    make_step_with_opcode(index, 0x33, depth, address) // CALLER
}

fn make_step_with_opcode(index: usize, opcode: u8, depth: usize, address: Address) -> StepRecord {
    make_step(index, opcode, depth, address)
}

fn addr(n: u64) -> Address {
    Address::from_low_u64_be(n)
}

fn slot(n: u64) -> H256 {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    H256::from(bytes)
}

// ============================================================
// RPC Client Parsing Tests (already inline in rpc_client.rs)
// ============================================================

// The rpc_client.rs has its own #[cfg(test)] module with 12 tests.
// Here we test the higher-level autopsy components.

// ============================================================
// Enrichment Tests
// ============================================================

#[test]
fn test_enrich_no_sstores() {
    let mut trace = ReplayTrace {
        steps: vec![make_step(0, 0x00, 0, addr(1))],
        config: ReplayConfig::default(),
        gas_used: 21000,
        success: true,
        output: Bytes::new(),
    };
    let initial = rustc_hash::FxHashMap::default();
    enrich_storage_writes(&mut trace, &initial);
    // No change — no SSTORE steps
    assert!(trace.steps[0].storage_writes.is_none());
}

#[test]
fn test_enrich_single_sstore_with_initial() {
    let contract = addr(0x42);
    let s = slot(1);
    let mut trace = ReplayTrace {
        steps: vec![make_sstore_step(0, 0, contract, s, U256::from(100))],
        config: ReplayConfig::default(),
        gas_used: 21000,
        success: true,
        output: Bytes::new(),
    };
    let mut initial = rustc_hash::FxHashMap::default();
    initial.insert((contract, s), U256::from(50));

    enrich_storage_writes(&mut trace, &initial);

    let write = &trace.steps[0].storage_writes.as_ref().unwrap()[0];
    assert_eq!(write.old_value, U256::from(50));
    assert_eq!(write.new_value, U256::from(100));
}

#[test]
fn test_enrich_chained_sstores() {
    let contract = addr(0x42);
    let s = slot(1);
    let mut trace = ReplayTrace {
        steps: vec![
            make_sstore_step(0, 0, contract, s, U256::from(10)),
            make_sstore_step(1, 0, contract, s, U256::from(20)),
            make_sstore_step(2, 0, contract, s, U256::from(30)),
        ],
        config: ReplayConfig::default(),
        gas_used: 21000,
        success: true,
        output: Bytes::new(),
    };
    let initial = rustc_hash::FxHashMap::default();
    enrich_storage_writes(&mut trace, &initial);

    let w0 = &trace.steps[0].storage_writes.as_ref().unwrap()[0];
    assert_eq!(w0.old_value, U256::zero()); // No initial, defaults to zero
    assert_eq!(w0.new_value, U256::from(10));

    let w1 = &trace.steps[1].storage_writes.as_ref().unwrap()[0];
    assert_eq!(w1.old_value, U256::from(10)); // Previous write's new_value
    assert_eq!(w1.new_value, U256::from(20));

    let w2 = &trace.steps[2].storage_writes.as_ref().unwrap()[0];
    assert_eq!(w2.old_value, U256::from(20));
    assert_eq!(w2.new_value, U256::from(30));
}

#[test]
fn test_collect_sstore_slots_deduplicates() {
    let contract = addr(0x42);
    let s = slot(1);
    let steps = vec![
        make_sstore_step(0, 0, contract, s, U256::from(10)),
        make_sstore_step(1, 0, contract, s, U256::from(20)),
        make_sstore_step(2, 0, contract, slot(2), U256::from(30)),
    ];
    let slots = collect_sstore_slots(&steps);
    assert_eq!(slots.len(), 2); // slot(1) deduplicated
}

// ============================================================
// Classifier Tests
// ============================================================

#[test]
fn test_classify_empty_trace() {
    let patterns = AttackClassifier::classify(&[]);
    assert!(patterns.is_empty());
}

#[test]
fn test_classify_no_attacks() {
    let steps = vec![
        make_step(0, 0x60, 0, addr(1)), // PUSH1
        make_step(1, 0x01, 0, addr(1)), // ADD
        make_step(2, 0x00, 0, addr(1)), // STOP
    ];
    let patterns = AttackClassifier::classify(&steps);
    assert!(patterns.is_empty());
}

#[test]
fn test_detect_reentrancy() {
    let victim = addr(0x42);
    let attacker = addr(0x99);

    let steps = vec![
        // Victim calls attacker at depth 0
        make_call_step(0, 0, victim, attacker, U256::zero()),
        // Attacker executes at depth 1
        make_step(1, 0x60, 1, attacker),
        // Attacker re-enters victim via CALL at depth 1
        make_call_step(2, 1, attacker, victim, U256::zero()),
        // Victim executes at depth 2
        make_step(3, 0x60, 2, victim),
        // Victim does SSTORE during re-entry
        make_sstore_step(4, 2, victim, slot(1), U256::from(999)),
    ];

    let patterns = AttackClassifier::classify(&steps);
    assert!(!patterns.is_empty());
    assert!(matches!(patterns[0], AttackPattern::Reentrancy { .. }));
}

#[test]
fn test_detect_flash_loan() {
    let lender = addr(0x10);
    let borrower = addr(0x20);

    // Total 100 steps, borrow in first quarter, repay in last quarter
    let mut steps: Vec<StepRecord> = Vec::new();

    // Step 0: Large borrow (first quarter of 100 = step 0..25)
    steps.push(make_call_step(
        0,
        0,
        lender,
        borrower,
        U256::from(1_000_000),
    ));

    // Fill middle with NOPs
    for i in 1..80 {
        steps.push(make_step(i, 0x00, 0, borrower));
    }

    // Step 80: Repay in last quarter (75..100)
    steps.push(make_call_step(
        80,
        0,
        borrower,
        lender,
        U256::from(1_000_100),
    ));

    // Fill rest
    for i in 81..100 {
        steps.push(make_step(i, 0x00, 0, borrower));
    }

    let patterns = AttackClassifier::classify(&steps);
    let flash_loans: Vec<_> = patterns
        .iter()
        .filter(|p| matches!(p, AttackPattern::FlashLoan { .. }))
        .collect();
    assert!(!flash_loans.is_empty());
}

#[test]
fn test_detect_price_manipulation() {
    let oracle = addr(0x50);
    let dex = addr(0x60);
    let victim_contract = addr(0x42);

    let steps = vec![
        // Read oracle price
        make_staticcall_step(0, 0, victim_contract, oracle),
        // Swap on DEX (LOG3 Transfer)
        make_log3_transfer(1, 0, dex, addr(0xA), addr(0xB)),
        // Read oracle price again
        make_staticcall_step(2, 0, victim_contract, oracle),
    ];

    let patterns = AttackClassifier::classify(&steps);
    let price_manip: Vec<_> = patterns
        .iter()
        .filter(|p| matches!(p, AttackPattern::PriceManipulation { .. }))
        .collect();
    assert!(!price_manip.is_empty());
}

#[test]
fn test_detect_access_control_bypass() {
    let contract = addr(0x42);

    let steps = vec![
        // SSTORE without any CALLER check
        make_sstore_step(0, 0, contract, slot(1), U256::from(1)),
    ];

    let patterns = AttackClassifier::classify(&steps);
    let bypasses: Vec<_> = patterns
        .iter()
        .filter(|p| matches!(p, AttackPattern::AccessControlBypass { .. }))
        .collect();
    assert!(!bypasses.is_empty());
}

#[test]
fn test_no_access_control_bypass_with_caller_check() {
    let contract = addr(0x42);

    let steps = vec![
        make_caller_step(0, 0, contract),                         // CALLER
        make_sstore_step(1, 0, contract, slot(1), U256::from(1)), // SSTORE
    ];

    let patterns = AttackClassifier::classify(&steps);
    let bypasses: Vec<_> = patterns
        .iter()
        .filter(|p| matches!(p, AttackPattern::AccessControlBypass { .. }))
        .collect();
    assert!(bypasses.is_empty()); // CALLER check present → no bypass
}

// ============================================================
// Fund Flow Tests
// ============================================================

#[test]
fn test_trace_empty() {
    let flows = FundFlowTracer::trace(&[]);
    assert!(flows.is_empty());
}

#[test]
fn test_trace_eth_transfer() {
    let from = addr(0x42);
    let to = addr(0x99);

    let steps = vec![make_call_step(0, 0, from, to, U256::from(1_000_000))];

    let flows = FundFlowTracer::trace(&steps);
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].from, from);
    assert_eq!(flows[0].to, to);
    assert_eq!(flows[0].value, U256::from(1_000_000));
    assert!(flows[0].token.is_none()); // ETH transfer
}

#[test]
fn test_trace_zero_value_call_excluded() {
    let steps = vec![make_call_step(0, 0, addr(1), addr(2), U256::zero())];
    let flows = FundFlowTracer::trace(&steps);
    assert!(flows.is_empty()); // Zero-value calls are not fund flows
}

#[test]
fn test_trace_erc20_transfer() {
    let token = addr(0xDEAD);
    let from = addr(0xA);
    let to = addr(0xB);

    let steps = vec![make_log3_transfer(0, 0, token, from, to)];

    let flows = FundFlowTracer::trace(&steps);
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].from, from);
    assert_eq!(flows[0].to, to);
    assert_eq!(flows[0].token, Some(token));
}

#[test]
fn test_trace_mixed_eth_and_erc20() {
    let steps = vec![
        make_call_step(0, 0, addr(1), addr(2), U256::from(500)),
        make_log3_transfer(1, 0, addr(0xDEAD), addr(3), addr(4)),
    ];

    let flows = FundFlowTracer::trace(&steps);
    assert_eq!(flows.len(), 2);
    // Should be sorted by step_index
    assert!(flows[0].token.is_none()); // ETH first
    assert!(flows[1].token.is_some()); // ERC-20 second
}

// ============================================================
// Report Tests
// ============================================================

#[test]
fn test_report_empty() {
    let report = AutopsyReport::build(H256::zero(), 12345, 0, vec![], vec![], vec![]);
    assert_eq!(report.total_steps, 0);
    assert!(report.attack_patterns.is_empty());
    assert!(report.fund_flows.is_empty());
    assert!(report.key_steps.is_empty());
    assert!(report.suggested_fixes.is_empty());
    assert!(report.summary.contains("No attack patterns"));
}

#[test]
fn test_report_with_reentrancy() {
    let patterns = vec![AttackPattern::Reentrancy {
        target_contract: addr(0x42),
        reentrant_call_step: 10,
        state_modified_step: 15,
        call_depth_at_entry: 1,
    }];

    let report = AutopsyReport::build(H256::zero(), 100, 50, patterns, vec![], vec![]);

    assert_eq!(report.attack_patterns.len(), 1);
    assert!(report.summary.contains("Reentrancy"));
    assert!(!report.suggested_fixes.is_empty());
    assert!(
        report
            .suggested_fixes
            .iter()
            .any(|f| f.contains("ReentrancyGuard"))
    );
}

#[test]
fn test_report_json_roundtrip() {
    let report = AutopsyReport::build(H256::zero(), 100, 10, vec![], vec![], vec![]);
    let json = report.to_json().expect("should serialize");
    assert!(json.contains("\"block_number\""));
    assert!(json.contains("\"total_steps\""));
    assert!(json.contains("\"summary\""));
}

#[test]
fn test_report_markdown_sections() {
    let patterns = vec![AttackPattern::AccessControlBypass {
        sstore_step: 5,
        contract: addr(0x42),
    }];

    let flows = vec![super::super::autopsy::types::FundFlow {
        from: addr(1),
        to: addr(2),
        value: U256::from(100),
        token: None,
        step_index: 3,
    }];

    let diffs = vec![StorageWrite {
        address: addr(0x42),
        slot: slot(1),
        old_value: U256::from(0),
        new_value: U256::from(1),
    }];

    let report = AutopsyReport::build(H256::zero(), 100, 20, patterns, flows, diffs);
    let md = report.to_markdown();

    assert!(md.contains("# Smart Contract Autopsy Report"));
    assert!(md.contains("## Attack Patterns Detected"));
    assert!(md.contains("## Fund Flow"));
    assert!(md.contains("## Storage Changes"));
    assert!(md.contains("## Key Steps"));
    assert!(md.contains("## Suggested Fixes"));
}

#[test]
fn test_report_key_steps_sorted() {
    let patterns = vec![
        AttackPattern::Reentrancy {
            target_contract: addr(0x42),
            reentrant_call_step: 20,
            state_modified_step: 30,
            call_depth_at_entry: 1,
        },
        AttackPattern::AccessControlBypass {
            sstore_step: 5,
            contract: addr(0x42),
        },
    ];

    let report = AutopsyReport::build(H256::zero(), 100, 50, patterns, vec![], vec![]);

    // Key steps should be sorted by step_index
    let indices: Vec<usize> = report.key_steps.iter().map(|s| s.step_index).collect();
    for i in 1..indices.len() {
        assert!(indices[i] >= indices[i - 1], "key_steps not sorted");
    }
}

#[test]
fn test_report_affected_contracts_deduped() {
    let flows = vec![
        super::super::autopsy::types::FundFlow {
            from: addr(1),
            to: addr(2),
            value: U256::from(100),
            token: None,
            step_index: 0,
        },
        super::super::autopsy::types::FundFlow {
            from: addr(1),
            to: addr(3),
            value: U256::from(200),
            token: None,
            step_index: 1,
        },
    ];

    let report = AutopsyReport::build(H256::zero(), 100, 10, vec![], flows, vec![]);

    // addr(1) should appear only once
    let count = report
        .affected_contracts
        .iter()
        .filter(|&&a| a == addr(1))
        .count();
    assert_eq!(count, 1);
}

#[test]
fn test_report_severity_levels() {
    let patterns = vec![AttackPattern::Reentrancy {
        target_contract: addr(0x42),
        reentrant_call_step: 10,
        state_modified_step: 15,
        call_depth_at_entry: 1,
    }];

    let report = AutopsyReport::build(H256::zero(), 100, 50, patterns, vec![], vec![]);

    assert!(
        report
            .key_steps
            .iter()
            .any(|s| s.severity == Severity::Critical)
    );
}

// ============================================================
// StepRecord New Fields Tests
// ============================================================

#[test]
fn test_step_record_none_fields_skip_serializing() {
    let step = StepRecord {
        step_index: 0,
        pc: 0,
        opcode: 0x00,
        depth: 0,
        gas_remaining: 21000,
        stack_top: vec![],
        stack_depth: 0,
        memory_size: 0,
        code_address: Address::zero(),
        call_value: None,
        storage_writes: None,
        log_topics: None,
    };
    let json = serde_json::to_string(&step).unwrap();
    assert!(!json.contains("call_value"));
    assert!(!json.contains("storage_writes"));
    assert!(!json.contains("log_topics"));
}

#[test]
fn test_step_record_some_fields_serialize() {
    let step = StepRecord {
        step_index: 0,
        pc: 0,
        opcode: 0xF1,
        depth: 0,
        gas_remaining: 21000,
        stack_top: vec![],
        stack_depth: 0,
        memory_size: 0,
        code_address: Address::zero(),
        call_value: Some(U256::from(1000)),
        storage_writes: None,
        log_topics: None,
    };
    let json = serde_json::to_string(&step).unwrap();
    assert!(json.contains("call_value"));
    assert!(!json.contains("storage_writes"));
    assert!(!json.contains("log_topics"));
}

// ============================================================
// Integration: Recorder enrichment of CALL/LOG/SSTORE
// ============================================================

#[test]
fn test_recorder_captures_call_value() {
    use crate::recorder::DebugRecorder;
    use crate::types::ReplayConfig;
    use ethrex_levm::call_frame::Stack;
    use ethrex_levm::debugger_hook::OpcodeRecorder;

    let mut recorder = DebugRecorder::new(ReplayConfig::default());
    let mut stack = Stack::default();

    // CALL stack: gas, to, value, ...
    stack.push(U256::from(0)).unwrap(); // retSize
    stack.push(U256::from(0)).unwrap(); // retOffset
    stack.push(U256::from(0)).unwrap(); // argsSize
    stack.push(U256::from(0)).unwrap(); // argsOffset
    stack.push(U256::from(5000)).unwrap(); // value
    stack.push(U256::from(0x99)).unwrap(); // to
    stack.push(U256::from(100_000)).unwrap(); // gas

    recorder.record_step(0xF1, 0, 1_000_000, 0, &stack, 0, addr(0x42));

    assert_eq!(recorder.steps.len(), 1);
    assert_eq!(recorder.steps[0].call_value, Some(U256::from(5000)));
}

#[test]
fn test_recorder_captures_log_topics() {
    use crate::recorder::DebugRecorder;
    use crate::types::ReplayConfig;
    use ethrex_levm::call_frame::Stack;
    use ethrex_levm::debugger_hook::OpcodeRecorder;

    let mut recorder = DebugRecorder::new(ReplayConfig::default());
    let mut stack = Stack::default();

    let topic = U256::from(0xDEADBEEF_u64);
    // LOG1 stack: offset, size, topic0
    stack.push(topic).unwrap(); // topic0
    stack.push(U256::from(32)).unwrap(); // size
    stack.push(U256::from(0)).unwrap(); // offset

    recorder.record_step(0xA1, 0, 1_000_000, 0, &stack, 64, addr(0x42));

    assert_eq!(recorder.steps.len(), 1);
    let topics = recorder.steps[0].log_topics.as_ref().unwrap();
    assert_eq!(topics.len(), 1);
}

#[test]
fn test_recorder_captures_sstore() {
    use crate::recorder::DebugRecorder;
    use crate::types::ReplayConfig;
    use ethrex_levm::call_frame::Stack;
    use ethrex_levm::debugger_hook::OpcodeRecorder;

    let mut recorder = DebugRecorder::new(ReplayConfig::default());
    let mut stack = Stack::default();

    // SSTORE stack: key, value
    stack.push(U256::from(42)).unwrap(); // value
    stack.push(U256::from(1)).unwrap(); // key

    recorder.record_step(0x55, 0, 1_000_000, 0, &stack, 0, addr(0x42));

    assert_eq!(recorder.steps.len(), 1);
    let writes = recorder.steps[0].storage_writes.as_ref().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].address, addr(0x42));
    assert_eq!(writes[0].new_value, U256::from(42));
    assert_eq!(writes[0].old_value, U256::zero()); // Not enriched yet
}

#[test]
fn test_recorder_non_special_opcode_has_no_extras() {
    use crate::recorder::DebugRecorder;
    use crate::types::ReplayConfig;
    use ethrex_levm::call_frame::Stack;
    use ethrex_levm::debugger_hook::OpcodeRecorder;

    let mut recorder = DebugRecorder::new(ReplayConfig::default());
    let stack = Stack::default();

    recorder.record_step(0x01, 0, 1_000_000, 0, &stack, 0, addr(0x42)); // ADD

    assert!(recorder.steps[0].call_value.is_none());
    assert!(recorder.steps[0].storage_writes.is_none());
    assert!(recorder.steps[0].log_topics.is_none());
}
