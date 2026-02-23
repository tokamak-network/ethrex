use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    pub captured_at: SystemTime,
    pub block_height: u64,
    pub execution_rpc_timeout_rate: f64,
    pub cpu_usage_percent: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scenario {
    BlockHeightStall,
    ExecutionRpcTimeout,
    CpuPressure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Incident {
    pub scenario: Scenario,
    pub severity: Severity,
    pub message: String,
    pub detected_at: SystemTime,
    pub evidence: serde_json::Value,
}
