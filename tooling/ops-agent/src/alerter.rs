use crate::models::Incident;
use teloxide::{
    prelude::{Requester, RequesterExt},
    types::ChatId,
    Bot,
};
use thiserror::Error;
use tokio::time::{Duration, sleep};

#[derive(Debug, Error)]
pub enum AlertError {
    #[error("telegram request failed: {0}")]
    Telegram(#[from] teloxide::RequestError),
}

#[async_trait::async_trait]
pub trait Notifier {
    async fn send_incident(&self, incident: &Incident) -> Result<(), AlertError>;
}

#[derive(Clone)]
pub struct TelegramAlerter {
    bot: Bot,
    chat_id: ChatId,
    max_retries: u8,
    retry_delay: Duration,
}

impl TelegramAlerter {
    pub fn new(bot_token: String, chat_id: i64) -> Self {
        Self {
            bot: Bot::new(bot_token),
            chat_id: ChatId(chat_id),
            max_retries: 3,
            retry_delay: Duration::from_millis(500),
        }
    }

    pub fn with_retry_policy(mut self, max_retries: u8, retry_delay: Duration) -> Self {
        self.max_retries = max_retries;
        self.retry_delay = retry_delay;
        self
    }
}

#[async_trait::async_trait]
impl Notifier for TelegramAlerter {
    async fn send_incident(&self, incident: &Incident) -> Result<(), AlertError> {
        let message = format!(
            "[ops-agent][{:?}] {:?}\n{}\nEvidence: {}",
            incident.severity,
            incident.scenario,
            incident.message,
            incident.evidence
        );

        let mut attempt: u8 = 0;
        loop {
            attempt = attempt.saturating_add(1);

            match self.bot.send_message(self.chat_id, message.clone()).send().await {
                Ok(_) => return Ok(()),
                Err(error) => {
                    if attempt >= self.max_retries {
                        return Err(AlertError::Telegram(error));
                    }
                    sleep(self.retry_delay).await;
                }
            }
        }
    }
}
