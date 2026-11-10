//! Participant-only read state and durable, bounded mention push delivery.
use super::*;
use crate::{
    models::assistant_group::{GroupMessage, MESSAGES_COLLECTION_NAME as MESSAGES},
    services::api_key_mutation_service as transactions,
};
use serde::Serialize;

const COALESCE_SECONDS: i64 = 300;
#[derive(Default, Serialize)]
pub struct Counts {
    pub last_seen_seq: i64,
    pub unread_count: u64,
    pub mention_count: u64,
}

pub async fn counts(db: &Database, actor: &str, id: &str) -> AppResult<Counts> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await?;
    read_group(db, actor, id, &mut session).await?;
    let seen = db
        .collection::<model::ReadState>(model::READ_STATES)
        .find_one(doc! {"group_id":id,"user_id":actor})
        .session(&mut session)
        .await?
        .map_or(0, |s| s.last_seen_seq);
    let base = doc! {"group_id":id,"collaboration.version":1,"seq":{"$gt":seen},
    "role":{"$in":["user","agent"]},"author_user_id":{"$ne":actor}};
    let unread_count = db
        .collection::<GroupMessage>(MESSAGES)
        .count_documents(base.clone())
        .session(&mut session)
        .await?;
    let mut mentions = base;
    mentions.insert("mentioned_participant_ids", actor);
    let mention_count = db
        .collection::<GroupMessage>(MESSAGES)
        .count_documents(mentions)
        .session(&mut session)
        .await?;
    session.commit_transaction().await?;
    Ok(Counts {
        last_seen_seq: seen,
        unread_count,
        mention_count,
    })
}

pub async fn mark_read(db: &Database, actor: &str, id: &str, seq: i64) -> AppResult<()> {
    require_available(db, actor).await?;
    mark_read_ready(db, actor, id, seq).await
}
async fn mark_read_ready(db: &Database, actor: &str, id: &str, seq: i64) -> AppResult<()> {
    if seq < 0 {
        return Err(invalid("Read position must not be negative"));
    }
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .and_run(
            (db.clone(), actor.to_owned(), id.to_owned()),
            |session, ctx| {
                let (db, actor, id) = &*ctx;
                Box::pin(async move {
                    let result: AppResult<()> = async {
                        let group = read_group(db, actor, id, session).await?;
                        fence_group(db, &group, session).await?;
                        ensure_state(db, actor, id, session).await?;
                        db.collection::<model::ReadState>(model::READ_STATES)
                            .update_one(
                                doc! {"group_id":id,"user_id":actor},
                                doc! {"$max":{"last_seen_seq":seq.min(group.message_count)}},
                            )
                            .session(session)
                            .await?;
                        Ok(())
                    }
                    .await;
                    transactions::transaction_result(result)
                })
            },
        )
        .await
        .map_err(transactions::map_transaction_error)
}
async fn ensure_state(
    db: &Database,
    actor: &str,
    id: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let row = model::ReadState {
        id: uuid::Uuid::new_v4().to_string(),
        group_id: id.into(),
        user_id: actor.into(),
        last_seen_seq: 0,
        last_push_at: None,
    };
    db.collection::<model::ReadState>(model::READ_STATES).update_one(doc! {"group_id":id,"user_id":actor},
        doc! {"$setOnInsert":bson::to_document(&row).map_err(|e|AppError::Internal(e.to_string()))?})
        .upsert(true).session(session).await?;
    Ok(())
}

/// Called inside each public message transaction, after live resolution. No
/// additional work on messages with no people mentions or on legacy groups.
pub(super) async fn enqueue(
    db: &Database,
    group: &AssistantGroup,
    message: &GroupMessage,
    session: &mut ClientSession,
) -> AppResult<()> {
    if message.collaboration.is_none() || !matches!(message.role.as_str(), "user" | "agent") {
        return Ok(());
    }
    for user in &message.mentioned_participant_ids {
        if message.author_user_id.as_ref() == Some(user)
            || !group.participant_user_ids.contains(user)
        {
            continue;
        }
        let row = model::MentionNotification {
            id: uuid::Uuid::new_v4().to_string(),
            group_id: group.id.clone(),
            user_id: user.clone(),
            message_id: message.id.clone(),
            seq: message.seq,
            claimed: false,
            created_at: Utc::now(),
        };
        db.collection::<model::MentionNotification>(model::MENTION_NOTIFICATIONS)
            .insert_one(row)
            .session(&mut *session)
            .await?;
    }
    Ok(())
}

/// Departure removes pending delivery, so a later rejoin cannot revive it.
/// Preserve the private read/coalescing marker: leave/rejoin must not reset the
/// five-minute push limit. Only live participants can read or advance it; group
/// deletion and personal erasure remove it with the rest of their inventory.
pub(super) async fn depart(
    db: &Database,
    group: &str,
    user: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<bson::Document>(crate::models::assistant_notification::COLLECTION_NAME)
        .delete_many(doc! {"source.group_id":group,"user_id":user})
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(model::MENTION_NOTIFICATIONS)
        .delete_many(doc! {"group_id":group,"user_id":user})
        .session(session)
        .await?;
    Ok(())
}

async fn claim(db: &Database, item: &model::MentionNotification) -> AppResult<bool> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().read_concern(mongodb::options::ReadConcern::snapshot())
        .and_run((db.clone(),item.clone()),|session,ctx| {let (db,item)=&*ctx;Box::pin(async move {
            let work:AppResult<bool>=async {
                let group=read_group(db,&item.user_id,&item.group_id,session).await?;
                fence_group(db,&group,session).await?;
                if db.collection::<model::MentionNotification>(model::MENTION_NOTIFICATIONS).update_one(
                    doc! {"_id":&item.id,"claimed":false},doc! {"$set":{"claimed":true}}).session(&mut *session).await?.matched_count!=1 {return Ok(false);}
                ensure_state(db,&item.user_id,&item.group_id,session).await?;
                // One shared limiter for human AND agent mentions: a chatty
                // agent cannot bypass the per-person, per-group five-minute cap.
                let cutoff=Utc::now()-chrono::Duration::seconds(COALESCE_SECONDS);
                Ok(db.collection::<model::ReadState>(model::READ_STATES).update_one(
                    doc! {"group_id":&item.group_id,"user_id":&item.user_id,"last_seen_seq":{"$lt":item.seq},
                    "$or":[{"last_push_at":null},{"last_push_at":{"$lte":bson::DateTime::from_chrono(cutoff)}}]},
                    doc! {"$set":{"last_push_at":bson::DateTime::now()}}).session(session).await?.matched_count==1)
            }.await; transactions::transaction_result(work)
        })}).await.map_err(transactions::map_transaction_error)
}

/// A bounded sweep also recovers a crash between message commit and scheduling.
/// Claimed effects are never retried (an ambiguous send must not duplicate).
pub async fn deliver(state: &crate::AppState, group: Option<&str>) -> AppResult<()> {
    // Bound immediate dispatch tasks on each replica; the periodic sweep
    // recovers intents queued while another delivery batch holds this permit.
    static DELIVERY: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let Ok(_permit) = DELIVERY.try_acquire() else {
        return Ok(());
    };
    let mut filter = doc! {"claimed":false};
    if let Some(group) = group {
        filter.insert("group_id", group);
    }
    let pending: Vec<model::MentionNotification> = state
        .db
        .collection(model::MENTION_NOTIFICATIONS)
        .find(filter)
        .sort(doc! {"created_at":1})
        .limit(64)
        .await?
        .try_collect()
        .await?;
    for item in pending {
        let result = deliver_one(state, &item).await;
        if result.is_err() {
            tracing::debug!(group_id=%item.group_id,"Group mention delivery skipped");
        }
        // No content or tokens in this queue. Failed/disabled/stale delivery is
        // best-effort; in-app counts remain the durable source of truth.
        state
            .db
            .collection::<model::MentionNotification>(model::MENTION_NOTIFICATIONS)
            .delete_one(doc! {"_id":&item.id})
            .await?;
    }
    Ok(())
}
async fn deliver_one(state: &crate::AppState, item: &model::MentionNotification) -> AppResult<()> {
    deliver_one_with(state, item, |user, group, name, sender| async move {
        crate::services::notification_service::send_group_mention_push(
            state, &user, &group, &name, &sender,
        )
        .await
    })
    .await
}
async fn deliver_one_with<F, Fut>(
    state: &crate::AppState,
    item: &model::MentionNotification,
    send: F,
) -> AppResult<()>
where
    F: FnOnce(String, String, String, String) -> Fut,
    Fut: std::future::Future<Output = AppResult<bool>>,
{
    require_available(&state.db, &item.user_id).await?;
    let channel=state.db.collection::<crate::models::notification_channel::NotificationChannel>(crate::models::notification_channel::COLLECTION_NAME)
        .find_one(doc! {"user_id":&item.user_id,"push_enabled":true,"group_mentions_enabled":{"$ne":false}}).await?;
    if channel.is_none_or(|c| c.push_devices.is_empty()) || !claim(&state.db, item).await? {
        return Ok(());
    }
    // Recheck readability immediately before dispatch. Only public transcript
    // metadata is loaded; no private thread, draft, card or owner memory.
    let group = get(&state.db, &item.user_id, &item.group_id).await?;
    let message=state.db.collection::<bson::Document>(MESSAGES).find_one(doc! {"_id":&item.message_id,"group_id":&item.group_id,
        "collaboration.version":1,"role":{"$in":["user","agent"]},"mentioned_participant_ids":&item.user_id,"author_user_id":{"$ne":&item.user_id}})
        .projection(doc! {"agent_id":1,"author_display_name":1}).await?.ok_or_else(missing)?;
    let sender = if let Ok(agent) = message.get_str("agent_id") {
        roster(&state.db, &item.user_id, &item.group_id)
            .await?
            .into_iter()
            .find(|a| a.id == agent)
            .map(|a| a.display_name.unwrap_or(a.name))
            .unwrap_or_else(|| "An agent".into())
    } else {
        message
            .get_str("author_display_name")
            .unwrap_or("A participant")
            .to_owned()
    };
    let delivered = send(
        item.user_id.clone(),
        group.id.clone(),
        group.name.clone(),
        sender,
    )
    .await?;
    let _=crate::services::audit_service::log_actor_event(state.db.clone(),&crate::services::audit_service::AuditActor {
        user_id:item.user_id.clone(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None
    },"assistant_group_mention_notification",Some(serde_json::json!({"group_id":group.id,"message_id":item.message_id,"delivered":delivered}))).await;
    Ok(())
}

#[cfg(test)]
#[path = "notification_tests.rs"]
mod tests;
