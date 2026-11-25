//! Metadata-only lifecycle of relay events whose answer remains locally delivered.
use crate::{
    errors::{AppError, AppResult},
    models::{
        nyxbot_channel::{
            COLLECTION_NAME as CHANNELS, EVENTS_COLLECTION_NAME as EVENTS, NyxbotChannel,
            NyxbotEvent,
        },
        nyxbot_response_read::{Command, NoticeKind, OrgNotice, ReplyPayload, ResponseRead},
    },
    services::nyxbot_response_recovery::{self as recovery, Payload},
};
use bson::{Document, doc};
use chrono::{Duration, Utc};
use futures::future::BoxFuture;
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

pub(crate) fn notice_target_digest(
    facts: &crate::models::channel_thread::ChannelThreadFacts,
) -> String {
    let identity = serde_json::json!([
        facts.version,
        facts.kind,
        facts.chat_id,
        facts.parent_chat_id,
        facts.message_id,
        facts.root_id,
        facts.native_thread_id,
        facts.parent_message_id,
        facts.sender_kind,
        facts.address,
        facts.mentions_others,
    ]);
    crate::crypto::token::hash_token(&identity.to_string())
}

/// Recognize only a terminal no-turn event whose original provider command
/// and source still agree with its server-written notice. Turn reads always
/// ignore this additive field, including stale data left on older documents.
pub(crate) fn org_notice(raw: &Document, event: &NyxbotEvent) -> AppResult<Option<OrgNotice>> {
    if event.turn_id.is_some() || !raw.contains_key("org_notice") {
        return Ok(None);
    }
    let rejected = || AppError::Conflict("thread_target_unavailable".into());
    let notice: OrgNotice = bson::from_bson(raw["org_notice"].clone()).map_err(|_| rejected())?;
    let mapping = raw.get_document("response_read").map_err(|_| rejected())?;
    let original = mapping
        .get_array("commands")
        .ok()
        .and_then(|commands| commands.first())
        .and_then(|command| command.as_document())
        .and_then(|command| command.get_str("response_id").ok());
    let delivery = event.delivery.as_ref().ok_or_else(rejected)?;
    let origin = &delivery.origin;
    let thread = origin.thread.as_deref().ok_or_else(rejected)?;
    let execution = origin.org_execution.as_deref().ok_or_else(rejected)?;
    if notice.version != 1
        || notice.expires_at <= Utc::now()
        || event.expires_at <= Utc::now()
        || event.status != "refused"
        || event.conversation_id.is_some()
        || event.custom_binding_id.is_some()
        || raw.get_str("gateway_response_id").is_ok()
        || event.relay_message_id.as_deref() != Some(&event.event_id)
        || origin.nyxbot_channel_id != event.channel_id
        || delivery.transport != "gateway"
        || delivery.version != 1
        || !matches!(origin.platform.as_str(), "lark" | "feishu")
        || thread.source_message_id != event.event_id
        || event.resolved_thread_id.as_deref() != Some(&thread.child_id)
        || thread.sender_id != delivery.sender_id
        || thread.guest != delivery.guest
        || execution.guest != thread.guest
        || execution.version != 1
        || execution.agent_id != thread.agent_id
        || mapping.get_i32("version") != Ok(1)
        || mapping.get_bool("thread_reply") != Ok(true)
        || mapping
            .get_datetime("expires_at")
            .ok()
            .map(|t| t.to_chrono())
            != Some(notice.expires_at)
        || mapping.get_str("binding_id").ok() != Some(&notice.binding_id)
        || notice.binding_id.is_empty()
        || original != Some(&notice.response_id)
    {
        return Err(rejected());
    }
    if raw.contains_key("response_reply") {
        let payload = raw.get_document("response_reply").map_err(|_| rejected())?;
        if payload.get_str("response_id").ok() != original || payload.get_i32("version") != Ok(1) {
            return Err(rejected());
        }
    }
    Ok(Some(notice))
}

pub(crate) struct SelectedThread {
    pub binding: crate::models::channel_thread_follow::ThreadTurnBinding,
    pub binding_id: String,
    pub partition: String,
    pub target_sha256: String,
}

/// Only the original, prepared event can terminalize as a notice. An early
/// refusal with no delivery keeps its ordinary response and no notice marker.
pub(crate) fn record_reply<'a>(
    db: &'a Database,
    row: &'a NyxbotChannel,
    event: &'a str,
    id: &'a str,
    notice: Option<(NoticeKind, &'a SelectedThread, &'a str)>,
    fields: Document,
) -> BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let mut filter = doc! {
            "_id":event,"channel_id":&row.id,"user_id":&row.user_id,
            "response_read.version":1,"response_read.expires_at":{"$gt":bson::DateTime::now()},
            "turn_id":bson::Bson::Null,"response_reply":bson::Bson::Null,
            "status":{"$in":["running","resuming"]},
        };
        let Some((kind, selected, text)) = notice else {
            let update = doc! {"$set": fields, "$unset": {"event_context_ciphertext": ""}};
            let changed = db
                .collection::<Document>(EVENTS)
                .update_one(filter, update)
                .await?;
            if changed.matched_count != 1 {
                return Err(AppError::Conflict(
                    "Response reply admission changed".into(),
                ));
            }
            return Ok(());
        };
        let binding = &selected.binding;
        let org = row
            .org_specialist
            .as_ref()
            .ok_or_else(|| AppError::Conflict("Response origin changed".into()))?;
        filter.extend(doc! {
            "event_id":&binding.source_message_id, "relay_message_id":&binding.source_message_id,
            "resolved_thread_id":&binding.child_id, "conversation_id":bson::Bson::Null,
            "response_read.binding_id":&selected.binding_id, "response_read.thread_reply":true,
            "response_read.commands.0.response_id":id, "org_notice":bson::Bson::Null,
            "$or":[{"delivery":bson::Bson::Null},{
                "delivery.version":1, "delivery.transport":"gateway",
                "delivery.origin.nyxbot_channel_id":&row.id,"delivery.origin.platform":&row.platform,
                "delivery.origin.partition":&selected.partition,
                "delivery.origin.thread":bson::to_bson(binding).map_err(|_| AppError::Internal("Thread binding unavailable".into()))?,
                "delivery.origin.org_execution.version":1, "delivery.origin.org_execution.org_id":&org.org_id,
                "delivery.origin.org_execution.revision":&org.revision, "delivery.origin.org_execution.agent_id":&binding.agent_id,
                "delivery.origin.org_execution.sender_hash":crate::services::org_channel_service::sender_identity(row,&binding.sender_id),
                "delivery.origin.org_execution.guest":binding.guest,"delivery.sender_id":&binding.sender_id,"delivery.guest":binding.guest,
                "delivery.source_route_id":&row.route_id,"delivery.source_route_api_key_id":&row.route_api_key_id,
            }],
        });
        let mut set = fields
            .into_iter()
            .map(|(k, v)| (k, bson::Bson::Document(doc! {"$literal":v})))
            .collect::<bson::Document>();
        set.insert("org_notice",doc!{"$cond":[{"$ne":[{"$ifNull":["$delivery",bson::Bson::Null]},bson::Bson::Null]},{
            "version":1,"kind":{"$literal":bson::to_bson(&kind).map_err(|_| AppError::Internal("Notice kind unavailable".into()))?},
            "binding_id":{"$literal":&selected.binding_id}, "response_id":{"$literal":id},
            "text_sha256":crate::crypto::token::hash_token(text),
            "target_sha256":{"$literal":&selected.target_sha256},"bot_id":{"$literal":&row.channel_bot_id},
            "pending":true,"expires_at":"$response_read.expires_at",
        },"$$REMOVE"]});
        let ingress = doc! {
            "_id":&binding.source_message_id, "direction":"inbound",
            "channel_bot_id":&row.channel_bot_id,"user_id":&org.org_id,
            "agent_api_key_id":&row.route_api_key_id,"conversation_id":&row.route_id,
            "platform":&row.platform,"sender_platform_id":&binding.sender_id,
            "nyxbot_admission.resolved_thread_id":&binding.child_id,"nyxbot_admission.turn_id":null,
        };
        let context = (db.clone(), filter, set, ingress);
        let wrote_marker = crate::services::transaction_service::run_db_only(
            db,
            &context,
            |tx, (db, filter, set, ingress)| {
                Box::pin(async move {
                    let event = db
                        .collection::<bson::Document>(EVENTS)
                        .find_one_and_update(
                            filter.clone(),
                            vec![
                                doc! {"$set":set.clone()},
                                doc! {"$unset":"event_context_ciphertext"},
                            ],
                        )
                        .session(&mut *tx)
                        .return_document(mongodb::options::ReturnDocument::After)
                        .projection(doc! {"org_notice.version":1})
                        .await?
                        .ok_or_else(|| {
                            AppError::Conflict("Response reply admission changed".into())
                        })?;
                    if !event.contains_key("org_notice") {
                        return Ok(false);
                    }
                    // Retained ingress must still require this exact notice if its
                    // shorter-lived response mapping or whole event is later removed.
                    let marked = db
                        .collection::<bson::Document>(
                            crate::models::channel_message::COLLECTION_NAME,
                        )
                        .update_one(
                            ingress.clone(),
                            doc! {"$set":{"nyxbot_admission.requires_org_notice":true}},
                        )
                        .session(tx)
                        .await?;
                    if marked.matched_count != 1 {
                        return Err(AppError::Conflict("Notice source changed".into()));
                    }
                    Ok(true)
                })
            },
        )
        .await?;
        if wrote_marker
            && db
                .collection::<bson::Document>(CHANNELS)
                .find_one(doc! {
                    "_id":&row.id,"channel_bot_id":&row.channel_bot_id,"status":"active",
                    "binding_id":&selected.binding_id, "org_specialist.revision":&org.revision,
                    "route_id":&row.route_id,"route_api_key_id":&row.route_api_key_id,
                })
                .projection(doc! {"_id":1})
                .await?
                .is_none()
        {
            crate::services::org_thread_notice::record_refusal(
                db,
                &row.id,
                &binding.source_message_id,
            )
            .await;
        }
        Ok(())
    })
}

pub struct View {
    pub payload: Payload,
    pub terminal: bool,
    pub admitted: bool,
    pub retryable: bool,
    pub reply: Option<(String, ReplyPayload)>,
    pub notice_digest: Option<String>,
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
    load_mode(db, channel, id, Transport::Live).await
}

/// A provider credential authenticated against the original channel but its
/// transport has since closed. Only a mapped fixed notice may complete empty.
pub(crate) async fn closed(
    db: &Database,
    channel: &NyxbotChannel,
    id: &str,
    request_hash: Option<&str>,
) -> AppResult<Option<View>> {
    load_mode(db, channel, id, Transport::Closed { request_hash }).await
}

enum Transport<'a> {
    Live,
    Closed { request_hash: Option<&'a str> },
}
fn load_mode<'a>(
    db: &'a Database,
    channel: &'a NyxbotChannel,
    id: &'a str,
    transport: Transport<'a>,
) -> BoxFuture<'a, AppResult<Option<View>>> {
    Box::pin(async move {
        if channel.transport != "gateway"
            || channel.status != "active"
            || channel.gateway_source.is_some()
        {
            return Ok(None);
        }
        let mut filter = doc! {
            "channel_id":&channel.id,"user_id":&channel.user_id,"custom_binding_id":bson::Bson::Null,
            "response_read.version":1,"response_read.commands.response_id":id,
            "response_read.binding_id":&channel.binding_id,
            "response_read.expires_at":{"$gt":bson::DateTime::now()},"expires_at":{"$gt":bson::DateTime::now()},
            "gateway_response_id":bson::Bson::Null,
        };
        if let Transport::Closed { request_hash } = transport {
            filter.insert("org_notice.version", 1);
            if let Some(hash) = request_hash {
                filter.insert(
                    "response_read.commands",
                    doc! {"$elemMatch":{"response_id":id,"request_hash":hash}},
                );
            }
        }
        let Some(raw) = db
            .collection::<Document>(EVENTS)
            .find_one(filter)
            .projection(doc! {"event_context_ciphertext":0,"media":0})
            .await?
        else {
            return Ok(None);
        };
        view_mode(
            db,
            channel,
            &raw,
            id,
            matches!(transport, Transport::Closed { .. }),
        )
        .await
    })
}

async fn view_mode(
    db: &Database,
    channel: &NyxbotChannel,
    raw: &Document,
    id: &str,
    closed: bool,
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
    let notice = org_notice(raw, &event)?;
    if closed && notice.is_none() {
        return Ok(None);
    }
    let suppress = if let Some(notice) = &notice {
        let suppress = closed || !notice_transport_live(db, channel, &event, notice).await?;
        if suppress {
            super::org_thread_notice::record_refusal(db, &channel.id, &event.event_id).await;
        }
        suppress
    } else {
        false
    };
    if notice.is_none()
        && let Some(origin) = event.delivery.as_ref().map(|d| &d.origin)
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
        if event.turn_id.is_some() || event.status == "queued" {
            payload.snapshot["thread_reply"] = serde_json::json!(true);
        }
    }
    payload.progress();
    if terminal {
        if suppress
            && notice.as_ref().is_some_and(|n| n.response_id == id)
            && raw.contains_key("response_reply")
        {
            // A caller can lose transport after acknowledging output-item 2.
            // Keep completion at cursor 3 without exposing that item again.
            payload.frames.push(serde_json::json!({
                "type": "response.in_progress",
                "sequence_number": payload.frames.len(),
                "response": payload.snapshot,
            }));
        }
        payload.finish(None, false);
        recovery::suppress_output(&mut payload);
    }
    let selected_reply =
        if !suppress && event.turn_id.is_none() && terminal && raw.contains_key("response_reply") {
            let reply: ReplyPayload =
                bson::from_bson(raw.get("response_reply").cloned().expect("checked reply"))
                    .map_err(|_| AppError::Internal("Response reply unavailable".into()))?;
            if reply.version != 1 {
                return Err(AppError::Internal("Response reply unavailable".into()));
            }
            (reply.response_id == id).then_some((event.id, reply))
        } else {
            None
        };
    Ok(Some(View {
        payload,
        terminal,
        admitted: mapping.thread_reply.is_some()
            && (event.turn_id.is_some() || event.status == "queued" || terminal),
        retryable: event.status == "retryable",
        reply: selected_reply,
        notice_digest: notice.map(|n| n.text_sha256),
    }))
}

async fn notice_transport_live(
    db: &Database,
    channel: &NyxbotChannel,
    event: &NyxbotEvent,
    notice: &OrgNotice,
) -> AppResult<bool> {
    let Some(delivery) = &event.delivery else {
        return Ok(false);
    };
    let Some(org) = &channel.org_specialist else {
        return Ok(false);
    };
    let Some(actor) = &delivery.origin.org_execution else {
        return Ok(false);
    };
    if org.org_id != actor.org_id
        || org.revision != actor.revision
        || channel.bot_owner_id.as_deref() != Some(&org.org_id)
        || channel.channel_bot_id != notice.bot_id
        || !super::channel_thread_service::gateway::negotiated(channel)
        || delivery.source_route_id.is_none()
        || delivery.source_route_id != channel.route_id
        || delivery.source_route_api_key_id.as_ref() != Some(&channel.route_api_key_id)
        || delivery.origin.platform != channel.platform
    {
        return Ok(false);
    }
    for (collection, filter) in [
        (
            crate::models::channel_bot::COLLECTION_NAME,
            doc! {"_id":&notice.bot_id,"user_id":&org.org_id,"is_active":true,"status":"active"},
        ),
        (
            crate::models::channel_conversation::COLLECTION_NAME,
            doc! {"_id":&channel.route_id,"user_id":&org.org_id,"channel_bot_id":&notice.bot_id,
            "agent_api_key_id":&channel.route_api_key_id,"is_active":true,"retired_by_transfer":{"$ne":true}},
        ),
        (
            crate::models::api_key::COLLECTION_NAME,
            doc! {"_id":&channel.route_api_key_id,"user_id":&org.org_id,"is_active":true,
            "$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}]},
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id":1})
            .await?
            .is_none()
        {
            return Ok(false);
        }
    }
    Ok(true)
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
    for (name, keys, partial) in [
        (
            "org_notice_pending",
            doc! {"channel_id":1,"org_notice.expires_at":1},
            doc! {"org_notice.pending":true},
        ),
        (
            "org_notice_source",
            doc! {"org_notice.bot_id":1,"event_id":1},
            doc! {"org_notice.bot_id":{"$exists":true}},
        ),
    ] {
        db.collection::<Document>(EVENTS)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(
                        IndexOptions::builder()
                            .name(name.to_owned())
                            .partial_filter_expression(partial)
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
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
                    doc! {"$unset":{field:"","response_reply":"","org_notice":""}},
                )
                .await?;
        }
    }
    Ok(())
}
