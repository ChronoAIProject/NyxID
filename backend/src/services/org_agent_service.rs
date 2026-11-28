//! Live organization-agent ACL. The agent owner and the execution actor differ.
pub(crate) mod authoring;

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

#[derive(Clone, Copy, Debug, Default)]
pub struct SpecialistRights {
    pub can_use: bool,
    pub can_maintain: bool,
    pub can_govern: bool,
    maintain_revision: i64,
    pub(crate) eligibility_revision: i64,
}

pub fn specialist_rights(
    actor: &str,
    agent: &AssistantAgent,
    access: &OwnerAccess,
) -> SpecialistRights {
    if matches!(access, OwnerAccess::Direct) && actor == agent.user_id {
        return SpecialistRights {
            can_use: true,
            can_maintain: true,
            can_govern: false,
            maintain_revision: 0,
            eligibility_revision: 0,
        };
    }
    let Some(role) = role(access) else {
        return SpecialistRights::default();
    };
    SpecialistRights {
        eligibility_revision: access.eligibility_revision(),
        ..member_rights(actor, agent, role)
    }
}

/// Shared use audience predicate for complete agents and projected activity metadata.
pub(crate) fn member_can_use(
    actor: &str,
    kind: crate::models::assistant_agent::AgentKind,
    users: Option<&[String]>,
    role: OrgRole,
) -> bool {
    role.can_proxy()
        && kind != crate::models::assistant_agent::AgentKind::Nyxbot
        && users.is_none_or(|ids| ids.iter().any(|id| id == actor))
}

pub fn member_rights(actor: &str, agent: &AssistantAgent, role: OrgRole) -> SpecialistRights {
    let eligible = role.can_proxy() && !agent.is_nyxbot();
    let includes = |list: &Option<Vec<String>>| {
        list.as_ref()
            .is_none_or(|ids| ids.iter().any(|id| id == actor))
    };
    SpecialistRights {
        can_use: member_can_use(actor, agent.kind, agent.org_user_ids.as_deref(), role),
        can_maintain: eligible && includes(&agent.org_maintainer_user_ids),
        maintain_revision: agent.org_maintain_revision,
        eligibility_revision: 0,
        can_govern: role == OrgRole::Admin && !agent.is_nyxbot(),
    }
}

/// Resolve the same membership and role scope as a single owner lookup for a page of agents.
pub(crate) async fn owners_for_page(
    db: &Database,
    actor: &str,
    owners: std::collections::HashSet<&str>,
) -> AppResult<std::collections::HashMap<String, (OwnerAccess, User)>> {
    use crate::models::{
        org_membership::{MemberScopeSource, OrgMembership},
        org_role_scope::OrgRoleScope,
    };
    use std::collections::HashMap;
    let ids: Vec<_> = owners.into_iter().filter(|id| *id != actor).collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut users: HashMap<String, User> = db
        .collection::<User>(USERS)
        .find(doc! {"_id":{"$in":&ids},"user_type":"org","is_active":true})
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .map(|u| (u.id.clone(), u))
        .collect();
    let memberships: Vec<OrgMembership> = db
        .collection(crate::models::org_membership::COLLECTION_NAME)
        .find(doc! {"org_user_id":{"$in":ids},"member_user_id":actor,"revoked_at":null})
        .await?
        .try_collect()
        .await?;
    let inherited: Vec<_> = memberships
        .iter()
        .filter(|m| m.scope_source == MemberScopeSource::Inherit)
        .map(|m| &m.org_user_id)
        .collect();
    let scopes: HashMap<_, _> = if inherited.is_empty() {
        HashMap::new()
    } else {
        db.collection::<OrgRoleScope>(crate::models::org_role_scope::COLLECTION_NAME)
            .find(doc! {"org_user_id":{"$in":inherited}})
            .await?
            .try_collect::<Vec<_>>()
            .await?
            .into_iter()
            .map(|s| ((s.org_user_id, s.role), s.allowed_service_ids))
            .collect()
    };
    Ok(memberships
        .into_iter()
        .filter_map(|m| {
            let owner = users.remove(&m.org_user_id)?;
            let scope = match m.scope_source {
                MemberScopeSource::Override => m.allowed_service_ids.clone(),
                MemberScopeSource::Inherit => scopes
                    .get(&(m.org_user_id.clone(), m.role))
                    .cloned()
                    .flatten(),
            };
            Some((
                m.org_user_id.clone(),
                (org_service::access_for_membership(m, scope), owner),
            ))
        })
        .collect())
}

pub async fn governed_agent(db: &Database, actor: &str, id: &str) -> AppResult<AssistantAgent> {
    let missing = || AppError::NotFound("Organization specialist not found".into());
    let agent = db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(doc! {"_id":id,"kind":"specialist","user_id":{"$ne":actor}})
        .await?
        .ok_or_else(missing)?;
    let acl = access(db, actor, &agent.user_id).await?;
    if !acl.can_read() {
        return Err(missing());
    }
    if !specialist_rights(actor, &agent, &acl).can_govern {
        return Err(AppError::Forbidden(
            "Organization administrator access is required".into(),
        ));
    }
    Ok(agent)
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
    if agent.is_nyxbot() || resolve_agent_access(db, actor, agent).await?.is_none() {
        return Err(AppError::Forbidden(
            "Active organization membership with service access is required to use this agent"
                .into(),
        ));
    }
    Ok(())
}

pub async fn require_maintain(db: &Database, actor: &str, agent: &AssistantAgent) -> AppResult<()> {
    maintenance_revision(db, actor, agent).await.map(|_| ())
}

pub(crate) async fn maintenance_revision(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
) -> AppResult<i64> {
    if actor == agent.user_id {
        return Ok(0);
    }
    let acl = Box::pin(access(db, actor, &agent.user_id)).await?;
    if !can_maintain(&acl) || agent.is_nyxbot() {
        return Err(AppError::Forbidden(
            "Only organization Admins and Members may maintain this agent".into(),
        ));
    }
    if !specialist_rights(actor, agent, &acl).can_maintain {
        return Err(AppError::Forbidden(
            "Organization specialist maintenance access is required".into(),
        ));
    }
    Ok(acl.eligibility_revision())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberFence {
    ReadOnly,
    Member,
    MemberAndOrg,
}

pub(crate) async fn member_rights_in_session(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    actor: &str,
    agent: &AssistantAgent,
    fence: MemberFence,
) -> AppResult<SpecialistRights> {
    use super::org_channel_service::live_doc;
    live_doc(
        db,
        tx,
        USERS,
        doc! {"_id":&agent.user_id,"user_type":"org","is_active":true},
        fence == MemberFence::MemberAndOrg,
    )
    .await?;
    live_doc(
        db,
        tx,
        USERS,
        doc! {"_id":actor,"user_type":{"$ne":"org"},"is_active":true},
        fence != MemberFence::ReadOnly,
    )
    .await?;
    let membership = live_doc(db,tx,crate::models::org_membership::COLLECTION_NAME,
        doc! {"member_user_id":actor,"org_user_id":&agent.user_id,"revoked_at":null,"role":{"$in":["admin","member"]}},fence != MemberFence::ReadOnly).await?;
    let membership: crate::models::org_membership::OrgMembership = bson::from_document(membership)
        .map_err(|_| AppError::Forbidden("Membership unavailable".into()))?;
    Ok(SpecialistRights {
        eligibility_revision: membership.eligibility_revision,
        ..member_rights(actor, agent, membership.role)
    })
}

/// Membership loss prunes explicit selections in the membership mutation's transaction.
pub(crate) async fn prune_member(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    org: &str,
    actor: &str,
) -> AppResult<()> {
    let mut filter = member_list_filter(actor);
    filter.insert("user_id", org);
    db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_many(filter, prune_member_update(actor))
        .session(tx)
        .await?;
    Ok(())
}

pub(super) fn member_list_filter(actor: &str) -> bson::Document {
    doc! {"kind":"specialist","$or":[{"org_maintainer_user_ids":actor},{"org_user_ids":actor}]}
}

pub(super) fn prune_member_update(actor: &str) -> Vec<bson::Document> {
    let contains = |field: &str| bson::bson!({"$in":[{"$literal":actor},{"$ifNull":[format!("${field}"),[]]}]});
    let prune = |field: &str| bson::bson!({"$cond":[contains(field),{"$filter":{"input":format!("${field}"),"cond":{"$ne":["$$this",{"$literal":actor}]}}},format!("${field}")]});
    let advance = |revision: &str, field: &str| bson::bson!({"$add":[{"$ifNull":[format!("${revision}"),0]},{"$cond":[contains(field),1_i64,0_i64]}]});
    vec![doc! {"$set":{
        "org_access_revision":{"$add":[{"$ifNull":["$org_access_revision",0]},1_i64]},
        "org_use_revision":advance("org_use_revision","org_user_ids"),
        "org_maintain_revision":advance("org_maintain_revision","org_maintainer_user_ids"),
        "org_user_ids":prune("org_user_ids"),"org_maintainer_user_ids":prune("org_maintainer_user_ids"),
    }}]
}

fn live_specialist_filter(org: &str, agent_id: &str) -> bson::Document {
    doc! {"_id":agent_id,"user_id":org,"kind":"specialist","destroyed_at":null}
}

pub(crate) async fn fence_member_use(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    actor: &str,
    org: &str,
    agent_id: &str,
    fence_policy: bool,
) -> AppResult<crate::models::org_membership::MemberUse> {
    let agents = db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME);
    let filter = live_specialist_filter(org, agent_id);
    let agent = if fence_policy {
        agents
            .find_one_and_update(filter, doc! {"$inc":{"thread_fence":1_i64}})
            .session(&mut *tx)
            .await?
    } else {
        agents.find_one(filter).session(&mut *tx).await?
    }
    .ok_or_else(super::org_group_service::missing)?;
    let rights = member_rights_in_session(db, tx, actor, &agent, MemberFence::Member).await?;
    if !rights.can_use {
        return Err(AppError::Forbidden(
            "Specialist use access is required".into(),
        ));
    }
    Ok(agent.member_use(rights.eligibility_revision))
}

pub(crate) fn ordinary_member_org(
    row: &crate::models::assistant_conversation::AssistantConversation,
) -> Option<&str> {
    row.agent_owner_id.as_deref().filter(|org| {
        *org != row.user_id
            && !row.shared_agent_thread
            && row.collaboration.is_none()
            && !row.guest_turn
    })
}

/// Settlement can race a narrowing. Queued results retain the revision for the
/// consumer's fenced authorization, so independent writers can finish together.
pub(crate) async fn settlement_use_is_live(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    actor: &str,
    org: &str,
    agent_id: &str,
    revision: crate::models::org_membership::MemberUse,
) -> AppResult<bool> {
    let Some(agent) = db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(live_specialist_filter(org, agent_id))
        .session(&mut *tx)
        .await?
    else {
        return Ok(false);
    };
    if agent.org_use_revision != revision.org_use_revision {
        return Ok(false);
    }
    match member_rights_in_session(db, tx, actor, &agent, MemberFence::ReadOnly).await {
        Ok(rights) => {
            Ok(rights.can_use && rights.eligibility_revision == revision.eligibility_revision)
        }
        Err(AppError::Forbidden(_) | AppError::NotFound(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) async fn use_revision_is_live(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    actor: &str,
    org: &str,
    agent: &str,
    revision: crate::models::org_membership::MemberUse,
) -> AppResult<bool> {
    match fence_member_use(db, tx, actor, org, agent, true).await {
        Ok(current) => Ok(current == revision),
        Err(AppError::Forbidden(_) | AppError::NotFound(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) async fn event_source_is_live(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    actor: &str,
    event: &crate::models::assistant_conversation::AgentEvent,
) -> AppResult<bool> {
    if event.source_owner_id.is_none()
        && !matches!(
            event.kind.as_str(),
            "subagent_settled" | "permission_requested"
        )
    {
        return Ok(true);
    }
    let Some(id) = event.agent_id.as_deref() else {
        // Unattributed legacy status notices carry no specialist assignment.
        return Ok(event.source_owner_id.is_none() && event.delegated_result.is_none());
    };
    let legacy_owner;
    let org = match event.source_owner_id.as_deref() {
        Some(owner) if owner == actor => return Ok(true),
        Some(owner) => owner,
        None => {
            // Legacy delegated results predate the persistent revision. Identify their owner once.
            let Some(agent) = db
                .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
                .find_one(doc! {"_id":id})
                .session(&mut *tx)
                .await?
            else {
                return Ok(false);
            };
            if agent.user_id == actor {
                return Ok(true);
            }
            let owner_is_org = db
                .collection::<bson::Document>(USERS)
                .find_one(doc! {"_id":&agent.user_id,"user_type":"org"})
                .projection(doc! {"_id":1})
                .session(&mut *tx)
                .await?
                .is_some();
            if !owner_is_org {
                return Ok(true);
            }
            legacy_owner = agent.user_id;
            &legacy_owner
        }
    };
    use_revision_is_live(
        db,
        tx,
        actor,
        org,
        id,
        crate::models::org_membership::MemberUse {
            org_use_revision: event.source_org_use_revision,
            eligibility_revision: event.source_eligibility_revision,
        },
    )
    .await
}

pub(crate) fn fence_maintenance<'a>(
    db: &'a Database,
    tx: &'a mut mongodb::ClientSession,
    actor: &'a str,
    target: &'a AssistantAgent,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        fence_maintenance_revision(db, tx, actor, target)
            .await
            .map(|_| ())
    })
}

pub(crate) async fn fence_maintenance_revision(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    actor: &str,
    target: &AssistantAgent,
) -> AppResult<i64> {
    if actor == target.user_id {
        return Ok(0);
    }
    let agent = db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id":&target.id,"user_id":&target.user_id,"kind":"specialist"},
            doc! {"$inc":{"thread_fence":1_i64}},
        )
        .session(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound("Specialist not found".into()))?;
    let rights = member_rights_in_session(db, tx, actor, &agent, MemberFence::MemberAndOrg).await?;
    if agent.org_maintain_revision != target.org_maintain_revision || !rights.can_maintain {
        return Err(AppError::Forbidden(
            "Specialist maintenance access changed".into(),
        ));
    }
    Ok(rights.eligibility_revision)
}

/// Personal writes retain their existing single-document operation.
pub(crate) async fn update_maintained(
    db: &Database,
    actor: &str,
    target: &AssistantAgent,
    filter: bson::Document,
    update: bson::Document,
) -> AppResult<Option<AssistantAgent>> {
    use mongodb::options::ReturnDocument;
    let collection =
        db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME);
    if actor == target.user_id {
        return Ok(collection
            .find_one_and_update(filter, update)
            .return_document(ReturnDocument::After)
            .await?);
    }
    let context = (db, actor, target, collection, filter, update);
    super::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, actor, target, coll, filter, update)| {
            Box::pin(async move {
                fence_maintenance(db, tx, actor, target).await?;
                Ok(coll
                    .find_one_and_update(filter.clone(), update.clone())
                    .return_document(ReturnDocument::After)
                    .session(tx)
                    .await?)
            })
        },
    )
    .await
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

/// Organization profiles visible to Viewers do not authorize live thread activity.
pub(crate) async fn activity_owners(
    db: &Database,
    actor: &str,
) -> AppResult<std::collections::HashMap<String, OrgRole>> {
    let memberships = org_service::list_memberships_for_member(db, actor, false).await?;
    let roles: std::collections::HashMap<_, _> = memberships
        .into_iter()
        .filter(|membership| membership.role.can_proxy())
        .map(|membership| (membership.org_user_id, membership.role))
        .collect();
    if roles.is_empty() {
        return Ok(roles);
    }
    let ids: Vec<_> = roles.keys().collect();
    let active = db
        .collection::<User>(USERS)
        .find(doc! {"_id": {"$in": ids}, "user_type": "org", "is_active": true})
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    Ok(active
        .into_iter()
        .filter_map(|user| roles.get(&user.id).map(|role| (user.id, *role)))
        .collect())
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
    if agent.grants.account_read || !agent.saved_login_ids.is_empty() {
        return Err(AppError::Forbidden(
            "Organization agents cannot receive personal account access or saved logins".into(),
        ));
    }
    for id in &agent.grants.platform_service_ids {
        let model = db.collection::<crate::models::platform_model::PlatformModel>("platform_models")
            .find_one(doc! {"billing_service_id":id,"enabled":true,"lifecycle":"active"}).await?
            .ok_or_else(|| AppError::ValidationError("Grant the organization's connection ID; only platform model offerings support an unbound catalog grant".into()))?;
        super::assistant_platform_model::require_ready(db).await?;
        super::platform_model_service::authorized_model(db, actor, &model.public_id).await?;
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

/// Authorize the requested guest-policy mutation, including clearing a restriction.
/// Looking only at the resulting maps would let a Member restore the default Use
/// level. Fence the admin membership in the same transaction as the policy write.
pub(crate) async fn authorize_guest_change(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    if actor == agent.user_id {
        return Ok(());
    }
    require_creation_enabled(db, actor).await?;
    super::org_channel_service::enabled(db, actor).await?;
    super::org_channel_service::require_admin(db, actor, &agent.user_id).await?;
    for (collection, filter) in [
        (
            "users",
            doc! {"_id":&agent.user_id,"user_type":"org","is_active":true},
        ),
        ("users", doc! {"_id":actor,"is_active":true}),
        (
            "org_memberships",
            doc! {"org_user_id":&agent.user_id,"member_user_id":actor,"role":"admin","revoked_at":null},
        ),
    ] {
        super::org_channel_service::live_doc(db, session, collection, filter, true).await?;
    }
    Ok(())
}

/// Request-local, server-resolved membership. Never persisted or cached across
/// authentications. Private fields bind reuse to the same person and owner.
#[derive(Debug)]
pub struct RequestAccess {
    actor: String,
    owner: String,
    acl: OwnerAccess,
    specialist: Option<(String, SpecialistRights)>,
    credential: Option<String>,
    shared: Option<super::shared_agent_service::RequestAccess>,
    channel: Option<super::org_channel_service::Access>,
}

impl RequestAccess {
    pub(crate) fn channel(&self) -> Option<&super::org_channel_service::Access> {
        self.channel.as_ref()
    }

    pub(crate) async fn from_channel(
        db: &Database,
        channel: super::org_channel_service::Access,
    ) -> AppResult<std::sync::Arc<Self>> {
        let e = channel.execution();
        let acl = if e.guest {
            OwnerAccess::Forbidden
        } else {
            let acl = access(db, &e.actor_id, &e.org_id).await?;
            if e.member_use != channel.agent().member_use(acl.eligibility_revision())
                || !specialist_rights(&e.actor_id, channel.agent(), &acl).can_use
            {
                return Err(super::org_channel_service::denied());
            }
            acl
        };
        Ok(std::sync::Arc::new(Self {
            actor: e.actor_id.clone(),
            owner: e.org_id.clone(),
            specialist: Some((
                channel.agent().id.clone(),
                specialist_rights(&e.actor_id, channel.agent(), &acl),
            )),
            credential: None,
            acl,
            shared: None,
            channel: Some(channel),
        }))
    }

    pub(crate) fn shared(&self) -> Option<&super::shared_agent_service::RequestAccess> {
        self.shared.as_ref()
    }

    pub(crate) fn from_shared(shared: super::shared_agent_service::RequestAccess) -> Self {
        Self {
            actor: shared.actor().into(),
            owner: shared.owner().into(),
            acl: OwnerAccess::Forbidden,
            specialist: None,
            credential: None,
            shared: Some(shared),
            channel: None,
        }
    }

    pub(crate) fn for_thread(
        self: &std::sync::Arc<Self>,
        agent: &AssistantAgent,
        row: &crate::models::assistant_conversation::AssistantConversation,
    ) -> AppResult<std::sync::Arc<Self>> {
        if self.shared.is_some() || self.channel.is_some() {
            return Ok(self.clone());
        }
        self.check_conversation(row)?;
        if agent.id.as_str() != row.agent_id.as_deref().unwrap_or_default()
            || !self.permits_agent(&row.user_id, agent)
            || agent.member_use(self.eligibility_revision()) != row.member_use
        {
            return Err(AppError::Forbidden("Specialist use access changed".into()));
        }
        let rights = specialist_rights(&row.user_id, agent, &self.acl);
        Ok(std::sync::Arc::new(Self {
            actor: self.actor.clone(),
            owner: self.owner.clone(),
            acl: self.acl.clone(),
            specialist: Some((agent.id.clone(), rights)),
            credential: Some(row.credential_api_key_id.clone()),
            shared: None,
            channel: None,
        }))
    }

    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }

    /// Whether this request snapshot permits the given organization service.
    /// The snapshot is the authoritative ACL for an organization-bound key;
    /// callers must not resolve membership a second time.
    pub(crate) fn allows_service(&self, service: &UserService) -> bool {
        if let Some(channel) = &self.channel {
            return channel.allows_service(service)
                && (channel.execution().guest
                    || (self.acl.allows_resource(&service.id)
                        && (!service.admin_only || self.acl.can_write())));
        }
        if let Some(shared) = &self.shared {
            return shared.allows_connection(service);
        }
        self.acl.allows_resource(&service.id) && (!service.admin_only || self.acl.can_write())
    }

    pub(crate) fn can_maintain(&self) -> bool {
        self.specialist
            .as_ref()
            .is_some_and(|(_, rights)| rights.can_maintain)
    }

    pub(crate) fn eligibility_revision(&self) -> i64 {
        self.acl.eligibility_revision()
    }

    pub(crate) fn maintain_revision(&self) -> i64 {
        self.specialist
            .as_ref()
            .map_or(0, |(_, rights)| rights.maintain_revision)
    }

    pub(crate) async fn fence_thread(
        self: &std::sync::Arc<Self>,
        db: &Database,
        row: &crate::models::assistant_conversation::AssistantConversation,
        tx: &mut mongodb::ClientSession,
    ) -> AppResult<std::sync::Arc<Self>> {
        self.check_conversation(row)?;
        if self.shared.is_some() || self.channel.is_some() {
            return Ok(self.clone());
        }
        let agent = db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .find_one_and_update(doc! {"_id":row.agent_id.as_deref(),"user_id":&self.owner,"kind":"specialist","destroyed_at":null},doc! {"$inc":{"thread_fence":1_i64}})
            .session(&mut *tx).await?.ok_or_else(super::shared_agent_service::missing)?;
        let rights =
            member_rights_in_session(db, tx, &row.user_id, &agent, MemberFence::MemberAndOrg)
                .await?;
        if !rights.can_use || row.member_use != agent.member_use(rights.eligibility_revision) {
            return Err(AppError::Forbidden("Specialist use access changed".into()));
        }
        Ok(std::sync::Arc::new(Self {
            actor: self.actor.clone(),
            owner: self.owner.clone(),
            acl: self.acl.clone(),
            specialist: Some((agent.id, rights)),
            credential: Some(row.credential_api_key_id.clone()),
            shared: None,
            channel: None,
        }))
    }

    pub(crate) fn permits_agent(&self, actor: &str, agent: &AssistantAgent) -> bool {
        if !self.matches(actor, &agent.user_id) {
            return false;
        }
        if let Some(channel) = self.channel() {
            return channel.agent().id == agent.id;
        }
        if let Some(shared) = self.shared() {
            return shared.agent().id == agent.id;
        }
        self.specialist
            .as_ref()
            .is_none_or(|(id, _)| id == &agent.id)
            && specialist_rights(actor, agent, &self.acl).can_use
    }

    pub(crate) fn approval_admin_permits(&self, ids: &[String]) -> bool {
        self.acl.can_write() && self.acl.allows_any_resource(ids)
    }

    pub(crate) fn is_admin(&self) -> bool {
        matches!(self.acl, OwnerAccess::AsOrgAdmin { .. })
    }

    pub(crate) fn matches(&self, actor: &str, owner: &str) -> bool {
        self.actor == actor
            && self.owner == owner
            && (self.shared.is_some() || self.channel.is_some() || can_use(&self.acl))
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
        if self
            .specialist
            .as_ref()
            .is_some_and(|(expected, _)| expected != id)
        {
            return Err(super::shared_agent_service::missing());
        }
        if let Some(channel) = &self.channel {
            if channel.agent().id != id {
                return Err(super::org_channel_service::denied());
            }
            let mut agent = channel.agent().clone();
            if channel.execution().guest {
                agent.memory.clear();
                agent.machine_node_ids.clear();
                agent.saved_login_ids.clear();
                agent.grants.account_read = false;
            }
            return Ok(agent);
        }
        if let Some(shared) = &self.shared {
            if !shared.matches(actor, &self.owner, id) {
                return Err(super::shared_agent_service::missing());
            }
            return Ok(shared.agent().clone());
        }
        let agent = db
            .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .find_one(doc! {"_id": id, "user_id": &self.owner, "kind": "specialist"})
            .await?
            .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;
        if !self.permits_agent(actor, &agent) {
            return Err(super::shared_agent_service::missing());
        }
        Ok(agent)
    }

    fn check(&self, actor: &str, owner: &str) -> AppResult<()> {
        if !self.matches(actor, owner) {
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
        if self
            .specialist
            .as_ref()
            .is_some_and(|(id, _)| Some(id.as_str()) != row.agent_id.as_deref())
            || self
                .credential
                .as_ref()
                .is_some_and(|id| *id != row.credential_api_key_id)
        {
            return Err(super::shared_agent_service::missing());
        }
        if let Some(channel) = &self.channel {
            return channel.check_conversation(row);
        }
        self.check(
            &row.user_id,
            row.agent_owner_id.as_deref().unwrap_or_default(),
        )?;
        if let Some(shared) = &self.shared
            && (!shared.matches(
                &row.user_id,
                &self.owner,
                row.agent_id.as_deref().unwrap_or_default(),
            ))
        {
            return Err(super::shared_agent_service::missing());
        }
        if self.shared.is_some() {
            super::assistant_group_service::check_shared_surface(row)?;
        }
        if row.guest_turn || row.channel.is_some() {
            return Err(AppError::Forbidden(
                "Organization agents require a private member thread".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn test_request_access(actor: &str, owner: &str, acl: OwnerAccess) -> RequestAccess {
    RequestAccess {
        actor: actor.into(),
        owner: owner.into(),
        acl,
        specialist: None,
        credential: None,
        shared: None,
        channel: None,
    }
}

pub async fn resolve_key_access(
    db: &Database,
    actor: &str,
    owner: Option<&str>,
) -> AppResult<Option<std::sync::Arc<RequestAccess>>> {
    resolve_access(db, actor, owner, None, None).await
}

pub async fn resolve_agent_access(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
) -> AppResult<Option<std::sync::Arc<RequestAccess>>> {
    resolve_access(db, actor, Some(&agent.user_id), Some(agent), None).await
}

/// Resolve only the personal shared-agent lane. Organization specialists keep
/// the existing lazy access resolution used by their execution paths; callers
/// that construct a chat authority must not eagerly attach an org snapshot.
pub async fn resolve_shared_agent_access(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
) -> AppResult<Option<std::sync::Arc<RequestAccess>>> {
    let shared = match super::shared_agent_service::require(db, actor, agent).await {
        Ok(shared) => shared,
        Err(AppError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(std::sync::Arc::new(RequestAccess::from_shared(
        shared,
    ))))
}

pub async fn resolve_credential_access(
    db: &Database,
    key: &crate::models::api_key::ApiKey,
) -> AppResult<Option<std::sync::Arc<RequestAccess>>> {
    // Channel, relay and ordinary API keys may carry an assistant owner
    // marker for legacy routing metadata, but they are not conversation
    // credentials.  Keep those keys on the existing no-read path; otherwise
    // a foreign marker would trigger a personal-thread lookup that can never
    // resolve and would break channel ingress.
    if key.platform.as_deref()
        != Some(super::assistant_agent_credential_service::ASSISTANT_PLATFORM)
        && key.assistant_group_id.is_none()
        && !key.allow_auto_connected_services
    {
        return Ok(None);
    }
    let Some(owner) = key.assistant_agent_owner_id.as_deref() else {
        return Ok(None);
    };
    let row = db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"user_id":&key.user_id,"credential_api_key_id":&key.id})
        .await?;
    if let Some(row) = row
        .as_ref()
        .filter(|row| super::org_channel_service::origin(row).is_some())
    {
        if super::org_channel_service::origin(row)
            .and_then(|o| o.org_execution.as_ref())
            .is_none_or(|e| e.org_id != owner)
        {
            return Err(super::org_channel_service::denied());
        }
        let access = super::org_channel_service::conversation_access(db, row).await?;
        if access
            .as_ref()
            .and_then(|a| a.channel())
            .is_some_and(|c| !c.execution().guest && c.execution().member_use != row.member_use)
        {
            return Err(super::org_channel_service::denied());
        }
        return Ok(access);
    }
    if key.user_id == owner {
        return Ok(None);
    }
    let row = row
        .filter(|r| r.agent_owner_id.as_deref() == Some(owner))
        .ok_or_else(super::shared_agent_service::missing)?;
    let agent = db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(doc! {"_id":row.agent_id.as_deref().ok_or_else(super::shared_agent_service::missing)?,"user_id":owner,"kind":"specialist"}).await?
        .ok_or_else(super::shared_agent_service::missing)?;
    let access = resolve_access(db, &key.user_id, Some(owner), Some(&agent), Some(&key.id)).await?;
    if agent.member_use(access.as_ref().map_or(0, |a| a.eligibility_revision())) != row.member_use {
        return Err(AppError::Forbidden("Specialist use access changed".into()));
    }
    Ok(access)
}

async fn resolve_access(
    db: &Database,
    actor: &str,
    owner: Option<&str>,
    agent: Option<&AssistantAgent>,
    key: Option<&str>,
) -> AppResult<Option<std::sync::Arc<RequestAccess>>> {
    let Some(owner) = owner.filter(|id| *id != actor) else {
        return Ok(None);
    };
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id":owner,"is_active":true})
        .await?
        .ok_or_else(super::shared_agent_service::missing)?;
    if !user.user_type.is_org() {
        let shared = super::shared_agent_service::require(
            db,
            actor,
            agent.ok_or_else(super::shared_agent_service::missing)?,
        )
        .await?;
        return Ok(Some(std::sync::Arc::new(RequestAccess::from_shared(
            shared,
        ))));
    }
    let acl = org_service::resolve_owner_access(db, actor, owner).await?;
    if !can_use(&acl) {
        return Err(AppError::Forbidden(
            "Organization agent access was revoked or is no longer permitted".into(),
        ));
    }
    let specialist = if let Some(agent) = agent {
        let rights = specialist_rights(actor, agent, &acl);
        if !rights.can_use {
            return Err(AppError::Forbidden(
                "Specialist use access is required".into(),
            ));
        }
        Some((agent.id.clone(), rights))
    } else {
        None
    };
    Ok(Some(std::sync::Arc::new(RequestAccess {
        actor: actor.into(),
        owner: owner.into(),
        acl,
        specialist,
        credential: key.map(str::to_owned),
        shared: None,
        channel: None,
    })))
}

#[cfg(test)]
pub(crate) async fn resolve_org_access(
    db: &Database,
    actor: &str,
    owner: &str,
    active: bool,
) -> AppResult<Option<std::sync::Arc<RequestAccess>>> {
    if !active {
        return Err(super::shared_agent_service::missing());
    }
    let acl = Box::pin(org_service::resolve_owner_access(db, actor, owner)).await?;
    if !can_use(&acl) {
        return Err(AppError::Forbidden(
            "Organization agent access was revoked or is no longer permitted".into(),
        ));
    }
    Ok(Some(std::sync::Arc::new(RequestAccess {
        actor: actor.into(),
        owner: owner.into(),
        acl,
        specialist: None,
        credential: None,
        shared: None,
        channel: None,
    })))
}

/// Reuse only an actor/owner-bound request snapshot. Internal callers without
/// authentication resolve once and can pass the result through their helpers.
pub(crate) async fn key_access_with_snapshot(
    db: &Database,
    actor: &str,
    owner: Option<&str>,
    snapshot: Option<&std::sync::Arc<RequestAccess>>,
) -> AppResult<Option<std::sync::Arc<RequestAccess>>> {
    if let Some(snapshot) = snapshot {
        bound_access(actor, owner, Some(snapshot.as_ref()))?;
        return Ok(Some(snapshot.clone()));
    }
    resolve_key_access(db, actor, owner).await
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
    chat_agent_with_access(db, chat)
        .await
        .map(|(agent, _)| agent)
}

/// Return the access resolved for the agent so permission requests can reuse it
/// in the acknowledgement transaction instead of resolving membership again.
pub(crate) async fn chat_agent_with_access(
    db: &Database,
    chat: &super::assistant_acknowledgement_service::ChatAuthority,
) -> AppResult<(AssistantAgent, Option<std::sync::Arc<RequestAccess>>)> {
    if let Some(access) = chat.org_agent_access.as_deref() {
        return Ok((
            access.agent(db, &chat.user_id, &chat.agent_id).await?,
            chat.org_agent_access.clone(),
        ));
    }
    let agent = db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(doc! {"_id": &chat.agent_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Agent not found".into()))?;
    if agent.user_id != chat.user_id && agent.is_nyxbot() {
        return Err(AppError::NotFound("Agent not found".into()));
    }
    let access = resolve_agent_access(db, &chat.user_id, &agent).await?;
    Ok((agent, access))
}

fn bound_access<'a>(
    actor: &str,
    owner: Option<&str>,
    snapshot: Option<&'a RequestAccess>,
) -> AppResult<Option<&'a RequestAccess>> {
    if let Some(snapshot) = snapshot.filter(|s| s.channel.is_some()) {
        snapshot.check(actor, owner.unwrap_or(actor))?;
        return Ok(Some(snapshot));
    }
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
    if let Some(shared) = access.shared() {
        let mut ids: Vec<_> = shared.connections().keys().cloned().collect();
        ids.sort();
        return Ok(Some(ids));
    }
    let owner = &access.owner;
    let rows: Vec<UserService> = db
        .collection(SERVICES)
        .find(doc! {"_id": {"$in": &key.allowed_service_ids}, "user_id": owner, "is_active": true})
        .await?
        .try_collect()
        .await?;
    Ok(Some(
        rows.into_iter()
            .filter(|s| access.allows_service(s))
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
    if access.shared().is_some() || access.channel().is_some_and(|c| c.execution().guest) {
        return Ok((false, Vec::new()));
    }
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
    if let Some(shared) = access.shared() {
        let id = instance.ok_or_else(super::shared_agent_service::missing)?;
        let service = db
            .collection::<UserService>(SERVICES)
            .find_one(doc! {"_id": id, "is_active": true})
            .await?
            .ok_or_else(super::shared_agent_service::missing)?;
        if !shared.allows_connection(&service) {
            return Err(super::shared_agent_service::missing());
        }
        return Ok(());
    }
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
    if row.is_none_or(|s| !access.allows_service(&s)) {
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
            // Resolving the immutable selector is not admission. Each operation checks
            // its own use or maintenance right at its protected entry point.
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
