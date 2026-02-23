use ethrex_ops_agent::{
    alerter::TelegramAlerter,
    collector::Collector,
    config::AppConfig,
    diagnoser::Diagnoser,
    service::process_snapshot,
    storage::IncidentRepository,
};
use tokio::time;
use std::time::Duration;
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

    let alerter = TelegramAlerter::new(config.telegram_bot_token, config.telegram_chat_id)
        .with_retry_policy(
            config.telegram_retry_max,
            Duration::from_millis(config.telegram_retry_delay_ms),
        );

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

        let sent_count = process_snapshot(&mut diagnoser, &repository, &alerter, &snapshot).await;
        if sent_count > 0 {
            info!(sent_count, "incidents processed and alerted");
        }
    }
}
