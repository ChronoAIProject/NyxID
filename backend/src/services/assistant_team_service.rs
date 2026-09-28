//! NyxBot teams: one orchestrator conversation and its disposable subagents.
//!
//! Every agent is an ordinary NyxAgent conversation with its own restricted
//! key. The orchestrator runs with Full access; a subagent's key carries only
//! the grants recorded on its row. Starting turns and waking agents needs the
//! HTTP proxy and lives in `handlers::assistant_team`; this module owns the
//! durable state.
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::{
        assistant_acknowledgement::{AssistantAcknowledgement, COLLECTION_NAME as ACKS},
        assistant_conversation::{
            AccessMode, AgentEvent, AgentRole, AssistantConversation,
            COLLECTION_NAME as CONVERSATIONS, SubagentGrants,
        },
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    },
    services::{
        api_key_mutation_service as transactions,
        assistant_agent_credential_service::{
            self as credentials, AssistantCredential, KeyAuthority,
        },
        assistant_nyxagent::{self as engine, excerpt, identifier, live_turn},
        assistant_profile_routing::{self as routing, RouteRole},
        assistant_settings_service, audit_service, key_service, mcp_service,
        node_ws_manager::NodeWsManager,
    },
};

pub const MAX_NAME_CHARS: usize = 32;
pub const MAX_CHARTER_CHARS: usize = 2048;
pub const MAX_GRANT_TARGETS: usize = 32;
/// Subagents with no turn for this long are destroyed by the idle sweep.
pub const IDLE_DESTROY_DAYS: i64 = 7;
pub const IDLE_SWEEP_INTERVAL_SECS: u64 = 3600;
/// Loop guards for server-started event turns.
pub const EVENT_TURNS_PER_HOUR: u64 = 20;
pub const MAX_EVENT_STREAK: i32 = 3;
/// Bounded like `nyx__wait_for_connection`, below NyxAgent's 150 s call timeout.
pub const MAX_WAIT_SECS: u64 = 120;
pub const READ_LIMIT: i64 = 20;
pub const REPLY_EXCERPT_CHARS: usize = 2000;

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_CHARS
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && name.as_bytes()[0].is_ascii_alphanumeric()
}

pub fn event(kind: &str, text: String, subagent_id: Option<&str>) -> AgentEvent {
    AgentEvent {
        id: Uuid::new_v4().to_string(),
        kind: kind.into(),
        text,
        subagent_id: subagent_id.map(str::to_owned),
        created_at: Utc::now(),
    }
}

fn not_found() -> AppError {
    AppError::NotFound("Subagent not found".into())
}

/// The owner's live orchestrator conversation.
pub async fn orchestrator(
    db: &Database,
    owner: &str,
    id: &str,
) -> AppResult<AssistantConversation> {
    let row = engine::get(db, owner, id).await?;
    if row.role != AgentRole::Orchestrator {
        return Err(AppError::NotFound("Conversation not found".into()));
    }
    Ok(row)
}

/// A team member by conversation ID or name. Destroyed members resolve too
/// (read-only views); mutating callers check `destroyed_at`.
pub async fn member(
    db: &Database,
    owner: &str,
    team_id: &str,
    name_or_id: &str,
) -> AppResult<AssistantConversation> {
    let collection = db.collection::<AssistantConversation>(CONVERSATIONS);
    let filter = if engine::valid_id(name_or_id) {
        doc! {"_id": name_or_id, "user_id": owner, "team_id": team_id}
    } else if valid_name(name_or_id) {
        doc! {"user_id": owner, "team_id": team_id, "agent_name": name_or_id}
    } else {
        return Err(not_found());
    };
    // Prefer the live member when a name was reused after a destroy.
    collection
        .find_one(filter)
        .sort(doc! {"destroyed_at": 1, "created_at": -1})
        .await?
        .ok_or_else(not_found)
}

pub async fn members(
    db: &Database,
    owner: &str,
    team_ids: &[String],
) -> AppResult<Vec<AssistantConversation>> {
    if team_ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(doc! {"user_id": owner, "team_id": {"$in": team_ids}})
        .sort(doc! {"created_at": 1})
        .limit(500)
        .await?
        .try_collect()
        .await?)
}

/// Services resolved from slugs or IDs exactly as MCP enforces them: user
/// services by `UserService` ID, platform services by catalog ID.
#[derive(Clone, Debug, Default)]
pub struct GrantTargets {
    pub service_ids: Vec<String>,
    pub platform_service_ids: Vec<String>,
    pub slugs: Vec<String>,
}

pub async fn resolve_targets(
    db: &Database,
    node_manager: &NodeWsManager,
    owner: &str,
    targets: &[String],
) -> AppResult<GrantTargets> {
    if targets.len() > MAX_GRANT_TARGETS {
        return Err(AppError::ValidationError(format!(
            "At most {MAX_GRANT_TARGETS} services can be granted at once"
        )));
    }
    let mut resolved = GrantTargets::default();
    if targets.is_empty() {
        return Ok(resolved);
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
        let service = catalog
            .services
            .iter()
            .find(|service| service.service_slug == target || service.service_id == target)
            .ok_or_else(|| {
                AppError::ValidationError(format!(
                    "Unknown or unavailable service: {}",
                    identifier(target)
                ))
            })?;
        let (list, id) = match &service.source {
            mcp_service::McpToolSource::Internal => {
                return Err(AppError::ValidationError(
                    "Grant account access with account_read instead of the nyxid service".into(),
                ));
            }
            mcp_service::McpToolSource::Platform { .. } => (
                &mut resolved.platform_service_ids,
                service.service_id.clone(),
            ),
            mcp_service::McpToolSource::UserManaged { .. } => {
                (&mut resolved.service_ids, service.service_id.clone())
            }
        };
        if !list.contains(&id) {
            list.push(id);
            resolved.slugs.push(service.service_slug.clone());
        }
    }
    Ok(resolved)
}

#[derive(Clone)]
pub struct SpawnRequest {
    pub name: String,
    pub charter: String,
    pub targets: GrantTargets,
    pub account_read: bool,
    pub specialty: Option<String>,
}
impl std::fmt::Debug for SpawnRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnRequest")
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

/// Create a subagent row, its key and its encrypted credential in one
/// transaction. Fencing the orchestrator row serializes concurrent spawns so
/// the owner's live-subagent limit holds.
pub async fn spawn(
    db: &Database,
    keys: &std::sync::Arc<EncryptionKeys>,
    owner: &str,
    orchestrator: &AssistantConversation,
    request: SpawnRequest,
) -> AppResult<Result<(AssistantConversation, AssistantCredential), TeamRefusal>> {
    if !valid_name(&request.name) {
        return Err(AppError::ValidationError(
            "Subagent names use 1-32 lowercase letters, digits, or hyphens".into(),
        ));
    }
    let charter = request.charter.trim().to_owned();
    if charter.is_empty() || charter.chars().count() > MAX_CHARTER_CHARS {
        return Err(AppError::ValidationError(format!(
            "A charter must contain 1 to {MAX_CHARTER_CHARS} characters"
        )));
    }
    if request
        .specialty
        .as_deref()
        .is_some_and(|value| !routing::valid_specialty(value))
    {
        return Err(AppError::ValidationError(
            "specialty uses up to 32 lowercase letters, digits, hyphens or underscores".into(),
        ));
    }
    let limit = assistant_settings_service::get(db, owner)
        .await?
        .max_live_subagents;
    let model = routing::model_for(
        db,
        RouteRole::Subagent {
            specialty: request.specialty.as_deref(),
        },
        &orchestrator.model,
    )
    .await;
    let id = format!("nyxa-{}", Uuid::new_v4().simple());
    let mut session = db.client().start_session().await?;
    let db_owned = db.clone();
    let keys = keys.clone();
    let owner_owned = owner.to_owned();
    let team_id = orchestrator.id.clone();
    let outcome = session
        .start_transaction()
        .and_run2(async move |session| {
            let db = &db_owned;
            let owner = owner_owned.as_str();
            let operation: AppResult<_> = async {
                let collection = db.collection::<AssistantConversation>(CONVERSATIONS);
                // Serialize team mutations on the orchestrator row.
                let fenced = collection
                    .update_one(
                        doc! {"_id": &team_id, "user_id": owner, "role": {"$ne": "subagent"}},
                        doc! {"$inc": {"team_fence": 1}},
                    )
                    .session(&mut *session)
                    .await?;
                if fenced.matched_count != 1 {
                    return Err(AppError::NotFound("Conversation not found".into()));
                }
                let live = collection
                    .count_documents(doc! {"user_id": owner, "team_id": &team_id,
                    "destroyed_at": bson::Bson::Null})
                    .session(&mut *session)
                    .await?;
                if live >= limit.max(0) as u64 {
                    return Ok(Err(TeamRefusal::LimitReached { limit }));
                }
                if collection
                    .find_one(doc! {"user_id": owner, "team_id": &team_id,
                    "agent_name": &request.name, "destroyed_at": bson::Bson::Null})
                    .session(&mut *session)
                    .await?
                    .is_some()
                {
                    return Ok(Err(TeamRefusal::NameTaken));
                }
                let now = Utc::now();
                let row = AssistantConversation {
                    id: id.clone(),
                    user_id: owner.into(),
                    title: request.name.clone(),
                    model: model.clone(),
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
                    role: AgentRole::Subagent,
                    team_id: Some(team_id.clone()),
                    agent_name: Some(request.name.clone()),
                    charter: Some(charter.clone()),
                    specialty: request.specialty.clone(),
                    grants: SubagentGrants {
                        service_ids: request.targets.service_ids.clone(),
                        platform_service_ids: request.targets.platform_service_ids.clone(),
                        account_read: request.account_read,
                    },
                    destroyed_at: None,
                    pending_events: Vec::new(),
                    event_streak: 0,
                    channel: None,
                };
                collection.insert_one(&row).session(&mut *session).await?;
                let credential = credentials::load_or_provision_in_session(
                    db,
                    &keys,
                    owner,
                    &row.id,
                    KeyAuthority::of(&row),
                    &mut *session,
                )
                .await?;
                let mut row = row;
                row.credential_api_key_id = credential.api_key_id.clone();
                collection
                    .update_one(
                        doc! {"_id": &row.id, "user_id": owner},
                        doc! {"$set": {"credential_api_key_id": &row.credential_api_key_id}},
                    )
                    .session(&mut *session)
                    .await?;
                Ok(Ok((row, credential)))
            }
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    if let Ok((row, credential)) = &outcome {
        credentials::audit_provision(db, owner, &row.id, credential, false).await;
        audit(
            db,
            owner,
            "assistant_subagent_spawned",
            serde_json::json!({
                "team_id": &orchestrator.id, "conversation_id": &row.id,
                "service_ids": &row.grants.service_ids,
                "platform_service_ids": &row.grants.platform_service_ids,
                "account_read": row.grants.account_read,
            }),
        )
        .await;
    }
    Ok(outcome)
}

/// Replace a live subagent's grants; its key converges in the same
/// transaction. Revoked targets also expire their pending requests.
pub async fn set_grants(
    db: &Database,
    owner: &str,
    team_id: &str,
    member_id: &str,
    grants: SubagentGrants,
) -> AppResult<AssistantConversation> {
    let mut session = db.client().start_session().await?;
    let db_owned = db.clone();
    let owner_owned = owner.to_owned();
    let team_id = team_id.to_owned();
    let member_id = member_id.to_owned();
    let row = session
        .start_transaction()
        .and_run2(async move |session| {
            let db = &db_owned;
            let owner = owner_owned.as_str();
            let operation: AppResult<_> = async {
                let collection = db.collection::<AssistantConversation>(CONVERSATIONS);
                let filter = doc! {"_id": &member_id, "user_id": owner, "team_id": &team_id,
                "destroyed_at": bson::Bson::Null};
                let mut row = collection
                    .find_one(filter.clone())
                    .session(&mut *session)
                    .await?
                    .ok_or_else(not_found)?;
                let removed_services: Vec<String> = row
                    .grants
                    .service_ids
                    .iter()
                    .chain(&row.grants.platform_service_ids)
                    .filter(|id| {
                        !grants.service_ids.contains(id)
                            && !grants.platform_service_ids.contains(id)
                    })
                    .cloned()
                    .collect();
                row.grants = grants.clone();
                collection
                    .update_one(
                        filter,
                        doc! {"$set": {"grants": bson::to_bson(&row.grants)
                        .map_err(|_| AppError::Internal("Grant encoding failed".into()))?}},
                    )
                    .session(&mut *session)
                    .await?;
                if !row.credential_api_key_id.is_empty() {
                    match credentials::apply_authority(
                        db,
                        owner,
                        &row.credential_api_key_id,
                        KeyAuthority::of(&row),
                        &mut *session,
                    )
                    .await
                    {
                        // A replaced key converges at its next turn start.
                        Ok(()) | Err(AppError::NotFound(_)) => {}
                        Err(error) => return Err(error),
                    }
                }
                let mut expire =
                    vec![doc! {"kind": "service", "service_id": {"$in": &removed_services}}];
                if !grants.account_read {
                    expire.push(doc! {"kind": "account"});
                }
                if !removed_services.is_empty() || !grants.account_read {
                    db.collection::<bson::Document>(ACKS)
                        .update_many(
                            doc! {"user_id": owner, "conversation_id": &row.id,
                            "status": "pending", "$or": expire},
                            doc! {"$set": {"status": "expired"}},
                        )
                        .session(&mut *session)
                        .await?;
                }
                Ok(row)
            }
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    audit(
        db,
        owner,
        "assistant_subagent_grants_changed",
        serde_json::json!({
            "team_id": row.team_id, "conversation_id": &row.id,
            "service_ids": &row.grants.service_ids,
            "platform_service_ids": &row.grants.platform_service_ids,
            "account_read": row.grants.account_read,
        }),
    )
    .await;
    Ok(row)
}

/// Destroy a subagent: request Stop on a live turn, revoke its key and
/// ciphertext, expire its cards and drop its queue. The transcript stays,
/// read-only, under the team.
pub async fn destroy(
    db: &Database,
    owner: &str,
    team_id: &str,
    member_id: &str,
) -> AppResult<AssistantConversation> {
    let mut session = db.client().start_session().await?;
    let db_owned = db.clone();
    let owner_owned = owner.to_owned();
    let team_id = team_id.to_owned();
    let member_id = member_id.to_owned();
    let (row, children) = session
        .start_transaction()
        .and_run2(async move |session| {
            let db = &db_owned;
            let owner = owner_owned.as_str();
            let operation: AppResult<_> = async {
                let collection = db.collection::<AssistantConversation>(CONVERSATIONS);
                let filter = doc! {"_id": &member_id, "user_id": owner, "team_id": &team_id,
                "destroyed_at": bson::Bson::Null};
                let mut row = collection
                    .find_one(filter.clone())
                    .session(&mut *session)
                    .await?
                    .ok_or_else(not_found)?;
                let now = Utc::now();
                let mut set = doc! {
                    "destroyed_at": bson::DateTime::from_chrono(now),
                    "pending_events": [],
                    "nyxagent_session_id": bson::Bson::Null,
                    "nyxagent_last_response_id": bson::Bson::Null,
                };
                if live_turn(&row, now).is_some() {
                    set.insert("active_turn.stop_requested", true);
                }
                collection
                    .update_one(filter, doc! {"$set": set})
                    .session(&mut *session)
                    .await?;
                let children = match key_service::delete_api_key_in_session(
                    db,
                    owner,
                    &row.credential_api_key_id,
                    None,
                    Some(&mut *session),
                )
                .await
                {
                    Ok(children) => children,
                    Err(AppError::NotFound(_)) => Vec::new(),
                    Err(error) => return Err(error),
                };
                db.collection::<bson::Document>(
                    crate::models::assistant_agent_credential::COLLECTION_NAME,
                )
                .delete_many(doc! {"user_id": owner, "conversation_id": &row.id})
                .session(&mut *session)
                .await?;
                db.collection::<bson::Document>(ACKS)
                    .update_many(
                        doc! {"user_id": owner, "conversation_id": &row.id,
                        "status": {"$in": ["pending", "allowed"]}},
                        doc! {"$set": {"status": "expired"}},
                    )
                    .session(&mut *session)
                    .await?;
                row.destroyed_at = Some(now);
                row.pending_events.clear();
                Ok((row, children))
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
        "assistant_subagent_destroyed",
        serde_json::json!({"team_id": row.team_id, "conversation_id": &row.id}),
    )
    .await;
    Ok(row)
}

#[derive(Clone, Debug, Serialize)]
pub struct ReplySummary {
    pub seq: i64,
    pub status: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemberSummary {
    pub id: String,
    pub name: String,
    pub charter: String,
    pub specialty: Option<String>,
    /// `running`, `idle`, or `destroyed`.
    pub status: &'static str,
    pub services: Vec<String>,
    pub account_read: bool,
    pub pending_requests: Vec<RequestSummary>,
    pub last_reply: Option<ReplySummary>,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RequestSummary {
    pub request_id: String,
    pub subagent: String,
    pub kind: String,
    pub service_slug: Option<String>,
    pub summary: String,
    pub requested_by: Option<String>,
    pub expires_at: DateTime<Utc>,
}

pub fn request_summary(row: &AssistantAcknowledgement, name: &str) -> RequestSummary {
    RequestSummary {
        request_id: row.id.clone(),
        subagent: name.to_owned(),
        kind: row.kind.clone(),
        service_slug: row.service_slug.clone(),
        summary: row.summary.clone(),
        requested_by: row.request_excerpt.clone(),
        expires_at: row.expires_at,
    }
}

pub fn status(row: &AssistantConversation, now: DateTime<Utc>) -> &'static str {
    if row.destroyed_at.is_some() {
        "destroyed"
    } else if live_turn(row, now).is_some() {
        "running"
    } else {
        "idle"
    }
}

/// Pending orchestrator-routed requests for a team, oldest first.
pub async fn pending_requests(
    db: &Database,
    owner: &str,
    team_id: &str,
) -> AppResult<Vec<AssistantAcknowledgement>> {
    Ok(db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find(
            doc! {"user_id": owner, "team_id": team_id, "decider": "orchestrator",
            "status": "pending", "expires_at": {"$gt": bson::DateTime::now()}},
        )
        .sort(doc! {"created_at": 1})
        .limit(50)
        .await?
        .try_collect()
        .await?)
}

async fn slug_names(
    db: &Database,
    rows: &[AssistantConversation],
) -> AppResult<HashMap<String, String>> {
    let user_services: Vec<String> = rows
        .iter()
        .flat_map(|row| row.grants.service_ids.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let platform: Vec<String> = rows
        .iter()
        .flat_map(|row| row.grants.platform_service_ids.clone())
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

/// Summaries of a team's members for the orchestrator's tools and the UI.
pub async fn summaries(
    db: &Database,
    owner: &str,
    team_id: &str,
    include_destroyed: bool,
    reply_chars: usize,
) -> AppResult<Vec<MemberSummary>> {
    let mut rows = members(db, owner, &[team_id.to_owned()]).await?;
    if !include_destroyed {
        rows.retain(|row| row.destroyed_at.is_none());
    }
    let names = slug_names(db, &rows).await?;
    let requests = pending_requests(db, owner, team_id).await?;
    let now = Utc::now();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let name = row.agent_name.clone().unwrap_or_default();
        out.push(MemberSummary {
            services: row
                .grants
                .service_ids
                .iter()
                .chain(&row.grants.platform_service_ids)
                .map(|id| names.get(id).cloned().unwrap_or_else(|| id.clone()))
                .collect(),
            account_read: row.grants.account_read,
            pending_requests: requests
                .iter()
                .filter(|request| request.conversation_id == row.id)
                .map(|request| request_summary(request, &name))
                .collect(),
            last_reply: if reply_chars == 0 {
                None
            } else {
                last_reply(db, owner, &row.id, reply_chars).await?
            },
            status: status(&row, now),
            charter: row.charter.clone().unwrap_or_default(),
            specialty: row.specialty.clone(),
            created_at: row.created_at,
            last_active_at: row.updated_at,
            id: row.id,
            name,
        });
    }
    Ok(out)
}

/// Recent messages of one member, bounded for the orchestrator's context.
pub async fn read(
    db: &Database,
    owner: &str,
    member: &AssistantConversation,
    limit: i64,
) -> AppResult<Vec<ReplySummary>> {
    let rows = engine::messages(db, owner, &member.id, limit.clamp(1, READ_LIMIT), None).await?;
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

/// Messages the user sent directly to team members since `since`. The
/// orchestrator is not woken by them; it learns about them on its next turn.
pub async fn direct_chats_note(
    db: &Database,
    owner: &str,
    team_id: &str,
    since: DateTime<Utc>,
) -> AppResult<String> {
    let rows = members(db, owner, &[team_id.to_owned()]).await?;
    if rows.is_empty() {
        return Ok(String::new());
    }
    let names: HashMap<&str, &str> = rows
        .iter()
        .map(|row| {
            (
                row.id.as_str(),
                row.agent_name.as_deref().unwrap_or("subagent"),
            )
        })
        .collect();
    let ids: Vec<&str> = names.keys().copied().collect();
    let messages: Vec<AssistantMessage> = db
        .collection::<AssistantMessage>(MESSAGES)
        .find(
            doc! {"user_id": owner, "conversation_id": {"$in": ids}, "role": "user",
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
        "\n\nSince your previous reply the user spoke directly to subagents \
        (quoted; the subagents handle these themselves):",
    );
    for message in messages {
        note.push_str(&format!(
            "\n- to {}: \"{}\"",
            identifier(
                names
                    .get(message.conversation_id.as_str())
                    .unwrap_or(&"subagent")
            ),
            excerpt(&message.text, 300).replace('"', "'")
        ));
    }
    Ok(note)
}

/// A compact roster for the orchestrator's instructions.
pub async fn roster_note(db: &Database, owner: &str, team_id: &str) -> AppResult<String> {
    let rows = summaries(db, owner, team_id, false, 0).await?;
    if rows.is_empty() {
        return Ok(String::new());
    }
    let mut note = String::from("\n\nYour live subagents (NyxID facts):");
    for row in rows.iter().take(16) {
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

/// Remove settled-reply events for the given members from the orchestrator's
/// queue once `wait_for_subagents` has delivered them in-turn.
pub async fn consume_settled_events(
    db: &Database,
    owner: &str,
    team_id: &str,
    member_ids: &[String],
) -> AppResult<()> {
    if member_ids.is_empty() {
        return Ok(());
    }
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .update_one(
            doc! {"_id": team_id, "user_id": owner},
            doc! {"$pull": {"pending_events": {
                "kind": "subagent_settled", "subagent_id": {"$in": member_ids},
            }}},
        )
        .await?;
    Ok(())
}

/// Idle agents with queued events whose wake-up was deferred (full pool or a
/// replica restart). Bounded; loop guards still apply at wake time.
pub async fn queued(db: &Database) -> AppResult<Vec<AssistantConversation>> {
    let rows: Vec<AssistantConversation> = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(doc! {"pending_events.created_at": {"$exists": true},
            "destroyed_at": bson::Bson::Null})
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

/// Subagents with no activity since `cutoff` and no live turn.
pub async fn idle_members(
    db: &Database,
    cutoff: DateTime<Utc>,
) -> AppResult<Vec<AssistantConversation>> {
    let rows: Vec<AssistantConversation> = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(doc! {"role": "subagent", "destroyed_at": bson::Bson::Null,
        "updated_at": {"$lt": bson::DateTime::from_chrono(cutoff)}})
        .limit(200)
        .await?
        .try_collect()
        .await?;
    let now = Utc::now();
    Ok(rows
        .into_iter()
        .filter(|row| live_turn(row, now).is_none())
        .collect())
}

/// Destroy idle subagents. One failure never stops the sweep.
pub async fn sweep_idle(db: &Database) -> AppResult<usize> {
    let cutoff = Utc::now() - chrono::Duration::days(IDLE_DESTROY_DAYS);
    let mut destroyed = 0;
    for row in idle_members(db, cutoff).await? {
        let Some(team_id) = row.team_id.as_deref() else {
            continue;
        };
        if destroy(db, &row.user_id, team_id, &row.id).await.is_ok() {
            destroyed += 1;
        }
    }
    Ok(destroyed)
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
