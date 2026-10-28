//! Personal specialist sharing. This module never confers general owner access.
use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::{AssistantAgent, GuestAccess, MemoryNote},
        assistant_sharing::{
            self as model, AgentShare, AgentSharing, PrivateNotes, ShareRole, ShareSubject,
        },
        feature_flag_override::FeatureFlagOverride,
        org_membership::{OrgMembership, OrgRole},
        user::User,
        user_service::UserService,
    },
};

pub const FLAG: &str = super::feature_flag_service::SHARED_AGENTS_FLAG_KEY;
const MAX_SHARES: usize = 500;

pub fn missing() -> AppError {
    AppError::NotFound("Shared agent not found".into())
}

/// Private fields bind this snapshot to one actor AND one exact agent.
#[derive(Clone)]
pub struct RequestAccess {
    actor: String,
    agent: AssistantAgent,
    policy: AgentSharing,
    role: ShareRole,
    connections: HashMap<String, UserService>,
    model_connections: HashMap<String, UserService>,
}

impl std::fmt::Debug for RequestAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedRequestAccess")
            .field("role", &self.role)
            .field("revision", &self.policy.revision)
            .finish_non_exhaustive()
    }
}

impl RequestAccess {
    pub fn actor(&self) -> &str {
        &self.actor
    }
    pub fn owner(&self) -> &str {
        &self.agent.user_id
    }
    pub fn agent_id(&self) -> &str {
        &self.agent.id
    }
    pub fn role(&self) -> ShareRole {
        self.role
    }
    pub fn revision(&self) -> i64 {
        self.policy.revision
    }
    pub fn version(&self) -> i64 {
        self.policy.version
    }
    pub fn matches(&self, actor: &str, owner: &str, agent: &str) -> bool {
        self.actor == actor && self.owner() == owner && self.agent_id() == agent
    }
    pub fn level(&self, service: &str) -> Option<GuestAccess> {
        if let Some(connection) = self.connections.get(service) {
            return owner_grant(&self.agent, connection)
                .and_then(|id| self.policy.shared_access.get(id).copied());
        }
        self.agent
            .grants
            .service_ids
            .iter()
            .chain(&self.agent.grants.platform_service_ids)
            .any(|id| id == service)
            .then(|| self.policy.shared_access.get(service).copied())
            .flatten()
    }
    pub fn connections(&self) -> &HashMap<String, UserService> {
        &self.connections
    }
    /// Additive discovery metadata; listing a candidate is never consent.
    pub fn discovery_metadata(&self, id: &str) -> Option<serde_json::Value> {
        let service = self.connections.get(id)?;
        let personal = service.user_id == self.actor;
        Some(serde_json::json!({
            "credential_source": if personal { "actor_personal" } else { "owner_shared" },
            "consent_required": personal, "connection_id": service.id,
            "connection_name": service.slug, "credential_owner_id": service.user_id,
            "catalog_service_id": service.catalog_service_id,
        }))
    }
    pub fn allows_connection(&self, service: &UserService) -> bool {
        self.connections
            .get(&service.id)
            .is_some_and(|s| s.user_id == service.user_id)
            && service.is_active
            && server_service(service)
    }
    pub fn allows_model_connection(&self, service: &UserService) -> bool {
        service.user_id == self.actor
            && service.is_active
            && server_service(service)
            && self.model_connections.contains_key(&service.id)
    }
    pub fn allows_resolution(&self, service: &UserService) -> bool {
        self.allows_connection(service) || self.allows_model_connection(service)
    }
    pub fn catalog_connections(&self, catalog: &str, include_models: bool) -> Vec<UserService> {
        let mut candidates: Vec<_> = self
            .connections
            .values()
            .chain(self.model_connections.values().filter(|_| include_models))
            .filter(|service| service.catalog_service_id.as_deref() == Some(catalog))
            .cloned()
            .collect();
        candidates.sort_by(|a, b| a.id.cmp(&b.id));
        candidates.dedup_by(|a, b| a.id == b.id);
        candidates
    }
    pub fn connection_for_slug(&self, slug: &str) -> AppResult<&UserService> {
        let mut candidates: Vec<_> = self
            .connections
            .values()
            .chain(self.model_connections.values())
            .filter(|service| service.slug == slug)
            .collect();
        candidates.sort_by(|a, b| a.id.cmp(&b.id));
        candidates.dedup_by(|a, b| a.id == b.id);
        match candidates.as_slice() {
            [service] => Ok(*service),
            [] => Err(missing()),
            _ => Err(AppError::Conflict(
                "More than one shared connection has this slug; select an exact connection ID"
                    .into(),
            )),
        }
    }
    /// Same-catalog actor alternatives inherit the agent's operation bounds.
    /// Multiple scopes intersect conservatively; different input contracts
    /// cannot silently widen an alternative connection.
    pub fn operation_scopes(&self) -> crate::models::agent_operation_scope::OperationScopes {
        let mut result = self.agent.operation_scopes.clone();
        let mut catalogs = crate::models::agent_operation_scope::OperationScopes::new();
        for scope in self.agent.operation_scopes.values() {
            let Some(catalog) = &scope.catalog_service_id else {
                continue;
            };
            catalogs
                .entry(catalog.clone())
                .and_modify(|existing| {
                    existing
                        .operations
                        .retain(|operation| scope.operations.contains(operation));
                    existing.revision = existing.revision.max(scope.revision);
                })
                .or_insert_with(|| scope.clone());
        }
        for (catalog, scope) in catalogs {
            result.insert(catalog, scope);
        }
        result
    }
    pub fn agent(&self) -> &AssistantAgent {
        &self.agent
    }
    pub fn team_memory(&self) -> &[MemoryNote] {
        &self.policy.memory
    }
}

/// Thin #1996 adapter. No new value is serialized into GuestAccess; None is withheld.
pub fn permits(level: Option<GuestAccess>, effects: super::mcp_service::OperationEffects) -> bool {
    match level {
        None => false,
        Some(GuestAccess::Read) => effects.reads && effects.uses,
        Some(GuestAccess::Use) => effects.uses,
        Some(GuestAccess::All) => true,
    }
}

pub fn server_service(service: &UserService) -> bool {
    service.service_type == "http"
        && service.node_id.is_none()
        && !service.forward_access_token
        && !service.inject_delegation_token
        && service.identity_propagation_mode == "none"
}

/// Catalog grants select only platform-bound owner instances, never arbitrary
/// owner BYOK connections for that catalog.
pub fn owner_grant<'a>(agent: &'a AssistantAgent, service: &'a UserService) -> Option<&'a str> {
    if service.user_id != agent.user_id {
        return None;
    }
    if agent.grants.service_ids.contains(&service.id) {
        return Some(&service.id);
    }
    service.catalog_service_id.as_deref().filter(|id| {
        service.credential_binding.as_deref() == Some("platform")
            && agent
                .grants
                .platform_service_ids
                .iter()
                .any(|granted| granted.as_str() == *id)
    })
}

/// The count includes every query issued by the batch loader (no hidden I/O in
/// predicates). Tests assert it is constant as agent/audience counts grow.
pub struct AccessBatch {
    pub agents: HashMap<String, RequestAccess>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub read_queries: usize,
}

fn flag_enabled(
    rows: &[FeatureFlagOverride],
    memberships: &[OrgMembership],
    active: &HashSet<String>,
    actor: &str,
) -> bool {
    let (org, platform): (Vec<_>, Vec<_>) =
        rows.iter().cloned().partition(|r| r.org_user_id.is_some());
    let roles: Vec<(String, OrgRole)> = memberships
        .iter()
        .filter(|m| m.member_user_id == actor && m.is_active() && active.contains(&m.org_user_id))
        .map(|m| (m.org_user_id.clone(), m.role))
        .collect();
    super::feature_flag_service::resolve_personal_definitions(
        [(FLAG, false, None)],
        &platform,
        &org,
        &roles,
        actor,
    )
    .iter()
    .any(|key| key == FLAG)
}

pub async fn load(
    db: &Database,
    actor: &str,
    agent_ids: Option<&[String]>,
) -> AppResult<AccessBatch> {
    let mut reads = 0;
    if agent_ids.is_some_and(<[String]>::is_empty) {
        return Ok(AccessBatch {
            agents: HashMap::new(),
            read_queries: reads,
        });
    }
    let mut memberships: Vec<OrgMembership> = db
        .collection(crate::models::org_membership::COLLECTION_NAME)
        .find(doc! {"member_user_id": actor, "revoked_at": null})
        .await?
        .try_collect()
        .await?;
    reads += 1;
    let orgs: Vec<_> = memberships
        .iter()
        .filter(|m| m.role.can_proxy())
        .map(|m| &m.org_user_id)
        .collect();
    let mut filter = doc! {"$or": [
        {"subject_kind": "person", "subject_id": actor},
        {"subject_kind": "org", "subject_id": {"$in": orgs}}
    ]};
    if let Some(ids) = agent_ids {
        filter.insert("agent_id", doc! {"$in": ids});
    }
    let shares: Vec<AgentShare> = db
        .collection(model::SHARES_COLLECTION_NAME)
        .find(filter)
        .limit(1001)
        .await?
        .try_collect()
        .await?;
    reads += 1;
    if shares.len() > 1000 {
        return Err(AppError::ValidationError(
            "Too many shared agents; select an agent".into(),
        ));
    }
    if shares.is_empty() {
        return Ok(AccessBatch {
            agents: HashMap::new(),
            read_queries: reads,
        });
    }
    let ids: Vec<_> = shares.iter().map(|s| &s.agent_id).collect();
    let agents: Vec<AssistantAgent> = db
        .collection(crate::models::assistant_agent::COLLECTION_NAME)
        .find(doc! {"_id": {"$in": &ids}, "kind": "specialist", "destroyed_at": null})
        .await?
        .try_collect()
        .await?;
    let policies: Vec<AgentSharing> = db
        .collection(model::COLLECTION_NAME)
        .find(doc! {"_id": {"$in": &ids}, "enabled": true})
        .await?
        .try_collect()
        .await?;
    reads += 2;
    let owners: Vec<_> = agents.iter().map(|a| a.user_id.clone()).collect();
    let owner_memberships: Vec<OrgMembership> = db
        .collection(crate::models::org_membership::COLLECTION_NAME)
        .find(doc! {"member_user_id": {"$in": &owners}, "revoked_at": null})
        .await?
        .try_collect()
        .await?;
    reads += 1;
    memberships.extend(owner_memberships);
    let mut users: HashSet<String> = owners.into_iter().collect();
    users.insert(actor.into());
    users.extend(memberships.iter().map(|m| m.org_user_id.clone()));
    let users: Vec<User> = db
        .collection(crate::models::user::COLLECTION_NAME)
        .find(doc! {"_id": {"$in": users.into_iter().collect::<Vec<_>>()}, "is_active": true})
        .await?
        .try_collect()
        .await?;
    reads += 1;
    let active_orgs: HashSet<_> = users
        .iter()
        .filter(|u| u.user_type.is_org())
        .map(|u| u.id.clone())
        .collect();
    let people: HashSet<_> = users
        .iter()
        .filter(|u| u.user_type.is_person())
        .map(|u| u.id.clone())
        .collect();
    let mut scopes = vec![bson::Bson::Null];
    scopes.extend(active_orgs.iter().cloned().map(bson::Bson::String));
    let flags: Vec<FeatureFlagOverride> = db
        .collection("feature_flag_overrides")
        .find(doc! {"flag_key": FLAG, "definition_id": null, "org_user_id": {"$in": scopes}})
        .await?
        .try_collect()
        .await?;
    reads += 1;
    let mut result = HashMap::new();
    if !people.contains(actor) || !flag_enabled(&flags, &memberships, &active_orgs, actor) {
        return Ok(AccessBatch {
            agents: result,
            read_queries: reads,
        });
    }
    let granted_ids: Vec<_> = agents
        .iter()
        .flat_map(|a| a.grants.service_ids.iter())
        .collect();
    let platform_ids: Vec<_> = agents
        .iter()
        .flat_map(|a| &a.grants.platform_service_ids)
        .collect();
    let owner_ids: Vec<_> = agents.iter().map(|a| &a.user_id).collect();
    let owner_services: Vec<UserService> = db
        .collection(crate::models::user_service::COLLECTION_NAME)
        .find(doc! {"is_active": true, "$or": [{"_id": {"$in": granted_ids}},
            {"user_id": {"$in": owner_ids}, "catalog_service_id": {"$in": platform_ids}, "credential_binding": "platform"}]})
        .await?
        .try_collect()
        .await?;
    let mut catalogs: HashSet<String> = owner_services
        .iter()
        .filter_map(|s| s.catalog_service_id.clone())
        .collect();
    catalogs.extend(
        agents
            .iter()
            .flat_map(|a| a.grants.platform_service_ids.iter().cloned()),
    );
    let catalog_rows: Vec<bson::Document> = db
        .collection("downstream_services")
        .find(doc! {"is_active": true, "$or": [{"_id": {"$in": catalogs.iter().collect::<Vec<_>>()}}, {"inference": {"$ne": null}}]})
        .projection(doc! {"_id": 1, "inference": 1, "service_category": 1, "service_type": 1})
        .await?
        .try_collect()
        .await?;
    reads += 1;
    let callable_catalogs: HashSet<String> = catalog_rows
        .iter()
        .filter(|d| {
            d.get_str("service_category") != Ok("internal")
                && d.get_str("service_type").unwrap_or("http") == "http"
        })
        .filter_map(|d| d.get_str("_id").ok().map(str::to_owned))
        .collect();
    let model_catalogs: HashSet<String> = catalog_rows
        .iter()
        .filter(|d| {
            d.get("inference")
                .is_some_and(|v| !matches!(v, bson::Bson::Null))
        })
        .filter_map(|d| d.get_str("_id").ok().map(str::to_owned))
        .collect();
    catalogs.extend(model_catalogs.iter().cloned());
    let actor_services: Vec<UserService> = db.collection(crate::models::user_service::COLLECTION_NAME)
        .find(doc! {"user_id": actor, "is_active": true, "catalog_service_id": {"$in": catalogs.into_iter().collect::<Vec<_>>()}})
        .await?.try_collect().await?;
    reads += 2;
    let policies: HashMap<_, _> = policies
        .into_iter()
        .map(|p| (p.agent_id.clone(), p))
        .collect();
    for agent in agents {
        if agent.user_id == actor
            || !people.contains(&agent.user_id)
            || !flag_enabled(&flags, &memberships, &active_orgs, &agent.user_id)
        {
            continue;
        }
        let Some(policy) = policies
            .get(&agent.id)
            .filter(|p| p.owner_id == agent.user_id)
        else {
            continue;
        };
        let roles: Vec<_> = shares
            .iter()
            .filter(|s| s.agent_id == agent.id && s.owner_id == agent.user_id)
            .filter(|s| match s.subject_kind {
                ShareSubject::Person => s.subject_id == actor,
                ShareSubject::Org => {
                    s.role == ShareRole::User
                        && active_orgs.contains(&s.subject_id)
                        && memberships.iter().any(|m| {
                            m.member_user_id == actor
                                && m.org_user_id == s.subject_id
                                && m.role.can_proxy()
                        })
                }
            })
            .map(|s| s.role)
            .collect();
        if roles.is_empty() {
            continue;
        }
        let role = if roles.contains(&ShareRole::Manager) {
            ShareRole::Manager
        } else {
            ShareRole::User
        };
        let agent_services: Vec<_> = owner_services
            .iter()
            .filter(|s| owner_grant(&agent, s).is_some())
            .filter(|s| {
                s.catalog_service_id
                    .as_ref()
                    .is_none_or(|id| callable_catalogs.contains(id))
            })
            .collect();
        let catalogs: HashSet<_> = agent_services
            .iter()
            .filter_map(|s| s.catalog_service_id.as_deref())
            .chain(agent.grants.platform_service_ids.iter().map(String::as_str))
            .collect();
        let connections = agent_services
            .into_iter()
            .filter(|s| {
                server_service(s)
                    && owner_grant(&agent, s)
                        .is_some_and(|id| policy.shared_access.contains_key(id))
            })
            .chain(actor_services.iter().filter(|s| {
                server_service(s)
                    && s.catalog_service_id
                        .as_deref()
                        .is_some_and(|c| catalogs.contains(c) && callable_catalogs.contains(c))
            }))
            .map(|s| (s.id.clone(), s.clone()))
            .collect();
        let model_connections = actor_services
            .iter()
            .filter(|s| {
                server_service(s)
                    && s.catalog_service_id
                        .as_ref()
                        .is_some_and(|id| model_catalogs.contains(id))
            })
            .map(|s| (s.id.clone(), s.clone()))
            .collect();
        result.insert(
            agent.id.clone(),
            RequestAccess {
                actor: actor.into(),
                agent,
                policy: policy.clone(),
                role,
                connections,
                model_connections,
            },
        );
    }
    Ok(AccessBatch {
        agents: result,
        read_queries: reads,
    })
}

pub async fn require(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
) -> AppResult<RequestAccess> {
    let mut batch = load(db, actor, Some(std::slice::from_ref(&agent.id))).await?;
    batch.agents.remove(&agent.id).ok_or_else(missing)
}

pub async fn personal_owner(db: &Database, actor: &str, id: &str) -> AppResult<AssistantAgent> {
    let agent = db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(doc! {"_id": id, "user_id": actor, "kind": "specialist", "destroyed_at": null})
        .await?
        .ok_or_else(missing)?;
    let owner = db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": actor, "is_active": true})
        .await?
        .ok_or_else(missing)?;
    if !owner.user_type.is_person() {
        return Err(missing());
    }
    Ok(agent)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Audience {
    pub subject_kind: ShareSubject,
    pub subject_id: String,
    pub role: ShareRole,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharingUpdate {
    pub expected_version: i64,
    pub enabled: bool,
    pub audience: Vec<Audience>,
    pub shared_access: BTreeMap<String, GuestAccess>,
}

fn role_rank(role: ShareRole) -> u8 {
    match role {
        ShareRole::User => 0,
        ShareRole::Manager => 1,
    }
}

fn role_name(role: ShareRole) -> &'static str {
    match role {
        ShareRole::User => "user",
        ShareRole::Manager => "manager",
    }
}

fn access_rank(level: GuestAccess) -> u8 {
    match level {
        GuestAccess::Read => 0,
        GuestAccess::Use => 1,
        GuestAccess::All => 2,
    }
}

fn access_name(level: GuestAccess) -> &'static str {
    match level {
        GuestAccess::Read => "read",
        GuestAccess::Use => "use",
        GuestAccess::All => "all",
    }
}

/// Whether a proposed policy grants any new sharing authority. This is kept
/// pure so every owner-card path can test the exact same widening semantics.
pub fn sharing_widens(
    enabled: bool,
    audience: &[Audience],
    shared_access: &BTreeMap<String, GuestAccess>,
    input: &SharingUpdate,
) -> bool {
    (!enabled && input.enabled)
        || input.audience.iter().any(|wanted| {
            audience
                .iter()
                .find(|existing| {
                    existing.subject_kind == wanted.subject_kind
                        && existing.subject_id == wanted.subject_id
                })
                .is_none_or(|existing| role_rank(wanted.role) > role_rank(existing.role))
        })
        || input.shared_access.iter().any(|(service, level)| {
            shared_access
                .get(service)
                .is_none_or(|old| access_rank(*level) > access_rank(*old))
        })
}

#[derive(Clone, Debug)]
pub struct SharingPreview {
    pub widening: bool,
    pub summary: String,
}

/// Read the current policy and audience for the owner-card preview. The
/// proposed `SharingUpdate` is also the card's argument digest, so a later
/// version change cannot be approved accidentally.
pub async fn preview_update(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
    input: &SharingUpdate,
) -> AppResult<SharingPreview> {
    personal_owner(db, actor, &agent.id).await?;
    let current = policy(db, agent).await?;
    let rows: Vec<AgentShare> = db
        .collection(model::SHARES_COLLECTION_NAME)
        .find(doc! {"agent_id": &agent.id, "owner_id": actor})
        .limit(MAX_SHARES as i64)
        .await?
        .try_collect()
        .await?;
    let current_audience: Vec<Audience> = rows
        .iter()
        .map(|row| Audience {
            subject_kind: row.subject_kind,
            subject_id: row.subject_id.clone(),
            role: row.role,
        })
        .collect();
    let widening = sharing_widens(
        current.enabled,
        &current_audience,
        &current.shared_access,
        input,
    );

    let subject_gains: Vec<&Audience> = input.audience.iter().filter(|_| widening).collect();
    let service_gains: Vec<(&String, &GuestAccess)> =
        input.shared_access.iter().filter(|_| widening).collect();

    let subject_ids: Vec<_> = subject_gains
        .iter()
        .map(|subject| subject.subject_id.clone())
        .collect();
    let users: Vec<User> = if subject_ids.is_empty() {
        Vec::new()
    } else {
        db.collection(crate::models::user::COLLECTION_NAME)
            .find(doc! {"_id": {"$in": &subject_ids}})
            .limit(MAX_SHARES as i64)
            .await?
            .try_collect()
            .await?
    };
    let subject_label = |subject: &Audience| {
        users
            .iter()
            .find(|user| user.id == subject.subject_id)
            .map(|user| {
                user.display_name
                    .as_deref()
                    .or(user.slug.as_deref())
                    .unwrap_or(&user.email)
                    .to_owned()
            })
            .unwrap_or_else(|| subject.subject_id.clone())
    };
    let subjects = subject_gains
        .iter()
        .map(|subject| format!("{} ({})", subject_label(subject), role_name(subject.role)))
        .collect::<Vec<_>>();

    let service_ids: Vec<_> = service_gains
        .iter()
        .map(|(service, _)| (*service).clone())
        .collect();
    let owner_services: Vec<UserService> = if service_ids.is_empty() {
        Vec::new()
    } else {
        db.collection(crate::models::user_service::COLLECTION_NAME)
            .find(doc! {"user_id": &agent.user_id, "_id": {"$in": &service_ids}})
            .await?
            .try_collect()
            .await?
    };
    let catalogs: Vec<crate::models::downstream_service::DownstreamService> =
        if service_ids.is_empty() {
            Vec::new()
        } else {
            db.collection(crate::models::downstream_service::COLLECTION_NAME)
                .find(doc! {"_id": {"$in": &service_ids}})
                .await?
                .try_collect()
                .await?
        };
    let service_label = |service: &str| {
        owner_services
            .iter()
            .find(|row| row.id == service || row.catalog_service_id.as_deref() == Some(service))
            .map(|row| row.slug.clone())
            .or_else(|| {
                catalogs
                    .iter()
                    .find(|row| row.id == service)
                    .map(|row| row.name.clone())
            })
            .unwrap_or_else(|| service.to_owned())
    };
    let access = service_gains
        .iter()
        .map(|(service, level)| format!("{} ({})", service_label(service), access_name(**level)))
        .collect::<Vec<_>>();
    let agent_label = agent.display_name.as_deref().unwrap_or(&agent.name);
    let mut details = Vec::new();
    if !subjects.is_empty() {
        details.push(format!("people/orgs: {}", subjects.join(", ")));
    }
    if !access.is_empty() {
        details.push(format!("owner services: {}", access.join(", ")));
    }
    if !current.enabled && input.enabled {
        details.push("sharing enabled".into());
    }
    let summary = if details.is_empty() {
        format!("Change sharing for {agent_label}")
    } else {
        format!("Delegate {agent_label}: {}", details.join("; "))
    };
    Ok(SharingPreview { widening, summary })
}

pub async fn policy(db: &Database, agent: &AssistantAgent) -> AppResult<AgentSharing> {
    Ok(db
        .collection::<AgentSharing>(model::COLLECTION_NAME)
        .find_one(doc! {"_id": &agent.id})
        .await?
        .unwrap_or_else(|| AgentSharing {
            agent_id: agent.id.clone(),
            owner_id: agent.user_id.clone(),
            enabled: false,
            version: 0,
            revision: 0,
            shared_access: BTreeMap::new(),
            memory: Vec::new(),
            updated_at: Utc::now(),
        }))
}

pub async fn audit(db: &Database, actor: &str, agent: &AssistantAgent, kind: &str) {
    let _ = super::audit_service::log_actor_event(
        db.clone(),
        &super::audit_service::AuditActor {
            user_id: actor.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        "assistant_sharing_changed",
        Some(serde_json::json!({"agent_id": agent.id,
        "agent_owner_id": agent.user_id, "mode": "shared", "change_kind": kind})),
    )
    .await;
}

/// Serialize new key admission with policy removal/widening, without retaining a snapshot across requests.
pub async fn fence(
    db: &Database,
    access: &RequestAccess,
    session: &mut ClientSession,
) -> AppResult<()> {
    let changed = db.collection::<AgentSharing>(model::COLLECTION_NAME)
        .update_one(doc! {"_id": access.agent_id(), "owner_id": access.owner(), "enabled": true, "revision": access.revision()},
            doc! {"$inc": {"execution_fence": 1}}).session(session).await?;
    if changed.matched_count != 1 {
        return Err(missing());
    }
    Ok(())
}

pub async fn update(
    db: &Database,
    actor: &str,
    id: &str,
    input: SharingUpdate,
) -> AppResult<AgentSharing> {
    let agent = personal_owner(db, actor, id).await?;
    if input.enabled && !super::feature_flag_service::personal_flag_enabled(db, actor, FLAG).await?
    {
        return Err(AppError::Forbidden("Shared agents are not enabled".into()));
    }
    if input.audience.len() > MAX_SHARES || input.expected_version < 0 {
        return Err(AppError::ValidationError(
            "Invalid sharing version or audience size".into(),
        ));
    }
    let mut unique = HashSet::new();
    for subject in &input.audience {
        if Uuid::parse_str(&subject.subject_id).is_err()
            || subject.subject_id == actor
            || !unique.insert(subject.subject_id.clone())
            || (subject.subject_kind == ShareSubject::Org && subject.role != ShareRole::User)
        {
            return Err(AppError::ValidationError("Invalid sharing audience".into()));
        }
    }
    let subjects: Vec<User> = db
        .collection(crate::models::user::COLLECTION_NAME)
        .find(doc! {"_id": {"$in": unique.into_iter().collect::<Vec<_>>()}, "is_active": true})
        .await?
        .try_collect()
        .await?;
    if input.audience.iter().any(|s| {
        !subjects.iter().any(|u| {
            u.id == s.subject_id
                && match s.subject_kind {
                    ShareSubject::Person => u.user_type.is_person(),
                    ShareSubject::Org => u.user_type.is_org(),
                }
        })
    }) {
        return Err(AppError::ValidationError(
            "Sharing audience is unavailable".into(),
        ));
    }
    let granted: HashSet<_> = agent
        .grants
        .service_ids
        .iter()
        .chain(&agent.grants.platform_service_ids)
        .collect();
    if input.shared_access.keys().any(|id| !granted.contains(id)) {
        return Err(AppError::ValidationError(
            "Only granted services may be shared".into(),
        ));
    }
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    let collection = db.collection::<AgentSharing>(model::COLLECTION_NAME);
    let previous = collection
        .find_one(doc! {"_id": id})
        .session(&mut session)
        .await?;
    let shares = db.collection::<AgentShare>(model::SHARES_COLLECTION_NAME);
    let mut share_cursor = shares
        .find(doc! {"agent_id": id, "owner_id": actor})
        .session(&mut session)
        .await?;
    let previous_shares: Vec<AgentShare> = share_cursor.stream(&mut session).try_collect().await?;
    let current_version = previous.as_ref().map_or(0, |p| p.version);
    if current_version != input.expected_version {
        return Err(AppError::Conflict(
            "Sharing changed; reload before saving".into(),
        ));
    }
    let current_enabled = previous.as_ref().is_some_and(|p| p.enabled);
    let same_audience = previous_shares.len() == input.audience.len()
        && input.audience.iter().all(|wanted| {
            previous_shares.iter().any(|existing| {
                existing.subject_kind == wanted.subject_kind
                    && existing.subject_id == wanted.subject_id
                    && existing.role == wanted.role
            })
        });
    let current_access = previous
        .as_ref()
        .map_or_else(BTreeMap::new, |p| p.shared_access.clone());
    let is_noop =
        current_enabled == input.enabled && same_audience && current_access == input.shared_access;
    let current_policy = previous.clone().unwrap_or_else(|| AgentSharing {
        agent_id: id.into(),
        owner_id: actor.into(),
        enabled: false,
        version: 0,
        revision: 0,
        shared_access: BTreeMap::new(),
        memory: Vec::new(),
        updated_at: Utc::now(),
    });
    if is_noop {
        // A no-op save must not rewrite the policy, fence a thread, revoke a
        // consent, stop a turn, or reset a live NyxAgent session.
        let current_agent = db
            .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .find_one(doc! {
                "_id": id,
                "user_id": actor,
                "destroyed_at": null,
                "updated_at": bson::DateTime::from_chrono(agent.updated_at),
            })
            .session(&mut session)
            .await?;
        if current_agent.is_none() {
            return Err(AppError::Conflict(
                "Agent changed; reload before sharing".into(),
            ));
        }
        session.commit_transaction().await?;
        return Ok(current_policy);
    }

    // Additive audience and access changes leave existing recipients' exact
    // policy revision, consents, turns and sessions valid. Any removal,
    // downgrade or changed/withheld service authority invalidates them.
    let authority_changed = previous.as_ref().is_some_and(|old| {
        (old.enabled && !input.enabled)
            || previous_shares.iter().any(|existing| {
                !input.audience.iter().any(|wanted| {
                    wanted.subject_kind == existing.subject_kind
                        && wanted.subject_id == existing.subject_id
                        && wanted.role == existing.role
                })
            })
            || old
                .shared_access
                .iter()
                .any(|(service, level)| input.shared_access.get(service) != Some(level))
    });
    let version = input.expected_version.checked_add(1).ok_or_else(missing)?;
    let revision = if previous.is_none() || authority_changed || (!current_enabled && input.enabled)
    {
        current_policy.revision.checked_add(1).ok_or_else(missing)?
    } else {
        current_policy.revision
    };

    if authority_changed {
        let fenced = db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .update_one(doc! {"_id": id, "user_id": actor, "destroyed_at": null, "updated_at": bson::DateTime::from_chrono(agent.updated_at)},
                doc! {"$inc": {"thread_fence": 1}}).session(&mut session).await?;
        if fenced.matched_count != 1 {
            return Err(AppError::Conflict(
                "Agent changed; reload before sharing".into(),
            ));
        }
    }
    let policy = AgentSharing {
        agent_id: id.into(),
        owner_id: actor.into(),
        enabled: input.enabled,
        version,
        revision,
        shared_access: input.shared_access,
        memory: current_policy.memory,
        updated_at: Utc::now(),
    };
    let mut policy_filter = doc! {"_id": id};
    if previous.is_some() {
        // Legacy policy rows predate `version`; their serde-defaulted zero is
        // still the first write version and must participate in the CAS.
        policy_filter.insert(
            "$or",
            vec![
                doc! {"version": input.expected_version},
                doc! {"version": {"$exists": false}},
            ],
        );
    }
    let policy_write = collection
        .update_one(
            policy_filter,
            doc! {
                "$set": {
                    "owner_id": &policy.owner_id,
                    "enabled": policy.enabled,
                    "version": policy.version,
                    "revision": policy.revision,
                    "shared_access": bson::to_bson(&policy.shared_access).map_err(|_| AppError::Internal("Sharing serialization failed".into()))?,
                    "memory": bson::to_bson(&policy.memory).map_err(|_| AppError::Internal("Memory serialization failed".into()))?,
                    "updated_at": bson::DateTime::from_chrono(policy.updated_at),
                },
                "$setOnInsert": {"_id": id},
            },
        )
        .upsert(previous.is_none())
        .session(&mut session)
        .await?;
    if previous.is_some() && policy_write.matched_count != 1 {
        return Err(AppError::Conflict(
            "Sharing changed; reload before saving".into(),
        ));
    }
    shares
        .delete_many(doc! {"agent_id": id})
        .session(&mut session)
        .await?;
    for subject in input.audience {
        shares
            .insert_one(AgentShare {
                id: Uuid::new_v4().to_string(),
                agent_id: id.into(),
                owner_id: actor.into(),
                subject_kind: subject.subject_kind,
                subject_id: subject.subject_id,
                role: subject.role,
                created_at: Utc::now(),
            })
            .session(&mut session)
            .await?;
    }
    if authority_changed {
        // Narrowing or changing authority invalidates every consent. Late tool
        // requests still resolve live ACLs, even before stopped turns settle.
        db.collection::<bson::Document>(model::CONSENTS_COLLECTION_NAME)
            .update_many(
                doc! {"agent_id": id},
                doc! {"$set": {"decision": "revoked"}},
            )
            .session(&mut session)
            .await?;
        db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_many(
                doc! {"agent_id": id, "user_id": {"$ne": actor}, "active_turn": {"$ne": null}},
                doc! {"$set": {"active_turn.stop_requested": true}},
            )
            .session(&mut session)
            .await?;
        db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_many(doc! {"agent_id": id, "shared_agent_thread": true}, doc! {"$set": {
                "nyxagent_session_id": null, "nyxagent_instruction_binding": null, "nyxagent_last_response_id": null, "pending_events": []}})
            .session(&mut session).await?;
    }
    session.commit_transaction().await?;
    audit(db, actor, &agent, "policy").await;
    Ok(policy)
}

pub async fn private_notes(db: &Database, actor: &str, agent: &str) -> AppResult<Vec<MemoryNote>> {
    Ok(db
        .collection::<PrivateNotes>(model::NOTES_COLLECTION_NAME)
        .find_one(doc! {"agent_id": agent, "actor_user_id": actor})
        .await?
        .map_or_else(Vec::new, |r| r.notes))
}

/// Native remember/forget on a shared private thread never edits owner or team memory.
pub async fn edit_private_note(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
    text: Option<&str>,
    note_id: Option<&str>,
) -> AppResult<String> {
    let access = require(db, actor, agent).await?;
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    fence(db, &access, &mut session).await?;
    let collection = db.collection::<PrivateNotes>(model::NOTES_COLLECTION_NAME);
    let filter = doc! {"agent_id": &agent.id, "actor_user_id": actor};
    let mut row = collection
        .find_one(filter.clone())
        .session(&mut session)
        .await?
        .unwrap_or_else(|| PrivateNotes {
            id: Uuid::new_v4().to_string(),
            agent_id: agent.id.clone(),
            actor_user_id: actor.into(),
            revision: 0,
            notes: Vec::new(),
            updated_at: Utc::now(),
        });
    let id = note_id
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    if note_id.is_some() && !row.notes.iter().any(|n| n.id == id) {
        return Err(missing());
    }
    if let Some(text) = text {
        let mut note = notes(vec![text.into()])?.remove(0);
        note.id.clone_from(&id);
        if let Some(existing) = row.notes.iter_mut().find(|n| n.id == id) {
            *existing = note;
        } else if row.notes.len() < crate::models::assistant_agent::MAX_MEMORY_NOTES {
            row.notes.push(note);
        } else {
            return Err(AppError::Conflict(
                "Private notes are full; replace or forget a note".into(),
            ));
        }
    } else {
        row.notes.retain(|n| n.id != id);
    }
    row.revision = row.revision.checked_add(1).ok_or_else(missing)?;
    row.updated_at = Utc::now();
    collection
        .replace_one(filter, &row)
        .upsert(true)
        .session(&mut session)
        .await?;
    session.commit_transaction().await?;
    audit(db, actor, agent, "private_note").await;
    Ok(id)
}

/// Human/tool discovery is explicit; ordinary personal turn roster construction
/// does not call this loader and retains its original read budget.
pub async fn summaries(
    db: &Database,
    actor: &str,
) -> AppResult<Vec<super::assistant_team_service::AgentSummary>> {
    use crate::models::assistant_conversation::AssistantConversation;
    let batch = load(db, actor, None).await?;
    let threads: Vec<AssistantConversation> = db
        .collection(crate::models::assistant_conversation::COLLECTION_NAME)
        .find(
            doc! {"user_id": actor, "shared_agent_thread": true, "group_id": null, "channel": null,
            "automation_thread": {"$ne": true}, "voice_parent_conversation_id": null},
        )
        .sort(doc! {"created_at": 1})
        .await?
        .try_collect()
        .await?;
    let mut result = Vec::new();
    for access in batch.agents.into_values() {
        let a = access.agent();
        let home = threads
            .iter()
            .find(|r| r.agent_id.as_deref() == Some(&a.id));
        result.push(super::assistant_team_service::AgentSummary {
            share_role: Some(access.role()),
            owner_id: a.user_id.clone(),
            owner_name: None,
            owner_slug: None,
            owner_kind: "person",
            org_role: None,
            can_maintain: false,
            can_use: true,
            machines: Vec::new(),
            logins: Vec::new(),
            services: access.connections.keys().cloned().collect(),
            account_read: false,
            guest_access: BTreeMap::new(),
            pending_requests: Vec::new(),
            last_reply: None,
            status: if threads.iter().any(|r| {
                r.agent_id.as_deref() == Some(&a.id)
                    && super::assistant_nyxagent::live_turn(r, Utc::now()).is_some()
            }) {
                "running"
            } else {
                "idle"
            },
            kind: a.kind,
            name: a.name.clone(),
            display_name: a.display_name.clone(),
            persona: a.persona.clone(),
            description: a.description.clone(),
            specialty: a.specialty.clone(),
            created_by: a.created_by.clone(),
            model: a.model.clone(),
            access_mode_override: None,
            channel_checking_notice: false,
            home_conversation_id: home.map(|r| r.id.clone()),
            memory_count: access.team_memory().len(),
            created_at: a.created_at,
            last_active_at: home.map_or(a.updated_at, |r| r.updated_at),
            destroyed_at: None,
            id: a.id.clone(),
        });
    }
    for thread in &threads {
        let Some(id) = thread.agent_id.as_deref() else {
            continue;
        };
        if result.iter().any(|a| a.id == id) {
            continue;
        }
        result.push(super::assistant_team_service::AgentSummary {
            id: id.into(), share_role: Some(ShareRole::User), owner_id: thread.agent_owner_id.clone().unwrap_or_default(),
            owner_name: None, owner_slug: None, owner_kind: "person", org_role: None, can_use: false, can_maintain: false,
            machines: Vec::new(), logins: Vec::new(), services: Vec::new(), account_read: false,
            guest_access: BTreeMap::new(), pending_requests: Vec::new(), last_reply: None, status: "idle",
            kind: crate::models::assistant_agent::AgentKind::Specialist, name: "Shared agent".into(),
            display_name: Some("Shared agent · access ended".into()), persona: None, specialty: None,
            description: "Your retained chats remain private and readable. Ask the owner to share this agent again to continue.".into(),
            created_by: "user".into(), model: thread.model.clone(), access_mode_override: None, channel_checking_notice: false,
            home_conversation_id: Some(thread.id.clone()), memory_count: 0, created_at: thread.created_at,
            last_active_at: thread.updated_at, destroyed_at: None,
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    Ok(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotesUpdate {
    pub expected_revision: i64,
    pub notes: Vec<String>,
}

fn notes(input: Vec<String>) -> AppResult<Vec<MemoryNote>> {
    use crate::models::assistant_agent::{MAX_MEMORY_NOTE_CHARS, MAX_MEMORY_NOTES};
    if input.len() > MAX_MEMORY_NOTES
        || input.iter().any(|s| {
            s.trim().is_empty()
                || s.chars().count() > MAX_MEMORY_NOTE_CHARS
                || s.chars().any(|c| c.is_control() && c != '\n')
        })
    {
        return Err(AppError::ValidationError(
            "Notes exceed the memory limits".into(),
        ));
    }
    Ok(input
        .into_iter()
        .map(|text| MemoryNote {
            id: Uuid::new_v4().to_string(),
            text: text.trim().into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .collect())
}

pub async fn update_notes(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
    input: NotesUpdate,
) -> AppResult<PrivateNotes> {
    let shared = if actor != agent.user_id {
        Some(require(db, actor, agent).await?)
    } else {
        None
    };
    let notes = notes(input.notes)?;
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    if let Some(access) = shared.as_ref() {
        fence(db, access, &mut session).await?;
    }
    let collection = db.collection::<PrivateNotes>(model::NOTES_COLLECTION_NAME);
    let filter = doc! {"agent_id": &agent.id, "actor_user_id": actor};
    let old = collection
        .find_one(filter.clone())
        .session(&mut session)
        .await?;
    if old.as_ref().map_or(0, |n| n.revision) != input.expected_revision {
        return Err(AppError::Conflict(
            "Notes changed; reload before saving".into(),
        ));
    }
    let row = PrivateNotes {
        id: old.map_or_else(|| Uuid::new_v4().to_string(), |r| r.id),
        agent_id: agent.id.clone(),
        actor_user_id: actor.into(),
        revision: input.expected_revision.checked_add(1).ok_or_else(missing)?,
        notes,
        updated_at: Utc::now(),
    };
    collection
        .replace_one(filter, &row)
        .upsert(true)
        .session(&mut session)
        .await?;
    session.commit_transaction().await?;
    audit(db, actor, agent, "private_notes").await;
    Ok(row)
}

pub async fn update_memory(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
    input: NotesUpdate,
) -> AppResult<AgentSharing> {
    let shared = if actor == agent.user_id {
        personal_owner(db, actor, &agent.id).await?;
        None
    } else {
        let access = require(db, actor, agent).await?;
        if access.role != ShareRole::Manager {
            return Err(missing());
        }
        Some(access)
    };
    let memory = notes(input.notes)?;
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    if let Some(access) = shared {
        fence(db, &access, &mut session).await?;
    }
    let collection = db.collection::<AgentSharing>(model::COLLECTION_NAME);
    let result = collection.find_one_and_update(doc! {"_id": &agent.id, "owner_id": &agent.user_id,
        "revision": input.expected_revision}, doc! {"$set": {"memory": bson::to_bson(&memory)
        .map_err(|_| AppError::Internal("Memory serialization failed".into()))?, "updated_at": bson::DateTime::now()},
        "$inc": {"revision": 1}}).return_document(mongodb::options::ReturnDocument::After)
        .session(&mut session).await?.ok_or_else(|| AppError::Conflict("Sharing changed; reload before saving".into()))?;
    session.commit_transaction().await?;
    audit(db, actor, agent, "team_memory").await;
    Ok(result)
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<bson::Document>(model::CONSENTS_COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"scope_key": 1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(
                            doc! {"decision": "pending", "scope_key": {"$type": "string"}},
                        )
                        .build(),
                )
                .build(),
        )
        .await?;
    for (collection, keys, unique) in [
        (
            model::SHARES_COLLECTION_NAME,
            doc! {"agent_id": 1, "subject_kind": 1, "subject_id": 1},
            true,
        ),
        (
            model::SHARES_COLLECTION_NAME,
            doc! {"subject_id": 1, "subject_kind": 1, "agent_id": 1},
            false,
        ),
        (
            model::NOTES_COLLECTION_NAME,
            doc! {"agent_id": 1, "actor_user_id": 1},
            true,
        ),
        (
            model::CONSENTS_COLLECTION_NAME,
            doc! {"actor_user_id": 1, "agent_id": 1, "connection_id": 1},
            false,
        ),
        (
            model::CONSENTS_COLLECTION_NAME,
            doc! {"conversation_id": 1, "turn_id": 1},
            false,
        ),
    ] {
        db.collection::<bson::Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(IndexOptions::builder().unique(unique).build())
                    .build(),
            )
            .await?;
    }
    Ok(())
}

/// Removing a grant permanently withholds it. Regranting never resurrects a share.
pub async fn remove_grants_in_session(
    db: &Database,
    agent: &str,
    removed: &[String],
    session: &mut ClientSession,
) -> AppResult<()> {
    if removed.is_empty() {
        return Ok(());
    }
    let mut unset = bson::Document::new();
    for id in removed {
        unset.insert(format!("shared_access.{id}"), "");
    }
    db.collection::<AgentSharing>(model::COLLECTION_NAME)
        .update_one(
            doc! {"_id": agent},
            doc! {"$unset": unset, "$inc": {"revision": 1}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(model::CONSENTS_COLLECTION_NAME)
        .update_many(
            doc! {"agent_id": agent},
            doc! {"$set": {"decision": "revoked"}},
        )
        .session(&mut *session)
        .await?;
    Ok(())
}

pub async fn audience(db: &Database, owner: &str, agent: &str) -> AppResult<Vec<Audience>> {
    personal_owner(db, owner, agent).await?;
    let rows: Vec<AgentShare> = db
        .collection(model::SHARES_COLLECTION_NAME)
        .find(doc! {"agent_id": agent, "owner_id": owner})
        .limit(MAX_SHARES as i64)
        .await?
        .try_collect()
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| Audience {
            subject_kind: r.subject_kind,
            subject_id: r.subject_id,
            role: r.role,
        })
        .collect())
}
