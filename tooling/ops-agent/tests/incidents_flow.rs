use ethrex_ops_agent::{diagnoser::Diagnoser, models::TelemetrySnapshot, storage::IncidentRepository};
use std::time::{Duration, SystemTime};
use tempfile::NamedTempFile;

fn snapshot_at(second: u64, block_height: u64, rpc_timeout_rate: f64, cpu_usage: f64) -> TelemetrySnapshot {
    TelemetrySnapshot {
        captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(second),
        block_height,
        execution_rpc_timeout_rate: rpc_timeout_rate,
        cpu_usage_percent: cpu_usage,
    }
}

#[test]
fn diagnoser_incidents_can_be_persisted() {
    let temp_file_result = NamedTempFile::new();
    assert!(temp_file_result.is_ok());
    let temp_file = match temp_file_result {
        Ok(file) => file,
        Err(_) => return,
    };

    let repository_result = IncidentRepository::open(temp_file.path());
    assert!(repository_result.is_ok());
    let repository = match repository_result {
        Ok(repo) => repo,
        Err(_) => return,
    };

    let mut diagnoser = Diagnoser::default();
    let samples = [
        snapshot_at(0, 100, 31.0, 91.0),
        snapshot_at(90, 100, 32.0, 92.0),
        snapshot_at(180, 100, 33.0, 93.0),
    ];

    let mut inserted = 0;
    for sample in samples {
        for incident in diagnoser.evaluate(&sample) {
            let row_id = repository.insert(&incident);
            assert!(row_id.is_ok());
            inserted += 1;
        }
    }

    assert!(inserted >= 3);
}
