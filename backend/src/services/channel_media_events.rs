//! Native media acceptance and crash disposition. Admission never downloads.
use super::{
    channel_media_authority::{self as authority, MediaRequest},
    feature_flag_service as flags,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::ChannelOrigin,
        channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
        nyxbot_channel::{
            EVENTS_COLLECTION_NAME as EVENTS, EventMedia, EventMediaItem, NyxbotEvent,
            NyxbotThread, THREADS_COLLECTION_NAME as THREADS,
        },
    },
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use sha2::{Digest, Sha256};

pub const INTERRUPTED_NOTE: &str = "A previous photo or file in this chat was interrupted before the assistant could process it. It was not processed; please resend it if it is still needed.";
const BATCH: i64 = 50;

/// Presence is independent of owner eligibility. Existing accepted media keeps
/// draining after flag disable; guests only get presence, never a claim.
pub async fn for_turn(
    db: &Database,
    channel: &crate::models::nyxbot_channel::NyxbotChannel,
    event_key: &str,
) -> AppResult<(usize, Vec<String>, Option<chrono::DateTime<Utc>>)> {
    let Some(event) = db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {
            "_id":event_key,"channel_id":&channel.id,"user_id":&channel.user_id
        })
        .await?
    else {
        return Ok((0, Vec::new(), None));
    };
    let declared = event
        .source_media_declared
        .unwrap_or_else(|| event.media.as_ref().map_or(0, |m| m.declared));
    let ids = event
        .media
        .as_ref()
        .map(|_| vec![event.id])
        .unwrap_or_default();
    Ok((declared, ids, Some(event.created_at)))
}

/// Only the closed CMA Lark media shape may replace rendered input_text.
/// Locators here are never used to fetch; selectors come from the stored row.
pub fn gateway_text<'a>(activity: &'a serde_json::Value, platform: &str) -> Option<&'a str> {
    if !matches!(platform, "lark" | "feishu")
        || activity["source"]["type"] != "nyxid_relay"
        || activity["source"]["platform"] != platform
        || activity["kind"]["type"] != "message"
    {
        return None;
    }
    let items = activity["kind"]["attachments"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 16)?;
    if !items.iter().all(|item| {
        (item["variant"] == "embedded" && item["source"].is_null())
            || (matches!(item["kind"].as_str(), Some("image" | "file"))
                && item["source"]["type"] == "nyxid_relay"
                && item["source"]["index"].as_u64().is_some_and(|i| i < 32))
    }) {
        return None;
    }
    activity["kind"]["text"]
        .as_str()
        .filter(|t| t.len() <= 64 * 1024)
}

/// Missing rows produce an explicit failed-media turn; an existing foreign row
/// is a deterministic Forbidden refusal rather than laundering an attacker-selected
/// event identifier. Database failures retain their retryable error class.
pub async fn presence(
    db: &Database,
    request: &MediaRequest,
    missing_media: bool,
) -> AppResult<usize> {
    if !matches!(request.channel.platform.as_str(), "lark" | "feishu")
        || !flags::personal_flag_enabled(
            db,
            &request.channel.user_id,
            flags::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
        )
        .await?
    {
        return Ok(0);
    }
    match authority::source(db, request).await {
        Ok(source) => {
            if source
                .attachments
                .iter()
                .any(|a| !matches!(a.content_type.as_str(), "image" | "file"))
            {
                return Ok(0);
            }
            Ok(source.attachments.len()
                + source.embedded_attachments.len()
                + source.embedded_attachments_omitted)
        }
        Err(AppError::NotFound(_)) => {
            if db
                .collection::<ChannelMessage>(MESSAGES)
                .find_one(doc! {"_id":&request.source_message_id})
                .await?
                .is_some()
            {
                Err(AppError::Forbidden(
                    "Channel media source is not authorized".into(),
                ))
            } else {
                Ok(usize::from(missing_media))
            }
        }
        Err(error) => Err(error),
    }
}

/// Source presence is computed for every sender. Only a verified owner with
/// live authority can acquire a media claim. No gateway handles enter this API.
pub async fn admission(db: &Database, request: &MediaRequest) -> AppResult<Option<EventMedia>> {
    if !matches!(request.channel.platform.as_str(), "lark" | "feishu") {
        return Ok(None);
    }
    let source = match authority::source(db, request).await {
        Ok(source) => source,
        Err(crate::errors::AppError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error),
    };
    let declared = source.attachments.len()
        + source.embedded_attachments.len()
        + source.embedded_attachments_omitted;
    if declared == 0
        || !request
            .channel
            .owner_sender_ids
            .contains(&request.sender_id)
        || !flags::personal_flag_enabled(
            db,
            &request.channel.user_id,
            flags::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
        )
        .await?
    {
        return Ok(None);
    }
    // Audio/video retain their existing transcript path; no media ingestion.
    if source
        .attachments
        .iter()
        .any(|item| !matches!(item.content_type.as_str(), "image" | "file"))
    {
        return Ok(None);
    }
    let authorized = match Box::pin(authority::resolve(db, request)).await {
        Ok(authorized) => authorized,
        Err(crate::errors::AppError::NotFound(_) | crate::errors::AppError::Forbidden(_)) => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let source = authorized.source;
    let Some(native) = source
        .native_event_id
        .as_deref()
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .or_else(|| {
            source
                .platform_message_id
                .as_deref()
                .filter(|id| !id.is_empty() && id.len() <= 256)
        })
    else {
        return Ok(None);
    };
    let key = format!(
        "{:x}",
        Sha256::digest(
            format!(
                "v1\0{}\0{}\0{}\0{}",
                authorized.bot.id, request.channel.platform, request.chat_id, native
            )
            .as_bytes()
        )
    );
    let items = [
        (false, &source.attachments),
        (true, &source.embedded_attachments),
    ]
    .into_iter()
    .flat_map(|(embedded, items)| {
        items
            .iter()
            .enumerate()
            .map(move |(index, item)| EventMediaItem {
                outcome: None,
                index,
                embedded,
                kind: item.content_type.clone(),
                size_bytes: item.size_bytes,
            })
    })
    .take(10)
    .collect();
    Ok(Some(EventMedia {
        authority: Some(crate::models::nyxbot_channel::MediaAuthority {
            route_api_key_id: request.channel.route_api_key_id.clone(),
            route_id: request.channel.route_id.clone(),
            agent_id: request.channel.agent_id.clone(),
            bot_owner_id: request.channel.bot_owner_id.clone(),
            follow_binding_generation: request.channel.follow_binding_generation,
        }),
        key,
        declared,
        items,
        channel_bot_id: authorized.bot.id,
        platform: request.channel.platform.clone(),
        platform_chat_id: request.chat_id.clone(),
        interrupted: false,
        noted: false,
    }))
}

/// Media acceptance contends with the durable owner-erasure fence. A held
/// callback cannot recreate a media source after local inventory has drained.
pub async fn insert_event(db: &Database, event: &NyxbotEvent) -> mongodb::error::Result<()> {
    if event.media.is_none() {
        db.collection::<NyxbotEvent>(EVENTS)
            .insert_one(event)
            .await?;
        return Ok(());
    }
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let event = event.clone();
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                super::erasure_admission::fence_in_session(&db, &event.user_id, session).await?;
                db.collection::<NyxbotEvent>(EVENTS)
                    .insert_one(&event)
                    .session(session)
                    .await?;
                Ok(())
            }
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    let rows = db.collection::<bson::Document>(EVENTS);
    rows.create_index(
        IndexModel::builder()
            .keys(doc! {"media.key":1})
            .options(
                IndexOptions::builder()
                    .name("nyxbot_media_key_unique".to_owned())
                    .unique(true)
                    .partial_filter_expression(doc! {"media.key":{"$type":"string"}})
                    .build(),
            )
            .build(),
    )
    .await?;
    rows.create_index(
        IndexModel::builder()
            .keys(doc! {"status":1,"conversation_id":1,"created_at":1})
            .options(
                IndexOptions::builder()
                    .name("nyxbot_media_unbound".to_owned())
                    .partial_filter_expression(doc! {"media.key":{"$type":"string"}})
                    .build(),
            )
            .build(),
    )
    .await?;
    Ok(())
}

/// A different relay UUID lost the native-media insert race. Store one bounded
/// alias only on the exact scoped source row; same-ID provider replays continue
/// through the old replay/resume contract.
pub async fn record_duplicate(db: &Database, event: &NyxbotEvent) -> AppResult<bool> {
    let Some(media) = &event.media else {
        return Ok(false);
    };
    let winner = db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"media.key":&media.key})
        .await?;
    let Some(winner) = winner else {
        return Ok(false);
    };
    if winner.relay_message_id == event.relay_message_id {
        return Ok(false);
    }
    let Some(source) = &event.relay_message_id else {
        return Ok(false);
    };
    db.collection::<ChannelMessage>(MESSAGES)
        .update_one(
            doc! {"_id":source,"direction":"inbound"},
            doc! {"$set":{"duplicate_of_media_key":&media.key}},
        )
        .await?;
    Ok(true)
}

/// The conditional write conflicts with both transactional turn and queue
/// admission. A late worker cannot bind a classified event after this barrier.
pub async fn sweep(db: &Database) -> AppResult<u64> {
    let cutoff = bson::DateTime::from_chrono(Utc::now() - Duration::minutes(5));
    let filter = doc! {"media.key":{"$type":"string"},"media.interrupted":{"$ne":true},
    "status":{"$in":["running","resuming","retryable"]},"conversation_id":bson::Bson::Null,
    "turn_id":bson::Bson::Null,"created_at":{"$lte":cutoff}};
    let rows = db.collection::<bson::Document>(EVENTS);
    let candidates: Vec<_> = rows
        .find(filter.clone())
        .projection(doc! {"_id":1})
        .sort(doc! {"created_at":1})
        .limit(BATCH)
        .await?
        .try_collect()
        .await?;
    let mut changed = 0;
    for row in candidates {
        let mut exact = filter.clone();
        exact.insert("_id", row.get("_id").cloned());
        changed += rows
            .update_one(
                exact,
                doc! {"$set":{"media.interrupted":true,"status":"interrupted"},
                "$unset":{"event_context_ciphertext":""}},
            )
            .await?
            .modified_count;
    }
    Ok(changed)
}

/// Consume at most one bounded batch in the turn transaction. A failed start
/// rolls back the marks; guests and unrelated chats never consume owner notes.
pub async fn note_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    origins: &[ChannelOrigin],
) -> AppResult<bool> {
    let mut noted = false;
    for origin in origins {
        let Some(chat) = db.collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"channel_id":&origin.nyxbot_channel_id,"user_id":owner,"partition":&origin.partition})
            .session(&mut *session).await? else { continue };
        let Some(chat_id) = chat.platform_chat_id else {
            continue;
        };
        let Some(channel) = db
            .collection::<crate::models::nyxbot_channel::NyxbotChannel>(
                crate::models::nyxbot_channel::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&origin.nyxbot_channel_id,"user_id":owner})
            .session(&mut *session)
            .await?
        else {
            continue;
        };
        // Native identity survives rebuilding a bot's channel. Keep the owner
        // fence while consuming interruptions from that same bot/platform/chat.
        let filter = doc! {"user_id":owner,
        "media.key":{"$type":"string"},"media.platform_chat_id":chat_id,"media.noted":{"$ne":true},
        "$and":[{"$or":[{"channel_id":&origin.nyxbot_channel_id},
            {"media.channel_bot_id":&channel.channel_bot_id,"media.platform":&channel.platform}]},
        {"$or":[{"media.interrupted":true},
            {"status":{"$in":["running","resuming","retryable"]},"conversation_id":bson::Bson::Null,
                "turn_id":bson::Bson::Null,"created_at":{"$lte":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(5))}}]}]};
        let rows = db.collection::<bson::Document>(EVENTS);
        let mut cursor = rows
            .find(filter.clone())
            .limit(BATCH)
            .session(&mut *session)
            .await?;
        let mut ids = Vec::new();
        while let Some(row) = cursor.next(&mut *session).await.transpose()? {
            ids.push(row.get("_id").cloned());
        }
        if !ids.is_empty() {
            let mut exact = filter;
            exact.insert("_id", doc! {"$in":ids});
            noted |= rows
                .update_many(exact, doc! {"$set":{"media.noted":true,"media.interrupted":true,"status":"interrupted"},
                    "$unset":{"event_context_ciphertext":""}})
                .session(&mut *session)
                .await?
                .modified_count
                > 0;
        }
    }
    Ok(noted)
}
