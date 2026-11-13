//! Durable provider response protocol for native and negotiated relay sources.
use super::*;
use crate::{
    models::nyxbot_gateway_response::{
        COLLECTION_NAME as RESPONSES, GatewayResponse as StoredResponse,
    },
    services::{
        coordination_service::{EventDedupClaimResult, EventDedupStore},
        nyxbot_gateway_source as source, nyxbot_response_recovery as recovery,
    },
};
use axum::extract::Query;
use serde::Deserialize;

pub(super) fn respond(
    state: AppState,
    row: NyxbotChannel,
    headers: HeaderMap,
    request: Value,
    idempotency: &str,
) -> futures::future::BoxFuture<'static, Response> {
    Box::pin(respond_impl(
        state,
        row,
        headers,
        request,
        idempotency.to_owned(),
    ))
}

async fn respond_impl(
    state: AppState,
    row: NyxbotChannel,
    headers: HeaderMap,
    request: Value,
    idempotency: String,
) -> Response {
    // CMA retries only this exact refusal as proven pre-admission. A generic
    // 503 is an uncertain submission and must never be used for preparation.
    if row.gateway_native_phase.as_deref() == Some("preparing") {
        return provider_refusal::coded((
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "1")],
            Json(json!({"error":{"type":"invalid_request_error","code":"conversation_not_ready"}})),
        )
            .into_response(), "conversation_not_ready");
    }
    match Box::pin(accept(&state, &row, &request, &idempotency)).await {
        Ok(stored) => stream(state, row, headers, stored.response_id, None),
        Err(error) => {
            let code = if error.status_code().is_client_error() {
                "intake_refused"
            } else {
                "intake_unavailable"
            };
            let _=state.db.collection::<NyxbotChannel>(CHANNELS).update_one(
                doc! {"_id":&row.id,"binding_id":&row.binding_id,"gateway_source":bson::to_bson(&row.gateway_source).unwrap_or(bson::Bson::Null)},
                doc! {"$set":{"gateway_native_delivery":code}}).await;
            provider_refusal::error(error)
        }
    }
}
async fn accept(
    state: &AppState,
    row: &NyxbotChannel,
    request: &Value,
    idempotency: &str,
) -> AppResult<StoredResponse> {
    if !source::ready(row) {
        return Err(AppError::OAuthAuthorityUnavailable);
    }
    let binding = row
        .binding_id
        .as_deref()
        .ok_or(AppError::OAuthAuthorityUnavailable)?;
    let partition = request["conversation"]
        .as_str()
        .filter(|s| valid_partition(s))
        .ok_or_else(|| AppError::ValidationError("Invalid native conversation".into()))?;
    let response_id = format!(
        "resp_{}",
        &sha256_hex(format!("{}:{binding}:{idempotency}", row.id))[..32]
    );
    let request_hash = sha256_hex(request.to_string());
    if let Some(stored) = recovery::load(&state.db, row, &response_id).await? {
        if stored.request_hash != request_hash {
            return Err(AppError::Conflict("Response command changed".into()));
        }
        return Ok(stored);
    }
    let context = &request["event_context"];
    if context["conversation_id"].as_str() != Some(partition)
        || !source::provider_context(row, context)
    {
        return Err(AppError::ValidationError(
            "Invalid native event context".into(),
        ));
    }
    let activity = &context["activity"];
    if matches!(
        row.gateway_source.as_ref(),
        Some(crate::models::nyxbot_channel::GatewaySource::WhatsApp { .. })
    ) {
        let bot =
            crate::services::channel_bot_service::get_bot(&state.db, &row.channel_bot_id).await?;
        let config = crate::services::whatsapp_native::load(&state.db, &bot)
            .await?
            .ok_or_else(|| AppError::Forbidden("native_whatsapp_identity_unavailable".into()))?;
        if crate::services::whatsapp_native::verified_account(
            &config,
            &bot,
            row.gateway_channel_id.as_deref().unwrap_or_default(),
            context,
        )
        .is_none()
        {
            return Err(AppError::Forbidden(
                "native_whatsapp_identity_unavailable".into(),
            ));
        }
    }
    let metadata = recovery::metadata(&request["metadata"])?;
    let event_id = activity["event_id"].as_str().unwrap_or_default();
    if metadata.get("cmaeg.event").map(String::as_str) != Some(event_id)
        || metadata.get("cmaeg.actor").map(String::as_str) != activity["actor"]["id"].as_str()
        || metadata.get("cmaeg.channel").map(String::as_str)
            != Some(&sha256_hex(row.gateway_channel_id.as_deref().unwrap_or_default())[..16])
        || !metadata["cmaeg.partition"]
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
        || metadata["cmaeg.partition"].len() != 16
    {
        return Err(AppError::ValidationError(
            "Invalid native command metadata".into(),
        ));
    }
    // Native IDs are parsed by the selected source's closed validator.
    let (namespace, native_id) = source::message_identity(row, activity)?;
    let scope = serde_json::to_string(&(Some(&row.channel_bot_id), bot_owner(row)))
        .map_err(|_| AppError::OAuthAuthorityUnavailable)?;
    let claim = match EventDedupStore::claim(
        &state.db,
        namespace,
        &scope,
        native_id,
        Duration::from_secs(30),
    )
    .await?
    {
        EventDedupClaimResult::Claimed(claim) => Some(claim),
        EventDedupClaimResult::Duplicate => {
            if !EventDedupStore::is_committed(&state.db, namespace, &scope, native_id).await? {
                return Err(AppError::Conflict(
                    "Native message admission in progress".into(),
                ));
            }
            None
        }
    };
    let now = Utc::now();
    let event_key = sha256_hex(format!("native-response:{}", response_id));
    let event = NyxbotEvent {
        trace_request_id: crate::turn_trace::ingress_request_id(),
        id: event_key.clone(),
        channel_id: row.id.clone(),
        user_id: row.user_id.clone(),
        partition: partition.into(),
        event_id: event_id.into(),
        relay_message_id: None,
        custom_binding_id: None,
        event_context_ciphertext: Some(
            state
                .encryption_keys
                .encrypt(
                    &serde_json::to_vec(context)
                        .map_err(|_| AppError::OAuthAuthorityUnavailable)?,
                )
                .await?,
        ),
        status: if claim.is_some() {
            "running"
        } else {
            "refused"
        }
        .into(),
        conversation_id: None,
        turn_id: None,
        created_at: now,
        expires_at: now + ChronoDuration::hours(recovery::RETENTION_HOURS),
        media: None,
        source_media_declared: Some(
            activity["kind"]["attachments"]
                .as_array()
                .map_or(0, Vec::len),
        ),
        native_bot_budget_claimed: false,
        retry_request_sha256: Some(request_hash.clone()),
        delivery: None,
        resolved_thread_id: None,
        resolved_conversation_id: None,
    };
    let identity = format!(
        "nyxbot_{}",
        &sha256_hex(format!("{}:{binding}:{partition}", row.id))[..32]
    );
    let mut payload = recovery::Payload::new(&response_id, &identity, metadata);
    if claim.is_none() {
        payload.finish(None, false);
    }
    let stored = StoredResponse {
        id: Uuid::new_v4().to_string(),
        response_id,
        user_id: row.user_id.clone(),
        channel_id: row.id.clone(),
        binding_id: binding.into(),
        partition: partition.into(),
        event_key: event_key.clone(),
        request_hash,
        conversation_id: None,
        terminal: claim.is_none(),
        revision: 0,
        payload_ciphertext: payload.seal(&state.encryption_keys).await?,
        created_at: now,
        expires_at: event.expires_at,
    };
    let db = state.db.clone();
    let inserted = stored.clone();
    let event = event.clone();
    let tx_row = row.clone();
    let tx_claim = claim.clone();
    let mut session = db.client().start_session().await?;
    let result=session.start_transaction().and_run2(async move |session| {
        let row = &tx_row;
        let result:AppResult<()> = async {
            let discord = matches!(&row.gateway_source, Some(crate::models::nyxbot_channel::GatewaySource::Discord { .. }));
            // Clear intake uncertainty atomically with the existing admission
            // fence, never a genuine delivery error. This adds no DB operation.
            let admission: mongodb::options::UpdateModifications = if discord {
                let clear = doc! {"$and":[{"$eq":["$delivery_status","unconfirmed"]},
                    {"$eq":["$delivery_error","discord_native_intake_unconfirmed"]}]};
                vec![doc! {"$set":{
                    "native_admission_revision":{"$add":[{"$ifNull":["$native_admission_revision",0_i64]},1_i64]},
                    "delivery_status":{"$cond":[&clear,"unknown","$delivery_status"]},
                    "delivery_error":{"$cond":[&clear,"$$REMOVE","$delivery_error"]}
                }}].into()
            } else {
                doc! {"$inc":{"native_admission_revision":1_i64}}.into()
            };
            // Contend with transport changes, binding revocation and erasure.
            let fenced=db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id,"transport":"gateway","status":"active",
                "binding_id":&row.binding_id,"agent_api_key_id":&row.agent_api_key_id,"gateway_native_phase":"ready",
                "gateway_channel_id":&row.gateway_channel_id,"gateway_source":bson::to_bson(&row.gateway_source).map_err(|_|AppError::OAuthAuthorityUnavailable)?,
                "gateway_setup_release_id":bson::Bson::Null,"pending_agent_api_key_id":bson::Bson::Null},
                admission).session(&mut *session).await?;
            if fenced.matched_count!=1 {return Err(AppError::OAuthAuthorityUnavailable);}
            let owner=db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME).update_one(
                doc! {"_id":&row.user_id,"is_active":true,"platform_personal_deleting":{"$ne":true}},
                doc! {"$inc":{"channel_metadata_admission_revision":1_i64}}).session(&mut *session).await?;
            if owner.matched_count!=1 {return Err(AppError::Forbidden("Owner unavailable".into()));}
            let mut bot_filter = doc! {"_id":&row.channel_bot_id,"user_id":bot_owner(row),"is_active":true,"status":{"$ne":"suspended"}};
            if let Some(crate::models::nyxbot_channel::GatewaySource::Discord { bot_id }) = &row.gateway_source {
                bot_filter.extend(doc! {"platform":"discord","platform_bot_id":bot_id,"status":{"$in":["active","pending_webhook"]}});
            }
            // Relay record_inbound already records this per message. Fold the
            // native equivalent into the mandatory bot fence instead of adding
            // another write or throttling the authoritative bot timestamp.
            let mut bot_update = doc! {"$inc":{"native_admission_revision":1_i64}};
            if discord {
                bot_update.insert("$set", doc! {"diagnostics.last_inbound_at":bson::DateTime::now(),
                    "diagnostics.last_inbound_source":"native_discord_gateway"});
            }
            let bot=db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(
                bot_filter.clone(), bot_update).session(&mut *session).await?;
            if bot.matched_count!=1 {return Err(AppError::Forbidden("Bot unavailable".into()));}
            if discord {
                // Only a first/current-hand-over or >60s-old observation writes
                // the channel evidence. Filter live storage, not the caller's
                // snapshot; concurrent admissions cannot undo newer evidence.
                db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {
                    "_id":&row.id,"binding_id":&row.binding_id,
                    "$or":[
                        {"gateway_intake.binding_id":{"$ne":&row.binding_id}},
                        {"gateway_intake.last_inbound_at":bson::Bson::Null},
                        {"gateway_intake.last_inbound_at":{"$lt":bson::DateTime::from_chrono(now-ChronoDuration::seconds(60))}},
                        {"$expr":{"$lt":["$gateway_intake.last_inbound_at","$gateway_intake.handed_over_at"]}}
                    ]}, vec![doc! {"$set":{"gateway_intake":{
                        "binding_id":{"$literal":&row.binding_id},
                        "handed_over_at":{"$cond":[{"$eq":["$gateway_intake.binding_id",{"$literal":&row.binding_id}]},
                            {"$ifNull":["$gateway_intake.handed_over_at",bson::Bson::Null]},bson::Bson::Null]},
                        "last_inbound_at":bson::DateTime::from_chrono(now)
                    }}}]).session(&mut *session).await?;
                bot_filter.insert("status", "pending_webhook");
                db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(
                    bot_filter, doc! {"$set":{"status":"active","updated_at":bson::DateTime::now(),
                    "diagnostics.last_inbound_at":bson::DateTime::now(),"diagnostics.last_inbound_source":"native_discord_gateway"}})
                    .session(&mut *session).await?;
            }
            let partition=db.collection::<NyxbotThread>(THREADS).update_one(doc! {"channel_id":&row.id,"partition":&inserted.partition},
                doc! {"$inc":{"native_admission_revision":1_i64}}).session(&mut *session).await?;
            if partition.matched_count!=1 {return Err(AppError::NotFound("Conversation not found".into()));}
            if let Some(claim)=&tx_claim {
                let committed=db.collection::<bson::Document>(crate::models::coordination::EVENT_DEDUP_COLLECTION_NAME)
                    .update_one(doc! {"_id":&claim.id,"claim_id":&claim.claim_id,"state":"claimed","$expr":{"$gt":["$expires_at","$$NOW"]}},
                        vec![doc! {"$set":{"state":"committed","updated_at":"$$NOW","expires_at":{"$dateAdd":{"startDate":"$$NOW","unit":"day","amount":30}}}}])
                    .session(&mut *session).await?;
                if committed.matched_count!=1 {return Err(AppError::Conflict("Native admission changed".into()));}
            }
            db.collection::<NyxbotEvent>(EVENTS).insert_one(&event).session(&mut *session).await?;
            db.collection::<StoredResponse>(RESPONSES).insert_one(&inserted).session(&mut *session).await?;
            Ok(())
        }.await;
        crate::services::api_key_mutation_service::transaction_result(result)
    }).await.map_err(crate::services::api_key_mutation_service::map_transaction_error);
    if let Err(error) = result {
        if let Some(claim) = &claim {
            let _ = EventDedupStore::release(&state.db, claim).await;
        }
        // An uncertain commit is recovered by the exact command, never executed twice.
        if let Some(saved) = recovery::load(&state.db, row, &stored.response_id).await? {
            if saved.request_hash != stored.request_hash {
                return Err(AppError::Conflict("Response command changed".into()));
            }
            return Ok(saved);
        }
        return Err(error);
    }
    if claim.is_none() {
        return Ok(stored);
    }
    // Acceptance (including frame zero) is durable before platform reads or
    // turn admission. CMA observes it within its 20s submission budget, then
    // drops the POST stream and uses GET recovery. Never tie execution to that
    // observer, or resubmit it from a recovery read after a process restart.
    let task_state = state.clone();
    let task_row = row.clone();
    let task_stored = stored.clone();
    let task_context = context.clone();
    crate::turn_trace::spawn(async move {
        // Keep the provider dispatch future off the Tokio worker stack. It
        // nests channel admission, Mongo transactions and platform history,
        // and native tests exercise it with the CI-sized 1.5 MiB stack.
        if let Err(error) =
            Box::pin(execute(&task_state, &task_row, &task_stored, &task_context)).await
        {
            tracing::warn!(code = error.error_code(), response_id = %task_stored.response_id,
                "Native provider dispatch did not settle; durable recovery retains authority");
        }
    });
    Ok(stored)
}

pub(super) fn native_sender(activity: &Value) -> Sender<'_> {
    use crate::models::channel_thread::ThreadSenderKind as Kind;
    Sender {
        original_kind: Some(match activity["actor"]["kind"].as_str() {
            Some("human") => Kind::Human,
            Some("bot") => Kind::Bot,
            _ => Kind::Unknown,
        }),
        id: activity["actor"]["id"].as_str().unwrap_or_default(),
        display_name: None,
        // Preserve the deployed native-chat behavior. Permission enrollment
        // separately requires the provider's positive human classification.
        kind: Kind::Human,
        media: vec![],
        original_message: true,
        owner_eligible: true,
    }
}

async fn execute(
    state: &AppState,
    row: &NyxbotChannel,
    stored: &StoredResponse,
    context: &Value,
) -> AppResult<()> {
    let activity = &context["activity"];
    let partition = stored.partition.as_str();
    let event_key = stored.event_key.as_str();
    let mut thread_reply = false;
    let sender = native_sender(activity);
    let text = if row.platform == "whatsapp"
        && crate::services::whatsapp_native::has_unavailable_media(activity)
    {
        let text = activity["kind"]["text"].as_str().unwrap_or_default();
        if text.trim().is_empty() {
            crate::services::whatsapp_native::MEDIA_UNAVAILABLE.to_owned()
        } else {
            format!(
                "{text}\n\n{}",
                crate::services::whatsapp_native::MEDIA_UNAVAILABLE
            )
        }
    } else {
        activity["kind"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    let result = Box::pin(gateway_inbound(
        state,
        row,
        partition,
        activity,
        context["event_ref"].as_str(),
        &sender,
        &text,
        true,
        event_key,
        &mut thread_reply,
    ))
    .await;
    match result {
        Ok(Inbound::Turn(_)) | Ok(Inbound::Queued) => {}
        Ok(Inbound::Reply(text)) => {
            recovery::finish(
                &state.db,
                &state.encryption_keys,
                stored,
                Some(&text),
                false,
            )
            .await?
        }
        Ok(Inbound::Silent(_)) => {
            recovery::finish(&state.db, &state.encryption_keys, stored, None, false).await?
        }
        Ok(Inbound::Busy) => {
            recovery::finish(
                &state.db,
                &state.encryption_keys,
                stored,
                Some("NyxBot is busy. Please try again shortly."),
                false,
            )
            .await?
        }
        Err(_) => {
            // A turn/queue commit may have won despite an uncertain return.
            let event = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"_id":&event_key})
                .await?;
            if event.is_some_and(|e| e.turn_id.is_none() && e.status != "queued") {
                recovery::finish(&state.db, &state.encryption_keys, stored, None, true).await?;
            }
        }
    }
    state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id,"binding_id":&row.binding_id},
        doc! {"$set":{"delivery_status":"ok","delivery_error":bson::Bson::Null,"delivery_seen_at":bson::DateTime::now()}}).await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct Cursor {
    pub(super) starting_after: Option<String>,
}
fn cursor(value: Option<&str>) -> AppResult<Option<u64>> {
    value
        .map(|s| {
            s.parse::<u64>()
                .ok()
                .filter(|n| *n < u64::MAX && n.to_string() == s)
                .ok_or_else(|| AppError::ValidationError("Invalid replay cursor".into()))
        })
        .transpose()
}
pub async fn snapshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    provider_refusal::scope(
        &state.db,
        provider_refusal::Endpoint::ResponseGet,
        snapshot_inner(State(state.clone()), Path(id), headers),
    )
    .await
}
async fn snapshot_inner(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let (row, _) = match provider_channel(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    if let Err(error) = relay_responses::claim_replay(&state, &row, &id).await {
        return provider_refusal::error(error);
    }
    match current(&state, &row, &id).await {
        Ok(Some((_, body, trace))) => {
            if let Some(trace) = trace {
                crate::turn_trace::associate(trace);
            }
            crate::turn_trace::response(trace, crate::turn_trace::Stage::Replay, async {
                Ok(([("cache-control", "no-store")], Json(body.snapshot)).into_response())
            })
            .await
            .unwrap_or_else(provider_refusal::error)
        }
        Ok(None) => problem(StatusCode::NOT_FOUND, "response_not_found"),
        Err(e) => provider_refusal::error(e),
    }
}
pub async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
    headers: HeaderMap,
) -> Response {
    provider_refusal::scope(
        &state.db,
        provider_refusal::Endpoint::Events,
        events_inner(State(state.clone()), Path(id), Query(query), headers),
    )
    .await
}
async fn events_inner(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
    headers: HeaderMap,
) -> Response {
    let (row, _) = match provider_channel(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let after = match cursor(query.starting_after.as_deref()) {
        Ok(v) => v,
        Err(e) => return provider_refusal::error(e),
    };
    if let Err(error) = relay_responses::claim_replay(&state, &row, &id).await {
        return provider_refusal::error(error);
    }
    match current(&state, &row, &id).await {
        Ok(Some((_, body, trace))) if after.is_none_or(|n| n < body.frames.len() as u64) => {
            if let Some(trace) = trace {
                crate::turn_trace::associate(trace);
            }
            stream(state, row, headers, id, after)
        }
        Ok(Some(_)) => problem(StatusCode::BAD_REQUEST, "invalid_replay_cursor"),
        Ok(None) => problem(StatusCode::NOT_FOUND, "response_not_found"),
        Err(e) => provider_refusal::error(e),
    }
}
async fn current(
    state: &AppState,
    row: &NyxbotChannel,
    id: &str,
) -> AppResult<
    Option<(
        StoredResponse,
        recovery::Payload,
        Option<crate::turn_trace::TurnContext>,
    )>,
> {
    if row.transport != "gateway"
        || row.status != "active"
        || (row.gateway_source.is_some() && !source::ready(row))
    {
        return Ok(None);
    }
    let Some(stored) = recovery::load(&state.db, row, id).await? else {
        return Ok(None);
    };
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&stored.event_key})
        .await?;
    if let Some(event) = event.as_ref() {
        provider_refusal::partition_known();
        provider_refusal::event(&event.event_id);
    }
    let trace = event
        .as_ref()
        .and_then(|event| event.turn_id.as_deref())
        .and_then(crate::turn_trace::TurnContext::for_turn);
    if let Some(event) = event.as_ref()
        && let (Some(delivery), Some(conversation)) = (&event.delivery, &event.conversation_id)
    {
        let history = engine::get(
            &state.db,
            crate::services::org_channel_service::event_actor(event),
            conversation,
        )
        .await?;
        if late_delivery::eligible(state, event, delivery, &history)
            .await?
            .is_none()
        {
            if state.db.collection::<bson::Document>(RESPONSES).update_one(
                doc! {"_id":&stored.id,"replay_refusal_recorded":{"$ne":true}},
                doc! {"$set":{"replay_refusal_recorded":true,"replay_refusal_reason":"authority_changed"}},
            ).await?.modified_count == 1 {
                audit(state, &row.user_id, "nyxbot_channel_answer_delivery", json!({
                    "event_id":stored.event_key,"conversation_id":stored.conversation_id,"turn_id":event.turn_id,
                    "transport":"gateway","outcome":"refused","reason":"authority_changed"
                })).await;
            }
            return Ok(None);
        }
    }
    drop(event);
    let event_key = &stored.event_key;
    recovery::refresh(
        &state.db,
        &state.encryption_keys,
        &stored,
        |text| async move {
            late_delivery::stream_images(
                state,
                event_key,
                &text,
                std::time::Instant::now() + Duration::from_secs(135),
            )
            .await
        },
    )
    .await?;
    let Some(stored) = recovery::load(&state.db, row, id).await? else {
        return Ok(None);
    };
    let mut payload = recovery::payload(&state.encryption_keys, &stored).await?;
    if row.gateway_source.is_none() && stored.terminal {
        let event = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"_id":&stored.event_key})
            .await?;
        if event
            .as_ref()
            .and_then(|e| e.delivery.as_ref())
            .is_some_and(|d| matches!(d.state.as_str(), "sending" | "sent" | "unknown" | "refused"))
        {
            relay_responses::suppress_output(&mut payload);
        }
    }
    Ok(Some((stored, payload, trace)))
}
pub(super) fn stream(
    state: AppState,
    row: NyxbotChannel,
    headers: HeaderMap,
    id: String,
    after: Option<u64>,
) -> Response {
    let response_id = id.clone();
    let observer = relay_responses::Observer::new(&state, &row, &id);
    let stream = async_stream::stream! {
        let _observer = observer;
        let deadline=tokio::time::Instant::now()+Duration::from_secs(if row.gateway_source.is_some() {55} else {570});
        let mut next=after.map_or(0,|n|n+1);
        let mut trace_operation: Option<crate::turn_trace::Operation> = None;
        loop {
            // Recheck the fresh key and binding, including while a stream waits.
            let live=provider_channel(&state,&headers).await;
            let result=match live {
                Ok((live,_)) if live.id==row.id && live.binding_id==row.binding_id => current(&state,&live,&id).await,
                _=>Err(AppError::OAuthAuthorityUnavailable),
            };
            match result {
                Ok(Some((stored,mut body, trace)))=>{
                    if trace_operation.is_none() && trace.is_some() {
                        trace_operation = Some(crate::turn_trace::Operation::start(trace, crate::turn_trace::Stage::Replay));
                    }
                    if stored.terminal && !relay_responses::stream_output(&state, &row, &stored, body.snapshot["status"].as_str().unwrap_or("failed")).await.unwrap_or(false) {
                        relay_responses::suppress_output(&mut body);
                    }
                    for frame in body.frames.iter().skip(next as usize) {
                        let seq=frame["sequence_number"].as_u64().unwrap_or_default();
                        next=seq+1;
                        yield Ok::<_,Infallible>(axum::response::sse::Event::default()
                            .id(seq.to_string()).event(frame["type"].as_str().unwrap_or_default()).data(frame.to_string()));
                    }
                    if stored.terminal {
                        if let Some(operation) = &mut trace_operation {
                            operation.finish(crate::turn_trace::Outcome::Streamed, Some(200), None);
                        }
                        break;
                    }
                },
                _=>{yield Ok(axum::response::sse::Event::default().event("error").data("{\"type\":\"error\",\"code\":\"provider_unavailable\"}"));break;},
            }
            if tokio::time::Instant::now()>=deadline {break;}
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };
    let mut response = axum::response::Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response();
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().expect("static header"));
    response
        .headers_mut()
        .insert("x-accel-buffering", "no".parse().expect("static header"));
    response.headers_mut().insert(
        "x-cma-response-id",
        response_id.parse().expect("response identifier"),
    );
    response
}

/// Unrelated asynchronous updates retain their original event, including a
/// followed thread. A newer message never substitutes a destination capability.
pub(super) fn send_update<'a>(
    state: &'a AppState,
    row: &'a NyxbotChannel,
    origin: &'a ChannelOrigin,
    conversation: &'a str,
    text: &'a str,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        match send_update_impl(state, row, origin, conversation, text, None).await? {
            "confirmed" => Ok(()),
            _ => Err(AppError::NotFound("Native reply source unavailable".into())),
        }
    })
}

/// An independently claimed permission result has its own stable occurrence.
pub(super) async fn send_occurrence(
    state: &AppState,
    row: &NyxbotChannel,
    origin: &ChannelOrigin,
    conversation: &str,
    text: &str,
    occurrence: &str,
) -> AppResult<&'static str> {
    send_update_impl(state, row, origin, conversation, text, Some(occurrence)).await
}

async fn send_update_impl(
    state: &AppState,
    row: &NyxbotChannel,
    origin: &ChannelOrigin,
    conversation: &str,
    text: &str,
    occurrence: Option<&str>,
) -> AppResult<&'static str> {
    let unavailable = || AppError::NotFound("Native reply source unavailable".into());
    if !source::ready(row) {
        return Err(unavailable());
    }
    let event_id = origin.native_event_id.as_deref().ok_or_else(unavailable)?;
    let response = state
        .db
        .collection::<StoredResponse>(RESPONSES)
        .find_one(doc! {
            "event_key":event_id,"channel_id":&row.id,"user_id":&row.user_id,
            "binding_id":&row.binding_id,"conversation_id":conversation,
            "expires_at":{"$gt":bson::DateTime::now()},
        })
        .await?
        .ok_or_else(unavailable)?;
    let event = state.db.collection::<NyxbotEvent>(EVENTS).find_one(doc! {
        "_id":&response.event_key,"user_id":&row.user_id,"conversation_id":conversation,
        "created_at":{"$gt":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::minutes(29))},
    }).await?.ok_or_else(unavailable)?;
    let trace = event
        .turn_id
        .as_deref()
        .and_then(crate::turn_trace::TurnContext::for_turn);
    let mut operation =
        crate::turn_trace::Operation::start(trace, crate::turn_trace::Stage::Channel);
    let result: AppResult<&'static str> = crate::turn_trace::scope(operation.context(), async {
        if event
            .delivery
            .as_ref()
            .is_none_or(|d| d.origin != *origin || d.transport != "gateway")
        {
            return Err(unavailable());
        }
        let history = engine::get(
            &state.db,
            crate::services::org_channel_service::event_actor(&event),
            conversation,
        )
        .await?;
        if late_delivery::eligible(
            state,
            &event,
            event.delivery.as_ref().ok_or_else(unavailable)?,
            &history,
        )
        .await?
        .is_none()
        {
            return Err(unavailable());
        }
        let ciphertext = event
            .event_context_ciphertext
            .as_ref()
            .ok_or_else(unavailable)?;
        let context: Value = serde_json::from_slice(&Zeroizing::new(
            state.encryption_keys.decrypt(ciphertext).await?,
        ))
        .map_err(|_| unavailable())?;
        if !source::provider_context(row, &context) {
            return Err(unavailable());
        }
        let reference = context["event_ref"].as_str().ok_or_else(unavailable)?;
        let key = decrypt_agent_key(state, row)
            .await?
            .ok_or_else(unavailable)?;
        let reply_parts = if matches!(
            row.gateway_source.as_ref(),
            Some(crate::models::nyxbot_channel::GatewaySource::WhatsApp { .. })
        ) {
            let reply = crate::services::channel_platform::OutboundReply {
                attachments: Vec::new(),
                text: Some(text.to_owned()),
                reply_to_platform_message_id: None,
                metadata: None,
            };
            crate::services::whatsapp_native::text_reply(&reply)?;
            crate::services::channel_reply_plan::text_parts(&reply, "whatsapp")?
        } else {
            vec![crate::services::channel_platform::OutboundReply {
                attachments: Vec::new(),
                text: Some(bounded_reply(text)),
                reply_to_platform_message_id: None,
                metadata: None,
            }]
        };
        let mut outcome = "reply_confirmed";
        for (index, part) in reply_parts.into_iter().enumerate() {
            let mut part_trace = crate::turn_trace::Operation::part(
                crate::turn_trace::TurnContext::current(), index,
            );
            let mut body = json!({"text":part.text.unwrap_or_default()});
            // A local followed reply chain is not necessarily a Discord thread.
            // CMA accepts native-thread mode only for the original thread channel.
            if context["activity"]["conversation"]["thread_id"].is_string() {
                body["thread_reply"] = json!(true);
            }
            let idempotency = occurrence.map(|id| format!("{id}:{index}"));
            let result = crate::turn_trace::scope(part_trace.context(), gateway_call(
                state,
                reqwest::Method::POST,
                &format!("/events/{}/replies", urlencode(reference)),
                &key,
                Some(&body),
                idempotency.as_deref(),
            ))
            .await;
            // Transport success alone is not a platform send receipt. Do not retry
            // an unknown outcome or report a rejected send as successful.
            outcome = match &result {
                Ok(result) if (200..300).contains(&result.status) => {
                    match result.body["delivery"]["outcome"].as_str() {
                        Some("confirmed") => "reply_confirmed",
                        Some("rejected") => "reply_rejected",
                        Some("unsupported") => "reply_unsupported",
                        Some("rate_limited") => "reply_rate_limited",
                        _ => "reply_unknown",
                    }
                }
                Ok(result) if (500..600).contains(&result.status) => "reply_unknown",
                Ok(_) => "reply_refused",
                Err(_) => "reply_unknown",
            };
            part_trace.finish(
                match outcome {
                    "reply_confirmed" => crate::turn_trace::Outcome::Confirmed,
                    "reply_rejected" | "reply_unsupported" | "reply_rate_limited" | "reply_refused" => crate::turn_trace::Outcome::Rejected,
                    _ => crate::turn_trace::Outcome::Unknown,
                },
                None,
                None,
            );
            if outcome != "reply_confirmed" {
                break;
            }
        }
        state.db.collection::<NyxbotChannel>(CHANNELS).update_one(
            doc! {"_id":&row.id,"binding_id":&row.binding_id,"transport":"gateway","gateway_native_phase":"ready"},
            doc! {"$set":{"gateway_native_delivery":outcome}},
        ).await?;
        Ok(match outcome {
            "reply_confirmed" => "confirmed",
            "reply_rejected" | "reply_unsupported" | "reply_rate_limited" | "reply_refused" => {
                "rejected"
            }
            _ => "unknown",
        })
    })
    .await;
    operation.finish(
        match &result {
            Ok("confirmed") => crate::turn_trace::Outcome::Confirmed,
            Ok("rejected") => crate::turn_trace::Outcome::Rejected,
            _ => crate::turn_trace::Outcome::Unknown,
        },
        None,
        None,
    );
    result
}
