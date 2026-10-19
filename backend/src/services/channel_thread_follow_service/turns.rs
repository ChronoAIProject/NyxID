use super::*;
use crate::models::{channel_bot::ChannelBot, channel_message::ChannelMessage};

/// Returns live policy after taking the same fence as stop/settings/allocation.
/// Queued work ignores subscription expiry only, never sender or agent changes.
pub async fn validate_in_session(
    db: &Database,
    owner: &str,
    origin: &ChannelOrigin,
    conversation: &str,
    admitted: bool,
    session: &mut ClientSession,
) -> AppResult<NyxbotThread> {
    let b = origin.thread.as_deref().ok_or_else(not_found)?;
    let channel = fence(db, &origin.nyxbot_channel_id, owner, session).await?;
    if channel.status != "active"
        || !super::super::channel_thread_service::gateway::supports(&channel)
    {
        return Err(not_found());
    }
    if let Some(org) = &channel.bot_owner_id
        && !super::super::org_service::is_admin(db, owner, org).await?
    {
        return Err(not_found());
    }
    let child = db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"_id":&b.child_id,
        "user_id":owner,"channel_id":&channel.id,"partition":&origin.partition,"record_scope":SCOPE,
        "conversation_id":conversation})
        .session(&mut *session)
        .await?
        .ok_or_else(not_found)?;
    if !supported_child(&child) {
        return Err(not_found());
    }
    let settings = db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"_id":&child.follow.settings_chat_id,
        "user_id":owner,"channel_id":&channel.id,"record_scope":{"$ne":SCOPE}})
        .session(&mut *session)
        .await?
        .ok_or_else(not_found)?;
    if channel_generation(&channel, &settings) != b.channel_generation {
        return Err(not_found());
    }
    let bot = db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {
            "_id":&channel.channel_bot_id,
            "user_id":channel.bot_owner_id.as_deref().unwrap_or(owner),
            "is_active":true,"status":"active"
        })
        .session(&mut *session)
        .await?
        .ok_or_else(not_found)?;
    let source = db
        .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(doc! {
            "_id":&b.source_message_id,"direction":"inbound",
            "channel_bot_id":&bot.id,"user_id":&bot.user_id,
            "platform":&bot.platform,"sender_platform_id":&b.sender_id,
            "agent_api_key_id":&channel.route_api_key_id
        })
        .session(&mut *session)
        .await?
        .ok_or_else(not_found)?;
    let sender_eligibility = match source.thread_context.as_ref() {
        Some(facts) => eligible_for_facts(&channel, &settings, facts, &b.sender_id),
        // Older inbound rows have no facts. Preserve their pre-follow
        // eligibility semantics instead of making the additive facts field a
        // prerequisite for an existing binding.
        None => eligible(&channel, &settings, &b.sender_id),
    };
    if child.follow.binding_generation != b.generation
        || settings.follow.binding_generation != b.generation
        || child.follow.bound_agent_id.as_deref() != Some(&b.agent_id)
        || settings
            .agent_id
            .as_deref()
            .or(channel.agent_id.as_deref())
            .is_some_and(|id| id != b.agent_id)
        || sender_eligibility != Some(b.guest)
    {
        return Err(not_found());
    }
    let on = super::super::feature_flag_service::personal_flag_enabled(
        db,
        owner,
        super::super::feature_flag_service::NYXBOT_THREAD_FOLLOW_FLAG_KEY,
    )
    .await?;
    if !admitted && !b.queued {
        if settings
            .follow
            .threads
            .as_deref()
            .is_some_and(|s| s != "follow")
            || child.follow.follow_revision != b.revision
        {
            return Err(not_found());
        }
        let available = match child.follow.follow_state.as_deref() {
            Some("opening") => child
                .follow
                .follow_opening_expires_at
                .is_some_and(|t| t > Utc::now()),
            Some("active") => follows(&child),
            Some("stopped" | "expired") => settings.reply_mode.as_deref() == Some("all"),
            _ => false,
        };
        if on && !available {
            return Err(not_found());
        }
    }
    if !admitted
        && !b.queued
        && !on
        && settings.reply_mode.as_deref() != Some("all")
        && !source.thread_context.as_ref().is_some_and(|f| {
            matches!(
                f.address,
                crate::models::channel_thread::ThreadAddress::Mention
                    | crate::models::channel_thread::ThreadAddress::ReplyToBot
                    | crate::models::channel_thread::ThreadAddress::MailboxTo
                    | crate::models::channel_thread::ThreadAddress::VerifiedReply
            )
        })
    {
        return Err(not_found());
    }
    if !source.thread_context.as_ref().is_some_and(|f| {
        f.version == 1
            && matches!(
                f.sender_kind,
                ThreadSenderKind::Human | ThreadSenderKind::Bot
            )
            && b.sender_kind.is_none_or(|kind| kind == f.sender_kind)
            && (f.sender_kind != ThreadSenderKind::Bot
                || (b.guest && f.address == crate::models::channel_thread::ThreadAddress::Mention))
            && Some(f.kind) == child.follow.thread_kind
            && f.root_id == child.follow.thread_root_id
            && Some(&f.chat_id) == child.platform_chat_id.as_ref()
    }) {
        return Err(not_found());
    }
    if db
        .collection::<bson::Document>("api_keys")
        .find_one(doc! {"_id":&channel.route_api_key_id,
        "user_id":&bot.user_id,"is_active":true})
        .session(&mut *session)
        .await?
        .is_none()
    {
        return Err(not_found());
    }
    let agent = db
        .collection::<crate::models::assistant_agent::AssistantAgent>(
            crate::models::assistant_agent::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&b.agent_id,"user_id":owner,"destroyed_at":bson::Bson::Null})
        .session(&mut *session)
        .await?;
    if agent.is_none() {
        return Err(not_found());
    }
    let route = super::super::channel_routing_service::resolve_agent(
        db,
        &bot.id,
        source
            .platform_conversation_id
            .as_deref()
            .ok_or_else(not_found)?,
        Some(&b.sender_id),
        &bot.user_id,
    )
    .await?;
    if !route.is_some_and(|r| {
        r.api_key_id == channel.route_api_key_id && r.conversation.id == source.conversation_id
    }) {
        return Err(not_found());
    }
    Ok(child)
}

pub async fn admit_turn(
    db: &Database,
    owner: &str,
    origin: &ChannelOrigin,
    conversation: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    if origin.thread.is_none() {
        return Ok(());
    }
    let child = validate_in_session(db, owner, origin, conversation, false, session).await?;
    activate(db, &child, origin.thread.as_deref().unwrap(), session).await
}

async fn activate(
    db: &Database,
    child: &NyxbotThread,
    b: &ThreadTurnBinding,
    session: &mut ClientSession,
) -> AppResult<()> {
    let now = Utc::now();
    let mut set = doc! {"last_message_id":&b.source_message_id,"last_message_at":bson::DateTime::from_chrono(now),
    "follow_last_admitted_at":bson::DateTime::from_chrono(now),"updated_at":bson::DateTime::from_chrono(now)};
    if matches!(
        child.follow.follow_state.as_deref(),
        Some("opening" | "active")
    ) && b.sender_kind != Some(ThreadSenderKind::Bot)
        && !b.queued
        && super::super::feature_flag_service::personal_flag_enabled(
            db,
            &child.user_id,
            super::super::feature_flag_service::NYXBOT_THREAD_FOLLOW_FLAG_KEY,
        )
        .await?
    {
        set.insert("follow_state", "active");
        set.insert(
            "follow_expires_at",
            bson::DateTime::from_chrono(now + Duration::hours(IDLE_HOURS)),
        );
        if child.follow.follow_state.as_deref() == Some("opening") {
            set.insert("follow_started_at", bson::DateTime::from_chrono(now));
        }
    }
    db.collection::<NyxbotThread>(THREADS)
        .update_one(doc! {"_id":&child.id}, doc! {"$set":set})
        .session(session)
        .await?;
    Ok(())
}

/// Drop revoked queued authority before constructing any owner event input.
pub async fn filter_events(
    db: &Database,
    owner: &str,
    conversation: &str,
    events: Vec<AgentEvent>,
    session: &mut ClientSession,
) -> AppResult<Vec<AgentEvent>> {
    let mut allowed = Vec::new();
    for mut event in events {
        if !event.source_event_ids.is_empty() {
            let mut sources = Vec::new();
            let mut origins = Vec::new();
            for source in &event.source_event_ids {
                if let Some(origin) = super::super::channel_turn_delivery::queued_origin_in_session(
                    db,
                    owner,
                    conversation,
                    source,
                    session,
                )
                .await?
                {
                    sources.push(source.clone());
                    origins.push(origin);
                } else {
                    db.collection::<bson::Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
                        .update_one(doc! {"_id": source, "user_id": owner,
                            "delivery.state": {"$in": ["waiting", "pending"]}},
                            doc! {"$set": {"status": "refused", "delivery.state": "refused"},
                                "$unset": {"event_context_ciphertext": "", "delivery.target_ciphertext": ""}})
                        .session(&mut *session).await?;
                }
            }
            event.source_event_ids = sources;
            event.reply_to = origins;
            if !event.source_event_ids.is_empty() {
                allowed.push(event);
            }
            continue;
        }
        // Legacy generic message queues have no recoverable original target.
        // Do not turn them into a reply to whichever message arrived last.
        if event.kind == "message" && event.reply_to.iter().any(|o| o.thread.is_none()) {
            continue;
        }
        let mut valid = true;
        for origin in &event.reply_to {
            if origin.thread.is_some() {
                match validate_in_session(db, owner, origin, conversation, true, session).await {
                    Ok(_) => {}
                    Err(AppError::NotFound(_)) => {
                        valid = false;
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        if valid {
            allowed.push(event);
        }
    }
    Ok(allowed)
}

pub async fn prune_queue(db: &Database, owner: &str, conversation: &str) -> AppResult<usize> {
    let db = db.clone();
    let owner = owner.to_owned();
    let conversation = conversation.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
            let db=&db;
            let owner=owner.as_str();
            let conversation=conversation.as_str();

        let work:AppResult<usize>=Box::pin(async{
            let rows=db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME);
            let Some(row)=rows.find_one(doc! {"_id":conversation,"user_id":owner}).session(&mut *session).await? else{return Ok(0);};
            let events=filter_events(db,owner,conversation,row.pending_events,session).await?;
            {
                rows.update_one(doc! {"_id":conversation,"user_id":owner},doc! {"$set":{"pending_events":bson::to_bson(&events).map_err(|_|not_found())?}})
                    .session(&mut *session).await?;
            }
            Ok(events.len())
        }).await;
        transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

/// A short transaction ends before any provider I/O. Every split send repeats
/// this authority check; the reply anchor always remains the admitted source.
pub async fn validate_delivery(
    db: &Database,
    owner: &str,
    origin: &ChannelOrigin,
    conversation: &str,
) -> AppResult<NyxbotThread> {
    let db = db.clone();
    let owner = owner.to_owned();
    let conversation = conversation.to_owned();
    let origin = origin.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let db = &db;
            let owner = owner.as_str();
            let conversation = conversation.as_str();
            let origin = &origin;

            transactions::transaction_result(
                Box::pin(validate_in_session(
                    db,
                    owner,
                    origin,
                    conversation,
                    true,
                    session,
                ))
                .await,
            )
        })
        .await
        .map_err(transactions::map_transaction_error)
}

/// Coalesce only inside this child. Its native thread already receives the
/// original answer, so repeats never grow queued/active reply-target arrays.
/// Renewal and duplicate recognition share the stop/admission fence.
pub async fn coalesce(
    db: &Database,
    owner: &str,
    conversation: &str,
    key: &str,
    origin: &ChannelOrigin,
) -> AppResult<Option<bool>> {
    if origin.thread.as_ref().is_none_or(|b| b.guest) {
        return Err(not_found());
    }
    let db = db.clone();
    let owner = owner.to_owned();
    let conversation = conversation.to_owned();
    let key = key.to_owned();
    let origin = origin.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let work: AppResult<Option<bool>> = Box::pin(async {
                let child =
                    validate_in_session(&db, &owner, &origin, &conversation, false, session)
                        .await?;
                let row = db
                    .collection::<AssistantConversation>(
                        crate::models::assistant_conversation::COLLECTION_NAME,
                    )
                    .find_one(doc! {"_id": &conversation, "user_id": &owner})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(not_found)?;
                let running = super::super::assistant_nyxagent::live_turn(&row, Utc::now())
                    .filter(|turn| turn.question_key.as_deref() == Some(key.as_str()));
                let queued = row
                    .pending_events
                    .iter()
                    .find(|event| event.question_key.as_deref() == Some(key.as_str()));
                let previous = if let Some(turn) = running {
                    turn.asked_from.as_ref()
                } else if let Some(event) = queued {
                    event.reply_to.first()
                } else {
                    return Ok(None);
                };
                let Some(previous) = previous.filter(|o| o.thread.is_some()) else {
                    return Ok(None);
                };
                match validate_in_session(&db, &owner, previous, &conversation, true, session).await
                {
                    Ok(_) => {}
                    Err(AppError::NotFound(_)) => return Ok(None),
                    Err(error) => return Err(error),
                }
                activate(&db, &child, origin.thread.as_deref().unwrap(), session).await?;
                Ok(Some(running.is_none()))
            })
            .await;
            transactions::transaction_result(work)
        })
        .await
        .map_err(transactions::map_transaction_error)
}
