//! Durable bounds for native bot handoffs; metadata only, no timer refill.
use super::{
    api_key_mutation_service as transactions, assistant_settings_service,
    coordination_service::RateWindowStore,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        channel_thread::ThreadSenderKind,
        nyxbot_channel::{
            EVENTS_COLLECTION_NAME as EVENTS, NyxbotChannel, NyxbotThread,
            THREADS_COLLECTION_NAME as THREADS,
        },
    },
};
use mongodb::{
    Database,
    bson::{self, doc},
};
use std::time::Duration;

const BUDGET_DENIED: &str = "native_bot_budget_denied";

/// A platform redelivery can receive a fresh relay UUID. Reuse the native
/// identity only from this installation's immutable, authenticated ingress
/// metadata. Native gateway sources without a relay row retain their existing
/// provider event identity. No content or provider identity lookup is needed.
/// Human native redeliveries share the claim too, so they cannot refill bot
/// hops again. Keep the v1 hash domain stable for already claimed bot messages.
pub async fn relay_event_key(
    db: &Database,
    channel: &NyxbotChannel,
    source_id: &str,
    chat_id: &str,
    sender_id: &str,
) -> AppResult<Option<String>> {
    use sha2::{Digest, Sha256};
    let mut filter = doc! {
        "_id": source_id, "direction": "inbound", "channel_bot_id": &channel.channel_bot_id,
        "user_id": channel.bot_owner_id.as_deref().unwrap_or(&channel.user_id),
        "platform": &channel.platform, "agent_api_key_id": &channel.route_api_key_id,
        "platform_conversation_id": chat_id, "sender_platform_id": sender_id,
    };
    if let Some(route) = &channel.route_id {
        filter.insert("conversation_id", route);
    }
    let Some(source) = db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(filter)
        .projection(doc! {"platform_message_id": 1, "native_event_id": 1})
        .await?
    else {
        return Ok(None);
    };
    let native = source
        .get_str("native_event_id")
        .ok()
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .or_else(|| {
            // Lark/Feishu legacy rows predate the persisted provider event
            // identity. Their message IDs are immutable event identities. A
            // Telegram row without update_id is intentionally not collapsed:
            // message_id is reused by edited_message and cannot prove replay.
            (!matches!(channel.platform.as_str(), "telegram" | "telegram-new"))
                .then(|| source.get_str("platform_message_id").ok())
                .flatten()
                .filter(|id| !id.is_empty() && id.len() <= 256)
        });
    let Some(native) = native else {
        return Ok(None);
    };
    let mut hash = Sha256::new();
    for part in [
        "native-bot-source:v1",
        &channel.channel_bot_id,
        &channel.platform,
        chat_id,
        native,
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    Ok(Some(format!("{:x}", hash.finalize())))
}

/// Called after native addressing and live chat consent. The existing source
/// event fences retries; settings-row budget and the owner's hourly handoff
/// window commit together. Claimed attempts, including uncertain/busy work,
/// are never refunded. Human messages alone refill the per-chat budget.
#[cfg(test)]
pub async fn admit(
    db: &Database,
    channel: &NyxbotChannel,
    chat: &NyxbotThread,
    kind: ThreadSenderKind,
    event: &str,
) -> AppResult<bool> {
    admit_with_owner_reset(db, channel, chat, kind, true, event).await
}

/// Admit a turn while optionally retaining the existing human group-hop
/// allowance. Authenticated Telegram channel posts have no user actor, so
/// they may use their legacy guest path without replenishing automated hops.
pub async fn admit_with_owner_reset(
    db: &Database,
    channel: &NyxbotChannel,
    chat: &NyxbotThread,
    kind: ThreadSenderKind,
    owner_reset: bool,
    event: &str,
) -> AppResult<bool> {
    if !matches!(kind, ThreadSenderKind::Human | ThreadSenderKind::Bot) {
        return Ok(false);
    }
    if !matches!(chat.kind.as_deref(), Some("group" | "channel")) {
        return Ok(kind == ThreadSenderKind::Human);
    }
    let settings = assistant_settings_service::get(db, &channel.user_id).await?;
    let limit = settings.max_group_handoffs.max(0);
    let hourly = settings.max_group_handoffs_per_hour.max(0) as u64;
    if kind == ThreadSenderKind::Bot && (limit == 0 || hourly == 0) {
        return Ok(false);
    }
    let db = db.clone();
    let channel = channel.clone();
    let chat = chat.clone();
    let event = event.to_owned();
    let mut session = db.client().start_session().await?;
    let result = session
        .start_transaction()
        .and_run2(async move |session| {
            let work: AppResult<bool> = async {
                let events = db.collection::<bson::Document>(EVENTS);
                let mut event_filter = doc! {
                    "_id": &event, "user_id": &channel.user_id, "channel_id": &channel.id,
                    "status": {"$in": ["running", "resuming"]},
                };
                let Some(prior) = events
                    .find_one(event_filter.clone())
                    .projection(doc! {"native_bot_budget_claimed": 1, "status": 1})
                    .session(&mut *session)
                    .await?
                else {
                    return Ok(false);
                };
                if prior.get_bool("native_bot_budget_claimed").unwrap_or(false) {
                    if kind != ThreadSenderKind::Human || prior.get_str("status") != Ok("resuming")
                    {
                        return Ok(false);
                    }
                    // The provider atomically resumed an identical, no-turn
                    // human Busy event. Its first reset remains consumed.
                    event_filter.insert("status", "resuming");
                    event_filter.insert("native_bot_budget_claimed", true);
                    return Ok(events
                        .update_one(event_filter, doc! {"$set": {"status": "running"}})
                        .session(&mut *session)
                        .await?
                        .matched_count
                        == 1);
                }
                if prior.get_str("status") != Ok("running") {
                    return Ok(false);
                }
                event_filter.insert("status", "running");
                event_filter.insert("native_bot_budget_claimed", doc! {"$ne": true});
                let mut filter = doc! {
                    "_id": &chat.id, "channel_id": &channel.id, "user_id": &channel.user_id,
                    "record_scope": {"$ne": super::channel_thread_follow_service::SCOPE},
                    "kind": {"$in": ["group", "channel"]},
                };
                let rows = db.collection::<bson::Document>(THREADS);
                if kind == ThreadSenderKind::Bot {
                    filter.insert(
                        "$or",
                        bson::bson!([
                            {"members": "everyone"}, {"members": null, "owner_seen": true}
                        ]),
                    );
                    filter.insert(
                        "$expr",
                        doc! {
                            "$gt": [{"$ifNull": ["$native_bot_hops_remaining", limit]}, 0]
                        },
                    );
                    // Contend on the existing chat row before reserving the owner
                    // window, so concurrent replicas serialize against one allowance.
                    let spent = rows.update_one(filter, vec![doc! {"$set": {
                    "native_bot_hops_remaining": {"$subtract": [
                        {"$min": [{"$ifNull": ["$native_bot_hops_remaining", limit]}, limit]}, 1
                    ]}
                }}]).session(&mut *session).await?;
                    if spent.matched_count != 1 {
                        return Ok(false);
                    }
                    if !RateWindowStore::admit_in_session(
                        &db,
                        "assistant_group_handoffs",
                        &channel.user_id,
                        hourly,
                        Duration::from_secs(3600),
                        session,
                    )
                    .await?
                    {
                        // Abort both the chat spend and source-event claim on
                        // hourly denial. Dispatched/uncertain work never reaches this path.
                        return Err(AppError::Conflict(BUDGET_DENIED.into()));
                    }
                } else if owner_reset {
                    let reset = rows
                        .update_one(
                            filter,
                            doc! {"$set": {
                                "native_bot_hops_remaining": limit,
                            }},
                        )
                        .session(&mut *session)
                        .await?;
                    if reset.matched_count != 1 {
                        return Ok(false);
                    }
                }
                events
                    .update_one(
                        event_filter,
                        doc! {"$set": {
                            "native_bot_budget_claimed": true,
                        }},
                    )
                    .session(&mut *session)
                    .await?;
                Ok(true)
            }
            .await;
            transactions::transaction_result(work)
        })
        .await
        .map_err(transactions::map_transaction_error);
    match result {
        Err(AppError::Conflict(code)) if code == BUDGET_DENIED => Ok(false),
        result => result,
    }
}
