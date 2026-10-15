//! Shared NyxAgent streaming, continuation and durable settlement mechanics.
//! HTTP adapters subscribe to events; execution owns its permit and persistence.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{assistant_conversation::AssistantConversation, assistant_message::AssistantMessage},
    mw::rate_limit::DirectChatPermit,
    services::assistant_nyxagent::{self as engine, TurnError, TurnResult},
};
use mongodb::bson::doc;
use serde_json::{Value, json};
use std::{future::IntoFuture, time::Duration};
use tokio::sync::broadcast;

pub(crate) struct Events {
    pub(crate) sender: broadcast::Sender<Value>,
    pub(crate) cursor: u64,
    pub(crate) reset_notified: bool,
}
impl Events {
    pub(crate) fn emit(&mut self, event: &str, mut data: Value) {
        self.cursor += 1;
        data["event"] = json!(event);
        data["cursor"] = json!(self.cursor);
        let _ = self.sender.send(data);
    }
    pub(crate) fn notice(&mut self) {
        if self.reset_notified {
            return;
        }
        self.reset_notified = true;
        self.emit(
            "turn.notice",
            json!({"code": "context_reset", "message": engine::CONTEXT_NOTICE}),
        );
    }
}

/// A terminal stream event alone does not prove that a reply was saved.
pub(crate) enum SettledExecution {
    Saved {
        message_id: String,
        error: Option<TurnError>,
    },
    Deleted,
    Unconfirmed,
}

/// Bound database attempts and backoff by one deadline. The permit is released
/// on every outcome; only Saved permits delivery and after-turn work.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn complete_turn<F, Fut>(
    db: &mongodb::Database,
    row: &AssistantConversation,
    turn_id: &str,
    message_id: &str,
    block_id: &str,
    result: &TurnResult,
    permit: DirectChatPermit,
    mut events: Events,
    settle_for: Duration,
    mut persist: F,
) -> SettledExecution
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = AppResult<Option<TurnError>>>,
{
    let deadline = tokio::time::Instant::now() + settle_for;
    let mut attempt = 0u32;
    let settled = loop {
        match tokio::time::timeout_at(deadline, persist()).await {
            Ok(Ok(error)) => {
                break SettledExecution::Saved {
                    message_id: message_id.to_owned(),
                    error,
                };
            }
            // A forced stop or orphan reclaim may already have saved this turn.
            // Database errors remain retryable, never evidence of deletion.
            Ok(Err(AppError::NotFound(_))) => {
                let messages = db.collection::<AssistantMessage>(
                    crate::models::assistant_message::COLLECTION_NAME,
                );
                let read = messages
                    .find_one(doc! {
                        "conversation_id": &row.id, "user_id": &row.user_id,
                        "turn_id": turn_id, "role": "assistant",
                    })
                    .sort(doc! {"created_at": -1});
                match tokio::time::timeout_at(deadline, read.into_future()).await {
                    Ok(Ok(Some(message))) => {
                        break SettledExecution::Saved {
                            message_id: message.id,
                            error: message.error_code.as_deref().map(TurnError::new),
                        };
                    }
                    Ok(Ok(None)) => return SettledExecution::Deleted,
                    Ok(Err(_)) => {}
                    Err(_) => break SettledExecution::Unconfirmed,
                }
            }
            Ok(Err(_)) => {}
            Err(_) => break SettledExecution::Unconfirmed,
        }
        if tokio::time::Instant::now() >= deadline {
            break SettledExecution::Unconfirmed;
        }
        attempt = attempt.saturating_add(1);
        let backoff = Duration::from_millis(100 * (1 << attempt.min(8)));
        tokio::time::sleep_until((tokio::time::Instant::now() + backoff).min(deadline)).await;
    };
    let error = match &settled {
        SettledExecution::Saved { error, .. } => error.clone(),
        SettledExecution::Unconfirmed => {
            tracing::error!(conversation_id = %row.id, turn_id, "Assistant settlement deadline exceeded");
            Some(TurnError::internal("assistant_unavailable", "settlement"))
        }
        SettledExecution::Deleted => unreachable!("deletion returns without terminal events"),
    };
    events.emit(
        "block.completed",
        json!({
            "block_id": block_id,
            "block": {"type": "text", "block_id": block_id, "text": result.text},
        }),
    );
    events.emit("message.completed", json!({"message_id": message_id}));
    let status = match error.as_ref().map(|error| error.code) {
        None => "completed",
        Some("cancelled") => "cancelled",
        Some(_) => "failed",
    };
    events.emit(
        "turn.completed",
        json!({"turn_id": turn_id, "status": status, "error": error}),
    );
    drop(permit);
    settled
}

// Terminal output can rewrite streamed text. Only append a proven suffix;
// block.completed remains authoritative when the provider changes its prefix.
pub(crate) fn emit_text_suffix(
    events: &mut Events,
    block_id: &str,
    emitted: &mut String,
    text: &str,
) {
    if let Some(suffix) = text.strip_prefix(emitted.as_str())
        && !suffix.is_empty()
    {
        events.emit("block.delta", json!({"block_id":block_id,"text":suffix}));
        emitted.push_str(suffix);
    }
}

pub(crate) async fn continue_turn(
    state: &AppState,
    row: &AssistantConversation,
    events: &mut Events,
    continuations: &mut crate::services::assistant_continuation::Continuations,
    binding: Option<&str>,
    error: &TurnError,
    text: &str,
) -> Result<bool, TurnError> {
    if !matches!(error.code, "tool_budget_exhausted" | "turn_timeout") {
        return Ok(false);
    }
    let current = engine::get(&state.db, &row.user_id, &row.id)
        .await
        .map_err(|_| TurnError::internal("assistant_unavailable", "continuation"))?;
    let active = current
        .active_turn
        .as_ref()
        .filter(|t| {
            !t.stop_requested && Some(&t.turn_id) == row.active_turn.as_ref().map(|t| &t.turn_id)
        })
        .ok_or_else(|| TurnError::internal("cancelled", "stop"))?;
    let progress =
        crate::services::assistant_continuation::window_digest(text, &active.tool_progress);
    if !continuations
        .next(error.code, binding.is_some(), progress)
        .map_err(|error| error.at_stage("continuation"))?
    {
        return Ok(false);
    }
    let lease = chrono::Utc::now() + chrono::Duration::seconds(engine::ACTIVE_TURN_TTL_SECS);
    let updated = state.db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(doc! {"_id":&row.id,"user_id":&row.user_id,"active_turn.turn_id":&active.turn_id,"active_turn.stop_requested":false},
            doc! {"$set":{"nyxagent_session_id":binding,"active_turn.continuations":continuations.count,"active_turn.tool_progress": {"started": 0_i64, "calls": 0_i64, "digest": ""},"active_turn.lease_expires_at":mongodb::bson::DateTime::from_chrono(lease)}})
        .await.map_err(|_|TurnError::internal("assistant_unavailable", "continuation"))?;
    if updated.matched_count != 1 {
        return Err(TurnError::internal("cancelled", "stop"));
    }
    tracing::info!(conversation_id = %row.id, upstream_error_code = error.code, continuation = continuations.count, "Continuing assistant task on its existing session");
    events.emit(
        "turn.continuing",
        json!({"turn_id":active.turn_id,"continuation":continuations.count}),
    );
    Ok(true)
}

/// Execution bounds of one upstream request. Tests on a current-thread
/// runtime may shorten them for their own thread only.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StreamBounds {
    /// Until response headers, then until the first body bytes.
    pub first_byte: Duration,
    /// Silence on the transport, keepalives included.
    pub transport_idle: Duration,
    /// No data event (only keepalives): the per-request turn cap.
    pub no_progress: Duration,
    /// The whole turn, across automatic continuations.
    pub turn_cap: Duration,
}

pub(crate) const STREAM_BOUNDS: StreamBounds = StreamBounds {
    first_byte: Duration::from_secs(30),
    transport_idle: Duration::from_secs(120),
    no_progress: Duration::from_secs(610),
    turn_cap: Duration::from_secs(
        610 * (crate::services::assistant_continuation::HARD_MAX as u64 + 1),
    ),
};

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_STREAM_BOUNDS: std::cell::Cell<Option<StreamBounds>> =
        const { std::cell::Cell::new(None) };
}

pub(crate) fn stream_bounds() -> StreamBounds {
    #[cfg(test)]
    if let Some(bounds) = TEST_STREAM_BOUNDS.with(std::cell::Cell::get) {
        return bounds;
    }
    STREAM_BOUNDS
}

/// Time left before the next body chunk must arrive.
pub(crate) fn stream_timeout(
    bounds: StreamBounds,
    transport_elapsed: Duration,
    semantic_elapsed: Duration,
) -> Duration {
    bounds
        .transport_idle
        .saturating_sub(transport_elapsed)
        .min(bounds.no_progress.saturating_sub(semantic_elapsed))
}

pub(crate) fn sanitize_upstream_request_id(value: &str) -> Option<String> {
    (value.len() <= 128
        && !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte)))
    .then(|| value.to_owned())
}
