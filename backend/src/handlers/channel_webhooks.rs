//! Webhook handler for incoming platform messages (unauthenticated).
//!
//! Each channel bot has a unique webhook URL. The platform (e.g. Telegram,
//! Discord, Lark, Feishu) posts updates to this endpoint. The handler
//! verifies the signature, routes the message to the correct agent, and
//! forwards it via the agent's callback URL.
//!
//! Agent replies flow back asynchronously via POST /api/v1/channel-relay/reply
//! (synchronous 200+body replies are not supported per ADR-013 / NyxID#221).

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use mongodb::bson::doc;

use crate::AppState;
use crate::handlers::channel_bots::{hash_conversation_id, resolve_adapter};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::services::channel_platform::PlatformVerifySecrets;
use crate::services::{channel_bot_service, channel_relay_service, channel_routing_service};
use crate::telemetry::{
    TelemetryClient, TelemetryContext, TelemetryEvent, emit_event, should_sample_event,
};

struct WebhookHandlerDeps<'a> {
    db: &'a crate::db::DbHandle,
    config: &'a crate::config::AppConfig,
    jwt_keys: &'a crate::crypto::jwt::JwtKeys,
    http_client: &'a reqwest::Client,
    encryption_keys: &'a crate::crypto::aes::EncryptionKeys,
    token_exchange_cache:
        &'a std::sync::Arc<crate::services::provider_token_exchange_service::TokenExchangeCache>,
    telemetry: Option<&'a TelemetryClient>,
}

impl<'a> From<&'a AppState> for WebhookHandlerDeps<'a> {
    fn from(value: &'a AppState) -> Self {
        Self {
            db: &value.db,
            config: &value.config,
            jwt_keys: &value.jwt_keys,
            http_client: &value.http_client,
            encryption_keys: value.encryption_keys.as_ref(),
            token_exchange_cache: &value.token_exchange_cache,
            telemetry: value.telemetry.as_deref(),
        }
    }
}

// ---------------------------------------------------------------------------
// Platform-specific webhook handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/webhooks/channel/telegram/{bot_id}
///
/// Receives webhook updates from Telegram for a specific channel bot.
/// Always returns 200 OK to prevent Telegram from retrying failed deliveries.
/// Errors are logged internally but never surfaced to the platform.
pub async fn telegram_webhook(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    if let Err(e) = handle_webhook_inner(&state, &bot_id, "telegram", &headers, &body).await {
        tracing::warn!(
            bot_id = %bot_id,
            platform = "telegram",
            error = %e,
            "channel webhook processing error (suppressed)"
        );
    }
    StatusCode::OK
}

/// POST /api/v1/webhooks/channel/discord/{bot_id}
///
/// Receives interaction events from Discord for a specific channel bot.
/// Discord requires immediate JSON responses for certain interactions (PING),
/// so this handler returns the appropriate response body when needed.
pub async fn discord_webhook(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Discord PING challenge must be answered before any bot lookup / verification.
    // The adapter can parse the body without needing bot state.
    let adapter = match resolve_adapter("discord", &state.token_exchange_cache) {
        Ok(a) => a,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "".to_string()).into_response(),
    };

    if let Some(challenge_response) = adapter.handle_challenge(&body) {
        return (StatusCode::OK, Json(challenge_response)).into_response();
    }

    // Discord interactions (APPLICATION_COMMAND=2, MESSAGE_COMPONENT=4) require
    // an immediate interaction response. Return a deferred reply (type 5) and
    // process in a background task -- the relay will send the actual response
    // as a follow-up message via the REST API.
    let is_interaction = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("type")?.as_u64())
        .is_some_and(|t| t == 2 || t == 4);

    if is_interaction {
        let state_bg = state.clone();
        let bot_id_bg = bot_id.clone();
        let headers_bg = headers.clone();
        let body_bg = body.clone();
        tokio::spawn(async move {
            if let Err(e) =
                handle_webhook_inner(&state_bg, &bot_id_bg, "discord", &headers_bg, &body_bg).await
            {
                tracing::warn!(
                    bot_id = %bot_id_bg,
                    platform = "discord",
                    error = %e,
                    "discord interaction relay error (background)"
                );
            }
        });
        // DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE (type 5)
        return (StatusCode::OK, Json(serde_json::json!({ "type": 5 }))).into_response();
    }

    // Non-interaction messages (gateway-style) -- process inline
    if let Err(e) = handle_webhook_inner(&state, &bot_id, "discord", &headers, &body).await {
        tracing::warn!(
            bot_id = %bot_id,
            platform = "discord",
            error = %e,
            "channel webhook processing error (suppressed)"
        );
    }
    (StatusCode::OK, "".to_string()).into_response()
}

/// POST /api/v1/webhooks/channel/lark/{bot_id}
///
/// Receives event callbacks from Lark (international) for a specific channel bot.
/// Lark url_verification challenges are answered only after bot lookup and
/// Verification Token validation, and may require decrypting the body first.
pub async fn lark_webhook(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    match handle_webhook_inner(&state, &bot_id, "lark", &headers, &body).await {
        Ok(Some(challenge_response)) => (StatusCode::OK, Json(challenge_response)).into_response(),
        Ok(None) => (StatusCode::OK, "".to_string()).into_response(),
        Err(e) => {
            tracing::warn!(
                bot_id = %bot_id,
                platform = "lark",
                error = %e,
                "channel webhook processing error (suppressed)"
            );
            (StatusCode::OK, "".to_string()).into_response()
        }
    }
}

/// POST /api/v1/webhooks/channel/feishu/{bot_id}
///
/// Receives event callbacks from Feishu (China mainland) for a specific channel bot.
/// Feishu url_verification challenges are answered only after bot lookup and
/// Verification Token validation, and may require decrypting the body first.
pub async fn feishu_webhook(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    match handle_webhook_inner(&state, &bot_id, "feishu", &headers, &body).await {
        Ok(Some(challenge_response)) => (StatusCode::OK, Json(challenge_response)).into_response(),
        Ok(None) => (StatusCode::OK, "".to_string()).into_response(),
        Err(e) => {
            tracing::warn!(
                bot_id = %bot_id,
                platform = "feishu",
                error = %e,
                "channel webhook processing error (suppressed)"
            );
            (StatusCode::OK, "".to_string()).into_response()
        }
    }
}

/// POST /api/v1/webhooks/channel/slack/{bot_id}
///
/// Receives Events API callbacks from Slack for a specific channel bot.
/// Slack requires a 2xx response within 3 seconds, so heavy processing
/// (signature verification, agent dispatch) runs in a background task and
/// the handler returns 200 OK immediately. The one-time `url_verification`
/// challenge is answered synchronously without bot lookup.
pub async fn slack_webhook(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let adapter = match resolve_adapter("slack", &state.token_exchange_cache) {
        Ok(a) => a,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "".to_string()).into_response(),
    };

    if let Some(challenge_response) = adapter.handle_challenge(&body) {
        return (StatusCode::OK, Json(challenge_response)).into_response();
    }

    // Honor Slack's 3-second ACK rule: ACK immediately, process asynchronously.
    let state_bg = state.clone();
    let bot_id_bg = bot_id.clone();
    let headers_bg = headers.clone();
    let body_bg = body.clone();
    tokio::spawn(async move {
        if let Err(e) =
            handle_webhook_inner(&state_bg, &bot_id_bg, "slack", &headers_bg, &body_bg).await
        {
            tracing::warn!(
                bot_id = %bot_id_bg,
                platform = "slack",
                error = %e,
                "slack webhook processing error (background, suppressed)"
            );
        }
    });

    (StatusCode::OK, "".to_string()).into_response()
}

// ---------------------------------------------------------------------------
// Shared inner handler
// ---------------------------------------------------------------------------

/// Generic webhook processing logic shared by all platform handlers.
///
/// Looks up the bot, verifies the webhook signature via the platform adapter,
/// parses inbound messages, resolves agent routing, forwards callbacks, and
/// optionally sends synchronous replies back to the platform.
///
/// Returns errors for logging; the outer platform handler suppresses them and
/// always returns 200 OK to prevent platforms from retrying failed deliveries.
async fn handle_webhook_inner(
    state: &AppState,
    bot_id: &str,
    expected_platform: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
    handle_webhook_inner_with_deps(
        WebhookHandlerDeps::from(state),
        bot_id,
        expected_platform,
        headers,
        body,
    )
    .await
}

async fn handle_webhook_inner_with_deps(
    state: WebhookHandlerDeps<'_>,
    bot_id: &str,
    expected_platform: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
    // Look up the bot
    let bot = match channel_bot_service::get_bot(state.db, bot_id).await {
        Ok(b) => b,
        Err(_) => {
            tracing::debug!(bot_id = %bot_id, "webhook for unknown bot");
            return Ok(None);
        }
    };

    // Verify the bot platform matches the webhook route
    if bot.platform != expected_platform {
        tracing::warn!(
            bot_id = %bot_id,
            expected = %expected_platform,
            actual = %bot.platform,
            "webhook platform mismatch"
        );
        return Ok(None);
    }

    // Reject if bot is inactive
    if !bot.is_active {
        tracing::debug!(bot_id = %bot_id, status = %bot.status, "webhook for inactive bot");
        return Ok(None);
    }

    // Allow pending_webhook bots through to verification -- they'll be promoted
    // after signature verification succeeds (not before).
    let is_pending_webhook = bot.status == "pending_webhook";
    if !is_pending_webhook && bot.status != "active" {
        tracing::debug!(bot_id = %bot_id, status = %bot.status, "webhook for non-active bot");
        return Ok(None);
    }

    let adapter = resolve_adapter(&bot.platform, state.token_exchange_cache).map_err(
        |e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("unsupported platform {}: {e}", bot.platform).into()
        },
    )?;

    let verify_secrets = build_verify_secrets(&state, &bot).await.map_err(
        |e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("failed to prepare webhook secrets: {e}").into()
        },
    )?;

    let prepared = adapter
        .prepare_webhook(&bot, Some(&verify_secrets), headers, body)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("webhook verification failed: {e}").into()
        })?;

    if let Some(challenge_response) = prepared.challenge_response {
        return Ok(Some(challenge_response));
    }

    // Auto-promote pending_webhook bots AFTER successful signature verification.
    // This proves the user correctly configured the webhook URL on the platform.
    if is_pending_webhook {
        let now = mongodb::bson::DateTime::from_chrono(chrono::Utc::now());
        let _ = state
            .db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME,
            )
            .update_one(
                mongodb::bson::doc! { "_id": &bot.id },
                mongodb::bson::doc! { "$set": {
                    "status": "active",
                    "webhook_registered": true,
                    "updated_at": now,
                }},
            )
            .await;
        tracing::info!(bot_id = %bot_id, "auto-promoted pending_webhook bot to active");
    }

    // Parse inbound messages
    let messages = adapter.parse_inbound(&prepared.body).await.map_err(
        |e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("failed to parse inbound messages: {e}").into()
        },
    )?;

    if messages.is_empty() {
        return Ok(None);
    }

    // Parse bot owner UUID once (used for relay token generation per-message)
    let bot_owner_uuid = bot.user_id.parse::<uuid::Uuid>().map_err(
        |e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("invalid bot owner user_id: {e}").into()
        },
    )?;

    for inbound in &messages {
        // Resolve which agent should handle this message
        let route = match channel_routing_service::resolve_agent(
            state.db,
            &bot.id,
            &inbound.conversation_id,
            Some(&inbound.sender_platform_id),
        )
        .await
        {
            Ok(Some(r)) => r,
            Ok(None) => {
                tracing::debug!(
                    bot_id = %bot.id,
                    conversation_id = %inbound.conversation_id,
                    "no agent route found, skipping message"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    bot_id = %bot.id,
                    error = %e,
                    "agent resolution failed"
                );
                continue;
            }
        };

        // Store the inbound message
        let stored_message = match channel_relay_service::store_inbound_message(
            state.db,
            &bot.id,
            &route.conversation.id,
            &bot.user_id,
            &bot.platform,
            inbound,
            &route.api_key_id,
        )
        .await
        {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, "failed to store inbound message");
                continue;
            }
        };

        // Telemetry: channel.message_received is sampled at 10% per
        // docs/TELEMETRY.md §6.5. Sampling key is the conversation hash,
        // NOT the user id: hashing on user_id would make each owner either
        // 100% in or 100% out of the sample and skew the funnel toward
        // whichever high-volume owner happens to hash in. Conversation-
        // keyed sampling gives ~10% of messages per conversation, which
        // averages to ~10% across owners. Webhook ingress has no AuthUser /
        // TelemetryContext — use default context and None for api_key_id.
        let distinct_id = bot.user_id.clone();
        let conversation_hash = hash_conversation_id(&route.conversation.platform_conversation_id);
        if should_sample_event("channel.message_received", &conversation_hash, 10) {
            emit_event(
                state.telemetry,
                &distinct_id,
                None,
                &TelemetryContext::default(),
                TelemetryEvent::ChannelMessageReceived {
                    platform: bot.platform.clone(),
                    conversation_id_hash: conversation_hash,
                },
            );
        }

        // Look up the API key for signing and name attribution
        let api_key = match state
            .db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &route.api_key_id })
            .await
        {
            Ok(Some(k)) => k,
            _ => {
                tracing::warn!(
                    api_key_id = %route.api_key_id,
                    "API key not found for callback signing"
                );
                let _ = channel_relay_service::update_callback_status(
                    state.db,
                    &stored_message.id,
                    "failed",
                )
                .await;
                continue;
            }
        };

        // Generate a relay token scoped to this agent key's permissions.
        // The token carries the bot owner's identity but inherits the agent
        // key's service/node scope restrictions.
        let user_access_token = {
            let scope = crate::services::token_service::FIRST_PARTY_ACCESS_SCOPES;
            let rbac_data =
                crate::services::rbac_helpers::build_rbac_claim_data(state.db, &bot.user_id, scope)
                    .await
                    .ok();
            let agent_scope = crate::crypto::jwt::RelayAgentScope {
                api_key_id: api_key.id.clone(),
                api_key_name: api_key.name.clone(),
                allowed_service_ids: api_key.allowed_service_ids.clone(),
                allowed_node_ids: api_key.allowed_node_ids.clone(),
                allow_all_services: api_key.allow_all_services,
                allow_all_nodes: api_key.allow_all_nodes,
            };
            crate::crypto::jwt::generate_relay_access_token(
                state.jwt_keys,
                state.config,
                &bot_owner_uuid,
                scope,
                rbac_data.as_ref(),
                &agent_scope,
            )
            .ok()
        };

        let reply_token = match crate::crypto::jwt::generate_relay_reply_token(
            state.jwt_keys,
            state.config,
            &api_key.id,
            &route.conversation.id,
            &stored_message.id,
            &route.conversation.platform,
        ) {
            Ok(token) => token,
            Err(e) => {
                tracing::error!(
                    message_id = %stored_message.id,
                    error = %e,
                    "failed to generate relay reply token"
                );
                let _ = channel_relay_service::update_callback_status(
                    state.db,
                    &stored_message.id,
                    "failed",
                )
                .await;
                continue;
            }
        };

        // Build the callback payload
        let payload = channel_relay_service::build_callback_payload(
            &stored_message,
            &route.conversation,
            &route.api_key_id,
            &api_key.name,
            inbound,
            Some(reply_token),
        );

        // Forward to the agent's callback URL
        let delivery = channel_relay_service::forward_to_agent(
            state.http_client,
            state.config,
            state.jwt_keys,
            &route.callback_url,
            payload,
            &api_key.id,
            &api_key.key_hash,
            user_access_token.as_deref(),
        )
        .await;

        // Sync 200+body replies are no longer supported (per ADR-013 / NyxID#221
        // comment 2). Agents must return 202 and post replies asynchronously
        // via POST /api/v1/channel-relay/reply. The callback status only
        // reflects delivery of the webhook to the agent's callback URL.
        match delivery.result {
            Ok(()) => {
                let _ = channel_relay_service::update_callback_status(
                    state.db,
                    &stored_message.id,
                    "delivered",
                )
                .await;
            }
            Err(e) => {
                tracing::warn!(
                    message_id = %stored_message.id,
                    upstream_status = ?delivery.http_status,
                    error = %e,
                    "callback delivery failed"
                );
                let _ = channel_relay_service::update_callback_status(
                    state.db,
                    &stored_message.id,
                    "failed",
                )
                .await;
            }
        }

        // Touch conversation last_message_at timestamp
        let _ = channel_routing_service::touch_conversation(state.db, &route.conversation.id).await;
    }

    Ok(None)
}

async fn decrypt_secret_field(
    state: &WebhookHandlerDeps<'_>,
    field_name: &str,
    value: Option<&Vec<u8>>,
) -> crate::errors::AppResult<Option<String>> {
    let Some(encrypted) = value else {
        return Ok(None);
    };

    let decrypted = state
        .encryption_keys
        .decrypt(encrypted)
        .await
        .map_err(|e| {
            crate::errors::AppError::Internal(format!("failed to decrypt {field_name}: {e}"))
        })?;

    let text = String::from_utf8(decrypted).map_err(|e| {
        crate::errors::AppError::Internal(format!(
            "{field_name} decryption produced invalid UTF-8: {e}"
        ))
    })?;

    Ok(Some(text))
}

async fn build_verify_secrets(
    state: &WebhookHandlerDeps<'_>,
    bot: &crate::models::channel_bot::ChannelBot,
) -> crate::errors::AppResult<PlatformVerifySecrets> {
    let mut secrets = PlatformVerifySecrets::default();

    match bot.platform.as_str() {
        "slack" => {
            secrets.slack_signing_secret = decrypt_secret_field(
                state,
                "Slack signing secret",
                bot.app_secret_encrypted.as_ref(),
            )
            .await?;
        }
        "lark" | "feishu" => {
            secrets.lark_verification_token = decrypt_secret_field(
                state,
                "Lark verification token",
                bot.lark_verification_token_encrypted.as_ref(),
            )
            .await?;
            secrets.lark_encrypt_key = decrypt_secret_field(
                state,
                "Lark encrypt key",
                bot.lark_encrypt_key_encrypted.as_ref(),
            )
            .await?;
        }
        _ => {}
    }

    Ok(secrets)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use axum::{Router, body::Bytes, http::StatusCode, routing::post};
    use chrono::Utc;
    use mongodb::bson::doc;
    use tokio::sync::{Mutex, oneshot};
    use tokio::time::{Duration, timeout};

    fn test_config(
        database_url: String,
        key_dir: &std::path::Path,
        encryption_key: &str,
    ) -> crate::config::AppConfig {
        crate::config::AppConfig {
            port: 3001,
            base_url: "http://localhost:3001".to_string(),
            frontend_url: "http://localhost:3000".to_string(),
            cors_allowed_origins: vec![],
            csrf_trusted_origins: vec![],
            database_url,
            database_max_connections: 10,
            environment: "development".to_string(),
            jwt_private_key_path: key_dir.join("private.pem").display().to_string(),
            jwt_public_key_path: key_dir.join("public.pem").display().to_string(),
            jwt_issuer: "http://localhost:3001".to_string(),
            jwt_access_ttl_secs: 900,
            jwt_relay_reply_ttl_secs: 1800,
            jwt_relay_callback_ttl_secs: 300,
            jwt_refresh_ttl_secs: 604800,
            google_client_id: None,
            google_client_secret: None,
            github_client_id: None,
            github_client_secret: None,
            apple_client_id: None,
            apple_team_id: None,
            apple_key_id: None,
            apple_private_key_path: None,
            smtp_host: None,
            smtp_port: None,
            smtp_username: None,
            smtp_password: None,
            smtp_from_address: None,
            encryption_key: Some(encryption_key.to_string()),
            encryption_key_previous: None,
            rate_limit_per_second: 10,
            rate_limit_burst: 30,
            trusted_proxy_ips: vec![],
            mtls_client_cert_header: None,
            cli_pairing_hmac_key: None,
            sa_token_ttl_secs: 3600,
            cookie_domain: None,
            telegram_bot_token: None,
            telegram_webhook_secret: None,
            telegram_webhook_url: None,
            telegram_bot_username: None,
            approval_expiry_interval_secs: 5,
            oauth_refresh_sweep_interval_secs: 600,
            oauth_refresh_sweep_window_secs: 900,
            fcm_service_account_path: None,
            fcm_project_id: None,
            apns_key_path: None,
            apns_key_id: None,
            apns_team_id: None,
            apns_topic: None,
            apns_sandbox: true,
            key_provider: "local".to_string(),
            aws_kms_key_arn: None,
            aws_kms_key_arn_previous: None,
            gcp_kms_key_name: None,
            gcp_kms_key_name_previous: None,
            node_heartbeat_interval_secs: 30,
            node_heartbeat_timeout_secs: 90,
            node_proxy_timeout_secs: 30,
            node_registration_token_ttl_secs: 3600,
            node_pending_credential_ttl_secs: 86_400,
            node_max_per_user: 10,
            node_max_ws_connections: 100,
            node_max_stream_duration_secs: 300,
            node_hmac_signing_enabled: true,
            proxy_max_body_size: 100 * 1024 * 1024,
            proxy_stream_idle_timeout_secs: 60,
            ssh_max_sessions_per_user: 4,
            ssh_connect_timeout_secs: 10,
            ssh_max_tunnel_duration_secs: 3600,
            ws_passthrough_max_connections: 200,
            channel_relay_callback_timeout_secs: 30,
            channel_relay_max_bots_per_user: 5,
            channel_relay_message_ttl_days: 30,
            channel_relay_edit_rate_limit_per_second: 10,
            channel_relay_edit_rate_limit_burst: 20,
            channel_event_rate_limit_per_second: 100,
            channel_event_rate_limit_burst: 200,
            channel_event_dedup_capacity: 32_768,
            channel_event_dedup_ttl_secs: 300,
            cloud_response_cache_ttl_secs: 0,
            cloud_response_cache_max_entry_bytes: 1024 * 1024,
            cloud_response_cache_max_entries: 256,
            invite_code_required: true,
            email_auth_enabled: false,
            auto_verify_email: false,
            telemetry_dsn: None,
            telemetry_host: None,
            share_analytics: false,
        }
    }

    fn lark_event_body(token: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "ev_123",
                "event_type": "im.message.receive_v1",
                "create_time": "1700000000",
                "token": token
            },
            "event": {
                "sender": {
                    "sender_id": {
                        "open_id": "ou_user123",
                        "name": "Alice"
                    }
                },
                "message": {
                    "message_id": "om_msg456",
                    "chat_id": "oc_chat789",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"Hello bot\"}"
                }
            }
        }))
        .unwrap()
    }

    async fn spawn_mock_callback_server() -> (
        String,
        Arc<Mutex<Vec<serde_json::Value>>>,
        oneshot::Sender<()>,
    ) {
        let received_requests = Arc::new(Mutex::new(Vec::new()));
        let route_requests = received_requests.clone();
        let app = Router::new().route(
            "/callback",
            post(move |body: Bytes| {
                let route_requests = route_requests.clone();
                async move {
                    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    route_requests.lock().await.push(parsed);
                    StatusCode::ACCEPTED
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        (
            format!("http://{address}/callback"),
            received_requests,
            shutdown_tx,
        )
    }

    #[tokio::test]
    async fn valid_lark_event_promotes_pending_webhook_bot_and_stores_message() {
        let Some(db) = crate::test_utils::connect_test_database("channel_webhooks").await else {
            eprintln!("skipping channel_webhooks integration test: no local MongoDB available");
            return;
        };

        let key_dir = tempfile::tempdir().unwrap();
        let config = test_config(
            "mongodb://ignored-for-test".to_string(),
            key_dir.path(),
            &"11".repeat(32),
        );
        // Use the process-wide cached test keypair instead of generating a
        // fresh 4096-bit RSA key per test run.
        let jwt_keys = crate::test_utils::cached_test_jwt_keys();
        let encryption_keys = crate::crypto::aes::EncryptionKeys::from_config(&config);
        let user_id = uuid::Uuid::new_v4().to_string();
        let bot_id = uuid::Uuid::new_v4().to_string();
        let api_key_id = uuid::Uuid::new_v4().to_string();
        let conversation_id = uuid::Uuid::new_v4().to_string();
        let (callback_url, received_requests, shutdown_tx) = spawn_mock_callback_server().await;

        let verification_token_encrypted = encryption_keys.encrypt(b"verify_token").await.unwrap();

        let bot = crate::models::channel_bot::ChannelBot {
            id: bot_id.clone(),
            user_id: user_id.clone(),
            platform: "lark".to_string(),
            label: "Lark Bot".to_string(),
            bot_token_encrypted: vec![0; 16],
            platform_bot_id: "cli_test".to_string(),
            platform_bot_username: "lark_bot".to_string(),
            webhook_registered: false,
            webhook_secret_hash: "unused".to_string(),
            app_id: Some("cli_test".to_string()),
            app_secret_encrypted: None,
            lark_verification_token_encrypted: Some(verification_token_encrypted),
            lark_encrypt_key_encrypted: None,
            public_key: None,
            status: "pending_webhook".to_string(),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let api_key = crate::models::api_key::ApiKey {
            id: api_key_id.clone(),
            user_id: user_id.clone(),
            name: "agent".to_string(),
            key_prefix: "nyxid_ag".to_string(),
            key_hash: "deadbeef".repeat(8),
            scopes: "read write".to_string(),
            last_used_at: None,
            expires_at: None,
            is_active: true,
            created_at: Utc::now(),
            description: None,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: true,
            allow_all_nodes: true,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            platform: Some("codex".to_string()),
            callback_url: Some(callback_url),
        };

        let conversation = crate::models::channel_conversation::ChannelConversation {
            id: conversation_id.clone(),
            user_id: user_id.clone(),
            channel_bot_id: Some(bot_id.clone()),
            platform: "lark".to_string(),
            platform_conversation_id: "oc_chat789".to_string(),
            platform_conversation_type: "private".to_string(),
            platform_sender_id: None,
            agent_api_key_id: api_key_id.clone(),
            default_agent: false,
            is_active: true,
            last_message_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        db.collection::<crate::models::channel_bot::ChannelBot>(
            crate::models::channel_bot::COLLECTION_NAME,
        )
        .insert_one(&bot)
        .await
        .unwrap();
        db.collection::<crate::models::api_key::ApiKey>(crate::models::api_key::COLLECTION_NAME)
            .insert_one(&api_key)
            .await
            .unwrap();
        db.collection::<crate::models::channel_conversation::ChannelConversation>(
            crate::models::channel_conversation::COLLECTION_NAME,
        )
        .insert_one(&conversation)
        .await
        .unwrap();

        let http_client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(1))
            .timeout(std::time::Duration::from_secs(1))
            .build()
            .unwrap();
        let token_exchange_cache = std::sync::Arc::new(
            crate::services::provider_token_exchange_service::TokenExchangeCache::new(),
        );
        let deps = WebhookHandlerDeps {
            db: &db,
            config: &config,
            jwt_keys: &jwt_keys,
            http_client: &http_client,
            encryption_keys: &encryption_keys,
            token_exchange_cache: &token_exchange_cache,
            telemetry: None,
        };

        let result = handle_webhook_inner_with_deps(
            deps,
            &bot_id,
            "lark",
            &HeaderMap::new(),
            &lark_event_body("verify_token"),
        )
        .await
        .unwrap();

        assert!(result.is_none());

        let updated_bot = db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME,
            )
            .find_one(doc! { "_id": &bot_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated_bot.status, "active");
        assert!(updated_bot.webhook_registered);

        let stored_count = db
            .collection::<crate::models::channel_message::ChannelMessage>(
                crate::models::channel_message::COLLECTION_NAME,
            )
            .count_documents(doc! { "channel_bot_id": &bot_id, "direction": "inbound" })
            .await
            .unwrap();
        assert_eq!(stored_count, 1);

        let delivered = timeout(Duration::from_secs(2), async {
            loop {
                let snapshot = received_requests.lock().await.clone();
                if !snapshot.is_empty() {
                    return snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("agent callback should be delivered");
        assert_eq!(delivered.len(), 1);
        let payload = &delivered[0];
        let reply_token = payload
            .get("reply_token")
            .and_then(|value| value.as_str())
            .expect("callback payload should include reply_token");
        let claims =
            crate::crypto::jwt::validate_relay_reply_token(&jwt_keys, &config, reply_token)
                .expect("reply_token should validate");
        assert_eq!(claims.api_key_id, api_key_id);
        assert_eq!(claims.conversation_id, conversation_id);
        assert_eq!(
            claims.inbound_message_id,
            payload["message_id"].as_str().expect("payload message id")
        );
        assert_eq!(claims.platform, "lark");

        let _ = shutdown_tx.send(());

        db.drop().await.unwrap();
    }

    // ── lark_event_body helper tests ────────────────────────────────────

    #[test]
    fn lark_event_body_produces_valid_json() {
        let body = lark_event_body("my_token");
        let parsed: serde_json::Value =
            serde_json::from_slice(&body).expect("lark_event_body should produce valid JSON");
        assert_eq!(parsed["schema"], "2.0");
        assert_eq!(parsed["header"]["token"], "my_token");
        assert_eq!(parsed["header"]["event_type"], "im.message.receive_v1");
    }

    #[test]
    fn lark_event_body_contains_expected_message_fields() {
        let body = lark_event_body("tok");
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let sender = &parsed["event"]["sender"]["sender_id"];
        assert_eq!(sender["open_id"], "ou_user123");

        let message = &parsed["event"]["message"];
        assert_eq!(message["message_id"], "om_msg456");
        assert_eq!(message["chat_id"], "oc_chat789");
        assert_eq!(message["chat_type"], "p2p");
        assert_eq!(message["message_type"], "text");
        assert_eq!(message["content"], r#"{"text":"Hello bot"}"#);
    }

    #[test]
    fn lark_event_body_token_is_parameterized() {
        let body_a = lark_event_body("token_a");
        let body_b = lark_event_body("token_b");
        let parsed_a: serde_json::Value = serde_json::from_slice(&body_a).unwrap();
        let parsed_b: serde_json::Value = serde_json::from_slice(&body_b).unwrap();
        assert_eq!(parsed_a["header"]["token"], "token_a");
        assert_eq!(parsed_b["header"]["token"], "token_b");
        assert_ne!(body_a, body_b);
    }

    // ── PlatformVerifySecrets default ───────────────────────────────────

    #[test]
    fn platform_verify_secrets_default_is_all_none() {
        let secrets = crate::services::channel_platform::PlatformVerifySecrets::default();
        assert!(secrets.slack_signing_secret.is_none());
        assert!(secrets.lark_verification_token.is_none());
        assert!(secrets.lark_encrypt_key.is_none());
    }

    // ── WebhookHandlerDeps lifetime wiring ──────────────────────────────

    #[test]
    fn test_config_produces_expected_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(
            "mongodb://localhost:27017/test".to_string(),
            dir.path(),
            &"ab".repeat(32),
        );
        assert_eq!(config.port, 3001);
        assert_eq!(config.base_url, "http://localhost:3001");
        assert_eq!(config.frontend_url, "http://localhost:3000");
        assert_eq!(config.jwt_access_ttl_secs, 900);
        assert_eq!(config.jwt_refresh_ttl_secs, 604800);
        assert_eq!(config.jwt_relay_reply_ttl_secs, 1800);
        assert_eq!(config.jwt_relay_callback_ttl_secs, 300);
        assert_eq!(config.rate_limit_per_second, 10);
        assert_eq!(config.rate_limit_burst, 30);
        assert_eq!(config.channel_relay_callback_timeout_secs, 30);
        assert_eq!(config.channel_relay_max_bots_per_user, 5);
        assert_eq!(config.channel_relay_message_ttl_days, 30);
        assert_eq!(config.channel_event_rate_limit_per_second, 100);
        assert_eq!(config.channel_event_dedup_capacity, 32_768);
        assert_eq!(config.channel_event_dedup_ttl_secs, 300);
        assert!(config.invite_code_required);
        assert!(!config.auto_verify_email);
    }

    // ── hash_conversation_id determinism and format ─────────────────────

    #[test]
    fn hash_conversation_id_is_deterministic() {
        let a = crate::handlers::channel_bots::hash_conversation_id("oc_chat789");
        let b = crate::handlers::channel_bots::hash_conversation_id("oc_chat789");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_conversation_id_produces_hex_string() {
        let hash = crate::handlers::channel_bots::hash_conversation_id("test-conv-id");
        // SHA-256 truncated to 8 bytes => 16 hex chars
        assert_eq!(hash.len(), 16);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_conversation_id_different_inputs_produce_different_hashes() {
        let a = crate::handlers::channel_bots::hash_conversation_id("conv-a");
        let b = crate::handlers::channel_bots::hash_conversation_id("conv-b");
        assert_ne!(a, b);
    }

    #[test]
    fn hash_conversation_id_empty_string() {
        let hash = crate::handlers::channel_bots::hash_conversation_id("");
        assert_eq!(hash.len(), 16);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ── should_sample_event determinism ─────────────────────────────────

    #[test]
    fn should_sample_event_is_deterministic() {
        use crate::telemetry::should_sample_event;
        let a = should_sample_event("channel.message_received", "conv-hash-abc", 10);
        let b = should_sample_event("channel.message_received", "conv-hash-abc", 10);
        assert_eq!(a, b);
    }

    #[test]
    fn should_sample_event_100_percent_always_true() {
        use crate::telemetry::should_sample_event;
        assert!(should_sample_event("any_event", "any_id", 100));
    }

    #[test]
    fn should_sample_event_0_percent_always_false() {
        use crate::telemetry::should_sample_event;
        assert!(!should_sample_event("any_event", "any_id", 0));
    }

    // ── Discord interaction type detection (inline logic test) ──────────

    #[test]
    fn discord_interaction_type_detection() {
        // The handler checks if type == 2 or type == 4 for interactions.
        let command_body = serde_json::json!({"type": 2, "data": {}});
        let component_body = serde_json::json!({"type": 4, "data": {}});
        let message_body = serde_json::json!({"type": 0, "content": "hello"});
        let ping_body = serde_json::json!({"type": 1});

        let is_interaction = |v: &serde_json::Value| -> bool {
            v.get("type")
                .and_then(|t| t.as_u64())
                .is_some_and(|t| t == 2 || t == 4)
        };

        assert!(is_interaction(&command_body));
        assert!(is_interaction(&component_body));
        assert!(!is_interaction(&message_body));
        assert!(!is_interaction(&ping_body));
    }

    // ── Mock callback server spawn test ─────────────────────────────────

    #[tokio::test]
    async fn mock_callback_server_accepts_post() {
        let (url, received, shutdown_tx) = spawn_mock_callback_server().await;

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .json(&serde_json::json!({"test": true}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 202);

        let snapshot = received.lock().await;
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0]["test"], true);

        let _ = shutdown_tx.send(());
    }
}
