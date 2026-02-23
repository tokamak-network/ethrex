use ethrex_ops_agent::{
    alerter::{Notifier, TelegramAlerter},
    collector::Collector,
    config::AppConfig,
    diagnoser::Diagnoser,
    storage::IncidentRepository,
};
use std::sync::Arc;
use tokio::time;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    if let Err(error) = run().await {
        error!(error = %error, "ops-agent startup failed");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let config = AppConfig::from_env().map_err(|error| error.to_string())?;

    let collector = Collector::new(
        config.prometheus_base_url.clone(),
        config.execution_rpc_url.clone(),
    );
    let mut diagnoser = Diagnoser::default();

    let repository = IncidentRepository::open(&config.sqlite_path).map_err(|error| error.to_string())?;
    let repository = Arc::new(repository);

    let alerter = TelegramAlerter::new(config.telegram_bot_token, config.telegram_chat_id);

    info!(poll_seconds = config.poll_interval.as_secs(), "ops-agent started in observe-only mode");

    let mut ticker = time::interval(config.poll_interval);

    loop {
        ticker.tick().await;

        let snapshot = match collector.collect_snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                warn!(error = %error, "snapshot collection failed");
                continue;
            }
        };

        let incidents = diagnoser.evaluate(&snapshot);
        if incidents.is_empty() {
            continue;
        }

        for incident in incidents {
            match repository.insert(&incident) {
                Ok(incident_id) => {
                    info!(incident_id, scenario = ?incident.scenario, "incident stored");
                    if let Err(error) = alerter.send_incident(&incident).await {
                        warn!(incident_id, error = %error, "failed to send telegram alert");
                    } else {
                        info!(incident_id, "telegram alert sent");
                    }
                }
                Err(error) => {
                    warn!(error = %error, "failed to store incident");
                }
            }
        }
    }
}
