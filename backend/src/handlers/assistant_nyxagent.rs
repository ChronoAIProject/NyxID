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
pub struct ActiveTurnResponse {
    turn_id: String,
    started_at: DateTime<Utc>,
}
#[derive(Serialize)]
pub struct ConversationResponse {
    id: String,
    title: String,
    model: String,
    access_mode: crate::models::assistant_conversation::AccessMode,
    created_at: DateTime<Utc>,
    last_message_at: DateTime<Utc>,
    message_count: i64,
    pending_acknowledgements: u32,
    active_turn: Option<ActiveTurnResponse>,
    context_reset_at: Option<DateTime<Utc>>,
}
impl From<AssistantConversation> for ConversationResponse {
    fn from(row: AssistantConversation) -> Self {
        let active_turn = engine::live_turn(&row, Utc::now()).map(|turn| ActiveTurnResponse {
            turn_id: turn.turn_id.clone(),
            started_at: turn.started_at,
        });
        Self {
            id: row.id,
            title: row.title,
            model: row.model,
            access_mode: row.access_mode,
            created_at: row.created_at,
            last_message_at: row.updated_at,
            message_count: row.message_count,
            pending_acknowledgements: 0,
            active_turn,
            context_reset_at: row.context_reset_at,
        }
    }
}
#[derive(Serialize)]
pub struct MessageResponse {
    id: String,
    seq: i64,
    turn_id: String,
    role: String,
    text: String,
    status: String,
    error_code: Option<String>,
    created_at: DateTime<Utc>,
}
impl From<AssistantMessage> for MessageResponse {
    fn from(row: AssistantMessage) -> Self {
        Self {
            id: row.id,
            seq: row.seq,
            turn_id: row.turn_id,
            role: row.role,
            text: row.text,
            status: row.status,
            error_code: row.error_code,
            created_at: row.created_at,
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageQuery {
    limit: Option<i64>,
    cursor: Option<String>,
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
    let mut rows = engine::list(&state.db, &user_id, limit + 1, query.cursor.as_deref()).await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = more.then(|| engine::index_cursor(rows.last().expect("nonempty page")));
    let counts = acknowledgements::pending_counts(
        &state.db,
        &user_id,
        &rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
    )
    .await?;
    Ok(Json(IndexResponse {
        conversations: rows
            .into_iter()
            .map(|row| {
                let count = counts.get(&row.id).copied().unwrap_or(0);
                let mut dto = ConversationResponse::from(row);
                dto.pending_acknowledgements = count;
                dto
            })
            .collect(),
        next_cursor,
    }))
}
#[derive(Serialize)]
pub struct HistoryResponse {
    conversation: ConversationResponse,
    messages: Vec<MessageResponse>,
    acknowledgements: Vec<AcknowledgementResponse>,
    before_seq: Option<i64>,
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
    let (conversation, mut rows) =
        engine::history_page(&state.db, &user_id, &id, limit + 1, query.before_seq).await?;
    let more = rows.len() > limit as usize;
    if more {
        rows.remove(0);
    }
    let before_seq = more.then(|| rows[0].seq);
    let acknowledgements = acknowledgements::history(&state.db, &user_id, &id).await?;
    let mut conversation = ConversationResponse::from(conversation);
    conversation.pending_acknowledgements = acknowledgements
        .iter()
        .filter(|ack| ack.status == "pending")
        .count() as u32;
    Ok(Json(HistoryResponse {
        conversation,
        acknowledgements: acknowledgements.into_iter().map(Into::into).collect(),
        messages: rows.into_iter().map(Into::into).collect(),
        before_seq,
    }))
}
#[derive(Serialize)]
pub struct AcknowledgementResponse {
    id: String,
    kind: String,
    status: String,
    summary: String,
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
            id: row.id,
            kind: row.kind,
            status: row.status,
            summary: row.summary,
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
    Ok(Json(row.into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessModeRequest {
    access_mode: crate::models::assistant_conversation::AccessMode,
}

pub async fn change_access_mode(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<AccessModeRequest>,
) -> AppResult<Json<ConversationResponse>> {
    let user = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user).await?;
    let (old, row) = crate::services::assistant_access_mode_service::change(
        &state.db,
        &user,
        &id,
        body.access_mode,
    )
    .await?;
    crate::services::assistant_access_mode_service::audit_change(
        &state.db,
        &user,
        &id,
        old,
        row.access_mode,
    )
    .await;
    Ok(Json(row.into()))
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
    Ok(Json(
        engine::rename(&state.db, &user_id, &id, &body.title)
            .await?
            .into(),
    ))
}
pub async fn stop(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let user_id = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user_id).await?;
    engine::request_stop(&state.db, &user_id, &id).await?;
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
    engine::get(&state.db, &user_id, &id).await?;
    let credential =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, &user_id, &id)
            .await?;
    let row = engine::delete(&state.db, &user_id, &id).await?;
    let policy = request.extensions().get::<BillingRoutePolicy>().copied();
    if let Some(session_id) = row.nyxagent_session_id {
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
    if !engine::row_contract(Some(&service)).valid() {
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
    // Ownership is checked before provisioning or touching a credential.
    if let Some(id) = &input.conversation_id {
        engine::get(&state.db, &user_id, id).await?;
    }
    let permit = state.direct_chat_limiter.try_acquire(&user_id).await?;
    let row = engine::begin_turn(&state.db, &user_id, &input, &state.encryption_keys).await?;
    let credential =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, &user_id, &row.id)
            .await?
            .ok_or_else(|| AppError::NotFound("Assistant credential not found".into()))?;
    let policy = parts.extensions.get::<BillingRoutePolicy>().copied();
    let (sender, receiver) = broadcast::channel(256);
    // Subscribe before spawning: even an immediate completion is observable.
    tokio::spawn(run_turn(
        state,
        auth,
        row,
        input.text,
        credential,
        policy,
        permit,
        Events { sender, cursor: 0 },
    ));
    Ok(subscribe_events(receiver))
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
        let execution = execute_turn(
            &state,
            &auth,
            &row,
            &text,
            &mut credential,
            policy,
            &mut events,
            &block_id,
            &mut partial,
        );
        tokio::pin!(execution);
        tokio::select! {
            result = &mut execution => result,
            () = permit.cancelled() => Err(TurnError::new("assistant_unavailable")),
            error = watch_stop(&state, &row, &turn_id) => Err(error),
            () = tokio::time::sleep(Duration::from_secs(engine::TURN_EXECUTION_SECS)) => {
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
    // Never echo the system-managed credential, even if an upstream reflects it.
    result.text = result
        .text
        .replace(credential.raw_key.as_str(), "[redacted]");
    complete_turn(
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
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = AppResult<Option<TurnError>>>,
{
    let deadline = tokio::time::Instant::now() + settle_for;
    let mut attempt = 0u32;
    let error = loop {
        match tokio::time::timeout_at(deadline, persist()).await {
            Ok(Ok(error)) => break error,
            // Deleted conversations and reclaimed turns must never be recreated.
            Ok(Err(AppError::NotFound(_))) => return,
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
    let history = engine::messages(
        &state.db,
        &row.user_id,
        &row.id,
        20,
        Some(row.message_count),
    )
    .await
    .map_err(|_| TurnError::new("assistant_unavailable"))?;
    let mut binding = row.nyxagent_session_id.clone();
    let mut prompt = if binding.is_none() && row.context_reset_reason.is_some() {
        events.notice();
        engine::instructions(&history)
    } else {
        engine::SYSTEM_PROMPT.into()
    };
    let mut recovery = engine::Recovery::default();
    loop {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let response = tokio::time::timeout_at(
            deadline,
            proxy(
                state,
                auth,
                credential,
                "POST",
                "v1/responses",
                Some(engine::upstream_body(
                    &row.model,
                    text,
                    binding.as_deref(),
                    &prompt,
                )),
                Some(turn_id),
                policy,
            ),
        )
        .await
        .map_err(|_| TurnError::new("first_byte_timeout"))?
        .map_err(|_| TurnError::new("assistant_unavailable"))?;
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
            let code = envelope["error"]["code"].as_str().unwrap_or_default();
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
                    prompt = engine::instructions(&history);
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
                    prompt = engine::instructions(&history);
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
        let mut emitted_bytes = 0;
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
            *partial = decoder
                .text
                .replace(credential.raw_key.as_str(), "[redacted]");
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
        return Ok(result);
    }
}

#[cfg(test)]
#[path = "assistant_nyxagent_tests.rs"]
mod tests;
