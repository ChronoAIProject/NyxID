//! Guest waits use the existing source event and delivery barrier. They must
//! never enter AssistantConversation.pending_events: old replicas wake that
//! queue with owner authority. The sealed payload contains no credentials.
use super::*;
use crate::services::assistant_guest_model_setup as setup;

#[derive(serde::Serialize, serde::Deserialize)]
struct WaitingGuest {
    conversation: String,
    agent: String,
    profile: String,
    text: String,
    note: Option<String>,
    title: Option<String>,
    question: Option<String>,
    question_key: Option<String>,
    origin: ChannelOrigin,
    #[serde(default)]
    reply_channel: Option<ChannelOrigin>,
}

/// Returns None for an already-ready lane (and for shared recipients). The
/// caller keeps its stream while waiting; the sweep can resume after a crash.
pub(super) async fn wait(
    state: &AppState,
    channel: &NyxbotChannel,
    start: &TurnStart,
) -> AppResult<Option<Inbound>> {
    if !start.guest {
        return Ok(None);
    }
    let Some(agent_id) = start.agent_id.as_deref() else {
        return Ok(None);
    };
    let agent =
        crate::services::assistant_team_service::agent(&state.db, &channel.user_id, agent_id)
            .await?;
    if !setup::eligible(&state.db, &channel.user_id, &agent).await? {
        return Ok(None);
    }
    let existing = if let Some(id) = start.conversation_id.as_deref() {
        Some(engine::get(&state.db, &channel.user_id, id).await?)
    } else {
        None
    };
    if let Some(row) = &existing
        && (engine::live_turn(row, Utc::now()).is_some()
            || setup::valid_thread_route(state, row).await?)
    {
        return Ok(None);
    }
    let profile = existing
        .as_ref()
        .map(|r| r.model.as_str())
        .unwrap_or_else(|| start.model.as_deref().unwrap_or(&agent.model));
    if setup::kick(state, &channel.user_id, agent_id, profile, false).await? {
        return Ok(None);
    }
    // Preserve the existing no-effect busy response when the channel pool is
    // already full. Setup may prepare the model, but must not turn that refusal
    // into a newly accepted source or consume the human retry's identity.
    let Some(permit) = state
        .direct_chat_limiter
        .try_acquire_pool(
            "assistant_channel",
            &channel.user_id,
            crate::services::assistant_team_service::CHANNEL_TURNS_PER_OWNER,
        )
        .await?
    else {
        return Ok(Some(Inbound::Busy));
    };
    drop(permit);
    let event = start
        .channel_event_id
        .as_deref()
        .ok_or(AppError::AssistantTurnRequired)?;
    let conversation = start
        .conversation_id
        .as_deref()
        .or(start.new_id.as_deref())
        .ok_or(AppError::AssistantTurnRequired)?;
    let payload = WaitingGuest {
        conversation: conversation.into(),
        agent: agent_id.into(),
        profile: profile.into(),
        text: start.text.clone(),
        note: start.note.clone(),
        title: start.title.clone(),
        question: start.question.clone(),
        question_key: start.question_key.clone(),
        origin: start
            .channel
            .clone()
            .or_else(|| start.reply_channel.clone())
            .ok_or(AppError::AssistantTurnRequired)?,
        reply_channel: start.reply_channel.clone(),
    };
    let bytes = Zeroizing::new(
        serde_json::to_vec(&payload)
            .map_err(|_| AppError::Internal("Guest queue unavailable".into()))?,
    );
    let ciphertext = state.encryption_keys.encrypt(&bytes).await?;
    let deadline = Utc::now() + ChronoDuration::seconds(60);
    let context = (
        state.db.clone(),
        channel.user_id.clone(),
        event.to_owned(),
        conversation.to_owned(),
        ciphertext,
        deadline,
    );
    let accepted=crate::services::transaction_service::run_db_only(&state.db,&context,|tx,(db,owner,event,conversation,ciphertext,deadline)|Box::pin(async move {
        crate::services::erasure_admission::fence_in_session(db,owner,tx).await?;
        let events=db.collection::<bson::Document>(EVENTS);
        if events.count_documents(doc!{"user_id":owner,"status":"queued","guest_setup_payload":{"$exists":true}}).session(&mut *tx).await?>=32 {return Ok(false);}
        Ok(events.update_one(doc!{"_id":event,"user_id":owner,"turn_id":bson::Bson::Null,"status":{"$in":["running","resuming"]}},
            doc!{"$set":{"status":"queued","guest_setup_conversation_id":conversation,"guest_setup_deadline":bson::DateTime::from_chrono(*deadline),
                "guest_setup_payload":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext.clone()}}}).session(tx).await?.matched_count==1)
    })).await?;
    if !accepted {
        return Ok(Some(Inbound::Busy));
    }
    loop {
        if let Some(outcome) = Box::pin(resume(state, event)).await? {
            return Ok(Some(outcome));
        }
        if Utc::now() >= deadline + ChronoDuration::seconds(5) {
            crate::services::channel_turn_delivery::detach(&state.db, event).await?;
            return Ok(Some(Inbound::Queued));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn resume(state: &AppState, id: &str) -> AppResult<Option<Inbound>> {
    let events = state.db.collection::<bson::Document>(EVENTS);
    let claim = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    let Some(event) = events.find_one_and_update(
        doc!{"_id":id,"status":"queued","turn_id":bson::Bson::Null,"guest_setup_payload":{"$exists":true},
            "$or":[{"guest_setup_lease":null},{"guest_setup_lease":{"$lte":now}}]},
        doc!{"$set":{"guest_setup_claim":&claim,"guest_setup_lease":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::seconds(10))}}
    ).await? else {
        let bound=events.find_one(doc!{"_id":id,"turn_id":{"$type":"string"}}).projection(doc!{"_id":1}).await?.is_some();
        if bound {crate::services::channel_turn_delivery::detach(&state.db,id).await?;return Ok(Some(Inbound::Queued));}
        return Ok(None);
    };
    let outcome = match Box::pin(resume_inner(state, &event)).await {
        Err(
            AppError::NotFound(_)
            | AppError::Forbidden(_)
            | AppError::AssistantTurnRequired
            | AppError::ChannelBotNotFound(_)
            | AppError::ChannelBotInactive(_)
            | AppError::ErasurePreparing,
        ) => {
            retire(state, id).await?;
            Ok(Some(Inbound::Silent))
        }
        outcome => outcome,
    };
    events
        .update_one(
            doc! {"_id":id,"guest_setup_claim":claim},
            doc! {"$unset":{"guest_setup_lease":"","guest_setup_claim":""}},
        )
        .await?;
    outcome
}

async fn retire(state: &AppState, id: &str) -> AppResult<()> {
    state.db.collection::<bson::Document>(EVENTS).update_one(
        doc!{"_id":id,"status":"queued","turn_id":bson::Bson::Null},
        doc!{"$set":{"status":"refused","delivery.state":"refused"},"$unset":{"guest_setup_payload":""}}
    ).await?;
    Ok(())
}

async fn resume_inner(state: &AppState, event: &bson::Document) -> AppResult<Option<Inbound>> {
    let invalid = || AppError::Internal("Guest queue unavailable".into());
    let id = event.get_str("_id").map_err(|_| invalid())?;
    let owner = event.get_str("user_id").map_err(|_| invalid())?;
    let bytes = Zeroizing::new(
        state
            .encryption_keys
            .decrypt(
                event
                    .get_binary_generic("guest_setup_payload")
                    .map_err(|_| invalid())?,
            )
            .await?,
    );
    let payload: WaitingGuest = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let channel = load_channel(
        state,
        owner,
        event.get_str("channel_id").map_err(|_| invalid())?,
    )
    .await?;
    let thread = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(
            doc! {"channel_id":&channel.id,"partition":&payload.origin.partition,"user_id":owner},
        )
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    let source: NyxbotEvent = bson::from_document(event.clone()).map_err(|_| invalid())?;
    let live_agent = thread.agent_id.as_deref().or(channel.agent_id.as_deref());
    let agent_matches = payload.origin.thread.is_some()
        || requested_agent(state, owner, live_agent).await?.id == payload.agent;
    let allowed = channel.status == "active"
        && source.expires_at > Utc::now()
        && source.delivery.as_ref().is_some_and(|d| {
            d.guest
                && d.transport == channel.transport
                && d.source_route_api_key_id.as_deref() == Some(&channel.route_api_key_id)
                && d.source_route_id == channel.route_id
                && (d.origin.thread.is_some()
                    || crate::services::channel_turn_delivery::guest_source_admitted(
                        &channel, &thread, d,
                    ))
        })
        && agent_matches;
    if !allowed {
        retire(state, id).await?;
        return Ok(Some(Inbound::Silent));
    }
    if payload.origin.thread.is_some() {
        crate::services::channel_thread_follow_service::validate_delivery(
            &state.db,
            owner,
            &payload.origin,
            &payload.conversation,
        )
        .await?;
    }
    let expired = event
        .get_datetime("guest_setup_deadline")
        .is_ok_and(|at| at.to_chrono() <= Utc::now());
    if expired {
        setup::expire_wait(state, owner, &payload.agent, &payload.profile).await?;
    }
    let record = setup::record(&state.db, owner, &payload.agent, &payload.profile).await?;
    let failed = record
        .as_ref()
        .is_some_and(|r| r.get_str("status") == Ok("failed"));
    if !expired
        && !failed
        && !setup::kick(state, owner, &payload.agent, &payload.profile, false).await?
    {
        return Ok(None);
    }
    // Ordinary begin_turn rechecks live agent, org, key, source and thread
    // authority and binds this event atomically. Competing workers cannot run it twice.
    let exists = engine::get(&state.db, owner, &payload.conversation)
        .await
        .is_ok();
    let mut start = TurnStart::from(&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: exists.then(|| payload.conversation.clone()),
        agent_id: Some(payload.agent),
        text: payload.text,
        model: Some(payload.profile),
        access_mode: None,
    });
    start.new_id = (!exists).then_some(payload.conversation);
    start.guest = true;
    start.origin = TurnOrigin::Channel;
    start.channel = Some(payload.origin);
    start.reply_channel = payload.reply_channel;
    start.channel_event_id = Some(id.into());
    start.note = payload.note;
    start.title = payload.title;
    start.question = payload.question;
    start.question_key = payload.question_key;
    match Box::pin(start_server_turn(
        state,
        owner,
        start,
        Pool::Channel { owner },
    ))
    .await?
    {
        Started::Turn { receiver, .. } => Ok(Some(Inbound::Turn(receiver))),
        Started::Busy | Started::PoolFull => Ok(None),
    }
}

pub(crate) async fn sweep(state: &AppState) -> AppResult<()> {
    let events:Vec<bson::Document> = state.db.collection(EVENTS)
        .find(doc!{"status":"queued","turn_id":bson::Bson::Null,"guest_setup_deadline":{"$exists":true},"guest_setup_payload":{"$exists":true}})
        .projection(doc!{"_id":1}).sort(doc!{"created_at":1}).limit(32).await?.try_collect().await?;
    for event in events {
        if let Ok(id) = event.get_str("_id") {
            crate::services::channel_turn_delivery::detach(&state.db, id).await?;
            let _ = Box::pin(resume(state, id)).await;
        }
    }
    Ok(())
}
