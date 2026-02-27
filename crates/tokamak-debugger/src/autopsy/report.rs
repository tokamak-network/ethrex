//! Autopsy report generation (JSON + Markdown).

use ethrex_common::{Address, H256, U256};
use serde::Serialize;

use crate::types::{StepRecord, StorageWrite};

use super::types::{AnnotatedStep, AttackPattern, FundFlow, Severity};

/// Execution statistics derived from the opcode trace.
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionOverview {
    pub max_call_depth: usize,
    pub unique_contracts: usize,
    pub call_count: usize,
    pub sstore_count: usize,
    pub sload_count: usize,
    pub log_count: usize,
    pub create_count: usize,
    /// Top 5 most frequent opcodes: (opcode_byte, name, count)
    pub top_opcodes: Vec<(u8, String, usize)>,
}

/// Complete autopsy report for a single transaction.
#[derive(Debug, Clone, Serialize)]
pub struct AutopsyReport {
    pub tx_hash: H256,
    pub block_number: u64,
    pub summary: String,
    pub execution_overview: ExecutionOverview,
    pub attack_patterns: Vec<AttackPattern>,
    pub fund_flows: Vec<FundFlow>,
    pub storage_diffs: Vec<StorageWrite>,
    pub total_steps: usize,
    pub key_steps: Vec<AnnotatedStep>,
    pub affected_contracts: Vec<Address>,
    pub suggested_fixes: Vec<String>,
}

impl AutopsyReport {
    /// Build a report from analysis results.
    pub fn build(
        tx_hash: H256,
        block_number: u64,
        steps: &[StepRecord],
        attack_patterns: Vec<AttackPattern>,
        fund_flows: Vec<FundFlow>,
        storage_diffs: Vec<StorageWrite>,
    ) -> Self {
        let total_steps = steps.len();
        let execution_overview = Self::compute_overview(steps);
        let key_steps = Self::identify_key_steps(&attack_patterns, &fund_flows);
        let affected_contracts = Self::collect_affected_contracts(&fund_flows, &storage_diffs);
        let suggested_fixes = Self::suggest_fixes(&attack_patterns);
        let summary = Self::generate_summary(&attack_patterns, &fund_flows, &execution_overview);

        Self {
            tx_hash,
            block_number,
            summary,
            execution_overview,
            attack_patterns,
            fund_flows,
            storage_diffs,
            total_steps,
            key_steps,
            affected_contracts,
            suggested_fixes,
        }
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Render as Markdown report.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str("# Smart Contract Autopsy Report\n\n");
        md.push_str(&format!("**Transaction**: `0x{:x}`\n", self.tx_hash));
        md.push_str(&format!("**Block**: {}\n", self.block_number));
        md.push_str(&format!("**Total Steps**: {}\n\n", self.total_steps));

        // Summary
        md.push_str("## Summary\n\n");
        md.push_str(&self.summary);
        md.push_str("\n\n");

        // Execution Overview (always shown)
        md.push_str("## Execution Overview\n\n");
        let ov = &self.execution_overview;
        md.push_str(&format!("| Metric | Value |\n|--------|-------|\n"));
        md.push_str(&format!("| Max call depth | {} |\n", ov.max_call_depth));
        md.push_str(&format!("| Unique contracts | {} |\n", ov.unique_contracts));
        md.push_str(&format!(
            "| CALL/STATICCALL/DELEGATECALL | {} |\n",
            ov.call_count
        ));
        md.push_str(&format!("| CREATE/CREATE2 | {} |\n", ov.create_count));
        md.push_str(&format!("| SLOAD | {} |\n", ov.sload_count));
        md.push_str(&format!("| SSTORE | {} |\n", ov.sstore_count));
        md.push_str(&format!("| LOG0-LOG4 | {} |\n\n", ov.log_count));

        if !ov.top_opcodes.is_empty() {
            md.push_str("**Top opcodes**: ");
            let parts: Vec<String> = ov
                .top_opcodes
                .iter()
                .map(|(_, name, count)| format!("{name}({count})"))
                .collect();
            md.push_str(&parts.join(", "));
            md.push_str("\n\n");
        }

        // Attack Patterns (always shown)
        md.push_str("## Attack Patterns\n\n");
        if self.attack_patterns.is_empty() {
            md.push_str("No known attack patterns detected.\n\n");
        } else {
            for (i, pattern) in self.attack_patterns.iter().enumerate() {
                md.push_str(&format!(
                    "### Pattern {} — {}\n\n",
                    i + 1,
                    pattern_name(pattern)
                ));
                md.push_str(&format_pattern_detail(pattern));
                md.push('\n');
            }
        }

        // Fund Flows (always shown)
        md.push_str("## Fund Flow\n\n");
        if self.fund_flows.is_empty() {
            md.push_str("No fund transfers detected.\n\n");
        } else {
            md.push_str("| Step | From | To | Value | Token |\n");
            md.push_str("|------|------|-----|-------|-------|\n");
            for flow in &self.fund_flows {
                let token = flow
                    .token
                    .map(|t| format!("`0x{t:x}`"))
                    .unwrap_or_else(|| "ETH".to_string());
                md.push_str(&format!(
                    "| {} | `0x{:x}` | `0x{:x}` | {} | {} |\n",
                    flow.step_index, flow.from, flow.to, flow.value, token
                ));
            }
            md.push('\n');
        }

        // Storage Diffs (always shown)
        md.push_str("## Storage Changes\n\n");
        if self.storage_diffs.is_empty() {
            md.push_str("No storage modifications detected.\n\n");
        } else {
            md.push_str("| Contract | Slot | Old Value | New Value |\n");
            md.push_str("|----------|------|-----------|----------|\n");
            for diff in &self.storage_diffs {
                md.push_str(&format!(
                    "| `0x{:x}` | `0x{:x}` | `{}` | `{}` |\n",
                    diff.address, diff.slot, diff.old_value, diff.new_value
                ));
            }
            md.push('\n');
        }

        // Key Steps (always shown)
        md.push_str("## Key Steps\n\n");
        if self.key_steps.is_empty() {
            md.push_str("No key steps identified.\n\n");
        } else {
            for step in &self.key_steps {
                let icon = match step.severity {
                    Severity::Critical => "[CRITICAL]",
                    Severity::Warning => "[WARNING]",
                    Severity::Info => "[INFO]",
                };
                md.push_str(&format!(
                    "- {icon} **Step {}**: {}\n",
                    step.step_index, step.annotation
                ));
            }
            md.push('\n');
        }

        // Affected Contracts (always shown)
        md.push_str("## Affected Contracts\n\n");
        if self.affected_contracts.is_empty() {
            md.push_str("None identified.\n\n");
        } else {
            for addr in &self.affected_contracts {
                md.push_str(&format!("- `0x{addr:x}`\n"));
            }
            md.push('\n');
        }

        // Suggested Fixes (always shown)
        md.push_str("## Suggested Fixes\n\n");
        if self.suggested_fixes.is_empty() {
            md.push_str("No specific fixes suggested (no attack patterns detected).\n\n");
        } else {
            for fix in &self.suggested_fixes {
                md.push_str(&format!("- {fix}\n"));
            }
            md.push('\n');
        }

        md
    }

    fn generate_summary(
        patterns: &[AttackPattern],
        flows: &[FundFlow],
        overview: &ExecutionOverview,
    ) -> String {
        let mut parts = Vec::new();

        // Execution context
        parts.push(format!(
            "Execution reached depth {} across {} contract(s) with {} external calls.",
            overview.max_call_depth, overview.unique_contracts, overview.call_count
        ));

        if !patterns.is_empty() {
            let names: Vec<&str> = patterns.iter().map(pattern_name).collect();
            parts.push(format!(
                "Detected {} attack pattern(s): {}.",
                patterns.len(),
                names.join(", ")
            ));
        }

        let eth_flows: Vec<_> = flows.iter().filter(|f| f.token.is_none()).collect();
        let token_flows: Vec<_> = flows.iter().filter(|f| f.token.is_some()).collect();

        if !eth_flows.is_empty() {
            let total_eth: U256 = eth_flows.iter().fold(U256::zero(), |acc, f| acc + f.value);
            parts.push(format!(
                "{} ETH transfer(s) totaling {} wei.",
                eth_flows.len(),
                total_eth
            ));
        }

        if !token_flows.is_empty() {
            parts.push(format!(
                "{} ERC-20 transfer(s) detected.",
                token_flows.len()
            ));
        }

        if overview.sstore_count > 0 {
            parts.push(format!(
                "{} storage write(s).",
                overview.sstore_count
            ));
        }

        if patterns.is_empty() && flows.is_empty() {
            parts.push("No known attack patterns detected.".to_string());
        }

        parts.join(" ")
    }

    fn compute_overview(steps: &[StepRecord]) -> ExecutionOverview {
        use std::collections::{HashMap, HashSet};

        let mut max_depth: usize = 0;
        let mut contracts = HashSet::new();
        let mut call_count = 0usize;
        let mut sstore_count = 0usize;
        let mut sload_count = 0usize;
        let mut log_count = 0usize;
        let mut create_count = 0usize;
        let mut opcode_freq: HashMap<u8, usize> = HashMap::new();

        for step in steps {
            if (step.depth as usize) > max_depth {
                max_depth = step.depth as usize;
            }
            contracts.insert(step.code_address);
            *opcode_freq.entry(step.opcode).or_default() += 1;

            match step.opcode {
                0xF1 | 0xF2 | 0xF4 | 0xFA => call_count += 1, // CALL, CALLCODE, DELEGATECALL, STATICCALL
                0xF0 | 0xF5 => create_count += 1,               // CREATE, CREATE2
                0x54 => sload_count += 1,                        // SLOAD
                0x55 => sstore_count += 1,                       // SSTORE
                0xA0..=0xA4 => log_count += 1,                   // LOG0-LOG4
                _ => {}
            }
        }

        // Top 5 opcodes
        let mut freq_vec: Vec<(u8, usize)> = opcode_freq.into_iter().collect();
        freq_vec.sort_by(|a, b| b.1.cmp(&a.1));
        let top_opcodes: Vec<(u8, String, usize)> = freq_vec
            .into_iter()
            .take(5)
            .map(|(op, count)| (op, opcode_name(op).to_string(), count))
            .collect();

        ExecutionOverview {
            max_call_depth: max_depth,
            unique_contracts: contracts.len(),
            call_count,
            sstore_count,
            sload_count,
            log_count,
            create_count,
            top_opcodes,
        }
    }

    fn identify_key_steps(patterns: &[AttackPattern], flows: &[FundFlow]) -> Vec<AnnotatedStep> {
        let mut key = Vec::new();

        for pattern in patterns {
            match pattern {
                AttackPattern::Reentrancy {
                    reentrant_call_step,
                    state_modified_step,
                    ..
                } => {
                    key.push(AnnotatedStep {
                        step_index: *reentrant_call_step,
                        annotation: "Re-entrant call detected".to_string(),
                        severity: Severity::Critical,
                    });
                    key.push(AnnotatedStep {
                        step_index: *state_modified_step,
                        annotation: "State modified after re-entry".to_string(),
                        severity: Severity::Critical,
                    });
                }
                AttackPattern::FlashLoan {
                    borrow_step,
                    repay_step,
                    borrow_amount,
                    ..
                } => {
                    key.push(AnnotatedStep {
                        step_index: *borrow_step,
                        annotation: format!("Flash loan borrow: {borrow_amount} wei"),
                        severity: Severity::Warning,
                    });
                    key.push(AnnotatedStep {
                        step_index: *repay_step,
                        annotation: "Flash loan repayment".to_string(),
                        severity: Severity::Warning,
                    });
                }
                AttackPattern::PriceManipulation {
                    oracle_read_before,
                    swap_step,
                    oracle_read_after,
                    ..
                } => {
                    key.push(AnnotatedStep {
                        step_index: *oracle_read_before,
                        annotation: "Oracle price read (before manipulation)".to_string(),
                        severity: Severity::Warning,
                    });
                    key.push(AnnotatedStep {
                        step_index: *swap_step,
                        annotation: "Swap / price manipulation".to_string(),
                        severity: Severity::Critical,
                    });
                    key.push(AnnotatedStep {
                        step_index: *oracle_read_after,
                        annotation: "Oracle price read (after manipulation)".to_string(),
                        severity: Severity::Warning,
                    });
                }
                AttackPattern::AccessControlBypass { sstore_step, .. } => {
                    key.push(AnnotatedStep {
                        step_index: *sstore_step,
                        annotation: "SSTORE without access control check".to_string(),
                        severity: Severity::Warning,
                    });
                }
            }
        }

        // Annotate large ETH transfers
        for flow in flows {
            if flow.token.is_none() && flow.value > U256::zero() {
                key.push(AnnotatedStep {
                    step_index: flow.step_index,
                    annotation: format!("ETH transfer: {} wei", flow.value),
                    severity: Severity::Info,
                });
            }
        }

        key.sort_by_key(|s| s.step_index);
        key
    }

    fn collect_affected_contracts(flows: &[FundFlow], diffs: &[StorageWrite]) -> Vec<Address> {
        let mut addrs: Vec<Address> = Vec::new();
        for flow in flows {
            if !addrs.contains(&flow.from) {
                addrs.push(flow.from);
            }
            if !addrs.contains(&flow.to) {
                addrs.push(flow.to);
            }
        }
        for diff in diffs {
            if !addrs.contains(&diff.address) {
                addrs.push(diff.address);
            }
        }
        addrs
    }

    fn suggest_fixes(patterns: &[AttackPattern]) -> Vec<String> {
        let mut fixes = Vec::new();
        for pattern in patterns {
            match pattern {
                AttackPattern::Reentrancy { .. } => {
                    fixes.push("Add a reentrancy guard (e.g., OpenZeppelin ReentrancyGuard) to state-changing functions.".to_string());
                    fixes.push("Follow the checks-effects-interactions pattern: update state before external calls.".to_string());
                }
                AttackPattern::FlashLoan { .. } => {
                    fixes.push("Add flash loan protection: check that token balances haven't been temporarily inflated.".to_string());
                    fixes.push(
                        "Use time-weighted average prices (TWAP) instead of spot prices."
                            .to_string(),
                    );
                }
                AttackPattern::PriceManipulation { .. } => {
                    fixes.push("Use a decentralized oracle (e.g., Chainlink) with TWAP instead of spot AMM prices.".to_string());
                    fixes.push("Add price deviation checks: revert if price moves > X% in a single transaction.".to_string());
                }
                AttackPattern::AccessControlBypass { .. } => {
                    fixes.push("Add access control modifiers (onlyOwner, role-based) to state-changing functions.".to_string());
                    fixes.push("Use OpenZeppelin AccessControl for role management.".to_string());
                }
            }
        }
        // Deduplicate
        fixes.dedup();
        fixes
    }
}

fn pattern_name(pattern: &AttackPattern) -> &'static str {
    match pattern {
        AttackPattern::Reentrancy { .. } => "Reentrancy",
        AttackPattern::FlashLoan { .. } => "Flash Loan",
        AttackPattern::PriceManipulation { .. } => "Price Manipulation",
        AttackPattern::AccessControlBypass { .. } => "Access Control Bypass",
    }
}

fn format_pattern_detail(pattern: &AttackPattern) -> String {
    match pattern {
        AttackPattern::Reentrancy {
            target_contract,
            reentrant_call_step,
            state_modified_step,
            call_depth_at_entry,
        } => {
            format!(
                "- **Target**: `0x{target_contract:x}`\n\
                 - **Re-entrant call at step**: {reentrant_call_step}\n\
                 - **State modified at step**: {state_modified_step}\n\
                 - **Entry depth**: {call_depth_at_entry}\n"
            )
        }
        AttackPattern::FlashLoan {
            borrow_step,
            borrow_amount,
            repay_step,
            repay_amount,
            provider,
            token,
        } => {
            let mut detail = String::new();
            if let Some(p) = provider {
                detail.push_str(&format!("- **Provider**: `0x{p:x}`\n"));
            }
            if let Some(t) = token {
                detail.push_str(&format!("- **Token**: `0x{t:x}`\n"));
            }
            if *borrow_amount > U256::zero() {
                detail.push_str(&format!(
                    "- **Borrow at step**: {borrow_step} ({borrow_amount} wei)\n"
                ));
            } else {
                detail.push_str(&format!("- **Borrow at step**: {borrow_step}\n"));
            }
            if *repay_amount > U256::zero() {
                detail.push_str(&format!(
                    "- **Repay at step**: {repay_step} ({repay_amount} wei)\n"
                ));
            } else {
                detail.push_str(&format!("- **Repay at step**: {repay_step}\n"));
            }
            detail
        }
        AttackPattern::PriceManipulation {
            oracle_read_before,
            swap_step,
            oracle_read_after,
            price_delta_percent,
        } => {
            format!(
                "- **Oracle read before**: step {oracle_read_before}\n\
                 - **Swap/manipulation**: step {swap_step}\n\
                 - **Oracle read after**: step {oracle_read_after}\n\
                 - **Price delta**: {price_delta_percent:.1}%\n"
            )
        }
        AttackPattern::AccessControlBypass {
            sstore_step,
            contract,
        } => {
            format!(
                "- **SSTORE at step**: {sstore_step}\n\
                 - **Contract**: `0x{contract:x}`\n"
            )
        }
    }
}

fn opcode_name(op: u8) -> &'static str {
    match op {
        0x00 => "STOP", 0x01 => "ADD", 0x02 => "MUL", 0x03 => "SUB",
        0x04 => "DIV", 0x05 => "SDIV", 0x06 => "MOD", 0x10 => "LT",
        0x11 => "GT", 0x14 => "EQ", 0x15 => "ISZERO", 0x16 => "AND",
        0x17 => "OR", 0x18 => "XOR", 0x19 => "NOT", 0x1A => "BYTE",
        0x1B => "SHL", 0x1C => "SHR", 0x1D => "SAR",
        0x20 => "KECCAK256", 0x30 => "ADDRESS", 0x31 => "BALANCE",
        0x32 => "ORIGIN", 0x33 => "CALLER", 0x34 => "CALLVALUE",
        0x35 => "CALLDATALOAD", 0x36 => "CALLDATASIZE", 0x37 => "CALLDATACOPY",
        0x38 => "CODESIZE", 0x39 => "CODECOPY", 0x3A => "GASPRICE",
        0x3B => "EXTCODESIZE", 0x3C => "EXTCODECOPY", 0x3D => "RETURNDATASIZE",
        0x3E => "RETURNDATACOPY", 0x3F => "EXTCODEHASH",
        0x40 => "BLOCKHASH", 0x41 => "COINBASE", 0x42 => "TIMESTAMP",
        0x43 => "NUMBER", 0x44 => "PREVRANDAO", 0x45 => "GASLIMIT",
        0x46 => "CHAINID", 0x47 => "SELFBALANCE",
        0x50 => "POP", 0x51 => "MLOAD", 0x52 => "MSTORE",
        0x53 => "MSTORE8", 0x54 => "SLOAD", 0x55 => "SSTORE",
        0x56 => "JUMP", 0x57 => "JUMPI", 0x58 => "PC",
        0x59 => "MSIZE", 0x5A => "GAS", 0x5B => "JUMPDEST",
        0x5F => "PUSH0",
        0x60..=0x7F => "PUSHn", 0x80..=0x8F => "DUPn", 0x90..=0x9F => "SWAPn",
        0xA0 => "LOG0", 0xA1 => "LOG1", 0xA2 => "LOG2",
        0xA3 => "LOG3", 0xA4 => "LOG4",
        0xF0 => "CREATE", 0xF1 => "CALL", 0xF2 => "CALLCODE",
        0xF3 => "RETURN", 0xF4 => "DELEGATECALL", 0xF5 => "CREATE2",
        0xFA => "STATICCALL", 0xFD => "REVERT", 0xFE => "INVALID",
        0xFF => "SELFDESTRUCT",
        _ => "UNKNOWN",
    }
}
