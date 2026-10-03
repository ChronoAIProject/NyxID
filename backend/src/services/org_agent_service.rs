//! Live organization-agent ACL. The agent owner and the execution actor differ.
use futures::TryStreamExt;
use mongodb::{Database, bson::doc};

use super::org_service::{self, OwnerAccess};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::AssistantAgent,
        org_membership::OrgRole,
        user::{COLLECTION_NAME as USERS, User},
        user_service::{COLLECTION_NAME as SERVICES, UserService},
    },
};

pub fn can_use(access: &OwnerAccess) -> bool {
    match access {
        OwnerAccess::Direct | OwnerAccess::AsOrgAdmin { .. } => true,
        OwnerAccess::AsOrgMember { role, .. } => role.can_proxy(),
        OwnerAccess::Forbidden => false,
    }
}

pub fn can_maintain(access: &OwnerAccess) -> bool {
    // Agent maintenance is collaborative; this does not change resource ACLs.
    can_use(access)
}

pub async fn access(db: &Database, actor: &str, owner: &str) -> AppResult<OwnerAccess> {
    if actor == owner {
        return Ok(OwnerAccess::Direct);
    }
    if db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": owner, "user_type": "org", "is_active": true})
        .await?
        .is_none()
    {
        return Ok(OwnerAccess::Forbidden);
    }
    Box::pin(org_service::resolve_owner_access(db, actor, owner)).await
}

pub async fn require_use(db: &Database, actor: &str, agent: &AssistantAgent) -> AppResult<()> {
    if actor == agent.user_id {
        return Ok(());
    }
    if !can_use(&Box::pin(access(db, actor, &agent.user_id)).await?) || agent.is_nyxbot() {
        return Err(AppError::Forbidden(
            "Active organization membership with service access is required to use this agent"
                .into(),
        ));
    }
    Ok(())
}

pub async fn require_maintain(db: &Database, actor: &str, agent: &AssistantAgent) -> AppResult<()> {
    if actor == agent.user_id {
        return Ok(());
    }
    if !can_maintain(&Box::pin(access(db, actor, &agent.user_id)).await?) || agent.is_nyxbot() {
        return Err(AppError::Forbidden(
            "Only organization Admins and Members may maintain this agent".into(),
        ));
    }
    Ok(())
}

/// Explicit selector resolution: ID, slug, then an unambiguous visible name.
pub async fn resolve_org_selector(db: &Database, actor: &str, selector: &str) -> AppResult<String> {
    let memberships = org_service::list_memberships_for_member(db, actor, false).await?;
    let ids: Vec<_> = memberships.iter().map(|m| &m.org_user_id).collect();
    let orgs: Vec<User> = db
        .collection(USERS)
        .find(doc! {"_id": {"$in": ids}, "user_type": "org", "is_active": true})
        .await?
        .try_collect()
        .await?;
    let exact = orgs
        .iter()
        .find(|o| o.id == selector || o.slug.as_deref() == Some(selector));
    let org = if let Some(org) = exact {
        org
    } else {
        let mut names = orgs
            .iter()
            .filter(|o| o.display_name.as_deref() == Some(selector));
        let org = names
            .next()
            .ok_or_else(|| AppError::NotFound("Organization not found".into()))?;
        if names.next().is_some() {
            return Err(AppError::ValidationError(
                "Organization name is ambiguous; use its ID or slug".into(),
            ));
        }
        org
    };
    Ok(org.id.clone())
}

pub async fn resolve_org(db: &Database, actor: &str, selector: &str) -> AppResult<String> {
    let owner = resolve_org_selector(db, actor, selector).await?;
    if !can_maintain(&access(db, actor, &owner).await?) {
        return Err(AppError::Forbidden(
            "Only organization Admins and Members may maintain agents".into(),
        ));
    }
    Ok(owner)
}

pub async fn visible_owners(db: &Database, actor: &str) -> AppResult<Vec<String>> {
    let memberships = org_service::list_memberships_for_member(db, actor, false).await?;
    let ids: Vec<_> = memberships.iter().map(|m| &m.org_user_id).collect();
    let mut owners: Vec<String> = db
        .collection::<User>(USERS)
        .find(doc! {"_id": {"$in": ids}, "user_type": "org", "is_active": true})
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .map(|o| o.id)
        .collect();
    owners.push(actor.into());
    Ok(owners)
}

pub async fn require_creation_enabled(db: &Database, actor: &str) -> AppResult<()> {
    if !super::feature_flag_service::personal_flag_enabled(db, actor, "assistant:org-agents")
        .await?
    {
        return Err(AppError::ValidationError(
            "Organization agents are not enabled yet".into(),
        ));
    }
    Ok(())
}

/// Validate resource ownership and the maintaining person's current service scope.
/// Organization platform connections are represented by their UserService IDs.
pub async fn validate_grants(db: &Database, actor: &str, agent: &AssistantAgent) -> AppResult<()> {
    if actor == agent.user_id {
        return Ok(());
    }
    require_maintain(db, actor, agent).await?;
    if agent.grants.account_read
        || !agent.saved_login_ids.is_empty()
        || !agent.guest_access.is_empty()
    {
        return Err(AppError::Forbidden("Organization agents cannot receive personal account access, saved logins or guest sharing".into()));
    }
    if !agent.grants.platform_service_ids.is_empty() {
        return Err(AppError::ValidationError(
            "Grant the organization's platform connection ID instead of an unbound catalog service"
                .into(),
        ));
    }
    let acl = access(db, actor, &agent.user_id).await?;
    for id in &agent.grants.service_ids {
        let row = db
            .collection::<UserService>(SERVICES)
            .find_one(doc! {"_id": id, "user_id": &agent.user_id, "is_active": true})
            .await?;
        if row.is_none_or(|s| !acl.allows_resource(id) || (s.admin_only && !acl.can_write())) {
            return Err(AppError::Forbidden(
                "Only accessible services owned by this organization may be granted".into(),
            ));
        }
    }
    super::api_key_scope_service::validate_node_ids(
        db,
        &agent.user_id,
        &agent.machine_node_ids,
        super::api_key_scope_service::ScopeAuthorization::OwnerOnly,
    )
    .await
}

/// Request-local, server-resolved membership. Never persisted or cached across
/// authentications. Private fields bind reuse to the same person and owner.
#[derive(Debug)]
pub struct RequestAccess {
    actor: String,
    owner: String,
    acl: OwnerAccess,
}

impl RequestAccess {
    pub(crate) fn approval_admin_permits(&self, ids: &[String]) -> bool {
        self.acl.can_write() && self.acl.allows_any_resource(ids)
    }

    pub(crate) fn is_admin(&self) -> bool {
        matches!(self.acl, OwnerAccess::AsOrgAdmin { .. })
    }

    pub(crate) fn matches(&self, actor: &str, owner: &str) -> bool {
        self.actor == actor && self.owner == owner && can_use(&self.acl)
    }
    pub async fn conversation(
        &self,
        db: &Database,
        actor: &str,
        id: &str,
    ) -> AppResult<crate::models::assistant_conversation::AssistantConversation> {
        self.check(actor, &self.owner)?;
        let row = db
            .collection::<crate::models::assistant_conversation::AssistantConversation>(
                crate::models::assistant_conversation::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": id, "user_id": actor})
            .await?
            .ok_or_else(|| AppError::NotFound("Conversation not found".into()))?;
        self.check_conversation(&row)?;
        if row.group_id.is_some() {
            super::org_group_service::check_thread_participation(db, &row, self).await?;
        }
        Ok(row)
    }

    pub async fn agent(&self, db: &Database, actor: &str, id: &str) -> AppResult<AssistantAgent> {
        self.check(actor, &self.owner)?;
        db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .find_one(doc! {"_id": id, "user_id": &self.owner, "kind": "specialist"})
            .await?
            .ok_or_else(|| AppError::NotFound("Agent not found".into()))
    }

    fn check(&self, actor: &str, owner: &str) -> AppResult<()> {
        if self.actor != actor || self.owner != owner || !can_use(&self.acl) {
            return Err(AppError::Forbidden(
                "Organization agent access snapshot does not match this request".into(),
            ));
        }
        Ok(())
    }

    pub fn check_conversation(
        &self,
        row: &crate::models::assistant_conversation::AssistantConversation,
    ) -> AppResult<()> {
        self.check(
            &row.user_id,
            row.agent_owner_id.as_deref().unwrap_or_default(),
        )?;
        if row.guest_turn || row.channel.is_some() {
            return Err(AppError::Forbidden(
                "Organization agents require a private member thread".into(),
            ));
        }
        Ok(())
    }
}

pub async fn resolve_key_access(
    db: &Database,
    actor: &str,
    owner: Option<&str>,
) -> AppResult<Option<std::sync::Arc<RequestAccess>>> {
    let Some(owner) = owner.filter(|owner| *owner != actor) else {
        return Ok(None);
    };
    let acl = Box::pin(access(db, actor, owner)).await?;
    if !can_use(&acl) {
        return Err(AppError::Forbidden(
            "Organization agent access was revoked or is no longer permitted".into(),
        ));
    }
    Ok(Some(std::sync::Arc::new(RequestAccess {
        actor: actor.into(),
        owner: owner.into(),
        acl,
    })))
}

/// Callers without an authentication context resolve fresh authority. Ordinary
/// keys take the no-read path in resolve_key_access.
pub async fn validate_key(db: &Database, actor: &str, owner: Option<&str>) -> AppResult<()> {
    resolve_key_access(db, actor, owner).await.map(|_| ())
}

pub async fn chat_agent(
    db: &Database,
    chat: &super::assistant_acknowledgement_service::ChatAuthority,
) -> AppResult<AssistantAgent> {
    if let Some(access) = chat.org_agent_access.as_deref() {
        return access.agent(db, &chat.user_id, &chat.agent_id).await;
    }
    let agent = super::assistant_team_service::agent(db, &chat.user_id, &chat.agent_id).await?;
    require_use(db, &chat.user_id, &agent).await?;
    Ok(agent)
}

fn bound_access<'a>(
    actor: &str,
    owner: Option<&str>,
    snapshot: Option<&'a RequestAccess>,
) -> AppResult<Option<&'a RequestAccess>> {
    let Some(owner) = owner.filter(|owner| *owner != actor) else {
        if snapshot.is_some() {
            return Err(AppError::Forbidden(
                "Organization agent access snapshot does not match this request".into(),
            ));
        }
        return Ok(None);
    };
    let snapshot = snapshot.ok_or_else(|| {
        AppError::Forbidden("Organization agent access snapshot is missing".into())
    })?;
    snapshot.check(actor, owner)?;
    Ok(Some(snapshot))
}

pub fn role(access: &OwnerAccess) -> Option<OrgRole> {
    match access {
        OwnerAccess::AsOrgAdmin { .. } => Some(OrgRole::Admin),
        OwnerAccess::AsOrgMember { role, .. } => Some(*role),
        _ => None,
    }
}

pub async fn key_services(
    db: &Database,
    key: &crate::models::api_key::ApiKey,
    snapshot: Option<&RequestAccess>,
) -> AppResult<Option<Vec<String>>> {
    let Some(access) = bound_access(
        &key.user_id,
        key.assistant_agent_owner_id.as_deref(),
        snapshot,
    )?
    else {
        return Ok(None);
    };
    let owner = &access.owner;
    let acl = &access.acl;
    let rows: Vec<UserService> = db
        .collection(SERVICES)
        .find(doc! {"_id": {"$in": &key.allowed_service_ids}, "user_id": owner, "is_active": true})
        .await?
        .try_collect()
        .await?;
    Ok(Some(
        rows.into_iter()
            .filter(|s| acl.allows_resource(&s.id) && (!s.admin_only || acl.can_write()))
            .map(|s| s.id)
            .collect(),
    ))
}

pub async fn key_nodes(
    db: &Database,
    key: &crate::models::api_key::ApiKey,
    snapshot: Option<&RequestAccess>,
) -> AppResult<(bool, Vec<String>)> {
    let Some(access) = bound_access(
        &key.user_id,
        key.assistant_agent_owner_id.as_deref(),
        snapshot,
    )?
    else {
        return Ok((key.allow_all_nodes, key.allowed_node_ids.clone()));
    };
    let rows: Vec<crate::models::node::Node> = db
        .collection(crate::models::node::COLLECTION_NAME)
        .find(doc! {"user_id": &access.owner, "is_active": true})
        .await?
        .try_collect()
        .await?;
    Ok((false, rows.into_iter().map(|n| n.id).collect()))
}

/// Final service fencing also covers pool failover and explicit routing hints.
pub async fn authorize_execution(
    db: &Database,
    auth: &crate::mw::auth::AuthUser,
    instance: Option<&str>,
) -> AppResult<()> {
    if auth.assistant_group_id.is_some() && auth.org_agent_access.is_none() {
        return Err(super::org_group_service::missing());
    }
    authorize_service_with_access(
        db,
        &auth.user_id.to_string(),
        auth.assistant_agent_owner_id.as_deref(),
        instance,
        auth.org_agent_access.as_deref(),
    )
    .await
}

pub async fn authorize_service(
    db: &Database,
    actor: &str,
    bound_owner: Option<&str>,
    instance: Option<&str>,
) -> AppResult<()> {
    let snapshot = resolve_key_access(db, actor, bound_owner).await?;
    authorize_service_with_access(db, actor, bound_owner, instance, snapshot.as_deref()).await
}

pub async fn authorize_service_with_access(
    db: &Database,
    actor: &str,
    bound_owner: Option<&str>,
    instance: Option<&str>,
    snapshot: Option<&RequestAccess>,
) -> AppResult<()> {
    let Some(access) = bound_access(actor, bound_owner, snapshot)? else {
        return Ok(());
    };
    let owner = &access.owner;
    let id = instance.ok_or_else(|| {
        AppError::Forbidden(
            "Organization agents must use an organization service connection".into(),
        )
    })?;
    let row = db
        .collection::<UserService>(SERVICES)
        .find_one(doc! {"_id": id, "user_id": owner, "is_active": true})
        .await?;
    let acl = &access.acl;
    if row.is_none_or(|s| !acl.allows_resource(id) || (s.admin_only && !acl.can_write())) {
        return Err(AppError::Forbidden(
            "This service is outside the organization's live member access".into(),
        ));
    }
    Ok(())
}

pub async fn tool_arguments(
    db: &Database,
    actor: &str,
    orchestrator: bool,
    args: &serde_json::Value,
) -> AppResult<serde_json::Value> {
    let Some(selector) = args.get("org").and_then(serde_json::Value::as_str) else {
        return Ok(args.clone());
    };
    if !orchestrator {
        return Err(AppError::Forbidden(
            "Only your personal NyxBot manages organization agents".into(),
        ));
    }
    let owner = resolve_org(db, actor, selector).await?;
    let mut args = args.clone();
    args["org"] = owner.clone().into();
    for field in ["agent", "subagent"] {
        if let Some(selector) = args.get(field).and_then(serde_json::Value::as_str) {
            let agent = db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
                .find_one(doc! {"user_id": &owner, "kind": "specialist", "destroyed_at": mongodb::bson::Bson::Null,
                    "$or": [{"_id": selector}, {"name": selector}]})
                .await?.ok_or_else(|| AppError::NotFound("Organization agent not found".into()))?;
            require_maintain(db, actor, &agent).await?;
            args[field] = agent.id.into();
        }
    }
    Ok(args)
}

pub async fn audit_change(db: &Database, actor: &str, agent: &AssistantAgent, kind: &str) {
    if actor == agent.user_id {
        return;
    }
    let _ = super::audit_service::log_actor_event(
        db.clone(),
        &super::audit_service::AuditActor {
            user_id: actor.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        "assistant_org_agent_changed",
        Some(serde_json::json!({"org": agent.user_id, "agent_id": agent.id, "change_kind": kind})),
    )
    .await;
}
