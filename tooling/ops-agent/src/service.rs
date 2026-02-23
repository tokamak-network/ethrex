use crate::{
    alerter::Notifier,
    diagnoser::Diagnoser,
    models::TelemetrySnapshot,
    storage::IncidentRepository,
};
use tracing::{info, warn};

pub async fn process_snapshot(
    diagnoser: &mut Diagnoser,
    repository: &IncidentRepository,
    notifier: &impl Notifier,
    snapshot: &TelemetrySnapshot,
) -> usize {
    let incidents = diagnoser.evaluate(snapshot);
    if incidents.is_empty() {
        return 0;
    }

    let mut sent_count = 0;
    for incident in incidents {
        match repository.insert(&incident) {
            Ok(incident_id) => {
                info!(incident_id, scenario = ?incident.scenario, "incident stored");
                if let Err(error) = notifier.send_incident(&incident).await {
                    warn!(incident_id, error = %error, "failed to send telegram alert");
                } else {
                    sent_count += 1;
                    info!(incident_id, "telegram alert sent");
                }
            }
            Err(error) => {
                warn!(error = %error, "failed to store incident");
            }
        }
    }

    sent_count
}
