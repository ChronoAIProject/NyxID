use super::*;
use crate::models::assistant_message::COLLECTION_NAME as MESSAGES;
use crate::services::channel_supplemental_delivery::{self as sources, SavedDestination};

#[derive(serde::Deserialize)]
struct SavedReply {
    #[serde(rename = "_id")]
    id: String,
    user_id: String,
    conversation_id: String,
    turn_id: String,
    text: String,
    status: String,
    created_at: bson::DateTime,
    handoff_delivery: Routing,
}

#[derive(serde::Deserialize)]
struct Routing {
    agent_id: String,
    targets: Vec<SavedDestination>,
}

fn authority(
    state: &AppState,
    row: &crate::models::assistant_conversation::AssistantConversation,
    origin: &ChannelOrigin,
) -> crate::services::channel_platform::PostEffectGuard {
    let db = state.db.clone();
    let owner = row.user_id.clone();
    let conversation = row.id.clone();
    let agent = row.agent_id.clone();
    let origin = origin.clone();
    std::sync::Arc::new(move || {
        let (db, owner, conversation, agent, origin) = (
            db.clone(),
            owner.clone(),
            conversation.clone(),
            agent.clone(),
            origin.clone(),
        );
        Box::pin(async move {
            let row = engine::get(&db, &owner, &conversation).await?;
            let denied = || AppError::Forbidden("Saved chat destination unavailable".into());
            if row.agent_id != agent
                || row.guest_turn
                || row.shared_agent_thread
                || row.collaboration.is_some()
            {
                return Err(denied());
            }
            let (channel, thread) =
                crate::services::channel_turn_delivery::target(&db, &row, &origin)
                    .await?
                    .ok_or_else(denied)?;
            if !thread.owner_chat
                || thread.kind.as_deref() != Some("private")
                || channel.org_specialist.is_some()
                || thread
                    .agent_id
                    .as_ref()
                    .or(channel.agent_id.as_ref())
                    .is_some_and(|id| Some(id) != agent.as_ref())
            {
                return Err(denied());
            }
            for (collection, filter) in [
                (
                    crate::models::user::COLLECTION_NAME,
                    doc! {"_id":&owner,"is_active":true},
                ),
                (
                    crate::models::assistant_agent::COLLECTION_NAME,
                    doc! {"_id":&agent,"user_id":&owner,"destroyed_at":bson::Bson::Null},
                ),
            ] {
                if db
                    .collection::<bson::Document>(collection)
                    .find_one(filter)
                    .projection(doc! {"_id":1})
                    .await?
                    .is_none()
                {
                    return Err(denied());
                }
            }
            Ok(())
        })
    })
}

pub(super) async fn sweep(state: &AppState) -> AppResult<()> {
    let ids = state
        .db
        .collection::<bson::Document>(MESSAGES)
        .find(doc! {"handoff_delivery_due_at":{"$type":"date","$lte":bson::DateTime::now()}})
        .sort(doc! {"handoff_delivery_due_at":1,"_id":1})
        .projection(doc! {"_id":1})
        .limit(32)
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    for record in ids {
        let Some(_work) = state.shutdown.background() else {
            break;
        };
        if let Ok(id) = record.get_str("_id") {
            deliver(state, id).await?;
        }
    }
    Ok(())
}

pub(crate) async fn after_turn(
    state: &AppState,
    owner: &str,
    conversation: &str,
    turn: &str,
    reply_id: &str,
) -> AppResult<()> {
    if !deliver(state, reply_id).await? {
        for target in
            sources::finished_handoff_targets(&state.db, owner, conversation, turn, reply_id)
                .await?
        {
            if target.source.turn_id != turn {
                continue;
            }
            let mut decision = supplemental::Decision::new(target.purpose());
            decision.event = target.source.event_id;
            decision.skip(supplemental::Reason::AlreadyClaimed);
            decision
                .record_ids(state, owner, conversation, Some(turn))
                .await;
        }
    }
    Ok(())
}

pub(super) async fn deliver(state: &AppState, reply_id: &str) -> AppResult<bool> {
    let lease = Uuid::new_v4().to_string();
    let Some(reply) = state.db.collection::<SavedReply>(MESSAGES).find_one_and_update(
        doc! {"_id":reply_id,"role":"assistant","handoff_delivery_due_at":{"$type":"date","$lte":bson::DateTime::now()}},
        doc! {"$set":{"handoff_delivery_lease":&lease,"handoff_delivery_due_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::minutes(2))}}
    ).await? else { return Ok(false) };
    let messages = state.db.collection::<bson::Document>(MESSAGES);
    let mut retry = false;
    let current = engine::get(&state.db, &reply.user_id, &reply.conversation_id).await;
    let row = match current {
        Ok(row) => Some(row),
        Err(AppError::Forbidden(_) | AppError::NotFound(_)) => None,
        Err(error) => return Err(error),
    };
    if let Some(mut row) = row
        && reply.created_at.to_chrono() + ChronoDuration::hours(24) > Utc::now()
    {
        row.active_turn = None;
        if row.agent_id.as_deref() == Some(reply.handoff_delivery.agent_id.as_str())
            && !row.guest_turn
            && row.collaboration.is_none()
            && !row.shared_agent_thread
        {
            let failed = reply.status != "completed";
            let images = if failed {
                None
            } else {
                Some(
                    images::ImageSendPlan::settled(
                        &state.db,
                        &reply.user_id,
                        &reply.conversation_id,
                        &reply.turn_id,
                    )
                    .await?,
                )
            };
            for target in reply.handoff_delivery.targets {
                if target.source.turn_id != reply.turn_id {
                    continue;
                }
                let purpose = target.purpose();
                let text = if failed {
                    "I couldn't finish answering that question. Please ask again."
                } else {
                    &reply.text
                };
                use crate::services::channel_text_preparation::{Parent, with_parent};
                let parent = Parent::SavedReply {
                    id: reply.id.clone(),
                    claim: lease.clone(),
                };
                let guard = authority(state, &row, &target.origin);
                let mut decision = supplemental::Decision::new(purpose);
                decision.saved_result = true;
                let result = Box::pin(crate::services::channel_platform::with_post_effect_guard(
                    guard,
                    with_parent(
                        parent,
                        deliver_from_source(
                            state,
                            &row,
                            &target.origin,
                            text,
                            images.as_ref(),
                            decision,
                            Some(target.source),
                        ),
                    ),
                ))
                .await;
                if let Err(error) = result {
                    retry |= matches!(
                        error,
                        AppError::DatabaseError(_)
                            | AppError::SecurityReadinessUnavailable
                            | AppError::Internal(_)
                    ) || error.retry_after_seconds().is_some();
                }
            }
        }
    }
    if retry {
        return Ok(true);
    }
    messages
        .update_one(
            doc! {"_id":reply.id,"handoff_delivery_lease":lease},
            doc! {"$unset":{"handoff_delivery_due_at":"","handoff_delivery_lease":""}},
        )
        .await?;
    Ok(true)
}
