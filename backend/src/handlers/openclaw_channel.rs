use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::channel_bots::hash_conversation_id;
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, openclaw_channel_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event, should_sample_event};

// --- Request / Response types ---

#[derive(Debug, Serialize)]
pub struct ChannelWebhookResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nyxid_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nyxid_user_email: Option<String>,
    pub available_providers: Vec<String>,
    pub openclaw_connected: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateMappingRequest {
    pub channel: String,
    pub channel_user_id: String,
}

#[derive(Debug, Serialize)]
pub struct MappingResponse {
    pub status: String,
    pub message: String,
    /// The webhook secret for this mapping. Only returned at creation time.
    /// Configure this in your OpenClaw channel plugin.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
}

// --- Handlers ---

/// POST /api/v1/integrations/openclaw/channel
///
/// Webhook endpoint called by OpenClaw when a message arrives on a channel.
/// Each user's OpenClaw instance signs requests with its own per-mapping
/// webhook secret. The handler:
/// 1. Parses the message to identify channel + channel_user_id
/// 2. Looks up the mapping to find the stored secret hash
/// 3. Verifies the webhook secret and HMAC signature
/// 4. Returns identity context
///
/// TODO(phase-6): Add dual-path lookup here. Before the legacy
/// `openclaw_channel_mappings` lookup below, try resolving through the new
/// channel relay system via `channel_routing_service::resolve_agent()`. If a
/// matching route is found in `channel_conversations`, forward the message to
/// the agent's callback URL and return early. Fall back to the legacy path
/// when no relay route exists. This keeps backward compatibility while
/// allowing users to migrate to the unified channel bot relay incrementally.
pub async fn handle_channel_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ChannelWebhookResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Extract required headers
    let signature = headers
        .get("x-openclaw-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Missing X-OpenClaw-Signature header"
                })),
            )
        })?;

    let webhook_secret = headers
        .get("x-openclaw-webhook-secret")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Missing X-OpenClaw-Webhook-Secret header"
                })),
            )
        })?;

    // Parse the message body to get channel + channel_user_id for mapping lookup
    let message: openclaw_channel_service::OpenClawChannelMessage = serde_json::from_slice(&body)
        .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Invalid message format: {e}")
            })),
        )
    })?;

    // Verify secret + signature against the per-user mapping
    let mapping = openclaw_channel_service::verify_webhook_for_mapping(
        &state.db,
        &message.channel,
        &message.channel_user_id,
        webhook_secret,
        &body,
        signature,
    )
    .await
    .map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "Webhook verification failed"
            })),
        )
    })?;

    let nyxid_user_id = Some(mapping.nyxid_user_id.clone());

    // Telemetry: channel.message_received is sampled at 10% per
    // docs/TELEMETRY.md §6.5. Sampling key is the conversation hash (not
    // user id): hashing on user_id would make each mapped NyxID user
    // either 100% in or 100% out of the sample. Conversation-keyed
    // sampling gives ~10% of messages per conversation and averages to
    // ~10% across users. Webhook ingress — no AuthUser / TelemetryContext,
    // so use default context and None for api_key_id.
    let distinct_id = mapping.nyxid_user_id.clone();
    let conversation_hash = hash_conversation_id(&message.channel_user_id);
    if should_sample_event("channel.message_received", &conversation_hash, 10) {
        emit_event(
            state.telemetry.as_deref(),
            &distinct_id,
            None,
            &TelemetryContext::default(),
            TelemetryEvent::ChannelMessageReceived {
                platform: format!("openclaw:{}", message.channel),
                conversation_id_hash: conversation_hash,
            },
        );
    }

    // Get available providers and check OpenClaw connection status
    let slugs =
        openclaw_channel_service::get_user_provider_slugs(&state.db, &mapping.nyxid_user_id)
            .await
            .unwrap_or_default();
    let openclaw_connected = slugs.iter().any(|s| s == "openclaw");

    // Audit log
    audit_service::log_async(
        state.db.clone(),
        nyxid_user_id.clone(),
        "openclaw_channel_message".to_string(),
        Some(serde_json::json!({
            "channel": &message.channel,
            "channel_user_id": &message.channel_user_id,
            "agent_id": &message.agent_id,
            "direction": &message.direction,
        })),
        crate::handlers::admin_helpers::extract_ip(&headers),
        crate::handlers::admin_helpers::extract_user_agent(&headers),
        None,
        None,
    );

    Ok(Json(ChannelWebhookResponse {
        status: "resolved".to_string(),
        nyxid_user_id,
        nyxid_user_email: None,
        available_providers: slugs,
        openclaw_connected,
    }))
}

/// POST /api/v1/integrations/openclaw/mappings
///
/// Create a mapping between an OpenClaw channel user and the authenticated NyxID user.
/// Returns a per-mapping webhook secret that must be configured in the user's
/// OpenClaw channel plugin. The secret is only shown once.
pub async fn create_mapping(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<CreateMappingRequest>,
) -> AppResult<Json<MappingResponse>> {
    if body.channel.is_empty() {
        return Err(AppError::ValidationError(
            "channel must not be empty".to_string(),
        ));
    }
    if body.channel.len() > 100 {
        return Err(AppError::ValidationError(
            "channel exceeds maximum length".to_string(),
        ));
    }
    if body.channel_user_id.is_empty() {
        return Err(AppError::ValidationError(
            "channel_user_id must not be empty".to_string(),
        ));
    }
    if body.channel_user_id.len() > 500 {
        return Err(AppError::ValidationError(
            "channel_user_id exceeds maximum length".to_string(),
        ));
    }

    let user_id_str = auth_user.user_id.to_string();

    let webhook_secret = openclaw_channel_service::upsert_mapping(
        &state.db,
        &body.channel,
        &body.channel_user_id,
        &user_id_str,
    )
    .await?;

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::ChannelMappingCreated {
            platform: format!("openclaw:{}", body.channel),
            conversation_id_hash: hash_conversation_id(&body.channel_user_id),
        },
    );

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "openclaw_channel_mapping_created",
        Some(serde_json::json!({
            "channel": &body.channel,
            "channel_user_id": &body.channel_user_id,
        })),
    );

    Ok(Json(MappingResponse {
        status: "created".to_string(),
        message:
            "Channel mapping created. Configure the webhook_secret in your OpenClaw channel plugin."
                .to_string(),
        webhook_secret: Some(webhook_secret),
    }))
}
