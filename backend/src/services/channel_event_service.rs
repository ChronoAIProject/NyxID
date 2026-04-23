//! HTTP Event Gateway service.
//!
//! Orchestrates forwarding of device events to agent callback URLs per
//! NyxID#221 / design doc `nyxid-event-gateway.md` / ADR-013.
//!
//! Flow (see `forward_event`):
//!
//! 1. Per-channel rate limit check (drop with [`AppError::RateLimited`]).
//! 2. Conversation lookup (404 on miss).
//! 3. Auth binding check — `auth_user.api_key_id` must match
//!    `conversation.agent_api_key_id` (401 on mismatch).
//! 4. Dedup LRU check — duplicates short-circuit and are logged as `deduped`.
//! 5. Load the `ApiKey` record for `callback_url` + `key_hash`.
//! 6. Build a `CallbackPayload` with `platform = "device"`.
//! 7. Measure latency around `channel_relay_service::forward_to_agent`.
//! 8. Append a metadata-only `ChannelEventLog` (ADR-013 compliant).
//!
//! This service never persists payload content.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use mongodb::bson::{Document, doc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::channel_conversation::{
    COLLECTION_NAME as CHANNEL_CONVERSATIONS, ChannelConversation,
};
use crate::models::channel_event_log::{
    COLLECTION_NAME as CHANNEL_EVENT_LOGS, ChannelEventLog, OUTCOME_CALLBACK_FAILED,
    OUTCOME_DEDUPED, OUTCOME_DELIVERED, OUTCOME_RATE_LIMITED,
};
use crate::models::channel_message::{COLLECTION_NAME as CHANNEL_MESSAGES, ChannelMessage};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::mw::rate_limit::PerChannelEventLimiter;
use crate::services::audit_service;
use crate::services::channel_relay_service::{
    self, CallbackAgent, CallbackContent, CallbackConversation, CallbackPayload, CallbackSender,
};
use crate::services::channel_routing_service;
use crate::services::event_dedup_cache::EventDedupCache;

/// Device event envelope as accepted from the HTTP client.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventEnvelope {
    pub event_id: String,
    pub source: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub payload: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// Successful outcome of `forward_event` — reported back to the HTTP client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardOutcome {
    Delivered,
    Deduped,
}

#[allow(clippy::too_many_arguments)]
pub async fn forward_event(
    db: &mongodb::Database,
    http_client: &reqwest::Client,
    config: &AppConfig,
    rate_limiter: &PerChannelEventLimiter,
    dedup_cache: &Arc<EventDedupCache>,
    auth_user: &AuthUser,
    conversation_id: &str,
    envelope: &EventEnvelope,
) -> AppResult<ForwardOutcome> {
    // 1. Auth extraction. Require a genuine API key (not a relay JWT or a
    //    session token). This runs before any DB work so that a
    //    misconfigured caller never touches the conversation store and
    //    cannot probe for existence via timing.
    //
    //    Defense in depth: the handler already gates on `AuthMethod::ApiKey`,
    //    but a direct service-level call from another code path must not
    //    accept a relay JWT that happens to carry an `api_key_id`.
    if auth_user.auth_method != AuthMethod::ApiKey {
        return Err(AppError::Unauthorized(
            "API key required for channel events".to_string(),
        ));
    }
    let api_key_id = auth_user
        .api_key_id
        .as_deref()
        .ok_or_else(|| AppError::Unauthorized("API key required for channel events".to_string()))?;

    // 2. Conversation lookup, scoped to this caller's API key AND to
    //    device-platform rows only.
    //
    //    The `platform: "device"` filter is load-bearing: without it, a
    //    caller bound to a bot-backed (telegram/discord/lark/feishu)
    //    conversation could POST to /channel-events/{bot_conversation_id}
    //    and synthesize a `platform="device"` ChannelMessage row whose
    //    parent conversation is still bot-backed. A later reply through
    //    /channel-relay/reply would then pass the device-guard (because
    //    `conversation.platform != "device"`) and dispatch through the
    //    PlatformAdapter, bypassing the one-way device-channel invariant
    //    and the Discord-interaction TTL shortening we rely on.
    //
    //    Collapsing platform, ownership, existence, and active state into
    //    one query preserves the opaque-401 property: all four miss
    //    reasons return the same error, so an attacker with a valid
    //    api_key cannot distinguish a foreign conversation from a
    //    nonexistent one or a bot conversation from a device one.
    //
    //    `is_active: true` mirrors `channel_routing_service::resolve_agent()`
    //    so device events respect the same off-switch as webhook-driven
    //    traffic. The rate limiter still runs *after* this check, so an
    //    unauthorized caller never burns a legitimate conversation's
    //    token bucket.
    let conversation = db
        .collection::<ChannelConversation>(CHANNEL_CONVERSATIONS)
        .find_one(conversation_lookup_filter(conversation_id, api_key_id))
        .await?
        .ok_or_else(|| {
            tracing::warn!(
                conversation_id = %conversation_id,
                provided_key = %api_key_id,
                "Channel event rejected: conversation not found, not bound to caller, or not a device channel"
            );
            AppError::Unauthorized(
                "conversation not found or not bound to this API key".to_string(),
            )
        })?;

    // Device conversations always have a concrete `platform_conversation_id`
    // (the conversation-creation handler rejects "*" / empty for device
    // channels). A wildcard value reaching this point would indicate an
    // orphaned bot-route being abused as an event target; reject defensively.
    debug_assert_ne!(conversation.platform_conversation_id, "*");
    if conversation.platform_conversation_id == "*" {
        return Err(AppError::ValidationError(
            "wildcard_conversation_not_supported: device events require a concrete \
             platform_conversation_id"
                .to_string(),
        ));
    }

    // 3. Per-channel rate limit. Only authenticated, authorized callers
    //    count against the bucket.
    if !rate_limiter.check(conversation_id) {
        tracing::warn!(
            conversation_id = %conversation_id,
            event_id = %envelope.event_id,
            "Channel event rate limit exceeded"
        );
        // Record the throttle so operators can distinguish rate-limited
        // drops from dedup hits or downstream failures when tracing bursts.
        // `None` upstream status (→ 0 sentinel) is correct: we never
        // touched the agent callback.
        write_log(
            db,
            conversation_id,
            api_key_id,
            envelope,
            None,
            0,
            OUTCOME_RATE_LIMITED,
        )
        .await;
        return Err(AppError::RateLimited);
    }

    // 4. Dedup check — read-only. We do NOT insert into the cache yet:
    //    if the forward fails, a client retry with the same event_id must
    //    be allowed through, not silently dropped as a duplicate. The
    //    cache is only marked after a successful delivery below.
    if dedup_cache.contains(conversation_id, &envelope.event_id) {
        // Dedup hits never reach the upstream agent — record `None` so
        // `upstream_status_code` is the 0 sentinel rather than a misleading
        // 200. The `outcome = "deduped"` field disambiguates from genuine
        // transport failures (which also use 0).
        write_log(
            db,
            conversation_id,
            api_key_id,
            envelope,
            None,
            0,
            OUTCOME_DEDUPED,
        )
        .await;
        audit(
            db,
            auth_user,
            conversation_id,
            envelope,
            ForwardOutcome::Deduped,
            0,
        );
        return Ok(ForwardOutcome::Deduped);
    }

    // 5. Load API key for callback URL + HMAC signing key.
    let api_key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": api_key_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("API key not found".to_string()))?;

    let callback_url = api_key.callback_url.as_deref().ok_or_else(|| {
        AppError::ChannelRelayFailed(
            "agent_callback_url_not_configured: target API key has no callback_url".to_string(),
        )
    })?;

    // 6. Carry forward the most recent inbound message's `thread_id`, if
    //    any, from a webhook-driven source. This covers both Discord
    //    deferred-interaction follow-up tokens (15-min / 5-follow-up
    //    window) and Telegram forum-topic ids (no TTL). See
    //    `lookup_recent_inbound_thread_id` for the kind-specific handling.
    //    For conversations with no recent thread_id, this returns `None`
    //    and the device-event row stores `thread_id = None`.
    let inherited_thread_id = lookup_recent_inbound_thread_id(db, conversation_id).await?;

    // 7. Persist a metadata-only ChannelMessage row so the agent can reply
    //    back through POST /api/v1/channel-relay/reply. Without this, the
    //    reply handler would 404 looking up the message by ID. Mirrors the
    //    webhook path's "persist before forward" ordering.
    let stored_message = channel_relay_service::store_device_event_message(
        db,
        conversation.channel_bot_id.as_deref(),
        conversation_id,
        &conversation.platform_conversation_id,
        &conversation.user_id,
        &envelope.event_id,
        &envelope.source,
        &envelope.event_type,
        api_key_id,
        inherited_thread_id,
    )
    .await?;

    // 7. Build the callback payload (platform = "device").
    //    Use the persisted NyxID message id, not the client-supplied
    //    event_id: the agent will echo it back in /channel-relay/reply and
    //    the handler resolves it via get_message() → ChannelMessage._id.
    let payload =
        build_device_callback_payload(&conversation, &api_key, envelope, &stored_message.id)?;

    // 8. Forward and measure latency.
    let started = std::time::Instant::now();
    let delivery = channel_relay_service::forward_to_agent(
        http_client,
        config,
        callback_url,
        &payload,
        &api_key.key_hash,
        None,
    )
    .await;
    let latency_ms = started.elapsed().as_millis() as i64;
    let upstream_status = delivery.http_status;

    match delivery.result {
        Ok(()) => {
            // Mark the event as seen *only* after a successful forward.
            // Concurrent duplicates that sneak past the read-only contains()
            // check are tolerated as best-effort at-least-once delivery.
            dedup_cache.insert_if_absent(conversation_id, &envelope.event_id);

            // Flip the ChannelMessage row from "pending" → "delivered" so
            // the bot-detail UI and /channel-relay/messages views reflect
            // the correct state. Mirrors the webhook path's behavior.
            if let Err(err) =
                channel_relay_service::update_callback_status(db, &stored_message.id, "delivered")
                    .await
            {
                tracing::warn!(
                    conversation_id = %conversation_id,
                    message_id = %stored_message.id,
                    error = %err,
                    "Failed to update callback_status to delivered"
                );
            }

            // Keep conversation activity metadata fresh so the bot-detail
            // UI and any recency checks treat active device conversations
            // as live (mirrors the regular webhook path).
            if let Err(err) = channel_routing_service::touch_conversation(db, conversation_id).await
            {
                tracing::warn!(
                    conversation_id = %conversation_id,
                    error = %err,
                    "Failed to update conversation last_message_at after event delivery"
                );
            }

            tracing::info!(
                conversation_id = %conversation_id,
                event_id = %envelope.event_id,
                upstream_status = ?upstream_status,
                latency_ms,
                "Channel event delivered"
            );
            write_log(
                db,
                conversation_id,
                api_key_id,
                envelope,
                upstream_status,
                latency_ms,
                OUTCOME_DELIVERED,
            )
            .await;
            audit(
                db,
                auth_user,
                conversation_id,
                envelope,
                ForwardOutcome::Delivered,
                upstream_status.map(i32::from).unwrap_or(0),
            );
            Ok(ForwardOutcome::Delivered)
        }
        Err(err) => {
            // Flip the ChannelMessage row from "pending" → "failed" so
            // retries don't accumulate phantom in-flight entries.
            if let Err(update_err) =
                channel_relay_service::update_callback_status(db, &stored_message.id, "failed")
                    .await
            {
                tracing::warn!(
                    conversation_id = %conversation_id,
                    message_id = %stored_message.id,
                    error = %update_err,
                    "Failed to update callback_status to failed"
                );
            }

            tracing::warn!(
                conversation_id = %conversation_id,
                event_id = %envelope.event_id,
                upstream_status = ?upstream_status,
                latency_ms,
                error = %err,
                "Channel event delivery failed"
            );
            write_log(
                db,
                conversation_id,
                api_key_id,
                envelope,
                upstream_status,
                latency_ms,
                OUTCOME_CALLBACK_FAILED,
            )
            .await;
            Err(err)
        }
    }
}

/// Build the filter used to look up the target conversation for a device
/// event.
///
/// **Invariants** (verify any change against the tests below — they are the
/// contract):
///
/// 1. `platform: "device"` — rejects bot-backed conversations even when
///    the caller's API key is correctly bound to them. Without this, a
///    Telegram/Discord/Lark/Feishu conversation could be used as a device
///    channel and an agent reply through `/channel-relay/reply` would
///    bypass the one-way device-channel invariant plus the shortened
///    Discord-interaction TTL we rely on for device-originated replies.
/// 2. `agent_api_key_id: api_key_id` — the caller must already be the
///    assigned agent for the conversation.
/// 3. `is_active: true` — mirrors the webhook resolver's off-switch.
///
/// All miss reasons fold into a single opaque 401 at the call site, so an
/// attacker holding a valid agent key cannot distinguish a nonexistent
/// channel from a foreign one or a bot channel from a device one.
fn conversation_lookup_filter(conversation_id: &str, api_key_id: &str) -> Document {
    doc! {
        "_id": conversation_id,
        "is_active": true,
        "agent_api_key_id": api_key_id,
        "platform": "device",
    }
}

/// Fetch the most recent **webhook-driven** inbound `ChannelMessage` for a
/// conversation and return its `thread_id`, for inheritance onto a newly
/// synthesized device-event row.
///
/// Covers two distinct routing-metadata kinds:
///
/// - **Discord deferred-interaction follow-up tokens**
///   (`interaction:{app}:{token}`). These expire after ~15 minutes and
///   must not be inherited from a stale source, so the `interaction:`
///   prefix triggers an age cap (2 minutes at copy time; the reply-side
///   check in `async_reply` tightens its window by a further 12 minutes
///   so combined `source_age + reply_delay` stays under Discord's TTL).
///
/// - **Telegram forum-topic ids** (numeric `message_thread_id` as string).
///   These have no TTL — a topic persists indefinitely — so any recent
///   inbound's thread_id is safe to carry forward.
///
/// Two filters work together to prevent stale-token poisoning:
///
/// 1. **`platform != "device"`** — without this, a previously injected
///    device-event row (which inherited the thread_id) would be the
///    "most recent inbound" and its token would be re-copied forever
///    even though the real source has long expired.
///
/// 2. **`thread_id exists`** — the filter runs in the MongoDB query
///    itself so we select the most recent inbound *with a thread_id*,
///    not the most recent inbound period. Without this, a later bland
///    inbound row would shadow a still-valid interaction or topic id
///    living on an earlier row.
///
/// Runs against the `(conversation_id, created_at desc)` index already
/// defined for `channel_messages`, so this is a cheap indexed lookup.
async fn lookup_recent_inbound_thread_id(
    db: &mongodb::Database,
    conversation_id: &str,
) -> AppResult<Option<String>> {
    let recent = db
        .collection::<ChannelMessage>(CHANNEL_MESSAGES)
        .find_one(doc! {
            "conversation_id": conversation_id,
            "direction": "inbound",
            "platform": { "$ne": "device" },
            "thread_id": { "$exists": true, "$ne": null },
        })
        .sort(doc! { "created_at": -1 })
        .await?;

    let Some(msg) = recent else {
        return Ok(None);
    };
    let Some(tid) = msg.thread_id else {
        return Ok(None);
    };

    // Discord interaction tokens have a ~15-minute TTL. Cap the inherited
    // token's source age at 2 minutes so the reply-side check (12 minutes
    // on device-event originals) bounds the combined age at 14 minutes
    // under Discord's hard limit. Telegram topic ids and other kinds have
    // no TTL.
    if tid.starts_with("interaction:") {
        const INTERACTION_COPY_WINDOW_SECS: i64 = 120;
        let age = Utc::now() - msg.created_at;
        if age > chrono::Duration::seconds(INTERACTION_COPY_WINDOW_SECS) {
            tracing::debug!(
                conversation_id = %conversation_id,
                age_secs = age.num_seconds(),
                "Skipping stale Discord interaction token inheritance"
            );
            return Ok(None);
        }
    }

    Ok(Some(tid))
}

fn build_device_callback_payload(
    conversation: &ChannelConversation,
    api_key: &ApiKey,
    envelope: &EventEnvelope,
    nyxid_message_id: &str,
) -> AppResult<CallbackPayload> {
    // Per design doc §4: content.text carries the full envelope JSON.
    let envelope_json = serde_json::to_string(envelope)
        .map_err(|e| AppError::Internal(format!("failed to serialize event envelope: {e}")))?;

    Ok(CallbackPayload {
        // NyxID-assigned message id so async replies via /channel-relay/reply
        // resolve to the persisted ChannelMessage. The client-supplied
        // `event_id` is preserved in `ChannelMessage.platform_message_id`.
        message_id: nyxid_message_id.to_string(),
        platform: "device".to_string(),
        reply_token: None,
        agent: CallbackAgent {
            api_key_id: api_key.id.clone(),
            name: api_key.name.clone(),
        },
        conversation: CallbackConversation {
            id: conversation.id.clone(),
            platform_id: conversation.platform_conversation_id.clone(),
            // Device conversations default their type to "device" at
            // creation time but callers may override (e.g. to distinguish
            // "camera" vs "sensor"). Honor the stored value.
            conversation_type: conversation.platform_conversation_type.clone(),
        },
        sender: CallbackSender {
            platform_id: envelope.source.clone(),
            display_name: None,
        },
        content: CallbackContent {
            content_type: envelope.event_type.clone(),
            text: Some(envelope_json),
            attachments: Vec::new(),
        },
        reply_to_message_id: None,
        reply_to_platform_message_id: None,
        thread_id: None,
        timestamp: envelope.timestamp.to_rfc3339(),
        raw_platform_data: None,
    })
}

async fn write_log(
    db: &mongodb::Database,
    conversation_id: &str,
    api_key_id: &str,
    envelope: &EventEnvelope,
    upstream_status_code: Option<u16>,
    latency_ms: i64,
    outcome: &str,
) {
    let log = ChannelEventLog {
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation_id.to_string(),
        api_key_id: api_key_id.to_string(),
        event_id: envelope.event_id.clone(),
        source: envelope.source.clone(),
        event_type: envelope.event_type.clone(),
        event_timestamp: envelope.timestamp,
        forwarded_at: Utc::now(),
        // 0 is a sentinel for transport errors where no HTTP response was
        // received. Other values are the actual agent status code.
        upstream_status_code: upstream_status_code.map(i32::from).unwrap_or(0),
        latency_ms,
        outcome: outcome.to_string(),
    };

    if let Err(err) = db
        .collection::<ChannelEventLog>(CHANNEL_EVENT_LOGS)
        .insert_one(&log)
        .await
    {
        tracing::warn!(
            conversation_id = %conversation_id,
            event_id = %envelope.event_id,
            error = %err,
            "Failed to write channel_event_log"
        );
    }
}

fn audit(
    db: &mongodb::Database,
    auth_user: &AuthUser,
    conversation_id: &str,
    envelope: &EventEnvelope,
    outcome: ForwardOutcome,
    upstream_status_code: i32,
) {
    audit_service::log_async(
        db.clone(),
        Some(auth_user.user_id.to_string()),
        "channel_event.forwarded".to_string(),
        Some(serde_json::json!({
            "conversation_id": conversation_id,
            "event_id": envelope.event_id,
            "source": envelope.source,
            "event_type": envelope.event_type,
            "outcome": match outcome {
                ForwardOutcome::Delivered => "delivered",
                ForwardOutcome::Deduped => "deduped",
            },
            "upstream_status_code": upstream_status_code,
        })),
        None,
        None,
        auth_user.api_key_id.clone(),
        auth_user.api_key_name.clone(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn envelope() -> EventEnvelope {
        EventEnvelope {
            event_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            source: "camera-analyzer".to_string(),
            event_type: "person_detected".to_string(),
            timestamp: Utc.with_ymd_and_hms(2026, 4, 8, 12, 0, 0).unwrap(),
            payload: Some(serde_json::json!({ "room": "living_room", "confidence": 0.95 })),
            metadata: None,
        }
    }

    fn conversation() -> ChannelConversation {
        ChannelConversation {
            id: "conv-1".to_string(),
            user_id: "user-1".to_string(),
            // Device channels have no backing bot. See NyxID#221 /
            // nyxid-event-gateway.md.
            channel_bot_id: None,
            platform: "device".to_string(),
            platform_conversation_id: "household-1".to_string(),
            platform_conversation_type: "device".to_string(),
            platform_sender_id: None,
            agent_api_key_id: "key-1".to_string(),
            default_agent: false,
            is_active: true,
            last_message_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn api_key() -> ApiKey {
        ApiKey {
            id: "key-1".to_string(),
            user_id: "user-1".to_string(),
            name: "test-agent".to_string(),
            key_prefix: "nyxid_ag_test".to_string(),
            key_hash: "0123456789abcdef".to_string(),
            scopes: String::new(),
            callback_url: Some("http://localhost/callback".to_string()),
            description: None,
            is_active: true,
            created_at: Utc::now(),
            last_used_at: None,
            expires_at: None,
            allow_all_services: true,
            allow_all_nodes: true,
            allowed_service_ids: Vec::new(),
            allowed_node_ids: Vec::new(),
            rate_limit_per_second: None,
            rate_limit_burst: None,
            platform: None,
        }
    }

    const TEST_NYXID_MSG_ID: &str = "nyxid-msg-uuid-1";

    #[test]
    fn build_payload_uses_device_platform() {
        let conv = conversation();
        let key = api_key();
        let env = envelope();
        let payload = build_device_callback_payload(&conv, &key, &env, TEST_NYXID_MSG_ID).unwrap();

        assert_eq!(payload.platform, "device");
        // message_id is the NyxID-assigned id, NOT the client event_id.
        // Async replies via /channel-relay/reply look up by this value.
        assert_eq!(payload.message_id, TEST_NYXID_MSG_ID);
        assert_eq!(payload.conversation.id, "conv-1");
        assert_eq!(payload.conversation.platform_id, "household-1");
        assert_eq!(payload.conversation.conversation_type, "device");
        assert_eq!(payload.sender.platform_id, "camera-analyzer");
        assert_eq!(payload.agent.api_key_id, "key-1");
        assert_eq!(payload.content.content_type, "person_detected");
        // The full envelope JSON (including the client event_id) must be
        // embedded in content.text so agents can still correlate back.
        let text = payload.content.text.unwrap();
        assert!(text.contains("\"event_id\""));
        assert!(text.contains("550e8400-e29b-41d4-a716-446655440000"));
        assert!(text.contains("\"payload\""));
        assert!(text.contains("living_room"));
    }

    #[test]
    fn payload_has_no_attachments_or_raw_data() {
        let payload = build_device_callback_payload(
            &conversation(),
            &api_key(),
            &envelope(),
            TEST_NYXID_MSG_ID,
        )
        .unwrap();
        assert!(payload.content.attachments.is_empty());
        assert!(payload.raw_platform_data.is_none());
        assert!(payload.thread_id.is_none());
    }

    #[test]
    fn envelope_accepts_optional_fields() {
        let json = r#"{
            "event_id": "abc",
            "source": "src",
            "type": "t",
            "timestamp": "2026-04-08T12:00:00Z"
        }"#;
        let parsed: EventEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.event_id, "abc");
        assert!(parsed.payload.is_none());
        assert!(parsed.metadata.is_none());
    }

    #[test]
    fn payload_embeds_both_payload_and_metadata() {
        let mut env = envelope();
        env.metadata = Some(serde_json::json!({ "analyzer_version": "1.0" }));
        let payload =
            build_device_callback_payload(&conversation(), &api_key(), &env, TEST_NYXID_MSG_ID)
                .unwrap();
        let text = payload.content.text.unwrap();
        // Both payload and metadata sections from the envelope must appear in
        // content.text, since ADR-013 forbids storing them separately.
        assert!(text.contains("\"payload\""));
        assert!(text.contains("\"metadata\""));
        assert!(text.contains("analyzer_version"));
        assert!(text.contains("living_room"));
    }

    #[test]
    fn payload_includes_event_timestamp_rfc3339() {
        let payload = build_device_callback_payload(
            &conversation(),
            &api_key(),
            &envelope(),
            TEST_NYXID_MSG_ID,
        )
        .unwrap();
        // RFC 3339 starts with YYYY-MM-DDTHH:MM:SS
        assert!(payload.timestamp.starts_with("2026-04-08T12:00:00"));
    }

    #[test]
    fn payload_agent_fields_come_from_api_key() {
        let payload = build_device_callback_payload(
            &conversation(),
            &api_key(),
            &envelope(),
            TEST_NYXID_MSG_ID,
        )
        .unwrap();
        assert_eq!(payload.agent.api_key_id, "key-1");
        assert_eq!(payload.agent.name, "test-agent");
    }

    #[test]
    fn lookup_filter_requires_platform_device() {
        // Regression for Codex review finding: without the platform="device"
        // clause, an agent bound to a Telegram/Discord/Lark/Feishu
        // conversation could POST to /channel-events/{bot_conversation_id}
        // and the resulting reply would bypass the one-way device-channel
        // invariant.
        let filter = conversation_lookup_filter("conv-1", "key-1");
        assert_eq!(filter.get_str("platform").unwrap(), "device");
    }

    #[test]
    fn lookup_filter_scopes_to_active_and_caller() {
        let filter = conversation_lookup_filter("conv-1", "key-1");
        assert_eq!(filter.get_str("_id").unwrap(), "conv-1");
        assert_eq!(filter.get_str("agent_api_key_id").unwrap(), "key-1");
        assert!(filter.get_bool("is_active").unwrap());
    }

    #[test]
    fn lookup_filter_has_no_extra_keys() {
        // Opaque-401 property: adding more conditions to this filter
        // without routing them through the same "not found or not bound"
        // error would create a distinguishable failure mode. Fail the
        // test when a future edit adds a key so the author is forced to
        // reason about the side effect.
        let filter = conversation_lookup_filter("c", "k");
        let mut keys: Vec<&str> = filter.keys().map(|s| s.as_str()).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["_id", "agent_api_key_id", "is_active", "platform"],
        );
    }

    #[test]
    fn message_id_is_not_the_client_event_id() {
        // Regression for the Codex finding: the agent must receive a
        // NyxID-assigned id (resolvable by get_message), not the
        // client-supplied envelope event_id.
        let env = envelope();
        let payload =
            build_device_callback_payload(&conversation(), &api_key(), &env, "fixed-nyxid-id")
                .unwrap();
        assert_eq!(payload.message_id, "fixed-nyxid-id");
        assert_ne!(payload.message_id, env.event_id);
    }
}
