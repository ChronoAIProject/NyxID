//! Native response protocol. Legacy relay streams never enter this module.
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

pub(super) async fn respond(
    state: AppState,
    row: NyxbotChannel,
    headers: HeaderMap,
    request: Value,
    idempotency: &str,
) -> Response {
    // CMA retries only this exact refusal as proven pre-admission. A generic
    // 503 is an uncertain submission and must never be used for preparation.
    if row.gateway_native_phase.as_deref() == Some("preparing") {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "1")],
            Json(json!({"error":{"type":"invalid_request_error","code":"conversation_not_ready"}})),
        )
            .into_response();
    }
    match accept(&state, &row, &request, idempotency).await {
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
            error.into_response()
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
        id: event_key.clone(),
        channel_id: row.id.clone(),
        user_id: row.user_id.clone(),
        partition: partition.into(),
        event_id: event_id.into(),
        relay_message_id: None,
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
            // Contend with transport changes, binding revocation and erasure.
            let fenced=db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id,"transport":"gateway","status":"active",
                "binding_id":&row.binding_id,"agent_api_key_id":&row.agent_api_key_id,"gateway_native_phase":"ready",
                "gateway_setup_release_id":bson::Bson::Null,"pending_agent_api_key_id":bson::Bson::Null},
                doc! {"$inc":{"native_admission_revision":1_i64}}).session(&mut *session).await?;
            if fenced.matched_count!=1 {return Err(AppError::OAuthAuthorityUnavailable);}
            let owner=db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME).update_one(
                doc! {"_id":&row.user_id,"is_active":true,"platform_personal_deleting":{"$ne":true}},
                doc! {"$inc":{"channel_metadata_admission_revision":1_i64}}).session(&mut *session).await?;
            if owner.matched_count!=1 {return Err(AppError::Forbidden("Owner unavailable".into()));}
            let bot=db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(
                doc! {"_id":&row.channel_bot_id,"user_id":bot_owner(row),"is_active":true,"status":{"$ne":"suspended"}},
                doc! {"$inc":{"native_admission_revision":1_i64}}).session(&mut *session).await?;
            if bot.matched_count!=1 {return Err(AppError::Forbidden("Bot unavailable".into()));}
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
    tokio::spawn(async move {
        if let Err(error) = execute(&task_state, &task_row, &task_stored, &task_context).await {
            tracing::warn!(code = error.error_code(), response_id = %task_stored.response_id,
                "Native provider dispatch did not settle; durable recovery retains authority");
        }
    });
    Ok(stored)
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
    let sender = Sender {
        id: activity["actor"]["id"].as_str().unwrap_or_default(),
        display_name: None,
        kind: crate::models::channel_thread::ThreadSenderKind::Human,
        media: vec![],
        original_message: true,
        owner_eligible: true,
    };
    let result = Box::pin(gateway_inbound(
        state,
        row,
        partition,
        activity,
        context["event_ref"].as_str(),
        &sender,
        activity["kind"]["text"].as_str().unwrap_or_default(),
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
        Ok(Inbound::Silent) => {
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
    let (row, _) = match provider_channel(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match current(&state, &row, &id).await {
        Ok(Some((_, body))) => {
            ([("cache-control", "no-store")], Json(body.snapshot)).into_response()
        }
        Ok(None) => problem(StatusCode::NOT_FOUND, "response_not_found"),
        Err(e) => e.into_response(),
    }
}
pub async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<Cursor>,
    headers: HeaderMap,
) -> Response {
    let after = match cursor(query.starting_after.as_deref()) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let (row, _) = match provider_channel(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match current(&state, &row, &id).await {
        Ok(Some((_, body))) if after.is_none_or(|n| n < body.frames.len() as u64) => {
            stream(state, row, headers, id, after)
        }
        Ok(Some(_)) => problem(StatusCode::BAD_REQUEST, "invalid_replay_cursor"),
        Ok(None) => problem(StatusCode::NOT_FOUND, "response_not_found"),
        Err(e) => e.into_response(),
    }
}
async fn current(
    state: &AppState,
    row: &NyxbotChannel,
    id: &str,
) -> AppResult<Option<(StoredResponse, recovery::Payload)>> {
    if !source::ready(row) {
        return Ok(None);
    }
    let Some(stored) = recovery::load(&state.db, row, id).await? else {
        return Ok(None);
    };
    if let Some(event) = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&stored.event_key})
        .await?
        && let (Some(delivery), Some(conversation)) = (&event.delivery, &event.conversation_id)
    {
        let history = engine::get(&state.db, &row.user_id, conversation).await?;
        if late_delivery::eligible(state, &event, delivery, &history)
            .await?
            .is_none()
        {
            return Ok(None);
        }
    }
    recovery::refresh(&state.db, &state.encryption_keys, &stored).await?;
    let Some(stored) = recovery::load(&state.db, row, id).await? else {
        return Ok(None);
    };
    let payload = recovery::payload(&state.encryption_keys, &stored).await?;
    Ok(Some((stored, payload)))
}
fn stream(
    state: AppState,
    row: NyxbotChannel,
    headers: HeaderMap,
    id: String,
    after: Option<u64>,
) -> Response {
    let response_id = id.clone();
    let stream = async_stream::stream! {
        let deadline=tokio::time::Instant::now()+Duration::from_secs(55);
        let mut next=after.map_or(0,|n|n+1);
        loop {
            // Recheck the fresh key and binding, including while a stream waits.
            let live=provider_channel(&state,&headers).await;
            let result=match live {
                Ok((live,_)) if live.id==row.id && live.binding_id==row.binding_id => current(&state,&live,&id).await,
                _=>Err(AppError::OAuthAuthorityUnavailable),
            };
            match result {
                Ok(Some((stored,body)))=>{
                    for frame in body.frames.iter().skip(next as usize) {
                        let seq=frame["sequence_number"].as_u64().unwrap_or_default();
                        let bytes=format!("id: {seq}\nevent: {}\ndata: {}\n\n",frame["type"].as_str().unwrap_or_default(),frame);
                        next=seq+1;
                        yield Ok::<_,Infallible>(Bytes::from(bytes));
                    }
                    if stored.terminal {break;}
                },
                _=>{yield Ok(Bytes::from_static(b"event: error\ndata: {\"type\":\"error\",\"code\":\"provider_unavailable\"}\n\n"));break;},
            }
            if tokio::time::Instant::now()>=deadline {break;}
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };
    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-store")
        .header("x-cma-response-id", response_id)
        .body(Body::from_stream(stream))
        .expect("static response")
}

/// Unrelated asynchronous updates retain their original event, including a
/// followed thread. A newer message never substitutes a destination capability.
pub(super) async fn send_update(
    state: &AppState,
    row: &NyxbotChannel,
    origin: &ChannelOrigin,
    conversation: &str,
    text: &str,
) -> AppResult<()> {
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
    if event
        .delivery
        .as_ref()
        .is_none_or(|d| d.origin != *origin || d.transport != "gateway")
    {
        return Err(unavailable());
    }
    let history = engine::get(&state.db, &row.user_id, conversation).await?;
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
    let mut body = json!({"text":bounded_reply(text)});
    // A local followed reply chain is not necessarily a Discord thread.
    // CMA accepts native-thread mode only for the original thread channel.
    if context["activity"]["conversation"]["thread_id"].is_string() {
        body["thread_reply"] = json!(true);
    }
    let result = gateway_call(
        state,
        reqwest::Method::POST,
        &format!("/events/{}/replies", urlencode(reference)),
        &key,
        Some(&body),
        None,
    )
    .await;
    // Transport success alone is not a platform send receipt. Do not retry an
    // unknown outcome or report a rejected/unsupported send as successful.
    let outcome = match &result {
        Ok(result) if (200..300).contains(&result.status) => {
            match result.body["delivery"]["outcome"].as_str() {
                Some("confirmed") => "reply_confirmed",
                Some("rejected") => "reply_rejected",
                Some("unsupported") => "reply_unsupported",
                Some("rate_limited") => "reply_rate_limited",
                _ => "reply_unknown",
            }
        }
        Ok(_) => "reply_refused",
        Err(_) => "reply_unknown",
    };
    state.db.collection::<NyxbotChannel>(CHANNELS).update_one(
        doc! {"_id":&row.id,"binding_id":&row.binding_id,"transport":"gateway","gateway_native_phase":"ready"},
        doc! {"$set":{"gateway_native_delivery":outcome}},
    ).await?;
    if outcome != "reply_confirmed" {
        return Err(unavailable());
    }
    Ok(())
}
