//! Human-only HTTP adapter. Detached execution owns the upstream stream and permit;
//! a browser is only a subscriber. Stop is a durable, owner-scoped request.
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{Request, StatusCode},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    convert::Infallible,
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{assistant_conversation::AssistantConversation, assistant_message::AssistantMessage},
    mw::{auth::AuthUser, rate_limit::DirectChatPermit},
    services::{
        assistant_acknowledgement_service as acknowledgements,
        assistant_agent_credential_service::{self as credentials, AssistantCredential},
        assistant_nyxagent::{
            self as engine, RecoveryAction, ResponseStream, TurnError, TurnResult,
        },
        assistant_service,
        billing::route_inventory::BillingRoutePolicy,
    },
};

#[derive(Serialize)]
pub struct MachineReceiptResponse {
    pub operation_id: String,
    pub node_id: String,
    pub machine_name: Option<String>,
    pub agent_id: String,
    pub context_mode: Option<String>,
    pub action: String,
    pub status: String,
    pub job_id: Option<String>,
    pub exit_code: Option<i64>,
    pub bytes: Option<u64>,
    pub duration_ms: Option<u64>,
    pub error_code: Option<u32>,
    pub screenshot_id: Option<String>,
    pub preview_id: Option<String>,
    pub preview_enabled: bool,
}
impl From<crate::models::machine_receipt::MachineReceipt> for MachineReceiptResponse {
    fn from(row: crate::models::machine_receipt::MachineReceipt) -> Self {
        Self {
            operation_id: row.operation_id,
            node_id: row.node_id,
            machine_name: None,
            agent_id: row.agent_id,
            context_mode: row.context_mode,
            action: row.action,
            status: row.status,
            job_id: row.job_id,
            exit_code: row.exit_code,
            bytes: row.bytes,
            duration_ms: row.duration_ms,
            error_code: row.error_code,
            screenshot_id: row.screenshot_id,
            preview_id: row.preview_id,
            preview_enabled: row.preview_enabled,
        }
    }
}

pub(crate) async fn resolve_machine_names(
    db: &mongodb::Database,
    viewer: &str,
    receipts: Vec<&mut MachineReceiptResponse>,
) -> AppResult<()> {
    let names = Box::pin(crate::services::machine_activity_service::machine_names(
        db,
        viewer,
        receipts.iter().map(|r| r.node_id.clone()).collect(),
    ))
    .await?;
    for receipt in receipts {
        receipt.machine_name = names.get(&receipt.node_id).cloned();
    }
    Ok(())
}

#[derive(Serialize)]
pub struct ActivityResponse {
    pub(crate) machine: Option<MachineReceiptResponse>,
    id: String,
    label: String,
    status: String,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
}
impl From<crate::models::assistant_conversation::TurnActivity> for ActivityResponse {
    fn from(row: crate::models::assistant_conversation::TurnActivity) -> Self {
        Self {
            machine: row.machine.map(|m| (*m).into()),
            id: row.id,
            label: row.label,
            status: row.status,
            started_at: row.started_at,
            ended_at: row.ended_at,
        }
    }
}
#[derive(Serialize)]
pub struct AttachmentResponse {
    expired: bool,
    image_input: Option<String>,
    origin: String,
    pages: Option<usize>,
    id: String,
    content_type: String,
    size: i64,
    label: String,
}
impl From<crate::models::assistant_conversation::TurnAttachment> for AttachmentResponse {
    fn from(row: crate::models::assistant_conversation::TurnAttachment) -> Self {
        Self {
            expired: false,
            image_input: row.image_input,
            origin: row.origin,
            pages: row.pages,
            id: row.id,
            content_type: row.content_type,
            size: row.size,
            label: row.label,
        }
    }
}
/// The page already authorized these attachments through its conversation/group.
pub(crate) async fn mark_expired_attachments(
    db: &mongodb::Database,
    user: &str,
    attachments: Vec<&mut AttachmentResponse>,
) -> AppResult<()> {
    let ids = attachments.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
    let expired = Box::pin(crate::services::assistant_upload_retention::expired_ids(
        db, user, &ids,
    ))
    .await?;
    for attachment in attachments {
        attachment.expired = expired.contains(&attachment.id);
    }
    Ok(())
}

#[derive(Serialize)]
pub struct ActiveTurnResponse {
    continuations: u32,
    turn_id: String,
    started_at: DateTime<Utc>,
    activities: Vec<ActivityResponse>,
    attachments: Vec<AttachmentResponse>,
}
#[derive(Serialize)]
pub struct AgentRefResponse {
    pub id: String,
    pub kind: crate::models::assistant_agent::AgentKind,
    pub name: String,
    pub display_name: Option<String>,
    /// Destroyed agents' threads are read-only.
    pub destroyed: bool,
}
#[derive(Serialize)]
pub struct ChannelOriginResponse {
    parent_chat_id: Option<String>,
    thread_id: Option<String>,
    parent_title: Option<String>,
    platform: String,
    /// The channel bot connection (`/nyxagent/channels/{id}`).
    channel_agent_id: String,
    bot_label: Option<String>,
    /// The chat this thread answers (`/nyxagent/channels/{id}/chats`), its
    /// kind (`private`, `group`, `channel`) and name, when known.
    chat_id: Option<String>,
    chat_kind: Option<String>,
    chat_title: Option<String>,
}
#[derive(Serialize)]
pub struct ConversationResponse {
    id: String,
    title: String,
    title_source: crate::models::assistant_conversation::TitleSource,
    model: String,
    /// Always `full`: the Ask/Full choice was retired. Kept for older clients.
    access_mode: crate::models::assistant_conversation::AccessMode,
    created_at: DateTime<Utc>,
    last_message_at: DateTime<Utc>,
    message_count: i64,
    pending_acknowledgements: u32,
    active_turn: Option<ActiveTurnResponse>,
    context_reset_at: Option<DateTime<Utc>>,
    role: crate::models::assistant_conversation::AgentRole,
    /// The agent this thread belongs to; `None` only when it could not be
    /// resolved (legacy rows resolve to the owner's NyxBot).
    agent: Option<AgentRefResponse>,
    /// Wake-up events waiting for this thread's next turn.
    pending_events: usize,
    channel: Option<ChannelOriginResponse>,
}
impl ConversationResponse {
    /// Attach the owning agent (legacy rows belong to NyxBot).
    pub(crate) fn with_agent(
        mut self,
        agent_id: Option<&str>,
        agents: &[crate::models::assistant_agent::AssistantAgent],
    ) -> Self {
        let agent = match agent_id {
            Some(id) => agents.iter().find(|agent| agent.id == id),
            None => agents.iter().find(|agent| agent.is_nyxbot()),
        };
        self.agent = agent.map(|agent| AgentRefResponse {
            id: agent.id.clone(),
            kind: agent.kind,
            name: agent.name.clone(),
            display_name: agent.display_name.clone(),
            destroyed: agent.destroyed_at.is_some(),
        });
        self
    }
    /// Fill in a channel thread's bot and chat.
    pub(crate) fn with_chat(
        mut self,
        details: &std::collections::HashMap<String, super::nyxbot::chats::ChatDetails>,
    ) -> Self {
        if let (Some(channel), Some(detail)) = (self.channel.as_mut(), details.get(&self.id)) {
            channel.bot_label = Some(detail.bot_label.clone());
            channel.chat_id = detail.chat_id.clone();
            channel.chat_kind = detail.kind.clone();
            channel.chat_title = detail.title.clone();
            channel.parent_chat_id = detail.parent_chat_id.clone();
            channel.thread_id = detail.thread_id.clone();
            channel.parent_title = detail.parent_title.clone();
        }
        self
    }
}
impl From<AssistantConversation> for ConversationResponse {
    fn from(row: AssistantConversation) -> Self {
        let active_turn = engine::live_turn(&row, Utc::now()).map(|turn| ActiveTurnResponse {
            continuations: turn.continuations,
            turn_id: turn.turn_id.clone(),
            started_at: turn.started_at,
            activities: turn
                .activities
                .iter()
                .cloned()
                .map(ActivityResponse::from)
                .collect(),
            attachments: turn
                .attachments
                .iter()
                .cloned()
                .map(AttachmentResponse::from)
                .collect(),
        });
        Self {
            id: row.id,
            title: row.title,
            title_source: row.title_source,
            model: row.model,
            access_mode: crate::models::assistant_conversation::AccessMode::Full,
            created_at: row.created_at,
            last_message_at: row.updated_at,
            message_count: row.message_count,
            pending_acknowledgements: 0,
            active_turn,
            context_reset_at: row.context_reset_at,
            role: row.role,
            agent: None,
            pending_events: row.pending_events.len(),
            channel: row.channel.map(|channel| ChannelOriginResponse {
                parent_chat_id: None,
                thread_id: None,
                parent_title: None,
                platform: channel.platform,
                channel_agent_id: channel.nyxbot_channel_id,
                bot_label: None,
                chat_id: None,
                chat_kind: None,
                chat_title: None,
            }),
        }
    }
}
#[derive(Serialize)]
struct VoiceTranscriptResponse {
    session_id: String,
    segment_id: String,
    start_ms: i64,
    end_ms: i64,
    sealed: bool,
    complete: bool,
    delivery: String,
    request_id: Option<String>,
    backend_message_id: Option<String>,
}
impl From<crate::models::assistant_message::VoiceTranscript> for VoiceTranscriptResponse {
    fn from(v: crate::models::assistant_message::VoiceTranscript) -> Self {
        Self {
            session_id: v.session_id,
            segment_id: v.segment_id,
            start_ms: v.start_ms,
            end_ms: v.end_ms,
            sealed: v.sealed,
            complete: v.complete,
            delivery: v.delivery,
            request_id: v.request_id,
            backend_message_id: v.backend_message_id,
        }
    }
}
#[derive(Serialize)]
pub struct MessageResponse {
    voice: Option<VoiceTranscriptResponse>,
    execution_pending: bool,
    id: String,
    seq: i64,
    turn_id: String,
    role: String,
    text: String,
    status: String,
    error_code: Option<String>,
    created_at: DateTime<Utc>,
    activities: Vec<ActivityResponse>,
    attachments: Vec<AttachmentResponse>,
    /// A user message written in a chat app: its platform.
    via: Option<String>,
}
impl From<AssistantMessage> for MessageResponse {
    fn from(row: AssistantMessage) -> Self {
        Self {
            voice: row.voice.map(VoiceTranscriptResponse::from),
            execution_pending: row.execution_pending,
            id: row.id,
            seq: row.seq,
            turn_id: row.turn_id,
            role: row.role,
            text: row.text,
            status: row.status,
            error_code: row.error_code,
            created_at: row.created_at,
            activities: row
                .activities
                .into_iter()
                .map(ActivityResponse::from)
                .collect(),
            attachments: row
                .attachments
                .into_iter()
                .map(AttachmentResponse::from)
                .collect(),
            via: row.via,
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageQuery {
    limit: Option<i64>,
    cursor: Option<String>,
    /// Only this agent's threads.
    agent_id: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    limit: Option<i64>,
    before_seq: Option<i64>,
}
fn limit(value: Option<i64>) -> AppResult<i64> {
    let value = value.unwrap_or(50);
    if !(1..=100).contains(&value) {
        return Err(AppError::BadRequest(
            "limit must be between 1 and 100".into(),
        ));
    }
    Ok(value)
}
#[derive(Serialize)]
pub struct IndexResponse {
    conversations: Vec<ConversationResponse>,
    next_cursor: Option<String>,
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<PageQuery>,
) -> AppResult<Json<IndexResponse>> {
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    let limit = limit(query.limit)?;
    crate::services::assistant_team_service::ensure_nyxbot(&state.db, &user_id).await?;
    let agents = crate::services::assistant_team_service::agents(&state.db, &user_id, true).await?;
    let agent = match query.agent_id.as_deref() {
        Some(id) => Some(
            agents
                .iter()
                .find(|agent| agent.id == id)
                .ok_or_else(|| AppError::NotFound("Agent not found".into()))?,
        ),
        None => None,
    };
    let mut rows = engine::list(
        &state.db,
        &user_id,
        limit + 1,
        query.cursor.as_deref(),
        agent,
    )
    .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = more.then(|| engine::index_cursor(rows.last().expect("nonempty page")));
    let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    let counts = acknowledgements::pending_counts(&state.db, &user_id, &ids).await?;
    let chats =
        super::nyxbot::chats::thread_details(&state, &user_id, &rows.iter().collect::<Vec<_>>())
            .await?;
    let mut conversations: Vec<_> = rows
        .into_iter()
        .map(|row| {
            let count = counts.get(&row.id).copied().unwrap_or(0);
            let agent_id = row.agent_id.clone();
            let mut dto = ConversationResponse::from(row)
                .with_agent(agent_id.as_deref(), &agents)
                .with_chat(&chats);
            dto.pending_acknowledgements = count;
            dto
        })
        .collect();
    let receipts = conversations
        .iter_mut()
        .filter_map(|c| c.active_turn.as_mut())
        .flat_map(|t| t.activities.iter_mut())
        .filter_map(|a| a.machine.as_mut())
        .collect();
    Box::pin(resolve_machine_names(&state.db, &user_id, receipts)).await?;
    Ok(Json(IndexResponse {
        conversations,
        next_cursor,
    }))
}

#[derive(Serialize)]
pub struct HistoryResponse {
    conversation: ConversationResponse,
    messages: Vec<MessageResponse>,
    acknowledgements: Vec<AcknowledgementResponse>,
    /// Pending proxy approvals raised by this chat's key; decided through
    /// `POST /approvals/requests/{id}/decide`.
    approvals: Vec<ChatApprovalResponse>,
    /// Things outside the chat this thread is waiting for (a bot being
    /// created, a service being connected, the owner verifying a chat app).
    /// NyxID resumes the thread by itself when each happens.
    waiting: Vec<super::nyxbot::WaitingItem>,
    before_seq: Option<i64>,
}
#[derive(Serialize)]
pub struct ChatApprovalResponse {
    id: String,
    service_slug: String,
    service_name: String,
    summary: String,
    approval_mode: crate::models::service_approval_config::ApprovalMode,
    agent_key_prefix: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}
impl From<engine::ChatApproval> for ChatApprovalResponse {
    fn from(row: engine::ChatApproval) -> Self {
        Self {
            id: row.id,
            service_slug: row.service_slug,
            service_name: row.service_name,
            summary: row.summary,
            approval_mode: row.approval_mode,
            agent_key_prefix: row.agent_key_prefix,
            created_at: row.created_at,
            expires_at: row.expires_at,
        }
    }
}
pub async fn history(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> AppResult<Json<HistoryResponse>> {
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    let limit = limit(query.limit)?;
    if query.before_seq.is_some_and(|seq| seq <= 0) {
        return Err(AppError::BadRequest("Invalid before_seq".into()));
    }
    let (mut conversation, mut rows) =
        engine::history_page(&state.db, &user_id, &id, limit + 1, query.before_seq).await?;
    let more = rows.len() > limit as usize;
    if more {
        rows.remove(0);
    }
    let before_seq = more.then(|| rows[0].seq);
    let acknowledgements = acknowledgements::history(&state.db, &user_id, &id).await?;
    let approvals =
        engine::pending_approvals(&state.db, &user_id, &conversation.credential_api_key_id).await?;
    // Best effort: the transcript never fails because of the waiting list.
    let waiting = super::nyxbot::waiting(&state, &user_id, &conversation.id)
        .await
        .unwrap_or_else(|error| {
            tracing::debug!(%error, "NyxBot waiting list unavailable");
            Vec::new()
        });
    let mut receipts: Vec<_> = rows
        .iter_mut()
        .flat_map(|m| m.activities.iter_mut())
        .filter_map(|a| a.machine.as_deref_mut())
        .collect();
    if let Some(turn) = conversation.active_turn.as_mut() {
        receipts.extend(
            turn.activities
                .iter_mut()
                .filter_map(|a| a.machine.as_deref_mut()),
        );
    }
    Box::pin(crate::services::machine_activity_service::refresh_jobs(
        &state.db, &user_id, receipts,
    ))
    .await?;
    let agents = crate::services::assistant_team_service::agents(&state.db, &user_id, true).await?;
    let agent_id = conversation.agent_id.clone();
    let chats = super::nyxbot::chats::thread_details(&state, &user_id, &[&conversation]).await?;
    let mut conversation = ConversationResponse::from(conversation)
        .with_agent(agent_id.as_deref(), &agents)
        .with_chat(&chats);
    conversation.pending_acknowledgements = acknowledgements
        .iter()
        .filter(|ack| ack.status == "pending")
        .count() as u32;
    let mut messages: Vec<MessageResponse> = rows.into_iter().map(Into::into).collect();
    let mut receipts: Vec<_> = messages
        .iter_mut()
        .flat_map(|m| m.activities.iter_mut())
        .filter_map(|a| a.machine.as_mut())
        .collect();
    if let Some(turn) = conversation.active_turn.as_mut() {
        receipts.extend(
            turn.activities
                .iter_mut()
                .filter_map(|a| a.machine.as_mut()),
        );
    }
    Box::pin(resolve_machine_names(&state.db, &user_id, receipts)).await?;
    let mut attachments: Vec<_> = messages
        .iter_mut()
        .flat_map(|m| m.attachments.iter_mut())
        .collect();
    if let Some(turn) = conversation.active_turn.as_mut() {
        attachments.extend(turn.attachments.iter_mut());
    }
    Box::pin(mark_expired_attachments(&state.db, &user_id, attachments)).await?;
    Ok(Json(HistoryResponse {
        conversation,
        acknowledgements: acknowledgements.into_iter().map(Into::into).collect(),
        approvals: approvals.into_iter().map(Into::into).collect(),
        waiting,
        messages,
        before_seq,
    }))
}
/// An image a tool returned during one of the owner's turns. Only verified
/// raster types are ever stored, and they are served inline with nosniff.
pub async fn attachment(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, attachment_id)): Path<(String, String)>,
) -> AppResult<Response> {
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    auth.ensure_live_assistant_turn(&state.db, "assistant.attachment")
        .await?;
    if Uuid::parse_str(&attachment_id).is_err() {
        return Err(AppError::NotFound("Attachment not found".into()));
    }
    let (content_type, bytes) = engine::read_attachment(
        &state.db,
        &state.encryption_keys,
        &user_id,
        &id,
        &attachment_id,
    )
    .await?;
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    for (name, value) in [
        ("content-type", content_type.as_str()),
        ("cache-control", "private, no-store"),
        ("x-content-type-options", "nosniff"),
        ("content-disposition", "inline"),
        ("content-security-policy", "default-src 'none'; sandbox"),
    ] {
        headers.insert(
            name,
            value
                .parse()
                .map_err(|_| AppError::Internal("Invalid attachment header".into()))?,
        );
    }
    Ok(response)
}
#[derive(Serialize)]
pub struct AcknowledgementResponse {
    continuation_owner: Option<&'static str>,
    continuation_receipt_id: Option<String>,
    trigger_run_id: Option<String>,
    id: String,
    kind: String,
    status: String,
    summary: String,
    /// `user` or `orchestrator` (a subagent's request its orchestrator decides).
    decider: String,
    decided_by: Option<String>,
    reason: Option<String>,
    service_slug: Option<String>,
    service_name: Option<String>,
    tool_name: Option<String>,
    created_at: DateTime<Utc>,
    decided_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
}
impl From<crate::models::assistant_acknowledgement::AssistantAcknowledgement>
    for AcknowledgementResponse
{
    fn from(row: crate::models::assistant_acknowledgement::AssistantAcknowledgement) -> Self {
        Self {
            continuation_owner: row.continuation_receipt_id.as_ref().map(|_| "server"),
            continuation_receipt_id: row.continuation_receipt_id,
            id: row.id,
            kind: row.kind,
            status: row.status,
            summary: row.summary,
            trigger_run_id: row.trigger_run_id,
            decider: row.decider,
            decided_by: row.decided_by,
            reason: row.reason,
            service_slug: row.service_slug,
            service_name: row.service_name,
            tool_name: row.tool_name,
            created_at: row.created_at,
            decided_at: row.decided_at,
            expires_at: row.expires_at,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Deny,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgementDecision {
    decision: Decision,
}

pub async fn decide_acknowledgement(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, ack_id)): Path<(String, String)>,
    Json(body): Json<AcknowledgementDecision>,
) -> AppResult<Json<AcknowledgementResponse>> {
    let user = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user).await?;
    let row = acknowledgements::decide(
        &state.db,
        &user,
        &id,
        &ack_id,
        matches!(body.decision, Decision::Allow),
    )
    .await?;
    acknowledgements::audit_decision(
        &state.db,
        &crate::services::audit_service::AuditActor::from_auth_user(&auth),
        &row,
    )
    .await;
    // A user decision on a subagent's request resumes the subagent.
    if row.decider == "orchestrator" {
        super::assistant_team::permission_decided(&state, &user, &row).await;
    }
    Ok(Json(row.into()))
}

/// Retired: every chat runs with Full access. Kept so older clients get a
/// stable, explicit answer instead of a 404.
pub async fn change_access_mode(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(_id): Path<String>,
) -> AppResult<Response> {
    engine::require_enabled(&state.db, &auth.user_id.to_string()).await?;
    Ok((
        StatusCode::GONE,
        Json(json!({
            "error": "access_mode_retired",
            "message": "Every NyxBot chat runs with Full access; the Ask/Full choice was removed.",
        })),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameRequest {
    title: String,
}
pub async fn rename(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<RenameRequest>,
) -> AppResult<Json<ConversationResponse>> {
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    let row = engine::rename(&state.db, &user_id, &id, &body.title).await?;
    // The same shape as the index row, so a rename never drops the agent.
    let agents = crate::services::assistant_team_service::agents(&state.db, &user_id, true).await?;
    let counts =
        acknowledgements::pending_counts(&state.db, &user_id, std::slice::from_ref(&row.id))
            .await?;
    let count = counts.get(&row.id).copied().unwrap_or(0);
    let agent_id = row.agent_id.clone();
    let chats = super::nyxbot::chats::thread_details(&state, &user_id, &[&row]).await?;
    let mut dto = ConversationResponse::from(row)
        .with_agent(agent_id.as_deref(), &agents)
        .with_chat(&chats);
    dto.pending_acknowledgements = count;
    let receipts = dto
        .active_turn
        .iter_mut()
        .flat_map(|t| t.activities.iter_mut())
        .filter_map(|a| a.machine.as_mut())
        .collect();
    Box::pin(resolve_machine_names(&state.db, &user_id, receipts)).await?;
    Ok(Json(dto))
}
pub async fn stop(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    engine::request_stop(&state.db, &user_id, &id).await?;
    super::machine_cancel::conversation(&state, &user_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    request: Request<Body>,
) -> AppResult<StatusCode> {
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    let target = engine::get(&state.db, &user_id, &id).await?;
    let ids = vec![target.id.clone()];
    let mut credentials_by_id = HashMap::new();
    for member in &ids {
        if let Some(credential) =
            credentials::load_for_conversation(&state.db, &state.encryption_keys, &user_id, member)
                .await?
        {
            credentials_by_id.insert(member.clone(), credential);
        }
    }
    let rows = engine::delete(&state.db, &user_id, &id).await?;
    let policy = request.extensions().get::<BillingRoutePolicy>().copied();
    for row in rows {
        let Some(session_id) = row.nyxagent_session_id.clone() else {
            continue;
        };
        let credential = credentials_by_id.remove(&row.id);
        let state = state.clone();
        let auth = auth.clone();
        tokio::spawn(async move {
            let result: AppResult<()> = async {
                if let Some(credential) = credential
                    && credential.api_key_id == row.credential_api_key_id
                {
                    let response = proxy(
                        &state,
                        &auth,
                        &credential,
                        "DELETE",
                        &format!("v1/sessions/{session_id}"),
                        None,
                        None,
                        policy,
                    )
                    .await?;
                    if !response.status().is_success() {
                        return Err(AppError::Internal("Session delete failed".into()));
                    }
                }
                Ok(())
            }
            .await;
            if result.is_err() {
                tracing::debug!("NyxAgent session deletion unavailable; local transcript deleted");
            }
        });
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Clone, Serialize)]
pub struct ModelResponse {
    id: String,
    label: String,
}
type ModelCache = HashMap<String, (Instant, Vec<ModelResponse>)>;
static MODELS: LazyLock<Mutex<ModelCache>> = LazyLock::new(|| Mutex::new(HashMap::new()));
pub async fn models(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Request<Body>,
) -> AppResult<Json<Vec<ModelResponse>>> {
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    let cache_key = state.db.name().to_owned();
    if let Some((time, models)) = MODELS.lock().await.get(&cache_key)
        && time.elapsed() < Duration::from_secs(60)
    {
        return Ok(Json(models.clone()));
    }
    let policy = request.extensions().get::<BillingRoutePolicy>().copied();
    let loaded: AppResult<Vec<ModelResponse>> = async {
        let credential = credentials::load_existing(&state.db, &state.encryption_keys, &user_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Assistant credential not provisioned".into()))?;
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            proxy(
                &state,
                &auth,
                &credential,
                "GET",
                "v1/models",
                None,
                None,
                policy,
            ),
        )
        .await
        .map_err(|_| AppError::Internal("Assistant profiles unavailable".into()))??;
        if !response.status().is_success() {
            return Err(AppError::Internal("Assistant profiles unavailable".into()));
        }
        let bytes = tokio::time::timeout(
            Duration::from_secs(10),
            axum::body::to_bytes(response.into_body(), 65536),
        )
        .await
        .map_err(|_| AppError::Internal("Assistant profiles unavailable".into()))?
        .map_err(|_| AppError::Internal("Assistant profiles unavailable".into()))?;
        let data: Value = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::Internal("Assistant profiles unavailable".into()))?;
        let rows: Vec<_> = data["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| row["id"].as_str())
            .filter(|id| engine::valid_model(id))
            .take(50)
            .map(|id| ModelResponse {
                id: id.into(),
                label: id.trim_start_matches("nyxagent/").into(),
            })
            .collect();
        if rows.is_empty() {
            return Err(AppError::Internal("Assistant profiles unavailable".into()));
        }
        Ok(rows)
    }
    .await;
    let Ok(rows) = loaded else {
        return Ok(Json(vec![ModelResponse {
            id: engine::DEFAULT_MODEL.into(),
            label: "chat".into(),
        }]));
    };
    let mut cache = MODELS.lock().await;
    if cache.len() >= 64 {
        cache.retain(|_, (time, _)| time.elapsed() < Duration::from_secs(60));
    }
    cache.insert(cache_key, (Instant::now(), rows.clone()));
    Ok(Json(rows))
}

// Fresh request: no caller query, Authorization, Cookie, debug capture, or other
// headers cross the boundary. Only the route's egress classification is retained.
#[allow(clippy::too_many_arguments)]
async fn proxy(
    state: &AppState,
    auth: &AuthUser,
    credential: &AssistantCredential,
    method: &str,
    path: &str,
    body: Option<Value>,
    turn_id: Option<&str>,
    policy: Option<BillingRoutePolicy>,
) -> AppResult<Response> {
    let service =
        assistant_service::resolve_admin_service_by_slug(&state.db, engine::SERVICE_SLUG).await?;
    if !engine::row_contract(Some(&service), None).valid() {
        return Err(AppError::Internal(
            "NyxAgent catalog configuration is invalid".into(),
        ));
    }
    let payload = body
        .map(|body| serde_json::to_vec(&body))
        .transpose()
        .map_err(|_| AppError::Internal("Assistant request encoding failed".into()))?
        .unwrap_or_default();
    let mut request = Request::builder()
        .method(method)
        .uri("/api/v1/assistant/nyxagent/turns")
        .header("content-type", "application/json")
        .header(
            "accept",
            if turn_id.is_some() {
                "text/event-stream"
            } else {
                "application/json"
            },
        )
        .body(Body::from(payload))
        .map_err(|_| AppError::Internal("Assistant request encoding failed".into()))?;
    if let Some(policy) = policy {
        request.extensions_mut().insert(policy);
    }
    let mut headers = vec![(
        "authorization".into(),
        format!("Bearer {}", credential.raw_key.as_str()),
    )];
    if let Some(turn_id) = turn_id {
        headers.push(("idempotency-key".into(), turn_id.into()));
    }
    super::proxy::execute_admin_proxy(
        state,
        auth,
        &service.id,
        path,
        request,
        headers,
        &mut String::new(),
    )
    .await
}

struct Events {
    sender: broadcast::Sender<Value>,
    cursor: u64,
}
impl Events {
    fn emit(&mut self, event: &str, mut data: Value) {
        self.cursor += 1;
        data["event"] = json!(event);
        data["cursor"] = json!(self.cursor);
        let _ = self.sender.send(data);
    }
    fn notice(&mut self) {
        self.emit(
            "turn.notice",
            json!({"code": "context_reset", "message": engine::CONTEXT_NOTICE}),
        );
    }
}

pub async fn turns(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    let (parts, body) = request.into_parts();
    let bytes =
        super::body_limit::read_body(body, engine::MAX_REQUEST_BYTES, "Assistant turn").await?;
    let input = engine::parse_turn(&bytes)?;
    if !input.attachment_ids.is_empty() {
        super::login_client_context::require_first_party_human(&auth)?;
    }
    // Ownership is checked before provisioning or touching a credential.
    let org_access = if let Some(id) = &input.conversation_id {
        engine::get_authorized(&state.db, &user_id, id).await?.1
    } else {
        None
    };
    let permit = state.direct_chat_limiter.try_acquire(&user_id).await?;
    let policy = parts.extensions.get::<BillingRoutePolicy>().copied();
    let mut start = engine::TurnStart::from(&input);
    start.org_access = org_access;
    if start.conversation_id.is_none() && start.model.is_none() {
        start.model = Some(
            crate::services::assistant_profile_routing::model_for(
                &state.db,
                crate::services::assistant_profile_routing::RouteRole::Orchestrator,
                engine::DEFAULT_MODEL,
            )
            .await,
        );
    }
    let (_, receiver) = start_turn(&state, auth, &start, policy, permit).await?;
    Ok(subscribe_events(receiver))
}

/// The billing classification of a server-started turn: the same metered
/// proxy egress as a browser turn on `/assistant/nyxagent/turns`.
pub(crate) const SERVER_TURN_POLICY: BillingRoutePolicy =
    BillingRoutePolicy::Metered(crate::services::billing::route_inventory::BillingIngress::Proxy);

/// Claim a turn and run it detached. The caller subscribes to the returned
/// receiver before any event is emitted, so even an immediate completion is
/// observable. `auth` is the owner acting (a browser session, or the owner
/// identity NyxID uses for server-started turns).
pub(crate) async fn start_turn(
    state: &AppState,
    auth: AuthUser,
    start: &engine::TurnStart,
    policy: Option<BillingRoutePolicy>,
    permit: DirectChatPermit,
) -> AppResult<(AssistantConversation, broadcast::Receiver<Value>)> {
    Box::pin(start_turn_with_voice(
        state, auth, start, policy, permit, None,
    ))
    .await
}

pub(crate) async fn start_turn_with_voice(
    state: &AppState,
    auth: AuthUser,
    start: &engine::TurnStart,
    policy: Option<BillingRoutePolicy>,
    permit: DirectChatPermit,
    voice_request_id: Option<&str>,
) -> AppResult<(AssistantConversation, broadcast::Receiver<Value>)> {
    let user_id = auth.user_id.to_string();
    let row = if voice_request_id.is_some() {
        Box::pin(engine::begin_turn_with_voice(
            &state.db,
            &user_id,
            start,
            &state.encryption_keys,
            voice_request_id,
        ))
        .await?
    } else {
        Box::pin(engine::begin_turn(
            &state.db,
            &user_id,
            start,
            &state.encryption_keys,
        ))
        .await?
    };
    let mut text = engine::turn_input(&row, start);
    if start.origin == crate::models::assistant_conversation::TurnOrigin::Channel
        && let Some(prelude) = Box::pin(super::nyxbot::thread_follow::prelude(state, &row)).await
    {
        text.push_str(&prelude);
    }
    let credential =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, &user_id, &row.id)
            .await?
            .ok_or_else(|| AppError::NotFound("Assistant credential not found".into()))?;
    let (sender, receiver) = broadcast::channel(256);
    tokio::spawn(run_turn(
        state.clone(),
        auth,
        row.clone(),
        text,
        credential,
        policy,
        permit,
        Events { sender, cursor: 0 },
    ));
    Ok((row, receiver))
}

fn subscribe_events(mut receiver: broadcast::Receiver<Value>) -> Response {
    let stream = async_stream::stream! {
        loop {
            let event = match receiver.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            };
            let terminal = event["event"] == "turn.completed";
            yield Ok::<_, Infallible>(
                Event::default()
                    .event(event["event"].as_str().unwrap_or("error"))
                    .data(event.to_string()),
            );
            if terminal {
                break;
            }
        }
    };
    sse_response(stream)
}

fn sse_response(
    stream: impl futures::Stream<Item = Result<Event, Infallible>> + Send + 'static,
) -> Response {
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", "no".parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "no-cache, no-transform".parse().unwrap());
    response
}

/// How long one live stream stays open before the browser reconnects (and
/// is authenticated again).
const LIVE_STREAM_SECS: u64 = 300;

/// `GET /assistant/nyxagent/live`: the owner's assistant changes as they
/// happen, so the browser refreshes a thread, the agents or a group the
/// moment NyxID changes it instead of polling. Frames carry identifiers
/// only, each also naming its `type`: `ready`, `conversation` `{id,
/// group_id, turn_id, messages}`, `group` `{id}`, `channels`, and `resync`
/// when changes may have been missed. 503 while this replica's change stream
/// is not delivering, 429 past the per-owner stream cap: the browser keeps
/// polling and retries.
pub async fn live(State(state): State<AppState>, auth: AuthUser) -> AppResult<Response> {
    use crate::services::assistant_live::LiveEvent;
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    // Promise live updates only while this replica's change stream delivers;
    // otherwise the browser keeps polling and retries shortly.
    let mut open = state.assistant_live.watch_open();
    if !*open.borrow_and_update() {
        let mut response = StatusCode::SERVICE_UNAVAILABLE.into_response();
        response
            .headers_mut()
            .insert("retry-after", "5".parse().unwrap());
        return Ok(response);
    }
    let Some(mut subscription) = state.assistant_live.subscribe_owner(&user_id) else {
        let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
        response
            .headers_mut()
            .insert("retry-after", "30".parse().unwrap());
        return Ok(response);
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(LIVE_STREAM_SECS);
    let stream = async_stream::stream! {
        yield Ok::<_, Infallible>(
            Event::default().event("ready").data(json!({"type": "ready"}).to_string()),
        );
        loop {
            let event = tokio::select! {
                event = subscription.events.recv() => event,
                _ = tokio::time::sleep_until(deadline) => break,
                // The change stream dropped: end, so the browser polls again.
                _ = open.wait_for(|open| !*open) => break,
            };
            let (name, data) = match event {
                Ok(LiveEvent::Conversation { id, user_id: owner, group_id, turn_id, messages, title_changed })
                    if owner == user_id =>
                {
                    ("conversation", json!({"type": "conversation", "id": id,
                        "group_id": group_id, "turn_id": turn_id, "messages": messages, "title_changed": title_changed}))
                }
                Ok(LiveEvent::OrgGroup { id, user_id: owner }) if owner == user_id => {
                    // The channel routes candidates; authority is live again at delivery,
                    // including the first delivery after subscribing.
                    if crate::services::org_group_service::get(&state.db,&user_id,&id,None).await.is_err() { continue; }
                    ("group", json!({"type":"group","id":id}))
                }
                Ok(LiveEvent::Group { id, user_id: owner }) if owner == user_id => {
                    ("group", json!({"type": "group", "id": id}))
                }
                Ok(LiveEvent::ChannelThread { id, user_id:owner, channel_id, parent_id, conversation_id }) if owner==user_id => {
                    ("channel_thread",json!({"type":"channel_thread","id":id,"channel_id":channel_id,"parent_id":parent_id,"conversation_id":conversation_id}))
                }
                Ok(LiveEvent::ChannelBot { user_id: owner, .. }) if owner == user_id => {
                    ("channels", json!({"type": "channels"}))
                }
                Ok(LiveEvent::Resync) | Err(broadcast::error::RecvError::Lagged(_)) => {
                    ("resync", json!({"type": "resync"}))
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Ok(_) => continue,
            };
            yield Ok(Event::default().event(name).data(data.to_string()));
        }
    };
    Ok(sse_response(stream))
}

#[allow(clippy::too_many_arguments)]
async fn run_turn(
    state: AppState,
    auth: AuthUser,
    row: AssistantConversation,
    text: String,
    mut credential: AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    permit: DirectChatPermit,
    mut events: Events,
) {
    let turn_id = row
        .active_turn
        .as_ref()
        .expect("claimed turn")
        .turn_id
        .clone();
    let message_id = Uuid::new_v4().to_string();
    let block_id = format!("{message_id}-text");
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
    let mut result = {
        // Execution includes upload planning and upstream streaming. Keep that
        // state off the caller's stack when this task is created by a tool.
        let mut execution = Box::pin(execute_turn(
            &state,
            &auth,
            &row,
            &text,
            &mut credential,
            policy,
            &mut events,
            &block_id,
            &mut partial,
        ));
        tokio::select! {
            result = &mut execution => result,
            () = permit.cancelled() => Err(TurnError::new("assistant_unavailable")),
            error = watch_stop(&state, &row, &turn_id) => Err(error),
            () = tokio::time::sleep(Duration::from_secs(610 * (crate::services::assistant_continuation::HARD_MAX as u64 + 1))) => {
                Err(TurnError::new("turn_timeout"))
            }
        }
    }
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
        let _ = super::machine_cancel::conversation(&state, &row.user_id, &row.id).await;
    }
    // Never echo the system-managed credential, even if an upstream reflects it.
    result.text = result
        .text
        .replace(credential.raw_key.as_str(), "[redacted]");
    let settled = complete_turn(
        &row,
        &turn_id,
        &message_id,
        &block_id,
        &result,
        permit,
        events,
        Duration::from_secs(engine::SETTLEMENT_GRACE_SECS),
        || {
            engine::finish_turn(
                &state.db,
                &row,
                &credential.api_key_id,
                &message_id,
                &result,
            )
        },
    )
    .await;
    if let Some(error) = settled {
        if error.is_none()
            && row.message_count == 1
            && crate::services::assistant_title_service::eligible(&row)
        {
            super::assistant_titles::spawn(state.clone(), auth.clone(), row.id.clone());
        }
        // Team wake-ups and channel deliveries follow a durable settlement only.
        super::assistant_team::after_turn_boxed(
            state.clone(),
            row.clone(),
            result.text.clone(),
            error,
        )
        .await;
    }
}

/// Bound both individual database attempts and backoff by one settlement deadline.
/// Owning the permit here guarantees release after either settlement or expiry.
#[allow(clippy::too_many_arguments)]
async fn complete_turn<F, Fut>(
    row: &AssistantConversation,
    turn_id: &str,
    message_id: &str,
    block_id: &str,
    result: &TurnResult,
    permit: DirectChatPermit,
    mut events: Events,
    settle_for: Duration,
    mut persist: F,
) -> Option<Option<TurnError>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = AppResult<Option<TurnError>>>,
{
    let deadline = tokio::time::Instant::now() + settle_for;
    let mut attempt = 0u32;
    let error = loop {
        match tokio::time::timeout_at(deadline, persist()).await {
            Ok(Ok(error)) => break error,
            // Deleted conversations and reclaimed turns must never be recreated.
            Ok(Err(AppError::NotFound(_))) => return None,
            Ok(Err(_)) => {
                attempt = attempt.saturating_add(1);
                let backoff = Duration::from_millis(100 * (1 << attempt.min(8)));
                tokio::time::sleep_until((tokio::time::Instant::now() + backoff).min(deadline))
                    .await;
            }
            Err(_) => {
                tracing::error!(
                    conversation_id = %row.id,
                    turn_id,
                    "Assistant settlement deadline exceeded"
                );
                break Some(TurnError::new("assistant_unavailable"));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::error!(
                conversation_id = %row.id,
                turn_id,
                "Assistant settlement deadline exceeded"
            );
            break Some(TurnError::new("assistant_unavailable"));
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
    events.emit(
        "turn.completed",
        json!({"turn_id": turn_id, "status": status, "error": error}),
    );
    drop(permit);
    Some(error)
}
async fn watch_stop(state: &AppState, row: &AssistantConversation, turn_id: &str) -> TurnError {
    loop {
        match engine::stop_requested(&state.db, &row.user_id, &row.id, turn_id).await {
            Ok(false) => tokio::time::sleep(Duration::from_millis(250)).await,
            Ok(true) => return TurnError::new("cancelled"),
            Err(_) => return TurnError::new("assistant_unavailable"),
        }
    }
}

/// Typed proxy failures collapse to `assistant_unavailable`, except a billing
/// refusal, which keeps its stable code so the client can offer a top-up.
fn proxy_turn_error(error: AppError) -> TurnError {
    match error {
        AppError::InsufficientCredits => TurnError::new("insufficient_credits"),
        _ => TurnError::new("assistant_unavailable"),
    }
}

/// Reads the NyxAgent `error.code` envelope, or NyxID's own flat envelope
/// (`{"error":"insufficient_credits",...}`) on a 402. Other flat categories are
/// not turn codes and stay unrecognized.
fn upstream_error_code(status: u16, envelope: &Value) -> &str {
    if let Some(code) = envelope["error"]["code"].as_str() {
        return code;
    }
    if status == 402 && envelope["error"].as_str() == Some("insufficient_credits") {
        return "insufficient_credits";
    }
    ""
}

#[allow(clippy::too_many_arguments)]
async fn execute_turn(
    state: &AppState,
    auth: &AuthUser,
    row: &AssistantConversation,
    text: &str,
    credential: &mut AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    events: &mut Events,
    block_id: &str,
    partial: &mut String,
) -> Result<TurnResult, TurnError> {
    let turn_id = &row.active_turn.as_ref().expect("claimed turn").turn_id;
    let mut history = engine::messages(
        &state.db,
        &row.user_id,
        &row.id,
        20,
        Some(row.message_count + 1),
    )
    .await
    .map_err(|_| TurnError::new("assistant_unavailable"))?;
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
    let previous = history
        .iter()
        .rev()
        .find(|message| message.role == "user")
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
                return Err(TurnError::new("assistant_unavailable"));
            }
            Err(_) => None,
        };
    decisions
        .push_str(&super::assistant_team::turn_notes(state, row, agent.as_ref(), previous).await);
    let mut binding = row.nyxagent_session_id.clone();
    let mut prompt = if binding.is_none() && row.context_reset_reason.is_some() {
        events.notice();
        engine::instructions(row, agent.as_ref(), &history)
    } else {
        engine::base_prompt(row, agent.as_ref())
    } + &decisions;
    let mut attachments = crate::services::assistant_upload_service::turn_attachments(
        &state.db,
        &row.user_id,
        &row.id,
        turn_id,
    )
    .await
    .map_err(|_| TurnError::new("assistant_unavailable"))?;
    let expired = Box::pin(crate::services::assistant_upload_retention::expired_ids(
        &state.db,
        &row.user_id,
        &attachments.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
    ))
    .await
    .map_err(|_| TurnError::new("assistant_unavailable"))?;
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
        let lookup = async {
            let response = proxy(
                state,
                auth,
                credential,
                "GET",
                "v1/capabilities",
                None,
                None,
                policy,
            )
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
        tokio::time::timeout(Duration::from_secs(5), lookup)
            .await
            .ok()
            .flatten()
            .unwrap_or(Value::Null)
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
    prompt.push_str(&listing);
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
            .map_err(|_| TurnError::new("assistant_unavailable"))?;
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
                .map_err(|_| TurnError::new("assistant_unavailable"))?;
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
        .map_err(|_| TurnError::new("assistant_unavailable"))?;
    let mut continuations = crate::services::assistant_continuation::Continuations::new(
        settings.max_auto_continuations,
    );
    let mut completed = String::new();
    let mut input = if text.trim().is_empty() {
        "Please discuss the attachments in this message."
    } else {
        text
    };
    let mut recovery = engine::Recovery::default();
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
            .map_err(|_| TurnError::new("assistant_unavailable"))?;
        }
        let request_key = if continuations.count == 0 {
            turn_id.clone()
        } else {
            format!("{turn_id}:continuation:{}", continuations.count)
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let response = tokio::time::timeout_at(
            deadline,
            proxy(
                state,
                auth,
                credential,
                "POST",
                "v1/responses",
                Some({
                    let mut body =
                        engine::upstream_body(&row.model, input, binding.as_deref(), &prompt);
                    if continuations.count == 0 && !image_parts.is_empty() {
                        let mut content = vec![json!({"type":"input_text", "text":input})];
                        content.extend(image_parts.clone());
                        body["input"] = json!([{"role":"user", "content":content}]);
                    }
                    body
                }),
                Some(&request_key),
                policy,
            ),
        )
        .await
        .map_err(|_| TurnError::new("first_byte_timeout"))?
        .map_err(proxy_turn_error)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let bytes = tokio::time::timeout(
                Duration::from_secs(10),
                axum::body::to_bytes(response.into_body(), 65536),
            )
            .await
            .ok()
            .and_then(Result::ok);
            let envelope: Value = bytes
                .as_ref()
                .and_then(|bytes| serde_json::from_slice(bytes).ok())
                .unwrap_or(Value::Null);
            let code = upstream_error_code(status, &envelope);
            if matches!(code, "tool_budget_exhausted" | "turn_timeout") {
                let error = TurnError::new(code);
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
            match recovery.decide(status, code, binding.is_some()) {
                RecoveryAction::Rebind => {
                    engine::clear_binding(
                        &state.db,
                        &row.user_id,
                        &row.id,
                        turn_id,
                        "session_lost",
                    )
                    .await
                    .map_err(|_| TurnError::new("assistant_unavailable"))?;
                    binding = None;
                    prompt =
                        engine::instructions(row, agent.as_ref(), &history) + &decisions + &listing;
                    events.notice();
                }
                RecoveryAction::ReplaceCredential => {
                    *credential = credentials::replace(
                        &state.db,
                        &state.encryption_keys,
                        &row.user_id,
                        &row.id,
                        &credential.api_key_id,
                    )
                    .await
                    .map_err(|_| TurnError::new("agent_key_required"))?;
                    binding = None;
                    prompt =
                        engine::instructions(row, agent.as_ref(), &history) + &decisions + &listing;
                    events.notice();
                }
                RecoveryAction::Backoff => {
                    let jitter = u64::from(rand::random::<u16>() % 500);
                    tokio::time::sleep(Duration::from_millis(
                        (1 << recovery.retries) * 1000 + jitter,
                    ))
                    .await;
                }
                RecoveryAction::Fail => {
                    return Err(TurnError::new(if status == 401 || status == 403 {
                        "agent_key_required"
                    } else {
                        code
                    }));
                }
            }
            continue;
        }
        if !response
            .headers()
            .get("content-type")
            .is_some_and(|v| v.to_str().is_ok_and(|v| v.starts_with("text/event-stream")))
        {
            return Err(TurnError::new("invalid_stream"));
        }
        let mut body = response.into_body().into_data_stream();
        let mut decoder = ResponseStream::default();
        let mut first = true;
        let mut emitted_bytes = completed.len();
        loop {
            let timeout = if first {
                deadline.saturating_duration_since(tokio::time::Instant::now())
            } else {
                Duration::from_secs(120)
            };
            let next = tokio::time::timeout(timeout, body.next())
                .await
                .map_err(|_| {
                    TurnError::new(if first {
                        "first_byte_timeout"
                    } else {
                        "idle_timeout"
                    })
                })?;
            first = false;
            let Some(chunk) = next else {
                break;
            };
            let chunk = chunk.map_err(|_| TurnError::new("invalid_stream"))?;
            let decoded = decoder.push(&chunk);
            *partial = completed.clone()
                + &decoder
                    .text
                    .replace(credential.raw_key.as_str(), "[redacted]");
            if partial.len() > engine::MAX_OUTPUT_BYTES {
                return Err(TurnError::new("output_too_large"));
            }
            decoded?;
            if decoder.terminal.is_some() {
                break;
            }
            // Hold a key-length suffix so a reflected key split across deltas
            // cannot leak before the next fragment reveals the full match.
            if decoder.terminal.is_none() {
                let mut safe_end = partial.len().saturating_sub(credential.raw_key.len());
                while !partial.is_char_boundary(safe_end) {
                    safe_end -= 1;
                }
                if safe_end > emitted_bytes {
                    events.emit(
                        "block.delta",
                        json!({"block_id": block_id, "text": &partial[emitted_bytes..safe_end]}),
                    );
                    emitted_bytes = safe_end;
                }
            }
        }
        let mut result = decoder
            .terminal
            .ok_or_else(|| TurnError::new("invalid_stream"))?;
        result.text = result
            .text
            .replace(credential.raw_key.as_str(), "[redacted]");
        binding = result.session_id.clone().or(binding);
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
                    if partial.len() > emitted_bytes {
                        events.emit(
                            "block.delta",
                            json!({"block_id":block_id,"text":&partial[emitted_bytes..]}),
                        );
                    }
                    if !partial.is_empty() {
                        events.emit("block.delta", json!({"block_id":block_id,"text":"\n\n"}));
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

async fn continue_turn(
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
        .map_err(|_| TurnError::new("assistant_unavailable"))?;
    let active = current
        .active_turn
        .as_ref()
        .filter(|t| {
            !t.stop_requested && Some(&t.turn_id) == row.active_turn.as_ref().map(|t| &t.turn_id)
        })
        .ok_or_else(|| TurnError::new("cancelled"))?;
    let progress =
        crate::services::assistant_continuation::window_digest(text, &active.tool_progress);
    if !continuations.next(error.code, binding.is_some(), progress)? {
        return Ok(false);
    }
    let lease = chrono::Utc::now() + chrono::Duration::seconds(engine::ACTIVE_TURN_TTL_SECS);
    let updated = state.db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(doc! {"_id":&row.id,"user_id":&row.user_id,"active_turn.turn_id":&active.turn_id,"active_turn.stop_requested":false},
            doc! {"$set":{"nyxagent_session_id":binding,"active_turn.continuations":continuations.count,"active_turn.tool_progress": {"started": 0_i64, "calls": 0_i64, "digest": ""},"active_turn.lease_expires_at":mongodb::bson::DateTime::from_chrono(lease)}})
        .await.map_err(|_|TurnError::new("assistant_unavailable"))?;
    if updated.matched_count != 1 {
        return Err(TurnError::new("cancelled"));
    }
    tracing::info!(conversation_id = %row.id, upstream_error_code = error.code, continuation = continuations.count, "Continuing assistant task on its existing session");
    events.emit(
        "turn.continuing",
        json!({"turn_id":active.turn_id,"continuation":continuations.count}),
    );
    Ok(true)
}

#[cfg(test)]
#[path = "assistant_nyxagent_tests.rs"]
mod tests;
