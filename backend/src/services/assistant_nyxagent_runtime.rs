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

#[path = "assistant_nyxagent_subscriber.rs"]
mod subscriber;
pub(crate) use subscriber::{cancel_background, record as window_record};

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
                ..
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
                    && !active.execution_started
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
            .try_acquire_pool("assistant_channel", &user_id, team::CHANNEL_TURNS_PER_OWNER)
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
                checkpoint: None,
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
    pub(crate) checkpoint:
        Option<tokio::sync::watch::Sender<crate::models::assistant_conversation::StreamReply>>,
    pub(crate) sender: broadcast::Sender<Value>,
    pub(crate) cursor: u64,
    pub(crate) reset_notified: bool,
}
impl Events {
    pub(crate) fn emit(&mut self, event: &str, data: Value) {
        self.emit_with_checkpoint(event, data, None);
    }

    fn emit_with_checkpoint(
        &mut self,
        event: &str,
        mut data: Value,
        upstream: Option<crate::models::assistant_conversation::SubscriberCheckpoint>,
    ) {
        self.cursor += 1;
        data["event"] = json!(event);
        data["cursor"] = json!(self.cursor);
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint.send_modify(|reply| {
                super::assistant_stream_checkpoint::observe(reply, &data);
                if upstream.is_some() {
                    reply.upstream = upstream;
                }
            });
        }
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
    Relinquished,
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
                if row
                    .active_turn
                    .as_ref()
                    .is_some_and(|turn| turn.execution_attempt.is_some())
                {
                    return SettledExecution::Relinquished;
                }
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
        SettledExecution::Deleted | SettledExecution::Relinquished => {
            unreachable!("only the settling worker emits terminal events")
        }
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
    let current = if row.collaboration.is_some() {
        super::collaboration_group_service::runtime::refresh(&state.db, row).await
    } else {
        engine::get(&state.db, &row.user_id, &row.id).await
    }
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
    let mut fields = doc! {"nyxagent_session_id":binding,"active_turn.continuations":continuations.count,"active_turn.tool_progress": {"started": 0_i64, "calls": 0_i64, "digest": ""},"active_turn.lease_expires_at":mongodb::bson::DateTime::from_chrono(lease)};
    if active
        .restart_admission
        .as_ref()
        .is_some_and(|admission| admission.window.is_some())
    {
        fields.insert(
            "active_turn.restart_admission.window.next_continuation",
            mongodb::bson::to_bson(continuations)
                .map_err(|_| TurnError::internal("assistant_unavailable", "continuation"))?,
        );
    }
    let update = doc! {"$set":fields};
    if active.platform_origin.is_some() {
        super::platform_personal_turn_service::update_platform_turn(&state.db, row, update)
            .await
            .map_err(|_| TurnError::internal("cancelled", "continuation"))?;
    } else {
        let mut filter = engine::WorkerFence::capture(row)
            .map_err(|_| TurnError::internal("cancelled", "continuation"))?
            .filter();
        filter.insert("active_turn.stop_requested", false);
        let updated = state
            .db
            .collection::<AssistantConversation>(
                crate::models::assistant_conversation::COLLECTION_NAME,
            )
            .update_one(filter, update)
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "continuation"))?;
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

/// Question excerpts for discovery only, never execution evidence.
/// Lookup failures only omit this optional context.
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
        "\n\nQuestion excerpts from the owner's other private threads are for duplicate-work discovery only, never execution evidence. \
        Answer the user in this thread; never tell them that their question is being handled in another conversation. \
        Cross-chat delivery is handled separately by NyxID. Consult nyxid__task_status before reporting progress, and say unknown without evidence:",
    );
    for other in others {
        note.push_str(&format!(
            "\n- \"{}\": \"{}\"{}",
            excerpt(&other.title, 60).replace('"', "'"),
            excerpt(
                other
                    .question
                    .as_deref()
                    .unwrap_or("(question unavailable)"),
                200
            )
            .replace('"', "'"),
            if this.is_some() && other.question_key.as_deref() == this {
                " (the same question as this one)"
            } else {
                ""
            }
        ));
        note.push_str(&format!(
            " (thread_id {}, turn_id {})",
            other.thread_id, other.turn_id
        ));
    }
    note
}

/// Optional turn hints: drained events, legacy prose notes, and for NyxBot its
/// roster, direct user chats with specialists and pending permission requests.
/// NyxID-authored and bounded; lookup failures only omit optional hints.
/// Required channel provenance is rendered separately by `channel_turn_context`.
pub(crate) async fn turn_notes(
    state: &AppState,
    row: &AssistantConversation,
    agent: Option<&AssistantAgent>,
    previous_user_message: Option<DateTime<Utc>>,
) -> String {
    let mut notes = engine::channel_note(row);
    notes.push_str(engine::specialist_handoff_capability(row));
    notes.push_str(
        &engine::additional_channel_length_note(&state.db, row)
            .await
            .unwrap_or_default(),
    );
    if let Some(turn) = row.active_turn.as_ref() {
        if !row.guest_turn && turn.origin != TurnOrigin::Event && !turn.events.is_empty() {
            notes.push_str(
                "\n\nNyxID events since your previous turn (authored by NyxID; only a quoted \
                owner message is a request from the user; continuation lines are indented):",
            );
            for event in &turn.events {
                notes.push_str("\n- ");
                notes.push_str(&engine::event_notice_text(event));
            }
        }
        if let Some(note) = turn.note.as_deref() {
            notes.push_str("\n\n");
            notes.push_str(&note.replace(engine::HANDOFF_UI_NOTE, ""));
        }
    }
    // Someone other than the owner is talking: nothing private to the owner
    // (memory, other chats, the team, pending requests) goes into this turn.
    if row.guest_turn || row.collaboration.is_some() {
        return notes;
    }
    if let Some(turn) = row.active_turn.as_ref() {
        let refs = &turn.delegated_images.references;
        if !refs.is_empty() {
            notes.push_str("\n\nNyxID adopted these delegated tool images into this turn. They retain their source lifetime and may accompany this reply only at the assignment's original authorized chat. Do not invent foreign IDs or claim confirmed delivery:");
            for reference in refs
                .iter()
                .take(super::assistant_delegated_images::MAX_REFERENCES)
            {
                notes.push_str(&format!(
                    "\n- image {} (specialist {})",
                    reference.attachment.id, reference.assignment.source_agent
                ));
            }
        }
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
        .filter(|member| member.id != agent.id && member.available)
        .map(|member| {
            if members
                .iter()
                .filter(|other| other.name.eq_ignore_ascii_case(&member.name))
                .count()
                > 1
                || members
                    .iter()
                    .any(|other| other.id.eq_ignore_ascii_case(&member.name))
            {
                format!("@{} ({})", member.id, identifier(&member.name))
            } else {
                format!("@{}", identifier(&member.name))
            }
        })
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
        attempt: row
            .active_turn
            .as_ref()
            .and_then(|turn| turn.execution_attempt.clone()),
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
    mut body: Option<Value>,
    request_id: Option<&str>,
    destination: Option<contract::NyxAgentDestination>,
    policy: Option<BillingRoutePolicy>,
) -> AppResult<axum::response::Response> {
    if let Some(body) = body
        .as_mut()
        .filter(|b| b["model"].as_str().is_some_and(|m| m.starts_with("nyxid/")))
    {
        let contract::NyxAgentExecutionContext::Owner { user_id, turn, .. } = authority else {
            return Err(AppError::AssistantModelUnavailable);
        };
        let row = _state.db.collection::<AssistantConversation>("assistant_conversations")
            .find_one(doc! {"_id":&turn.conversation_id,"user_id":user_id,"credential_api_key_id":&credential.api_key_id,
                "active_turn.turn_id":&turn.turn_id,"active_turn.stop_requested":false}).await?
            .ok_or(AppError::AssistantTurnRequired)?;
        let handle = row
            .active_turn
            .as_ref()
            .and_then(|t| t.platform_model_context.as_deref())
            .ok_or(AppError::AssistantModelUnavailable)?;
        if row.guest_turn || body["model"] != row.model {
            return Err(AppError::AssistantModelUnavailable);
        }
        body["nyxid_model_context"] = json!({"version":1,"handle":handle});
    }
    let method = method
        .parse()
        .map_err(|_| AppError::Internal("Invalid assistant method".into()))?;
    if let contract::NyxAgentExecutionContext::Owner { user_id, turn, .. } = authority
        && super::channel_guest_access::is_hosted_conversation(&turn.conversation_id)
    {
        super::channel_guest_access::require_live_request(
            &_state.db,
            user_id,
            &turn.conversation_id,
            &turn.turn_id,
            &credential.api_key_id,
        )
        .await?;
    }
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
                destination,
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
    mut row: AssistantConversation,
    text: String,
    credential: AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    permit: DirectChatPermit,
    events: Events,
) -> Option<OwnerTurnSettlement> {
    if !super::assistant_turn_restart::owns_subscriber(
        &row,
        &super::assistant_engine_worker::instance().id,
    ) {
        return None;
    }
    if row
        .active_turn
        .as_ref()
        .is_some_and(|turn| turn.platform_origin.is_some() && turn.restart_offer.is_none())
    {
        Box::pin(prepare_manager_worker(
            &state,
            egress.as_ref(),
            &mut row,
            &credential,
            policy,
            &permit,
        ))
        .await
        .ok()?;
    }
    let handoff = row
        .active_turn
        .as_ref()
        .filter(|turn| turn.restart_admission.is_some())
        .map(|_| (state.shutdown.clone(), state.db.clone(), row.clone()));
    let mut driver = Box::pin(drive_owner_turn(
        state, egress, row, text, credential, policy, permit, events,
    ));
    let Some((shutdown, db, admitted)) = handoff else {
        return driver.await;
    };
    tokio::select! {
        biased;
        () = shutdown.turn_offer_due() => {},
        result = &mut driver => return result,
    }
    let worker = super::assistant_engine_worker::instance();
    let offer = super::assistant_turn_restart::yield_on_shutdown(&db, &admitted, &worker.id);
    match tokio::time::timeout(super::shutdown::drain::TURN_OFFER_MARGIN, offer).await {
        Ok(Ok(true)) => None,
        Ok(Ok(false)) => driver.await,
        _ => {
            tracing::warn!(conversation_id = %admitted.id, "Assistant shutdown offer unconfirmed");
            None
        }
    }
}

async fn prepare_manager_worker(
    state: &AppState,
    egress: &dyn contract::NyxAgentEgress,
    row: &mut AssistantConversation,
    credential: &AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    permit: &DirectChatPermit,
) -> AppResult<()> {
    use super::assistant_nyxagent_model_cache::{self as profiles, InferenceCredential};
    use crate::models::assistant_conversation::{ChatRestartSource, RestartSource, TurnOrigin};
    let active = row
        .active_turn
        .as_ref()
        .ok_or(AppError::AssistantTurnRequired)?;
    let origin = active
        .platform_origin
        .as_ref()
        .ok_or(AppError::AssistantTurnRequired)?;
    let eligible = super::security_readiness_service::chat_restart_enrollment_ready()
        && engine::restart_enabled(&state.db, &row.user_id)
        && !row.is_subagent()
        && row.platform_thread.as_ref() == Some(&origin.thread)
        && origin.thread.user_id == row.user_id
        && !row.guest_turn
        && !row.guest_model_setup
        && row.channel.is_none()
        && row.group_id.is_none()
        && !row.invocation_controlled
        && !row.platform_cleanup_pending
        && !row.shared_agent_thread
        && row.collaboration.is_none()
        && row.voice_parent_conversation_id.is_none()
        && active.invocation_id.is_none()
        && active.trigger_run_id.is_none()
        && active.automation_work.is_none()
        && active.voice_request_id.is_none()
        && active.channel_context.is_none()
        && active.channel_adapter_origin.is_none()
        && active.channel_event_id.is_none()
        && active.events.iter().all(|event| {
            event.invocation_id.is_none()
                && event.source_event_ids.is_empty()
                && event.channel_contexts.is_empty()
                && event.automation_work.is_none()
        })
        && matches!(active.origin, TurnOrigin::User | TurnOrigin::Event);
    if !eligible {
        super::platform_personal_turn_service::claim_platform_worker(
            &state.db,
            row,
            &credential.api_key_id,
        )
        .await?;
        row.active_turn
            .as_mut()
            .expect("claimed turn")
            .worker_instance = Some(super::assistant_engine_worker::instance().clone());
        return Ok(());
    }
    let original = row.clone();
    let origin = origin.clone();
    let authority = owner_context(row)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let deployment = tokio::time::timeout_at(deadline, async {
        let (destination, value) =
            fetch_capabilities(state, CapabilityLookup::Admission, |destination| {
                request(
                    state,
                    egress,
                    &authority,
                    credential,
                    "GET",
                    "v1/capabilities",
                    None,
                    None,
                    Some(destination),
                    policy,
                )
            })
            .await?;
        reattach_deployment(destination, &value)
    })
    .await
    .ok()
    .flatten();
    let mut fields = mongodb::bson::Document::new();
    let reason = if let Some(deployment) = deployment {
        let models = tokio::time::timeout_at(
            deadline,
            profiles::load(
                &state.db,
                &row.user_id,
                request(
                    state,
                    egress,
                    &authority,
                    credential,
                    "GET",
                    "v1/models?include_capabilities=true",
                    None,
                    None,
                    Some(contract::NyxAgentDestination(
                        deployment.destination_hash.clone(),
                    )),
                    policy,
                ),
            ),
        )
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        let metadata = engine::RestartMetadata {
            deployment: None,
            models,
        };
        match metadata.profile(&row.model) {
            Some(InferenceCredential::Thread) => {
                let message = state.db.collection::<mongodb::bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
                    .find_one(doc! {"user_id":&row.user_id,"conversation_id":&row.id,"turn_id":&active.turn_id,"seq":active.initiating_message_seq,"role":if active.origin == TurnOrigin::Event {"event"} else {"user"}})
                    .projection(doc! {"_id":1}).await?.ok_or(AppError::AssistantTurnRequired)?;
                row.active_turn
                    .as_mut()
                    .expect("claimed turn")
                    .worker_instance = Some(super::assistant_engine_worker::instance().clone());
                engine::install_restart(
                    row,
                    message
                        .get_str("_id")
                        .map_err(|_| AppError::AssistantTurnRequired)?,
                    deployment,
                    false,
                    permit.token().cloned(),
                    RestartSource::FirstPartyOwner,
                    None,
                    Some(ChatRestartSource::PersonalManager {
                        origin: Box::new(origin),
                    }),
                );
                let turn = row
                    .active_turn
                    .as_ref()
                    .ok_or(AppError::AssistantTurnRequired)?;
                let encode = |_| AppError::AssistantTurnRequired;
                fields.insert(
                    "active_turn.restart_admission",
                    mongodb::bson::to_bson(&turn.restart_admission).map_err(encode)?,
                );
                fields.insert(
                    "active_turn.execution_attempt",
                    mongodb::bson::to_bson(&turn.execution_attempt).map_err(encode)?,
                );
                "watched_thread_credential"
            }
            Some(InferenceCredential::Platform) => "platform_credential",
            None => "profile_unknown",
        }
    } else {
        "capability_unavailable"
    };
    fields.insert("active_turn.restart_eligibility", reason);
    super::platform_personal_turn_service::claim_worker_with_restart(
        &state.db,
        &original,
        &credential.api_key_id,
        &fields,
    )
    .await?;
    let active = row
        .active_turn
        .as_mut()
        .ok_or(AppError::AssistantTurnRequired)?;
    active.restart_eligibility = Some(reason.to_owned());
    active.worker_instance = Some(super::assistant_engine_worker::instance().clone());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn drive_owner_turn(
    state: AppState,
    egress: std::sync::Arc<dyn contract::NyxAgentEgress>,
    mut row: AssistantConversation,
    text: String,
    mut credential: AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    mut permit: DirectChatPermit,
    mut events: Events,
) -> Option<OwnerTurnSettlement> {
    let shutdown_work = state.shutdown.turn(&row);
    let trace = row
        .active_turn
        .as_ref()
        .and_then(|turn| crate::turn_trace::TurnContext::for_turn(&turn.turn_id));
    crate::turn_trace::scope(trace, Box::pin(async move {
    let _typing = egress.channel_typing(&row).map(AbortOnDropHandle::new);
    let turn_id = row
        .active_turn
        .as_ref()
        .expect("claimed turn")
        .turn_id
        .clone();
    let message_id = row.active_turn.as_ref().and_then(|turn| turn.restart_admission.as_ref())
        .map(|admission| admission.reply_id.clone()).unwrap_or_else(|| Uuid::new_v4().to_string());
    let block_id = format!("{message_id}-text");
    let worker_fence = engine::WorkerFence::capture(&row).ok()?;
    let (checkpoint, _checkpoint_task) = super::assistant_stream_checkpoint::start(
        state.db.clone(),
        worker_fence.filter(),
        message_id.clone(),
        block_id.clone(),
    );
    events.checkpoint = Some(checkpoint);
    if let Some(active) = row.active_turn.as_ref().filter(|turn|turn.restart_offer.is_some()) {
        if let Some(reply) = &active.reply && let Some(sender) = &events.checkpoint {
            events.cursor = events.cursor.max(reply.cursor.max(0) as u64);
            sender.send_replace(reply.clone());
        }
        if active.execution_attempt.as_ref().is_some_and(|attempt|attempt.epoch <= 1) {
            events.emit("turn.notice",json!({"code":"continued_after_restart","message":"Continued after a restart."}));
        }
    }
    tracing::info!(turn_id = %turn_id,
        worker_instance = %super::assistant_engine_worker::instance().id,
        worker_hostname = super::assistant_engine_worker::instance().hostname.as_deref(),
        "Assistant turn started");
    let (heartbeat_stop, mut heartbeat_stop_rx) = oneshot::channel();
    let heartbeat_db = state.db.clone();
    // AbortOnDropHandle ties the child to this worker: a panic or task abort
    // cannot leave a fresh heartbeat behind to hide an orphaned turn.
    let heartbeat_task = AbortOnDropHandle::new(crate::services::shutdown::process().spawn_worker(async move {
        let period = Duration::from_secs(engine::ACTIVE_TURN_HEARTBEAT_SECS);
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        loop {
            tokio::select! {
                _ = &mut heartbeat_stop_rx => break,
                _ = interval.tick() => {
                    match engine::heartbeat(&heartbeat_db, &worker_fence).await {
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
    let started_at = row.active_turn.as_ref().expect("claimed turn").started_at;
    let active = row.active_turn.as_ref().expect("claimed turn");
    let queue_deadline = permit.is_queued().then(|| {
        let since = active.queued_at.unwrap_or(started_at);
        let wait = chrono::Duration::seconds(engine::TURN_QUEUE_WAIT_SECS as i64);
        (since, active.queue_deadline.unwrap_or(since + wait))
    });
    if let Some((since, deadline)) = queue_deadline {
        // `waiting` with the capacity fields keeps older clients' waiting display.
        events.emit(
            "turn.status",
            json!({"conversation_id": row.id, "turn_id": turn_id, "status": "waiting",
            "started_at": started_at, "queued": true, "waiting_for_capacity": true,
            "capacity_wait_started_at": since, "capacity_wait_deadline": deadline}),
        );
    } else {
        events.emit(
            "turn.status",
            json!({"conversation_id": row.id, "turn_id": turn_id, "status": "running",
                "started_at": started_at}),
        );
    }
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
    let queued = if let Some(error) = expired_handoff(&row) {
        Err(error)
    } else if !permit.can_execute() {
        Err(TurnError::internal("assistant_unavailable", "permit"))
    } else if let Some((since, deadline)) = queue_deadline {
            let queued =
                tokio::select! {
                    biased;
                    () = state.shutdown.interrupted() => Err(TurnError::internal("turn_lost", "shutdown")),
                    result = wait_for_running_slot(&state, &mut row, &turn_id, &mut permit, since, deadline) => result,
                };
            if queued.is_ok() {
                events.emit(
                    "turn.status",
                    json!({"conversation_id": row.id, "turn_id": turn_id, "status": "running",
                    "waiting_for_capacity": false}),
                );
            }
            queued
    } else {
        Ok(())
    };
    let mut partial = row.active_turn.as_ref().filter(|turn|turn.restart_offer.is_some()).and_then(|turn|turn.reply.as_ref()).map(|reply|reply.text.clone()).unwrap_or_default();
    let mut instruction_context = None;
    let mut recovered_binding = None;
    let mut result = match queued {
        Err(error) => TurnResult {
            text: partial.clone(),
            session_id: None,
            response_id: None,
            error: Some(error),
        },
        Ok(()) => crate::services::assistant_engine_worker::contain(&row, async {
        // Execution includes upload planning and upstream streaming. Keep that
        // state off the caller's stack when this task is created by a tool.
        let mut execution = Box::pin(async {
            let authority = owner_context(&row)
                .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
            if let Some(active) = row.active_turn.as_ref().filter(|turn| turn.restart_offer.is_some())
                && let Some(admission) = active.restart_admission.as_deref()
                && let Some(window) = &admission.window {
                let outcome = Box::pin(subscriber::recover(&state, egress.as_ref(), &authority, &row, &credential, policy,
                    admission, window, &mut events, &block_id, &mut partial, &mut recovered_binding)).await;
                if let Some(fallback) = model_card_fallback(&state, &admission.reply_id, &row, &outcome, &mut partial).await? {
                    return Ok(fallback);
                }
                return outcome;
            }
            let setup_deadline = super::assistant_guest_model_setup::guest_deadline(&state.db, &row).await
                .map_err(|_|TurnError::internal("assistant_unavailable","prepare"))?;
            let setup_generation = if row.guest_turn && let Some(agent)=row.agent_id.as_deref() {
                super::assistant_guest_model_setup::record(&state.db,&row.user_id,agent,&row.model).await.ok().flatten()
                    .and_then(|r|r.get_str("claim").ok().map(str::to_owned))
            } else {None};
            if row.guest_turn && !row.shared_agent_thread && let Some(agent) = row.agent_id.as_deref()
                && (row.channel.is_some() || row.active_turn.as_ref().is_some_and(|t| t.channel_event_id.is_some()))
                && !super::assistant_guest_model_setup::valid_thread_route(&state,&row).await
                    .map_err(|_|TurnError::internal("assistant_unavailable","prepare"))? {
                let now = Utc::now();
                engine::mark_queued(&state.db,&row,now,now+chrono::Duration::seconds(60)).await
                    .map_err(|_|TurnError::internal("assistant_unavailable","prepare"))?;
                let ready = Box::pin(super::assistant_guest_model_setup::ensure(&state,&row.user_id,agent,&row.model,false,setup_deadline,super::org_channel_service::origin(&row))).await;
                engine::mark_dequeued(&state.db,&row).await
                    .map_err(|_|TurnError::internal("assistant_unavailable","prepare"))?;
                if !ready.unwrap_or(false) {return Err(super::assistant_guest_model_setup::failure(&state.db,&row).await);}
            }
            let outcome = execute_owner_turn(
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
                None,
            )
            .await;
            let failure=match &outcome {Err(error)=>Some(error),Ok(result)=>result.error.as_ref()};
            if row.guest_turn && !row.shared_agent_thread
                && let (Some(error), Some(agent)) = (failure, row.agent_id.as_deref())
            {
                let current = if row.collaboration.is_some() {
                    super::collaboration_group_service::runtime::refresh(&state.db, &row).await.ok()
                } else {
                    engine::get(&state.db, &row.user_id, &row.id).await.ok()
                };
                if let Some(turn) = current.as_ref().and_then(|r| r.active_turn.as_ref())
                    && turn.turn_id == turn_id
                    && super::assistant_model_observer::attribute(error, turn, Utc::now()).code == "guest_model_route_required"
                    && let Some(diagnostic) = turn.model_route_diagnostic.as_ref()
                    && diagnostic.attempt_id.is_some() && diagnostic.attempt_id == turn.model_attempt_id
                    && super::assistant_service_execution::model_diagnostic::Reason::needs_owner_setup(&diagnostic.reason)
                {
                    if diagnostic.reason == "route_mismatch" {
                        Box::pin(super::channel_inbound_diagnostics::org_guest_turn(&state.db, turn, "guest_model_route_mismatch")).await;
                    }
                    let generation = super::assistant_guest_model_setup::record(&state.db, &row.user_id, agent, &row.model)
                        .await.ok().flatten().and_then(|r| r.get_str("claim").ok().map(str::to_owned));
                    let ready = Box::pin(super::assistant_guest_model_setup::ensure(
                        &state, &row.user_id, agent, &row.model, generation == setup_generation,
                        setup_deadline, super::org_channel_service::origin(&row),
                    )).await.unwrap_or(false);
                    if !ready {
                        return Err(super::assistant_guest_model_setup::failure(&state.db, &row).await);
                    }
                    // A new request key can execute the input again. Require a terminal
                    // response and the live turn's complete, empty tool history.
                    if outcome.as_ref().is_ok_and(|result| result.text.is_empty()) && partial.is_empty()
                        && turn.timing.first_output_ms.is_none() && turn.activity_count == Some(0)
                        && !turn.business_system_success && turn.timing.tool_calls == 0
                    {
                        instruction_context = None;
                        let key = format!("{turn_id}:guest-route");
                        return execute_owner_turn(
                            &state, egress.as_ref(), &authority, &row, &text, &mut credential, policy,
                            &mut events, &block_id, &mut partial, &mut instruction_context, Some(&key),
                        ).await;
                    }
                }
            }
            outcome
        });
        tokio::select! {
            () = state.shutdown.interrupted() => Err(TurnError::internal("turn_lost", "shutdown")),
            result = &mut execution => result,
            () = permit.cancelled() => Err(TurnError::internal("assistant_unavailable", "permit")),
            error = watch_stop(&state, &row, &turn_id) => Err(error),
            () = tokio::time::sleep(turn_remaining(&row)) => {
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
    }),
    };
    if row.active_turn.as_ref().is_some_and(|turn|turn.restart_admission.is_some()) && (result.response_id.is_none() || executor_lost(&result)) {
        let current = state.db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME).find_one(engine::WorkerFence::capture(&row).ok()?.filter()).await.ok()?;
        let current = current?;
        let confirmation = cancel_background(&state,egress.as_ref(),&current,&credential,policy).await;
        if result.error.is_some() && !executor_lost(&result) && let Some(mut terminal) = confirmation.and_then(|confirmed| confirmed.terminal) {
            if let Some(prepared) = &instruction_context { terminal.text = prepared.redact(&terminal.text); }
            if terminal.error.is_none() || result.text.is_empty() { result = terminal; }
        }
    }
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
            .is_none_or(|turn| turn.platform_origin.is_none() && turn.invocation_id.is_none())
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
    result.text = super::assistant_citation_markers::sanitize(&result.text)
        .replace(credential.raw_key.as_str(), "[redacted]");
    // A voice task's model-authored reply cannot reconstruct private URLs.
    // Sanitize before checkpoint/settlement as well as at visible publication.
    if row.voice_parent_conversation_id.is_some() {
        result.text = super::voice::links::redact_task_reply(
            &result.text,
            &super::voice::links::frontend_origins(&state.config),
        );
    }
    #[cfg(test)]
    super::channel_guest_access::test_hooks::wait_before_settlement(&state.db, &row.id).await;
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
            engine::finish_turn_inner(
                &state.db,
                &row,
                &credential.api_key_id,
                &message_id,
                &result,
                instruction_context.as_ref().map(|context| &context.binding).or(recovered_binding.as_ref()),
                engine::SettlementCondition::Worker,
            )
        },
    )
    .await;
    if let (Some(work), SettledExecution::Saved { error, .. }) = (&shutdown_work, &settled) {
        work.settled(error.as_ref().is_some_and(|error| error.stage == "shutdown"));
    }
    let _ = heartbeat_stop.send(());
    let _ = heartbeat_task.await;
    match settled {
        SettledExecution::Saved { message_id, error } => Some(OwnerTurnSettlement {
            message_id,
            error,
            text: result.text,
        }),
        SettledExecution::Deleted | SettledExecution::Relinquished | SettledExecution::Unconfirmed => None,
    }

    })).await
}

/// Bound on the queue's own writes: a stalled write must not hold capacity.
const QUEUE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn turn_remaining(row: &AssistantConversation) -> Duration {
    let cap = stream_bounds().turn_cap;
    let Some(active) = row
        .active_turn
        .as_ref()
        .filter(|active| active.restart_admission.is_some())
    else {
        return cap;
    };
    cap.saturating_sub(
        (Utc::now() - active.started_at)
            .to_std()
            .unwrap_or_default()
            .saturating_sub(Duration::from_millis(active.timing.queue_wait_ms)),
    )
}

pub(crate) fn expired_handoff(row: &AssistantConversation) -> Option<TurnError> {
    let active = row
        .active_turn
        .as_ref()
        .filter(|active| active.restart_offer.is_some())?;
    if active
        .queue_deadline
        .is_some_and(|deadline| deadline <= Utc::now())
    {
        Some(TurnError::internal("queue_timeout", "queue"))
    } else if turn_remaining(row).is_zero() {
        Some(TurnError::internal("turn_timeout", "turn_deadline"))
    } else {
        None
    }
}

/// A turn admitted while its owner's running replies were at their limit
/// waits here, inside its claimed turn, so Stop, deletion and orphan
/// recovery already apply to it, until one of them finishes. On failure the
/// permit's capacity is given back at once, before the turn settles.
pub(crate) async fn wait_for_running_slot(
    state: &AppState,
    row: &mut AssistantConversation,
    turn_id: &str,
    permit: &mut DirectChatPermit,
    since: DateTime<Utc>,
    deadline: DateTime<Utc>,
) -> Result<(), TurnError> {
    let marked = engine::mark_queued(&state.db, row, since, deadline);
    if let Err(error) = wait_queued(state, row, turn_id, permit, deadline, marked).await {
        permit.release();
        return Err(error);
    }
    if row
        .active_turn
        .as_ref()
        .is_some_and(|turn| turn.restart_admission.is_some())
        && let Some(token) = permit.token()
    {
        let wait = engine::mark_dequeued_with_capacity(&state.db, row, Some(token))
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "queue"))?;
        let turn = row.active_turn.as_mut().expect("claimed turn");
        turn.timing.queue_wait_ms = wait;
        turn.queued_at = None;
        turn.queue_deadline = None;
        return Ok(());
    }
    // Display only, for the moments before the engine request starts, which
    // clears the markers durably; the lease set on queuing covers execution.
    let (db, row) = (state.db.clone(), row.clone());
    state.shutdown.spawn(async move {
        let _ = tokio::time::timeout(QUEUE_WRITE_TIMEOUT, engine::mark_dequeued(&db, &row)).await;
    });
    Ok(())
}

/// The wait itself. `marked` records the queued state and lease; claims start
/// only after it lands. Each claim runs on its own task, so Stop and the
/// deadline stay observable while it is outstanding; dropping it releases any
/// slot it later claims.
pub(crate) async fn wait_queued(
    state: &AppState,
    row: &AssistantConversation,
    turn_id: &str,
    permit: &mut DirectChatPermit,
    deadline: DateTime<Utc>,
    marked: impl std::future::Future<Output = AppResult<bool>>,
) -> Result<(), TurnError> {
    let mut marked = std::pin::pin!(tokio::time::timeout(QUEUE_WRITE_TIMEOUT, marked));
    let mut is_marked = false;
    let mut claim: Option<crate::mw::rate_limit::SlotClaim> = None;
    loop {
        let remaining = (deadline - Utc::now()).to_std().unwrap_or_default();
        tokio::select! {
            () = permit.cancelled() => {
                return Err(TurnError::internal("assistant_unavailable", "queue"));
            }
            error = watch_stop(state, row, turn_id) => return Err(error),
            () = tokio::time::sleep(remaining) => {
                return Err(TurnError::internal("queue_timeout", "queue"));
            }
            written = &mut marked, if !is_marked => match written {
                Ok(Ok(true)) => is_marked = true,
                Ok(Ok(false)) => return Err(TurnError::internal("cancelled", "stop")),
                Ok(Err(_)) | Err(_) => {
                    return Err(TurnError::internal("assistant_unavailable", "queue"));
                }
            },
            claimed = async { claim.as_mut().expect("outstanding claim").await }, if claim.is_some() => {
                claim = None;
                match claimed {
                    // A slot claimed after the deadline is released, not used.
                    Ok(Ok(Some(slot))) if Utc::now() < deadline => {
                        if permit.take_slot(slot) {
                            return Ok(());
                        }
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::debug!(%error, "Queued assistant turn could not check for a slot");
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Queued assistant turn slot claim failed");
                    }
                }
            }
            () = tokio::time::sleep(crate::mw::rate_limit::DIRECT_CHAT_QUEUE_POLL), if is_marked && claim.is_none() => {
                claim = permit.claim_slot();
            }
        }
    }
}

async fn watch_stop(state: &AppState, row: &AssistantConversation, turn_id: &str) -> TurnError {
    // Collaboration turns recheck live authority in a snapshot transaction;
    // do that every 5 s (20 polls) and only the stored stop flag in between.
    let mut polls: u32 = 0;
    loop {
        if super::channel_guest_access::is_hosted_conversation(&row.id) {
            let full = polls.is_multiple_of(4);
            polls = polls.wrapping_add(1);
            if full
                && super::channel_guest_access::live_turn(
                    &state.db,
                    row,
                    engine::live_turn(row, Utc::now()),
                )
                .await
                .is_err()
            {
                return TurnError::internal("cancelled", "stop");
            }
        }
        let stopped = if row.collaboration.is_some() {
            let full = polls.is_multiple_of(20);
            polls = polls.wrapping_add(1);
            if full {
                super::collaboration_group_service::runtime::stop_requested(&state.db, row, turn_id)
                    .await
            } else {
                super::collaboration_group_service::runtime::stop_flag_set(&state.db, row, turn_id)
                    .await
            }
        } else {
            let read = engine::get(&state.db, &row.user_id, &row.id);
            #[cfg(test)]
            let read = engine::STOP_POLL.scope((), read);
            match read.await {
                Ok(current) => {
                    if let Some(original) = row
                        .active_turn
                        .as_ref()
                        .filter(|original| original.execution_attempt.is_some())
                        && current.active_turn.as_ref().is_some_and(|active| {
                            active.turn_id == original.turn_id
                                && active.execution_attempt != original.execution_attempt
                        })
                    {
                        return TurnError::internal("turn_lost", "subscriber_relinquished");
                    }
                    let turn = engine::live_turn(&current, Utc::now());
                    if let Some(turn) = turn.filter(|t| t.turn_id == turn_id && !t.stop_requested)
                        && let Some(mut error) =
                            super::assistant_model_observer::approval_error(turn)
                    {
                        error.message = super::assistant_model_observer::approval_message(
                            error.code,
                            &super::assistant_links::AssistantPage::ApprovalSettings
                                .url(&state.config.frontend_url),
                        );
                        return error;
                    }
                    Ok(turn.is_none_or(|t| t.turn_id != turn_id || t.stop_requested))
                }
                Err(error) => Err(error),
            }
        };
        match stopped {
            Ok(false) => tokio::time::sleep(Duration::from_millis(250)).await,
            Ok(true) => return TurnError::internal("cancelled", "stop"),
            Err(_) => return TurnError::internal("assistant_unavailable", "stop"),
        }
    }
}

/// Typed proxy failures collapse to `assistant_unavailable`, except a billing
/// failure, which keeps its stable code and specific recovery guidance.
pub(crate) fn proxy_turn_error(error: AppError) -> TurnError {
    TurnError::internal(
        super::assistant_model_observer::refusal_code(&error).unwrap_or("assistant_unavailable"),
        "connect",
    )
}

/// Reads the NyxAgent `error.code` envelope, or NyxID's own flat envelope
/// (`{"error":"insufficient_credits",...}`) on a 402. Other flat categories are
/// not turn codes and stay unrecognized.
pub(crate) fn upstream_error_code(status: u16, envelope: &Value) -> &str {
    if let Some(code) = envelope["error"]["code"].as_str() {
        return code;
    }
    if status == 402
        && let Some(code) = envelope["error"]
            .as_str()
            .and_then(super::assistant_model_observer::billing_failure_code)
    {
        return code;
    }
    ""
}

fn post_refusal(status: u16, code: &str, request_id: Option<String>) -> TurnError {
    let category = if engine::admission_refusal(status, code) {
        "capacity_exceeded"
    } else if matches!(status, 401 | 403) && code != "model_unavailable" {
        "agent_key_required"
    } else {
        code
    };
    let mut error = TurnError::new(category).at_stage("upstream_response");
    error.upstream_code = TurnError::new(code).upstream_code;
    error.http_status = Some(status);
    error.upstream_request_id = request_id.map(String::into_boxed_str);
    error
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
        engine::clear_binding(&state.db, row, reason).await
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
    request_key_base: Option<&str>,
) -> Result<TurnResult, TurnError> {
    let contract::NyxAgentExecutionContext::Owner {
        user_id,
        turn,
        platform_origin,
        ..
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
    if let Some(text) =
        super::execution_permission_dispatch::proposals::resume_turn(state, egress, row)
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "permission_resume"))?
    {
        *partial = text.clone();
        return Ok(TurnResult {
            text,
            session_id: row.nyxagent_session_id.clone(),
            response_id: None,
            error: None,
        });
    }
    let attempt = super::assistant_service_execution::begin_model_attempt(&state.db, row)
        .await
        .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
    let mut tool_images = images::ToolImages::default();
    let outcome = Box::pin(execute_turn_inner(
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
        request_key_base,
    ))
    .await;
    if let Some(fallback) = model_card_fallback(state, &attempt, row, &outcome, partial).await? {
        return Ok(fallback);
    }
    // A current callback refusal can precede any reply. Cancellation in either
    // result shape wins, and an unrelated pending action cannot mask a failure.
    let fallback_allowed = match &outcome {
        Err(error) => Some(error),
        Ok(result) => result.error.as_ref(),
    }
    .is_some_and(allows_fallback);
    if outcome.as_ref().is_err() || outcome.as_ref().is_ok_and(|r| r.error.is_some()) {
        tool_images.failed(state, row).await;
    }
    if tool_images.rounds > 0
        && let Some(mut fallback) = tool_images.fallback.take()
        && fallback_allowed
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

fn allows_fallback(error: &TurnError) -> bool {
    error.code != "cancelled" && error.upstream_code.as_deref() != Some("executor_lost")
}

async fn model_card_fallback(
    state: &AppState,
    attempt: &str,
    row: &AssistantConversation,
    outcome: &Result<TurnResult, TurnError>,
    partial: &mut String,
) -> Result<Option<TurnResult>, TurnError> {
    let refusal = super::assistant_service_execution::finish_model_attempt(&state.db, attempt, row)
        .await
        .map_err(|_| TurnError::internal("assistant_unavailable", "recovery"))?;
    let failure = match outcome {
        Err(error) => Some(error),
        Ok(result) => result.error.as_ref(),
    };
    if failure.is_some_and(allows_fallback)
        && let Some(summary) = refusal
    {
        let text = format!(
            "Please review the confirmation card: {summary}. I’ll continue after you allow it."
        );
        partial.clone_from(&text);
        return Ok(Some(TurnResult {
            text,
            session_id: None,
            response_id: None,
            error: None,
        }));
    }
    Ok(None)
}

fn executor_lost(result: &TurnResult) -> bool {
    result
        .error
        .as_ref()
        .is_some_and(|error| error.upstream_code.as_deref() == Some("executor_lost"))
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
            automation_run_id = turn.automation_work.as_ref().map(|work| work.run_id.as_str()).or(turn.trigger_run_id.as_deref()),
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
    request_key_base: Option<&str>,
) -> Result<TurnResult, TurnError> {
    let platform_plan = if row.model.starts_with("nyxid/") {
        if row.guest_turn {
            return Err(TurnError::new("model_unavailable"));
        }
        super::assistant_platform_model::require_ready(&state.db)
            .await
            .map_err(proxy_turn_error)?;
        let capability = upstream_capabilities(
            state,
            egress,
            authority,
            credential,
            Some(super::assistant_platform_model::CAPABILITY_POLICY),
        )
        .await;
        if !super::assistant_platform_model::bridge_ready(&capability) {
            return Err(TurnError::new("model_unavailable"));
        }
        super::assistant_platform_model::activate_bridge(&state.db, row, &capability)
            .await
            .map_err(proxy_turn_error)?;
        Some(
            super::assistant_platform_model::selection_plan(state, &row.user_id, &row.model)
                .await
                .map_err(proxy_turn_error)?,
        )
    } else {
        None
    };
    let media_context = if row.active_turn.as_ref().is_some_and(|turn| {
        turn.channel_event_id.is_some() || turn.events.iter().any(|e| !e.media_event_ids.is_empty())
    }) {
        Box::pin(crate::services::assistant_channel_media::ingest_and_bind(
            state, row,
        ))
        .await
        .map_err(|error| {
            TurnError::internal(
                if matches!(error, AppError::AssistantTurnRequired) {
                    "cancelled"
                } else {
                    "assistant_unavailable"
                },
                "media",
            )
        })?
    } else {
        String::new()
    };
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
    let mut history = if row.collaboration.is_some() {
        super::collaboration_group_service::runtime::history(
            &state.db,
            row,
            20,
            Some(row.message_count + 1),
        )
        .await
    } else {
        engine::messages(
            &state.db,
            &row.user_id,
            &row.id,
            20,
            Some(row.message_count + 1),
        )
        .await
    }
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
        let audience_scope = super::channel_guest_access::is_hosted_conversation(&row.id);
        history.retain(|message| {
            message.origin == Some(TurnOrigin::Channel)
                && (message.role == "user" || (!audience_scope && message.role == "assistant"))
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
    // Optional turn hints: team state, direct chats and drained events.
    // Required authenticated channel provenance is prepared separately below.
    let agent =
        match crate::services::assistant_team_service::agent_for_conversation(&state.db, row).await
        {
            Ok(agent) => Some(agent),
            Err(_) if row.agent_owner_id.is_some() => {
                return Err(TurnError::internal("assistant_unavailable", "prepare"));
            }
            Err(_) => None,
        };
    let _checking_notice = agent
        .as_ref()
        .filter(|agent| agent.channel_checking_notice)
        .and_then(|_| egress.checking_notice(row))
        .map(AbortOnDropHandle::new);
    decisions.push_str(&turn_notes(state, row, agent.as_ref(), previous).await);
    // Authenticated channel provenance is required, including every retained
    // queued source. Never run with a partially rendered or omitted identity.
    decisions.push_str(
        &super::channel_turn_context::turn_notes(&state.db, row)
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?,
    );
    decisions.push_str(
        &crate::services::async_service_operation::input_context(state, row)
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?,
    );
    // Rollout-gated authoring rules are instructions, so they belong in the
    // stable (fingerprinted) part rather than the per-turn quoted context.
    let authoring = !row.guest_model_setup
        && !row.guest_turn
        && crate::services::assistant_skill_authoring::enabled(&state.db, &row.user_id)
            .await
            .unwrap_or(false);
    let guidance = if !authoring {
        ""
    } else if row.is_subagent() {
        crate::services::assistant_skill_authoring::SPECIALIST_GUIDANCE
    } else {
        crate::services::assistant_skill_authoring::GUIDANCE
    };
    let mut guidance = guidance.to_owned();
    if let Some(guest) = turn.channel_guest.as_ref() {
        guidance.push_str(super::channel_guest_access::audience_guidance(
            guest.audience,
        ));
    }
    // NYX template outlines (`assistant:skill-runs`, default off): owner turns only.
    let outlines = !row.guest_model_setup
        && !row.guest_turn
        && Box::pin(crate::services::skill_outline::enabled(
            &state.db,
            &row.user_id,
        ))
        .await
        .unwrap_or(false);
    guidance.push_str(&crate::services::skill_outline::instructions(
        outlines, authoring,
    ));
    if !row.guest_model_setup
        && !row.guest_turn
        && row.group_id.is_none()
        && row.collaboration.is_none()
        && row.agent_owner_id.is_none()
        && row.platform_thread.is_none()
        && !row.invocation_controlled
        && row.channel.is_none()
        && row.reply_channel.is_none()
        && row.group_request_id.is_none()
        && row.active_turn.as_ref().is_some_and(|t| {
            t.voice_request_id.is_none()
                && t.trigger_run_id.is_none()
                && t.invocation_id.is_none()
                && t.platform_origin.is_none()
                && t.channel_adapter_origin.is_none()
                && t.asked_from.is_none()
                && t.report_to.is_none()
                && !t.with_nyxbot
                && t.also_deliver.is_empty()
        })
        && crate::services::generated_artifacts::enabled(&state.db, &row.user_id)
            .await
            .unwrap_or(false)
    {
        guidance.push_str(crate::services::generated_artifacts::GUIDANCE);
    }

    if row
        .active_turn
        .as_ref()
        .is_some_and(|turn| turn.voice_request_id.is_some())
    {
        guidance.push_str(crate::services::voice::runtime::TASK_GUIDANCE);
    }
    if !row.guest_model_setup
        && !row.guest_turn
        && crate::services::assistant_service_execution::instruction::prompt_applies(
            &state.db,
            &row.user_id,
            agent.as_ref(),
        )
        .await
        .unwrap_or(false)
    {
        guidance
            .push_str(crate::services::assistant_service_execution::instruction::PROMPT_GUIDANCE);
    }
    let guest_skill_turn = row.guest_turn
        && row
            .active_turn
            .as_ref()
            .is_some_and(|turn| turn.channel_guest.is_some());
    if guest_skill_turn
        && !row.guest_model_setup
        && let Ok(Some(chat)) = acknowledgements::chat_for_conversation(&state.db, row).await
        && chat.api_key_id == credential.api_key_id
        && chat.turn_id.as_deref() == Some(turn.turn_id.as_str())
        && chat.channel_guest.as_ref() == turn.channel_guest.as_ref()
        && let Ok(agent) =
            super::channel_guest_access::skills::visible_agent(&state.db, &chat).await
    {
        guidance.push_str(&super::channel_guest_access::skills::instructions(&agent));
    }
    if !row.guest_model_setup
        && let Some(agent) = agent.as_ref()
        && crate::services::agent_skill_preload::enabled(agent, row.guest_turn && !guest_skill_turn)
    {
        let preloaded =
            tokio::time::timeout(Duration::from_secs(30), egress.preload_skills(row, agent)).await;
        guidance.push_str(&match preloaded {
            Ok(Ok(content)) => content,
            _ => "\nPreloaded skills are unavailable for this turn. Continue with available information or another authorized source; do not assume their contents.\n".into(),
        });
    }
    let prepared = crate::services::assistant_instruction_context::Prepared::with_guidance(
        state.audit_chain_hmac_key.as_ref().as_ref(),
        row,
        agent.as_ref(),
        &history,
        &guidance,
    )
    .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
    *instruction_context = Some(prepared);
    let context = instruction_context.as_mut().expect("prepared above");
    // A recap leads with the durable checkpoint; guest audiences never get it.
    if !row.guest_turn {
        context.set_checkpoint(
            engine::checkpoint_for_conversation(&state.db, row)
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
    if recap && row.context_reset_reason.as_deref() != Some("work_boundary") {
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
    // Preserve posted metadata for explicit withheld/expired notices, while
    // only live, authorized attachments proceed to the shared modality gate.
    let listed_attachments = attachments.clone();
    let mut availability_notice = String::new();
    if row.collaboration.is_some() {
        let (available, notice) = super::collaboration_group_service::uploads::input_for_turn(
            &state.db,
            row,
            attachments,
        )
        .await
        .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
        attachments = available;
        availability_notice = notice;
    } else if row.guest_turn {
        let (available, notice) =
            super::assistant_channel_media::guest::input_for_turn(state, row, attachments)
                .await
                .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
        attachments = available;
        availability_notice = notice;
    } else {
        let expired = Box::pin(crate::services::assistant_upload_retention::expired_ids(
            &state.db,
            &row.user_id,
            &attachments.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
        ))
        .await
        .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
        if !expired.is_empty() {
            availability_notice = format!(
                "\nAttachments expired per retention policy; ask the owner to upload them again: {}",
                serde_json::json!(expired)
            );
            attachments.retain(|a| !expired.contains(&a.id));
        }
    }
    let has_images = attachments.iter().any(|a| {
        a.origin == "user_upload"
            && (a.content_type.starts_with("image/")
                || (a.content_type == "application/pdf"
                    && a.pdf
                        .as_ref()
                        .is_none_or(|pdf| !pdf.unread_pages.is_empty())))
    });
    let model_text_only = has_images
        && match platform_plan.as_ref() {
            Some(plan) => plan.image_limits().is_none(),
            None => {
                images::model_support(state, egress, authority, &row.model, policy).await
                    == Some(false)
            }
        };
    let mut listing = if row.guest_turn && row.collaboration.is_none() {
        // Never suggest attachment or machine tools to a channel guest.
        String::new()
    } else {
        crate::services::assistant_upload_service::listing(&listed_attachments, model_text_only)
    };
    listing.push_str(&availability_notice);
    let capabilities = if has_images && !model_text_only {
        upstream_capabilities(state, egress, authority, credential, policy).await
    } else {
        Value::Null
    };
    let capabilities = match platform_plan.as_ref() {
        Some(plan) => super::assistant_platform_model::image_capabilities(
            capabilities,
            plan.image_limits().as_ref(),
        ),
        None => capabilities,
    };
    let image_notice = if model_text_only {
        crate::services::assistant_upload_service::MODEL_IMAGE_FALLBACK
    } else if row.guest_turn && row.collaboration.is_none() {
        "The image attached to this guest message cannot be viewed in this turn. Do not claim to have read it."
    } else {
        crate::services::assistant_upload_service::IMAGE_FALLBACK
    };
    let mut pdf_input = Box::pin(
        crate::services::assistant_upload_service::pdf_vision::prepare(
            state,
            row,
            &attachments,
            &capabilities,
            model_text_only,
        ),
    )
    .await
    .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
    let mut image_attachments = attachments.clone();
    image_attachments.extend(pdf_input.images.clone());
    let (mut image_parts, omitted) = crate::services::assistant_upload_service::image_plan(
        &image_attachments,
        &capabilities,
        &state.config.base_url,
    );
    pdf_input.finalize(&omitted, "image_limit");
    let mut pdf_delivery = pdf_input.delivery();
    images::record_pdf_outcomes(state, row, &pdf_delivery.reports).await?;
    if !omitted.is_empty() {
        let notice = image_notice;
        listing.push_str(&format!(
            "\n{notice} Unviewable attachment IDs: {}",
            json!(omitted)
        ));
    }
    if !media_context.is_empty() {
        decisions.push('\n');
        decisions.push_str(&media_context);
    }
    let media_decision_offset = decisions.len();
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
        let reason = (status == "unavailable").then_some(if model_text_only {
            "model_text_only"
        } else if row.guest_turn && row.collaboration.is_none() {
            "guest_media_unavailable"
        } else {
            "agent_unsupported"
        });
        images::record_upload_outcome(state, row, &item.id, status, reason).await?;
    }
    // The notice prompts a transcript refresh; publish only after the fallback
    // metadata is durable, so that refresh cannot miss the explanation.
    if !omitted.is_empty() {
        events.emit(
            "turn.notice",
            json!({
                "code": "image_input_unavailable",
                "message": image_notice,
            }),
        );
    }
    let settings = crate::services::assistant_settings_service::get(&state.db, &row.user_id)
        .await
        .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
    let mut continuations =
        crate::services::assistant_continuation::Continuations::new(if row.guest_model_setup {
            0
        } else {
            settings.max_auto_continuations
        });
    let mut completed = String::new();
    let mut emitted = String::new();
    let mut first_output_recorded = false;
    let mut input = text;
    let mut recovery = engine::Recovery::default();
    let start_retry_deadline = tokio::time::Instant::now() + super::assistant_start_retry::BUDGET;
    let mut start_retries = 0;
    let mut admission: Option<(String, tokio::time::Instant, DateTime<Utc>)> = None;
    let mut capacity_error: Option<TurnError> = None;
    let channel_source = turn.channel_event_id.as_deref().filter(|_| {
        turn.origin == crate::models::assistant_conversation::TurnOrigin::Channel
            && turn.trigger_run_id.is_none()
            && turn.restart_admission.as_deref().is_none_or(|admission| matches!(admission.chat_source.as_deref(), Some(crate::models::assistant_conversation::ChatRestartSource::PrivateChannel { .. })))
    });
    let mut channel_retry_body: Option<Value> = None;
    let mut guest_input_ids = Vec::new();
    let mut foreground = None;
    loop {
        let row = foreground.as_ref().unwrap_or(row);
        // History and upload preparation can outlive the initial admission
        // check. Recheck every bound source before model execution, including
        // continuations; stop/expiry alone preserve already admitted work.
        let turn = row.active_turn.as_ref().expect("claimed turn");
        if channel_retry_body.is_none()
            && row.guest_turn
            && row.collaboration.is_none()
            && !listed_attachments.is_empty()
        {
            // Capabilities, admission waits and continuations can outlive the
            // initial check. Rebuild only this guest message's input from live
            // authority before every request; the image GET rechecks again.
            let (current, notice) = super::assistant_channel_media::guest::input_for_turn(
                state,
                row,
                listed_attachments.clone(),
            )
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
            if channel_source.is_some() {
                guest_input_ids = current
                    .iter()
                    .map(|attachment| attachment.id.clone())
                    .collect();
            }
            let (parts, unavailable) = crate::services::assistant_upload_service::image_plan(
                &current,
                &capabilities,
                &state.config.base_url,
            );
            image_parts = parts;
            listing = notice;
            if !unavailable.is_empty() {
                listing.push_str(image_notice);
            }
            for item in listed_attachments
                .iter()
                .filter(|a| a.content_type.starts_with("image/"))
            {
                if !current.iter().any(|allowed| allowed.id == item.id) {
                    images::record_upload_outcome(
                        state,
                        row,
                        &item.id,
                        "unavailable",
                        Some("guest_media_unavailable"),
                    )
                    .await?;
                }
            }
            decisions.truncate(media_decision_offset);
            decisions.push_str(&listing);
        }
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
            let conversation = crate::services::org_channel_service::follow_conversation(
                &state.db, row, origin, None,
            )
            .await
            .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
            Box::pin(
                crate::services::channel_thread_follow_service::validate_delivery(
                    &state.db,
                    &row.user_id,
                    origin,
                    &conversation,
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
        let base = request_key_base.unwrap_or(turn_id);
        let request_key = if tool_images.fallback.is_some() {
            format!("{base}:images:{}", tool_images.rounds)
        } else if continuations.count == 0 {
            base.to_owned()
        } else {
            format!("{base}:continuation:{}", continuations.count)
        };
        tool_images.attempted(state, row).await;
        let first_input = continuations.count == 0 && tool_images.fallback.is_none();
        let parts: &[Value] = if !tool_images.parts.is_empty() {
            &tool_images.parts
        } else if first_input {
            &image_parts
        } else {
            &[]
        };
        // Select page coverage with the exact image URLs in this request.
        // Channel capacity retries reuse the sealed request body below.
        let mut content = vec![json!({"type":"input_text","text":""})];
        content.extend(pdf_input.with_page_labels(parts));
        pdf_delivery.requested = pdf_input.requested(parts);
        let mut request_body = if let Some(original) = &channel_retry_body {
            original.clone()
        } else {
            let request_input = if tool_images.note.is_empty() {
                input.to_owned()
            } else {
                format!("{input}\n\n{}", tool_images.note)
            };
            let notes = if first_input || binding.is_none() {
                &decisions
            } else {
                &listing
            };
            let input = context.input_with_memory(
                &format!("{notes}{}", pdf_delivery.notice()),
                &request_input,
                continuations.count == 0 || binding.is_none(),
            );
            let mut body = engine::upstream_body(&row.model, &input, binding.as_deref(), &prompt);
            if row.guest_model_setup {
                body["input"] = json!(super::assistant_guest_model_setup::INSTRUCTION);
                body["instructions"] = json!(super::assistant_guest_model_setup::INSTRUCTION);
            }
            if !parts.is_empty() {
                content[0]["text"] = json!(input);
                body["input"] = json!([{"role":"user","content":content}]);
            }
            body
        };
        if let Some(parts) = request_body["input"][0]["content"].as_array() {
            pdf_delivery.requested = pdf_input.requested(parts);
        } else {
            pdf_delivery.requested.clear();
        }
        let request_user_images: Vec<_> = attachments
            .iter()
            .filter(|item| {
                item.origin == "user_upload"
                    && item.content_type.starts_with("image/")
                    && crate::services::assistant_upload_service::pdf_vision::includes_image(
                        &request_body,
                        &item.id,
                    )
            })
            .map(|item| item.id.clone())
            .collect();
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
        if let Some(source) = channel_source.filter(|_| channel_retry_body.is_some()) {
            let expiry = egress
                .channel_retry_expiry(row, source)
                .await
                .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
            let (_, deadline, at) = admission.as_mut().expect("channel request window");
            *at = (*at).min(expiry);
            *deadline = (*deadline).min(
                tokio::time::Instant::now() + (expiry - Utc::now()).to_std().unwrap_or_default(),
            );
        }
        let (_, capacity_deadline, capacity_deadline_at) =
            admission.as_ref().expect("request window");
        let (capacity_deadline, capacity_deadline_at) = (*capacity_deadline, *capacity_deadline_at);
        if tokio::time::Instant::now() >= capacity_deadline
            && let Some(error) = capacity_error.take()
        {
            return Err(error);
        }
        if super::channel_guest_access::is_hosted_conversation(&row.id) {
            super::channel_guest_access::require_live_request(
                &state.db,
                &row.user_id,
                &row.id,
                turn_id,
                &credential.api_key_id,
            )
            .await
            .map_err(|_| TurnError::internal("cancelled", "stop"))?;
        }
        if channel_retry_body.is_some() && !guest_input_ids.is_empty() {
            super::assistant_channel_media::guest::recheck_retry_inputs(
                &state.db,
                row,
                &guest_input_ids,
            )
            .await
            .map_err(|_| TurnError::internal("channel_retry_attachment_unavailable", "prepare"))?;
        }
        let saved_input = if turn.restart_admission.is_some() {
            request_body["background"] = json!(true);
            Some(
                subscriber::seal(
                    state,
                    row,
                    &request_key,
                    &request_body,
                    &completed,
                    &context.binding,
                    Some(subscriber::FollowUp::capture(
                        &continuations,
                        tool_images,
                        context,
                        &listing,
                        request_user_images.clone(),
                        pdf_delivery.clone(),
                    )),
                )
                .await?,
            )
        } else {
            None
        };
        let number = crate::services::assistant_turn_observability::attempt(
            &state.db,
            row,
            &request_key,
            capacity_deadline_at,
            saved_input,
            channel_retry_body.as_ref().map(|_| &binding),
        )
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
        let deadline = if start_retries > 0 {
            first_byte_deadline.min(start_retry_deadline)
        } else {
            first_byte_deadline
        };
        let mut first_byte = super::assistant_start_retry::FirstByteDeadline::new(
            deadline.saturating_duration_since(tokio::time::Instant::now()),
            bounds.no_progress,
        );
        let first_channel_body =
            (number == 1 && channel_source.is_some()).then(|| request_body.clone());
        let dispatched = tokio::time::Instant::now();
        let dispatched_at = Utc::now();
        let response = acknowledgement
            .wait(
                first_byte.wait(
                    &state.db,
                    row,
                    request(
                        state,
                        egress,
                        authority,
                        credential,
                        "POST",
                        "v1/responses",
                        Some(request_body),
                        Some(&request_key),
                        turn.restart_admission
                            .as_deref()
                            .and_then(|admission| admission.reattach.as_ref())
                            .map(|deployment| {
                                contract::NyxAgentDestination(deployment.destination_hash.clone())
                            }),
                        policy,
                    ),
                ),
                state,
                row,
                events,
                egress,
            )
            .await?;
        let response = match response {
            Ok(response) => response,
            Err(AppError::PoolAttemptTransport(
                crate::errors::PoolAttemptTransportKind::Connect,
            )) if channel_retry_body.is_none()
                && partial.is_empty()
                && super::assistant_start_retry::untouched(&state.db, row)
                    .await
                    .unwrap_or(false) =>
            {
                let error = TurnError::internal("assistant_start_unavailable", "connect");
                let Some(delay) = super::assistant_start_retry::delay(start_retries)
                    .filter(|delay| tokio::time::Instant::now() + *delay < start_retry_deadline)
                else {
                    return Err(error);
                };
                start_retries += 1;
                acknowledgement
                    .wait(tokio::time::sleep(delay), state, row, events, egress)
                    .await;
                continue;
            }
            Err(error) => return Err(proxy_turn_error(error)),
        };
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
            let exact_capacity = status == 503 && code == "capacity_exceeded";
            if code == "image_input_unsupported" {
                images::settle_pdf_request(
                    state,
                    row,
                    &mut pdf_delivery,
                    Some("image_input_unsupported"),
                    false,
                )
                .await?;
            }
            if channel_retry_body.is_some() && !engine::admission_refusal(status, code) {
                return Err(post_refusal(status, code, upstream_request_id));
            }
            let (capacity_deadline, capacity_deadline_at) = if exact_capacity
                && let (Some(original), Some(source)) = (first_channel_body, channel_source)
            {
                let expiry = egress
                    .channel_retry_expiry(row, source)
                    .await
                    .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?;
                let budget = (expiry - dispatched_at)
                    .to_std()
                    .unwrap_or_default()
                    .min(super::assistant_engine_policy::CHANNEL_CAPACITY_BUDGET);
                let until = dispatched + budget;
                let until_at = dispatched_at
                    + chrono::Duration::from_std(budget).expect("bounded capacity wait");
                admission = Some((request_key.clone(), until, until_at));
                channel_retry_body = Some(original);
                acknowledgement.enabled = false;
                attempt.deadline = until;
                (until, until_at)
            } else {
                (capacity_deadline, capacity_deadline_at)
            };
            if turn.restart_admission.is_some()
                && number == 1
                && status == 400
                && matches!(code, "background_disabled" | "idempotency_key_required")
            {
                let filter = engine::WorkerFence::matching(
                    row,
                    doc! {
                        "active_turn.restart_admission.window.key": &request_key,
                        "active_turn.restart_admission.credential_ended": {"$ne":true},
                        "active_turn.stop_requested":false,
                    },
                )
                .map_err(|_| TurnError::new("cancelled"))?;
                let mut fields =
                    doc! {"active_turn.restart_eligibility":"upstream_refused_background"};
                if let Some(source) = super::channel_supplemental_delivery::managed_source(turn) {
                    fields.insert(
                        "active_turn.execution_attempt.foreground_delivery",
                        mongodb::bson::to_bson(source)
                            .map_err(|_| TurnError::new("outcome_unknown"))?,
                    );
                }
                let saved = state
                    .db
                    .collection::<AssistantConversation>(
                        crate::models::assistant_conversation::COLLECTION_NAME,
                    )
                    .find_one_and_update(
                        filter,
                        doc! {"$unset":{"active_turn.restart_admission":""},
                        "$set":fields},
                    )
                    .return_document(mongodb::options::ReturnDocument::After)
                    .write_concern(mongodb::options::WriteConcern::majority())
                    .await
                    .map_err(|_| TurnError::new("outcome_unknown"))?;
                drop(attempt);
                foreground = Some(saved.ok_or_else(|| TurnError::new("cancelled"))?);
                continue;
            }
            if super::assistant_start_retry::refused(status, code)
                && partial.is_empty()
                && super::assistant_start_retry::untouched(&state.db, row)
                    .await
                    .unwrap_or(false)
            {
                let mut error =
                    TurnError::internal("assistant_start_unavailable", "upstream_response");
                error.http_status = Some(status);
                error.upstream_code = Some(code.into());
                let Some(delay) = super::assistant_start_retry::delay(start_retries)
                    .filter(|delay| tokio::time::Instant::now() + *delay < start_retry_deadline)
                else {
                    return Err(error);
                };
                start_retries += 1;
                attempt.log("start_backoff");
                acknowledgement
                    .wait(tokio::time::sleep(delay), state, row, events, egress)
                    .await;
                continue;
            }
            if tool_images.fallback.is_none()
                && matches!(code, "tool_budget_exhausted" | "turn_timeout")
            {
                let error = post_refusal(status, code, upstream_request_id);
                if turn.restart_admission.is_some() {
                    subscriber::save_refusal(
                        state,
                        row,
                        &request_key,
                        &error,
                        None,
                        capacity_deadline_at,
                        channel_retry_body.is_some(),
                    )
                    .await?;
                }
                if turn.restart_admission.is_some()
                    && binding.is_some()
                    && continuations.count < continuations.maximum
                {
                    subscriber::resolve_budget_window(
                        state,
                        egress,
                        authority,
                        row,
                        credential,
                        policy,
                        &request_key,
                    )
                    .await?;
                }
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
                        start_retries = 0;
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
                        credential,
                    )
                    .await
                    .map_err(|_| TurnError::internal("agent_key_required", "recovery"))?;
                    if !pdf_delivery.requested.is_empty()
                        && engine::get(&state.db, &row.user_id, &row.id)
                            .await
                            .is_ok_and(|current| {
                                current.credential_api_key_id != credential.api_key_id
                            })
                    {
                        // A successor bound only at settlement cannot fetch this
                        // turn's page URLs. Keep the exact text fallback.
                        image_parts.retain(|part| {
                            !pdf_delivery.requested.iter().any(|(id, _, _)| {
                                crate::services::assistant_upload_service::pdf_vision::is_image_part(part, id)
                            })
                        });
                        images::settle_pdf_request(
                            state,
                            row,
                            &mut pdf_delivery,
                            Some("credential_replaced"),
                            false,
                        )
                        .await?;
                    }
                    binding = None;
                    context.fresh_session();
                    prompt = context
                        .session_instructions(true, &saved_recap, load_recap)
                        .await
                        .map_err(|_| TurnError::new("assistant_unavailable"))?;
                    events.notice();
                }
                RecoveryAction::Backoff => {
                    let error = post_refusal(status, code, upstream_request_id);
                    let delay =
                        engine::capacity_delay(recovery.retries, retry_after, rand::random());
                    if turn.restart_admission.is_some() {
                        subscriber::save_refusal(
                            state,
                            row,
                            &request_key,
                            &error,
                            Some(delay),
                            capacity_deadline_at,
                            channel_retry_body.is_some(),
                        )
                        .await?;
                    }
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
                    let error = post_refusal(status, code, upstream_request_id);
                    if turn.restart_admission.is_some() {
                        subscriber::save_refusal(
                            state,
                            row,
                            &request_key,
                            &error,
                            None,
                            capacity_deadline_at,
                            channel_retry_body.is_some(),
                        )
                        .await?;
                    }
                    return Err(error);
                }
            }
            continue;
        }
        // The short deadline belongs only to startup refusal retries. Accepted
        // work and its later continuations retain the normal response budget.
        start_retries = 0;
        channel_retry_body = None;
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
        if !response
            .headers()
            .get("content-type")
            .is_some_and(|v| v.to_str().is_ok_and(|v| v.starts_with("text/event-stream")))
        {
            let mut error = TurnError::internal("invalid_stream", "upstream_response");
            error.http_status = Some(response.status().as_u16());
            return Err(error);
        }
        let background = turn.restart_admission.is_some();
        let mut body = response.into_body().into_data_stream();
        let mut decoder = ResponseStream::default();
        decoder.replay = background;
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
                let next = if received {
                    let timeout = stream_timeout(
                        bounds,
                        last_transport_activity.elapsed(),
                        last_semantic_progress.elapsed(),
                    );
                    tokio::time::timeout(timeout, body.next())
                        .await
                        .map_err(|_| {
                            if last_semantic_progress.elapsed() >= bounds.no_progress {
                                TurnError::internal("idle_timeout", "no_progress")
                            } else {
                                TurnError::internal("idle_timeout", "idle")
                            }
                        })?
                } else {
                    first_byte.wait(&state.db, row, body.next()).await?
                };
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
                let raw = decoder.text();
                *partial = completed.clone()
                    + &context.redact(&raw.replace(credential.raw_key.as_str(), "[redacted]"));
                if partial.len() > engine::MAX_OUTPUT_BYTES {
                    return Err(TurnError::internal("output_too_large", "stream"));
                }
                decoded?;
                if !first_output_recorded && !raw.is_empty() {
                    first_output_recorded = true;
                    let _ = super::assistant_turn_observability::first_output(&state.db, row).await;
                }
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
                let visible = &partial[..safe_end];
                let cursor = running_response_id.as_deref().zip(decoder.sequence);
                if visible != emitted
                    && (!background || cursor.is_some())
                    && (!background || held.is_empty() || !visible.starts_with(&emitted))
                {
                    let upstream = cursor.filter(|_| background).map(|(id, sequence)| {
                        subscriber::checkpoint(
                            &request_key, id, sequence, visible.strip_prefix(&completed).unwrap_or(""),
                            &decoder.output, &raw,
                        )
                    });
                    if let Some(suffix) = visible.strip_prefix(&emitted) {
                        events.emit_with_checkpoint(
                            "block.delta", json!({"block_id":block_id,"text":suffix}), upstream,
                        );
                    } else {
                        events.emit_with_checkpoint(
                            "block.completed",
                            json!({"block_id":block_id,"block":{"type":"text","block_id":block_id,"text":visible}}),
                            upstream,
                        );
                    }
                    emitted = visible.to_owned();
                }
                if decoder.reader_error.is_some() {
                    break;
                }
            }
            Ok(())
        }
        .await;
        if background && (streamed.is_err() || decoder.terminal.is_none()) {
            return subscriber::reconnect(
                state, egress, authority, row, credential, policy, events, block_id, partial,
            )
            .await;
        }
        if let Err(mut error) = streamed {
            if error.response_id.is_none() {
                error.response_id = running_response_id.map(String::into_boxed_str);
            }
            return Err(error);
        }
        let cleared = if background && decoder.terminal.as_ref().is_some_and(executor_lost) {
            Ok(true)
        } else if background {
            super::assistant_turn_restart::confirm_window(&state.db, row, &request_key).await
        } else {
            super::assistant_steering::set_running_response(&state.db, row, None).await
        };
        if !cleared.map_err(|_| TurnError::internal("assistant_unavailable", "stream"))? {
            return Err(TurnError::new("cancelled"));
        }
        let mut result = decoder.terminal.ok_or_else(|| {
            let mut error = TurnError::internal("invalid_stream", "stream");
            error.response_id = running_response_id.map(String::into_boxed_str);
            error
        })?;
        let rejected = decoder.image_input_unsupported
            || result.error.as_ref().is_some_and(images::image_rejected);
        let rejected_pdf_ids = if rejected {
            pdf_delivery.requested.clone()
        } else {
            Vec::new()
        };
        images::settle_pdf_request(
            state,
            row,
            &mut pdf_delivery,
            rejected.then_some("image_input_unsupported"),
            result.error.is_none(),
        )
        .await?;
        if rejected {
            // Only the current request can reject its image parts.
            if !request_user_images.is_empty() {
                for id in &request_user_images {
                    if images::record_upload_outcome(
                        state,
                        row,
                        id,
                        "unavailable",
                        Some("image_input_unsupported"),
                    )
                    .await
                    .is_err()
                    {
                        tracing::warn!("Image delivery notice could not be saved");
                    }
                }
            }
            image_parts.retain(|part| {
                !request_user_images
                    .iter()
                    .map(String::as_str)
                    .chain(rejected_pdf_ids.iter().map(|(id, _, _)| id.as_str()))
                    .any(|id| {
                        crate::services::assistant_upload_service::pdf_vision::is_image_part(
                            part, id,
                        )
                    })
            });
            tool_images.unsupported(state, row).await;
            events.emit("turn.notice", json!({"code":"image_input_unavailable","message":crate::services::assistant_upload_service::MODEL_IMAGE_FALLBACK}));
        }
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
                input = "Continue answering the user.";
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

type CapabilityKey = (String, String, String);
type CapabilityCache = HashMap<CapabilityKey, (Instant, Value)>;
static CAPABILITIES: LazyLock<Mutex<CapabilityCache>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const CAPABILITY_TTL: Duration = Duration::from_secs(60);

pub(crate) enum CapabilityLookup {
    Admission,
    CurrentCatalog,
}

fn unique_cached_destination(
    cache: &CapabilityCache,
    db: &mongodb::Database,
) -> Option<(contract::NyxAgentDestination, Value)> {
    let mut fresh = cache.iter().filter(|((database, _, _), (seen, _))| {
        database == db.name() && seen.elapsed() < CAPABILITY_TTL
    });
    let ((_, service, url), (_, value)) = fresh.next()?;
    if fresh.next().is_some() {
        return None;
    }
    Some((
        contract::NyxAgentDestination::for_service(service, url),
        value.clone(),
    ))
}

pub(crate) fn reattach_deployment(
    destination: contract::NyxAgentDestination,
    value: &Value,
) -> Option<crate::models::assistant_conversation::ReattachDeployment> {
    use crate::models::assistant_conversation::{ReattachDeployment, ReattachLimits};
    let limits: ReattachLimits = serde_json::from_value(value.get("reattach")?.clone()).ok()?;
    (limits.protocol == "nyxagent-reattach-v1"
        && limits.credential_watch.as_ref().is_some_and(|watch| {
            (1..=30).contains(&watch.interval_seconds)
                && (1..=15).contains(&watch.model_request_max_age_seconds)
        })
        && limits.recent_events > 0
        && limits.max_events >= limits.recent_events
        && limits.max_event_bytes > 0
        && limits.retention_seconds > 0
        && limits.absence_retention_seconds > 0
        && limits.max_subscribers_per_response > 0
        && limits.max_subscribers_per_owner >= limits.max_subscribers_per_response)
        .then_some(ReattachDeployment {
            destination_hash: destination.0,
            limits,
        })
}

/// Cache successful deployment metadata for image input and steering, never
/// authority or credentials. A turn's failed authentication must not disable
/// capabilities for other owners. Catalog destination changes invalidate support.
pub(crate) async fn upstream_capabilities(
    state: &AppState,
    egress: &dyn contract::NyxAgentEgress,
    authority: &contract::NyxAgentExecutionContext,
    credential: &AssistantCredential,
    policy: Option<BillingRoutePolicy>,
) -> Value {
    fetch_capabilities(
        state,
        CapabilityLookup::CurrentCatalog,
        |destination| async move {
            Box::pin(request(
                state,
                egress,
                authority,
                credential,
                "GET",
                "v1/capabilities",
                None,
                None,
                Some(destination),
                policy,
            ))
            .await
        },
    )
    .await
    .map_or(Value::Null, |(_, value)| value)
}

pub(crate) async fn fetch_capabilities<F: Future<Output = AppResult<axum::response::Response>>>(
    state: &AppState,
    lookup: CapabilityLookup,
    fetch: impl FnOnce(contract::NyxAgentDestination) -> F,
) -> Option<(contract::NyxAgentDestination, Value)> {
    if matches!(lookup, CapabilityLookup::Admission)
        && let Some(cached) = unique_cached_destination(&*CAPABILITIES.lock().await, &state.db)
    {
        return Some(cached);
    }
    let service =
        match assistant_service::resolve_admin_service_by_slug(&state.db, engine::SERVICE_SLUG)
            .await
        {
            Ok(service) => service,
            Err(error) => {
                tracing::warn!(
                    failure_class = "service_resolution",
                    http_status = error.status_code().as_u16(),
                    "NyxAgent capability lookup failed"
                );
                return None;
            }
        };
    let key = (
        state.db.name().to_owned(),
        service.id.clone(),
        service.base_url.clone(),
    );
    let destination = contract::NyxAgentDestination::for_service(&service.id, &service.base_url);
    if let Some((at, value)) = CAPABILITIES.lock().await.get(&key)
        && at.elapsed() < CAPABILITY_TTL
    {
        return Some((destination, value.clone()));
    }
    // Concurrent misses may fetch in parallel. Do not hold the cache lock over
    // network/body reads, or one slow credential would block every image turn.
    let lookup = async {
        let response = fetch(destination.clone())
            .await
            .map_err(|error| ("request", Some(error.status_code().as_u16())))?;
        let status = response.status();
        if !status.is_success() {
            return Err(("http_status", Some(status.as_u16())));
        }
        let bytes = axum::body::to_bytes(response.into_body(), 16384)
            .await
            .map_err(|_| ("body", Some(status.as_u16())))?;
        let value = serde_json::from_slice::<Value>(&bytes)
            .map_err(|_| ("invalid_json", Some(status.as_u16())))?;
        if !value.is_object() {
            return Err(("invalid_response", Some(status.as_u16())));
        }
        Ok(value)
    };
    let value = match tokio::time::timeout(Duration::from_secs(5), lookup)
        .await
        .unwrap_or(Err(("timeout", None)))
    {
        Ok(value) => value,
        Err((failure_class, http_status)) => {
            tracing::warn!(
                service_id = %service.id,
                failure_class,
                http_status,
                "NyxAgent capability lookup failed"
            );
            return None;
        }
    };
    let mut cache = CAPABILITIES.lock().await;
    if cache.len() >= 64 {
        cache.clear();
    }
    cache.insert(key, (Instant::now(), value.clone()));
    Some((destination, value))
}

#[path = "assistant_nyxagent_runtime_images.rs"]
pub(crate) mod images;

#[cfg(test)]
#[path = "assistant_nyxagent_runtime_capability_tests.rs"]
mod capability_tests;
