//! Provider boundary; only server-generated normalized commands cross it.
use super::{grok::Handle, openai};
use crate::errors::AppResult;
use serde_json::Value;

pub enum Transport {
    Openai(Box<openai::Socket>),
    Grok(Box<Handle>),
}
impl Transport {
    pub async fn send(&mut self, event: Value) -> AppResult<()> {
        match self {
            Self::Openai(socket) => openai::send(socket, event).await,
            Self::Grok(socket) => socket.send(event).await,
        }
    }
    pub async fn receive(&mut self) -> AppResult<Option<Value>> {
        match self {
            Self::Openai(socket) => {
                let event = openai::receive(socket).await?;
                if event
                    .as_ref()
                    .and_then(|v| v["type"].as_str())
                    .is_some_and(|t| t.starts_with("nyx."))
                {
                    return Err(crate::errors::AppError::VoiceProviderUnavailable);
                }
                Ok(event)
            }
            Self::Grok(socket) => socket.receive().await,
        }
    }
    pub fn is_grok(&self) -> bool {
        matches!(self, Self::Grok(_))
    }
}
