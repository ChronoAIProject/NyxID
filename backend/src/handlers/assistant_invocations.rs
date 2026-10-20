//! Bounded polling adapter for first-party owner invocations.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{assistant_invocation_service as invocations, assistant_nyxagent as engine},
};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationRequest {
    pub idempotency_key: String,
    pub text: String,
    pub conversation_id: Option<String>,
    pub agent_id: Option<String>,
    pub model: Option<String>,
}
fn valid_key(key: &str) -> bool {
    Uuid::parse_str(key).is_ok_and(|id| {
        id.get_variant() == uuid::Variant::RFC4122
            && id.get_version_num() == 4
            && id.to_string() == key
    })
}
fn parse(bytes: &[u8], owner: &str) -> AppResult<(InvocationRequest, engine::TurnStart, String)> {
    let input: InvocationRequest = serde_json::from_slice(bytes)
        .map_err(|_| AppError::ValidationError("Invalid invocation request".into()))?;
    if !valid_key(&input.idempotency_key) {
        return Err(AppError::ValidationError(
            "idempotency_key must be a canonical UUID v4".into(),
        ));
    }
    if input.conversation_id.is_some() && (input.agent_id.is_some() || input.model.is_some()) {
        return Err(AppError::ValidationError(
            "agent_id and model select new conversations only".into(),
        ));
    }
    let request_bytes = serde_json::to_vec(&json!({
        "text": input.text,
        "conversation_id": input.conversation_id,
        "agent_id": input.agent_id,
        "model": input.model,
    }))
    .map_err(|_| AppError::Internal("Invalid invocation".into()))?;
    let request = engine::parse_turn(&request_bytes)?;
    let material = json!({
        "domain": "nyxid:assistant-invocation:v1",
        "owner": owner,
        "key": input.idempotency_key,
        "text": input.text,
        "conversation_id": input.conversation_id,
        "agent_id": input.agent_id,
        "model": input.model,
    })
    .to_string();
    let digest =
        crate::services::assistant_action_receipts::fingerprint_sensitive_material(&material);
    Ok((input, engine::TurnStart::from(&request), digest))
}

#[derive(Serialize)]
pub struct ReplyResponse {
    pub conversation_id: String,
    pub turn_id: String,
    pub message_id: String,
    pub status: String,
    pub error_code: Option<String>,
    pub text: String,
    pub text_truncated: bool,
}
#[derive(Serialize)]
pub struct RequiredActionResponse {
    pub id: String,
    pub conversation_id: String,
    pub kind: String,
    pub expires_at: DateTime<Utc>,
}
#[derive(Serialize)]
pub struct InvocationResponse {
    pub invocation_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub status: String,
    pub terminal: bool,
    pub created_at: DateTime<Utc>,
    pub cancel_requested: bool,
    pub turn_count: i64,
    pub running_turns: u64,
    pub background_operations: u64,
    pub pending_continuations: u64,
    pub required_actions: Vec<RequiredActionResponse>,
    pub required_actions_truncated: bool,
    pub replies: Vec<ReplyResponse>,
    pub replies_truncated: bool,
    pub error_code: Option<String>,
    pub retry_after_ms: u32,
}

impl From<invocations::InvocationSnapshot> for InvocationResponse {
    fn from(row: invocations::InvocationSnapshot) -> Self {
        Self {
            invocation_id: row.invocation_id,
            conversation_id: row.conversation_id,
            turn_id: row.turn_id,
            status: row.status,
            terminal: row.terminal,
            created_at: row.created_at,
            cancel_requested: row.cancel_requested,
            turn_count: row.turn_count,
            running_turns: row.running_turns,
            background_operations: row.background_operations,
            pending_continuations: row.pending_continuations,
            required_actions_truncated: row.required_actions_truncated,
            required_actions: row
                .required_actions
                .into_iter()
                .map(|c| RequiredActionResponse {
                    id: c.id,
                    conversation_id: c.conversation_id,
                    kind: c.kind,
                    expires_at: c.expires_at,
                })
                .collect(),
            replies_truncated: row.replies_truncated,
            replies: row
                .replies
                .into_iter()
                .map(|r| ReplyResponse {
                    conversation_id: r.conversation_id,
                    turn_id: r.turn_id,
                    message_id: r.message_id,
                    status: r.status,
                    error_code: r.error_code,
                    text: r.text,
                    text_truncated: r.text_truncated,
                })
                .collect(),
            error_code: row.error_code,
            retry_after_ms: row.retry_after_ms,
        }
    }
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Request<Body>,
) -> AppResult<(StatusCode, Json<InvocationResponse>)> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    if !crate::services::feature_flag_service::personal_flag_enabled(
        &state.db,
        &owner,
        crate::services::feature_flag_service::ASSISTANT_INVOCATIONS_FLAG_KEY,
    )
    .await?
    {
        return Err(AppError::NotFound(
            "Assistant invocation admission is disabled".into(),
        ));
    }
    let bytes = super::body_limit::read_body(
        request.into_body(),
        engine::MAX_REQUEST_BYTES,
        "Assistant invocation",
    )
    .await?;
    let (input, mut start, digest) = parse(&bytes, &owner)?;
    // Replays bypass limiter/provisioning and retain their receipt after deletion.
    match invocations::get(&state.db, &owner, &input.idempotency_key).await {
        Ok(row) => {
            invocations::same_request(row, &digest)?;
            return Ok((
                StatusCode::OK,
                Json(
                    invocations::snapshot(&state.db, &owner, &input.idempotency_key)
                        .await?
                        .into(),
                ),
            ));
        }
        Err(AppError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    if let Some(id) = start.conversation_id.as_deref() {
        let row = engine::get(&state.db, &owner, id).await?;
        invocations::require_owner_conversation(&row)?;
        let agent =
            crate::services::assistant_team_service::agent_for_conversation(&state.db, &row)
                .await?;
        if agent.user_id != owner {
            return Err(invocations::not_found());
        }
    }
    let permit = state.direct_chat_limiter.try_acquire(&owner).await?;
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
    if let Some(model) = input.model.as_deref() {
        super::assistant_nyxagent::require_available_model(&state, &auth, model).await?;
    }
    let admission = Box::pin(invocations::admit(
        &state.db,
        &owner,
        &input.idempotency_key,
        &digest,
        &start,
        &state.encryption_keys,
    ))
    .await?;
    let fresh = admission.claim.is_some();
    if let Some(claim) = admission.claim {
        // No await between acknowledged commit and handing off the admitted
        // credential/permit. A replay never starts another detached worker.
        let (sender, _) = tokio::sync::broadcast::channel(256);
        let state2 = state.clone();
        let text = engine::turn_input(&claim.conversation, &start);
        let replaced = start.conversation_id.is_some();
        let owner = owner.clone();
        tokio::spawn(async move {
            claim.log_reclaimed();
            crate::services::assistant_agent_credential_service::audit_provision(
                &state2.db,
                &owner,
                &claim.conversation.id,
                &claim.credential,
                replaced,
            )
            .await;
            let _ = crate::services::audit_service::log_actor_event(
                state2.db.clone(),
                &crate::services::audit_service::AuditActor::from_auth_user(&auth),
                "assistant_invocation_admitted",
                Some(json!({
                    "invocation_id": input.idempotency_key,
                    "conversation_id": claim.conversation.id,
                    "turn_id": claim.conversation.active_turn.as_ref().map(|turn| &turn.turn_id),
                })),
            )
            .await;
            super::assistant_nyxagent::run_turn(
                state2,
                auth,
                claim.conversation,
                text,
                claim.credential,
                Some(super::assistant_nyxagent::SERVER_TURN_POLICY),
                permit,
                crate::services::assistant_nyxagent_runtime::Events {
                    sender,
                    cursor: 0,
                    reset_notified: false,
                },
            )
            .await;
        });
    }
    Ok((
        if fresh {
            StatusCode::ACCEPTED
        } else {
            StatusCode::OK
        },
        Json(
            invocations::snapshot(&state.db, &owner, &admission.invocation.idempotency_key)
                .await?
                .into(),
        ),
    ))
}
pub async fn show(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
) -> AppResult<Json<InvocationResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    if !valid_key(&key) {
        return Err(invocations::not_found());
    }
    Ok(Json(
        invocations::snapshot(&state.db, &owner, &key).await?.into(),
    ))
}
pub async fn cancel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
) -> AppResult<Json<InvocationResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    if !valid_key(&key) {
        return Err(invocations::not_found());
    }
    invocations::cancel(&state, &owner, &key).await?;
    let _ = crate::services::audit_service::log_actor_event(
        state.db.clone(),
        &crate::services::audit_service::AuditActor::from_auth_user(&auth),
        "assistant_invocation_cancel_requested",
        Some(json!({"invocation_id":key})),
    )
    .await;
    Ok(Json(
        invocations::snapshot(&state.db, &owner, &key).await?.into(),
    ))
}

#[cfg(test)]
#[path = "assistant_invocations_tests.rs"]
mod tests;
