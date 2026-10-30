use super::*;
use crate::{
    models::{
        assistant_agent::{AgentKind, AssistantAgent},
        assistant_sharing::{AgentShare, AgentSharing, ShareRole, ShareSubject},
        feature_flag_override::FeatureFlagOverride,
        org_membership::{MemberScopeSource, OrgMembership, OrgRole},
        org_role_scope::OrgRoleScope,
    },
    services::{self as svc, org_service::OwnerAccess},
};
use std::collections::{BTreeMap, HashSet};

/// Not a credential grant. Runtime must combine this binding with the existing
/// shared/org execution snapshot; service grants and consent are never enlarged.
pub struct AgentIdentity {
    pub(super) agent: AssistantAgent,
    pub(super) authority: Option<model::Authority>,
    pub(super) org_acl: Option<OwnerAccess>,
}
impl AgentIdentity {
    pub fn authority(&self) -> Option<&model::Authority> {
        self.authority.as_ref()
    }
    pub fn execution_user<'a>(&'a self, actor: &'a str) -> Option<&'a str> {
        self.authority.as_ref().map(|a| {
            if a.lane == model::Lane::Guest {
                a.owner_id.as_str()
            } else {
                actor
            }
        })
    }
}

/// One actor, group and optional immutable request chain. Fields are private;
/// callers cannot transplant this snapshot to another group/requester/agent.
pub struct RequestAccess {
    pub(super) group: AssistantGroup,
    pub(super) actor: String,
    pub(super) request_id: Option<String>,
    pub(super) agents: BTreeMap<String, AgentIdentity>,
    membership_ids: Vec<String>,
    active_org_ids: Vec<String>,
    pub(super) enabled_people: HashSet<String>,
}
impl RequestAccess {
    pub fn matches(&self, actor: &str, group: &str, request: Option<&str>) -> bool {
        self.actor == actor && self.group.id == group && self.request_id.as_deref() == request
    }
    pub fn agent(&self, id: &str) -> Option<&AgentIdentity> {
        self.agents.get(id)
    }
    pub fn membership_revision(&self) -> i64 {
        self.group
            .collaboration
            .as_ref()
            .expect("validated marker")
            .membership_revision
    }
}

pub async fn load(
    db: &Database,
    actor: &str,
    group_id: &str,
    request_id: Option<&str>,
) -> AppResult<RequestAccess> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await?;
    let group = read_group(db, actor, group_id, &mut session).await?;
    let mut access = for_group(db, actor, group, &mut session).await?;
    if let Some(id) = request_id {
        let request = db.collection::<crate::models::assistant_group::GroupRequest>(crate::models::assistant_group::REQUESTS_COLLECTION_NAME)
            .find_one(doc! {"_id":id,"group_id":group_id,"actor_user_id":actor,"collaboration.version":1,"dropped":{"$ne":true}}).session(&mut session).await?.ok_or_else(missing)?;
        let binding = request.collaboration.ok_or_else(missing)?;
        if binding.membership_revision != access.membership_revision() {
            return Err(stale());
        }
        access.request_id = Some(id.into());
    }
    session.commit_transaction().await?;
    Ok(access)
}

/// Bounded batched reads, including flags with live org overrides. None of these
/// reads is installed on unrelated personal or org-only turn paths.
pub(super) async fn for_group(
    db: &Database,
    actor: &str,
    group: AssistantGroup,
    session: &mut ClientSession,
) -> AppResult<RequestAccess> {
    marker(&group)?;
    if !group.participant_user_ids.iter().any(|p| p == actor) {
        return Err(missing());
    }
    let agents: Vec<AssistantAgent> = rows(
        db,
        crate::models::assistant_agent::COLLECTION_NAME,
        doc! {"_id":{"$in":&group.member_agent_ids}},
        session,
    )
    .await?;
    let mut people: HashSet<String> = agents.iter().map(|a| a.user_id.clone()).collect();
    people.extend([actor.into(), group.user_id.clone()]);
    let memberships: Vec<OrgMembership> = rows(
        db,
        crate::models::org_membership::COLLECTION_NAME,
        doc! {"member_user_id":{"$in":people.iter().collect::<Vec<_>>()},"revoked_at":null},
        session,
    )
    .await?;
    people.extend(memberships.iter().map(|m| m.org_user_id.clone()));
    let users: Vec<User> = rows(
        db,
        USERS,
        doc! {"_id":{"$in":people.iter().collect::<Vec<_>>()},"is_active":true},
        session,
    )
    .await?;
    let active_orgs: HashSet<_> = users
        .iter()
        .filter(|u| u.user_type.is_org())
        .map(|u| u.id.clone())
        .collect();
    let active_people: HashSet<_> = users
        .iter()
        .filter(|u| u.user_type.is_person())
        .map(|u| u.id.clone())
        .collect();
    let flags: Vec<FeatureFlagOverride> = rows(db, crate::models::feature_flag_override::COLLECTION_NAME,
        doc! {"flag_key":svc::shared_agent_service::FLAG,"definition_id":null,"$or":[{"org_user_id":null,"target_kind":"global"},{"target_kind":"user","target_key":{"$in":active_people.iter().collect::<Vec<_>>()}},{"org_user_id":{"$in":active_orgs.iter().collect::<Vec<_>>()},"target_kind":{"$in":["org","role"]}}]}, session).await?;
    let enabled = |id: &str| {
        active_people.contains(id)
            && svc::shared_agent_service::flag_enabled(&flags, &memberships, &active_orgs, id)
    };
    if !enabled(actor) || !enabled(&group.user_id) {
        return Err(missing());
    }
    let actor_orgs: Vec<_> = memberships
        .iter()
        .filter(|m| {
            m.member_user_id == actor && m.role.can_proxy() && active_orgs.contains(&m.org_user_id)
        })
        .map(|m| &m.org_user_id)
        .collect();
    let shares: Vec<AgentShare> = rows(db, crate::models::assistant_sharing::SHARES_COLLECTION_NAME,
        doc! {"agent_id":{"$in":&group.member_agent_ids},"$or":[{"subject_kind":"person","subject_id":actor},{"subject_kind":"org","subject_id":{"$in":&actor_orgs}}]}, session).await?;
    let policies: Vec<AgentSharing> = rows(
        db,
        crate::models::assistant_sharing::COLLECTION_NAME,
        doc! {"_id":{"$in":&group.member_agent_ids},"enabled":true},
        session,
    )
    .await?;
    let guests: Vec<model::AgentAccess> = rows(
        db,
        model::AGENT_ACCESS,
        doc! {"group_id":&group.id},
        session,
    )
    .await?;
    let scopes: Vec<OrgRoleScope> = rows(
        db,
        crate::models::org_role_scope::COLLECTION_NAME,
        doc! {"org_user_id":{"$in":actor_orgs}},
        session,
    )
    .await?;
    let mut identities = BTreeMap::new();
    for agent in agents {
        let mut org_acl = None;
        let live = agent.kind == AgentKind::Specialist && agent.destroyed_at.is_none();
        let authority = if !live {
            None
        } else if agent.user_id == actor {
            Some(model::Authority {
                owner_id: actor.into(),
                lane: model::Lane::Owner,
                revision: 0,
            })
        } else if active_orgs.contains(&agent.user_id) {
            memberships
                .iter()
                .find(|m| {
                    m.member_user_id == actor
                        && m.org_user_id == agent.user_id
                        && m.role.can_proxy()
                })
                .map(|m| {
                    let allowed_service_ids = match m.scope_source {
                        MemberScopeSource::Override => m.allowed_service_ids.clone(),
                        MemberScopeSource::Inherit => scopes
                            .iter()
                            .find(|s| s.org_user_id == m.org_user_id && s.role == m.role)
                            .and_then(|s| s.allowed_service_ids.clone()),
                    };
                    org_acl = Some(if m.role == OrgRole::Admin {
                        OwnerAccess::AsOrgAdmin {
                            org_user_id: agent.user_id.clone(),
                            membership_id: m.id.clone(),
                            allowed_service_ids,
                        }
                    } else {
                        OwnerAccess::AsOrgMember {
                            org_user_id: agent.user_id.clone(),
                            membership_id: m.id.clone(),
                            role: m.role,
                            allowed_service_ids,
                        }
                    });
                    model::Authority {
                        owner_id: agent.user_id.clone(),
                        lane: model::Lane::OrgMember,
                        revision: 0,
                    }
                })
        } else if !enabled(&agent.user_id) {
            None
        } else if let Some(policy) = policies.iter().find(|p| {
            p.agent_id == agent.id
                && p.owner_id == agent.user_id
                && shares.iter().any(|s| {
                    s.agent_id == agent.id
                        && s.owner_id == agent.user_id
                        && (s.subject_kind == ShareSubject::Person || s.role == ShareRole::User)
                })
        }) {
            Some(model::Authority {
                owner_id: agent.user_id.clone(),
                lane: model::Lane::Shared,
                revision: policy.revision,
            })
        } else {
            guests
                .iter()
                .find(|g| {
                    g.agent_id == agent.id
                        && g.owner_id == agent.user_id
                        && g.guest_enabled
                        && g.consenting_owner_id.as_deref() == Some(agent.user_id.as_str())
                })
                .map(|g| model::Authority {
                    owner_id: agent.user_id.clone(),
                    lane: model::Lane::Guest,
                    revision: g.revision,
                })
        };
        identities.insert(
            agent.id.clone(),
            AgentIdentity {
                agent,
                authority,
                org_acl,
            },
        );
    }
    Ok(RequestAccess {
        group,
        actor: actor.into(),
        request_id: None,
        agents: identities,
        membership_ids: memberships.iter().map(|m| m.id.clone()).collect(),
        active_org_ids: active_orgs.iter().cloned().collect(),
        enabled_people: active_people
            .iter()
            .filter(|id| enabled(id))
            .cloned()
            .collect(),
    })
}

/// Same-snapshot writes serialize admission with removal, unshare, destruction,
/// owner deactivation and the exact guest opt-in. Permission errors never select
/// a different lane; only a later human request may choose a new principal.
pub(super) async fn fence(
    db: &Database,
    access: &RequestAccess,
    agent_id: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    fence_group(db, &access.group, session).await?;
    fence_person(db, &access.actor, session).await?;
    // Org audiences and org-scoped flag overrides also participate in the
    // transaction fence. A concurrent membership removal cannot leave a shared
    // admission authorized by the transaction's earlier membership read.
    if !access.membership_ids.is_empty() {
        db.collection::<OrgMembership>(crate::models::org_membership::COLLECTION_NAME)
            .update_many(
                doc! {"_id":{"$in":&access.membership_ids},"revoked_at":null},
                doc! {"$inc":{"collaboration_fence":1}},
            )
            .session(&mut *session)
            .await?;
    }
    if !access.active_org_ids.is_empty() {
        db.collection::<User>(USERS)
            .update_many(
                doc! {"_id":{"$in":&access.active_org_ids},"is_active":true,"user_type":"org"},
                doc! {"$inc":{"collaboration_fence":1}},
            )
            .session(&mut *session)
            .await?;
    }
    let identity = access.agent(agent_id).ok_or_else(missing)?;
    let authority = identity
        .authority()
        .ok_or_else(|| AppError::Forbidden(REFUSAL.into()))?;
    if db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(doc! {"_id":agent_id,"user_id":&authority.owner_id,"destroyed_at":null,"kind":"specialist"},doc! {"$inc":{"thread_fence":1}}).session(&mut *session).await?.matched_count != 1 { return Err(missing()); }
    if authority.lane != model::Lane::OrgMember {
        fence_person(db, &authority.owner_id, session).await?;
    }
    match authority.lane {
        model::Lane::Shared => {
            if db.collection::<AgentSharing>(crate::models::assistant_sharing::COLLECTION_NAME).update_one(
                doc! {"_id":agent_id,"owner_id":&authority.owner_id,"enabled":true,"revision":authority.revision},doc! {"$inc":{"execution_fence":1}}).session(&mut *session).await?.matched_count != 1 { return Err(missing()); }
        }
        model::Lane::Guest => {
            if db.collection::<model::AgentAccess>(model::AGENT_ACCESS).update_one(
                doc! {"group_id":&access.group.id,"agent_id":agent_id,"owner_id":&authority.owner_id,"guest_enabled":true,"revision":authority.revision},doc! {"$inc":{"execution_fence":1}}).session(&mut *session).await?.matched_count != 1 { return Err(missing()); }
        }
        model::Lane::OrgMember => {
            let membership_id = match identity.org_acl.as_ref() {
                Some(OwnerAccess::AsOrgAdmin {membership_id,..}) | Some(OwnerAccess::AsOrgMember {membership_id,..}) => membership_id,
                _ => return Err(missing()),
            };
            if db.collection::<OrgMembership>(crate::models::org_membership::COLLECTION_NAME).update_one(
                doc! {"_id":membership_id,"revoked_at":null,"role":{"$in":["admin","member"]}},doc! {"$inc":{"collaboration_fence":1}}).session(&mut *session).await?.matched_count != 1 { return Err(missing()); }
            if db.collection::<User>(USERS).update_one(doc! {"_id":&authority.owner_id,"is_active":true,"user_type":"org"},doc! {"$inc":{"collaboration_fence":1}}).session(&mut *session).await?.matched_count != 1 { return Err(missing()); }
        }
        model::Lane::Owner => {}
    }
    Ok(())
}
