use crate::models::{Incident, Scenario, Severity, TelemetrySnapshot};
use serde_json::json;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct Diagnoser {
    stall_threshold: Duration,
    rpc_timeout_threshold: f64,
    rpc_timeout_consecutive_limit: u32,
    cpu_threshold: f64,
    cpu_pressure_consecutive_limit: u32,
    last_height: Option<u64>,
    last_height_change_at: Option<SystemTime>,
    rpc_timeout_consecutive_count: u32,
    cpu_pressure_consecutive_count: u32,
}

impl Default for Diagnoser {
    fn default() -> Self {
        Self {
            stall_threshold: Duration::from_secs(180),
            rpc_timeout_threshold: 30.0,
            rpc_timeout_consecutive_limit: 2,
            cpu_threshold: 90.0,
            cpu_pressure_consecutive_limit: 3,
            last_height: None,
            last_height_change_at: None,
            rpc_timeout_consecutive_count: 0,
            cpu_pressure_consecutive_count: 0,
        }
    }
}

impl Diagnoser {
    pub fn evaluate(&mut self, snapshot: &TelemetrySnapshot) -> Vec<Incident> {
        let mut incidents = Vec::new();

        self.evaluate_block_height(snapshot, &mut incidents);
        self.evaluate_rpc_timeout(snapshot, &mut incidents);
        self.evaluate_cpu_pressure(snapshot, &mut incidents);

        incidents
    }

    fn evaluate_block_height(&mut self, snapshot: &TelemetrySnapshot, incidents: &mut Vec<Incident>) {
        match self.last_height {
            Some(previous_height) if snapshot.block_height > previous_height => {
                self.last_height = Some(snapshot.block_height);
                self.last_height_change_at = Some(snapshot.captured_at);
            }
            Some(previous_height) if snapshot.block_height == previous_height => {
                if let Some(last_change_at) = self.last_height_change_at {
                    let stalled_for = snapshot
                        .captured_at
                        .duration_since(last_change_at)
                        .unwrap_or(Duration::from_secs(0));

                    if stalled_for >= self.stall_threshold {
                        incidents.push(Incident {
                            scenario: Scenario::BlockHeightStall,
                            severity: Severity::Critical,
                            message: format!(
                                "Block height stalled for {}s at height {}",
                                stalled_for.as_secs(),
                                snapshot.block_height
                            ),
                            detected_at: snapshot.captured_at,
                            evidence: json!({
                                "block_height": snapshot.block_height,
                                "stalled_for_seconds": stalled_for.as_secs(),
                                "threshold_seconds": self.stall_threshold.as_secs(),
                            }),
                        });
                    }
                }
            }
            _ => {
                self.last_height = Some(snapshot.block_height);
                self.last_height_change_at = Some(snapshot.captured_at);
            }
        }
    }

    fn evaluate_rpc_timeout(&mut self, snapshot: &TelemetrySnapshot, incidents: &mut Vec<Incident>) {
        if snapshot.execution_rpc_timeout_rate > self.rpc_timeout_threshold {
            self.rpc_timeout_consecutive_count += 1;
        } else {
            self.rpc_timeout_consecutive_count = 0;
        }

        if self.rpc_timeout_consecutive_count >= self.rpc_timeout_consecutive_limit {
            incidents.push(Incident {
                scenario: Scenario::ExecutionRpcTimeout,
                severity: Severity::Warning,
                message: format!(
                    "Execution RPC timeout rate high: {:.2}% ({} consecutive)",
                    snapshot.execution_rpc_timeout_rate,
                    self.rpc_timeout_consecutive_count
                ),
                detected_at: snapshot.captured_at,
                evidence: json!({
                    "timeout_rate": snapshot.execution_rpc_timeout_rate,
                    "threshold": self.rpc_timeout_threshold,
                    "consecutive_count": self.rpc_timeout_consecutive_count,
                }),
            });
        }
    }

    fn evaluate_cpu_pressure(&mut self, snapshot: &TelemetrySnapshot, incidents: &mut Vec<Incident>) {
        if snapshot.cpu_usage_percent > self.cpu_threshold {
            self.cpu_pressure_consecutive_count += 1;
        } else {
            self.cpu_pressure_consecutive_count = 0;
        }

        if self.cpu_pressure_consecutive_count >= self.cpu_pressure_consecutive_limit {
            incidents.push(Incident {
                scenario: Scenario::CpuPressure,
                severity: Severity::Warning,
                message: format!(
                    "CPU pressure detected: {:.2}% ({} consecutive)",
                    snapshot.cpu_usage_percent,
                    self.cpu_pressure_consecutive_count
                ),
                detected_at: snapshot.captured_at,
                evidence: json!({
                    "cpu_usage_percent": snapshot.cpu_usage_percent,
                    "threshold": self.cpu_threshold,
                    "consecutive_count": self.cpu_pressure_consecutive_count,
                }),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_at(second: u64, block_height: u64, rpc_timeout_rate: f64, cpu_usage: f64) -> TelemetrySnapshot {
        TelemetrySnapshot {
            captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(second),
            block_height,
            execution_rpc_timeout_rate: rpc_timeout_rate,
            cpu_usage_percent: cpu_usage,
        }
    }

    #[test]
    fn detects_block_height_stall_after_180_seconds() {
        let mut diagnoser = Diagnoser::default();
        let first = snapshot_at(0, 100, 0.0, 20.0);
        let second = snapshot_at(180, 100, 0.0, 20.0);

        let _ = diagnoser.evaluate(&first);
        let incidents = diagnoser.evaluate(&second);

        assert!(incidents.iter().any(|incident| incident.scenario == Scenario::BlockHeightStall));
    }

    #[test]
    fn detects_rpc_timeout_after_two_consecutive_breaches() {
        let mut diagnoser = Diagnoser::default();
        let first = snapshot_at(0, 100, 31.0, 20.0);
        let second = snapshot_at(10, 101, 35.0, 20.0);

        let first_incidents = diagnoser.evaluate(&first);
        assert!(first_incidents.is_empty());

        let second_incidents = diagnoser.evaluate(&second);
        assert!(second_incidents.iter().any(|incident| incident.scenario == Scenario::ExecutionRpcTimeout));
    }

    #[test]
    fn detects_cpu_pressure_after_three_consecutive_breaches() {
        let mut diagnoser = Diagnoser::default();
        let snapshots = [
            snapshot_at(0, 100, 0.0, 91.0),
            snapshot_at(10, 101, 0.0, 92.0),
            snapshot_at(20, 102, 0.0, 93.0),
        ];

        for snapshot in &snapshots[..2] {
            let incidents = diagnoser.evaluate(snapshot);
            assert!(incidents.is_empty());
        }

        let incidents = diagnoser.evaluate(&snapshots[2]);
        assert!(incidents.iter().any(|incident| incident.scenario == Scenario::CpuPressure));
    }
}
