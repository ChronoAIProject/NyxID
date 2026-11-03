//! Durable response recovery for authenticated relay-sourced gateway events.
use super::*;
use crate::{
    models::nyxbot_gateway_response::{COLLECTION_NAME as RESPONSES, GatewayResponse},
    services::{nyxbot_gateway_service as gateway, nyxbot_response_recovery as recovery},
};
use std::sync::Arc;

pub(super) async fn prepare(
    state: &AppState,
    row: &NyxbotChannel,
    event: &NyxbotEvent,
    request: &Value,
    response_id: &str,
) -> AppResult<Option<GatewayResponse>> {
    // Old gateways and management probes retain the existing live-stream path.
    if request["metadata"].is_null()
        || row.binding_id.is_none()
        || request["event_context"]["activity"]["source"]["type"] != "nyxid_relay"
    {
        return Ok(None);
    }
    // Readiness and CMA discovery are compatibility gates, not rollout consent.
    // Resolve for the linking person, including their live org overrides.
    // Off preserves the legacy path even when recovery metadata is malformed.
    if !feature_flag_service::personal_flag_enabled(
        &state.db,
        &row.user_id,
        feature_flag_service::NYXBOT_RELAY_RESPONSE_RECOVERY_FLAG_KEY,
    )
    .await?
    {
        return Ok(None);
    }
    let Ok(metadata) = recovery::metadata(&request["metadata"]) else {
        return Ok(None);
    };
    let activity = &request["event_context"]["activity"];
    if metadata["cmaeg.event"] != event.event_id
        || Some(metadata["cmaeg.actor"].as_str()) != activity["actor"]["id"].as_str()
        || metadata["cmaeg.channel"]
            != sha256_hex(row.gateway_channel_id.as_deref().unwrap_or_default())[..16]
        || metadata["cmaeg.partition"].len() != 16
        || !metadata["cmaeg.partition"]
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(AppError::ValidationError(
            "Invalid recovery command identity".into(),
        ));
    }
    if crate::services::security_readiness_service::require(&state.db, recovery::RELAY_READINESS)
        .await
        .is_err()
    {
        return Ok(None);
    }
    // Thread-follow already has a distinct target-selection contract. Keep its
    // existing delivery path until the resolved thread choice can be frozen
    // before the durable created frame, rather than changing it after emission.
    if crate::services::channel_thread_service::gateway::candidate(row, activity) {
        return Ok(None);
    }
    let base = gateway::gateway_base(state).await?;
    let creator = gateway::creator_bearer(state, &row.user_id)?;
    if !advertised(&base, &creator).await {
        return Ok(None);
    }
    let binding = row.binding_id.as_deref().expect("checked binding");
    let identity = format!(
        "nyxbot_{}",
        &sha256_hex(format!("{}:{binding}:{}", row.id, event.partition))[..32]
    );
    let payload = recovery::Payload::new(response_id, &identity, metadata);
    let stored = GatewayResponse {
        id: Uuid::new_v4().to_string(),
        response_id: response_id.into(),
        user_id: row.user_id.clone(),
        channel_id: row.id.clone(),
        binding_id: binding.into(),
        partition: event.partition.clone(),
        event_key: event.id.clone(),
        request_hash: sha256_hex(request.to_string()),
        conversation_id: None,
        terminal: false,
        revision: 0,
        payload_ciphertext: payload.seal(&state.encryption_keys).await?,
        created_at: event.created_at,
        expires_at: event.created_at + ChronoDuration::hours(recovery::RETENTION_HOURS),
    };
    let db = state.db.clone();
    let admitted = stored.clone();
    let mut session = db.client().start_session().await?;
    let result = session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let changed = db.collection::<NyxbotEvent>(EVENTS).update_one(doc! {
                "_id":&admitted.event_key,"user_id":&admitted.user_id,"turn_id":bson::Bson::Null,
                "status":{"$in":["running","resuming"]},"gateway_response_id":bson::Bson::Null,
            },doc! {"$set":{"gateway_response_id":&admitted.response_id}}).session(&mut *session).await?;
            if changed.matched_count != 1 { return Err(AppError::Conflict("Response admission changed".into())) }
            db.collection::<GatewayResponse>(RESPONSES).insert_one(&admitted).session(&mut *session).await?;
            Ok(())
        }.await;
        crate::services::api_key_mutation_service::transaction_result(result)
    }).await.map_err(crate::services::api_key_mutation_service::map_transaction_error);
    if let Err(error) = result {
        // A lost commit acknowledgement never causes another dispatch.
        if let Some(saved) = recovery::load(&state.db, row, response_id).await?
            && saved.request_hash == stored.request_hash
        {
            return Ok(Some(saved));
        }
        return Err(error);
    }
    Ok(Some(stored))
}

pub(super) fn capable(caps: &Value) -> bool {
    caps["relay_authoring"]["response_recovery_providers"]
        .as_array()
        .is_some_and(|providers| providers.iter().any(|p| p == "nyxbot"))
}

async fn advertised(base: &str, bearer: &str) -> bool {
    CAPABILITIES.advertised(base, bearer).await
}

#[derive(Default)]
struct CapabilityCache {
    entries: std::sync::Mutex<std::collections::HashMap<String, CapabilityEntry>>,
}

struct CapabilityEntry {
    used: tokio::time::Instant,
    // Only the public capability result is cached; never the creator bearer.
    flight: Arc<tokio::sync::Mutex<Option<(tokio::time::Instant, bool)>>>,
}

static CAPABILITIES: std::sync::LazyLock<CapabilityCache> =
    std::sync::LazyLock::new(CapabilityCache::default);

impl CapabilityCache {
    async fn advertised(&self, base: &str, bearer: &str) -> bool {
        let flight = {
            let mut entries = self.entries.lock().expect("capability cache");
            if !entries.contains_key(base)
                && entries.len() >= 64
                && let Some(oldest) = entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.used)
                    .map(|(base, _)| base.clone())
            {
                entries.remove(&oldest);
            }
            let entry = entries
                .entry(base.to_owned())
                .or_insert_with(|| CapabilityEntry {
                    used: tokio::time::Instant::now(),
                    flight: Arc::new(tokio::sync::Mutex::new(None)),
                });
            entry.used = tokio::time::Instant::now();
            entry.flight.clone()
        };
        // Serialize refreshes for one gateway, without holding the map lock
        // across I/O or blocking another gateway's discovery.
        let mut cached = flight.lock().await;
        if let Some((expires, value)) = *cached
            && expires > tokio::time::Instant::now()
        {
            return value;
        }
        let (value, seconds) = match tokio::time::timeout(
            Duration::from_secs(2),
            gateway::gateway_call_at_base(
                base,
                reqwest::Method::GET,
                "/channel-capabilities",
                bearer,
                None,
                None,
            ),
        )
        .await
        {
            Ok(Ok(caps)) if caps.status == 200 => {
                let supported = capable(&caps.body);
                (supported, if supported { 300 } else { 30 })
            }
            // A short error backoff coalesces concurrent misses without
            // treating an unavailable or unauthorized probe as a negative TTL.
            _ => (false, 1),
        };
        *cached = Some((
            tokio::time::Instant::now() + Duration::from_secs(seconds),
            value,
        ));
        value
    }
}

#[cfg(test)]
pub(super) fn clear_capabilities(base: &str) {
    CAPABILITIES.entries.lock().unwrap().remove(base);
}

pub(super) async fn settle_admission(
    state: &AppState,
    stored: &GatewayResponse,
    result: &AppResult<Inbound>,
    _thread_reply: bool,
) {
    let current = state
        .db
        .collection::<GatewayResponse>(RESPONSES)
        .find_one(doc! {"_id":&stored.id})
        .await
        .ok()
        .flatten();
    let stored = current.as_ref().unwrap_or(stored);
    let (text, failed) = match result {
        Ok(Inbound::Turn(_)) | Ok(Inbound::Queued) => return,
        Ok(Inbound::Reply(text)) => (Some(text.as_str()), false),
        Ok(Inbound::Silent) => (None, false),
        Ok(Inbound::Busy) => (
            Some("NyxBot is busy. This message was not started. Please try again shortly."),
            false,
        ),
        Err(_) => {
            let event = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"_id":&stored.event_key})
                .await;
            if !matches!(event, Ok(Some(ref event)) if event.turn_id.is_none() && event.status != "queued")
            {
                return;
            }
            (None, true)
        }
    };
    if let Err(error) =
        recovery::finish(&state.db, &state.encryption_keys, stored, text, failed).await
    {
        // The durable response remains pending for the sweep. Never drop a
        // failed terminal write without a content-free diagnostic.
        tracing::warn!(event_id = %stored.event_key, code = error.error_code(),
            reason = "response_settlement_deferred", "Channel response settlement deferred");
    }
}

/// Observation is not delivery. An observer disconnect may hand an unclaimed
/// answer to the old late sender. A successful recovery read claims it first,
/// fencing that sender atomically (including before turn admission).
pub(super) async fn claim_replay(state: &AppState, row: &NyxbotChannel, id: &str) -> AppResult<()> {
    if row.gateway_source.is_some() {
        return Ok(());
    }
    let Some(stored) = recovery::load(&state.db, row, id).await? else {
        return Ok(());
    };
    state.db.collection::<NyxbotEvent>(EVENTS).update_one(doc! {
        "_id":&stored.event_key,"channel_id":&row.id,"user_id":&row.user_id,
        "delivery.state":{"$in":[bson::Bson::Null,"waiting","pending","streamed","replay"]},
    }, vec![doc! {"$set":{"gateway_replay_claimed":true,
        "delivery":{"$cond":[{"$eq":[{"$ifNull":["$delivery",bson::Bson::Null]},bson::Bson::Null]},
            "$$REMOVE",{"$mergeObjects":["$delivery",{"state":"replay"}]}]}}}]).await?;
    Ok(())
}

pub(super) struct Observer {
    state: AppState,
    row: NyxbotChannel,
    response: String,
}
impl Observer {
    pub(super) fn new(state: &AppState, row: &NyxbotChannel, response: &str) -> Option<Self> {
        row.gateway_source.is_none().then(|| Self {
            state: state.clone(),
            row: row.clone(),
            response: response.into(),
        })
    }
}
impl Drop for Observer {
    fn drop(&mut self) {
        let state = self.state.clone();
        let row = self.row.clone();
        let id = self.response.clone();
        tokio::spawn(async move {
            if let Ok(Some(stored)) = recovery::load(&state.db, &row, &id).await {
                // Also fence admission which has not created delivery metadata yet.
                let _ = state
                    .db
                    .collection::<NyxbotEvent>(EVENTS)
                    .update_one(
                        doc! {"_id":&stored.event_key},
                        doc! {"$set":{"gateway_observer_detached":true}},
                    )
                    .await;
                let _ =
                    crate::services::channel_turn_delivery::detach(&state.db, &stored.event_key)
                        .await;
                late_delivery::process(&state, &stored.event_key).await;
            }
        });
    }
}

/// The POST observer claims before exposing any terminal text. GET recovery
/// already owns `replay`; a late sender that won is never echoed as another send.
pub(super) async fn stream_output(
    state: &AppState,
    row: &NyxbotChannel,
    stored: &GatewayResponse,
    status: &str,
) -> AppResult<bool> {
    if row.gateway_source.is_some() {
        return Ok(true);
    }
    if crate::services::channel_turn_delivery::claim_stream(&state.db, &stored.event_key, status)
        .await?
    {
        let event = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"_id":&stored.event_key})
            .await?;
        audit(
            state,
            &row.user_id,
            "nyxbot_channel_answer_delivery",
            json!({
                "event_id":stored.event_key,"conversation_id":stored.conversation_id,
                "turn_id":event.and_then(|event| event.turn_id),
                "transport":"gateway","outcome":"streamed","reason":"platform_delivery_unconfirmed"
            }),
        )
        .await;
        return Ok(true);
    }
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&stored.event_key})
        .await?;
    Ok(event.is_some_and(|e| {
        e.delivery
            .as_ref()
            .is_none_or(|d| matches!(d.state.as_str(), "replay" | "streamed"))
    }))
}

pub(super) fn suppress_output(body: &mut recovery::Payload) {
    body.snapshot["output"] = json!([]);
    body.snapshot["status"] = json!("completed");
    body.snapshot
        .as_object_mut()
        .expect("snapshot")
        .remove("error");
    body.frames.retain(|frame| {
        matches!(
            frame["type"].as_str(),
            Some(
                "response.created"
                    | "response.in_progress"
                    | "response.completed"
                    | "response.failed"
            )
        )
    });
    for (sequence, frame) in body.frames.iter_mut().enumerate() {
        frame["sequence_number"] = json!(sequence);
    }
    if let Some(last) = body.frames.last_mut() {
        last["type"] = json!("response.completed");
        last["response"] = body.snapshot.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relay_recovery_requires_advertised_provider_support() {
        assert!(!capable(&json!({})));
        assert!(!capable(
            &json!({"relay_authoring":{"response_recovery_providers":["cma"]}})
        ));
        assert!(capable(
            &json!({"relay_authoring":{"response_recovery_providers":["nyxbot"]}})
        ));
    }

    #[tokio::test]
    async fn relay_recovery_discovery_has_a_short_fallback_deadline() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let gateway = MockServer::start().await;
        Mock::given(path("/channel-capabilities"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(3))
                    .set_body_json(
                        json!({"relay_authoring":{"response_recovery_providers":["nyxbot"]}}),
                    ),
            )
            .expect(1)
            .mount(&gateway)
            .await;
        assert!(
            !CapabilityCache::default()
                .advertised(&gateway.uri(), "test")
                .await
        );
    }

    #[tokio::test]
    async fn relay_recovery_discovery_coalesces_requests_and_expires_errors_quickly() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        for supported in [true, false] {
            let gateway = MockServer::start().await;
            let base = gateway.uri();
            let cache = CapabilityCache::default();
            Mock::given(path("/channel-capabilities"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_delay(Duration::from_millis(25))
                        .set_body_json(if supported {
                            json!({"relay_authoring":{"response_recovery_providers":["nyxbot"]}})
                        } else {
                            json!({})
                        }),
                )
                .expect(1)
                .mount(&gateway)
                .await;
            let values =
                futures::future::join_all((0..8).map(|_| cache.advertised(&base, "test"))).await;
            assert!(values.into_iter().all(|value| value == supported));
            assert_eq!(cache.advertised(&base, "test").await, supported);
            gateway.verify().await;
        }
        let gateway = MockServer::start().await;
        let cache = CapabilityCache::default();
        Mock::given(path("/channel-capabilities"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&gateway)
            .await;
        assert!(!cache.advertised(&gateway.uri(), "test").await);
        assert!(!cache.advertised(&gateway.uri(), "test").await);
        gateway.verify().await;
        gateway.reset().await;
        Mock::given(path("/channel-capabilities"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"relay_authoring":{"response_recovery_providers":["nyxbot"]}}),
            ))
            .expect(1)
            .mount(&gateway)
            .await;
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(cache.advertised(&gateway.uri(), "test").await);
        gateway.verify().await;
    }
}
