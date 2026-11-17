//! Metadata-only lifecycle of relay events whose answer remains locally delivered.
use crate::{
    errors::{AppError, AppResult},
    models::{
        nyxbot_channel::{EVENTS_COLLECTION_NAME as EVENTS, NyxbotChannel, NyxbotEvent},
        nyxbot_response_read::{Command, ReplyPayload, ResponseRead},
    },
    services::nyxbot_response_recovery::{self as recovery, Payload},
};
use bson::{Document, doc};
use chrono::{Duration, Utc};
use mongodb::{Database, IndexModel, options::IndexOptions};
use serde_json::Value;

pub fn command(id: &str, request: &Value) -> Command {
    let metadata = &request["metadata"];
    Command {
        response_id: id.into(),
        request_hash: crate::crypto::token::hash_token(&request.to_string()),
        metadata: [
            "cmaeg.channel",
            "cmaeg.event",
            "cmaeg.partition",
            "cmaeg.actor",
        ]
        .into_iter()
        .filter_map(|key| {
            metadata[key]
                .as_str()
                .filter(|v| !v.is_empty() && v.len() <= 512)
                .map(|value| (key.into(), value.into()))
        })
        .collect(),
    }
}

pub fn identity(channel: &NyxbotChannel, event: &NyxbotEvent, command: Command) -> ResponseRead {
    let digest = crate::crypto::token::hash_token(&format!(
        "{}:{}:{}",
        channel.id,
        channel.binding_id.as_deref().unwrap_or_default(),
        event.partition
    ));
    ResponseRead {
        version: 1,
        binding_id: channel.binding_id.clone(),
        conversation_id: format!("nyxbot_{}", &digest[..32]),
        commands: vec![command],
        thread_reply: None,
        expires_at: event.created_at + Duration::hours(recovery::RETENTION_HOURS),
    }
}

pub async fn admit(
    db: &Database,
    channel: &NyxbotChannel,
    event: &NyxbotEvent,
    command: Command,
) -> AppResult<Option<ResponseRead>> {
    if channel.transport != "gateway"
        || channel.gateway_source.is_some()
        || event.relay_message_id.is_none()
        || event.custom_binding_id.is_some()
    {
        return Ok(None);
    }
    let mapping = identity(channel, event, command);
    let value = bson::to_bson(&mapping)
        .map_err(|_| AppError::Internal("Response identity unavailable".into()))?;
    let changed = db
        .collection::<Document>(EVENTS)
        .update_one(
            doc! {
                "_id":&event.id,"channel_id":&channel.id,"user_id":&channel.user_id,
                "turn_id":bson::Bson::Null,"status":{"$in":["running","resuming"]},
                "gateway_response_id":bson::Bson::Null,"response_read_admitted":{"$ne":true},
                "response_read_pending.version":1,
                "response_read_pending.commands":{"$elemMatch":{"response_id":&mapping.commands[0].response_id,"request_hash":&mapping.commands[0].request_hash}},
            },
            vec![doc! {"$set":{
                "response_read":{"$mergeObjects":[{"$literal":value},{"commands":"$response_read_pending.commands"}]},
                "response_read_admitted":true,
            }},doc! {"$unset":"response_read_pending"}],
        )
        .await?;
    if changed.matched_count != 1 {
        return Err(AppError::Conflict("Response admission changed".into()));
    }
    Ok(Some(mapping))
}

pub fn mapping(raw: &Document) -> AppResult<Option<ResponseRead>> {
    raw.get_document("response_read")
        .ok()
        .map(|value| {
            bson::from_document(value.clone())
                .map_err(|_| AppError::Internal("Response identity unavailable".into()))
        })
        .transpose()
}

pub struct View {
    pub payload: Payload,
    pub terminal: bool,
    pub admitted: bool,
    pub retryable: bool,
    pub reply_ciphertext: Option<Vec<u8>>,
}

pub async fn freeze_thread(db: &Database, event: &str, thread_reply: bool) -> AppResult<()> {
    let changed = db.collection::<Document>(EVENTS).update_one(doc! {
        "_id":event,"response_read.version":1,"gateway_response_id":bson::Bson::Null,
        "response_read.thread_reply":{"$in":[bson::Bson::Null,bson::Bson::Boolean(thread_reply)]},
    },doc! {"$set":{"response_read.thread_reply":thread_reply}}).await?;
    if changed.matched_count != 1 {
        return Err(AppError::Conflict("Response routing changed".into()));
    }
    Ok(())
}

pub async fn load(db: &Database, channel: &NyxbotChannel, id: &str) -> AppResult<Option<View>> {
    if channel.transport != "gateway"
        || channel.status != "active"
        || channel.gateway_source.is_some()
    {
        return Ok(None);
    }
    let Some(raw) = db.collection::<Document>(EVENTS).find_one(doc! {
        "channel_id":&channel.id,"user_id":&channel.user_id,"custom_binding_id":bson::Bson::Null,
        "response_read.version":1,"response_read.commands.response_id":id,
        "response_read.binding_id":&channel.binding_id,
        "response_read.expires_at":{"$gt":bson::DateTime::now()},"expires_at":{"$gt":bson::DateTime::now()},
        "gateway_response_id":bson::Bson::Null,
    }).projection(doc! {"event_context_ciphertext":0,"media":0}).await? else { return Ok(None) };
    view(db, channel, &raw, id).await
}

pub async fn view(
    db: &Database,
    channel: &NyxbotChannel,
    raw: &Document,
    id: &str,
) -> AppResult<Option<View>> {
    let Some(mapping) = mapping(raw)? else {
        return Ok(None);
    };
    if channel.transport != "gateway"
        || channel.status != "active"
        || channel.gateway_source.is_some()
        || raw.get_str("channel_id").ok() != Some(&channel.id)
        || raw.get_str("user_id").ok() != Some(&channel.user_id)
        || raw.get_str("gateway_response_id").is_ok()
        || mapping.version != 1
        || mapping.expires_at <= Utc::now()
        || mapping.binding_id != channel.binding_id
    {
        return Ok(None);
    }
    let Some(command) = mapping.commands.iter().find(|c| c.response_id == id) else {
        return Ok(None);
    };
    let event: NyxbotEvent = bson::from_document(raw.clone())
        .map_err(|_| AppError::Internal("Response identity unavailable".into()))?;
    if event.expires_at <= Utc::now()
        || event.custom_binding_id.is_some()
        || event.relay_message_id.is_none()
    {
        return Ok(None);
    }
    if db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {
            "_id":&channel.user_id,"is_active":true,"platform_personal_deleting":{"$ne":true},
        })
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Ok(None);
    }
    if let Some(origin) = event.delivery.as_ref().map(|d| &d.origin)
        && let Some(execution) = &origin.org_execution
    {
        Box::pin(super::org_channel_service::validate(
            db,
            origin,
            &execution.actor_id,
            &execution.agent_id,
        ))
        .await?;
    }
    let terminal =
        if let (Some(conversation), Some(turn)) = (&event.conversation_id, &event.turn_id) {
            let actor = super::org_channel_service::event_actor(&event);
            if db
                .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
                .find_one(doc! {
                    "_id":conversation,"user_id":actor,
                })
                .projection(doc! {"_id":1})
                .await?
                .is_none()
            {
                return Ok(None);
            }
            db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
            .find_one(doc! {
                "user_id":actor,"conversation_id":conversation,"turn_id":turn,"role":"assistant",
                "status":{"$in":["completed","failed","cancelled","interrupted"]},
            })
            .projection(doc! {"_id":0,"status":1})
            .await?
            .is_some()
        } else {
            matches!(
                event.status.as_str(),
                "completed" | "failed" | "refused" | "cancelled" | "interrupted" | "silent"
            )
        };
    let mut payload = Payload::new(id, &mapping.conversation_id, command.metadata.clone());
    if mapping.thread_reply == Some(true) {
        payload.frames[0]["response"]["thread_reply"] = serde_json::json!(true);
    }
    payload.progress();
    if terminal {
        payload.finish(None, false);
        recovery::suppress_output(&mut payload);
    }
    let reply_ciphertext =
        if event.turn_id.is_none() && terminal && raw.contains_key("response_reply") {
            let reply: ReplyPayload =
                bson::from_bson(raw.get("response_reply").cloned().expect("checked reply"))
                    .map_err(|_| AppError::Internal("Response reply unavailable".into()))?;
            if reply.version != 1 {
                return Err(AppError::Internal("Response reply unavailable".into()));
            }
            (reply.response_id == id).then_some(reply.ciphertext)
        } else {
            None
        };
    Ok(Some(View {
        payload,
        terminal,
        admitted: mapping.thread_reply.is_some()
            && (event.turn_id.is_some() || event.status == "queued" || terminal),
        retryable: event.status == "retryable",
        reply_ciphertext,
    }))
}

pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    db.collection::<Document>(EVENTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"response_read.commands.response_id":1})
                .options(
                    IndexOptions::builder()
                        .name("response_read_identity".to_owned())
                        .partial_filter_expression(doc! {"response_read.version":1})
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(EVENTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"response_read.expires_at":1})
                .options(
                    IndexOptions::builder()
                        .name("response_read_expiry".to_owned())
                        .partial_filter_expression(doc! {"response_read.version":1})
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(EVENTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"response_read_pending.expires_at":1})
                .options(
                    IndexOptions::builder()
                        .name("response_read_pending_expiry".to_owned())
                        .partial_filter_expression(doc! {"response_read_pending.version":1})
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

pub async fn expire(db: &Database) -> AppResult<()> {
    use futures::TryStreamExt;
    for field in ["response_read", "response_read_pending"] {
        let expiry = format!("{field}.expires_at");
        let version = format!("{field}.version");
        let rows: Vec<Document> = db
            .collection::<Document>(EVENTS)
            .find(doc! {
                &version:1,&expiry:{"$lte":bson::DateTime::now()},
            })
            .projection(doc! {"_id":1})
            .limit(128)
            .await?
            .try_collect()
            .await?;
        let ids: Vec<bson::Bson> = rows
            .into_iter()
            .filter_map(|mut row| row.remove("_id"))
            .collect();
        if !ids.is_empty() {
            db.collection::<Document>(EVENTS)
                .update_many(
                    doc! {"_id":{"$in":ids},&expiry:{"$lte":bson::DateTime::now()}},
                    doc! {"$unset":{field:"","response_reply":""}},
                )
                .await?;
        }
    }
    Ok(())
}
