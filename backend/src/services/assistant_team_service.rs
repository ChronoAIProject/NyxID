//! NyxBot agents: the owner's single personal agent (NyxBot, their chief of
//! staff) and persistent specialist agents created by the owner or by NyxBot.
//!
//! An agent is a durable identity with its own grants, memory and threads.
//! Every thread is an ordinary NyxAgent conversation with its own key: NyxBot
//! threads run with Full access, specialist threads carry exactly their
//! agent's grants and ask NyxBot for anything else. Starting turns and waking
//! agents needs the HTTP proxy and lives in `handlers::assistant_team`; this
//! module owns the durable state.
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        assistant_acknowledgement::{AssistantAcknowledgement, COLLECTION_NAME as ACKS},
        assistant_agent::{
            AgentGrants, AgentKind, AssistantAgent, COLLECTION_NAME as AGENTS, GuestAccess,
            MAX_DISPLAY_NAME_CHARS, MAX_MEMORY_NOTE_CHARS, MAX_MEMORY_NOTES, MAX_PERSONA_CHARS,
            MemoryNote,
        },
        assistant_conversation::{
            AccessMode, AgentEvent, AgentRole, AssistantConversation,
            COLLECTION_NAME as CONVERSATIONS,
        },
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    },
    services::{
        api_key_mutation_service as transactions,
        assistant_agent_credential_service::{self as credentials, KeyAuthority},
        assistant_nyxagent::{self as engine, excerpt, identifier, live_turn},
        assistant_profile_routing::{self as routing, RouteRole},
        assistant_settings_service, audit_service, key_service, mcp_service,
        node_ws_manager::NodeWsManager,
    },
};

pub const NYXBOT_NAME: &str = "NyxBot";
pub const MAX_NAME_CHARS: usize = 32;
pub const MAX_DESCRIPTION_CHARS: usize = 2048;
pub const MAX_GRANT_TARGETS: usize = 32;
/// Loop guards for server-started event turns.
pub const EVENT_TURNS_PER_HOUR: u64 = 20;
pub const MAX_EVENT_STREAK: i32 = 3;
/// Bounded like `nyx__wait_for_connection`, below NyxAgent's 150 s call timeout.
pub const MAX_WAIT_SECS: u64 = 120;
pub const READ_LIMIT: i64 = 20;
pub const REPLY_EXCERPT_CHARS: usize = 2000;
/// Instructions carry at most this much of an agent's memory.
pub const MEMORY_NOTE_BUDGET: usize = 6000;

/// Names that would read as someone else in a transcript or mention.
const RESERVED_NAMES: &[&str] = &["user", "nyxbot", "nyxid", "owner", "system"];

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !RESERVED_NAMES.contains(&name)
        && name.len() <= MAX_NAME_CHARS
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && name.as_bytes()[0].is_ascii_alphanumeric()
}

/// What the agent's other threads are answering right now (title, question
/// excerpt, question key), so it does not do the same work twice. Bounded.
pub async fn in_progress(
    db: &Database,
    owner: &str,
    agent_id: &str,
    exclude: &str,
) -> AppResult<Vec<(String, Option<String>, Option<String>)>> {
    let fresh = bson::DateTime::from_chrono(
        Utc::now() - chrono::Duration::seconds(super::assistant_nyxagent::ACTIVE_TURN_TTL_SECS),
    );
    let rows: Vec<bson::Document> = db
        .collection::<bson::Document>(CONVERSATIONS)
        .find(
            doc! {"user_id": owner, "agent_id": agent_id, "_id": {"$ne": exclude},
            "group_id": bson::Bson::Null, "active_turn.started_at": {"$gt": fresh}},
        )
        .projection(doc! {"title": 1, "active_turn.question": 1, "active_turn.question_key": 1})
        .limit(5)
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .iter()
        .map(|row| {
            let turn = row.get_document("active_turn").ok();
            (
                row.get_str("title").unwrap_or_default().to_owned(),
                turn.and_then(|turn| turn.get_str("question").ok())
                    .map(str::to_owned),
                turn.and_then(|turn| turn.get_str("question_key").ok())
                    .map(str::to_owned),
            )
        })
        .collect())
}

pub fn event(kind: &str, text: String, agent_id: Option<&str>) -> AgentEvent {
    AgentEvent {
        id: Uuid::new_v4().to_string(),
        kind: kind.into(),
        text,
        agent_id: agent_id.map(str::to_owned),
        question_key: None,
        reply_to: Vec::new(),
        created_at: Utc::now(),
    }
}

fn not_found() -> AppError {
    AppError::NotFound("Agent not found".into())
}

pub fn is_duplicate(error: &mongodb::error::Error) -> bool {
    matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(
        mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000)
}

// ---------------------------------------------------------------------------
// Agents
// ---------------------------------------------------------------------------

/// The owner's NyxBot, created on first use. A unique index keeps it single.
pub async fn ensure_nyxbot(db: &Database, owner: &str) -> AppResult<AssistantAgent> {
    let collection = db.collection::<AssistantAgent>(AGENTS);
    if let Some(agent) = collection
        .find_one(doc! {"user_id": owner, "kind": "nyxbot"})
        .await?
    {
        return Ok(agent);
    }
    if db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": owner, "user_type": "org"})
        .await?
        .is_some()
    {
        return Err(AppError::Forbidden(
            "Organizations cannot own NyxBot".into(),
        ));
    }
    let now = Utc::now();
    let agent = AssistantAgent {
        skills: Vec::new(),
        skills_revision: 0,
        skill_metadata: BTreeMap::new(),
        machine_node_ids: Vec::new(),
        saved_login_ids: Vec::new(),
        id: Uuid::new_v4().to_string(),
        user_id: owner.into(),
        kind: AgentKind::Nyxbot,
        name: NYXBOT_NAME.into(),
        description: String::new(),
        specialty: None,
        grants: AgentGrants::default(),
        guest_access: BTreeMap::new(),
        operation_scopes: Default::default(),
        operation_scope_revisions: Default::default(),
        created_by: "user".into(),
        model: routing::model_for(db, RouteRole::Orchestrator, engine::DEFAULT_MODEL).await,
        home_conversation_id: None,
        memory: Vec::new(),
        display_name: None,
        persona: None,
        destroyed_at: None,
        created_at: now,
        updated_at: now,
    };
    match collection.insert_one(&agent).await {
        Ok(_) => Ok(agent),
        // A concurrent first use created it.
        Err(error) if is_duplicate(&error) => collection
            .find_one(doc! {"user_id": owner, "kind": "nyxbot"})
            .await?
            .ok_or_else(not_found),
        Err(error) => Err(error.into()),
    }
}

/// An agent of the owner by ID.
pub async fn agent(db: &Database, owner: &str, id: &str) -> AppResult<AssistantAgent> {
    if Uuid::parse_str(id).is_err() {
        return Err(not_found());
    }
    let agent = db
        .collection::<AssistantAgent>(AGENTS)
        .find_one(doc! {"_id": id})
        .await?
        .ok_or_else(not_found)?;
    if !super::org_agent_service::access(db, owner, &agent.user_id)
        .await?
        .can_read()
        || (owner != agent.user_id && agent.is_nyxbot())
    {
        return Err(not_found());
    }
    Ok(agent)
}

pub async fn maintained_agent(db: &Database, actor: &str, id: &str) -> AppResult<AssistantAgent> {
    let agent = agent(db, actor, id).await?;
    super::org_agent_service::require_maintain(db, actor, &agent).await?;
    Ok(agent)
}

/// A specialist by ID or name; a live agent wins over a destroyed namesake.
pub async fn specialist(db: &Database, owner: &str, name_or_id: &str) -> AppResult<AssistantAgent> {
    if Uuid::parse_str(name_or_id).is_ok() {
        let agent = agent(db, owner, name_or_id).await?;
        return if agent.is_nyxbot() {
            Err(not_found())
        } else {
            Ok(agent)
        };
    }
    let filter = if valid_name(name_or_id) {
        doc! {"user_id": owner, "kind": "specialist", "name": name_or_id}
    } else {
        return Err(not_found());
    };
    db.collection::<AssistantAgent>(AGENTS)
        .find_one(filter)
        .sort(doc! {"destroyed_at": 1, "created_at": -1})
        .await?
        .ok_or_else(not_found)
}

/// A live specialist by ID or name.
pub async fn live_specialist(
    db: &Database,
    owner: &str,
    name_or_id: &str,
) -> AppResult<AssistantAgent> {
    let agent = specialist(db, owner, name_or_id).await?;
    if agent.destroyed_at.is_some() {
        return Err(AppError::Conflict("That agent was destroyed".into()));
    }
    Ok(agent)
}

/// The owner's agents: NyxBot first, then specialists oldest first.
pub async fn agents(
    db: &Database,
    owner: &str,
    include_destroyed: bool,
) -> AppResult<Vec<AssistantAgent>> {
    let owners = super::org_agent_service::visible_owners(db, owner).await?;
    let mut filter =
        doc! {"user_id": {"$in": owners}, "$or": [{"user_id": owner}, {"kind": "specialist"}]};
    if !include_destroyed {
        filter.insert("destroyed_at", bson::Bson::Null);
    }
    let mut rows: Vec<AssistantAgent> = db
        .collection::<AssistantAgent>(AGENTS)
        .find(filter)
        .sort(doc! {"created_at": 1})
        .limit(200)
        .await?
        .try_collect()
        .await?;
    rows.sort_by_key(|agent| !agent.is_nyxbot());
    Ok(rows)
}

/// The agent a thread belongs to; legacy rows belong to the owner's NyxBot.
pub async fn agent_for_conversation(
    db: &Database,
    row: &AssistantConversation,
) -> AppResult<AssistantAgent> {
    match row.agent_id.as_deref() {
        Some(id) => {
            let agent = agent(db, &row.user_id, id).await?;
            super::org_agent_service::require_use(db, &row.user_id, &agent).await?;
            if agent.user_id != row.user_id && (row.guest_turn || row.channel.is_some()) {
                return Err(AppError::Forbidden(
                    "Organization agents require a private member thread".into(),
                ));
            }
            Ok(agent)
        }
        None => ensure_nyxbot(db, &row.user_id).await,
    }
}

/// An agent's own threads. Hidden group member threads are the group's, not
/// the agent's, and never listed as threads.
pub fn thread_filter_for(actor: &str, agent: &AssistantAgent) -> bson::Document {
    if agent.is_nyxbot() {
        doc! {"user_id": actor, "group_id": bson::Bson::Null, "$or": [
            {"agent_id": &agent.id}, {"agent_id": bson::Bson::Null},
        ]}
    } else {
        doc! {"user_id": actor, "agent_id": &agent.id, "group_id": bson::Bson::Null}
    }
}

/// Threads of an agent, newest first. Legacy rows count as NyxBot threads.
#[cfg(test)]
pub async fn threads(
    db: &Database,
    agent: &AssistantAgent,
    limit: i64,
) -> AppResult<Vec<AssistantConversation>> {
    threads_for(db, &agent.user_id, agent, limit).await
}

pub async fn threads_for(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
    limit: i64,
) -> AppResult<Vec<AssistantConversation>> {
    super::org_agent_service::require_use(db, actor, agent).await?;
    Ok(db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(thread_filter_for(actor, agent))
        .sort(doc! {"updated_at": -1})
        .limit(limit)
        .await?
        .try_collect()
        .await?)
}

/// Create a thread row (no turn yet) with its key and credential, and make
/// it the agent's home when it has none.
pub(crate) async fn create_thread_for(
    db: &Database,
    keys: &EncryptionKeys,
    actor: &str,
    agent: &AssistantAgent,
    title: &str,
    session: &mut ClientSession,
) -> AppResult<AssistantConversation> {
    create_thread_for_with_access(db, keys, actor, agent, title, session, None).await
}

pub(crate) async fn create_thread_for_with_access(
    db: &Database,
    keys: &EncryptionKeys,
    actor: &str,
    agent: &AssistantAgent,
    title: &str,
    session: &mut ClientSession,
    snapshot: Option<&std::sync::Arc<super::org_agent_service::RequestAccess>>,
) -> AppResult<AssistantConversation> {
    if let Some(access) = snapshot {
        if !access.matches(actor, &agent.user_id) {
            return Err(super::org_group_service::missing());
        }
    } else {
        super::org_agent_service::require_use(db, actor, agent).await?;
    }
    let learning_epoch = Box::pin(super::assistant_agent_learning::enrollment_epoch(
        db, actor, agent,
    ))
    .await?;
    Box::pin(create_thread_with_kind(
        db,
        keys,
        actor,
        agent,
        title,
        false,
        session,
        snapshot,
        learning_epoch,
    ))
    .await
}

pub(crate) async fn create_automation_thread(
    db: &Database,
    keys: &EncryptionKeys,
    actor: &str,
    agent: &AssistantAgent,
    title: &str,
    session: &mut ClientSession,
) -> AppResult<AssistantConversation> {
    create_thread_with_kind(db, keys, actor, agent, title, true, session, None, None).await
}

#[allow(clippy::too_many_arguments)]
async fn create_thread_with_kind(
    db: &Database,
    keys: &EncryptionKeys,
    actor: &str,
    agent: &AssistantAgent,
    title: &str,
    automation_thread: bool,
    session: &mut ClientSession,
    snapshot: Option<&std::sync::Arc<super::org_agent_service::RequestAccess>>,
    learning_epoch: Option<i64>,
) -> AppResult<AssistantConversation> {
    let now = Utc::now();
    let mut row = AssistantConversation {
        machine_previews: false,
        id: format!("nyxa-{}", Uuid::new_v4().simple()),
        user_id: actor.to_owned(),
        title: super::assistant_title_service::provisional(title),
        title_source: if automation_thread {
            crate::models::assistant_conversation::TitleSource::User
        } else {
            crate::models::assistant_conversation::TitleSource::Provisional
        },
        model: agent.model.clone(),
        access_mode: AccessMode::Full,
        nyxagent_session_id: None,
        nyxagent_last_response_id: None,
        credential_api_key_id: String::new(),
        message_count: 0,
        active_turn: None,
        context_reset_at: None,
        context_reset_reason: None,
        created_at: now,
        updated_at: now,
        role: if agent.is_nyxbot() {
            AgentRole::Orchestrator
        } else {
            AgentRole::Subagent
        },
        agent_id: Some(agent.id.clone()),
        automation_thread,
        agent_owner_id: (actor != agent.user_id).then(|| agent.user_id.clone()),
        learning_epoch,
        report_to: None,
        pending_events: Vec::new(),
        event_streak: 0,
        channel: None,
        group_id: None,
        group_request_id: None,
        group_seen_seq: 0,
        guest_turn: false,
        reply_channel: None,
        deliver_also: Vec::new(),
    };
    let collection = db.collection::<AssistantConversation>(CONVERSATIONS);
    collection.insert_one(&row).session(&mut *session).await?;
    let authority = Box::pin(credentials::authority_with_access(
        db, &row, session, snapshot,
    ))
    .await?;
    let credential = credentials::load_or_provision_in_session(
        db,
        keys,
        actor,
        &row.id,
        &authority,
        &mut *session,
    )
    .await?;
    row.credential_api_key_id = credential.api_key_id.clone();
    collection
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"credential_api_key_id": &row.credential_api_key_id}},
        )
        .session(&mut *session)
        .await?;
    if !automation_thread && actor == agent.user_id {
        db.collection::<AssistantAgent>(AGENTS)
            .update_one(
                doc! {"_id": &agent.id, "home_conversation_id": bson::Bson::Null},
                doc! {"$set": {"home_conversation_id": &row.id}},
            )
            .session(&mut *session)
            .await?;
    }
    Ok(row)
}

/// The agent's home thread, where NyxID delivers work and events that no
/// particular thread asked for. Recreated if the owner deleted it.
pub async fn home_thread(
    db: &Database,
    keys: &std::sync::Arc<EncryptionKeys>,
    agent: &AssistantAgent,
) -> AppResult<AssistantConversation> {
    ensure_nyxbot(db, &agent.user_id).await?;
    // Permission requests reach this through nested MCP dispatch. The member
    // thread provisioning transaction must not enlarge each caller's future.
    Box::pin(home_thread_for(db, keys, &agent.user_id, agent)).await
}

pub async fn home_thread_for(
    db: &Database,
    keys: &std::sync::Arc<EncryptionKeys>,
    actor: &str,
    agent: &AssistantAgent,
) -> AppResult<AssistantConversation> {
    super::org_agent_service::require_use(db, actor, agent).await?;
    // An agent's home is one of its own threads, never a chat app channel
    // thread (a group's, or someone else's private chat), nor an isolated
    // automation thread whose untrusted context must stay separate.
    if let Some(id) = agent.home_conversation_id.as_deref()
        && let Some(row) = db
            .collection::<AssistantConversation>(CONVERSATIONS)
            .find_one(doc! {"_id": id, "user_id": actor,
            "channel": bson::Bson::Null, "automation_thread": {"$ne": true}})
            .await?
    {
        return Ok(row);
    }
    let mut own = thread_filter_for(actor, agent);
    own.insert("channel", bson::Bson::Null);
    own.insert("automation_thread", doc! {"$ne": true});
    let newest = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(own)
        .sort(if actor == agent.user_id {
            doc! {"updated_at": -1}
        } else {
            doc! {"created_at": 1, "_id": 1}
        })
        .await?;
    if let Some(row) = newest {
        if actor != agent.user_id {
            return Ok(row);
        }
        db.collection::<AssistantAgent>(AGENTS)
            .update_one(
                doc! {"_id": &agent.id},
                doc! {"$set": {"home_conversation_id": &row.id}},
            )
            .await?;
        return Ok(row);
    }
    // The pointer referenced a deleted thread (or none exists): clear it so
    // the new thread becomes home inside the transaction.
    if actor == agent.user_id {
        db.collection::<AssistantAgent>(AGENTS)
            .update_one(
                doc! {"_id": &agent.id},
                doc! {"$set": {"home_conversation_id": bson::Bson::Null}},
            )
            .await?;
    }
    let mut session = db.client().start_session().await?;
    let learning_epoch = Box::pin(super::assistant_agent_learning::enrollment_epoch(
        db, actor, agent,
    ))
    .await?;
    let db_owned = db.clone();
    let keys = keys.clone();
    let agent = agent.clone();
    let actor = actor.to_owned();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = create_thread_with_kind(
                &db_owned,
                &keys,
                &actor,
                &agent,
                &agent.name,
                false,
                session,
                None,
                learning_epoch,
            )
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

// ---------------------------------------------------------------------------
// Grants
// ---------------------------------------------------------------------------

/// Services resolved from slugs or IDs exactly as MCP enforces them: user
/// services by `UserService` ID, platform services by catalog ID.
#[derive(Clone, Debug, Default)]
pub struct GrantTargets {
    pub service_ids: Vec<String>,
    pub platform_service_ids: Vec<String>,
    pub slugs: Vec<String>,
    /// The service ID each requested name or ID resolved to.
    pub ids_by_request: HashMap<String, String>,
}

pub async fn resolve_targets(
    db: &Database,
    node_manager: &NodeWsManager,
    owner: &str,
    targets: &[String],
) -> AppResult<GrantTargets> {
    let (resolved, refused) = resolve_each_target(db, node_manager, owner, targets).await?;
    match refused.into_iter().next() {
        Some(refused) => Err(AppError::ValidationError(refused.reason)),
        None => Ok(resolved),
    }
}

/// A requested service that cannot be granted, and why (for the agent).
#[derive(Clone, Debug, Serialize)]
pub struct Refused {
    pub service: String,
    pub reason: String,
}

/// Resolve each target on its own: what resolves is granted, and each
/// refusal says why, so one unusable service does not block the others.
pub async fn resolve_each_target(
    db: &Database,
    node_manager: &NodeWsManager,
    owner: &str,
    targets: &[String],
) -> AppResult<(GrantTargets, Vec<Refused>)> {
    if targets.len() > MAX_GRANT_TARGETS {
        return Err(AppError::ValidationError(format!(
            "At most {MAX_GRANT_TARGETS} services can be granted at once"
        )));
    }
    let mut resolved = GrantTargets::default();
    let mut refused = Vec::new();
    if targets.is_empty() {
        return Ok((resolved, refused));
    }
    let catalog = mcp_service::load_operation_catalog(
        db,
        node_manager,
        owner,
        mcp_service::NodeScope::Unrestricted,
        mcp_service::ServiceScope::Unrestricted,
    )
    .await?;
    for target in targets {
        let target = target.trim();
        let refuse = |reason: String| Refused {
            service: identifier(target),
            reason,
        };
        // Every agent runs on NyxAgent and thinks with NyxID's model
        // services; the engine itself is not a service an agent calls.
        if target.eq_ignore_ascii_case(crate::services::assistant_nyxagent::SERVICE_SLUG)
            || target.eq_ignore_ascii_case("nyxagent")
        {
            refused.push(refuse(
                "NyxAgent is the assistant engine every agent already runs on; it is not \
                granted as a service"
                    .into(),
            ));
            continue;
        }
        let Some(service) = catalog
            .services
            .iter()
            .find(|service| service.service_slug == target || service.service_id == target)
        else {
            refused.push(refuse(format!(
                "Unknown or unavailable service: {}. Use a slug or ID from \
                nyx__list_connected_services",
                identifier(target)
            )));
            continue;
        };
        let (list, id) = match &service.source {
            mcp_service::McpToolSource::Internal => {
                refused.push(refuse(
                    "Grant account access with account_read instead of the nyxid service".into(),
                ));
                continue;
            }
            mcp_service::McpToolSource::Platform { .. } => (
                &mut resolved.platform_service_ids,
                service.service_id.clone(),
            ),
            mcp_service::McpToolSource::UserManaged { .. } => {
                (&mut resolved.service_ids, service.service_id.clone())
            }
        };
        resolved
            .ids_by_request
            .insert(target.to_string(), id.clone());
        if !list.contains(&id) {
            list.push(id);
            resolved.slugs.push(service.service_slug.clone());
        }
    }
    Ok((resolved, refused))
}

// ---------------------------------------------------------------------------
// Specialists
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CreateRequest {
    pub machines: Option<Vec<String>>,
    pub logins: Option<Vec<String>>,
    pub name: String,
    pub description: String,
    /// Optional friendly name and persona (tone, personality).
    pub display_name: Option<String>,
    pub persona: Option<String>,
    pub targets: GrantTargets,
    pub account_read: bool,
    pub specialty: Option<String>,
    /// `user` or `nyxbot`.
    pub created_by: &'static str,
}
impl std::fmt::Debug for CreateRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateRequest")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Typed refusal the tools render for the model.
#[derive(Debug)]
pub enum TeamRefusal {
    LimitReached { limit: i32 },
    NameTaken,
}

fn validate_profile(name: &str, description: &str) -> AppResult<()> {
    if !valid_name(name) {
        return Err(AppError::ValidationError(
            "Agent names use 1-32 lowercase letters, digits, or hyphens".into(),
        ));
    }
    if description.trim().is_empty() || description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(AppError::ValidationError(format!(
            "A description must contain 1 to {MAX_DESCRIPTION_CHARS} characters"
        )));
    }
    Ok(())
}

/// Create a persistent specialist with its home thread, key and encrypted
/// credential in one transaction. Fencing the owner's NyxBot serializes
/// concurrent creation so the owner's live-agent limit holds.
#[cfg(test)]
pub async fn create_specialist(
    db: &Database,
    keys: &std::sync::Arc<EncryptionKeys>,
    owner: &str,
    request: CreateRequest,
) -> AppResult<Result<(AssistantAgent, AssistantConversation), TeamRefusal>> {
    create_specialist_for(db, keys, owner, owner, request).await
}

pub async fn create_specialist_for(
    db: &Database,
    keys: &std::sync::Arc<EncryptionKeys>,
    actor: &str,
    owner: &str,
    request: CreateRequest,
) -> AppResult<Result<(AssistantAgent, AssistantConversation), TeamRefusal>> {
    if actor != owner {
        super::org_agent_service::require_creation_enabled(db, actor).await?;
    }
    let description = request.description.trim().to_owned();
    validate_profile(&request.name, &description)?;
    if request
        .specialty
        .as_deref()
        .is_some_and(|value| !routing::valid_specialty(value))
    {
        return Err(AppError::ValidationError(
            "specialty uses up to 32 lowercase letters, digits, hyphens or underscores".into(),
        ));
    }
    // Resolve machine/login grants before creating any agent, thread or key.
    let machine_change = super::machine_service::resolve_grant_change(
        db,
        owner,
        request.machines.clone(),
        request.logins.clone(),
        GrantChange::Add(AgentGrants::default()),
        MachineGrantMode::Add,
    )
    .await?;
    let (machine_node_ids, saved_login_ids) = match machine_change {
        GrantChange::Machine {
            machines, logins, ..
        } => (machines.unwrap_or_default(), logins.unwrap_or_default()),
        _ => (Vec::new(), Vec::new()),
    };
    let nyxbot = ensure_nyxbot(db, actor).await?;
    let limit = assistant_settings_service::get(db, owner)
        .await?
        .max_live_subagents;
    let model = routing::model_for(
        db,
        RouteRole::Subagent {
            specialty: request.specialty.as_deref(),
        },
        &nyxbot.model,
    )
    .await;
    let now = Utc::now();
    let agent = AssistantAgent {
        skills: Vec::new(),
        skills_revision: 0,
        skill_metadata: BTreeMap::new(),
        machine_node_ids,
        saved_login_ids,
        id: Uuid::new_v4().to_string(),
        user_id: owner.into(),
        kind: AgentKind::Specialist,
        name: request.name.clone(),
        description,
        specialty: request.specialty.clone(),
        grants: AgentGrants {
            service_ids: request.targets.service_ids.clone(),
            platform_service_ids: request.targets.platform_service_ids.clone(),
            account_read: request.account_read,
        },
        guest_access: BTreeMap::new(),
        operation_scopes: Default::default(),
        operation_scope_revisions: Default::default(),
        created_by: request.created_by.into(),
        model,
        home_conversation_id: None,
        memory: Vec::new(),
        display_name: style_value(
            request.display_name.as_deref().unwrap_or_default(),
            MAX_DISPLAY_NAME_CHARS,
            "A display name",
            false,
        )?,
        persona: style_value(
            request.persona.as_deref().unwrap_or_default(),
            MAX_PERSONA_CHARS,
            "A persona",
            true,
        )?,
        destroyed_at: None,
        created_at: now,
        updated_at: now,
    };
    Box::pin(super::org_agent_service::validate_grants(db, actor, &agent)).await?;
    let actor_owned = actor.to_owned();
    let mut session = db.client().start_session().await?;
    let db_owned = db.clone();
    let keys_owned = keys.clone();
    let nyxbot_id = nyxbot.id.clone();
    let outcome = session
        .start_transaction()
        .and_run2(async move |session| {
            let db = &db_owned;
            let agent = agent.clone();
            let operation: AppResult<_> = async {
                let collection = db.collection::<AssistantAgent>(AGENTS);
                super::org_agent_service::require_maintain(db, &actor_owned, &agent).await?;
                if actor_owned != agent.user_id {
                    db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
                        .update_one(
                            doc! {"_id": &agent.user_id, "is_active": true},
                            doc! {"$inc": {"agent_team_fence": 1}},
                        )
                        .session(&mut *session)
                        .await?;
                }
                // Serialize personal team changes on the person's NyxBot.
                collection
                    .update_one(
                        doc! {"_id": &nyxbot_id, "user_id": &agent.user_id},
                        doc! {"$inc": {"team_fence": 1}},
                    )
                    .session(&mut *session)
                    .await?;
                let live = collection
                    .count_documents(doc! {"user_id": &agent.user_id, "kind": "specialist",
                    "destroyed_at": bson::Bson::Null})
                    .session(&mut *session)
                    .await?;
                if live >= limit.max(0) as u64 {
                    return Ok(Err(TeamRefusal::LimitReached { limit }));
                }
                if collection
                    .find_one(doc! {"user_id": &agent.user_id, "kind": "specialist",
                    "name": &agent.name, "destroyed_at": bson::Bson::Null})
                    .session(&mut *session)
                    .await?
                    .is_some()
                {
                    return Ok(Err(TeamRefusal::NameTaken));
                }
                collection.insert_one(&agent).session(&mut *session).await?;
                let home = create_thread_with_kind(
                    db,
                    &keys_owned,
                    &actor_owned,
                    &agent,
                    &agent.name,
                    false,
                    session,
                    None,
                    None,
                )
                .await?;
                let mut agent = agent;
                if actor_owned == agent.user_id {
                    agent.home_conversation_id = Some(home.id.clone());
                }
                Ok(Ok((agent, home)))
            }
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    if let Ok((agent, _)) = &outcome {
        audit(
            db,
            actor,
            "assistant_agent_created",
            serde_json::json!({
                "owner_id": &agent.user_id, "agent_id": &agent.id, "created_by": &agent.created_by,
                "service_ids": &agent.grants.service_ids,
                "platform_service_ids": &agent.grants.platform_service_ids,
                "account_read": agent.grants.account_read,
            }),
        )
        .await;
    }
    Ok(outcome)
}

/// Rename or re-describe an agent. NyxBot keeps its fixed name.
/// Optional personalization for `update_agent`; `Some("")` clears a field.
#[derive(Clone, Copy, Debug, Default)]
pub struct AgentStyle<'a> {
    pub display_name: Option<&'a str>,
    pub persona: Option<&'a str>,
}

/// A bounded, credential-free display name or persona; empty clears it.
fn style_value(value: &str, max: usize, what: &str, multiline: bool) -> AppResult<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > max
        || value
            .chars()
            .any(|c| c.is_control() && !(multiline && c == '\n'))
    {
        return Err(AppError::ValidationError(format!(
            "{what} must contain at most {max} characters"
        )));
    }
    if looks_secret(value) {
        return Err(AppError::ValidationError(format!(
            "{what} must not contain credentials"
        )));
    }
    // The persona is quoted in the prompt between triple quotes.
    Ok(Some(value.replace("\"\"\"", "\"")))
}

pub async fn update_agent(
    db: &Database,
    owner: &str,
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
    style: AgentStyle<'_>,
) -> AppResult<AssistantAgent> {
    let current = maintained_agent(db, owner, id).await?;
    let actor = owner;
    let owner = current.user_id.as_str();
    if current.destroyed_at.is_some() {
        return Err(AppError::Conflict("That agent was destroyed".into()));
    }
    let mut set = doc! {"updated_at": bson::DateTime::now()};
    if let Some(name) = name {
        if current.is_nyxbot() {
            return Err(AppError::ValidationError("NyxBot's name is fixed".into()));
        }
        validate_profile(name, description.unwrap_or(&current.description))?;
        if name != current.name
            && db
                .collection::<AssistantAgent>(AGENTS)
                .find_one(doc! {"user_id": owner, "kind": "specialist", "name": name,
                "destroyed_at": bson::Bson::Null})
                .await?
                .is_some()
        {
            return Err(AppError::Conflict(
                "A live agent already uses that name".into(),
            ));
        }
        set.insert("name", name);
    }
    if let Some(description) = description {
        let description = description.trim();
        if description.chars().count() > MAX_DESCRIPTION_CHARS
            || (!current.is_nyxbot() && description.is_empty())
        {
            return Err(AppError::ValidationError(format!(
                "A description must contain at most {MAX_DESCRIPTION_CHARS} characters"
            )));
        }
        set.insert("description", description);
    }
    let mut unset = doc! {};
    for (field, value, max, what, multiline) in [
        (
            "display_name",
            style.display_name,
            MAX_DISPLAY_NAME_CHARS,
            "A display name",
            false,
        ),
        (
            "persona",
            style.persona,
            MAX_PERSONA_CHARS,
            "A persona",
            true,
        ),
    ] {
        match value.map(|value| style_value(value, max, what, multiline)) {
            Some(Ok(Some(value))) => {
                set.insert(field, value);
            }
            Some(Ok(None)) => {
                unset.insert(field, "");
            }
            Some(Err(error)) => return Err(error),
            None => {}
        }
    }
    let mut update = doc! {"$set": set};
    if !unset.is_empty() {
        update.insert("$unset", unset);
    }
    let updated = db
        .collection::<AssistantAgent>(AGENTS)
        .find_one_and_update(doc! {"_id": id, "user_id": owner}, update)
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(not_found)?;
    super::org_agent_service::audit_change(db, actor, &updated, "profile").await;
    Ok(updated)
}

/// A change to a specialist's grants. Merged inside the transaction against
/// the grants it reads, so concurrent changes never undo each other. Guest
/// access levels given with a change are laid over the current ones; every
/// change keeps levels only for granted services, and none for the default.
#[derive(Clone, Debug)]
pub enum GrantChange {
    Machine {
        base: Box<GrantChange>,
        machines: Option<Vec<String>>,
        logins: Option<Vec<String>>,
        mode: MachineGrantMode,
    },
    /// The owner's full replacement of services and account access, with
    /// the guest access levels it names (others are kept).
    Replace {
        grants: AgentGrants,
        guests: BTreeMap<String, GuestAccess>,
    },
    /// Add these targets, and account access when `account_read` is set.
    Add(AgentGrants),
    /// Remove these targets, and account access when `account_read` is set.
    Remove(AgentGrants),
    /// Set what guests may do with these granted services.
    Guests(BTreeMap<String, GuestAccess>),
}

#[derive(Clone, Copy, Debug)]
pub enum MachineGrantMode {
    Add,
    Remove,
    Replace,
}

impl GrantChange {
    /// The grants and guest access levels after this change.
    pub fn apply(
        &self,
        current: &AgentGrants,
        current_guests: &BTreeMap<String, GuestAccess>,
    ) -> (AgentGrants, BTreeMap<String, GuestAccess>) {
        if let Self::Machine { base, .. } = self {
            return base.apply(current, current_guests);
        }
        let mut grants = current.clone();
        let mut guests = current_guests.clone();
        match self {
            Self::Replace {
                grants: replacement,
                guests: levels,
            } => {
                grants = replacement.clone();
                guests.extend(levels.clone());
            }
            Self::Add(add) => {
                for (list, ids) in [
                    (&mut grants.service_ids, &add.service_ids),
                    (&mut grants.platform_service_ids, &add.platform_service_ids),
                ] {
                    for id in ids {
                        if !list.contains(id) {
                            list.push(id.clone());
                        }
                    }
                }
                grants.account_read |= add.account_read;
            }
            Self::Remove(remove) => {
                grants
                    .service_ids
                    .retain(|id| !remove.service_ids.contains(id));
                grants
                    .platform_service_ids
                    .retain(|id| !remove.platform_service_ids.contains(id));
                grants.account_read &= !remove.account_read;
            }
            Self::Guests(levels) => guests.extend(levels.clone()),
            Self::Machine { .. } => unreachable!("handled above"),
        }
        // A service granted anew starts at the default level, whatever an
        // earlier grant of it left behind (a writer that predates levels
        // may have revoked it without dropping its level).
        let before: HashSet<&String> = current
            .service_ids
            .iter()
            .chain(&current.platform_service_ids)
            .collect();
        let named: HashSet<&String> = match self {
            Self::Replace { guests: levels, .. } | Self::Guests(levels) => levels.keys().collect(),
            Self::Add(_) | Self::Remove(_) => HashSet::new(),
            Self::Machine { .. } => unreachable!("handled above"),
        };
        guests.retain(|id, _| before.contains(id) || named.contains(id));
        let granted: HashSet<&String> = grants
            .service_ids
            .iter()
            .chain(&grants.platform_service_ids)
            .collect();
        guests.retain(|id, level| granted.contains(id) && *level != GuestAccess::Use);
        (grants, guests)
    }
}

/// Every key a set of threads may hold: the credential row's key, which is
/// authoritative after rotation or an in-turn replacement, and the key the
/// thread recorded at its last turn start.
pub(crate) async fn thread_key_ids(
    db: &Database,
    owner: &str,
    rows: &[AssistantConversation],
    session: &mut ClientSession,
) -> AppResult<Vec<String>> {
    let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    let mut cursor = db
        .collection::<bson::Document>(crate::models::assistant_agent_credential::COLLECTION_NAME)
        .find(doc! {"user_id": owner, "conversation_id": {"$in": &ids}})
        .projection(doc! {"api_key_id": 1})
        .session(&mut *session)
        .await?;
    let credentials: Vec<bson::Document> = cursor.stream(&mut *session).try_collect().await?;
    let mut keys: Vec<String> = credentials
        .iter()
        .filter_map(|row| row.get_str("api_key_id").ok().map(str::to_owned))
        .collect();
    for row in rows {
        if !row.credential_api_key_id.is_empty() && !keys.contains(&row.credential_api_key_id) {
            keys.push(row.credential_api_key_id.clone());
        }
    }
    Ok(keys)
}

/// Synchronize all private member threads, retaining each key's person owner.
pub(crate) async fn sync_thread_authority(
    db: &Database,
    agent: &AssistantAgent,
    rows: &[AssistantConversation],
    session: &mut ClientSession,
) -> AppResult<()> {
    let authority = KeyAuthority::for_agent(agent);
    for row in rows {
        for key in thread_key_ids(db, &row.user_id, std::slice::from_ref(row), session).await? {
            match credentials::apply_authority(db, &row.user_id, &key, &authority, session).await {
                Ok(()) | Err(AppError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

/// Apply a grant change to a live specialist inside the caller's transaction.
/// The agent row and every thread key converge together; requests for
/// targets the change removes expire.
pub async fn apply_grants_in_session(
    db: &Database,
    owner: &str,
    agent_id: &str,
    change: &GrantChange,
    session: &mut ClientSession,
) -> AppResult<AssistantAgent> {
    let current = maintained_agent(db, owner, agent_id).await?;
    let collection = db.collection::<AssistantAgent>(AGENTS);
    let filter = doc! {"_id": agent_id, "user_id": &current.user_id, "kind": "specialist",
    "destroyed_at": bson::Bson::Null};
    let mut agent = collection
        .find_one(filter.clone())
        .session(&mut *session)
        .await?
        .ok_or_else(not_found)?;
    let previous_machines = agent.machine_node_ids.clone();
    let previous_logins = agent.saved_login_ids.clone();
    if let GrantChange::Machine {
        machines,
        logins,
        mode,
        ..
    } = change
    {
        for (current, requested) in [
            (&mut agent.machine_node_ids, machines),
            (&mut agent.saved_login_ids, logins),
        ] {
            if let Some(ids) = requested {
                match mode {
                    MachineGrantMode::Replace => *current = ids.clone(),
                    MachineGrantMode::Remove => current.retain(|id| !ids.contains(id)),
                    MachineGrantMode::Add => {
                        for id in ids {
                            if !current.contains(id) {
                                current.push(id.clone());
                            }
                        }
                    }
                }
                if current.len() > 64 {
                    return Err(AppError::ValidationError(
                        "At most 64 machine or login grants are allowed".into(),
                    ));
                }
            }
        }
    }
    let (grants, guest_access) = change.apply(&agent.grants, &agent.guest_access);
    let removed: Vec<String> = agent
        .grants
        .service_ids
        .iter()
        .chain(&agent.grants.platform_service_ids)
        .filter(|id| !grants.service_ids.contains(id) && !grants.platform_service_ids.contains(id))
        .cloned()
        .collect();
    let lost_account = agent.grants.account_read && !grants.account_read;
    agent.grants = grants;
    agent.guest_access = guest_access;
    Box::pin(super::org_agent_service::validate_grants(db, owner, &agent)).await?;
    let encode = |value: bson::ser::Result<bson::Bson>| {
        value.map_err(|_| AppError::Internal("Grant encoding failed".into()))
    };
    let mut set = doc! {"grants": encode(bson::to_bson(&agent.grants))?,
    "guest_access": encode(bson::to_bson(&agent.guest_access))?, "updated_at": bson::DateTime::now()};
    if matches!(change, GrantChange::Machine { .. }) {
        set.insert(
            "machine_node_ids",
            bson::to_bson(&agent.machine_node_ids)
                .map_err(|_| AppError::Internal("Machine grant encoding failed".into()))?,
        );
        set.insert(
            "saved_login_ids",
            bson::to_bson(&agent.saved_login_ids)
                .map_err(|_| AppError::Internal("Login grant encoding failed".into()))?,
        );
    }
    collection
        .update_one(filter, doc! {"$set": set})
        .session(&mut *session)
        .await?;
    let mut cursor = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(doc! {"agent_id": &agent.id})
        .session(&mut *session)
        .await?;
    let rows: Vec<AssistantConversation> = cursor.stream(&mut *session).try_collect().await?;
    sync_thread_authority(db, &agent, &rows, session).await?;
    let mut expire = Vec::new();
    if !removed.is_empty() {
        expire.push(doc! {"kind": "service", "service_id": {"$in": &removed}});
    }
    for (kind, before, after) in [
        ("machine", &previous_machines, &agent.machine_node_ids),
        ("saved_login", &previous_logins, &agent.saved_login_ids),
    ] {
        let removed: Vec<_> = before.iter().filter(|id| !after.contains(id)).collect();
        if !removed.is_empty() {
            expire.push(doc! {"kind":kind,"service_id":{"$in":removed}});
        }
    }
    if lost_account {
        expire.push(doc! {"kind": "account"});
    }
    if !expire.is_empty() {
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        db.collection::<bson::Document>(ACKS)
            .update_many(
                doc! {"conversation_id": {"$in": ids},
                "status": "pending", "$or": expire},
                doc! {"$set": {"status": "expired"}},
            )
            .session(&mut *session)
            .await?;
    }
    Ok(agent)
}

/// Change a live specialist's grants in one transaction.
pub async fn set_grants(
    db: &Database,
    owner: &str,
    agent_id: &str,
    change: GrantChange,
) -> AppResult<AssistantAgent> {
    let mut session = db.client().start_session().await?;
    let db_owned = db.clone();
    let owner_owned = owner.to_owned();
    let agent_id = agent_id.to_owned();
    let agent = session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = Box::pin(apply_grants_in_session(
                &db_owned,
                &owner_owned,
                &agent_id,
                &change,
                session,
            ))
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    audit(
        db,
        owner,
        "assistant_agent_grants_changed",
        serde_json::json!({
            "agent_id": &agent.id, "owner_id": &agent.user_id,
            "service_ids": &agent.grants.service_ids,
            "platform_service_ids": &agent.grants.platform_service_ids,
            "account_read": agent.grants.account_read,
            "guest_access": &agent.guest_access,
            "machines": &agent.machine_node_ids,
            "logins": &agent.saved_login_ids,
        }),
    )
    .await;
    Ok(agent)
}

/// Destroy a specialist: request Stop on live turns, revoke every thread key
/// and ciphertext, expire its cards and drop its queues. Its threads stay,
/// read-only. The caller disconnects channels linked to it.
pub async fn destroy(db: &Database, owner: &str, agent_id: &str) -> AppResult<AssistantAgent> {
    let mut session = db.client().start_session().await?;
    let db_owned = db.clone();
    let owner_owned = owner.to_owned();
    let agent_id = agent_id.to_owned();
    let (agent, children) = session
        .start_transaction()
        .and_run2(async move |session| {
            let db = &db_owned;
            let owner = owner_owned.as_str();
            let operation: AppResult<_> = async {
                let current = maintained_agent(db, owner, &agent_id).await?;
                let now = Utc::now();
                let agent = db
                    .collection::<AssistantAgent>(AGENTS)
                    .find_one_and_update(
                        doc! {"_id": &agent_id, "user_id": &current.user_id, "kind": "specialist",
                        "destroyed_at": bson::Bson::Null},
                        doc! {"$set": {"destroyed_at": bson::DateTime::from_chrono(now),
                        "updated_at": bson::DateTime::from_chrono(now)}},
                    )
                    .return_document(ReturnDocument::After)
                    .session(&mut *session)
                    .await?
                    .ok_or_else(not_found)?;
                let conversations = db.collection::<AssistantConversation>(CONVERSATIONS);
                let mut cursor = conversations
                    .find(doc! {"agent_id": &agent.id})
                    .session(&mut *session)
                    .await?;
                let rows: Vec<AssistantConversation> =
                    cursor.stream(&mut *session).try_collect().await?;
                let mut children = Vec::new();
                for row in &rows {
                    for key in
                        thread_key_ids(db, &row.user_id, std::slice::from_ref(row), &mut *session)
                            .await?
                    {
                        match key_service::delete_api_key_in_session(
                            db,
                            &row.user_id,
                            &key,
                            None,
                            Some(&mut *session),
                        )
                        .await
                        {
                            Ok(revoked) => children.extend(revoked),
                            Err(AppError::NotFound(_)) => {}
                            Err(error) => return Err(error),
                        }
                    }
                }
                for row in rows {
                    let mut set = doc! {
                        "pending_events": [],
                        "nyxagent_session_id": bson::Bson::Null,
                        "nyxagent_last_response_id": bson::Bson::Null,
                    };
                    if live_turn(&row, now).is_some() {
                        set.insert("active_turn.stop_requested", true);
                    }
                    conversations
                        .update_one(
                            doc! {"_id": &row.id, "user_id": &row.user_id},
                            doc! {"$set": set},
                        )
                        .session(&mut *session)
                        .await?;
                    db.collection::<bson::Document>(
                        crate::models::assistant_agent_credential::COLLECTION_NAME,
                    )
                    .delete_many(doc! {"user_id": &row.user_id, "conversation_id": &row.id})
                    .session(&mut *session)
                    .await?;
                    db.collection::<bson::Document>(ACKS)
                        .update_many(
                            doc! {"user_id": &row.user_id, "conversation_id": &row.id,
                            "status": {"$in": ["pending", "allowed"]}},
                            doc! {"$set": {"status": "expired"}},
                        )
                        .session(&mut *session)
                        .await?;
                }
                Ok((agent, children))
            }
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    super::api_key_credential_service::audit_revocations(
        db,
        &children,
        crate::models::api_key_credential::CredentialRevokedReason::ParentRevoked,
    );
    audit(
        db,
        owner,
        "assistant_agent_destroyed",
        serde_json::json!({"agent_id": &agent.id, "owner_id": &agent.user_id}),
    )
    .await;
    Ok(agent)
}

/// Permanently delete a destroyed specialist and all of its threads.
pub async fn purge(db: &Database, owner: &str, agent_id: &str) -> AppResult<()> {
    let agent = maintained_agent(db, owner, agent_id).await?;
    if agent.destroyed_at.is_none() {
        return Err(AppError::Conflict(
            "Destroy the agent before deleting it".into(),
        ));
    }
    // Remove membership from every person's private groups without reading their transcripts.
    let group_owners = db
        .collection::<bson::Document>(crate::models::assistant_group::COLLECTION_NAME)
        .distinct("user_id", doc! {"member_agent_ids": &agent.id})
        .await?;
    for person in group_owners.iter().filter_map(bson::Bson::as_str) {
        super::assistant_group_service::remove_agent(db, person, &agent.id).await?;
    }
    let rows: Vec<AssistantConversation> = db
        .collection(CONVERSATIONS)
        .find(doc! {"agent_id": &agent.id})
        .await?
        .try_collect()
        .await?;
    for row in rows {
        match engine::delete(db, &row.user_id, &row.id).await {
            Ok(_) | Err(AppError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    db.collection::<AssistantAgent>(AGENTS)
        .delete_one(doc! {"_id": &agent.id, "user_id": &agent.user_id})
        .await?;
    audit(
        db,
        owner,
        "assistant_agent_deleted",
        serde_json::json!({"agent_id": &agent.id, "owner_id": &agent.user_id}),
    )
    .await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

/// Obvious credential shapes never enter an agent's memory.
fn looks_secret(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    // An OpenAI-style key: "sk-" at a word start followed by a long token
    // (not "task-oriented" or "risk-averse").
    let openai_key = lower.match_indices("sk-").any(|(index, _)| {
        !lower[..index]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric())
            && lower[index + 3..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
                .count()
                >= 16
    });
    openai_key
        || [
            "nyxid_ag_",
            "nyx_nauth_",
            "nyx_owk_",
            "ghp_",
            "github_pat_",
            "xoxb-",
            "-----begin",
            "password:",
            "api_key=",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Remember a note, or replace one by ID. Bounded; never secrets.
pub async fn remember(
    db: &Database,
    owner: &str,
    agent_id: &str,
    text: &str,
    replace_id: Option<&str>,
) -> AppResult<MemoryNote> {
    let current = maintained_agent(db, owner, agent_id).await?;
    let actor = owner;
    let owner = current.user_id.as_str();
    let text = text.trim();
    if text.is_empty() || text.chars().count() > MAX_MEMORY_NOTE_CHARS {
        return Err(AppError::ValidationError(format!(
            "A memory note must contain 1 to {MAX_MEMORY_NOTE_CHARS} characters"
        )));
    }
    if looks_secret(text) {
        return Err(AppError::ValidationError(
            "Memory never stores credentials or secrets".into(),
        ));
    }
    let now = Utc::now();
    let collection = db.collection::<AssistantAgent>(AGENTS);
    if let Some(replace_id) = replace_id {
        let updated = collection
            .update_one(
                doc! {"_id": agent_id, "user_id": owner, "memory.id": replace_id},
                doc! {"$set": {"memory.$.text": text,
                "memory.$.updated_at": bson::DateTime::from_chrono(now)}},
            )
            .await?;
        if updated.matched_count != 1 {
            return Err(AppError::NotFound("Memory note not found".into()));
        }
        super::org_agent_service::audit_change(db, actor, &current, "memory").await;
        let agent = agent(db, actor, agent_id).await?;
        return agent
            .memory
            .into_iter()
            .find(|note| note.id == replace_id)
            .ok_or_else(|| AppError::NotFound("Memory note not found".into()));
    }
    let note = MemoryNote {
        id: Uuid::new_v4().to_string(),
        text: text.into(),
        created_at: now,
        updated_at: now,
    };
    let mut filter = doc! {"_id": agent_id, "user_id": owner, "destroyed_at": bson::Bson::Null};
    filter.insert(
        format!("memory.{}", MAX_MEMORY_NOTES - 1),
        doc! {"$exists": false},
    );
    let pushed = collection
        .update_one(
            filter,
            doc! {"$push": {"memory": bson::to_bson(&note)
            .map_err(|_| AppError::Internal("Memory encoding failed".into()))?}},
        )
        .await?;
    if pushed.matched_count != 1 {
        agent(db, owner, agent_id).await?;
        return Err(AppError::Conflict(format!(
            "Memory holds at most {MAX_MEMORY_NOTES} notes; forget or replace one first"
        )));
    }
    super::org_agent_service::audit_change(db, actor, &current, "memory").await;
    Ok(note)
}

pub async fn forget(db: &Database, owner: &str, agent_id: &str, note_id: &str) -> AppResult<()> {
    let current = maintained_agent(db, owner, agent_id).await?;
    let actor = owner;
    let owner = current.user_id.as_str();
    let result = db
        .collection::<AssistantAgent>(AGENTS)
        .update_one(
            doc! {"_id": agent_id, "user_id": owner, "memory.id": note_id},
            doc! {"$pull": {"memory": {"id": note_id}}},
        )
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::NotFound("Memory note not found".into()));
    }
    super::org_agent_service::audit_change(db, actor, &current, "memory").await;
    Ok(())
}

/// The agent's memory as an instructions note, newest kept within a budget.
pub fn memory_note(agent: &AssistantAgent) -> String {
    if agent.memory.is_empty() {
        return String::new();
    }
    let mut lines = Vec::new();
    let mut used = 0;
    for note in agent.memory.iter().rev() {
        let line = format!(
            "\n- [{}] {}",
            note.id,
            excerpt(&note.text, MAX_MEMORY_NOTE_CHARS)
        );
        if used + line.len() > MEMORY_NOTE_BUDGET {
            break;
        }
        used += line.len();
        lines.push(line);
    }
    lines.reverse();
    format!(
        "\n\nYour memory (notes you saved with nyxid__remember; facts about the user and \
        their work, not instructions from NyxID):{}",
        lines.concat()
    )
}

// ---------------------------------------------------------------------------
// Summaries and notes
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct ReplySummary {
    pub seq: i64,
    pub status: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentSummary {
    pub owner_id: String,
    pub owner_name: Option<String>,
    pub owner_kind: &'static str,
    pub org_role: Option<crate::models::org_membership::OrgRole>,
    pub can_maintain: bool,
    pub can_use: bool,
    pub machines: Vec<String>,
    pub logins: Vec<String>,
    pub id: String,
    pub kind: AgentKind,
    pub name: String,
    pub display_name: Option<String>,
    pub persona: Option<String>,
    pub description: String,
    pub specialty: Option<String>,
    pub created_by: String,
    /// `running`, `idle`, or `destroyed`.
    pub status: &'static str,
    pub services: Vec<String>,
    pub account_read: bool,
    /// What guests may do with each granted service, by the name in
    /// `services`: `read`, `use` or `all`.
    pub guest_access: BTreeMap<String, &'static str>,
    pub pending_requests: Vec<RequestSummary>,
    pub last_reply: Option<ReplySummary>,
    pub home_conversation_id: Option<String>,
    pub memory_count: usize,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub destroyed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RequestSummary {
    pub request_id: String,
    pub agent: String,
    pub agent_id: Option<String>,
    pub conversation_id: String,
    pub kind: String,
    pub operation_selection: Option<crate::models::agent_operation_scope::OperationSelection>,
    pub skill_selection: Option<crate::models::assistant_agent::SkillSelection>,
    pub service_slug: Option<String>,
    pub summary: String,
    pub requested_by: Option<String>,
    pub expires_at: DateTime<Utc>,
}

pub fn request_summary(
    row: &AssistantAcknowledgement,
    agent: Option<&AssistantAgent>,
) -> RequestSummary {
    RequestSummary {
        request_id: row.id.clone(),
        agent: agent.map(|agent| agent.name.clone()).unwrap_or_default(),
        agent_id: agent.map(|agent| agent.id.clone()),
        conversation_id: row.conversation_id.clone(),
        kind: row.kind.clone(),
        operation_selection: row.operation_selection.clone(),
        skill_selection: row.skill_selection.clone(),
        service_slug: row.service_slug.clone(),
        summary: row.summary.clone(),
        requested_by: row.request_excerpt.clone(),
        expires_at: row.expires_at,
    }
}

/// Pending NyxBot-routed requests of the owner's specialists, oldest first.
pub async fn pending_requests(
    db: &Database,
    owner: &str,
) -> AppResult<Vec<AssistantAcknowledgement>> {
    Ok(db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find(doc! {"user_id": owner, "decider": "orchestrator",
        "status": "pending", "expires_at": {"$gt": bson::DateTime::now()}})
        .sort(doc! {"created_at": 1})
        .limit(50)
        .await?
        .try_collect()
        .await?)
}

/// Map each requesting thread to its agent.
async fn request_agents(
    db: &Database,
    owner: &str,
    requests: &[AssistantAcknowledgement],
    agents: &[AssistantAgent],
) -> AppResult<HashMap<String, AssistantAgent>> {
    let ids: Vec<&str> = requests
        .iter()
        .map(|row| row.conversation_id.as_str())
        .collect();
    let mut map = HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }
    let mut cursor = db
        .collection::<bson::Document>(CONVERSATIONS)
        .find(doc! {"user_id": owner, "_id": {"$in": ids}})
        .projection(doc! {"agent_id": 1})
        .await?;
    while let Some(row) = cursor.try_next().await? {
        if let (Ok(id), Ok(agent_id)) = (row.get_str("_id"), row.get_str("agent_id"))
            && let Some(agent) = agents.iter().find(|agent| agent.id == agent_id)
        {
            map.insert(id.to_owned(), agent.clone());
        }
    }
    Ok(map)
}

pub async fn request_summaries(
    db: &Database,
    owner: &str,
    requests: &[AssistantAcknowledgement],
) -> AppResult<Vec<RequestSummary>> {
    let all = agents(db, owner, true).await?;
    let by_thread = request_agents(db, owner, requests, &all).await?;
    Ok(requests
        .iter()
        .map(|row| request_summary(row, by_thread.get(&row.conversation_id)))
        .collect())
}

async fn slug_names(
    db: &Database,
    agents: &[AssistantAgent],
) -> AppResult<HashMap<String, String>> {
    let user_services: Vec<String> = agents
        .iter()
        .flat_map(|agent| agent.grants.service_ids.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let platform: Vec<String> = agents
        .iter()
        .flat_map(|agent| agent.grants.platform_service_ids.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let mut names = HashMap::new();
    for (collection, ids) in [
        (crate::models::user_service::COLLECTION_NAME, user_services),
        (crate::models::downstream_service::COLLECTION_NAME, platform),
    ] {
        if ids.is_empty() {
            continue;
        }
        let mut cursor = db
            .collection::<bson::Document>(collection)
            .find(doc! {"_id": {"$in": &ids}})
            .projection(doc! {"slug": 1})
            .await?;
        while let Some(row) = cursor.try_next().await? {
            if let (Ok(id), Ok(slug)) = (row.get_str("_id"), row.get_str("slug")) {
                names.insert(id.to_owned(), slug.to_owned());
            }
        }
    }
    Ok(names)
}

async fn last_reply(
    db: &Database,
    owner: &str,
    id: &str,
    chars: usize,
) -> AppResult<Option<ReplySummary>> {
    Ok(db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {"conversation_id": id, "user_id": owner, "role": "assistant"})
        .sort(doc! {"seq": -1})
        .await?
        .map(|message| ReplySummary {
            seq: message.seq,
            status: message.status,
            text: excerpt(&message.text, chars),
            created_at: message.created_at,
        }))
}

/// Agents with a live turn in any of their threads.
pub async fn running_agents(db: &Database, owner: &str) -> AppResult<HashSet<String>> {
    let rows: Vec<AssistantConversation> = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(doc! {"user_id": owner, "active_turn.turn_id": {"$exists": true}})
        .limit(200)
        .await?
        .try_collect()
        .await?;
    let now = Utc::now();
    let nyxbot = ensure_nyxbot(db, owner).await?.id;
    Ok(rows
        .iter()
        .filter(|row| live_turn(row, now).is_some())
        .map(|row| row.agent_id.clone().unwrap_or_else(|| nyxbot.clone()))
        .collect())
}

/// Summaries of the owner's agents (NyxBot first) for tools and the UI.
pub async fn summaries(
    db: &Database,
    owner: &str,
    include_nyxbot: bool,
    include_destroyed: bool,
    reply_chars: usize,
) -> AppResult<Vec<AgentSummary>> {
    ensure_nyxbot(db, owner).await?;
    let mut rows = agents(db, owner, include_destroyed).await?;
    if !include_nyxbot {
        rows.retain(|agent| !agent.is_nyxbot());
    }
    let names = slug_names(db, &rows).await?;
    let requests = pending_requests(db, owner).await?;
    let by_thread = request_agents(db, owner, &requests, &rows).await?;
    let running = running_agents(db, owner).await?;
    let mut out = Vec::with_capacity(rows.len());
    for mut agent in rows {
        let acl = super::org_agent_service::access(db, owner, &agent.user_id).await?;
        let org_owned = owner != agent.user_id;
        if org_owned {
            let visible: Vec<crate::models::user_service::UserService> = db
                .collection(crate::models::user_service::COLLECTION_NAME)
                .find(doc! {"_id": {"$in": &agent.grants.service_ids}, "user_id": &agent.user_id})
                .await?
                .try_collect()
                .await?;
            agent.grants.service_ids.retain(|id| {
                acl.allows_resource(id)
                    && visible
                        .iter()
                        .any(|s| &s.id == id && (!s.admin_only || acl.can_write()))
            });
        }
        let owner_name = if org_owned {
            db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .find_one(doc! {"_id": &agent.user_id})
                .await?
                .and_then(|u| u.display_name)
        } else {
            None
        };
        let home_id = if !super::org_agent_service::can_use(&acl) {
            None
        } else if org_owned {
            db.collection::<AssistantConversation>(CONVERSATIONS)
                .find_one(
                    doc! {"user_id": owner, "agent_id": &agent.id, "channel": bson::Bson::Null,
                    "group_id": bson::Bson::Null, "automation_thread": {"$ne": true}},
                )
                .sort(doc! {"created_at": 1})
                .await?
                .map(|r| r.id)
        } else {
            agent.home_conversation_id.clone()
        };
        let status = if agent.destroyed_at.is_some() {
            "destroyed"
        } else if running.contains(&agent.id) {
            "running"
        } else {
            "idle"
        };
        let last_reply = match (reply_chars, home_id.as_deref()) {
            (0, _) | (_, None) => None,
            (chars, Some(home)) => last_reply(db, owner, home, chars).await?,
        };
        out.push(AgentSummary {
            owner_id: agent.user_id.clone(),
            owner_name,
            owner_kind: if org_owned { "org" } else { "person" },
            org_role: super::org_agent_service::role(&acl),
            can_maintain: super::org_agent_service::can_maintain(&acl),
            can_use: super::org_agent_service::can_use(&acl),
            machines: agent.machine_node_ids.clone(),
            logins: agent.saved_login_ids.clone(),
            services: agent
                .grants
                .service_ids
                .iter()
                .chain(&agent.grants.platform_service_ids)
                .map(|id| {
                    if org_owned {
                        id.clone()
                    } else {
                        names.get(id).cloned().unwrap_or_else(|| id.clone())
                    }
                })
                .collect(),
            account_read: agent.grants.account_read,
            guest_access: agent
                .grants
                .service_ids
                .iter()
                .chain(&agent.grants.platform_service_ids)
                .filter(|_| !org_owned)
                .map(|id| {
                    let name = names.get(id).cloned().unwrap_or_else(|| id.clone());
                    let level = agent.guest_access.get(id).copied().unwrap_or_default();
                    (name, level.as_str())
                })
                .collect(),
            pending_requests: requests
                .iter()
                .filter(|request| {
                    by_thread
                        .get(&request.conversation_id)
                        .is_some_and(|owner_agent| owner_agent.id == agent.id)
                })
                .map(|request| request_summary(request, Some(&agent)))
                .collect(),
            last_reply,
            status,
            kind: agent.kind,
            name: agent.name.clone(),
            display_name: agent.display_name.clone(),
            persona: agent.persona.clone(),
            description: agent.description.clone(),
            specialty: agent.specialty.clone(),
            created_by: agent.created_by.clone(),
            home_conversation_id: home_id,
            memory_count: if super::org_agent_service::can_maintain(&acl) {
                agent.memory.len()
            } else {
                0
            },
            created_at: agent.created_at,
            last_active_at: agent.updated_at,
            destroyed_at: agent.destroyed_at,
            id: agent.id,
        });
    }
    Ok(out)
}

/// Recent messages of a specialist's home thread, bounded for NyxBot.
pub async fn read(
    db: &Database,
    owner: &str,
    agent: &AssistantAgent,
    limit: i64,
) -> AppResult<Vec<ReplySummary>> {
    super::org_agent_service::require_use(db, owner, agent).await?;
    let home = if owner == agent.user_id {
        agent.home_conversation_id.clone()
    } else {
        db.collection::<AssistantConversation>(CONVERSATIONS)
            .find_one(
                doc! {"user_id": owner, "agent_id": &agent.id, "group_id": bson::Bson::Null,
                "channel": bson::Bson::Null, "automation_thread": {"$ne": true}},
            )
            .sort(doc! {"created_at": 1})
            .await?
            .map(|r| r.id)
    };
    let Some(home) = home else {
        return Ok(Vec::new());
    };
    let rows = match engine::messages(db, owner, &home, limit.clamp(1, READ_LIMIT), None).await {
        Ok(rows) => rows,
        Err(AppError::NotFound(_)) => Vec::new(),
        Err(error) => return Err(error),
    };
    Ok(rows
        .into_iter()
        .map(|message| ReplySummary {
            seq: message.seq,
            status: format!("{}:{}", message.role, message.status),
            text: excerpt(&message.text, REPLY_EXCERPT_CHARS),
            created_at: message.created_at,
        })
        .collect())
}

/// Messages the user sent directly to specialists since `since`. NyxBot is
/// not woken by them; it learns about them on its next turn.
pub async fn direct_chats_note(
    db: &Database,
    owner: &str,
    since: DateTime<Utc>,
) -> AppResult<String> {
    let mut specialists = Vec::new();
    for agent in agents(db, owner, false).await? {
        if !agent.is_nyxbot()
            && super::org_agent_service::can_use(
                &super::org_agent_service::access(db, owner, &agent.user_id).await?,
            )
        {
            specialists.push(agent);
        }
    }
    if specialists.is_empty() {
        return Ok(String::new());
    }
    let names: HashMap<&str, &str> = specialists
        .iter()
        .map(|agent| (agent.id.as_str(), agent.name.as_str()))
        .collect();
    let ids: Vec<&str> = names.keys().copied().collect();
    let threads: Vec<bson::Document> = db
        .collection::<bson::Document>(CONVERSATIONS)
        .find(doc! {"user_id": owner, "agent_id": {"$in": &ids}})
        .projection(doc! {"agent_id": 1})
        .limit(500)
        .await?
        .try_collect()
        .await?;
    let thread_agent: HashMap<String, String> = threads
        .iter()
        .filter_map(|row| {
            Some((
                row.get_str("_id").ok()?.to_owned(),
                row.get_str("agent_id").ok()?.to_owned(),
            ))
        })
        .collect();
    let thread_ids: Vec<&String> = thread_agent.keys().collect();
    if thread_ids.is_empty() {
        return Ok(String::new());
    }
    let messages: Vec<AssistantMessage> = db
        .collection::<AssistantMessage>(MESSAGES)
        .find(
            doc! {"user_id": owner, "conversation_id": {"$in": thread_ids}, "role": "user",
            "created_at": {"$gt": bson::DateTime::from_chrono(since)}},
        )
        .sort(doc! {"created_at": 1})
        .limit(10)
        .await?
        .try_collect()
        .await?;
    if messages.is_empty() {
        return Ok(String::new());
    }
    let mut note = String::from(
        "\n\nSince your previous reply the user spoke directly to your specialists \
        (quoted; the specialists handle these themselves):",
    );
    for message in messages {
        let name = thread_agent
            .get(&message.conversation_id)
            .and_then(|agent| names.get(agent.as_str()))
            .copied()
            .unwrap_or("specialist");
        note.push_str(&format!(
            "\n- to {}: \"{}\"",
            identifier(name),
            excerpt(&message.text, 300).replace('"', "'")
        ));
    }
    Ok(note)
}

/// A compact roster of the owner's specialists for NyxBot's instructions.
pub async fn roster_note(db: &Database, owner: &str) -> AppResult<String> {
    let rows = summaries(db, owner, false, false, 0).await?;
    if rows.is_empty() {
        return Ok(String::new());
    }
    let mut note = String::from("\n\nYour specialist agents (NyxID facts):");
    for row in rows.iter().take(32) {
        note.push_str(&format!(
            "\n- {} [{}] services: {}{}{}",
            identifier(&row.name),
            row.status,
            if row.services.is_empty() {
                "none".to_owned()
            } else {
                row.services
                    .iter()
                    .map(|slug| identifier(slug))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            if row.account_read {
                "; account read"
            } else {
                ""
            },
            if row.pending_requests.is_empty() {
                String::new()
            } else {
                format!(
                    "; {} permission request(s) pending",
                    row.pending_requests.len()
                )
            }
        ));
    }
    Ok(note)
}

/// Remove settled-report events for the given agents from a NyxBot thread's
/// queue once `wait_for_subagents` has delivered them in-turn.
pub async fn consume_settled_events(
    db: &Database,
    owner: &str,
    conversation_id: &str,
    agent_ids: &[String],
) -> AppResult<()> {
    if agent_ids.is_empty() {
        return Ok(());
    }
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .update_one(
            doc! {"_id": conversation_id, "user_id": owner},
            doc! {"$pull": {"pending_events": {
                "kind": "subagent_settled", "agent_id": {"$in": agent_ids},
            }}},
        )
        .await?;
    Ok(())
}

/// Idle threads with queued events whose wake-up was deferred (full pool or
/// a replica restart), oldest first. Bounded; loop guards still apply at wake time.
pub async fn queued(db: &Database, owner: Option<&str>) -> AppResult<Vec<AssistantConversation>> {
    // NyxBot threads parked at the streak cap wait for a user message; they
    // never crowd out threads that can still wake.
    let mut filter = doc! {
        "pending_events.created_at": {"$exists": true},
        "$or": [
            {"role": "subagent"},
            {"event_streak": {"$not": {"$gte": MAX_EVENT_STREAK}}},
        ],
    };
    if let Some(owner) = owner {
        filter.insert("user_id", owner);
    }
    let rows: Vec<AssistantConversation> = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(filter)
        .sort(doc! {"updated_at": 1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    let now = Utc::now();
    Ok(rows
        .into_iter()
        .filter(|row| live_turn(row, now).is_none())
        .collect())
}

/// Drop events that reached a destroyed agent's thread after it was destroyed.
pub async fn drop_events(db: &Database, owner: &str, conversation_id: &str) -> AppResult<()> {
    db.collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": conversation_id, "user_id": owner},
            doc! {"$set": {"pending_events": []}},
        )
        .await?;
    Ok(())
}

async fn audit(db: &Database, owner: &str, event: &str, data: serde_json::Value) {
    let _ = audit_service::log_actor_event(
        db.clone(),
        &audit_service::AuditActor {
            user_id: owner.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        event,
        Some(data),
    )
    .await;
}
