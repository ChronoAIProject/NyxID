//! Human decisions bound to one conversation and one credential generation.
use chrono::{DateTime, Duration, Utc};
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
        assistant_agent::GuestAccess,
        assistant_agent_credential::COLLECTION_NAME as CREDENTIALS,
        assistant_conversation::{
            AgentRole, AssistantConversation, COLLECTION_NAME as CONVERSATIONS,
        },
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    },
    mw::auth::ASSISTANT_ACCOUNT_SCOPE,
    services::{api_key_mutation_service as mutations, audit_service, key_service},
};

pub const PENDING_SECONDS: i64 = 15 * 60;
pub const ACTION_SECONDS: i64 = 10 * 60;

#[derive(Clone)]
pub struct ChatAuthority {
    pub turn_id: Option<String>,
    pub turn_stopped: bool,
    pub machine_node_ids: Vec<String>,
    pub saved_login_ids: Vec<String>,
    pub conversation_id: String,
    pub user_id: String,
    pub api_key_id: String,
    pub role: AgentRole,
    /// The agent this thread belongs to (NyxBot or a specialist).
    pub agent_id: String,
    pub agent_name: String,
    /// The thread's newest turn acts for a channel chat guest (not the
    /// owner): service calls only as far as the owner lets guests use each
    /// service (`AssistantAgent::guest_access`); see `guest_refusal`.
    pub guest: bool,
    pub confirmation_policy: Option<crate::models::trigger_schedule::ConfirmationPolicy>,
}
impl ChatAuthority {
    /// NyxBot threads run with Full access; specialists only with their grants.
    pub fn is_orchestrator(&self) -> bool {
        self.role == AgentRole::Orchestrator
    }
}
impl std::fmt::Debug for ChatAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChatAuthority { [REDACTED] }")
    }
}

/// What NyxBot is told when a guest turn calls a tool: NyxBot holds all of
/// the owner's services, so it uses none for other people.
pub fn orchestrator_guest_refusal() -> Value {
    json!({"error": "owner_only", "instructions": "You are answering someone other than the \
        owner, so you use no tools here: answer from the conversation. If people in this chat \
        should use certain services, the owner can give the chat a specialist with just those \
        services (ask NyxBot in NyxID)."})
}

/// What a guest turn (someone other than the owner) is told when it asks
/// for something only the owner can ask for.
pub fn guest_refusal() -> Value {
    json!({"error": "owner_only", "instructions": "You are answering someone other than the \
        owner. Only the owner can ask for account actions, new connections, more access, \
        anything that needs their approval, or more than the owner lets guests do with a \
        service. Help with your services otherwise, and say that only the bot's owner can \
        ask for that."})
}

/// What a guest turn is told when a service call goes beyond what the owner
/// lets guests do with that service.
pub fn guest_service_refusal(service: &str, access: GuestAccess) -> Value {
    let allowed = match access {
        GuestAccess::Read => "only look things up with",
        _ => "look things up, create and act with, but not change or delete anything in,",
    };
    json!({"error": "owner_only", "service": service, "guest_access": access.as_str(),
        "instructions": format!("You are answering someone other than the owner, who may \
        {allowed} {service}. Help within that, and say that only the bot's owner can ask for \
        more; the owner can change it by asking NyxBot.")})
}

/// What a guest turn is told when a service call carries a method override.
pub fn guest_method_override_refusal(service: &str) -> Value {
    json!({"error": "owner_only", "service": service,
        "instructions": format!("You are answering someone other than the owner: calls to \
        {service} for them use the operation's own HTTP method, never a method override \
        (an X-HTTP-Method-Override header, a _method field, or a method field naming another \
        verb), and a request body sent as JSON must be valid JSON. Call it that way, or say \
        that only the bot's owner can ask for that.")})
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
    let agent = super::assistant_team_service::agent_for_conversation(db, &conversation).await?;
    if agent.destroyed_at.is_some() {
        return Err(not_found());
    }
    let confirmation_policy = if let Some(run_id) = conversation
        .active_turn
        .as_ref()
        .and_then(|turn| turn.trigger_run_id.as_deref())
    {
        db.collection::<crate::models::trigger_run::TriggerRun>(
            crate::models::trigger_run::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": run_id, "user_id": user})
        .await?
        .and_then(|run| run.confirmation_policy)
    } else {
        None
    };
    Ok(Some(ChatAuthority {
        turn_id: conversation.active_turn.as_ref().map(|t| t.turn_id.clone()),
        turn_stopped: conversation
            .active_turn
            .as_ref()
            .is_none_or(|t| t.stop_requested),
        machine_node_ids: agent.machine_node_ids.clone(),
        saved_login_ids: agent.saved_login_ids.clone(),
        user_id: user.into(),
        api_key_id: key.into(),
        conversation_id: conversation_id.into(),
        role: if agent.is_nyxbot() {
            AgentRole::Orchestrator
        } else {
            AgentRole::Subagent
        },
        agent_id: agent.id,
        agent_name: agent.name,
        guest: conversation.guest_turn,
        confirmation_policy,
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
        trigger_run_id: None,
        created_at: now,
        decided_at: None,
        expires_at: now + Duration::seconds(PENDING_SECONDS),
        decider: if orchestrated { "orchestrator" } else { "user" }.into(),
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
                // Ordinary denials stay bound to the initiating user/orchestrator
                // message across event turns. Only trigger runs use the active turn.
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
                row.requested_turn_id = conversation.active_turn.as_ref()
                    .filter(|turn| turn.trigger_run_id.is_some())
                    .map(|turn| turn.turn_id.clone())
                    .or_else(|| started_by.as_ref().map(|message| message.turn_id.clone()));
                row.trigger_run_id = conversation.active_turn.as_ref().and_then(|turn| turn.trigger_run_id.clone());
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
            _ => format!(
                "Ask the user to confirm the action card, then retry with \
                acknowledgement_id. Never confirm it yourself. Where no card can be shown (a \
                chat app or a group chat), ask them to reply \"yes {code}\" to confirm or \
                \"no {code}\" to cancel.",
                code = confirm_code(&row.id)
            ),
        }
    };
    let mut value = json!({"error": if denied {"acknowledgement_denied"} else {"acknowledgement_required"},
        "kind": row.kind, "acknowledgement_id": row.id, "service_slug": row.service_slug,
        "service_name": row.service_name, "summary": row.summary, "decider": row.decider,
        "instructions": instructions});
    if row.kind == "action" && !denied {
        value["confirm_phrase"] = json!(format!("yes {}", confirm_code(&row.id)));
    }
    value
}

/// A short code the owner quotes to confirm one specific card by reply.
pub fn confirm_code(id: &str) -> String {
    let hex: String = id.chars().filter(char::is_ascii_hexdigit).take(6).collect();
    format!(
        "{:04}",
        u32::from_str_radix(&hex, 16).unwrap_or_default() % 10_000
    )
}

/// A plain confirmation reply: yes/no, optionally followed by a card code.
pub fn parse_reply(text: &str) -> Option<(bool, Option<String>)> {
    let normalized = text
        .trim()
        .trim_end_matches(['.', '!', '。', '！'])
        .trim()
        .to_lowercase();
    let (head, code) = match normalized.rsplit_once(char::is_whitespace) {
        Some((head, code)) if code.len() == 4 && code.bytes().all(|b| b.is_ascii_digit()) => {
            (head.trim().to_owned(), Some(code.to_owned()))
        }
        _ => (normalized.clone(), None),
    };
    let allow = match head.as_str() {
        "yes" | "y" | "yes please" | "confirm" | "confirmed" | "allow" | "approve" | "ok"
        | "okay" | "go ahead" | "do it" | "是" | "是的" | "确认" | "好" | "好的" | "可以" => {
            true
        }
        "no" | "n" | "deny" | "cancel" | "stop" | "reject" | "don't" | "do not" | "否" | "不"
        | "不要" | "取消" => false,
        _ => return None,
    };
    Some((allow, code))
}

/// Decide the owner's action card a reply answers, from a chat app or group
/// where no card can be shown. A quoted code picks that card; a plain yes/no
/// applies only when exactly one card was raised since `since` (the owner's
/// previous message), so an answer to another question never confirms a
/// stale card.
pub async fn decide_reply(
    db: &Database,
    owner: &str,
    conversation_ids: &[String],
    text: &str,
    since: Option<DateTime<Utc>>,
) -> AppResult<Option<AssistantAcknowledgement>> {
    let Some((allow, code)) = parse_reply(text) else {
        return Ok(None);
    };
    if conversation_ids.is_empty() {
        return Ok(None);
    }
    let now = Utc::now();
    let pending: Vec<AssistantAcknowledgement> = db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find(
            doc! {"user_id": owner, "conversation_id": {"$in": conversation_ids},
            "kind": "action", "status": "pending", "decider": "user",
            "expires_at": {"$gt": bson::DateTime::from_chrono(now)}},
        )
        .await?
        .try_collect()
        .await?;
    let chosen = match code {
        Some(code) => pending
            .into_iter()
            .find(|ack| confirm_code(&ack.id) == code),
        None => {
            let mut recent: Vec<AssistantAcknowledgement> = pending
                .into_iter()
                .filter(|ack| since.is_none_or(|since| ack.created_at > since))
                .collect();
            if recent.len() == 1 {
                recent.pop()
            } else {
                None
            }
        }
    };
    let Some(ack) = chosen else {
        return Ok(None);
    };
    decide(db, owner, &ack.conversation_id, &ack.id, allow)
        .await
        .map(Some)
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
        // NyxBot holds every service; other people get none of them.
        if chat.guest {
            return Ok(Some((orchestrator_guest_refusal(), None)));
        }
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
    // A guest never widens what the agent may use: no permission request.
    if chat.guest {
        return Ok(Some((guest_refusal(), None)));
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
pub enum Decider {
    /// The owner, from the card's own conversation.
    User,
    /// The owner's NyxBot deciding a specialist's request.
    Nyxbot,
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

/// Decide a card. The owner decides from the card's conversation; the owner's
/// NyxBot decides only NyxBot-routed specialist requests. A specialist grant is
/// also written to the agent's durable grants so every thread keeps it.
pub async fn decide_as(
    db: &Database,
    user: &str,
    conversation: Option<&str>,
    id: &str,
    allow: bool,
    decider: Decider,
    reason: Option<&str>,
) -> AppResult<AssistantAcknowledgement> {
    let db = db.clone();
    let user = user.to_owned();
    let conversation = conversation.map(str::to_owned);
    let id = id.to_owned();
    let by_nyxbot = decider == Decider::Nyxbot;
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
                if by_nyxbot {
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
                    turn_id: target.active_turn.as_ref().map(|t| t.turn_id.clone()),
                    turn_stopped: target.active_turn.as_ref().is_none_or(|t| t.stop_requested),
                    machine_node_ids: Vec::new(),
                    saved_login_ids: Vec::new(),
                    confirmation_policy: None,
                    user_id: user.clone(),
                    conversation_id: row.conversation_id.clone(),
                    api_key_id: row.api_key_id.clone(),
                    role: target.role,
                    agent_id: target.agent_id.clone().unwrap_or_default(),
                    agent_name: String::new(),
                    guest: target.guest_turn,
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
                if matches!(row.kind.as_str(), "machine" | "saved_login") {
                    let id = row.service_id.as_deref().ok_or_else(not_found)?;
                    if row.kind == "machine" {
                        let node = super::node_service::get_node_by_id(&db, id)
                            .await?
                            .ok_or_else(not_found)?;
                        if !node.is_active
                            || !super::org_service::resolve_owner_access(&db, &user, &node.user_id)
                                .await?
                                .can_write()
                        {
                            return Err(not_found());
                        }
                    } else {
                        super::saved_login_service::get(&db, &user, id).await?;
                    }
                    if allow && subagent {
                        let field = if row.kind == "machine" {
                            "machine_node_ids"
                        } else {
                            "saved_login_ids"
                        };
                        let mut add = doc! {};
                        add.insert(field, id);
                        let result = db
                            .collection::<bson::Document>(
                                crate::models::assistant_agent::COLLECTION_NAME,
                            )
                            .update_one(
                                doc! {
                                    "_id": target.agent_id.as_deref().ok_or_else(not_found)?,
                                    "user_id": &user,
                                    "kind": "specialist",
                                    "destroyed_at": bson::Bson::Null,
                                },
                                doc! {
                                    "$addToSet": add,
                                    "$set": { "updated_at": bson::DateTime::now() },
                                },
                            )
                            .session(&mut *session)
                            .await?;
                        if result.matched_count != 1 {
                            return Err(not_found());
                        }
                    }
                }
                if allow && subagent {
                    // A specialist's grant lives on its agent and converges on
                    // every one of its thread keys, not just the requesting one.
                    let mut grant = crate::models::assistant_agent::AgentGrants::default();
                    match (row.kind.as_str(), row.service_id.clone()) {
                        ("service", Some(service_id)) if row.platform => {
                            grant.platform_service_ids.push(service_id)
                        }
                        ("service", Some(service_id)) => grant.service_ids.push(service_id),
                        ("account", _) => grant.account_read = true,
                        _ => {}
                    }
                    if grant != Default::default() {
                        let agent_id = target.agent_id.as_deref().ok_or_else(not_found)?;
                        super::assistant_team_service::apply_grants_in_session(
                            &db,
                            &user,
                            agent_id,
                            &super::assistant_team_service::GrantChange::Add(grant),
                            &mut *session,
                        )
                        .await?;
                    }
                } else if allow && row.kind == "service" {
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
                row.decided_by = Some(if by_nyxbot { "orchestrator" } else { "user" }.into());
                row.reason = reason.clone();
                if allow && row.kind == "action" {
                    row.expires_at = now + Duration::seconds(ACTION_SECONDS);
                }
                collection
                    .replace_one(filter, &row)
                    .session(&mut *session)
                    .await?;
                if let Some(run_id) = &row.trigger_run_id {
                    // Durable wakeup in the decision transaction. Settlement
                    // reads card state and writes this same work row, preventing
                    // a simultaneous settlement from overwriting the wakeup.
                    db.collection::<bson::Document>(super::trigger_schedule::WORK)
                        .update_one(
                            doc! {"_id": run_id},
                            doc! {"$set": {
                                "at": bson::DateTime::from_chrono(now), "fence": "", "deferrals": 0,
                            }},
                        )
                        .session(&mut *session)
                        .await?;
                }
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

/// Confirm exact changing actions initiated by untrusted webhook data. Native
/// tools use their closed inventory; service callers use catalog/HTTP semantics.
pub fn webhook_confirmation_required(
    chat: &ChatAuthority,
    read_only: bool,
    destructive: bool,
) -> bool {
    use crate::models::trigger_schedule::ConfirmationPolicy;
    match chat.confirmation_policy {
        Some(ConfirmationPolicy::Changes) => !read_only,
        Some(ConfirmationPolicy::Destructive) => destructive,
        None => false,
    }
}

pub async fn webhook_action_gate(
    db: &Database,
    chat: &ChatAuthority,
    tool: &str,
    args: &Value,
    read_only: bool,
    destructive: bool,
) -> AppResult<Option<Value>> {
    if !webhook_confirmation_required(chat, read_only, destructive) {
        return Ok(None);
    }
    if let Some(id) = args["acknowledgement_id"].as_str() {
        return Ok((!consume_action(db, chat, id, tool, args).await?).then(|| json!({
            "error": "acknowledgement_invalid",
            "instructions": "This action card is missing, expired, used or does not match the call.",
        })));
    }
    let summary = if tool == "nyx__machine_exec" {
        let services = args["services"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        format!(
            "Webhook automation requests {tool} on {}; declared services: {services}. Review this action before allowing it.",
            args["machine"].as_str().unwrap_or_default()
        )
    } else {
        format!("Webhook automation requests {tool}. Review this action before allowing it.")
    };
    let card = request(
        db,
        chat,
        Request {
            kind: "action",
            service: None,
            tool: Some(tool),
            arguments: Some(args),
            summary: &summary,
            platform: false,
        },
    )
    .await?;
    Ok(Some(refusal(&card)))
}
