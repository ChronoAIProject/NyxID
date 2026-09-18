//! Human decisions bound to one conversation and one credential generation.
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::{
        api_key::{ApiKey, COLLECTION_NAME as KEYS},
        assistant_acknowledgement::{AssistantAcknowledgement, COLLECTION_NAME as ACKS},
        assistant_agent_credential::COLLECTION_NAME as CREDENTIALS,
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    },
    mw::auth::ASSISTANT_ACCOUNT_SCOPE,
    services::{api_key_mutation_service as mutations, audit_service, key_service},
};

pub const PENDING_SECONDS: i64 = 15 * 60;
pub const ACTION_SECONDS: i64 = 10 * 60;

#[derive(Clone)]
pub struct ChatAuthority {
    pub conversation_id: String,
    pub user_id: String,
    pub api_key_id: String,
    pub access_mode: crate::models::assistant_conversation::AccessMode,
}
impl std::fmt::Debug for ChatAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChatAuthority { [REDACTED] }")
    }
}

pub async fn for_key(
    db: &Database,
    user: &str,
    key: Option<&str>,
) -> AppResult<Option<ChatAuthority>> {
    let Some(key) = key else { return Ok(None) };
    let Some(row) = db
        .collection::<bson::Document>(CREDENTIALS)
        .find_one(doc! {"user_id": user, "api_key_id": key})
        .projection(doc! {"conversation_id": 1})
        .await?
    else {
        return Ok(None);
    };
    let conversation_id = row.get_str("conversation_id").map_err(|_| not_found())?;
    let conversation = super::assistant_nyxagent::get(db, user, conversation_id).await?;
    Ok(Some(ChatAuthority {
        user_id: user.into(),
        api_key_id: key.into(),
        conversation_id: conversation_id.into(),
        access_mode: conversation.access_mode,
    }))
}

fn not_found() -> AppError {
    AppError::NotFound("Acknowledgement not found".into())
}

pub fn arguments_digest(arguments: &Value) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let sorted: std::collections::BTreeMap<_, _> = map
                    .iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect();
                Value::Object(sorted.into_iter().collect())
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            _ => value.clone(),
        }
    }
    let mut value = arguments.clone();
    if let Some(map) = value.as_object_mut() {
        map.remove("acknowledgement_id");
    }
    hex::encode(Sha256::digest(canonical(&value).to_string().as_bytes()))
}

async fn fence(
    db: &Database,
    chat: &ChatAuthority,
    session: &mut ClientSession,
) -> AppResult<(AssistantConversation, ApiKey)> {
    let credential = db
        .collection::<bson::Document>(CREDENTIALS)
        .update_one(
            doc! {"user_id": &chat.user_id, "conversation_id": &chat.conversation_id,
            "api_key_id": &chat.api_key_id},
            doc! {"$inc": {"authority_fence": 1}},
        )
        .session(&mut *session)
        .await?;
    if credential.matched_count != 1 {
        return Err(not_found());
    }
    let row = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one_and_update(
            doc! {"_id": &chat.conversation_id, "user_id": &chat.user_id},
            doc! {"$inc": {"authority_fence": 1}},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(not_found)?;
    let key = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id": &chat.api_key_id, "user_id": &chat.user_id, "is_active": true})
        .session(&mut *session)
        .await?
        .ok_or_else(not_found)?;
    if key.expires_at.is_some_and(|expiry| expiry <= Utc::now()) {
        return Err(not_found());
    }
    Ok((row, key))
}

pub async fn expire(db: &Database, user: &str, conversation: &str) -> AppResult<()> {
    db.collection::<AssistantAcknowledgement>(ACKS)
        .update_many(
            doc! {"user_id": user, "conversation_id": conversation,
            "expires_at": {"$lte": bson::DateTime::now()}, "$or": [
                {"status": "pending"}, {"status": "allowed", "kind": "action"},
            ]},
            doc! {"$set": {"status": "expired"}},
        )
        .await?;
    Ok(())
}

pub async fn history(
    db: &Database,
    user: &str,
    conversation: &str,
) -> AppResult<Vec<AssistantAcknowledgement>> {
    super::assistant_nyxagent::get(db, user, conversation).await?;
    expire(db, user, conversation).await?;
    let collection = db.collection::<AssistantAcknowledgement>(ACKS);
    let mut rows: Vec<_> = collection
        .find(doc! {
            "user_id": user, "conversation_id": conversation, "status": "pending",
        })
        .await?
        .try_collect()
        .await?;
    let decided: Vec<_> = collection
        .find(doc! {
            "user_id": user, "conversation_id": conversation, "status": {"$ne": "pending"},
        })
        .sort(doc! {"created_at": -1, "_id": -1})
        .limit(20)
        .await?
        .try_collect()
        .await?;
    rows.extend(decided);
    rows.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    Ok(rows)
}

pub async fn pending_counts(
    db: &Database,
    user: &str,
    ids: &[String],
) -> AppResult<std::collections::HashMap<String, u32>> {
    let mut cursor = db
        .collection::<bson::Document>(ACKS)
        .aggregate([
            doc! {"$match": {"user_id": user, "conversation_id": {"$in": ids},
            "status": "pending", "expires_at": {"$gt": bson::DateTime::now()}}},
            doc! {"$group": {"_id": "$conversation_id", "count": {"$sum": 1}}},
        ])
        .await?;
    let mut counts = std::collections::HashMap::new();
    while let Some(row) = cursor.try_next().await? {
        if let (Ok(id), Ok(count)) = (row.get_str("_id"), row.get_i32("count")) {
            counts.insert(id.into(), count as u32);
        }
    }
    Ok(counts)
}

pub struct Request<'a> {
    pub kind: &'a str,
    pub service: Option<(&'a str, &'a str, &'a str)>,
    pub tool: Option<&'a str>,
    pub arguments: Option<&'a Value>,
    pub summary: &'a str,
    /// `service` requests only: the target is a platform-provided catalog entry.
    pub platform: bool,
}
impl std::fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AcknowledgementRequest { [REDACTED] }")
    }
}

pub async fn request(
    db: &Database,
    chat: &ChatAuthority,
    request: Request<'_>,
) -> AppResult<AssistantAcknowledgement> {
    expire(db, &chat.user_id, &chat.conversation_id).await?;
    let now = Utc::now();
    let candidate = AssistantAcknowledgement {
        id: Uuid::new_v4().to_string(),
        conversation_id: chat.conversation_id.clone(),
        user_id: chat.user_id.clone(),
        api_key_id: chat.api_key_id.clone(),
        kind: request.kind.into(),
        service_id: request.service.map(|s| s.0.into()),
        service_slug: request.service.map(|s| s.1.into()),
        service_name: request.service.map(|s| s.2.into()),
        platform: request.platform,
        tool_name: request.tool.map(str::to_owned),
        arguments_digest: request.arguments.map(arguments_digest),
        summary: request.summary.into(),
        status: "pending".into(),
        requested_turn_id: None,
        created_at: now,
        decided_at: None,
        expires_at: now + Duration::seconds(PENDING_SECONDS),
    };
    let db = db.clone();
    let chat = chat.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = async {
                let (conversation, _) = fence(&db, &chat, session).await?;
                let mut row = candidate.clone();
                row.requested_turn_id = db
                    .collection::<AssistantMessage>(MESSAGES)
                    .find_one(doc! {
                        "conversation_id": &conversation.id,
                        "user_id": &chat.user_id,
                        "role": "user",
                    })
                    .sort(doc! {"seq": -1})
                    .session(&mut *session)
                    .await?
                    .map(|message| message.turn_id);
                let filter = doc! {"conversation_id": &chat.conversation_id,
                "user_id": &chat.user_id, "api_key_id": &chat.api_key_id, "kind": &row.kind,
                "service_id": &row.service_id, "tool_name": &row.tool_name,
                "arguments_digest": &row.arguments_digest,
                "$or": [{"status": "pending", "expires_at": {"$gt": bson::DateTime::from_chrono(now)}},
                    {"status": "denied", "requested_turn_id": &row.requested_turn_id}]};
                // A valid allowed action can be reused only by explicitly presenting its id.
                if let Some(existing) = db
                    .collection::<AssistantAcknowledgement>(ACKS)
                    .find_one(filter)
                    .sort(doc! {"created_at": -1})
                    .session(&mut *session)
                    .await?
                {
                    return Ok(existing);
                }
                db.collection::<AssistantAcknowledgement>(ACKS)
                    .insert_one(&row)
                    .session(&mut *session)
                    .await?;
                Ok(row)
            }
            .await;
            mutations::transaction_result(operation)
        })
        .await
        .map_err(mutations::map_transaction_error)
}

pub fn refusal(row: &AssistantAcknowledgement) -> Value {
    let denied = row.status == "denied";
    let instructions = if denied {
        "The user denied this request. Do not retry or request another \
                card unless the user explicitly asks again in a later message."
            .into()
    } else {
        match row.kind.as_str() {
            "service" => format!(
                "Ask the user to approve access to {} for this chat (a card is \
                shown in the chat), then retry.",
                row.service_name.as_deref().unwrap_or("this service")
            ),
            "account" => "Ask the user to approve account management for this chat (a card \
                is shown in the chat), then retry."
                .into(),
            _ => "Ask the user to confirm the action card, then retry with \
                acknowledgement_id. Never confirm it yourself."
                .into(),
        }
    };
    json!({"error": if denied {"acknowledgement_denied"} else {"acknowledgement_required"},
        "kind": row.kind, "acknowledgement_id": row.id, "service_slug": row.service_slug,
        "service_name": row.service_name, "summary": row.summary, "instructions": instructions})
}

/// Gate a service call in Ask mode. `platform` targets are catalog entries the
/// user reaches through NyxID's platform credential rather than a connection of
/// their own; they are granted on the key's `allowed_platform_service_ids`.
pub async fn service_gate(
    db: &Database,
    chat: &ChatAuthority,
    id: &str,
    slug: &str,
    name: &str,
    platform: bool,
) -> AppResult<Option<Value>> {
    if chat.access_mode == crate::models::assistant_conversation::AccessMode::Full {
        return Ok(None);
    }
    let key = key_service::get_api_key(db, &chat.user_id, &chat.api_key_id).await?;
    let granted = if platform {
        key.allowed_platform_service_ids
            .iter()
            .any(|allowed| allowed == id)
    } else {
        key_service::effective_allowed_service_ids(db, &key)
            .await?
            .iter()
            .any(|allowed| allowed == id)
    };
    if granted {
        return Ok(None);
    }
    let summary = if platform {
        format!("Allow this chat to use {name} (NyxID platform credential)?")
    } else {
        format!("Allow this chat to use {name}?")
    };
    let row = request(
        db,
        chat,
        Request {
            kind: "service",
            service: Some((id, slug, name)),
            tool: None,
            arguments: None,
            summary: &summary,
            platform,
        },
    )
    .await?;
    Ok(Some(refusal(&row)))
}

pub async fn account_gate(db: &Database, chat: &ChatAuthority) -> AppResult<Option<Value>> {
    if chat.access_mode == crate::models::assistant_conversation::AccessMode::Full {
        return Ok(None);
    }
    let key = key_service::get_api_key(db, &chat.user_id, &chat.api_key_id).await?;
    if key
        .scopes
        .split_whitespace()
        .any(|scope| scope == ASSISTANT_ACCOUNT_SCOPE)
    {
        return Ok(None);
    }
    let row = request(
        db,
        chat,
        Request {
            kind: "account",
            service: None,
            tool: None,
            arguments: None,
            summary: "Allow this chat to manage your NyxID account (keys, channel bots, \
                services, nodes, approval settings)?",
            platform: false,
        },
    )
    .await?;
    Ok(Some(refusal(&row)))
}

pub async fn decide(
    db: &Database,
    user: &str,
    conversation: &str,
    id: &str,
    allow: bool,
) -> AppResult<AssistantAcknowledgement> {
    super::assistant_nyxagent::get(db, user, conversation).await?;
    expire(db, user, conversation).await?;
    let db = db.clone();
    let user = user.to_owned();
    let conversation = conversation.to_owned();
    let id = id.to_owned();
    let mut session = db.client().start_session().await?;
    let row = session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = async {
                let collection = db.collection::<AssistantAcknowledgement>(ACKS);
                let filter = doc! {"_id": &id, "user_id": &user, "conversation_id": &conversation};
                let mut row = collection
                    .find_one(filter.clone())
                    .session(&mut *session)
                    .await?
                    .ok_or_else(not_found)?;
                if row.status != "pending" || row.expires_at <= Utc::now() {
                    return Err(AppError::Conflict(
                        "Acknowledgement is no longer pending".into(),
                    ));
                }
                let chat = ChatAuthority {
                    user_id: user.clone(),
                    conversation_id: conversation.clone(),
                    api_key_id: row.api_key_id.clone(),
                    access_mode: Default::default(),
                };
                let (_, key) = fence(&db, &chat, session).await?;
                let now = Utc::now();
                if row.kind == "service" {
                    let service_id = row.service_id.as_deref().ok_or_else(not_found)?;
                    if row.platform {
                        // A platform grant names an active catalog entry. Visibility
                        // through platform grants is re-checked on every execution,
                        // so a stale entry on the key can never execute by itself.
                        db.collection::<bson::Document>(
                            crate::models::downstream_service::COLLECTION_NAME,
                        )
                        .find_one(doc! {"_id": service_id, "is_active": true})
                        .session(&mut *session)
                        .await?
                        .ok_or_else(not_found)?;
                    } else {
                        // Only owner-visible UserService rows can receive chat grants.
                        // Reject inaccessible rows before any decision.
                        super::api_key_scope_service::validate_service_ids(
                            &db,
                            &user,
                            &[service_id.into()],
                            super::api_key_scope_service::ScopeAuthorization::for_actor(Some(
                                &user,
                            )),
                        )
                        .await
                        .map_err(|error| match error {
                            AppError::ValidationError(_) => not_found(),
                            error => error,
                        })?;
                    }
                }
                if allow && row.kind == "service" {
                    let service_id = row.service_id.as_deref().ok_or_else(not_found)?;
                    let field = if row.platform {
                        "allowed_platform_service_ids"
                    } else {
                        "allowed_service_ids"
                    };
                    mutations::update_one(
                        &db,
                        doc! {"_id": &key.id, "user_id": &user},
                        doc! {"$addToSet": {field: service_id}},
                        Some(&mut *session),
                    )
                    .await?;
                } else if allow && row.kind == "account" {
                    let mut scopes: Vec<_> = key.scopes.split_whitespace().collect();
                    if !scopes.contains(&ASSISTANT_ACCOUNT_SCOPE) {
                        scopes.push(ASSISTANT_ACCOUNT_SCOPE);
                    }
                    mutations::update_one(
                        &db,
                        doc! {"_id": &key.id, "user_id": &user},
                        doc! {"$set": {"scopes": scopes.join(" ")}},
                        Some(&mut *session),
                    )
                    .await?;
                }
                row.status = if allow { "allowed" } else { "denied" }.into();
                row.decided_at = Some(now);
                if allow && row.kind == "action" {
                    row.expires_at = now + Duration::seconds(ACTION_SECONDS);
                }
                collection
                    .replace_one(filter, &row)
                    .session(&mut *session)
                    .await?;
                Ok(row)
            }
            .await;
            mutations::transaction_result(operation)
        })
        .await
        .map_err(mutations::map_transaction_error)?;
    Ok(row)
}

/// Atomically consume a digest-bound action. Failures never fall through to execution.
pub async fn consume_action(
    db: &Database,
    chat: &ChatAuthority,
    id: &str,
    tool: &str,
    arguments: &Value,
) -> AppResult<bool> {
    let db = db.clone();
    let chat = chat.clone();
    let id = id.to_owned();
    let tool = tool.to_owned();
    let digest = arguments_digest(arguments);
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let operation = async {
            fence(&db, &chat, session).await?;
            let result = db.collection::<AssistantAcknowledgement>(ACKS).update_one(
                doc! {"_id": &id, "user_id": &chat.user_id, "conversation_id": &chat.conversation_id,
                    "api_key_id": &chat.api_key_id, "kind": "action", "tool_name": &tool,
                    "arguments_digest": &digest,
                    "status": "allowed",
                    "expires_at": {"$gt": bson::DateTime::now()},
                },
                doc! {"$set": {"status": "used"}},
            ).session(&mut *session).await?;
            Ok(result.modified_count == 1)
        }.await;
        mutations::transaction_result(operation)
    }).await.map_err(mutations::map_transaction_error)
}

pub async fn audit_decision(
    db: &Database,
    actor: &audit_service::AuditActor,
    row: &AssistantAcknowledgement,
) {
    let _ = audit_service::log_actor_event(
        db.clone(),
        actor,
        "assistant_acknowledgement_decided",
        Some(json!({
            "conversation_id": row.conversation_id, "kind": row.kind,
            "service_id": row.service_id, "tool_name": row.tool_name,
            "decision": if row.status == "allowed" {"allow"} else {"deny"},
        })),
    )
    .await;
}
