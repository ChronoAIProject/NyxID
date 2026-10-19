use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::errors::{AppError, AppResult};

pub struct TelegramApi<'a> {
    pub http: &'a reqwest::Client,
    pub base_url: &'a str,
}

impl<'a> TelegramApi<'a> {
    pub fn new(http: &'a reqwest::Client) -> Self {
        Self {
            http,
            base_url: "https://api.telegram.org",
        }
    }

    pub async fn call(&self, token: &str, method: &str, input: Value) -> AppResult<Value> {
        let failure = || {
            AppError::ChannelPlatformError(format!(
                "Telegram {method} failed; retry or check the manager configuration"
            ))
        };
        let mut response = self
            .http
            .post(format!("{}/bot{token}/{method}", self.base_url))
            .timeout(std::time::Duration::from_secs(20))
            .json(&input)
            .send()
            .await
            .map_err(|_| failure())?;
        if !response.status().is_success() {
            return Err(failure());
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err(failure());
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut envelope: Value = serde_json::from_slice(&bytes).map_err(|_| failure())?;
        if envelope["ok"] != true {
            return Err(failure());
        }
        Ok(envelope["result"].take())
    }

    pub async fn token(&self, manager: &str, bot_id: i64) -> AppResult<Zeroizing<String>> {
        match self
            .call(manager, "getManagedBotToken", json!({"user_id": bot_id}))
            .await?
        {
            Value::String(value) if !value.is_empty() => Ok(Zeroizing::new(value)),
            _ => Err(AppError::ChannelPlatformError(
                "Telegram returned an invalid bot token".into(),
            )),
        }
    }
}
