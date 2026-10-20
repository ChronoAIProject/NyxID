//! Durable arbitration between the provider stream and asynchronous channel replies.
//! Never reclaim a dispatch barrier: an interrupted external send is ambiguous.
use crate::models::nyxbot_channel::{NyxbotChannel, NyxbotThread};
use bson::doc;
use chrono::Utc;
use mongodb::{ClientSession, Database};

use crate::{
    errors::{AppError, AppResult},
    models::nyxbot_channel::EVENTS_COLLECTION_NAME as EVENTS,
};

pub async fn bind_in_session(
    db: &Database,
    session: &mut ClientSession,
    event: &str,
    owner: &str,
    conversation: &str,
    turn: &str,
) -> AppResult<()> {
    let bound = db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "user_id": owner, "status": {"$in": ["running", "resuming", "queued"]},
            "turn_id": bson::Bson::Null, "delivery.version": 1},
            doc! {"$set": {"conversation_id": conversation, "turn_id": turn}},
        )
        .session(session)
        .await?;
    if bound.matched_count != 1 {
        return Err(AppError::Conflict("Channel event already admitted".into()));
    }
    Ok(())
}

pub async fn detach(db: &Database, event: &str) -> AppResult<()> {
    db.collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.version": 1, "delivery.state": "waiting"},
            doc! {"$set": {"delivery.state": "pending"}},
        )
        .await?;
    Ok(())
}

/// The write precedes the first committed message frame. Once claimed, a lost
/// stream cannot safely be retried without a gateway delivery acknowledgement.
pub async fn claim_stream(db: &Database, event: &str, status: &str) -> AppResult<bool> {
    Ok(db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.version": 1, "delivery.state": "waiting",
            "delivery.stream_deadline": {"$gt": bson::DateTime::from_chrono(Utc::now())}},
            doc! {"$set": {"delivery.state": "streamed", "status": status}},
        )
        .await?
        .modified_count
        == 1)
}

/// One irreversible outbound attempt, shared by the callback and all sweepers.
pub async fn claim_send(db: &Database, event: &str, claim: &str) -> AppResult<bool> {
    Ok(db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.version": 1, "delivery.state": "pending"},
            doc! {"$set": {"delivery.state": "sending", "delivery.claim_id": claim}},
        )
        .await?
        .modified_count
        == 1)
}

pub async fn finish(db: &Database, event: &str, claim: &str, status: &str) -> AppResult<()> {
    db.collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.state": "sending", "delivery.claim_id": claim},
            doc! {"$set": {"delivery.state": status}},
        )
        .await?;
    Ok(())
}

/// No external effect has started. Silence is terminal and cannot reclaim an
/// ambiguous send or a stream whose frames may already have reached a provider.
pub async fn silence(db: &Database, event: &str) -> AppResult<bool> {
    Ok(db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.version": 1,
            "delivery.state": {"$in": ["waiting", "pending"]}},
            doc! {"$set": {"delivery.state": "silent", "status": "completed"},
            "$unset": {"event_context_ciphertext": "", "delivery.target_ciphertext": ""}},
        )
        .await?
        .modified_count
        == 1)
}

/// Admit one source-bound owner question, or join an identical question. Queue
/// capacity, destination cap, event acceptance and turn binding commit together.
/// The generic relay never participates in this assistant-owned queue.
pub async fn enqueue(
    db: &Database,
    owner: &str,
    conversation: &str,
    event: crate::models::assistant_conversation::AgentEvent,
) -> AppResult<bool> {
    use super::api_key_mutation_service as transactions;
    let db = db.clone();
    let owner = owner.to_owned();
    let conversation = conversation.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            transactions::transaction_result(
                Box::pin(enqueue_in_session(
                    &db,
                    &owner,
                    &conversation,
                    &event,
                    session,
                ))
                .await,
            )
        })
        .await
        .map_err(transactions::map_transaction_error)
}

/// Each driver retry receives immutable incoming work. Attempt-local queued
/// flags may bypass expiry only after acceptance has actually committed.
pub(crate) async fn enqueue_in_session(
    db: &Database,
    owner: &str,
    conversation: &str,
    incoming: &crate::models::assistant_conversation::AgentEvent,
    session: &mut ClientSession,
) -> AppResult<bool> {
    use super::assistant_nyxagent as engine;
    use crate::models::assistant_conversation::{
        AssistantConversation, COLLECTION_NAME as CONVERSATIONS,
    };
    let source = incoming
        .source_event_ids
        .first()
        .cloned()
        .ok_or_else(|| AppError::Internal("Channel source unavailable".into()))?;
    // Transaction retries must revalidate the original unaccepted
    // origin. Never leak queued=true from an aborted attempt.
    let mut event = incoming.clone();
    let rows = db.collection::<AssistantConversation>(CONVERSATIONS);
    let mut row = rows
        .find_one(doc! {"_id": &conversation, "user_id": &owner})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::NotFound("Conversation not found".into()))?;
    for origin in &mut event.reply_to {
        if let Some(binding) = origin.thread.as_mut() {
            binding.queued = true;
        }
    }
    let key = event.question_key.as_deref();
    let matching = |other: Option<&str>| key.is_some() && key == other;
    let active = engine::live_turn(&row, Utc::now())
        .filter(|turn| {
            !turn.stop_requested
                && turn.invocation_id.is_none()
                && !row.guest_turn
                && row.group_id.is_none()
                && turn.platform_origin.is_none()
                && turn.channel_adapter_origin.is_none()
                && turn.trigger_run_id.is_none()
                && matching(turn.question_key.as_deref())
                && (if turn.origin == crate::models::assistant_conversation::TurnOrigin::User {
                    // A personal channel repeat may join the owner's app/home
                    // answer without changing its primary destination or turn.
                    row.channel.is_none()
                        && row.platform_thread.is_none()
                        && turn.asked_from.is_none()
                        && event
                            .reply_to
                            .first()
                            .is_some_and(|origin| origin.thread.is_none())
                } else {
                    matches!(
                        turn.origin,
                        crate::models::assistant_conversation::TurnOrigin::Channel
                            | crate::models::assistant_conversation::TurnOrigin::Event
                    ) && turn.channel_event_id.is_some()
                        && compatible_sources(
                            turn.asked_from.as_ref().or_else(|| {
                                turn.events
                                    .iter()
                                    .find(|e| !e.source_event_ids.is_empty())
                                    .and_then(|e| e.reply_to.first())
                            }),
                            event.reply_to.first(),
                        )
                })
        })
        .map(|turn| turn.turn_id.clone());
    let mut bound_turn = None;
    if let Some(turn) = active {
        // Count durable destinations rather than ChannelOrigin equality:
        // two messages in one chat still have different source targets.
        let count = db
            .collection::<bson::Document>(EVENTS)
            .count_documents(doc! {
            "user_id": &owner, "conversation_id": &conversation, "turn_id": &turn,
            "delivery.version": 1,
                            })
            .session(&mut *session)
            .await?;
        if count >= (engine::MAX_ALSO_DELIVER + 1) as u64 {
            return Ok(false);
        }
        // Fence settlement and concurrent destination admissions on the
        // same conversation document, including repeated same-chat work.
        rows.update_one(
            doc! {"_id": &conversation, "user_id": &owner,
            "active_turn.turn_id": &turn},
            doc! {"$inc": {"channel_source_revision": 1}},
        )
        .session(&mut *session)
        .await?;
        bound_turn = Some(turn);
    } else if let Some(pending) = row.pending_events.iter_mut().find(|e| {
        e.invocation_id.is_none()
            && !e.source_event_ids.is_empty()
            && matching(e.question_key.as_deref())
            && compatible_sources(e.reply_to.first(), event.reply_to.first())
    }) {
        if pending.source_event_ids.len() > engine::MAX_ALSO_DELIVER {
            return Ok(false);
        }
        pending.source_event_ids.push(source.clone());
    } else {
        if row.pending_events.len() >= engine::MAX_PENDING_EVENTS {
            return Ok(false);
        }
        row.pending_events.push(event.clone());
    }
    // Capacity refusal must not activate or extend a native follow. Validate
    // and activate only after capacity succeeds, using the immutable incoming
    // origin so an uncommitted queued flag never bypasses admission gates.
    for origin in &incoming.reply_to {
        super::channel_thread_follow_service::admit_turn(db, owner, origin, conversation, session)
            .await?;
    }
    let accepted = db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {
                "_id": &source, "user_id": &owner, "status": {"$in": ["running", "resuming"]},
                "turn_id": bson::Bson::Null, "delivery.version": 1,
            },
            doc! {"$set": {"status": "queued", "conversation_id": &conversation,
            "turn_id": bson::Bson::Null, "delivery.state": "pending"}},
        )
        .session(&mut *session)
        .await?;
    if accepted.matched_count != 1 {
        return Err(AppError::Conflict("Channel event already admitted".into()));
    }
    // Validate the source while it is queued/unbound in this
    // transaction, before binding a coalesced active destination.
    if queued_origin_in_session(db, owner, conversation, &source, session)
        .await?
        .is_none()
    {
        return Err(AppError::Forbidden(
            "Channel source authority changed".into(),
        ));
    }
    if let Some(turn) = &bound_turn {
        bind_in_session(db, session, &source, owner, conversation, turn).await?;
    } else {
        rows.update_one(
            doc! {"_id": &conversation, "user_id": &owner},
            doc! {"$set": {"pending_events": bson::to_bson(&row.pending_events)
            .map_err(|_| AppError::Internal("Channel queue unavailable".into()))?}},
        )
        .session(&mut *session)
        .await?;
    }
    Ok(true)
}

/// Home owner chats can share work across destinations. Native children retain
/// their exact agent/generation/child authority, never a text-only match.
fn compatible_sources(
    a: Option<&crate::models::assistant_conversation::ChannelOrigin>,
    b: Option<&crate::models::assistant_conversation::ChannelOrigin>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => match (&a.thread, &b.thread) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                !a.guest
                    && !b.guest
                    && a.child_id == b.child_id
                    && a.agent_id == b.agent_id
                    && a.generation == b.generation
                    && a.channel_generation == b.channel_generation
            }
            _ => false,
        },
        _ => false,
    }
}

/// Transactional admission revalidation for each queued destination, with
/// revision writes fencing live authority. Callers terminalize ineligible
/// sources independently, preserving other destinations of a repeat.
pub async fn queued_origin_in_session(
    db: &Database,
    owner: &str,
    conversation: &str,
    source: &str,
    session: &mut ClientSession,
) -> AppResult<Option<crate::models::assistant_conversation::ChannelOrigin>> {
    use crate::models::{channel_bot::ChannelBot, nyxbot_channel::NyxbotEvent};
    let Some(event) = db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {
            "_id": source, "user_id": owner, "conversation_id": conversation,
            "status": "queued", "turn_id": bson::Bson::Null, "delivery.version": 1,
            "delivery.guest": false, "delivery.state": "pending",
            "expires_at": {"$gt": bson::DateTime::from_chrono(Utc::now())},
        })
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    let Some(d) = event.delivery else {
        return Ok(None);
    };
    if d.origin.thread.is_some() {
        return match super::channel_thread_follow_service::validate_in_session(
            db,
            owner,
            &d.origin,
            conversation,
            true,
            session,
        )
        .await
        {
            Ok(_) => {
                // The source query proved durable queued acceptance, and live
                // native authority has now been revalidated. Project that
                // acceptance without changing the captured admission origin.
                let mut origin = d.origin;
                if let Some(binding) = origin.thread.as_mut() {
                    binding.queued = true;
                }
                Ok(Some(origin))
            }
            Err(AppError::NotFound(_) | AppError::Forbidden(_)) => Ok(None),
            Err(error) => Err(error),
        };
    }
    let Some(row) = db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": conversation, "user_id": owner})
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    let Some((channel, thread)) = target_in_session(db, &row, &d.origin, session, true).await?
    else {
        return Ok(None);
    };
    if channel.transport != d.transport
        || !matches!(
            admission(&channel, &thread, &d.sender_id, Some(d.addressed)),
            Admission::Owner
        )
    {
        return Ok(None);
    }
    use crate::models::assistant_agent::{AssistantAgent, COLLECTION_NAME as AGENTS};
    let mut agent_filter = doc! {"user_id": owner, "destroyed_at": bson::Bson::Null};
    if let Some(id) = &row.agent_id {
        agent_filter.insert("_id", id);
    } else {
        agent_filter.insert("kind", "nyxbot");
    }
    let Some(resolved) = db
        .collection::<AssistantAgent>(AGENTS)
        .find_one_and_update(
            agent_filter,
            doc! {"$inc": {"channel_admission_revision": 1_i64}},
        )
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    if thread
        .agent_id
        .as_ref()
        .or(channel.agent_id.as_ref())
        .is_some_and(|id| id != &resolved.id)
    {
        return Ok(None);
    }
    // Serialize against bot platform adoption, deletion, and route revocation.
    match super::platform_bot_authority::fence_personal_routes(
        db,
        &channel.channel_bot_id,
        channel.bot_owner_id.as_deref().unwrap_or(owner),
        session,
    )
    .await
    {
        Ok(()) => {}
        Err(AppError::Forbidden(_) | AppError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error),
    }

    let Some(bot) = db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id": &channel.channel_bot_id,
            "user_id": channel.bot_owner_id.as_deref().unwrap_or(owner),
            "is_active": true, "status": {"$ne": "suspended"}},
            doc! {"$inc": {"channel_admission_revision": 1_i64}},
        )
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    if super::channel_platform::canonical_platform(&bot.platform) != channel.platform {
        return Ok(None);
    }
    // The bot fence serializes route management, but is not evidence that
    // the original managed route still exists and belongs to this channel.
    // Fence that exact route too: deletion/rebinding must win before tools run.
    let (Some(route_id), Some(route_key)) = (
        d.source_route_id.as_deref(),
        d.source_route_api_key_id.as_deref(),
    ) else {
        return Ok(None);
    };
    if channel.route_id.as_deref() != Some(route_id) || channel.route_api_key_id != route_key {
        return Ok(None);
    }
    let route = db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .find_one_and_update(
            doc! {
                "_id": route_id,
                "user_id": channel.bot_owner_id.as_deref().unwrap_or(owner),
                "channel_bot_id": &channel.channel_bot_id, "platform": &bot.platform,
                "agent_api_key_id": &channel.route_api_key_id,
                "is_active": true, "retired_by_transfer": {"$ne": true},
            },
            doc! {"$inc": {"channel_admission_revision": 1_i64}},
        )
        .session(&mut *session)
        .await?;
    if route.is_none() {
        return Ok(None);
    }

    let key = db.collection::<bson::Document>("api_keys").find_one_and_update(doc! {
        "_id": &channel.route_api_key_id, "user_id": channel.bot_owner_id.as_deref().unwrap_or(owner), "is_active": true,
        "$or": [{"expires_at": bson::Bson::Null}, {"expires_at": {"$gt": bson::DateTime::from_chrono(Utc::now())}}],
    }, doc! {"$inc": {"channel_admission_revision": 1_i64}}).session(&mut *session).await?;
    let user = db
        .collection::<bson::Document>("users")
        .find_one_and_update(
            doc! {
                "_id": owner, "is_active": true,
            },
            doc! {"$inc": {"channel_admission_revision": 1_i64}},
        )
        .session(&mut *session)
        .await?;
    Ok((key.is_some() && user.is_some()).then_some(d.origin))
}

/// Who an inbound chat message is from, as far as the agent is concerned.
pub(crate) enum Admission {
    /// The verified owner.
    Owner,
    /// Someone else the chat lets talk to the agent.
    Guest,
    /// A private chat with someone the bot does not answer.
    Refuse,
    /// Not for the agent (not addressed, or members may not talk).
    Silent,
    /// A member addressed the agent in a group the owner has not talked in
    /// yet (and has not set who may talk).
    Waiting,
}

pub(crate) fn admission(
    row: &NyxbotChannel,
    chat: &NyxbotThread,
    sender_id: &str,
    addressed: Option<bool>,
) -> Admission {
    let owner = row.owner_sender_ids.iter().any(|id| id == sender_id);
    // When the platform cannot tell, the owner's messages count as addressed
    // and nobody else's do.
    let addressed = addressed.unwrap_or(owner);
    if chat.kind.as_deref() == Some("private") {
        return if owner {
            Admission::Owner
        } else if row.private_chats.as_deref() == Some("everyone") {
            Admission::Guest
        } else {
            Admission::Refuse
        };
    }
    if !addressed
        && !(chat.kind.as_deref() == Some("private") || chat.reply_mode.as_deref() == Some("all"))
    {
        Admission::Silent
    } else if owner {
        Admission::Owner
    } else if match chat.members.as_deref() {
        Some("everyone") => true,
        Some(_) => false,
        None => chat.owner_seen,
    } {
        Admission::Guest
    } else if chat.members.is_none() && addressed && chat.kind.as_deref() == Some("group") {
        // A broadcast channel's posts come from the channel, never the owner:
        // no promise that cannot come true.
        Admission::Waiting
    } else {
        Admission::Silent
    }
}

/// Shared live target authority for late delivery and queued execution.
/// Captured native/platform/service-adapter turns retain their separate paths.
pub async fn target(
    db: &Database,
    row: &crate::models::assistant_conversation::AssistantConversation,
    origin: &crate::models::assistant_conversation::ChannelOrigin,
) -> AppResult<Option<(NyxbotChannel, NyxbotThread)>> {
    let mut session = db.client().start_session().await?;
    target_in_session(db, row, origin, &mut session, false).await
}

async fn target_in_session(
    db: &Database,
    row: &crate::models::assistant_conversation::AssistantConversation,
    origin: &crate::models::assistant_conversation::ChannelOrigin,
    session: &mut ClientSession,
    fence: bool,
) -> AppResult<Option<(NyxbotChannel, NyxbotThread)>> {
    use crate::models::nyxbot_channel::{
        COLLECTION_NAME as CHANNELS, THREADS_COLLECTION_NAME as THREADS,
    };
    if fence {
        db.collection::<bson::Document>(CHANNELS)
            .update_one(
                doc! {"_id": &origin.nyxbot_channel_id, "user_id": &row.user_id},
                doc! {"$inc": {"channel_admission_revision": 1_i64}},
            )
            .session(&mut *session)
            .await?;
        db.collection::<bson::Document>(THREADS).update_one(
            doc! {"channel_id": &origin.nyxbot_channel_id, "partition": &origin.partition, "user_id": &row.user_id},
            doc! {"$inc": {"channel_admission_revision": 1_i64}},
        ).session(&mut *session).await?;
    }
    let Some(channel) = db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {
            "_id": &origin.nyxbot_channel_id, "user_id": &row.user_id, "status": "active",
        })
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    if !super::platform_bot_authority::permits_personal_routes(db, &channel.channel_bot_id).await? {
        return Ok(None);
    }
    if let Some(org) = &channel.bot_owner_id {
        let filter = doc! {"org_user_id": org, "member_user_id": &row.user_id,
        "revoked_at": bson::Bson::Null, "role": "admin"};
        let membership =
            db.collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME);
        let allowed = if fence {
            membership
                .find_one_and_update(filter, doc! {"$inc": {"channel_admission_revision": 1_i64}})
                .session(&mut *session)
                .await?
                .is_some()
        } else {
            membership
                .find_one(filter)
                .session(&mut *session)
                .await?
                .is_some()
        };
        if !allowed {
            return Ok(None);
        }
    }
    let Some(thread) = db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {
            "channel_id": &channel.id, "user_id": &row.user_id, "partition": &origin.partition,
            "conversation_id": &row.id,
        })
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    let own_chat = row.channel.as_ref() == Some(origin);
    let private_home = thread.kind.as_deref() == Some("private")
        && thread.owner_chat
        && row.group_id.is_none()
        && row.platform_thread.is_none();
    Ok((own_chat || private_home).then_some((channel, thread)))
}
