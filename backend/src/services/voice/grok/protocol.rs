use super::super::openai::{self, Socket};
use crate::errors::{
    AppError, AppResult,
    voice_start::{Provider, ProviderFailure, Stage},
};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use zeroize::Zeroizing;

pub fn configuration(model: &str, voice: &str, instructions: &str) -> Value {
    json!({"type":"session.update","session":{
        "model":model,"voice":voice,"instructions":format!("{instructions} Delegate using delegate_to_agent with the server-issued utterance_id attached to each input. Never invent an ID. The intent_hint is advisory only."),
        "turn_detection":{"type":null},"resumption":{"enabled":false},
        "audio":{"input":{"format":{"type":"audio/pcm","rate":24000},"transport":"json",
            "transcription":{"model":"grok-transcribe"}},
            "output":{"format":{"type":"audio/pcm","rate":24000},"transport":"json"}},
        "tools":[{"type":"function","name":"delegate_to_agent",
            "description":"Queue the user's exact utterance with NyxID. Returns a task receipt promptly; results arrive separately.",
            "parameters":{"type":"object","additionalProperties":false,
                "properties":{"utterance_id":{"type":"string"},"intent_hint":{"type":"string","maxLength":128}},
                "required":["utterance_id","intent_hint"]}}]
    }})
}

pub async fn connect(
    key: &Zeroizing<String>,
    model: &str,
    voice: &str,
    instructions: &str,
) -> AppResult<(Socket, tokio::time::Instant)> {
    connect_to(
        "wss://api.x.ai/v1/realtime",
        key,
        model,
        voice,
        instructions,
    )
    .await
}
async fn connect_to(
    origin: &str,
    key: &Zeroizing<String>,
    model: &str,
    voice: &str,
    instructions: &str,
) -> AppResult<(Socket, tokio::time::Instant)> {
    let mut url = url::Url::parse(origin).map_err(|_| AppError::VoiceProviderUnavailable)?;
    url.query_pairs_mut().append_pair("model", model);
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| AppError::VoiceProviderUnavailable)?;
    let mut auth = format!("Bearer {}", key.as_str())
        .parse::<http::HeaderValue>()
        .map_err(|_| AppError::VoiceProviderUnavailable)?;
    auth.set_sensitive(true);
    request
        .headers_mut()
        .insert(http::header::AUTHORIZATION, auth);
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(openai::MAX_FRAME))
        .max_frame_size(Some(openai::MAX_FRAME));
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        // connect_async does not follow redirects; no credential-bearing retry.
        let (mut socket, _) =
            tokio_tungstenite::connect_async_with_config(request, Some(config), false)
                .await
                .map_err(|e| {
                    super::super::diagnostics::socket_error(e, Provider::Xai, Stage::ProviderCreate)
                })?;
        let started = tokio::time::Instant::now();
        let created = openai::receive(&mut socket)
            .await?
            .ok_or(AppError::VoiceProviderUnavailable)?;
        if created["type"] != "session.created" {
            return Err(provider_answer(&created));
        }
        openai::send(&mut socket, configuration(model, voice, instructions)).await?;
        for _ in 0..8 {
            let event = openai::receive(&mut socket)
                .await?
                .ok_or(AppError::VoiceProviderUnavailable)?;
            if event["type"] == "session.updated" {
                if !acknowledged(&event["session"], model, voice) {
                    return Err(Stage::ProviderAnswer.error(AppError::VoiceProviderUnavailable));
                }
                return Ok((socket, started));
            }
            if event["type"] != "conversation.created" {
                return Err(provider_answer(&event));
            }
        }
        Err(Stage::ProviderAnswer.error(AppError::VoiceProviderUnavailable))
    })
    .await
    .map_err(|_| Stage::Transport.error(AppError::VoiceProviderUnavailable))?
}

fn provider_answer(event: &Value) -> AppError {
    if event["type"] == "error" {
        ProviderFailure::new(Provider::Xai, None, event.to_string().as_bytes())
            .error(Stage::ProviderCreate)
    } else {
        Stage::ProviderAnswer.error(AppError::VoiceProviderUnavailable)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delegation {
    pub utterance_id: String,
    pub intent_hint: String,
}
pub fn delegation(value: &Value) -> Option<Delegation> {
    if value["name"] != "delegate_to_agent" {
        return None;
    }
    let args = value["arguments"].as_str().filter(|s| s.len() <= 1024)?;
    let parsed: Delegation = serde_json::from_str(args).ok()?;
    (uuid::Uuid::parse_str(&parsed.utterance_id).is_ok() && parsed.intent_hint.len() <= 128)
        .then_some(parsed)
}

#[cfg(test)]
pub(super) async fn fixture(address: std::net::SocketAddr) -> AppResult<Socket> {
    connect_to(
        &format!("ws://{address}/v1/realtime"),
        &Zeroizing::new("fixture-secret".into()),
        "catalog-grok",
        "eve",
        "context",
    )
    .await
    .map(|(socket, _)| socket)
}

/// Fail closed on unsafe settings, including omitted manual-mode acknowledgement.
pub(super) fn acknowledged(session: &Value, model: &str, voice: &str) -> bool {
    let expected = configuration(model, voice, "");
    session.get("turn_detection").and_then(|v| v.get("type")) == Some(&Value::Null)
        && session["model"] == model
        && session["voice"] == voice
        && session
            .get("resumption")
            .is_none_or(|v| v["enabled"] == false)
        && session["tools"] == expected["session"]["tools"]
        && session
            .get("audio")
            .is_none_or(|v| *v == expected["session"]["audio"])
}
