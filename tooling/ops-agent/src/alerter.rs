use crate::models::Incident;
use teloxide::{
    prelude::{Requester, RequesterExt},
    types::ChatId,
    Bot,
};
use thiserror::Error;

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
}

impl TelegramAlerter {
    pub fn new(bot_token: String, chat_id: i64) -> Self {
        Self {
            bot: Bot::new(bot_token),
            chat_id: ChatId(chat_id),
        }
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

        self.bot.send_message(self.chat_id, message).send().await?;
        Ok(())
    }
}
