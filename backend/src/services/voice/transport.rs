//! Provider boundary; only server-generated normalized commands cross it.
use super::{grok::Handle, openai};
use crate::errors::{AppError, AppResult};
use serde_json::{Value, json};

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
    /// Presentation facts are context for natural speech, never an exact read-back.
    pub(super) async fn presentation_context(&mut self, facts: Value) -> AppResult<()> {
        validate_context(&facts)?;
        if self.is_grok() {
            return self
                .send(json!({"type":"nyx.presentation_context","facts":facts}))
                .await;
        }
        // Build the complete sequence before sending any part. Thinking appends
        // supply silent factual context; only the last part requests speech.
        for event in presentation_appends(facts)? {
            self.send(event).await?;
        }
        Ok(())
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

fn validate_context(facts: &Value) -> AppResult<()> {
    if !facts.is_object() || facts.to_string().len() > super::speech::CONTEXT_MAX_BYTES {
        return Err(AppError::Internal(
            "Voice presentation context exceeds its bound".into(),
        ));
    }
    Ok(())
}

fn append(kind: &str, content: String) -> Value {
    json!({"type":kind,"event_id":uuid::Uuid::new_v4().to_string(),
        "delegation_id":null,"content":content})
}

fn presentation_appends(mut facts: Value) -> AppResult<Vec<Value>> {
    validate_context(&facts)?;
    let content = facts.to_string();
    if content.len() <= super::runtime::COMMENTARY_MAX_BYTES {
        return Ok(vec![append("session.commentary.append", content)]);
    }
    let answer = facts["answer"]
        .as_str()
        .ok_or_else(|| AppError::Internal("Voice presentation has no answer".into()))?
        .to_owned();
    facts["answer"] = json!("");
    facts["result_id"] = json!(uuid::Uuid::new_v4().to_string());
    // At most one part per original byte. Reserve the largest possible part
    // numbers so JSON quoting and metadata always fit the provider append bound.
    facts["part"] = json!(super::speech::CONTEXT_MAX_BYTES);
    facts["parts"] = json!(super::speech::CONTEXT_MAX_BYTES);
    let budget = super::runtime::COMMENTARY_MAX_BYTES
        .checked_sub(facts.to_string().len())
        .filter(|budget| *budget >= 6)
        .ok_or_else(|| {
            AppError::Internal("Voice presentation metadata exceeds its bound".into())
        })?;
    let mut chunks = vec![String::new()];
    let mut size = 0;
    for ch in answer.chars() {
        // Count the JSON-escaped character without the enclosing quotes. Split
        // only at Unicode boundaries; no findings are selected or discarded.
        let encoded_size = serde_json::to_string(&ch)
            .expect("a character is serializable")
            .len()
            - 2;
        if size + encoded_size > budget {
            chunks.push(String::new());
            size = 0;
        }
        chunks.last_mut().expect("answer chunk").push(ch);
        size += encoded_size;
    }
    let count = chunks.len();
    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            facts["answer"] = json!(chunk);
            facts["part"] = json!(index + 1);
            facts["parts"] = json!(count);
            append(
                if index + 1 == count {
                    "session.commentary.append"
                } else {
                    "session.thinking.append"
                },
                facts.to_string(),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::protocol::Role};

    async fn socket_pair() -> (Transport, openai::Socket) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let connect = tokio::net::TcpStream::connect(listener.local_addr().unwrap());
        let (client, server) = tokio::join!(connect, listener.accept());
        let client = WebSocketStream::from_raw_socket(
            MaybeTlsStream::Plain(client.unwrap()),
            Role::Client,
            None,
        )
        .await;
        let server = WebSocketStream::from_raw_socket(
            MaybeTlsStream::Plain(server.unwrap().0),
            Role::Server,
            None,
        )
        .await;
        (Transport::Openai(Box::new(client)), server)
    }

    #[tokio::test]
    async fn long_results_arrive_losslessly_before_the_only_spoken_append() {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let (mut transport, mut provider) = socket_pair().await;
            let answer =
                "`net.connected=true` 音量95 العربية မြန်မာ 👩🏽‍💻\n\"quoted\"\\path\u{0}".repeat(30);
            let facts = json!({"type":"task_result","state":"completed",
                "presentation":"original","answer":answer,"error_code":null});
            let mut complete = String::new();
            let mut result_id = None;
            let mut event_ids = std::collections::HashSet::new();
            let read = async {
                let mut index = 0;
                loop {
                    let frame = provider.next().await.unwrap().unwrap();
                    let event: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
                    assert!(event_ids.insert(event["event_id"].as_str().unwrap().to_owned()));
                    assert!(event["delegation_id"].is_null());
                    let content = event["content"].as_str().unwrap();
                    assert!(content.len() <= super::super::runtime::COMMENTARY_MAX_BYTES);
                    let part: Value = serde_json::from_str(content).unwrap();
                    index += 1;
                    assert_eq!(part["part"], index);
                    assert_eq!(part["type"], facts["type"]);
                    assert_eq!(part["state"], facts["state"]);
                    assert_eq!(part["error_code"], facts["error_code"]);
                    let id = part["result_id"].as_str().unwrap();
                    assert_eq!(result_id.get_or_insert_with(|| id.to_owned()), id);
                    complete.push_str(part["answer"].as_str().unwrap());
                    if event["type"] == "session.commentary.append" {
                        assert!(index > 1);
                        assert_eq!(part["parts"], index);
                        assert_eq!(complete, answer);
                        break;
                    }
                    assert_eq!(event["type"], "session.thinking.append");
                    assert!(index < part["parts"].as_u64().unwrap());
                }
            };
            let send = transport.presentation_context(facts.clone());
            let (sent, ()) = tokio::join!(send, read);
            sent.unwrap();
        })
        .await
        .unwrap();
    }

    #[test]
    fn short_context_keeps_one_append_and_oversized_context_is_rejected_whole() {
        let facts = json!({"type":"task_result","state":"failed",
            "presentation":"original","answer":"permission_denied","error_code":"permission_denied"});
        let events = presentation_appends(facts.clone()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "session.commentary.append");
        assert_eq!(
            serde_json::from_str::<Value>(events[0]["content"].as_str().unwrap()).unwrap(),
            facts
        );
        assert!(
            presentation_appends(
                json!({"answer":"x".repeat(super::super::speech::CONTEXT_MAX_BYTES)})
            )
            .is_err()
        );
        assert!(presentation_appends(json!(["not an object"])).is_err());
    }
}
