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
use tracing::Instrument;
use uuid::Uuid;

#[cfg(test)]
use crate::services::assistant_nyxagent::TurnError;
use crate::services::assistant_nyxagent_runtime::Events;
#[cfg(test)]
use crate::services::assistant_nyxagent_runtime::{
    STREAM_BOUNDS, SettledExecution, StreamBounds, TEST_STREAM_BOUNDS, complete_turn,
    proxy_turn_error, sanitize_upstream_request_id, stream_timeout, upstream_error_code,
};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{assistant_conversation::AssistantConversation, assistant_message::AssistantMessage},
    mw::{auth::AuthUser, rate_limit::DirectChatPermit},
    services::{
        assistant_acknowledgement_service as acknowledgements,
        assistant_agent_credential_service::{self as credentials, AssistantCredential},
        assistant_nyxagent as engine, assistant_service,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<crate::models::assistant_conversation::ToolFailure>,
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
            failure: row.failure,
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
pub struct PlatformThreadResponse {
    platform: String,
    manager_id: String,
    binding_id: String,
    binding_generation: i64,
}
impl From<&crate::models::assistant_conversation::PlatformThread> for PlatformThreadResponse {
    fn from(thread: &crate::models::assistant_conversation::PlatformThread) -> Self {
        Self {
            platform: thread.platform.clone(),
            manager_id: thread.profile_id.clone(),
            binding_id: thread.binding_id.clone(),
            binding_generation: thread.binding_generation,
        }
    }
}
#[derive(Serialize)]
pub struct PlatformTurnOriginResponse {
    binding_id: String,
    binding_generation: i64,
    manager_authority_epoch: i64,
    personal_control_revision: i64,
}
impl From<&crate::models::assistant_conversation::PlatformTurnOrigin>
    for PlatformTurnOriginResponse
{
    fn from(origin: &crate::models::assistant_conversation::PlatformTurnOrigin) -> Self {
        Self {
            binding_id: origin.thread.binding_id.clone(),
            binding_generation: origin.thread.binding_generation,
            manager_authority_epoch: origin.manager_authority_epoch,
            personal_control_revision: origin.personal_control_revision,
        }
    }
}
#[derive(Serialize)]
pub struct ActiveTurnResponse {
    platform_origin: Option<PlatformTurnOriginResponse>,
    response_id: Option<String>,
    steering_allowed: bool,
    stop_requested: bool,
    continuations: u32,
    activity_total: i64,
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
    platform_thread: Option<PlatformThreadResponse>,
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
    context_reset_reason: Option<String>,
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
            platform_origin: turn
                .platform_origin
                .as_ref()
                .map(PlatformTurnOriginResponse::from),
            response_id: turn
                .running_response
                .as_ref()
                .map(|r| r.response_id.clone()),
            steering_allowed: matches!(
                crate::services::assistant_steering::unavailable(&row, &turn.turn_id),
                None | Some("starting")
            ),
            stop_requested: turn.stop_requested,
            continuations: turn.continuations,
            activity_total: turn.activity_total,
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
            platform_thread: row
                .platform_thread
                .as_ref()
                .map(PlatformThreadResponse::from),
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
            context_reset_reason: row.context_reset_reason,
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
pub struct ErrorDetailResponse {
    stage: String,
    http_status: Option<u16>,
    upstream_request_id: Option<String>,
    response_id: Option<String>,
    continuations: u32,
    turn_id: String,
    occurred_at: DateTime<Utc>,
}

impl From<crate::models::assistant_message::AssistantErrorDetail> for ErrorDetailResponse {
    fn from(value: crate::models::assistant_message::AssistantErrorDetail) -> Self {
        Self {
            stage: value.stage,
            http_status: value.http_status,
            upstream_request_id: value.upstream_request_id,
            response_id: value.response_id,
            continuations: value.continuations,
            turn_id: value.turn_id,
            occurred_at: value.occurred_at,
        }
    }
}

#[derive(Serialize)]
pub struct MessageResponse {
    platform_origin: Option<PlatformTurnOriginResponse>,
    recovery: Option<RecoveryResponse>,
    pub feedback: Option<crate::handlers::assistant_feedback::FeedbackResponse>,
    steering: Option<steering::SteeringResponse>,
    voice: Option<VoiceTranscriptResponse>,
    execution_pending: bool,
    continuation_of: Option<String>,
    id: String,
    seq: i64,
    turn_id: String,
    role: String,
    text: String,
    status: String,
    error_code: Option<String>,
    upstream_error_code: Option<String>,
    error_detail: Option<ErrorDetailResponse>,
    created_at: DateTime<Utc>,
    activities: Vec<ActivityResponse>,
    attachments: Vec<AttachmentResponse>,
    /// A user message written in a chat app: its platform.
    via: Option<String>,
    activity_total: i64,
}
impl From<AssistantMessage> for MessageResponse {
    fn from(row: AssistantMessage) -> Self {
        Self {
            platform_origin: row
                .platform_origin
                .as_ref()
                .map(PlatformTurnOriginResponse::from),
            recovery: None,
            feedback: None,
            steering: row
                .steering
                .as_deref()
                .map(steering::SteeringResponse::from),
            voice: row.voice.map(VoiceTranscriptResponse::from),
            execution_pending: row.execution_pending,
            continuation_of: row.continuation_of,
            id: row.id,
            seq: row.seq,
            turn_id: row.turn_id,
            role: row.role,
            text: row.text,
            status: row.status,
            error_code: row.error_code,
            upstream_error_code: row.upstream_error_code,
            error_detail: row.error_detail.map(ErrorDetailResponse::from),
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
            activity_total: row.activity_total,
        }
    }
}
#[derive(Serialize)]
pub struct RecoveryResponse {
    status: &'static str,
    reason: &'static str,
    text: Option<String>,
    attachments: Vec<String>,
}
impl From<engine::RecoveryPreview> for RecoveryResponse {
    fn from(preview: engine::RecoveryPreview) -> Self {
        Self {
            status: preview.status,
            reason: preview.reason,
            text: preview.text,
            attachments: preview.attachments.into_iter().map(|a| a.label).collect(),
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
    card: Option<Uuid>,
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
    let mut acknowledgements = acknowledgements::history_for_messages(
        &state.db,
        &user_id,
        &id,
        rows.first().map(|message| message.created_at),
    )
    .await?;
    if let Some(card) = query.card {
        acknowledgements::include_card(
            &state.db,
            &user_id,
            &id,
            &card.to_string(),
            &mut acknowledgements,
        )
        .await?;
    }
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
    let agent_available = agents.iter().any(|agent| {
        agent_id
            .as_deref()
            .map_or_else(|| agent.is_nyxbot(), |id| agent.id == id)
            && agent.destroyed_at.is_none()
    });
    let recovery = engine::recovery_previews(
        &state.db,
        &user_id,
        &id,
        &conversation,
        &rows,
        agent_available,
    )
    .await?;
    let chats = super::nyxbot::chats::thread_details(&state, &user_id, &[&conversation]).await?;
    let mut conversation = ConversationResponse::from(conversation)
        .with_agent(agent_id.as_deref(), &agents)
        .with_chat(&chats);
    conversation.pending_acknowledgements = acknowledgements
        .iter()
        .filter(|ack| ack.status == "pending")
        .count() as u32;
    let mut messages: Vec<MessageResponse> = rows.into_iter().map(Into::into).collect();
    for message in &mut messages {
        message.recovery = recovery.get(&message.id).cloned().map(Into::into);
    }
    let ids: Vec<_> = messages.iter().map(|m| m.id.clone()).collect();
    let feedback =
        crate::services::assistant_feedback_service::states_for_messages(&state.db, &user_id, &ids)
            .await?;
    super::assistant_feedback::attach(&mut messages, feedback, |m| {
        (m.id.as_str(), &mut m.feedback)
    });
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

/// Refresh one failed reply's recovery decision before any execution attempt.
pub async fn recovery(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, message_id)): Path<(String, String)>,
) -> AppResult<Json<RecoveryResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    let preview = engine::recovery_for_message(&state.db, &user_id, &id, &message_id).await?;
    Ok(Json(preview.into()))
}

#[derive(Serialize)]
pub struct CheckpointActionResponse {
    pub label: String,
    pub turn_id: String,
}

/// The working context a conversation would resume from: its durable
/// checkpoint (absent until a turn settles after checkpoints shipped) plus
/// the live turn's running actions. Metadata and bounded excerpts only.
#[derive(Serialize)]
pub struct ContextResponse {
    pub conversation_id: String,
    pub agent_id: Option<String>,
    pub binding_state: &'static str,
    pub reset_reason: Option<String>,
    pub reset_at: Option<DateTime<Utc>>,
    pub active_turn_id: Option<String>,
    pub pending_actions: Vec<String>,
    pub checkpoint: Option<CheckpointResponse>,
}

#[derive(Serialize)]
pub struct CheckpointResponse {
    pub turn_id: String,
    pub status: String,
    pub error_code: Option<String>,
    pub goal: Option<String>,
    pub completed_actions: Vec<CheckpointActionResponse>,
    pub failed_actions: Vec<CheckpointActionResponse>,
    pub output: Option<String>,
    pub last_successful_turn_id: Option<String>,
    pub last_successful_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl From<crate::models::assistant_checkpoint::AssistantCheckpoint> for CheckpointResponse {
    fn from(row: crate::models::assistant_checkpoint::AssistantCheckpoint) -> Self {
        let actions = |actions: Vec<crate::models::assistant_checkpoint::CheckpointAction>| {
            actions
                .into_iter()
                .map(|action| CheckpointActionResponse {
                    label: action.label,
                    turn_id: action.turn_id,
                })
                .collect()
        };
        Self {
            turn_id: row.turn_id,
            status: row.status,
            error_code: row.error_code,
            goal: row.goal,
            completed_actions: actions(row.completed_actions),
            failed_actions: actions(row.failed_actions),
            output: row.output,
            last_successful_turn_id: row.last_successful_turn_id,
            last_successful_at: row.last_successful_at,
            updated_at: row.updated_at,
        }
    }
}

impl From<engine::CheckpointSummary> for ContextResponse {
    fn from(summary: engine::CheckpointSummary) -> Self {
        Self {
            active_turn_id: engine::live_turn(&summary.conversation, Utc::now())
                .map(|turn| turn.turn_id.clone()),
            conversation_id: summary.conversation.id,
            agent_id: summary.conversation.agent_id,
            binding_state: summary.binding_state,
            reset_reason: summary.conversation.context_reset_reason,
            reset_at: summary.conversation.context_reset_at,
            pending_actions: summary.pending_actions,
            checkpoint: summary.checkpoint.map(Into::into),
        }
    }
}

pub async fn context(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ContextResponse>> {
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    let summary = engine::checkpoint_summary(&state.db, &user_id, &id).await?;
    Ok(Json(summary.into()))
}

/// The owner's explicit context reset; 409 `turn_active` while a turn runs.
pub async fn reset_context(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ContextResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    engine::reset_context(&state.db, &user_id, &id).await?;
    let summary = engine::checkpoint_summary(&state.db, &user_id, &id).await?;
    Ok(Json(summary.into()))
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
    authored_skill: Option<Value>,
    continuation_owner: Option<&'static str>,
    continuation_receipt_id: Option<String>,
    trigger_run_id: Option<String>,
    id: String,
    kind: String,
    status: String,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<String>,
    /// `user` or `orchestrator` (a subagent's request its orchestrator decides).
    decider: String,
    decided_by: Option<String>,
    reason: Option<String>,
    /// Action cards: risk class, the access mode that asked and why.
    #[serde(skip_serializing_if = "Option::is_none")]
    confirmation: Option<crate::models::assistant_acknowledgement::ActionConfirmation>,
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
            continuation_owner: (row.continuation_receipt_id.is_some() || row.authored_skill.is_some()).then_some("server"),
            authored_skill: row.authored_skill.map(|s| json!({"agent_id":s.agent_id,"proposal_id":s.proposal_id,"revision":s.revision,"skills_revision":s.skills_revision})),
            continuation_receipt_id: row.continuation_receipt_id,
            id: row.id,
            kind: row.kind,
            status: row.status,
            summary: row.summary,
            link: row.link,
            trigger_run_id: row.trigger_run_id,
            decider: row.decider,
            decided_by: row.decided_by,
            reason: row.reason,
            confirmation: row.confirmation,
            service_slug: row.service_slug,
            service_name: row.service_name,
            tool_name: row.tool_name,
            created_at: row.created_at,
            decided_at: row.decided_at,
            expires_at: row.expires_at,
        }
    }
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Deny,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgementDecision {
    decision: Decision,
    #[serde(default)]
    accept_reduced_scope: bool,
}

pub async fn decide_acknowledgement(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, ack_id)): Path<(String, String)>,
    Json(body): Json<AcknowledgementDecision>,
) -> AppResult<Json<AcknowledgementResponse>> {
    let user = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user).await?;
    if let Some(card) = state.db.collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .find_one(doc! {"_id":&ack_id,"user_id":&user,"conversation_id":&id,"authored_skill":{"$ne":bson::Bson::Null}}).await? {
        super::login_client_context::require_first_party_human(&auth)?;
        let card = Box::pin(super::assistant_agent_learning::decide_authored_with_scope(&state, &auth, card, matches!(body.decision, Decision::Allow), body.accept_reduced_scope)).await?;
        return Ok(Json(card.into()));
    }
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
    if matches!(body.decision, Decision::Deny) {
        Box::pin(super::assistant_team::owner_denial_decided(
            &state, &user, &row,
        ))
        .await;
    }
    if matches!(body.decision, Decision::Allow)
        && let Some(action) = row.machine_context.as_ref()
    {
        super::machine_access::apply_human_context_action(&state, &user, action).await?;
    }
    Ok(Json(row.into()))
}

/// Retired conversation consent mode. Kept so older clients get a
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
            "message": "Conversation consent modes were retired. Change the permission mode in NyxBot settings or the composer.",
        })),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameRequest {
    title: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequest {
    model: String,
}

pub async fn change_model(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ModelRequest>,
) -> AppResult<Json<ConversationResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    require_available_model(&state, &auth, &body.model).await?;
    let (before, row) = engine::change_model(&state.db, &owner, &id, &body.model).await?;
    engine::audit_model_change(
        &state.db,
        &crate::services::audit_service::AuditActor::from_auth_user(&auth),
        &row,
        &before,
    )
    .await;
    Ok(Json(ConversationResponse::from(row)))
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
    super::mcp_transport::async_operations::cancel_conversation(&state, &user_id, &id).await?;
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
    // Preserve the existing voice live-slot rule before recording a stop. The
    // delete service repeats this check in its transaction for race safety.
    if state
        .db
        .collection::<mongodb::bson::Document>(
            crate::models::assistant_voice_session::COLLECTION_NAME,
        )
        .find_one(doc! {"user_id": &user_id, "conversation_id": &id, "live_slot": true})
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "End the voice call before deleting this conversation".into(),
        ));
    }
    // Deletion is an explicit Stop followed by removal. Stop every related
    // hidden voice thread as well; workers that settle later see a missing row
    // and quietly become no-ops.
    let mut related = vec![target];
    let mut child_cursor = state
        .db
        .collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find(doc! {"user_id": &user_id, "voice_parent_conversation_id": &id})
        .await?;
    while let Some(child) = child_cursor.next().await {
        related.push(child?);
    }
    for row in &related {
        engine::request_stop(&state.db, &user_id, &row.id).await?;
        super::mcp_transport::async_operations::cancel_conversation(&state, &user_id, &row.id)
            .await?;
        super::machine_cancel::conversation(&state, &user_id, &row.id).await?;
    }
    let mut credentials_by_id = HashMap::new();
    for member in &related {
        if let Some(credential) = credentials::load_for_conversation(
            &state.db,
            &state.encryption_keys,
            &user_id,
            &member.id,
        )
        .await?
        {
            credentials_by_id.insert(member.id.clone(), credential);
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
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}
type ModelCache = HashMap<String, (Instant, Vec<ModelResponse>)>;
static MODELS: LazyLock<Mutex<ModelCache>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Keep upstream display metadata; the routing alias is not a model name.
fn model_response(row: &Value) -> Option<ModelResponse> {
    let id = row["id"].as_str().filter(|id| engine::valid_model(id))?;
    let label = ["display_name", "label", "name"]
        .into_iter()
        .filter_map(|key| row[key].as_str())
        .map(str::trim)
        .find(|name| !name.is_empty());
    Some(ModelResponse {
        id: id.into(),
        label: label
            .unwrap_or_else(|| {
                if id == engine::DEFAULT_MODEL {
                    "Default model"
                } else {
                    id.trim_start_matches("nyxagent/")
                }
            })
            .into(),
        available: row["available"].as_bool() != Some(false),
        description: row["description"].as_str().map(str::to_owned),
    })
}

/// The models NyxAgent currently offers this owner, or `None` when its list
/// cannot be read. Cached per owner for a minute: availability follows the
/// owner's assistant credential, so owners never share an entry.
pub(crate) async fn live_models(
    state: &AppState,
    auth: &AuthUser,
    policy: Option<BillingRoutePolicy>,
) -> Option<Vec<ModelResponse>> {
    let user_id = auth.user_id.to_string();
    let cache_key = format!("{}:{user_id}", state.db.name());
    if let Some((time, models)) = MODELS.lock().await.get(&cache_key)
        && time.elapsed() < Duration::from_secs(60)
    {
        return Some(models.clone());
    }
    let loaded: AppResult<Vec<ModelResponse>> = async {
        let credential = credentials::load_existing(&state.db, &state.encryption_keys, &user_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Assistant credential not provisioned".into()))?;
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            proxy(
                state,
                auth,
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
            .filter_map(model_response)
            .take(50)
            .collect();
        if rows.is_empty() {
            return Err(AppError::Internal("Assistant profiles unavailable".into()));
        }
        Ok(rows)
    }
    .await;
    let rows = loaded.ok()?;
    let mut cache = MODELS.lock().await;
    if cache.len() >= 64 {
        cache.retain(|_, (time, _)| time.elapsed() < Duration::from_secs(60));
    }
    cache.insert(cache_key, (Instant::now(), rows.clone()));
    Some(rows)
}

/// Validate a model selection against the live list. When NyxAgent's list is
/// unreachable a well-formed model is accepted; a turn that NyxAgent then
/// refuses reports `model_unavailable` with the same guidance.
pub(crate) async fn require_available_model(
    state: &AppState,
    auth: &AuthUser,
    model: &str,
) -> AppResult<()> {
    if !engine::valid_model(model) {
        return Err(AppError::AssistantModelUnavailable);
    }
    // The same metered proxy classification as the models route itself.
    match live_models(state, auth, Some(SERVER_TURN_POLICY)).await {
        Some(rows) if !rows.iter().any(|row| row.id == model && row.available) => {
            Err(AppError::AssistantModelUnavailable)
        }
        _ => Ok(()),
    }
}

pub async fn models(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Request<Body>,
) -> AppResult<Json<Vec<ModelResponse>>> {
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    let policy = request.extensions().get::<BillingRoutePolicy>().copied();
    Ok(Json(
        live_models(&state, &auth, policy).await.unwrap_or_else(|| {
            vec![ModelResponse {
                id: engine::DEFAULT_MODEL.into(),
                label: "Default model".into(),
                available: true,
                description: None,
            }]
        }),
    ))
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
    proxy_raw(
        state,
        auth,
        credential.raw_key.as_str(),
        method,
        path,
        body,
        turn_id,
        policy,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn proxy_raw(
    state: &AppState,
    auth: &AuthUser,
    raw_key: &str,
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
            if path == "v1/responses" {
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
    let mut headers = vec![("authorization".into(), format!("Bearer {}", raw_key))];
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

struct HttpNyxAgentEgress {
    state: AppState,
    auth: AuthUser,
}

pub(crate) fn http_egress(
    state: AppState,
    auth: AuthUser,
) -> std::sync::Arc<dyn crate::services::platform_manager_contract::NyxAgentEgress> {
    std::sync::Arc::new(HttpNyxAgentEgress { state, auth })
}

#[async_trait::async_trait]
impl crate::services::platform_manager_contract::NyxAgentEgress for HttpNyxAgentEgress {
    async fn request(
        &self,
        context: &crate::services::platform_manager_contract::NyxAgentExecutionContext,
        credential: &crate::services::platform_manager_contract::RuntimeCredential,
        request: crate::services::platform_manager_contract::NyxAgentRequest,
    ) -> AppResult<Response> {
        let crate::services::platform_manager_contract::NyxAgentExecutionContext::Owner {
            user_id,
            ..
        } = context
        else {
            return Err(AppError::Forbidden("Owner execution required".into()));
        };
        if user_id != &self.auth.user_id.to_string() {
            return Err(AppError::Forbidden("Owner execution required".into()));
        }
        proxy_raw(
            &self.state,
            &self.auth,
            credential.raw_key.as_str(),
            request.method.as_str(),
            &request.relative_path,
            request.body,
            (!request.idempotency_key.is_empty()).then_some(request.idempotency_key.as_str()),
            request.billing_policy,
        )
        .await
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
    if !input.attachment_ids.is_empty() || input.retry_of.is_some() {
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
    let mut row = if voice_request_id.is_some() {
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
    if let Some(watch) = crate::services::async_service_operation::bound(&state.db, &row).await? {
        crate::services::async_service_operation::apply_delivery(&mut row, &watch);
    }
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
    tokio::spawn(
        run_turn(
            state.clone(),
            auth,
            row.clone(),
            text,
            credential,
            policy,
            permit,
            Events {
                sender,
                cursor: 0,
                reset_notified: false,
            },
        )
        .in_current_span(),
    );
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
    credential: AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    permit: DirectChatPermit,
    events: Events,
) {
    let egress = if row
        .active_turn
        .as_ref()
        .is_some_and(|turn| turn.platform_origin.is_some())
    {
        crate::services::assistant_nyxagent_egress::producer(
            state.clone(),
            crate::services::platform_personal_binding_service::producer(state.clone()),
        )
    } else {
        http_egress(state.clone(), auth.clone())
    };
    let settled = crate::services::assistant_nyxagent_runtime::run_owner_turn(
        state.clone(),
        egress,
        row.clone(),
        text,
        credential,
        policy,
        permit,
        events,
    )
    .await;
    if let Some(settled) = settled {
        tracing::debug!(conversation_id = %row.id, message_id = %settled.message_id, "Assistant reply settled");
        if settled.error.is_none()
            && row.message_count == 1
            && crate::services::assistant_title_service::eligible(&row)
        {
            super::assistant_titles::spawn(state.clone(), auth, row.id.clone());
        }
        if let Some(turn) = row.active_turn.as_ref() {
            let reply = crate::services::platform_manager_contract::ReplyRef {
                turn: crate::services::platform_manager_contract::TurnRef {
                    conversation_id: row.id.clone(),
                    turn_id: turn.turn_id.clone(),
                },
                reply_id: settled.message_id.clone(),
            };
            if let Err(error) = crate::services::platform_manager_delivery::saved_personal_reply(
                &state, &row, &reply,
            )
            .await
            {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Saved manager event reply delivery deferred or refused"
                );
            }
        }
        super::assistant_team::after_turn_boxed(state, row, settled.text, settled.error).await;
    }
}

#[cfg(test)]
#[path = "assistant_nyxagent_tests.rs"]
mod tests;

#[path = "assistant_nyxagent_steering.rs"]
mod steering;
pub use steering::{capabilities, steer};

#[cfg(test)]
use crate::services::assistant_nyxagent_runtime::images;

#[cfg(test)]
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
    instruction_context: &mut Option<crate::services::assistant_instruction_context::Prepared>,
) -> Result<engine::TurnResult, TurnError> {
    let egress = http_egress(state.clone(), auth.clone());
    let context = crate::services::assistant_nyxagent_runtime::owner_context(row)
        .map_err(|_| TurnError::new("assistant_unavailable"))?;
    crate::services::assistant_nyxagent_runtime::execute_owner_turn(
        state,
        egress.as_ref(),
        &context,
        row,
        text,
        credential,
        policy,
        events,
        block_id,
        partial,
        instruction_context,
    )
    .await
}
