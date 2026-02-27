//! Autopsy report generation (JSON + Markdown).

use ethrex_common::{Address, H256, U256};
use serde::Serialize;

use crate::types::StorageWrite;

use super::types::{AnnotatedStep, AttackPattern, FundFlow, Severity};

/// Complete autopsy report for a single transaction.
#[derive(Debug, Clone, Serialize)]
pub struct AutopsyReport {
    pub tx_hash: H256,
    pub block_number: u64,
    pub summary: String,
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
        total_steps: usize,
        attack_patterns: Vec<AttackPattern>,
        fund_flows: Vec<FundFlow>,
        storage_diffs: Vec<StorageWrite>,
    ) -> Self {
        let key_steps = Self::identify_key_steps(&attack_patterns, &fund_flows);
        let affected_contracts = Self::collect_affected_contracts(&fund_flows, &storage_diffs);
        let suggested_fixes = Self::suggest_fixes(&attack_patterns);
        let summary = Self::generate_summary(&attack_patterns, &fund_flows);

        Self {
            tx_hash,
            block_number,
            summary,
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

        // Attack Patterns
        if !self.attack_patterns.is_empty() {
            md.push_str("## Attack Patterns Detected\n\n");
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

        // Fund Flows
        if !self.fund_flows.is_empty() {
            md.push_str("## Fund Flow\n\n");
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

        // Storage Diffs
        if !self.storage_diffs.is_empty() {
            md.push_str("## Storage Changes\n\n");
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

        // Key Steps
        if !self.key_steps.is_empty() {
            md.push_str("## Key Steps\n\n");
            for step in &self.key_steps {
                let icon = match step.severity {
                    Severity::Critical => "🔴",
                    Severity::Warning => "🟡",
                    Severity::Info => "🔵",
                };
                md.push_str(&format!(
                    "- {icon} **Step {}**: {}\n",
                    step.step_index, step.annotation
                ));
            }
            md.push('\n');
        }

        // Affected Contracts
        if !self.affected_contracts.is_empty() {
            md.push_str("## Affected Contracts\n\n");
            for addr in &self.affected_contracts {
                md.push_str(&format!("- `0x{addr:x}`\n"));
            }
            md.push('\n');
        }

        // Suggested Fixes
        if !self.suggested_fixes.is_empty() {
            md.push_str("## Suggested Fixes\n\n");
            for fix in &self.suggested_fixes {
                md.push_str(&format!("- {fix}\n"));
            }
            md.push('\n');
        }

        md
    }

    fn generate_summary(patterns: &[AttackPattern], flows: &[FundFlow]) -> String {
        if patterns.is_empty() && flows.is_empty() {
            return "No attack patterns or fund flows detected.".to_string();
        }

        let mut parts = Vec::new();

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

        parts.join(" ")
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
        } => {
            format!(
                "- **Borrow at step**: {borrow_step} ({borrow_amount} wei)\n\
                 - **Repay at step**: {repay_step} ({repay_amount} wei)\n"
            )
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
