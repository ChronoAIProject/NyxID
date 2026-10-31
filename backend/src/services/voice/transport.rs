//! Provider boundary; only server-generated normalized commands cross it.
use super::{grok::Handle, openai};
use crate::errors::AppResult;
use serde_json::Value;

pub enum Transport {
    Openai(Box<openai::Socket>),
    Grok(Box<Handle>),
    Shared {
        inner: Box<Transport>,
        db: mongodb::Database,
        binding: Box<crate::models::assistant_sharing::SharedExecution>,
    },
}
impl Transport {
    pub async fn send(&mut self, event: Value) -> AppResult<()> {
        match self {
            Self::Shared { inner, db, binding } => {
                // Closing is bounded cleanup, never new model/tool authority.
                if event["type"] != "session.close" {
                    crate::services::shared_agent_transport::revalidate(db, binding).await?;
                }
                Box::pin(inner.send(event)).await
            }
            Self::Openai(socket) => openai::send(socket, event).await,
            Self::Grok(socket) => socket.send(event).await,
        }
    }
    pub async fn receive(&mut self) -> AppResult<Option<Value>> {
        match self {
            Self::Shared { inner, .. } => Box::pin(inner.receive()).await,
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
        match self {
            Self::Grok(_) => true,
            Self::Shared { inner, .. } => inner.is_grok(),
            _ => false,
        }
    }
}
