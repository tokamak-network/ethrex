use std::{env, num::ParseIntError, path::PathBuf, time::Duration};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub prometheus_base_url: String,
    pub execution_rpc_url: String,
    pub sqlite_path: PathBuf,
    pub telegram_bot_token: String,
    pub telegram_chat_id: i64,
    pub poll_interval: Duration,
    pub telegram_retry_max: u8,
    pub telegram_retry_delay_ms: u64,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing env var: {0}")]
    MissingEnv(String),
    #[error("invalid integer in env var {name}: {source}")]
    InvalidInteger { name: String, source: ParseIntError },
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let prometheus_base_url = read_required("OPS_AGENT_PROMETHEUS_BASE_URL")?;
        let execution_rpc_url = read_required("OPS_AGENT_EXECUTION_RPC_URL")?;
        let sqlite_path = PathBuf::from(
            env::var("OPS_AGENT_SQLITE_PATH").unwrap_or_else(|_| "ops-agent.sqlite".to_owned()),
        );
        let telegram_bot_token = read_required("OPS_AGENT_TELEGRAM_BOT_TOKEN")?;
        let telegram_chat_id = read_i64("OPS_AGENT_TELEGRAM_CHAT_ID")?;

        let poll_seconds = env::var("OPS_AGENT_POLL_SECONDS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(30);

        let telegram_retry_max = env::var("OPS_AGENT_TELEGRAM_RETRY_MAX")
            .ok()
            .and_then(|raw| raw.parse::<u8>().ok())
            .unwrap_or(3);

        let telegram_retry_delay_ms = env::var("OPS_AGENT_TELEGRAM_RETRY_DELAY_MS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .unwrap_or(500);

        Ok(Self {
            prometheus_base_url,
            execution_rpc_url,
            sqlite_path,
            telegram_bot_token,
            telegram_chat_id,
            poll_interval: Duration::from_secs(poll_seconds),
            telegram_retry_max,
            telegram_retry_delay_ms,
        })
    }
}

fn read_required(name: &str) -> Result<String, ConfigError> {
    env::var(name).map_err(|_| ConfigError::MissingEnv(name.to_owned()))
}

fn read_i64(name: &str) -> Result<i64, ConfigError> {
    let raw = read_required(name)?;
    raw.parse::<i64>().map_err(|source| ConfigError::InvalidInteger {
        name: name.to_owned(),
        source,
    })
}
