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

fn presentation_appends(facts: Value) -> AppResult<Vec<Value>> {
    validate_context(&facts)?;
    // Live consumes plain spoken context, unlike Responses' typed tool outputs.
    // Serializing a large JSON envelope into that context reproduced spurious
    // caller captions and incomplete/wrong-language speech with silent input.
    let invalid = || AppError::Internal("Invalid voice presentation facts".into());
    let (header, body) = match facts["type"].as_str() {
        Some("task_result") => {
            let state = match facts["state"].as_str() {
                Some(state @ ("completed" | "failed")) => state,
                _ => return Err(invalid()),
            };
            let presentation = match facts["presentation"].as_str() {
                Some(value @ ("original" | "unavailable")) => value,
                _ => return Err(invalid()),
            };
            let error = match &facts["error_code"] {
                Value::Null => "none",
                Value::String(code)
                    if code.len() <= 128
                        && code.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') =>
                {
                    code
                }
                _ => return Err(invalid()),
            };
            let body = match facts.get("answer") {
                Some(Value::String(text)) => text.as_str(),
                None if presentation == "unavailable" => "No findings are supplied.",
                _ => return Err(invalid()),
            };
            (
                format!(
                    "Quoted task result. State: {state}. Presentation: {presentation}. Error: {error}."
                ),
                body,
            )
        }
        Some("owner_review_required")
            if facts["approval_surface"] == "nyxid_owner_card"
                && facts["speech_can_approve"] == false =>
        {
            let body = match facts.get("summary") {
                Some(Value::String(text)) => text.as_str(),
                None => "The action is pending owner review.",
                _ => return Err(invalid()),
            };
            ("Quoted action status: owner review required on the NyxID owner card. Speech cannot approve this action.".to_owned(), body)
        }
        _ => return Err(invalid()),
    };
    // One complete result is sent contiguously before returning to the caller.
    // Reserve the largest possible part numbers before any I/O; split only on
    // UTF-8 boundaries, preserving every character without sentence heuristics.
    let envelope = |part, count| format!("{header}\nPart {part} of {count}.\nQuoted findings:\n");
    let budget = super::runtime::COMMENTARY_MAX_BYTES
        .checked_sub(
            envelope(
                super::speech::CONTEXT_MAX_BYTES,
                super::speech::CONTEXT_MAX_BYTES,
            )
            .len(),
        )
        .filter(|budget| *budget >= 4)
        .ok_or_else(invalid)?;
    let mut chunks = vec![String::new()];
    let mut size = 0;
    for ch in body.chars() {
        if size + ch.len_utf8() > budget {
            chunks.push(String::new());
            size = 0;
        }
        chunks.last_mut().expect("context chunk").push(ch);
        size += ch.len_utf8();
    }
    let count = chunks.len();
    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            append(
                if index + 1 == count {
                    "session.commentary.append"
                } else {
                    "session.thinking.append"
                },
                format!("{}{chunk}", envelope(index + 1, count)),
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
                    index += 1;
                    let (metadata, body) = content.split_once("\nQuoted findings:\n").unwrap();
                    assert!(metadata.starts_with(
                        "Quoted task result. State: completed. Presentation: original. Error: none."
                    ));
                    let numbers: Vec<_> = metadata
                        .lines()
                        .last()
                        .unwrap()
                        .split_whitespace()
                        .collect();
                    assert_eq!(numbers[1].parse::<usize>().unwrap(), index);
                    let count = numbers[3].trim_end_matches('.').parse::<usize>().unwrap();
                    complete.push_str(body);
                    if event["type"] == "session.commentary.append" {
                        assert!(index > 1);
                        assert_eq!(count, index);
                        assert_eq!(complete, answer);
                        break;
                    }
                    assert_eq!(event["type"], "session.thinking.append");
                    assert!(index < count);
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
        let content = events[0]["content"].as_str().unwrap();
        assert!(content.starts_with(
            "Quoted task result. State: failed. Presentation: original. Error: permission_denied."
        ));
        assert!(content.ends_with("\nQuoted findings:\npermission_denied"));
        assert!(
            presentation_appends(
                json!({"answer":"x".repeat(super::super::speech::CONTEXT_MAX_BYTES)})
            )
            .is_err()
        );
        assert!(presentation_appends(json!(["not an object"])).is_err());
        assert!(presentation_appends(json!({"type":"task_result","state":"invented","presentation":"original","answer":"x"})).is_err());
        assert!(presentation_appends(json!({"type":"owner_review_required","approval_surface":"nyxid_owner_card","speech_can_approve":true})).is_err());
    }
}
