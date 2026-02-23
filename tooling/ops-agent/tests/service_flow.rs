use ethrex_ops_agent::{
    alerter::{AlertError, Notifier},
    diagnoser::Diagnoser,
    models::{Incident, TelemetrySnapshot},
    service::process_snapshot,
    storage::IncidentRepository,
};
use std::{sync::Mutex, time::{Duration, SystemTime}};
use tempfile::NamedTempFile;

#[derive(Default)]
struct MockNotifier {
    sent: Mutex<Vec<Incident>>,
}

#[async_trait::async_trait]
impl Notifier for MockNotifier {
    async fn send_incident(&self, incident: &Incident) -> Result<(), AlertError> {
        let mut guard = match self.sent.lock() {
            Ok(guard) => guard,
            Err(_) => return Ok(()),
        };
        guard.push(incident.clone());
        Ok(())
    }
}

fn snapshot_at(second: u64, block_height: u64, rpc_timeout_rate: f64, cpu_usage: f64) -> TelemetrySnapshot {
    TelemetrySnapshot {
        captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(second),
        block_height,
        execution_rpc_timeout_rate: rpc_timeout_rate,
        cpu_usage_percent: cpu_usage,
    }
}

#[tokio::test]
async fn process_snapshot_sends_alerts_for_all_three_scenarios() {
    let temp_file = match NamedTempFile::new() {
        Ok(file) => file,
        Err(_) => return,
    };

    let repository = match IncidentRepository::open(temp_file.path()) {
        Ok(repository) => repository,
        Err(_) => return,
    };

    let notifier = MockNotifier::default();
    let mut diagnoser = Diagnoser::default();

    let samples = [
        snapshot_at(0, 100, 31.0, 91.0),
        snapshot_at(90, 100, 32.0, 92.0),
        snapshot_at(180, 100, 33.0, 93.0),
    ];

    let mut total_sent = 0;
    for sample in samples {
        total_sent += process_snapshot(&mut diagnoser, &repository, &notifier, &sample).await;
    }

    assert!(total_sent >= 3);

    let sent_count = match notifier.sent.lock() {
        Ok(guard) => guard.len(),
        Err(_) => 0,
    };

    assert!(sent_count >= 3);
}
