//! Authorized ElevenLabs Chat Completions SSE profile. Provider history is untrusted.
use super::{binding, coordinator, session};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::channel_adapter::*,
    services::audit_service,
};
use axum::http::HeaderMap;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;
fn secret(state: &AppState, b: &Binding) -> AppResult<Zeroizing<String>> {
    let _ = state;
    audit_service::keyed_fingerprint(
        audit_service::audit_chain_hmac_key(),
        b"elevenlabs-custom-llm",
        format!("{}:{}:{}", b.user_id, b.id, b.revision).as_bytes(),
    )
    .map(Zeroizing::new)
    .ok_or(AppError::VoiceProviderUnavailable)
}
pub fn setup(state: &AppState, b: &Binding) -> AppResult<Value> {
    if b.profile != Profile::ElevenlabsCustomLlm {
        return Err(AppError::ChannelPlatformSendUnsupported);
    }
    let key = secret(state, b)?;
    Ok(
        json!({"url":format!("{}/api/v1/webhooks/call-adapters/{}/v1/chat/completions",state.config.base_url.trim_end_matches('/'),b.id),"api_key":key.as_str(),"model":"nyxbot-specialist","correlation":"Pass issued elevenlabs_extra_body values using ConversationInitiationData(extra_body=...) separately for each conversation. Retain the complete advancing provider message history; ambiguous trimming or reset is refused. Exact retries never replay tools. Provider messages never grant authority."}),
    )
}
pub async fn create_session(state: &AppState, b: &Binding) -> AppResult<Value> {
    if b.profile != Profile::ElevenlabsCustomLlm {
        return Err(AppError::ChannelPlatformSendUnsupported);
    }
    binding::live(state, b).await?;
    let (s, _) = session::prepare(
        state,
        b,
        &uuid::Uuid::new_v4().to_string(),
        "external_guest",
        "external_speech",
    )
    .await?;
    let token = Zeroizing::new(format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    ));
    session::activate_noncall(
        state,
        b,
        &s,
        Some(hex::encode(Sha256::digest(token.as_bytes()))),
    )
    .await?;
    Ok(
        json!({"session_id":s.id,"expires_at":s.expires_at.to_rfc3339(),"extra_body":{"nyxid_session_id":s.id,"nyxid_session_token":token.as_str()}}),
    )
}
pub async fn authenticate(
    state: &AppState,
    b: &Binding,
    headers: &HeaderMap,
    body: &Value,
) -> AppResult<CallSession> {
    if b.profile != Profile::ElevenlabsCustomLlm || body["stream"] != true {
        return Err(AppError::BadRequest(
            "Streaming Chat Completions required".into(),
        ));
    }
    let supplied = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized(
            "Custom LLM authentication required".into(),
        ))?;
    let expected = secret(state, b)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(expected.as_bytes())
        .map_err(|_| AppError::VoiceProviderUnavailable)?;
    mac.update(b"verify");
    let mut check = Hmac::<Sha256>::new_from_slice(supplied.as_bytes())
        .map_err(|_| AppError::VoiceProviderUnavailable)?;
    check.update(b"verify");
    check
        .verify_slice(&mac.finalize().into_bytes())
        .map_err(|_| AppError::Unauthorized("Custom LLM authentication failed".into()))?;
    let extra = body
        .get("elevenlabs_extra_body")
        .ok_or(AppError::AssistantTurnRequired)?;
    let id = extra["nyxid_session_id"]
        .as_str()
        .filter(|s| uuid::Uuid::parse_str(s).is_ok())
        .ok_or(AppError::AssistantTurnRequired)?;
    let token = extra["nyxid_session_token"]
        .as_str()
        .filter(|s| s.len() == 64)
        .ok_or(AppError::AssistantTurnRequired)?;
    let s = session::load(state, id).await?;
    if s.binding_id != b.id
        || s.profile != b.profile
        || s.socket_ticket_hash.as_deref()
            != Some(hex::encode(Sha256::digest(token.as_bytes())).as_str())
    {
        return Err(AppError::AssistantTurnRequired);
    }
    session::fence(state, &s).await?;
    Ok(s)
}
pub fn final_utterance(body: &Value) -> AppResult<(&str, String)> {
    let messages = body["messages"]
        .as_array()
        .filter(|m| !m.is_empty() && m.len() <= 256)
        .ok_or_else(|| AppError::BadRequest("Invalid Custom LLM messages".into()))?;
    let last = messages.last().ok_or(AppError::AssistantTurnRequired)?;
    if last["role"] != "user" {
        return Err(AppError::BadRequest("Final user utterance required".into()));
    }
    let text = last["content"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 8192)
        .ok_or(AppError::AssistantTurnRequired)?;
    // Only the final user content enters the existing conversation. System,
    // assistant, tool messages and a caller-supplied user ID grant no authority.
    let evidence = serde_json::to_vec(messages).map_err(|_| AppError::AssistantTurnRequired)?;
    let event = session::identity(&[
        "elevenlabs-evidence",
        std::str::from_utf8(&evidence).map_err(|_| AppError::AssistantTurnRequired)?,
    ]);
    Ok((text, event))
}
pub fn history(body: &Value, previous: &[String]) -> AppResult<Vec<String>> {
    let messages = body["messages"]
        .as_array()
        .filter(|m| !m.is_empty() && m.len() <= 256)
        .ok_or(AppError::AssistantTurnRequired)?;
    let hashes: Vec<String> = messages
        .iter()
        .map(|m| session::identity(&[&m.to_string()]))
        .collect();
    if !previous.is_empty() && (hashes.len() <= previous.len() || !hashes.starts_with(previous)) {
        return Err(AppError::Conflict("Custom LLM retry or ambiguous history reset/trim; no turn was replayed. Retain advancing full history or start a new guest session.".into()));
    }
    Ok(hashes)
}
pub fn chunk(id: &str, text: &str, last: bool) -> String {
    format!(
        "data: {}\n\n",
        json!({"id":id,"object":"chat.completion.chunk","created":Utc::now().timestamp(),"model":"nyxbot-specialist","choices":[{"index":0,"delta":if last{json!({})}else{json!({"content":text})},"finish_reason":if last{Some("stop")}else{None}}]})
    )
}
pub async fn start(
    state: AppState,
    b: &Binding,
    s: &CallSession,
    body: &Value,
) -> AppResult<(
    tokio::sync::mpsc::Receiver<coordinator::Output>,
    tokio::task::JoinHandle<
        Option<crate::services::assistant_nyxagent_runtime::OwnerTurnSettlement>,
    >,
)> {
    let (text, event) = final_utterance(body)?;
    coordinator::turn_with_history(
        state,
        b,
        s,
        &event,
        text,
        Some(history(body, &s.custom_history)?),
    )
    .await
}
