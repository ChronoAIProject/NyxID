//! Shared post-parse channel routing and callback dispatch.

use crate::AppState;
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::services::{channel_relay_service, channel_routing_service};
use crate::telemetry::{
    TelemetryClient, TelemetryContext, TelemetryEvent, emit_event, should_sample_event,
};
use bson::doc;

pub(crate) struct InboundDeps<'a> {
    pub(crate) db: &'a crate::db::DbHandle,
    pub(crate) config: &'a crate::config::AppConfig,
    pub(crate) jwt_keys: &'a crate::crypto::jwt::JwtKeys,
    pub(crate) http_client: &'a reqwest::Client,
    pub(crate) encryption_keys: &'a crate::crypto::aes::EncryptionKeys,
    pub(crate) token_exchange_cache:
        &'a std::sync::Arc<crate::services::provider_token_exchange_service::TokenExchangeCache>,
    pub(crate) telemetry: Option<&'a TelemetryClient>,
}

impl<'a> From<&'a AppState> for InboundDeps<'a> {
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

/// Truncated SHA-256 of a platform conversation ID, for use in telemetry
/// properties where raw conversation IDs must not be emitted. Returns the
/// first 16 hex chars (8 bytes) of the digest — enough entropy for
/// per-conversation cardinality analysis, short enough to stay ergonomic.
pub(crate) fn hash_conversation_id(id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

pub(crate) async fn process_inbound_messages(
    state: InboundDeps<'_>,
    bot: &crate::models::channel_bot::ChannelBot,
    adapter: &dyn crate::services::channel_platform::PlatformAdapter,
    messages: &[crate::services::channel_platform::InboundMessage],
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    if messages.is_empty() {
        return Ok(true);
    }

    // Parse bot owner UUID once (used for relay token generation per-message)
    let bot_owner_uuid = bot.user_id.parse::<uuid::Uuid>().map_err(
        |e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("invalid bot owner user_id: {e}").into()
        },
    )?;

    let mut complete = true;
    for inbound in messages {
        if adapter.dedup_inbound_by_platform_message_id()
            && channel_relay_service::inbound_platform_message_exists(
                state.db,
                &bot.id,
                &bot.platform,
                &inbound.platform_message_id,
            )
            .await?
        {
            tracing::debug!(
                bot_id = %bot.id, platform = %bot.platform,
                platform_message_id = %inbound.platform_message_id,
                "skipping duplicate inbound platform message"
            );
            continue;
        }
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
                complete = false;
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
                complete = false;
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

    Ok(complete)
}
