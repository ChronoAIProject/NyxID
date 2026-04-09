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
use crate::handlers::channel_bots::resolve_adapter;
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::services::{channel_bot_service, channel_relay_service, channel_routing_service};

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
/// Lark url_verification challenges are answered immediately with the challenge value.
pub async fn lark_webhook(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let adapter = match resolve_adapter("lark", &state.token_exchange_cache) {
        Ok(a) => a,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "".to_string()).into_response(),
    };

    if let Some(challenge_response) = adapter.handle_challenge(&body) {
        return (StatusCode::OK, Json(challenge_response)).into_response();
    }

    if let Err(e) = handle_webhook_inner(&state, &bot_id, "lark", &headers, &body).await {
        tracing::warn!(
            bot_id = %bot_id,
            platform = "lark",
            error = %e,
            "channel webhook processing error (suppressed)"
        );
    }
    (StatusCode::OK, "".to_string()).into_response()
}

/// POST /api/v1/webhooks/channel/feishu/{bot_id}
///
/// Receives event callbacks from Feishu (China mainland) for a specific channel bot.
/// Feishu url_verification challenges are answered immediately with the challenge value.
pub async fn feishu_webhook(
    State(state): State<AppState>,
    Path(bot_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let adapter = match resolve_adapter("feishu", &state.token_exchange_cache) {
        Ok(a) => a,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "".to_string()).into_response(),
    };

    if let Some(challenge_response) = adapter.handle_challenge(&body) {
        return (StatusCode::OK, Json(challenge_response)).into_response();
    }

    if let Err(e) = handle_webhook_inner(&state, &bot_id, "feishu", &headers, &body).await {
        tracing::warn!(
            bot_id = %bot_id,
            platform = "feishu",
            error = %e,
            "channel webhook processing error (suppressed)"
        );
    }
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
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Look up the bot
    let bot = match channel_bot_service::get_bot(&state.db, bot_id).await {
        Ok(b) => b,
        Err(_) => {
            tracing::debug!(bot_id = %bot_id, "webhook for unknown bot");
            return Ok(());
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
        return Ok(());
    }

    // Reject if bot is inactive
    if !bot.is_active {
        tracing::debug!(bot_id = %bot_id, status = %bot.status, "webhook for inactive bot");
        return Ok(());
    }

    // Allow pending_webhook bots through to verification -- they'll be promoted
    // after signature verification succeeds (not before).
    let is_pending_webhook = bot.status == "pending_webhook";
    if !is_pending_webhook && bot.status != "active" {
        tracing::debug!(bot_id = %bot_id, status = %bot.status, "webhook for non-active bot");
        return Ok(());
    }

    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache).map_err(
        |e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("unsupported platform {}: {e}", bot.platform).into()
        },
    )?;

    // For Lark/Feishu, the adapter uses webhook_secret_hash as the HMAC key.
    // The real verification token is the app_secret, so decrypt it and inject
    // it into a cloned bot for the verification step.
    let bot_for_verify = if matches!(bot.platform.as_str(), "lark" | "feishu") {
        if let Some(ref encrypted) = bot.app_secret_encrypted {
            match state.encryption_keys.decrypt(encrypted).await {
                Ok(decrypted) => {
                    let mut cloned = bot.clone();
                    cloned.webhook_secret_hash = String::from_utf8_lossy(&decrypted).to_string();
                    cloned
                }
                Err(_) => bot.clone(),
            }
        } else {
            bot.clone()
        }
    } else {
        bot.clone()
    };

    // Verify webhook signature
    adapter
        .verify_webhook(&bot_for_verify, headers, body)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("webhook verification failed: {e}").into()
        })?;

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
    let messages = adapter.parse_inbound(body).await.map_err(
        |e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("failed to parse inbound messages: {e}").into()
        },
    )?;

    if messages.is_empty() {
        return Ok(());
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
            &state.db,
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
            &state.db,
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
                    &state.db,
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
            let rbac_data = crate::services::rbac_helpers::build_rbac_claim_data(
                &state.db,
                &bot.user_id,
                scope,
            )
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
                &state.jwt_keys,
                &state.config,
                &bot_owner_uuid,
                scope,
                rbac_data.as_ref(),
                &agent_scope,
            )
            .ok()
        };

        // Build the callback payload
        let payload = channel_relay_service::build_callback_payload(
            &stored_message,
            &route.conversation,
            &route.api_key_id,
            &api_key.name,
            inbound,
        );

        // Forward to the agent's callback URL
        let delivery = channel_relay_service::forward_to_agent(
            &state.http_client,
            &state.config,
            &route.callback_url,
            &payload,
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
                    &state.db,
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
                    &state.db,
                    &stored_message.id,
                    "failed",
                )
                .await;
            }
        }

        // Touch conversation last_message_at timestamp
        let _ =
            channel_routing_service::touch_conversation(&state.db, &route.conversation.id).await;
    }

    Ok(())
}
