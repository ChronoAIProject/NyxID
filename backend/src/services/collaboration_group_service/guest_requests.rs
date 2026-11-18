//! A request is a notification, never execution authority. The owner still
//! enables the existing group-specific guest policy through GuestAccess.
use super::*;
use crate::services::{
    api_key_mutation_service as transactions,
    assistant_links::{AGENT_DETAILS_GRANTS, AssistantPage},
};

pub fn pending<'a>(
    policy: &'a model::AgentAccess,
    group: &AssistantGroup,
) -> Option<&'a model::GuestAccessRequest> {
    policy.guest_request.as_ref().filter(|r| {
        r.status == "pending"
            && r.expires_at > Utc::now()
            && group.participant_user_ids.contains(&r.requested_by)
    })
}

pub fn validate_pending(
    policy: &model::AgentAccess,
    group: &AssistantGroup,
    id: &str,
) -> AppResult<()> {
    if pending(policy, group).is_none_or(|r| r.id != id) {
        return Err(AppError::Conflict("This permission request expired or was cancelled; reload the agent's group access settings".into()));
    }
    Ok(())
}

pub fn projection(
    policy: &model::AgentAccess,
    group: &AssistantGroup,
    actor: &str,
) -> Option<serde_json::Value> {
    policy.guest_request.as_ref().map(|r| {
        let status = if r.status == "pending" && pending(policy, group).is_none() {"expired"} else {&r.status};
        serde_json::json!({"id":r.id,"status":status,"expires_at":r.expires_at,
            "can_cancel":status=="pending" && (actor==r.requested_by || actor==group.user_id || actor==policy.owner_id)})
    })
}

/// Preflight before resolving the owner's own notification destination.
pub async fn target(db: &Database, actor: &str, group: &str, agent: &str) -> AppResult<String> {
    require_available(db, actor).await?;
    let group = get(db, actor, group).await?;
    if !group.member_agent_ids.iter().any(|id| id == agent) {
        return Err(missing());
    }
    let policy = db
        .collection::<model::AgentAccess>(model::AGENT_ACCESS)
        .find_one(doc! {"group_id": &group.id, "agent_id": agent})
        .await?
        .ok_or_else(missing)?;
    if db
        .collection::<crate::models::assistant_agent::AssistantAgent>(
            crate::models::assistant_agent::COLLECTION_NAME,
        )
        .find_one(
            doc! {"_id":agent,"user_id":&policy.owner_id,"destroyed_at":null,"kind":"specialist"},
        )
        .await?
        .is_none()
    {
        return Err(missing());
    }
    let owner = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &policy.owner_id, "is_active":true})
        .await?
        .ok_or_else(missing)?;
    if !owner.user_type.is_person() {
        return Err(invalid(
            "Organization agents cannot allow guests; ask an organization administrator for membership access",
        ));
    }
    if actor == owner.id {
        return Err(invalid(
            "You own this agent; enable participant access in its settings",
        ));
    }
    Ok(owner.id)
}

/// Queue the owner event in the same transaction as a new request; duplicates
/// do not notify twice, including simultaneous asks from different participants.
pub async fn ask(
    db: &Database,
    actor: &str,
    group: &str,
    agent: &str,
    notification_thread: &str,
) -> AppResult<(model::GuestAccessRequest, bool)> {
    require_available(db, actor).await?;
    membership::rate_limit(db, actor).await?;
    let mut session = db.client().start_session().await?;
    let result = session.start_transaction().and_run(
        (db.clone(), actor.to_owned(), group.to_owned(), agent.to_owned(), notification_thread.to_owned()),
        |session, context| {
        let (db, actor, group, agent, notification_thread) = &*context;
        let (actor, group, agent, notification_thread) = (actor.as_str(), group.as_str(), agent.as_str(), notification_thread.as_str());
        Box::pin(async move {
        let work: AppResult<(model::GuestAccessRequest, bool)> = async {
            let group = read_group(db, actor, group, session).await?;
            let access = identity::for_group(db, actor, group, session).await?;
            let identity = access.agent(agent).ok_or_else(missing)?;
            if identity.agent.destroyed_at.is_some() { return Err(missing()); }
            let owner = &identity.agent.user_id;
            person(db, owner, session).await?;
            if !access.enabled_people.contains(owner) { return Err(missing()); }
            if owner==actor { return Err(invalid("You own this agent; enable participant access in its settings")); }
            let policy = db.collection::<model::AgentAccess>(model::AGENT_ACCESS)
                .find_one(doc! {"group_id":&access.group.id,"agent_id":agent,"owner_id":owner})
                .session(&mut *session).await?.ok_or_else(missing)?;
            if policy.guest_enabled { return Err(invalid("Participant access is already enabled")); }
            fence_person(db, actor, session).await?;
            fence_person(db, owner, session).await?;
            fence_group(db, &access.group, session).await?;
            if let Some(request) = pending(&policy, &access.group) { return Ok((request.clone(), false)); }
            let now=Utc::now();
            let request=model::GuestAccessRequest {id:uuid::Uuid::new_v4().to_string(),requested_by:actor.into(),status:"pending".into(),created_at:now,expires_at:now+chrono::Duration::days(7)};
            let link=AssistantPage::AgentDetails {agent, tab:AGENT_DETAILS_GRANTS}.path();
            let mut event=super::super::assistant_team_service::event("group_guest_access_requested",
                format!("A group participant requests permission to use your agent in group {}. Review and approve in {link}. Approval enables guest access for current and future participants under your guest service policy, with your identity and billing and no guest action cards. Only you may decide. Request ID: {}. Do not enable automatically.",access.group.id,request.id),Some(agent));
            event.id=request.id.clone();
            let destination=db.collection::<crate::models::assistant_conversation::AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
                .find_one(doc! {"_id":notification_thread,"user_id":owner,"role":"orchestrator","collaboration":null,"group_id":null}).session(&mut *session).await?.ok_or_else(missing)?;
            if !super::super::assistant_nyxagent::append_events_in_session(db,owner,&destination.id,&[event],session).await? { return Err(missing()); }
            if db.collection::<model::AgentAccess>(model::AGENT_ACCESS).update_one(doc! {"_id":&policy.id,"revision":policy.revision},
                doc! {"$set":{"guest_request":bson::to_bson(&request).map_err(|_|AppError::Internal("Request encoding failed".into()))?}})
                .session(&mut *session).await?.matched_count!=1 {return Err(stale());}
            Ok((request,true))
        }.await;
        transactions::transaction_result(work)
    })}).await.map_err(transactions::map_transaction_error)?;
    if result.1 {
        audit_request(db, actor, group, agent, &result.0.id, "requested").await;
    }
    Ok(result)
}

pub async fn cancel(
    db: &Database,
    actor: &str,
    group: &str,
    agent: &str,
    request: &str,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run(
        (db.clone(), actor.to_owned(), group.to_owned(), agent.to_owned(), request.to_owned()),
        |session, context| {
        let (db, actor, group, agent, request) = &*context;
        let (actor, group, agent, request) = (actor.as_str(), group.as_str(), agent.as_str(), request.as_str());
        Box::pin(async move {
        let work: AppResult<()> = async {
            let group=db.collection::<AssistantGroup>(GROUPS).find_one(doc! {"_id":group,"collaboration.version":1}).session(&mut *session).await?.ok_or_else(missing)?;
            marker(&group)?;
            let policy=db.collection::<model::AgentAccess>(model::AGENT_ACCESS).find_one(doc! {"group_id":&group.id,"agent_id":agent}).session(&mut *session).await?.ok_or_else(missing)?;
            let row=policy.guest_request.as_ref().filter(|r|r.id==request).ok_or_else(missing)?;
            if actor!=policy.owner_id && (actor!=group.user_id && actor!=row.requested_by || !group.participant_user_ids.iter().any(|p|p==actor)) {return Err(missing());}
            if row.status != "cancelled" { validate_pending(&policy, &group, request)?; }
            fence_person(db,actor,session).await?;
            fence_group(db,&group,session).await?;
            db.collection::<model::AgentAccess>(model::AGENT_ACCESS).update_one(doc! {"_id":&policy.id,"guest_request.id":request,"guest_request.status":"pending"},doc! {"$set":{"guest_request.status":"cancelled"}}).session(&mut *session).await?;
            Ok(())
        }.await;
        transactions::transaction_result(work)
    })}).await.map_err(transactions::map_transaction_error)?;
    audit_request(db, actor, group, agent, request, "cancelled").await;
    Ok(())
}

pub(super) async fn audit_request(
    db: &Database,
    actor: &str,
    group: &str,
    agent: &str,
    request: &str,
    change: &str,
) {
    let _=super::super::audit_service::log_actor_event(db.clone(),&super::super::audit_service::AuditActor {
        user_id:actor.into(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None,
    },"assistant_group_guest_access_requested",Some(serde_json::json!({"actor_id":actor,"group_id":group,"agent_id":agent,"request_id":request,"change":change}))).await;
}
