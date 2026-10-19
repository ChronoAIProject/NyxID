//! Shared NyxAgent streaming, continuation and durable settlement mechanics.
//! HTTP adapters subscribe to events; execution owns its permit and persistence.
use crate::models::assistant_agent::AssistantAgent;
use crate::models::assistant_conversation::TurnOrigin;
use crate::services::assistant_nyxagent::{excerpt, identifier};
use crate::services::{assistant_group_service as groups, assistant_team_service as team};
use chrono::{DateTime, Utc};

use super::assistant_nyxagent::{RecoveryAction, ResponseStream};
use super::assistant_service;
use super::{
    assistant_acknowledgement_service as acknowledgements,
    assistant_agent_credential_service::{self as credentials, AssistantCredential},
    billing::route_inventory::BillingRoutePolicy,
    platform_manager_contract as contract,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{assistant_conversation::AssistantConversation, assistant_message::AssistantMessage},
    mw::rate_limit::DirectChatPermit,
    services::assistant_nyxagent::{self as engine, TurnError, TurnResult},
};
use futures::StreamExt;
use mongodb::bson::doc;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::LazyLock, time::Instant};
use std::{future::IntoFuture, time::Duration};
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use tokio::sync::oneshot;
use tokio_util::task::AbortOnDropHandle;
use uuid::Uuid;

/// HTTP composition supplies existing reporting and queue wake-up behavior.
/// Called only with a confirmed durable reply, never a candidate stream result.
#[async_trait::async_trait]
pub trait OwnerTurnSettledHook: Send + Sync {
    async fn settled(&self, row: AssistantConversation, reply: contract::ReplyRef);
}

pub fn producer(
    state: AppState,
    egress: std::sync::Arc<dyn contract::NyxAgentEgress>,
    public_runtime: std::sync::Arc<dyn contract::PublicNyxAgentRuntime>,
) -> std::sync::Arc<dyn contract::NyxAgentExecutionProducer> {
    std::sync::Arc::new(ExecutionProducer {
        state,
        egress,
        public_runtime,
    })
}

struct ExecutionProducer {
    state: AppState,
    egress: std::sync::Arc<dyn contract::NyxAgentEgress>,
    public_runtime: std::sync::Arc<dyn contract::PublicNyxAgentRuntime>,
}

#[async_trait::async_trait]
impl contract::NyxAgentExecutionProducer for ExecutionProducer {
    async fn execute_nyxagent_turn(
        &self,
        db: &mongodb::Database,
        context: contract::NyxAgentExecutionContext,
        input: contract::TurnInput,
        events: tokio::sync::mpsc::Sender<contract::ExecutionEvent>,
    ) -> AppResult<()> {
        // Public preparation is selected before feature, owner, history, key,
        // limiter or agent lookup. A public context never enters the owner loop.
        let (user_id, turn, origin) = match context {
            contract::NyxAgentExecutionContext::Public(context) => {
                return super::platform_public_execution::execute(
                    db,
                    self.public_runtime.clone(),
                    self.egress.clone(),
                    context,
                    input,
                    events,
                )
                .await;
            }
            contract::NyxAgentExecutionContext::Owner {
                user_id,
                turn,
                platform_origin: Some(origin),
            } => (user_id, turn, origin),
            // Ordinary HTTP wrappers retain their real-auth egress adapter.
            contract::NyxAgentExecutionContext::Owner {
                platform_origin: None,
                ..
            } => return Err(AppError::AssistantTurnRequired),
        };
        engine::require_enabled(db, &user_id).await?;
        let row = engine::get(db, &user_id, &turn.conversation_id).await?;
        let expected = super::platform_personal_turn_service::model_origin(&origin);
        let active = engine::live_turn(&row, Utc::now())
            .filter(|active| {
                active.turn_id == turn.turn_id
                    && !active.stop_requested
                    && active.platform_origin.as_ref() == Some(&expected)
            })
            .ok_or(AppError::AssistantTurnRequired)?;
        if origin.authority.user_id != user_id
            || row.platform_thread.as_ref() != Some(&expected.thread)
        {
            return Err(AppError::AssistantTurnRequired);
        }
        // Execution resumes the admitted input; the caller cannot replace it.
        let seq = active
            .initiating_message_seq
            .ok_or(AppError::AssistantTurnRequired)?;
        let message = db
            .collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME)
            .find_one(doc! {
                "conversation_id":&turn.conversation_id,"user_id":&user_id,
                "turn_id":&turn.turn_id,"seq":seq,"role":{"$in":["user","system"]},
            })
            .await?
            .ok_or(AppError::AssistantTurnRequired)?;
        if message.text != input.text || message.platform_origin.as_ref() != Some(&expected) {
            return Err(AppError::AssistantTurnRequired);
        }
        // Load only the existing encrypted key. No provisioning, authority
        // convergence, admission, replacement or owner fallback occurs here.
        let credential = credentials::load_for_conversation(
            db,
            &self.state.encryption_keys,
            &user_id,
            &turn.conversation_id,
        )
        .await?
        .filter(|credential| credential.api_key_id == row.credential_api_key_id)
        .ok_or(AppError::AssistantTurnRequired)?;
        let permit = self
            .state
            .direct_chat_limiter
            .try_acquire_pool("assistant_channel", &user_id, 2)
            .await?
            .ok_or(AppError::AssistantTurnActive)?;
        let (sender, _) = broadcast::channel(256);
        let saved = run_owner_turn(
            self.state.clone(),
            self.egress.clone(),
            row.clone(),
            message.text,
            credential,
            Some(BillingRoutePolicy::Metered(
                super::billing::route_inventory::BillingIngress::Proxy,
            )),
            permit,
            Events {
                sender,
                cursor: 0,
                reset_notified: false,
            },
        )
        .await
        .ok_or(AppError::AssistantTurnRequired)?;
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            events.send(contract::ExecutionEvent::ReplyReady(contract::ReplyRef {
                turn: turn.clone(),
                reply_id: saved.message_id,
            })),
        )
        .await;
        if saved.error.is_none()
            && row.message_count == 1
            && super::assistant_title_service::eligible(&row)
        {
            super::assistant_title_service::spawn(self.state.clone(), row.user_id, row.id);
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            events.send(contract::ExecutionEvent::Settled { turn }),
        )
        .await;
        Ok(())
    }
}

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
    let saved_message_id = match &settled {
        SettledExecution::Saved { message_id, .. } => message_id.as_str(),
        _ => message_id,
    };
    let metadata = db.collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(doc! {"_id": saved_message_id, "conversation_id": &row.id, "user_id": &row.user_id, "turn_id": turn_id})
        .await.ok().flatten().and_then(|message| message.turn_metadata);
    events.emit(
        "turn.completed",
        json!({"turn_id": turn_id, "status": status, "error": error, "turn_metadata": metadata}),
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
    let update = doc! {"$set":{"nyxagent_session_id":binding,"active_turn.continuations":continuations.count,"active_turn.tool_progress": {"started": 0_i64, "calls": 0_i64, "digest": ""},"active_turn.lease_expires_at":mongodb::bson::DateTime::from_chrono(lease)}};
    if active.platform_origin.is_some() {
        super::platform_personal_turn_service::update_platform_turn(&state.db, row, update)
            .await
            .map_err(|_| TurnError::internal("cancelled", "continuation"))?;
    } else {
        let updated=state.db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(doc! {"_id":&row.id,"user_id":&row.user_id,"active_turn.turn_id":&active.turn_id,"active_turn.stop_requested":false},update)
            .await.map_err(|_|TurnError::internal("assistant_unavailable", "continuation"))?;
        if updated.matched_count != 1 {
            return Err(TurnError::internal("cancelled", "stop"));
        }
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

/// What the agent's other threads are working on right now, so it neither
/// redoes that work nor starts it twice. Lookup failures only omit it.
async fn in_progress_note(
    state: &AppState,
    row: &AssistantConversation,
    agent: &AssistantAgent,
) -> String {
    let Ok(others) = team::in_progress(&state.db, &row.user_id, &agent.id, &row.id).await else {
        return String::new();
    };
    if others.is_empty() {
        return String::new();
    }
    let this = row
        .active_turn
        .as_ref()
        .and_then(|turn| turn.question_key.as_deref());
    let mut note = String::from(
        "\n\nYour other threads are working on these right now; do not start the same work \
        again (if you are asked the same thing, say it is in progress there):",
    );
    for (title, question, key) in others {
        note.push_str(&format!(
            "\n- \"{}\": \"{}\"{}",
            excerpt(&title, 60).replace('"', "'"),
            excerpt(question.as_deref().unwrap_or("(working)"), 200).replace('"', "'"),
            if this.is_some() && key.as_deref() == this {
                " (the same question as this one)"
            } else {
                ""
            }
        ));
    }
    note
}

/// Turn-scoped input context: drained events, a channel sender's context,
/// and for NyxBot its roster, direct
/// user chats with specialists and pending permission requests. NyxID-authored
/// and bounded; lookup failures only omit a note.
pub(crate) async fn turn_notes(
    state: &AppState,
    row: &AssistantConversation,
    agent: Option<&AssistantAgent>,
    previous_user_message: Option<DateTime<Utc>>,
) -> String {
    let mut notes = engine::channel_note(row);
    if let Some(turn) = row.active_turn.as_ref() {
        if !row.guest_turn && turn.origin != TurnOrigin::Event && !turn.events.is_empty() {
            notes.push_str(
                "\n\nNyxID events since your previous turn (authored by NyxID; only a quoted \
                owner message is a request from the user):",
            );
            for event in &turn.events {
                notes.push_str("\n- ");
                notes.push_str(&excerpt(&event.text, engine::event_text_limit(event)));
            }
        }
        if let Some(note) = turn.note.as_deref() {
            notes.push_str("\n\n");
            notes.push_str(note);
        }
    }
    // Someone other than the owner is talking: nothing private to the owner
    // (memory, other chats, the team, pending requests) goes into this turn.
    if row.guest_turn {
        return notes;
    }
    if let Some(agent) = agent {
        // Only the agent's own threads hear about its other chats.
        if row.channel.is_none() && row.group_request_id.is_none() {
            notes.push_str(&in_progress_note(state, row, agent).await);
        }
        if row.group_request_id.is_none()
            && let Some(note) = group_note(state, row, agent).await
        {
            notes.push_str("\n\n");
            notes.push_str(&note);
        }
    }
    if row.is_subagent() {
        return notes;
    }
    let owner = row.user_id.as_str();
    notes.push_str(
        &team::roster_note(&state.db, owner)
            .await
            .unwrap_or_default(),
    );
    if let Some(since) = previous_user_message {
        notes.push_str(
            &team::direct_chats_note(&state.db, owner, since)
                .await
                .unwrap_or_default(),
        );
        notes.push_str(
            &crate::services::assistant_voice::published_result_notes(
                &state.db, owner, &row.id, since,
            )
            .await
            .unwrap_or_default(),
        );
    }
    if let Ok(requests) = team::pending_requests(&state.db, owner).await
        && let Ok(summaries) = team::request_summaries(&state.db, owner, &requests).await
        && !summaries.is_empty()
    {
        notes.push_str(
            "\n\nPending specialist permission requests (decide with nyxid__decide_permission):",
        );
        for request in summaries.iter().take(10) {
            notes.push_str(&format!(
                "\n- request_id {} from {}: {} {}",
                request.request_id,
                identifier(&request.agent),
                identifier(&request.kind),
                identifier(request.service_slug.as_deref().unwrap_or("account"))
            ));
        }
    }
    notes
}

pub(crate) async fn group_note(
    state: &AppState,
    row: &AssistantConversation,
    agent: &AssistantAgent,
) -> Option<String> {
    let group_id = row.group_id.as_deref()?;
    if row.agent_owner_id.is_some() {
        let access =
            crate::services::org_group_service::get(&state.db, &row.user_id, group_id, None)
                .await
                .ok()?;
        if access.org.is_some() {
            return crate::services::org_group_service::note(&state.db, &access, agent)
                .await
                .ok();
        }
    }
    let group = groups::get(&state.db, &row.user_id, group_id).await.ok()?;
    let members = groups::members(&state.db, &row.user_id, &group)
        .await
        .ok()?;
    let others: Vec<String> = members
        .iter()
        .filter(|member| member.id != agent.id && member.destroyed_at.is_none())
        .map(|member| format!("@{}", identifier(&member.name)))
        .collect();
    Some(format!(
        "You are {} in the group chat {} with the user and {}. Your reply is posted to the \
        group as you. Answer what was asked of you. To hand work to a member write their \
        @name; mention nobody when you are done or answering the user. The user cannot see \
        confirmation cards here: only when a tool result contains a confirmation card, ask them to reply with \
        its confirm_phrase (for example \"yes 4821\"). When a result has review_url instead \
        of confirm_phrase, send that full URL so the owner can review all files in NyxID. \
        Group replies never approve publication of a skill draft.",
        identifier(&agent.name),
        identifier(&group.name),
        if others.is_empty() {
            "no other agents".to_owned()
        } else {
            others.join(", ")
        }
    ))
}

/// The private text is only for ordinary after-turn hooks. Manager delivery
/// publishes the durable message ID and resolves its retained body separately.
pub(crate) struct OwnerTurnSettlement {
    pub message_id: String,
    pub text: String,
    pub error: Option<TurnError>,
}

pub(crate) fn owner_context(
    row: &AssistantConversation,
) -> AppResult<contract::NyxAgentExecutionContext> {
    Ok(contract::NyxAgentExecutionContext::Owner {
        user_id: row.user_id.clone(),
        turn: contract::TurnRef {
            conversation_id: row.id.clone(),
            turn_id: row
                .active_turn
                .as_ref()
                .map(|turn| turn.turn_id.clone())
                .unwrap_or_default(),
        },
        platform_origin: row
            .active_turn
            .as_ref()
            .and_then(|turn| turn.platform_origin.as_ref())
            .map(super::platform_personal_turn_service::contract_origin)
            .transpose()?,
    })
}

#[allow(clippy::too_many_arguments)]
async fn request(
    _state: &AppState,
    egress: &dyn contract::NyxAgentEgress,
    authority: &contract::NyxAgentExecutionContext,
    credential: &AssistantCredential,
    method: &str,
    path: &str,
    body: Option<Value>,
    request_id: Option<&str>,
    policy: Option<BillingRoutePolicy>,
) -> AppResult<axum::response::Response> {
    let method = method
        .parse()
        .map_err(|_| AppError::Internal("Invalid assistant method".into()))?;
    egress
        .request(
            authority,
            &contract::RuntimeCredential {
                api_key_id: credential.api_key_id.clone(),
                raw_key: credential.raw_key.clone(),
            },
            contract::NyxAgentRequest {
                method,
                relative_path: path.to_owned(),
                body,
                idempotency_key: request_id
                    .map(str::to_owned)
                    .unwrap_or_else(|| match authority {
                        contract::NyxAgentExecutionContext::Owner {
                            turn,
                            platform_origin: Some(_),
                            ..
                        } => format!("{}:metadata:{}", turn.turn_id, Uuid::new_v4()),
                        _ => String::new(),
                    }),
                billing_policy: policy,
            },
        )
        .await
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_owner_turn(
    state: AppState,
    egress: std::sync::Arc<dyn contract::NyxAgentEgress>,
    row: AssistantConversation,
    text: String,
    mut credential: AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    permit: DirectChatPermit,
    mut events: Events,
) -> Option<OwnerTurnSettlement> {
    if row
        .active_turn
        .as_ref()
        .is_some_and(|turn| turn.platform_origin.is_some())
        && super::platform_personal_turn_service::claim_platform_worker(
            &state.db,
            &row,
            &credential.api_key_id,
        )
        .await
        .is_err()
    {
        return None;
    }
    let turn_id = row
        .active_turn
        .as_ref()
        .expect("claimed turn")
        .turn_id
        .clone();
    let message_id = Uuid::new_v4().to_string();
    let block_id = format!("{message_id}-text");
    tracing::info!(turn_id = %turn_id,
        worker_instance = %super::assistant_engine_worker::instance().id,
        worker_hostname = super::assistant_engine_worker::instance().hostname.as_deref(),
        "Assistant turn started");
    let (heartbeat_stop, mut heartbeat_stop_rx) = oneshot::channel();
    let heartbeat_db = state.db.clone();
    let heartbeat_user = row.user_id.clone();
    let heartbeat_conversation = row.id.clone();
    let heartbeat_turn = turn_id.clone();
    // AbortOnDropHandle ties the child to this worker: a panic or task abort
    // cannot leave a fresh heartbeat behind to hide an orphaned turn.
    let heartbeat_task = AbortOnDropHandle::new(tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_secs(engine::ACTIVE_TURN_HEARTBEAT_SECS));
        interval.tick().await;
        loop {
            tokio::select! {
                _ = &mut heartbeat_stop_rx => break,
                _ = interval.tick() => {
                    match engine::heartbeat(&heartbeat_db, &heartbeat_user, &heartbeat_conversation, &heartbeat_turn).await {
                        Ok(true) => {}
                        // The turn was fenced (stopped, reclaimed or deleted).
                        Ok(false) => break,
                        // A transient database error must not silence a live
                        // worker: the orphan sweep would reclaim its turn. Retry
                        // on the next tick.
                        Err(_) => {}
                    }
                }
            }
        }
    }));
    events.emit(
        "turn.status",
        json!({"conversation_id": row.id, "turn_id": turn_id, "status": "running"}),
    );
    events.emit(
        "message.started",
        json!({"message_id": message_id, "role": "assistant"}),
    );
    events.emit(
        "block.started",
        json!({
            "message_id": message_id,
            "block_id": block_id,
            "index": 0,
            "block": {"type": "text", "block_id": block_id, "text": ""},
        }),
    );
    let mut partial = String::new();
    let mut instruction_context = None;
    let mut result = crate::services::assistant_engine_worker::contain(&row, async {
        // Execution includes upload planning and upstream streaming. Keep that
        // state off the caller's stack when this task is created by a tool.
        let mut execution = Box::pin(async {
            let authority = owner_context(&row)
                .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
            execute_owner_turn(
                &state,
                egress.as_ref(),
                &authority,
                &row,
                &text,
                &mut credential,
                policy,
                &mut events,
                &block_id,
                &mut partial,
                &mut instruction_context,
            )
            .await
        });
        tokio::select! {
            result = &mut execution => result,
            () = permit.cancelled() => Err(TurnError::internal("assistant_unavailable", "permit")),
            error = watch_stop(&state, &row, &turn_id) => Err(error),
            () = tokio::time::sleep(stream_bounds().turn_cap) => {
                Err(TurnError::internal("turn_timeout", "turn_deadline"))
            }
        }
    })
    .await
    .unwrap_or_else(|error| TurnResult {
        text: partial,
        session_id: None,
        response_id: None,
        error: Some(error),
    });
    if result
        .error
        .as_ref()
        .is_some_and(|error| error.code == "cancelled")
    {
        // Platform revocation already captured durable exact effects. App Stop
        // keeps its ordinary broader cancellation at its explicit endpoint.
        if row
            .active_turn
            .as_ref()
            .is_none_or(|turn| turn.platform_origin.is_none())
        {
            let _ = crate::services::machine_service::cancel_conversation(
                &state,
                &row.user_id,
                &row.id,
            )
            .await;
        }
    }
    // Never echo the system-managed credential, even if an upstream reflects it.
    result.text = result
        .text
        .replace(credential.raw_key.as_str(), "[redacted]");
    let settled = complete_turn(
        &state.db,
        &row,
        &turn_id,
        &message_id,
        &block_id,
        &result,
        permit,
        events,
        Duration::from_secs(engine::SETTLEMENT_GRACE_SECS),
        || {
            engine::finish_turn_with_instructions(
                &state.db,
                &row,
                &credential.api_key_id,
                &message_id,
                &result,
                instruction_context.as_ref().map(|context| &context.binding),
            )
        },
    )
    .await;
    let _ = heartbeat_stop.send(());
    let _ = heartbeat_task.await;
    match settled {
        SettledExecution::Saved { message_id, error } => Some(OwnerTurnSettlement {
            message_id,
            error,
            text: result.text,
        }),
        SettledExecution::Deleted | SettledExecution::Unconfirmed => None,
    }
}

async fn watch_stop(state: &AppState, row: &AssistantConversation, turn_id: &str) -> TurnError {
    loop {
        match engine::stop_requested(&state.db, &row.user_id, &row.id, turn_id).await {
            Ok(false) => tokio::time::sleep(Duration::from_millis(250)).await,
            Ok(true) => return TurnError::internal("cancelled", "stop"),
            Err(_) => return TurnError::internal("assistant_unavailable", "stop"),
        }
    }
}

/// Typed proxy failures collapse to `assistant_unavailable`, except a billing
/// refusal, which keeps its stable code so the client can offer a top-up.
pub(crate) fn proxy_turn_error(error: AppError) -> TurnError {
    match error {
        AppError::InsufficientCredits => TurnError::internal("insufficient_credits", "connect"),
        AppError::WalletSuspended => TurnError::internal("wallet_suspended", "connect"),
        AppError::PlanEntitlementRequired(_) => {
            TurnError::internal("plan_entitlement_required", "connect")
        }
        _ => TurnError::internal("assistant_unavailable", "connect"),
    }
}

/// Reads the NyxAgent `error.code` envelope, or NyxID's own flat envelope
/// (`{"error":"insufficient_credits",...}`) on a 402. Other flat categories are
/// not turn codes and stay unrecognized.
pub(crate) fn upstream_error_code(status: u16, envelope: &Value) -> &str {
    if let Some(code) = envelope["error"]["code"].as_str() {
        return code;
    }
    if status == 402
        && let Some(
            code @ ("insufficient_credits" | "wallet_suspended" | "plan_entitlement_required"),
        ) = envelope["error"].as_str()
    {
        return code;
    }
    ""
}

async fn clear_session(
    state: &AppState,
    row: &AssistantConversation,
    reason: &str,
) -> AppResult<()> {
    if row
        .active_turn
        .as_ref()
        .is_some_and(|turn| turn.platform_origin.is_some())
    {
        super::platform_personal_turn_service::update_platform_turn(&state.db,row,doc! {"$set":{
            "nyxagent_session_id":mongodb::bson::Bson::Null,"nyxagent_instruction_binding":mongodb::bson::Bson::Null,
            "nyxagent_last_response_id":mongodb::bson::Bson::Null,"context_reset_at":mongodb::bson::DateTime::now(),"context_reset_reason":reason,
        }}).await
    } else {
        engine::clear_binding(
            &state.db,
            &row.user_id,
            &row.id,
            &row.active_turn
                .as_ref()
                .ok_or(AppError::AssistantTurnRequired)?
                .turn_id,
            reason,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_owner_turn(
    state: &AppState,
    egress: &dyn contract::NyxAgentEgress,
    authority: &contract::NyxAgentExecutionContext,
    row: &AssistantConversation,
    text: &str,
    credential: &mut AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    events: &mut Events,
    block_id: &str,
    partial: &mut String,
    instruction_context: &mut Option<crate::services::assistant_instruction_context::Prepared>,
) -> Result<TurnResult, TurnError> {
    let contract::NyxAgentExecutionContext::Owner {
        user_id,
        turn,
        platform_origin,
    } = authority
    else {
        return Err(TurnError::internal("assistant_unavailable", "prepare"));
    };
    if user_id != &row.user_id
        || turn.conversation_id != row.id
        || row
            .active_turn
            .as_ref()
            .is_none_or(|active| active.turn_id != turn.turn_id)
    {
        return Err(TurnError::internal("assistant_unavailable", "prepare"));
    }
    let expected = row
        .active_turn
        .as_ref()
        .and_then(|turn| turn.platform_origin.as_ref())
        .map(super::platform_personal_turn_service::contract_origin)
        .transpose()
        .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
    if &expected != platform_origin {
        return Err(TurnError::internal("assistant_unavailable", "prepare"));
    }
    if expected.is_some() {
        super::platform_personal_turn_service::update_platform_turn(
            &state.db,
            row,
            doc! {"$inc":{"execution_fence":1_i64}},
        )
        .await
        .map_err(|_| TurnError::internal("cancelled", "prepare"))?;
    }
    let attempt = super::assistant_service_execution::begin_model_attempt(&state.db, row)
        .await
        .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
    let mut tool_images = images::ToolImages::default();
    let mut outcome = Box::pin(execute_turn_inner(
        state,
        egress,
        authority,
        row,
        text,
        credential,
        policy,
        events,
        block_id,
        partial,
        instruction_context,
        &mut tool_images,
    ))
    .await;
    let model_refusal =
        super::assistant_service_execution::finish_model_attempt(&state.db, &attempt)
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "recovery"))?;
    // A current callback refusal can precede any reply. Cancellation in either
    // result shape wins, and an unrelated pending action cannot mask a failure.
    let cancelled = outcome
        .as_ref()
        .is_err_and(|error| error.code == "cancelled")
        || outcome.as_ref().is_ok_and(|result| {
            result
                .error
                .as_ref()
                .is_some_and(|error| error.code == "cancelled")
        });
    if !cancelled
        && (outcome.is_err() || outcome.as_ref().is_ok_and(|result| result.error.is_some()))
        && let Some(summary) = model_refusal
    {
        let text = format!(
            "Please review the confirmation card: {summary}. I’ll continue after you allow it."
        );
        *partial = text.clone();
        outcome = Ok(TurnResult {
            text,
            session_id: None,
            response_id: None,
            error: None,
        });
    }
    if outcome.as_ref().is_err() || outcome.as_ref().is_ok_and(|r| r.error.is_some()) {
        tool_images.failed(state, row).await;
    }
    if tool_images.rounds > 0
        && let Some(mut fallback) = tool_images.fallback.take()
        && (outcome.as_ref().is_err_and(|e| e.code != "cancelled")
            || outcome
                .as_ref()
                .is_ok_and(|r| r.error.as_ref().is_some_and(|e| e.code != "cancelled")))
    {
        events.emit("turn.notice", json!({"code":"image_input_unavailable", "message":"Tool images could not be viewed; the completed answer was preserved."}));
        clear_session(state, row, "image_input_unavailable")
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "recovery"))?;
        fallback.session_id = None;
        fallback.response_id = None;
        *instruction_context = None;
        *partial = fallback.text.clone();
        return Ok(fallback);
    }
    outcome
}

struct EngineAttempt<'a> {
    row: &'a AssistantConversation,
    number: u32,
    http_status: Option<u16>,
    upstream_code: Option<Box<str>>,
    wait_started: Option<DateTime<Utc>>,
    wait_ended: Option<DateTime<Utc>>,
    accrued_wait_ms: u64,
    deadline: tokio::time::Instant,
    logged: bool,
}
impl EngineAttempt<'_> {
    fn log(&mut self, decision: &str) {
        let turn = self.row.active_turn.as_ref().expect("claimed turn");
        tracing::info!(target: "nyxid::assistant_engine", conversation_id = %self.row.id,
            turn_id = %turn.turn_id, origin = crate::services::assistant_turn_observability::origin(turn),
            worker_instance = %crate::services::assistant_engine_worker::instance().id,
            worker_hostname = crate::services::assistant_engine_worker::instance().hostname.as_deref(),
            attempt = self.number, http_status = self.http_status, upstream_code = self.upstream_code.as_deref(),
            decision, waited_ms = self.accrued_wait_ms.saturating_add(self.wait_started.map_or(0, |at| (self.wait_ended.unwrap_or_else(Utc::now) - at).num_milliseconds().max(0) as u64)),
            remaining_deadline_ms = self.deadline.saturating_duration_since(tokio::time::Instant::now()).as_millis() as u64,
            "Assistant engine attempt");
        self.logged = true;
    }
}
impl Drop for EngineAttempt<'_> {
    fn drop(&mut self) {
        if !self.logged {
            self.log("failed");
        }
    }
}

struct AdmissionAcknowledgement {
    due: Option<tokio::time::Instant>,
    started_at: Option<DateTime<Utc>>,
    emitted: bool,
    checked: bool,
    enabled: bool,
    notice: Option<CapacityNotice>,
}
impl AdmissionAcknowledgement {
    async fn start(
        &mut self,
        state: &AppState,
        row: &AssistantConversation,
        deadline: DateTime<Utc>,
    ) -> Result<(), TurnError> {
        if self.due.is_some() {
            return Ok(());
        }
        let now = Utc::now();
        let started = row
            .active_turn
            .as_ref()
            .and_then(|t| t.engine_request_started_at)
            .unwrap_or(now);
        crate::services::assistant_turn_observability::request_started(
            &state.db, row, started, deadline,
        )
        .await
        .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
        self.started_at = Some(started);
        let remaining = (started
            + chrono::Duration::from_std(crate::services::assistant_engine_policy::ACK_DELAY)
                .expect("bounded acknowledgement delay")
            - now)
            .to_std()
            .unwrap_or_default();
        self.due = Some(tokio::time::Instant::now() + remaining);
        Ok(())
    }

    async fn wait<F: std::future::Future>(
        &mut self,
        future: F,
        state: &AppState,
        row: &AssistantConversation,
        events: &mut Events,
        egress: &dyn contract::NyxAgentEgress,
    ) -> F::Output {
        if !self.enabled || self.checked {
            return future.await;
        }
        let Some(due) = self.due else {
            return future.await;
        };
        tokio::pin!(future);
        tokio::select! {
            biased;
            result = &mut future => result,
            _ = tokio::time::sleep_until(due) => {
                self.checked = true;
                if let Ok(Some(turn)) = crate::services::assistant_turn_observability::acknowledge(&state.db, row, Utc::now()).await {
                    self.emitted = true;
                    events.emit("turn.status", json!({"conversation_id": row.id, "turn_id": turn.turn_id,
                        "status": "waiting", "waiting_for_capacity": turn.waiting_for_capacity,
                        "capacity_wait_note_origin": turn.capacity_wait_note_origin,
                        "engine_request_started_at": turn.engine_request_started_at,
                        "capacity_wait_started_at": turn.capacity_wait_started_at,
                        "capacity_wait_deadline": turn.capacity_wait_deadline}));
                    self.notice = egress.admission_acknowledged(row).map(CapacityNotice);
                }
                future.await
            }
        }
    }

    fn accepted(&mut self) {
        self.enabled = false;
        self.emitted = false;
        self.notice.take();
    }
}

struct CapacityNotice(tokio::task::JoinHandle<()>);
impl Drop for CapacityNotice {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_turn_inner(
    state: &AppState,
    egress: &dyn contract::NyxAgentEgress,
    authority: &contract::NyxAgentExecutionContext,
    row: &AssistantConversation,
    text: &str,
    credential: &mut AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    events: &mut Events,
    block_id: &str,
    partial: &mut String,
    instruction_context: &mut Option<crate::services::assistant_instruction_context::Prepared>,
    tool_images: &mut images::ToolImages,
) -> Result<TurnResult, TurnError> {
    let turn = row.active_turn.as_ref().expect("claimed turn");
    let run_deadline = if let Some(run_id) = turn
        .trigger_run_id
        .as_ref()
        .filter(|_| !turn.trigger_continuation)
    {
        Some(
            state
                .db
                .collection::<crate::models::trigger_run::TriggerRun>(
                    crate::models::trigger_run::COLLECTION_NAME,
                )
                .find_one(doc! {"_id": run_id, "user_id": &row.user_id})
                .await
                .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?
                .ok_or_else(|| TurnError::internal("assistant_unavailable", "prepare"))?
                .deadline,
        )
    } else {
        None
    };
    let mut wait_started = turn
        .capacity_wait_started_at
        .filter(|_| turn.waiting_for_capacity);
    let mut accrued_wait_ms = turn.capacity_wait_ms;
    let mut acknowledgement = AdmissionAcknowledgement {
        due: None,
        started_at: None,
        emitted: false,
        checked: false,
        enabled: turn.origin != crate::models::assistant_conversation::TurnOrigin::Trigger
            && turn.trigger_run_id.is_none()
            && turn.capacity_accepted_at.is_none(),
        notice: None,
    };
    let turn_id = &row.active_turn.as_ref().expect("claimed turn").turn_id;
    let mut history = engine::messages(
        &state.db,
        &row.user_id,
        &row.id,
        20,
        Some(row.message_count + 1),
    )
    .await
    .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
    // Queued speech is visible to the human, but never becomes an instruction
    // until its own claim. Include earlier completed replies even if their
    // sequence is after the input that queued this turn.
    history.retain(|message| !message.execution_pending && message.turn_id != *turn_id);
    // A guest's recap holds only what the chat itself saw.
    if row.guest_turn {
        use crate::models::assistant_conversation::TurnOrigin;
        // Only the chat's own messages and the replies to them; never NyxID's
        // notices, event-turn replies (not always delivered) or anything
        // said in the app.
        history.retain(|message| {
            message.origin == Some(TurnOrigin::Channel)
                && matches!(message.role.as_str(), "user" | "assistant")
        });
    }
    // Cards decided while an earlier turn was still running never reached the
    // model: NyxAgent ends a turn on a card and answers repeats locally. Report
    // decisions made since the previous user message; a lookup failure only
    // omits the note.
    let previous = engine::previous_user_message(
        &state.db,
        &row.user_id,
        &row.id,
        row.active_turn
            .as_ref()
            .and_then(|turn| turn.initiating_message_seq)
            .or(Some(row.message_count)),
    )
    .await
    .unwrap_or_default()
    .map(|message| message.created_at);
    let mut decisions = match previous.filter(|_| !row.guest_turn) {
        Some(previous) => {
            acknowledgements::decided_since(&state.db, &row.user_id, &row.id, previous)
                .await
                .map(|rows| acknowledgements::decisions_note(&rows))
                .unwrap_or_default()
        }
        None => String::new(),
    };
    // Turn-scoped NyxID notes: team state, direct chats, drained events and
    // channel sender context. Lookup failures only omit a note.
    let agent =
        match crate::services::assistant_team_service::agent_for_conversation(&state.db, row).await
        {
            Ok(agent) => Some(agent),
            Err(_) if row.agent_owner_id.is_some() => {
                return Err(TurnError::internal("assistant_unavailable", "prepare"));
            }
            Err(_) => None,
        };
    decisions.push_str(&turn_notes(state, row, agent.as_ref(), previous).await);
    decisions.push_str(
        &crate::services::async_service_operation::input_context(
            &state.db,
            &state.encryption_keys,
            row,
        )
        .await
        .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?,
    );
    // Rollout-gated authoring rules are instructions, so they belong in the
    // stable (fingerprinted) part rather than the per-turn quoted context.
    let guidance = if !row.guest_turn
        && crate::services::assistant_skill_authoring::enabled(&state.db, &row.user_id)
            .await
            .unwrap_or(false)
    {
        if row.is_subagent() {
            crate::services::assistant_skill_authoring::SPECIALIST_GUIDANCE
        } else {
            crate::services::assistant_skill_authoring::GUIDANCE
        }
    } else {
        ""
    };
    let prepared = crate::services::assistant_instruction_context::Prepared::with_guidance(
        state.audit_chain_hmac_key.as_ref().as_ref(),
        row,
        agent.as_ref(),
        &history,
        guidance,
    )
    .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
    *instruction_context = Some(prepared);
    let context = instruction_context.as_mut().expect("prepared above");
    // A recap leads with the durable checkpoint; guest audiences never get it.
    if !row.guest_turn {
        context.set_checkpoint(
            engine::checkpoint(&state.db, &row.user_id, &row.id)
                .await
                .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?,
        );
    }
    let mut binding = row.nyxagent_session_id.clone();
    if context.reset {
        clear_session(state, row, "instructions_changed")
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
        binding = None;
    }
    let recap = binding.is_none()
        && (context.reset
            || row
                .context_reset_reason
                .as_deref()
                .is_some_and(|reason| reason != "user_reset"));
    if recap {
        events.notice();
    }
    let saved_recap = tokio::sync::OnceCell::new();
    let load_recap = || engine::recap(&state.db, row, &history, turn_id);
    let mut prompt = context
        .session_instructions(recap, &saved_recap, load_recap)
        .await
        .map_err(|_| TurnError::new("assistant_unavailable"))?;
    let mut attachments = crate::services::assistant_upload_service::turn_attachments(
        &state.db,
        &row.user_id,
        &row.id,
        turn_id,
    )
    .await
    .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
    let expired = Box::pin(crate::services::assistant_upload_retention::expired_ids(
        &state.db,
        &row.user_id,
        &attachments.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
    ))
    .await
    .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
    let mut listing = crate::services::assistant_upload_service::listing(&attachments);
    if !expired.is_empty() {
        listing.push_str(&format!(
            "\nAttachments expired per retention policy; ask the owner to upload them again: {}",
            serde_json::json!(expired)
        ));
        attachments.retain(|a| !expired.contains(&a.id));
    }
    let capabilities = if attachments
        .iter()
        .any(|a| a.origin == "user_upload" && a.content_type.starts_with("image/"))
    {
        upstream_capabilities(state, egress, authority, credential, policy).await
    } else {
        Value::Null
    };
    let (image_parts, omitted) = crate::services::assistant_upload_service::image_plan(
        &attachments,
        &capabilities,
        &state.config.base_url,
    );
    if !omitted.is_empty() {
        let notice = crate::services::assistant_upload_service::IMAGE_FALLBACK;
        listing.push_str(&format!(
            "\n{notice} Unviewable attachment IDs: {}",
            json!(omitted)
        ));
    }
    decisions.push_str(&listing);
    // Persist the delivery outcome on metadata, so a reload does not hide the fallback.
    for item in attachments
        .iter()
        .filter(|a| a.origin == "user_upload" && a.content_type.starts_with("image/"))
    {
        let status = if omitted.contains(&item.id) {
            "unavailable"
        } else {
            "sent"
        };
        state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::assistant_message::COLLECTION_NAME,
            )
            .update_one(
                doc! {
                    "user_id": &row.user_id,
                    "conversation_id": &row.id,
                    "turn_id": turn_id,
                    "attachments.id": &item.id,
                },
                doc! {"$set": {"attachments.$.image_input": status}},
            )
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
        if let Some(group_id) = &row.group_id {
            let group_owner = if row.group_request_id.is_some() {
                row.agent_owner_id.as_deref().unwrap_or(&row.user_id)
            } else {
                &row.user_id
            };
            state
                .db
                .collection::<mongodb::bson::Document>(
                    crate::models::assistant_group::MESSAGES_COLLECTION_NAME,
                )
                .update_one(
                    doc! {"user_id": group_owner,"group_id": group_id,"attachments.id": &item.id},
                    doc! {"$set": {"attachments.$.image_input":status}},
                )
                .await
                .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
        }
    }
    // The notice prompts a transcript refresh; publish only after the fallback
    // metadata is durable, so that refresh cannot miss the explanation.
    if !omitted.is_empty() {
        events.emit(
            "turn.notice",
            json!({
                "code": "image_input_unavailable",
                "message": crate::services::assistant_upload_service::IMAGE_FALLBACK,
            }),
        );
    }
    let settings = crate::services::assistant_settings_service::get(&state.db, &row.user_id)
        .await
        .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
    let mut continuations = crate::services::assistant_continuation::Continuations::new(
        settings.max_auto_continuations,
    );
    let mut completed = String::new();
    let mut emitted = String::new();
    let mut input = if text.trim().is_empty() {
        "Please discuss the attachments in this message."
    } else {
        text
    };
    let mut recovery = engine::Recovery::default();
    let mut admission: Option<(String, tokio::time::Instant, DateTime<Utc>)> = None;
    let mut capacity_error: Option<TurnError> = None;
    loop {
        // History and upload preparation can outlive the initial admission
        // check. Recheck every bound source before model execution, including
        // continuations; stop/expiry alone preserve already admitted work.
        let turn = row.active_turn.as_ref().expect("claimed turn");
        let origins: Vec<_> = match turn.origin {
            crate::models::assistant_conversation::TurnOrigin::Channel => turn
                .asked_from
                .as_ref()
                .or(row.channel.as_ref())
                .into_iter()
                .collect(),
            crate::models::assistant_conversation::TurnOrigin::Event => {
                let queued: Vec<_> = turn
                    .events
                    .iter()
                    .flat_map(|e| &e.reply_to)
                    .filter(|o| o.thread.is_some())
                    .collect();
                if queued.is_empty() {
                    row.channel.iter().collect()
                } else {
                    queued
                }
            }
            _ => Vec::new(),
        };
        for origin in origins.into_iter().filter(|o| o.thread.is_some()) {
            Box::pin(
                crate::services::channel_thread_follow_service::validate_delivery(
                    &state.db,
                    &row.user_id,
                    origin,
                    &row.id,
                ),
            )
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
        }
        if !crate::services::assistant_steering::set_running_response(&state.db, row, None)
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?
        {
            return Err(TurnError::internal("cancelled", "stop"));
        }
        let request_key = if tool_images.fallback.is_some() {
            format!("{turn_id}:images:{}", tool_images.rounds)
        } else if continuations.count == 0 {
            turn_id.clone()
        } else {
            format!("{turn_id}:continuation:{}", continuations.count)
        };
        tool_images.attempted(state, row).await;
        let request_body = {
            let request_input = if tool_images.note.is_empty() {
                input.to_owned()
            } else {
                format!("{input}\n\n{}", tool_images.note)
            };
            let input = context.input_with_memory(
                if (continuations.count == 0 && tool_images.fallback.is_none()) || binding.is_none()
                {
                    // Recovery into a fresh session must restore the
                    // turn facts that its discarded session had seen.
                    &decisions
                } else {
                    // Documents have no image part. Keep their listing
                    // and availability notices visible on continuations
                    // too, once per request, without repeating events.
                    &listing
                },
                &request_input,
                continuations.count == 0 || binding.is_none(),
            );
            let mut body = engine::upstream_body(&row.model, &input, binding.as_deref(), &prompt);
            let parts = if !tool_images.parts.is_empty() {
                &tool_images.parts
            } else if continuations.count == 0 && tool_images.fallback.is_none() {
                &image_parts
            } else {
                &Vec::new()
            };
            if !parts.is_empty() {
                let mut content = vec![json!({"type":"input_text", "text":input})];
                content.extend(parts.clone());
                body["input"] = json!([{"role":"user", "content":content}]);
            }
            body
        };
        // Refusal retries share one request window. Explicit continuations
        // are new requests; credential/session recovery flags stay turn-scoped.
        if admission
            .as_ref()
            .is_none_or(|(key, _, _)| key != &request_key)
        {
            let now = Utc::now();
            let deadline = if admission.is_none() {
                run_deadline
            } else if turn.trigger_run_id.is_some() && !turn.trigger_continuation {
                Some(
                    now + chrono::Duration::from_std(
                        crate::services::assistant_engine_policy::AUTOMATION_CAPACITY_CAP,
                    )
                    .expect("bounded automation cap"),
                )
            } else {
                None
            };
            let budget = engine::capacity_budget(now, deadline, now);
            admission = Some((
                request_key.clone(),
                tokio::time::Instant::now() + budget,
                now + chrono::Duration::from_std(budget).expect("bounded admission budget"),
            ));
            capacity_error = None;
            recovery.retries = 0;
        }
        let (_, capacity_deadline, capacity_deadline_at) =
            admission.as_ref().expect("request window");
        let (capacity_deadline, capacity_deadline_at) = (*capacity_deadline, *capacity_deadline_at);
        if tokio::time::Instant::now() >= capacity_deadline
            && let Some(error) = capacity_error.take()
        {
            return Err(error);
        }
        let number = crate::services::assistant_turn_observability::attempt(&state.db, row)
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
        let mut attempt = EngineAttempt {
            row,
            number,
            http_status: None,
            upstream_code: None,
            wait_started,
            wait_ended: None,
            accrued_wait_ms,
            deadline: capacity_deadline,
            logged: false,
        };
        acknowledgement
            .start(state, row, capacity_deadline_at)
            .await?;
        if tokio::time::Instant::now() >= capacity_deadline
            && let Some(error) = capacity_error.take()
        {
            return Err(error);
        }
        let bounds = stream_bounds();
        let first_byte_deadline = tokio::time::Instant::now() + bounds.first_byte;
        let deadline = first_byte_deadline;
        let response = acknowledgement
            .wait(
                tokio::time::timeout_at(
                    deadline,
                    request(
                        state,
                        egress,
                        authority,
                        credential,
                        "POST",
                        "v1/responses",
                        Some(request_body),
                        Some(&request_key),
                        policy,
                    ),
                ),
                state,
                row,
                events,
                egress,
            )
            .await
            .map_err(|_| TurnError::internal("first_byte_timeout", "first_byte"))?
            .map_err(proxy_turn_error)?;
        attempt.http_status = Some(response.status().as_u16());
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let upstream_request_id = response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .and_then(sanitize_upstream_request_id);
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(engine::retry_after);
            let bytes = acknowledgement.wait(tokio::time::timeout_at(
                tokio::time::Instant::now() + crate::services::assistant_engine_policy::ADMISSION_ERROR_BODY_TIMEOUT,
                axum::body::to_bytes(response.into_body(), crate::services::assistant_engine_policy::ADMISSION_ERROR_BODY_MAX_BYTES),
            ), state, row, events, egress)
            .await
            .map_err(|_| TurnError::internal("first_byte_timeout", "first_byte"))?
            .map_err(|_| TurnError::internal("invalid_stream", "upstream_response"))?;
            let envelope: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let code = upstream_error_code(status, &envelope);
            attempt.upstream_code = TurnError::new(code).upstream_code;
            if tool_images.fallback.is_none()
                && matches!(code, "tool_budget_exhausted" | "turn_timeout")
            {
                let mut error = TurnError::new(code).at_stage("upstream_response");
                error.http_status = Some(status);
                match continue_turn(
                    state,
                    row,
                    events,
                    &mut continuations,
                    binding.as_deref(),
                    &error,
                    partial,
                )
                .await
                {
                    Ok(true) => {
                        input = crate::services::assistant_continuation::INSTRUCTION;
                        continue;
                    }
                    Ok(false) => {
                        return Ok(TurnResult {
                            text: partial.clone(),
                            session_id: binding,
                            response_id: None,
                            error: Some(error),
                        });
                    }
                    Err(error) => {
                        return Ok(TurnResult {
                            text: partial.clone(),
                            session_id: binding,
                            response_id: None,
                            error: Some(error),
                        });
                    }
                }
            }
            capacity_error = None;
            match recovery.decide(status, code, binding.is_some()) {
                RecoveryAction::Rebind => {
                    events.emit(
                        "turn.notice",
                        json!({"code": "retrying", "message": "Retrying…"}),
                    );
                    clear_session(state, row, "session_lost")
                        .await
                        .map_err(|_| TurnError::internal("assistant_unavailable", "recovery"))?;
                    binding = None;
                    context.fresh_session();
                    prompt = context
                        .session_instructions(true, &saved_recap, load_recap)
                        .await
                        .map_err(|_| TurnError::new("assistant_unavailable"))?;
                    events.notice();
                }
                RecoveryAction::ReplaceCredential => {
                    events.emit(
                        "turn.notice",
                        json!({"code": "retrying", "message": "Retrying…"}),
                    );
                    *credential = credentials::replace_for_turn(
                        &state.db,
                        &state.encryption_keys,
                        row,
                        &credential.api_key_id,
                    )
                    .await
                    .map_err(|_| TurnError::internal("agent_key_required", "recovery"))?;
                    binding = None;
                    context.fresh_session();
                    prompt = context
                        .session_instructions(true, &saved_recap, load_recap)
                        .await
                        .map_err(|_| TurnError::new("assistant_unavailable"))?;
                    events.notice();
                }
                RecoveryAction::Backoff => {
                    let mut error =
                        TurnError::new("capacity_exceeded").at_stage("upstream_response");
                    error.upstream_code = TurnError::new(code).upstream_code;
                    error.http_status = Some(status);
                    error.upstream_request_id = upstream_request_id.map(String::into_boxed_str);
                    capacity_error = Some(error);
                    let started = *wait_started.get_or_insert_with(Utc::now);
                    engine::waiting_for_capacity(&state.db, row, started, capacity_deadline_at)
                        .await
                        .map_err(|_| TurnError::internal("assistant_unavailable", "recovery"))?;
                    events.emit(
                        "turn.status",
                        json!({"conversation_id": row.id, "turn_id": turn_id,
                        "status": "waiting", "waiting_for_capacity": true,
                        "capacity_wait_note_origin": acknowledgement.emitted.then_some("nyxid_system"),
                        "engine_request_started_at": acknowledgement.started_at,
                        "capacity_wait_started_at": started, "capacity_wait_deadline": capacity_deadline_at}),
                    );
                    attempt.log("backoff");
                    let delay =
                        engine::capacity_delay(recovery.retries, retry_after, rand::random());
                    acknowledgement
                        .wait(
                            tokio::time::sleep_until(
                                (tokio::time::Instant::now() + delay).min(capacity_deadline),
                            ),
                            state,
                            row,
                            events,
                            egress,
                        )
                        .await;
                }
                RecoveryAction::Fail => {
                    let mut error = TurnError::new(if status == 401 || status == 403 {
                        "agent_key_required"
                    } else {
                        code
                    })
                    .at_stage("upstream_response");
                    // Keep the upstream's own (sanitized) code, never NyxID's mapping.
                    error.upstream_code = TurnError::new(code).upstream_code;
                    error.http_status = Some(status);
                    error.upstream_request_id = upstream_request_id.map(String::into_boxed_str);
                    return Err(error);
                }
            }
            continue;
        }
        attempt.wait_ended = Some(Utc::now());
        attempt.log("accepted");
        acknowledgement.accepted();
        crate::services::assistant_turn_observability::accepted(&state.db, row)
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "recovery"))?;
        events.emit("turn.status", json!({"conversation_id": row.id, "turn_id": turn_id, "status": "running", "waiting_for_capacity": false, "engine_accepted": true}));
        if let Some(started) = wait_started.take() {
            accrued_wait_ms = accrued_wait_ms.saturating_add(
                (attempt.wait_ended.expect("accepted") - started)
                    .num_milliseconds()
                    .max(0) as u64,
            );
        }
        let deadline = first_byte_deadline;
        if !response
            .headers()
            .get("content-type")
            .is_some_and(|v| v.to_str().is_ok_and(|v| v.starts_with("text/event-stream")))
        {
            let mut error = TurnError::internal("invalid_stream", "upstream_response");
            error.http_status = Some(response.status().as_u16());
            return Err(error);
        }
        let mut body = response.into_body().into_data_stream();
        let mut decoder = ResponseStream::default();
        // Any transport bytes, keepalive comments included, satisfy the
        // first-byte bound. After that a transport-idle bound and a separate
        // no-progress bound apply: keepalives keep the connection open but
        // only data events (text, lifecycle, tool progress) are progress.
        let mut received = false;
        let mut last_transport_activity = tokio::time::Instant::now();
        let mut last_semantic_progress = tokio::time::Instant::now();
        let mut running_response_id: Option<String> = None;
        let streamed: Result<(), TurnError> = async {
            loop {
                let timeout = if received {
                    stream_timeout(
                        bounds,
                        last_transport_activity.elapsed(),
                        last_semantic_progress.elapsed(),
                    )
                } else {
                    deadline.saturating_duration_since(tokio::time::Instant::now())
                };
                let next = tokio::time::timeout(timeout, body.next())
                    .await
                    .map_err(|_| {
                        if !received {
                            TurnError::internal("first_byte_timeout", "first_byte")
                        } else if last_semantic_progress.elapsed() >= bounds.no_progress {
                            TurnError::internal("idle_timeout", "no_progress")
                        } else {
                            TurnError::internal("idle_timeout", "idle")
                        }
                    })?;
                let Some(chunk) = next else {
                    break;
                };
                let chunk = chunk.map_err(|_| TurnError::internal("invalid_stream", "stream"))?;
                received = true;
                last_transport_activity = tokio::time::Instant::now();
                let before = decoder.data_events;
                let decoded = decoder.push(&chunk);
                if decoder.data_events != before {
                    last_semantic_progress = tokio::time::Instant::now();
                }
                *partial = completed.clone()
                    + &context.redact(
                        &decoder
                            .text
                            .replace(credential.raw_key.as_str(), "[redacted]"),
                    );
                if partial.len() > engine::MAX_OUTPUT_BYTES {
                    return Err(TurnError::internal("output_too_large", "stream"));
                }
                decoded?;
                if let Some(mut created) = decoder.created.take() {
                    created.credential_api_key_id = credential.api_key_id.clone();
                    running_response_id = Some(created.response_id.clone());
                    if !crate::services::assistant_steering::set_running_response(
                        &state.db,
                        row,
                        Some(&created),
                    )
                    .await
                    .map_err(|_| TurnError::internal("assistant_unavailable", "stream"))?
                    {
                        return Err(TurnError::internal("cancelled", "stop"));
                    }
                }
                if decoder.terminal.is_some() {
                    break;
                }
                let held = engine::secret_prefix_suffix(
                    partial,
                    &[
                        credential.raw_key.as_str(),
                        context.binding.marker.as_deref().unwrap_or(""),
                    ],
                );
                let safe_end = partial.len() - held.len();
                emit_text_suffix(events, block_id, &mut emitted, &partial[..safe_end]);
            }
            Ok(())
        }
        .await;
        if let Err(mut error) = streamed {
            if error.response_id.is_none() {
                error.response_id = running_response_id.map(String::into_boxed_str);
            }
            return Err(error);
        }
        crate::services::assistant_steering::set_running_response(&state.db, row, None)
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "stream"))?;
        let mut result = decoder.terminal.ok_or_else(|| {
            let mut error = TurnError::internal("invalid_stream", "stream");
            error.response_id = running_response_id.map(String::into_boxed_str);
            error
        })?;
        result.text = context.redact(
            &result
                .text
                .replace(credential.raw_key.as_str(), "[redacted]"),
        );
        binding = result.session_id.clone().or(binding);
        if tool_images.fallback.is_some() && result.error.is_some() {
            result.text = completed + &result.text;
            return Ok(result);
        }
        if result.error.is_none() {
            result.text = completed.clone() + &result.text;
            *partial = result.text.clone();
            result.session_id = binding.clone();
            tool_images.fallback = Some(result.clone());
            tool_images.sent(state, row).await;
            let prepared = if binding.is_some() {
                match tool_images
                    .prepare(state, egress, authority, row, credential, policy)
                    .await
                {
                    Ok(prepared) => prepared,
                    Err(error) if error.code == "cancelled" => return Err(error),
                    Err(_) => {
                        tracing::warn!("Optional tool image preparation could not complete");
                        false
                    }
                }
            } else {
                false
            };
            if prepared {
                images::renew(
                    state,
                    row,
                    binding.as_deref().expect("bound above"),
                    events,
                    tool_images.rounds,
                )
                .await?;
                emit_text_suffix(events, block_id, &mut emitted, partial);
                if !result.text.is_empty() {
                    events.emit("block.delta", json!({"block_id":block_id,"text":"\n\n"}));
                    emitted.push_str("\n\n");
                }
                completed = if result.text.is_empty() {
                    String::new()
                } else {
                    format!("{}\n\n", result.text)
                };
                input = "Continue after viewing the tool images.";
                continue;
            }
            tool_images.fallback = None;
            return Ok(result);
        }
        if let Some(error) = &result.error {
            match continue_turn(
                state,
                row,
                events,
                &mut continuations,
                binding.as_deref(),
                error,
                &result.text,
            )
            .await
            {
                Ok(true) => {
                    emit_text_suffix(events, block_id, &mut emitted, partial);
                    if !partial.is_empty() {
                        events.emit("block.delta", json!({"block_id":block_id,"text":"\n\n"}));
                        emitted.push_str("\n\n");
                    }
                    completed = if partial.is_empty() {
                        String::new()
                    } else {
                        format!("{partial}\n\n")
                    };
                    input = crate::services::assistant_continuation::INSTRUCTION;
                    continue;
                }
                Err(error) => result.error = Some(error),
                Ok(false) => {}
            }
        }
        result.text = completed + &result.text;
        result.session_id = binding;
        return Ok(result);
    }
}

type CapabilityCache = HashMap<String, (Instant, Value)>;
static CAPABILITIES: LazyLock<Mutex<CapabilityCache>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// One bounded metadata cache for image input and steering; never cache authority
/// or credentials. Include the destination so catalog changes cannot reuse support.
pub(crate) async fn upstream_capabilities(
    state: &AppState,
    egress: &dyn contract::NyxAgentEgress,
    authority: &contract::NyxAgentExecutionContext,
    credential: &AssistantCredential,
    policy: Option<BillingRoutePolicy>,
) -> Value {
    let Ok(service) =
        assistant_service::resolve_admin_service_by_slug(&state.db, engine::SERVICE_SLUG).await
    else {
        return Value::Null;
    };
    let key = format!("{}:{}:{}", state.db.name(), service.id, service.base_url);
    let mut cache = CAPABILITIES.lock().await;
    if let Some((at, value)) = cache.get(&key)
        && at.elapsed() < Duration::from_secs(60)
    {
        return value.clone();
    }
    let lookup = async {
        let response = Box::pin(request(
            state,
            egress,
            authority,
            credential,
            "GET",
            "v1/capabilities",
            None,
            None,
            policy,
        ))
        .await
        .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = axum::body::to_bytes(response.into_body(), 16384)
            .await
            .ok()?;
        serde_json::from_slice::<Value>(&bytes).ok()
    };
    let value = tokio::time::timeout(Duration::from_secs(5), lookup)
        .await
        .ok()
        .flatten()
        .unwrap_or(Value::Null);
    if cache.len() >= 64 {
        cache.clear();
    }
    cache.insert(key, (Instant::now(), value.clone()));
    value
}

#[path = "assistant_nyxagent_runtime_images.rs"]
pub(crate) mod images;
