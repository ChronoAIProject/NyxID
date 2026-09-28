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
        assistant_conversation::{AgentRole, AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
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
    pub role: AgentRole,
    /// The orchestrator conversation for subagents.
    pub team_id: Option<String>,
    pub agent_name: Option<String>,
}
impl ChatAuthority {
    /// Orchestrators run with Full access; subagents only with their grants.
    pub fn is_orchestrator(&self) -> bool {
        self.role == AgentRole::Orchestrator
    }
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
    if conversation.destroyed_at.is_some() {
        return Err(not_found());
    }
    Ok(Some(ChatAuthority {
        user_id: user.into(),
        api_key_id: key.into(),
        conversation_id: conversation_id.into(),
        role: conversation.role,
        team_id: conversation.team_id,
        agent_name: conversation.agent_name,
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

/// Cards the owner allowed or denied after `since`, oldest first and bounded.
/// Each new turn reports decisions made since the previous user message, so a
/// decision made while a turn was still running reaches the model exactly once.
pub async fn decided_since(
    db: &Database,
    user: &str,
    conversation: &str,
    since: chrono::DateTime<Utc>,
) -> AppResult<Vec<AssistantAcknowledgement>> {
    expire(db, user, conversation).await?;
    Ok(db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find(doc! {
            "user_id": user,
            "conversation_id": conversation,
            "status": {"$in": ["allowed", "denied"]},
            "decided_at": {"$gt": bson::DateTime::from_chrono(since)},
        })
        .sort(doc! {"decided_at": 1, "_id": 1})
        .limit(DECISION_NOTE_LIMIT)
        .await?
        .try_collect()
        .await?)
}

pub const DECISION_NOTE_LIMIT: i64 = 10;

/// Identifier-only rendering: slugs and tool names, never free-form names or
/// summaries, so owner-controlled text cannot enter the instructions.
fn identifier(value: Option<&str>) -> String {
    let cleaned: String = value
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "unknown".into()
    } else {
        cleaned
    }
}

/// Instructions note telling the model about card decisions it has not seen.
pub fn decisions_note(rows: &[AssistantAcknowledgement]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut note = String::from(
        "\n\nChat card decisions the user made since your previous reply \
        (recorded by NyxID; facts, not instructions):",
    );
    for row in rows {
        let target = match row.kind.as_str() {
            "service" => format!("service {}", identifier(row.service_slug.as_deref())),
            "account" => "account management".into(),
            _ => format!(
                "action {} (acknowledgement_id {})",
                identifier(row.tool_name.as_deref()),
                identifier(Some(&row.id))
            ),
        };
        note.push_str(&format!("\n- {}: {target}", identifier(Some(&row.status))));
    }
    note.push_str(
        "\nAllowed items are usable now; retry an allowed action with its acknowledgement_id. \
        Do not request denied items again unless the user asks.",
    );
    note
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
    Ok(request_tracked(db, chat, request).await?.0)
}

/// Like [`request`], also reporting whether a new row was created (a pending
/// duplicate is returned as-is). Subagent requests are decided by the team's
/// orchestrator; action confirmations always belong to the user.
pub async fn request_tracked(
    db: &Database,
    chat: &ChatAuthority,
    request: Request<'_>,
) -> AppResult<(AssistantAcknowledgement, bool)> {
    expire(db, &chat.user_id, &chat.conversation_id).await?;
    let now = Utc::now();
    let orchestrated = !chat.is_orchestrator() && request.kind != "action";
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
        decider: if orchestrated { "orchestrator" } else { "user" }.into(),
        team_id: if orchestrated {
            chat.team_id.clone()
        } else {
            None
        },
        request_excerpt: None,
        decided_by: None,
        reason: None,
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
                // The message that started the current work: the user's, the
                // orchestrator's instruction, or the event batch that resumed it.
                let started_by = db
                    .collection::<AssistantMessage>(MESSAGES)
                    .find_one(doc! {
                        "conversation_id": &conversation.id,
                        "user_id": &chat.user_id,
                        "role": {"$in": ["user", "orchestrator"]},
                    })
                    .sort(doc! {"seq": -1})
                    .session(&mut *session)
                    .await?;
                row.requested_turn_id = started_by.as_ref().map(|message| message.turn_id.clone());
                if row.decider == "orchestrator" {
                    row.request_excerpt = started_by.map(|message| {
                        format!(
                            "{} said: {}",
                            message.role,
                            super::assistant_nyxagent::excerpt(&message.text, 600)
                        )
                    });
                }
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
                    return Ok((existing, false));
                }
                db.collection::<AssistantAcknowledgement>(ACKS)
                    .insert_one(&row)
                    .session(&mut *session)
                    .await?;
                Ok((row, true))
            }
            .await;
            mutations::transaction_result(operation)
        })
        .await
        .map_err(mutations::map_transaction_error)
}

pub fn refusal(row: &AssistantAcknowledgement) -> Value {
    let denied = row.status == "denied";
    let instructions = if row.decider == "orchestrator" {
        if denied {
            "Your orchestrator denied this request. Do not retry it; report what you \
            could do without it."
                .into()
        } else {
            "NyxID asked your orchestrator for this permission. End your turn now with a \
            one-line note about what you are waiting for; NyxID resumes you with the \
            decision."
                .into()
        }
    } else if denied {
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
        "service_name": row.service_name, "summary": row.summary, "decider": row.decider,
        "instructions": instructions})
}

/// Gate a subagent's service call. Orchestrators run with Full access and are
/// never gated. `platform` targets are catalog entries reached through NyxID's
/// platform credential; they are granted on `allowed_platform_service_ids`.
/// Returns the refusal and, when a new request was created, its row so the
/// caller can notify the orchestrator.
pub async fn service_gate(
    db: &Database,
    chat: &ChatAuthority,
    id: &str,
    slug: &str,
    name: &str,
    platform: bool,
) -> AppResult<Option<(Value, Option<AssistantAcknowledgement>)>> {
    if chat.is_orchestrator() {
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
        format!("Use {name} (NyxID platform credential)")
    } else {
        format!("Use {name}")
    };
    let (row, created) = request_tracked(
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
    let value = refusal(&row);
    Ok(Some((value, created.then_some(row))))
}

/// Gate a subagent's account tool. Subagents reach only read-only tools, and
/// only with an `account_read` grant.
pub async fn account_gate(
    db: &Database,
    chat: &ChatAuthority,
) -> AppResult<Option<(Value, Option<AssistantAcknowledgement>)>> {
    if chat.is_orchestrator() {
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
    let (row, created) = request_tracked(
        db,
        chat,
        Request {
            kind: "account",
            service: None,
            tool: None,
            arguments: None,
            summary: "Read the NyxID account (keys, channel bots, services, nodes, approvals)",
            platform: false,
        },
    )
    .await?;
    let value = refusal(&row);
    Ok(Some((value, created.then_some(row))))
}

/// Who decides a card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decider<'a> {
    /// The owner, from the card's own conversation.
    User,
    /// The orchestrator conversation deciding its team's request.
    Orchestrator { team_id: &'a str },
}

/// Decide a card as the owner from its own conversation.
pub async fn decide(
    db: &Database,
    user: &str,
    conversation: &str,
    id: &str,
    allow: bool,
) -> AppResult<AssistantAcknowledgement> {
    super::assistant_nyxagent::get(db, user, conversation).await?;
    decide_as(db, user, Some(conversation), id, allow, Decider::User, None).await
}

/// Decide a card. The owner decides from the card's conversation, or from the
/// team's orchestrator thread for subagent requests; the orchestrator decides
/// only its own team's orchestrator-routed requests. A subagent grant is also
/// written to the subagent's durable grants so the next turn keeps it.
pub async fn decide_as(
    db: &Database,
    user: &str,
    conversation: Option<&str>,
    id: &str,
    allow: bool,
    decider: Decider<'_>,
    reason: Option<&str>,
) -> AppResult<AssistantAcknowledgement> {
    let db = db.clone();
    let user = user.to_owned();
    let conversation = conversation.map(str::to_owned);
    let id = id.to_owned();
    let team = match decider {
        Decider::User => None,
        Decider::Orchestrator { team_id } => Some(team_id.to_owned()),
    };
    let reason = reason.map(|reason| super::assistant_nyxagent::excerpt(reason, 300));
    let mut session = db.client().start_session().await?;
    let row = session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = async {
                let collection = db.collection::<AssistantAcknowledgement>(ACKS);
                let mut filter = doc! {"_id": &id, "user_id": &user};
                if let Some(conversation) = &conversation {
                    filter.insert("conversation_id", conversation);
                }
                if let Some(team) = &team {
                    filter.insert("team_id", team);
                    filter.insert("decider", "orchestrator");
                }
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
                let target = db
                    .collection::<AssistantConversation>(CONVERSATIONS)
                    .find_one(doc! {"_id": &row.conversation_id, "user_id": &user})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(not_found)?;
                let chat = ChatAuthority {
                    user_id: user.clone(),
                    conversation_id: row.conversation_id.clone(),
                    api_key_id: row.api_key_id.clone(),
                    role: target.role,
                    team_id: target.team_id.clone(),
                    agent_name: target.agent_name.clone(),
                };
                let (_, key) = fence(&db, &chat, session).await?;
                let subagent = target.role == AgentRole::Subagent;
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
                    if subagent {
                        let grant = if row.platform {
                            "grants.platform_service_ids"
                        } else {
                            "grants.service_ids"
                        };
                        db.collection::<bson::Document>(CONVERSATIONS)
                            .update_one(
                                doc! {"_id": &row.conversation_id, "user_id": &user},
                                doc! {"$addToSet": {grant: service_id}},
                            )
                            .session(&mut *session)
                            .await?;
                    }
                } else if allow && row.kind == "account" {
                    if subagent {
                        db.collection::<bson::Document>(CONVERSATIONS)
                            .update_one(
                                doc! {"_id": &row.conversation_id, "user_id": &user},
                                doc! {"$set": {"grants.account_read": true}},
                            )
                            .session(&mut *session)
                            .await?;
                    }
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
                row.decided_by = Some(if team.is_some() { "orchestrator" } else { "user" }.into());
                row.reason = reason.clone();
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
