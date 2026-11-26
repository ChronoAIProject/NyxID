//! Guest waits use the existing source event and delivery barrier. They must
//! never enter AssistantConversation.pending_events: old replicas wake that
//! queue with owner authority. The sealed payload contains no credentials.
use super::*;
use crate::services::assistant_guest_model_setup as setup;

use crate::models::nyxbot_channel::WaitingGuest;

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
    let guest = start
        .channel_context
        .as_ref()
        .and_then(|context| context.channel_guest.as_ref());
    if let Some(guest) = guest {
        crate::services::channel_guest_access::revalidate(&state.db, guest).await?;
    }
    let org_origin = start
        .channel
        .as_ref()
        .or(start.reply_channel.as_ref())
        .filter(|o| o.org_execution.is_some());
    let actor = org_origin
        .and_then(|o| o.org_execution.as_ref())
        .map_or(channel.user_id.as_str(), |e| e.actor_id.as_str());
    let Some(agent_id) = start.agent_id.as_deref() else {
        return Ok(None);
    };
    let agent = crate::services::assistant_team_service::agent(&state.db, actor, agent_id).await?;
    if !setup::eligible(&state.db, actor, &agent).await? {
        return Ok(None);
    }
    let existing = if let Some(id) = start.conversation_id.as_deref() {
        Some(engine::get(&state.db, actor, id).await?)
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
    let ready = if let Some(origin) = org_origin {
        setup::kick_org_channel(state, origin, profile).await?
    } else {
        setup::kick(state, actor, agent_id, profile, false).await?
    };
    if ready {
        return Ok(None);
    }
    // Preserve the existing no-effect busy response when the channel pool is
    // already full. Setup may prepare the model, but must not turn that refusal
    // into a newly accepted source or consume the human retry's identity.
    let Some(permit) = state
        .direct_chat_limiter
        .try_acquire_pool(
            if guest.is_some() {
                "assistant_channel_guest"
            } else {
                "assistant_channel"
            },
            actor,
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
        media_event_ids: start.media_event_ids.clone(),
        channel_context: start.channel_context.clone(),
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
    let keys = &state.encryption_keys;
    let (field, marker) = if guest.is_some() {
        (
            event_record::CHANNEL_GUEST,
            event_record::CHANNEL_GUEST_VERSION,
        )
    } else {
        (event_record::GUEST, event_record::GUEST_VERSION)
    };
    let ciphertext =
        event_record::seal(keys, event, &channel.user_id, &[field, &channel.id], &bytes).await?;
    let deadline = Utc::now() + ChronoDuration::seconds(60);
    let update = event_record::set_material(
        doc! {
            "status": "queued",
            "guest_setup_conversation_id": conversation,
            "guest_setup_deadline": bson::DateTime::from_chrono(deadline),
            field: bson::Binary { subtype:bson::spec::BinarySubtype::Generic, bytes:ciphertext },
        },
        marker,
        event_record::version(keys),
    );
    let context = (
        state.db.clone(),
        channel.user_id.clone(),
        channel.id.clone(),
        event.to_owned(),
        update,
        field,
        guest.cloned(),
    );
    let accepted = crate::services::transaction_service::run_db_only(
        &state.db,
        &context,
        |tx, (db, owner, channel, event, update, field, guest)| {
            Box::pin(async move {
                crate::services::erasure_admission::fence_in_session(db, owner, tx).await?;
                if let Some(guest) = guest {
                    crate::services::channel_guest_access::fence_in_session(db, tx, guest).await?;
                }
                let events = db.collection::<bson::Document>(EVENTS);
                let queued = events
                    .count_documents(doc! {
                        "user_id": owner,
                        "status": "queued",
                        *field: {"$exists": true},
                    })
                    .session(&mut *tx)
                    .await?;
                if queued >= 32 {
                    return Ok(false);
                }
                let result = events
                    .update_one(
                        doc! {
                            "_id": event,
                            "user_id": owner,
                            "channel_id": channel,
                            "turn_id": bson::Bson::Null,
                            "status": {"$in": ["running", "resuming"]},
                        },
                        update.clone(),
                    )
                    .session(tx)
                    .await?;
                Ok(result.matched_count == 1)
            })
        },
    )
    .await?;
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
        doc!{"_id":id,"status":"queued","turn_id":bson::Bson::Null,"$and":[
                {"$or":[{"guest_setup_payload":{"$exists":true}},{"channel_guest_setup_payload":{"$exists":true}}]},
                {"$or":[{"guest_setup_lease":null},{"guest_setup_lease":{"$lte":now}}]}
            ]},
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
            Ok(Some(Inbound::Silent("refused_policy")))
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
    state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":id,"status":"queued","turn_id":bson::Bson::Null},
            doc! {"$set":{"status":"refused","delivery.state":"refused"},"$unset": {
                "guest_setup_payload": "",
                "guest_setup_context_version": "",
                "channel_guest_setup_payload": "",
                "channel_guest_setup_context_version": "",
            }},
        )
        .await?;
    Ok(())
}

async fn resume_inner(state: &AppState, event: &bson::Document) -> AppResult<Option<Inbound>> {
    let invalid = || AppError::Internal("Guest queue unavailable".into());
    let id = event.get_str("_id").map_err(|_| invalid())?;
    let owner = event.get_str("user_id").map_err(|_| invalid())?;
    let addon = event.contains_key(event_record::CHANNEL_GUEST);
    let bytes = event_record::guest_setup(&state.encryption_keys, event).await?;
    let payload: WaitingGuest = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let guest = payload
        .channel_context
        .as_ref()
        .and_then(|context| context.channel_guest.as_ref());
    match (addon, guest) {
        (true, Some(guest))
            if guest.source_event_id == id
                && guest.hosted_conversation_id() == payload.conversation =>
        {
            crate::services::channel_guest_access::revalidate(&state.db, guest).await?;
        }
        (false, None)
            if !crate::services::channel_guest_access::is_hosted_conversation(
                &payload.conversation,
            ) => {}
        _ => return Err(AppError::AssistantTurnRequired),
    }
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
    let actor = payload
        .origin
        .org_execution
        .as_ref()
        .map_or(owner, |e| e.actor_id.as_str())
        .to_owned();
    let org_access = if let Some(e) = &payload.origin.org_execution {
        Some(
            crate::services::org_channel_service::validate(
                &state.db,
                &payload.origin,
                &actor,
                &e.agent_id,
            )
            .await?,
        )
    } else {
        None
    };
    let source: NyxbotEvent = bson::from_document(event.clone()).map_err(|_| invalid())?;
    let live_agent = thread.agent_id.as_deref().or(channel.agent_id.as_deref());
    let agent_matches = payload.origin.thread.is_some()
        || requested_agent(state, owner, live_agent).await?.id == payload.agent;
    let custom_binding_allowed = if let Some(binding_id) = source.custom_binding_id.as_deref() {
        let binding = state
            .db
            .collection::<crate::models::nyxbot_event_binding::NyxbotEventBinding>(
                crate::models::nyxbot_event_binding::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": binding_id, "user_id": owner})
            .await?;
        if let Some(binding) = binding {
            let destination = state
                .db
                .collection::<crate::models::nyxbot_channel::NyxbotThread>(
                    crate::models::nyxbot_channel::THREADS_COLLECTION_NAME,
                )
                .find_one(doc! {
                    "channel_id": &binding.destination_channel_id,
                    "user_id": owner,
                    "partition": &binding.destination_partition,
                    "last_message_id": {"$ne": null},
                })
                .await?;
            binding.destination_channel_id == channel.id
                && payload.origin.partition.starts_with("custom:")
                && destination.is_some()
                && crate::services::nyxbot_event_binding_service::authorize_provider(
                    state, &binding,
                )
                .await
                .is_ok()
        } else {
            false
        }
    } else {
        false
    };
    let allowed = channel.status == "active"
        && source.expires_at > Utc::now()
        && source.delivery.as_ref().is_some_and(|d| {
            d.guest
                && d.transport == channel.transport
                && d.source_route_api_key_id.as_deref() == Some(&channel.route_api_key_id)
                && d.source_route_id == channel.route_id
                && (d.origin.thread.is_some()
                    || custom_binding_allowed
                    || crate::services::channel_turn_delivery::guest_source_admitted(
                        &channel, &thread, d,
                    ))
        })
        && agent_matches;
    if !allowed {
        retire(state, id).await?;
        return Ok(Some(Inbound::Silent("refused_policy")));
    }
    if payload.origin.thread.is_some() {
        crate::services::channel_thread_follow_service::validate_delivery(
            &state.db,
            &actor,
            &payload.origin,
            &payload.conversation,
        )
        .await?;
    }
    let expired = event
        .get_datetime("guest_setup_deadline")
        .is_ok_and(|at| at.to_chrono() <= Utc::now());
    if expired {
        setup::expire_wait(state, &actor, &payload.agent, &payload.profile).await?;
    }
    let record = setup::record(&state.db, &actor, &payload.agent, &payload.profile).await?;
    let failed = record
        .as_ref()
        .is_some_and(|r| r.get_str("status") == Ok("failed"));
    if !expired
        && !failed
        && !(if payload.origin.org_execution.is_some() {
            setup::kick_org_channel(state, &payload.origin, &payload.profile).await?
        } else {
            setup::kick(state, owner, &payload.agent, &payload.profile, false).await?
        })
    {
        return Ok(None);
    }
    // Ordinary begin_turn rechecks live agent, org, key, source and thread
    // authority and binds this event atomically. Competing workers cannot run it twice.
    let exists = engine::get(&state.db, &actor, &payload.conversation)
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
    start.media_event_ids = payload.media_event_ids;
    start.org_access = org_access;
    start.origin = TurnOrigin::Channel;
    start.channel = Some(payload.origin);
    start.reply_channel = payload.reply_channel;
    start.channel_event_id = Some(id.into());
    start.channel_context = payload.channel_context;
    start.note = payload.note;
    start.title = payload.title;
    start.question = payload.question;
    start.question_key = payload.question_key;
    match Box::pin(start_server_turn(
        state,
        &actor,
        start,
        if addon {
            Pool::ChannelGuest { owner: &actor }
        } else {
            Pool::Channel { owner: &actor }
        },
    ))
    .await?
    {
        Started::Turn { receiver, .. } => Ok(Some(Inbound::Turn(receiver))),
        Started::Busy | Started::PoolFull => Ok(None),
    }
}

pub(crate) async fn sweep(state: &AppState) -> AppResult<()> {
    let events:Vec<bson::Document> = state.db.collection(EVENTS)
        .find(doc!{"status":"queued","turn_id":bson::Bson::Null,"guest_setup_deadline":{"$exists":true},"$or":[{"guest_setup_payload":{"$exists":true}},{"channel_guest_setup_payload":{"$exists":true}}]})
        .projection(doc!{"_id":1}).sort(doc!{"created_at":1}).limit(32).await?.try_collect().await?;
    for event in events {
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        if let Ok(id) = event.get_str("_id") {
            crate::services::channel_turn_delivery::detach(&state.db, id).await?;
            let _ = Box::pin(resume(state, id)).await;
        }
    }
    Ok(())
}
