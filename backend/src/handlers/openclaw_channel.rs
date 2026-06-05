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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::openclaw_channel_service::{self, MAPPINGS_COLLECTION};
    use crate::test_utils::*;
    use axum::extract::State;
    use axum::http::HeaderValue;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    fn signature(secret: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    fn webhook_headers(secret: &str, signature: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-openclaw-webhook-secret",
            HeaderValue::from_str(secret).unwrap(),
        );
        headers.insert(
            "x-openclaw-signature",
            HeaderValue::from_str(signature).unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn test_create_mapping_success() {
        let Some(db) = connect_test_database("h_openclaw_create_ok").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(resp) = create_mapping(
            State(state),
            auth,
            TelemetryContext::default(),
            Json(CreateMappingRequest {
                channel: "whatsapp".to_string(),
                channel_user_id: "user-123".to_string(),
            }),
        )
        .await
        .unwrap();

        assert_eq!(resp.status, "created");
        assert!(resp.webhook_secret.is_some());
        let secret = resp.webhook_secret.unwrap();
        assert_eq!(secret.len(), 64);
    }

    #[tokio::test]
    async fn test_create_mapping_empty_channel_rejected() {
        let Some(db) = connect_test_database("h_openclaw_empty_chan").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let result = create_mapping(
            State(state),
            auth,
            TelemetryContext::default(),
            Json(CreateMappingRequest {
                channel: "".to_string(),
                channel_user_id: "user-123".to_string(),
            }),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_mapping_empty_channel_user_id_rejected() {
        let Some(db) = connect_test_database("h_openclaw_empty_uid").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let result = create_mapping(
            State(state),
            auth,
            TelemetryContext::default(),
            Json(CreateMappingRequest {
                channel: "telegram".to_string(),
                channel_user_id: "".to_string(),
            }),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_mapping_upsert_rotates_secret() {
        let Some(db) = connect_test_database("h_openclaw_upsert").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db.clone());

        let Json(first) = create_mapping(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(CreateMappingRequest {
                channel: "discord".to_string(),
                channel_user_id: "disc-user-1".to_string(),
            }),
        )
        .await
        .unwrap();

        let Json(second) = create_mapping(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(CreateMappingRequest {
                channel: "discord".to_string(),
                channel_user_id: "disc-user-1".to_string(),
            }),
        )
        .await
        .unwrap();

        assert_ne!(
            first.webhook_secret.unwrap(),
            second.webhook_secret.unwrap()
        );

        let count = db
            .collection::<openclaw_channel_service::OpenClawChannelMapping>(MAPPINGS_COLLECTION)
            .count_documents(mongodb::bson::doc! {
                "channel": "discord",
                "channel_user_id": "disc-user-1",
            })
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_create_mapping_rejects_length_limits_with_exact_errors() {
        let state = test_app_state_no_db().await;
        let user_id = uuid::Uuid::new_v4().to_string();

        let long_channel = create_mapping(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(CreateMappingRequest {
                channel: "c".repeat(101),
                channel_user_id: "user-123".to_string(),
            }),
        )
        .await
        .expect_err("channel length should be validated before DB access");
        assert!(matches!(
            long_channel,
            AppError::ValidationError(message) if message == "channel exceeds maximum length"
        ));

        let long_channel_user_id = create_mapping(
            State(state),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(CreateMappingRequest {
                channel: "telegram".to_string(),
                channel_user_id: "u".repeat(501),
            }),
        )
        .await
        .expect_err("channel_user_id length should be validated before DB access");
        assert!(matches!(
            long_channel_user_id,
            AppError::ValidationError(message)
                if message == "channel_user_id exceeds maximum length"
        ));
    }

    #[tokio::test]
    async fn test_handle_channel_message_missing_signature_returns_401() {
        let state = test_app_state_no_db().await;
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-openclaw-webhook-secret",
            HeaderValue::from_static("secret"),
        );

        let err = handle_channel_message(State(state), headers, Bytes::from_static(b"{}"))
            .await
            .expect_err("missing signature header should be unauthorized");

        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
        assert_eq!(
            err.1.0,
            serde_json::json!({ "error": "Missing X-OpenClaw-Signature header" })
        );
    }

    #[tokio::test]
    async fn test_handle_channel_message_rejects_wrong_secret() {
        let Some(db) = connect_test_database("h_openclaw_wrong_secret").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db);

        let Json(resp) = create_mapping(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(CreateMappingRequest {
                channel: "telegram".to_string(),
                channel_user_id: "tg-user-1".to_string(),
            }),
        )
        .await
        .unwrap();
        assert!(resp.webhook_secret.is_some());

        let body = serde_json::json!({
            "channel": "telegram",
            "channel_user_id": "tg-user-1",
            "message": "hello",
            "direction": "inbound"
        })
        .to_string();
        let wrong_secret = "wrong-secret";
        let headers = webhook_headers(wrong_secret, &signature(wrong_secret, body.as_bytes()));

        let err = handle_channel_message(State(state), headers, Bytes::from(body))
            .await
            .expect_err("wrong webhook secret should fail verification");

        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
        assert_eq!(
            err.1.0,
            serde_json::json!({ "error": "Webhook verification failed" })
        );
    }

    #[tokio::test]
    async fn test_handle_channel_message_accepts_valid_signature() {
        let Some(db) = connect_test_database("h_openclaw_valid_signature").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db);

        let Json(mapping) = create_mapping(
            State(state.clone()),
            test_auth_user(&user_id),
            TelemetryContext::default(),
            Json(CreateMappingRequest {
                channel: "whatsapp".to_string(),
                channel_user_id: "wa-user-7".to_string(),
            }),
        )
        .await
        .unwrap();
        let secret = mapping.webhook_secret.unwrap();
        let body = serde_json::json!({
            "channel": "whatsapp",
            "channel_user_id": "wa-user-7",
            "agent_id": "agent-1",
            "session_key": "session-1",
            "message": "hello",
            "direction": "inbound",
            "metadata": { "thread": "thread-1" }
        })
        .to_string();
        let headers = webhook_headers(&secret, &signature(&secret, body.as_bytes()));

        let Json(response) = handle_channel_message(State(state), headers, Bytes::from(body))
            .await
            .unwrap();

        assert_eq!(response.status, "resolved");
        assert_eq!(response.nyxid_user_id.as_deref(), Some(user_id.as_str()));
        assert!(response.nyxid_user_email.is_none());
        assert!(response.available_providers.is_empty());
        assert!(!response.openclaw_connected);
    }
}
