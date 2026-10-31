use super::*;
use crate::services::{self as svc, api_key_mutation_service as transactions};
use chrono::{DateTime, Duration};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

pub enum Change {
    Invite {
        intended_user_id: Option<String>,
    },
    CancelInvite {
        invite_id: String,
    },
    QuickAdd {
        person_id: String,
        history_disclosed: bool,
    },
    Remove {
        person_id: String,
    },
    Leave,
    Delete,
    ProposeTransfer {
        person_id: String,
    },
    AcceptTransfer,
    AddAgent {
        agent_id: String,
        /// Server-resolved from an authenticated participant proposal, or the
        /// creator for a direct add. Never accept this principal from tool args.
        proposed_by: String,
    },
    ProposeAgent {
        agent_id: String,
    },
    AcceptAgent {
        proposal_id: String,
    },
    RemoveAgent {
        agent_id: String,
    },
    GuestAccess {
        agent_id: String,
        enabled: bool,
        expected_revision: i64,
    },
    Rename {
        name: String,
    },
    Lead {
        agent_id: String,
    },
}

pub struct IssuedInvite {
    pub id: String,
    pub token: Zeroizing<String>,
    pub expires_at: DateTime<Utc>,
    pub history_disclosure: &'static str,
}
impl std::fmt::Debug for IssuedInvite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuedGroupInvite").finish_non_exhaustive()
    }
}
pub struct Mutation {
    pub group: Option<AssistantGroup>,
    pub invite: Option<IssuedInvite>,
}

fn token_hash(token: &str) -> AppResult<String> {
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(missing());
    }
    Ok(hex::encode(Sha256::digest(token.as_bytes())))
}

fn name(value: &str) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > crate::models::assistant_group::MAX_NAME_CHARS {
        return Err(invalid("A group name has 1 to 60 characters"));
    }
    Ok(value.into())
}

pub async fn create(
    db: &Database,
    actor: &str,
    title: &str,
    agents: &[String],
) -> AppResult<AssistantGroup> {
    require_rollout(db).await?;
    create_ready(db, actor, title, agents).await
}

pub(super) async fn create_ready(
    db: &Database,
    actor: &str,
    title: &str,
    agents: &[String],
) -> AppResult<AssistantGroup> {
    let now = Utc::now();
    let group = AssistantGroup {
        id: format!("nyxg-{}", Uuid::new_v4().simple()),
        user_id: actor.into(),
        collaboration: Some(model::Collaboration {
            version: model::VERSION,
            creator_user_id: actor.into(),
            membership_revision: 1,
        }),
        participant_user_ids: vec![actor.into()],
        created_by_user_id: None,
        name: name(title)?,
        member_agent_ids: agents.to_vec(),
        lead_agent_id: agents.first().cloned().unwrap_or_default(),
        last_recipient_ids: vec![],
        last_recipient_ids_by_actor: Default::default(),
        created_by: "user".into(),
        message_count: 0,
        pending_agent_ids: vec![],
        hops_remaining: 0,
        followers: vec![],
        pending_checked_at: None,
        last_message_at: None,
        created_at: now,
        updated_at: now,
    };
    marker(&group)?;
    let mut session = db.client().start_session().await?;
    let group = session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .and_run(
            (db.clone(), actor.to_owned(), group, agents.to_vec()),
            |session, context| {
                let (db, actor, group, agents) = &*context;
                let actor = actor.as_str();
                Box::pin(async move {
                    let result: AppResult<AssistantGroup> = async {
                        fence_person(db, actor, session).await?;
                        let access = identity::for_group(db, actor, group.clone(), session).await?;
                        for id in agents {
                            let target = access
                                .agent(id)
                                .and_then(AgentIdentity::authority)
                                .ok_or_else(|| invalid("Choose specialists you can use"))?;
                            if target.lane == model::Lane::Guest {
                                return Err(invalid("Choose specialists you can use"));
                            }
                            insert_agent(db, group, id, &target.owner_id, actor, session).await?;
                        }
                        db.collection::<AssistantGroup>(GROUPS)
                            .insert_one(group)
                            .session(session)
                            .await?;
                        Ok(group.clone())
                    }
                    .await;
                    transactions::transaction_result(result)
                })
            },
        )
        .await
        .map_err(transactions::map_transaction_error)?;
    audit(db, actor, &group.id, "created").await;
    Ok(group)
}

async fn insert_agent(
    db: &Database,
    group: &AssistantGroup,
    agent_id: &str,
    owner: &str,
    proposer: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<model::AgentAccess>(model::AGENT_ACCESS)
        .insert_one(model::AgentAccess {
            id: Uuid::new_v4().to_string(),
            group_id: group.id.clone(),
            agent_id: agent_id.into(),
            owner_id: owner.into(),
            added_by_user_id: proposer.into(),
            membership_generation: marker(group)?.membership_revision,
            guest_enabled: false,
            consenting_owner_id: None,
            revision: 1,
            updated_at: Utc::now(),
        })
        .session(session)
        .await?;
    Ok(())
}

async fn rate_limit(db: &Database, actor: &str) -> AppResult<()> {
    if !svc::coordination_service::RateWindowStore::admit(
        db,
        "collaboration_group_writes",
        actor,
        60,
        std::time::Duration::from_secs(3600),
    )
    .await?
    .allowed
    {
        return Err(invalid("Too many group changes; try again later"));
    }
    Ok(())
}

pub async fn apply(
    db: &Database,
    actor: &str,
    id: &str,
    expected_revision: i64,
    change: Change,
    delegation_approved: bool,
) -> AppResult<Mutation> {
    // Revocation and departure remain available when the feature/readiness is off.
    if !matches!(
        change,
        Change::Leave
            | Change::Delete
            | Change::Remove { .. }
            | Change::RemoveAgent { .. }
            | Change::CancelInvite { .. }
            | Change::GuestAccess { enabled: false, .. }
    ) {
        require_rollout(db).await?;
        rate_limit(db, actor).await?;
    }
    apply_ready(
        db,
        actor,
        id,
        expected_revision,
        change,
        delegation_approved,
    )
    .await
}

pub(super) async fn apply_ready(
    db: &Database,
    actor: &str,
    id: &str,
    expected_revision: i64,
    change: Change,
    delegation_approved: bool,
) -> AppResult<Mutation> {
    let raw_token = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
    let hash = token_hash(&raw_token)?;
    let mut session = db.client().start_session().await?;
    let (group, invite) = session.start_transaction().read_concern(mongodb::options::ReadConcern::snapshot()).and_run((db.clone(), actor.to_owned(), id.to_owned(), expected_revision, change, delegation_approved, hash.clone()), |session, context| {let (db, actor, id, expected_revision, change, delegation_approved, hash)=&*context;let actor=actor.as_str();let id=id.as_str();let expected_revision=*expected_revision;let delegation_approved=*delegation_approved;Box::pin(async move {
        let result: AppResult<(Option<AssistantGroup>, Option<model::Invite>)> = async {
            // Owners may opt in an attached agent without acquiring transcript access.
            let mut group = if matches!(change, Change::GuestAccess {..}) {
                let g = db.collection::<AssistantGroup>(GROUPS).find_one(doc! {"_id":id,"collaboration.version":1}).session(&mut *session).await?.ok_or_else(missing)?;
                marker(&g)?;
                if g.user_id!=actor {
                    let Change::GuestAccess {agent_id,..}=&change else {return Err(missing());};
                    if db.collection::<model::AgentAccess>(model::AGENT_ACCESS).find_one(doc! {"group_id":id,"agent_id":agent_id,"owner_id":actor}).session(&mut *session).await?.is_none() {return Err(missing());}
                }
                g
            } else { read_group(db,actor,id,session).await? };
            if marker(&group)?.membership_revision != expected_revision { return Err(stale()); }
            fence_person(db,actor,session).await?;
            fence_group(db,&group,session).await?;
            let creator = group.user_id == actor;
            if !creator && !matches!(change,Change::Leave | Change::AcceptTransfer | Change::ProposeAgent {..} | Change::GuestAccess {..}) { return Err(missing()); }
            if !matches!(change,Change::Leave | Change::Delete | Change::Remove {..} | Change::RemoveAgent {..} | Change::CancelInvite {..} | Change::GuestAccess {enabled:false,..}) {
                // Uses the same batched live flag/participant snapshot as admission.
                identity::for_group(db,&group.user_id,group.clone(),session).await?;
            }
            let mut changed = false;
            match &change {
                Change::Invite {intended_user_id} => {
                    if let Some(person_id) = intended_user_id { person(db,person_id,session).await?; }
                    let count = db.collection::<model::Invite>(model::INVITES).count_documents(doc! {"group_id":id,"accepted_at":null,"cancelled_at":null,"expires_at":{"$gt":bson::DateTime::now()}}).session(&mut *session).await?;
                    if count >= 32 { return Err(invalid("This group already has 32 pending invites")); }
                    let now = Utc::now();
                    let invite = model::Invite { id:Uuid::new_v4().to_string(), group_id:id.into(), issuer_user_id:actor.into(),token_hash:hash.clone(),
                        intended_user_id:intended_user_id.clone(),created_at:now,expires_at:now+Duration::days(7),accepted_at:None,accepted_by_user_id:None,cancelled_at:None };
                    db.collection::<model::Invite>(model::INVITES).insert_one(&invite).session(session).await?;
                    return Ok((Some(group),Some(invite)));
                }
                Change::CancelInvite {invite_id} => {
                    db.collection::<model::Invite>(model::INVITES).update_one(doc! {"_id":invite_id,"group_id":id,"accepted_at":null},doc! {"$set":{"cancelled_at":bson::DateTime::now()}}).session(&mut *session).await?;
                }
                Change::QuickAdd {person_id,history_disclosed} => {
                    if !history_disclosed { return Err(invalid(HISTORY_DISCLOSURE)); }
                    if !group.participant_user_ids.contains(person_id) {
                        if group.participant_user_ids.len() >= 16 { return Err(invalid("A group can have at most 16 people")); }
                        fence_person(db,person_id,session).await?;
                        common_org(db,actor,person_id,session).await?;
                        group.participant_user_ids.push(person_id.clone());
                        identity::for_group(db,person_id,group.clone(),session).await?;
                        changed = true;
                    }
                }
                Change::Remove {person_id} => {
                    if person_id == &group.user_id { return Err(invalid("Transfer management to an accepting participant, or delete the group")); }
                    changed = group.participant_user_ids.contains(person_id);
                    group.participant_user_ids.retain(|id| id != person_id);
                    group.last_recipient_ids_by_actor.remove(person_id);
                }
                Change::Leave => {
                    if creator {
                        if group.participant_user_ids.len() != 1 { return Err(invalid("Transfer management to an accepting participant, or delete the group")); }
                        delete(db,&group,session).await?;
                        return Ok((None,None));
                    }
                    group.participant_user_ids.retain(|id| id != actor);
                    group.last_recipient_ids_by_actor.remove(actor);
                    changed = true;
                }
                Change::Delete => {
                    delete(db,&group,session).await?;
                    return Ok((None,None));
                }
                Change::ProposeTransfer {person_id} => {
                    if person_id == actor || !group.participant_user_ids.contains(person_id) { return Err(invalid("Choose another current participant")); }
                    fence_person(db,person_id,session).await?;
                    let proposal = model::Transfer {group_id:id.into(),from_user_id:actor.into(),to_user_id:person_id.clone(),membership_revision:expected_revision,expires_at:Utc::now()+Duration::days(7)};
                    db.collection::<model::Transfer>(model::TRANSFERS).replace_one(doc! {"_id":id},proposal).upsert(true).session(&mut *session).await?;
                }
                Change::AcceptTransfer => {
                    let proposal = db.collection::<model::Transfer>(model::TRANSFERS).find_one(doc! {"_id":id,"to_user_id":actor,"from_user_id":&group.user_id,"membership_revision":expected_revision,"expires_at":{"$gt":bson::DateTime::now()}}).session(&mut *session).await?.ok_or_else(missing)?;
                    fence_person(db,&proposal.from_user_id,session).await?;
                    identity::for_group(db,actor,group.clone(),session).await?;
                    group.user_id = actor.into();
                    group.collaboration.as_mut().expect("validated").creator_user_id = actor.into();
                    // Old creator-issued bearer links cannot survive a transfer.
                    db.collection::<model::Invite>(model::INVITES).update_many(doc! {"group_id":id,"accepted_at":null},doc! {"$set":{"cancelled_at":bson::DateTime::now()}}).session(&mut *session).await?;
                    changed = true;
                }
                Change::AddAgent {agent_id,proposed_by} => {
                    if !group.member_agent_ids.contains(agent_id) {
                        if group.member_agent_ids.len() >= 8 { return Err(invalid("A group can have at most 8 specialists")); }
                        if !group.participant_user_ids.contains(proposed_by) { return Err(missing()); }
                        group.member_agent_ids.push(agent_id.clone());
                        let access = identity::for_group(db,proposed_by,group.clone(),session).await?;
                        let authority = access.agent(agent_id).and_then(AgentIdentity::authority).filter(|a| a.lane != model::Lane::Guest).ok_or_else(|| invalid("The proposing participant must be able to use this specialist"))?;
                        identity::fence(db,&access,agent_id,session).await?;
                        insert_agent(db,&group,agent_id,&authority.owner_id,proposed_by,session).await?;
                        changed = true;
                    }
                }
                Change::ProposeAgent {agent_id} => {
                    if group.member_agent_ids.contains(agent_id) {return Err(invalid("Agent is already a member"));}
                    let mut proposed = group.clone();
                    // Identity lookup is bounded to eight members; replace the roster
                    // for proposal validation rather than expanding a full group.
                    proposed.member_agent_ids=vec![agent_id.clone()];proposed.lead_agent_id=agent_id.clone();
                    let access=identity::for_group(db,actor,proposed,session).await?;
                    let authority=access.agent(agent_id).and_then(AgentIdentity::authority).filter(|a|a.lane!=model::Lane::Guest).ok_or_else(missing)?;
                    let _ = authority;
                    identity::fence(db,&access,agent_id,session).await?;
                    let proposal=model::AgentProposal {id:format!("{id}:{actor}:{agent_id}"),group_id:id.into(),agent_id:agent_id.clone(),proposed_by:actor.into(),membership_revision:expected_revision,expires_at:Utc::now()+Duration::days(7)};
                    db.collection::<model::AgentProposal>(model::PROPOSALS).replace_one(doc! {"_id":&proposal.id},&proposal).upsert(true).session(&mut *session).await?;
                }
                Change::AcceptAgent {proposal_id} => {
                    let proposal=db.collection::<model::AgentProposal>(model::PROPOSALS).find_one(doc! {"_id":proposal_id,"group_id":id,"membership_revision":expected_revision,"expires_at":{"$gt":bson::DateTime::now()}}).session(&mut *session).await?.ok_or_else(missing)?;
                    if group.member_agent_ids.len()>=8 {return Err(invalid("A group can have at most 8 specialists"));}
                    if !group.participant_user_ids.contains(&proposal.proposed_by) || group.member_agent_ids.contains(&proposal.agent_id) {return Err(stale());}
                    group.member_agent_ids.push(proposal.agent_id.clone());
                    let access=identity::for_group(db,&proposal.proposed_by,group.clone(),session).await?;
                    let authority=access.agent(&proposal.agent_id).and_then(AgentIdentity::authority).filter(|a|a.lane!=model::Lane::Guest).ok_or_else(missing)?;
                    identity::fence(db,&access,&proposal.agent_id,session).await?;
                    insert_agent(db,&group,&proposal.agent_id,&authority.owner_id,&proposal.proposed_by,session).await?;
                    if db.collection::<model::AgentProposal>(model::PROPOSALS).delete_one(doc! {"_id":proposal_id,"membership_revision":expected_revision}).session(&mut *session).await?.deleted_count!=1 {return Err(stale());}
                    changed=true;
                }
                Change::RemoveAgent {agent_id} => {
                    if group.member_agent_ids.contains(agent_id) {
                        if group.member_agent_ids.len() == 1 { return Err(invalid("Keep at least one specialist, or delete the group")); }
                        group.member_agent_ids.retain(|id| id != agent_id);
                        if group.lead_agent_id == *agent_id { group.lead_agent_id = group.member_agent_ids[0].clone(); }
                        for ids in group.last_recipient_ids_by_actor.values_mut() { ids.retain(|id| id != agent_id); }
                        db.collection::<model::AgentAccess>(model::AGENT_ACCESS).delete_one(doc! {"group_id":id,"agent_id":agent_id}).session(&mut *session).await?;
                        changed = true;
                    }
                }
                Change::GuestAccess {agent_id,enabled,expected_revision:guest_revision} => {
                    if !group.member_agent_ids.contains(agent_id) { return Err(missing()); }
                    let agent = db.collection::<crate::models::assistant_agent::AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
                        .find_one(doc! {"_id":agent_id,"kind":"specialist","destroyed_at":null}).session(&mut *session).await?.ok_or_else(missing)?;
                    if actor != agent.user_id && (!creator || *enabled) { return Err(missing()); }
                    // Organization specialists can never opt into guest publishing.
                    person(db,&agent.user_id,session).await?;
                    let previous = db.collection::<model::AgentAccess>(model::AGENT_ACCESS).find_one(doc! {"group_id":id,"agent_id":agent_id,"owner_id":&agent.user_id,"revision":guest_revision}).session(&mut *session).await?.ok_or_else(stale)?;
                    if *enabled && !identity::for_group(db,&group.user_id,group.clone(),session).await?.enabled_people.contains(actor) { return Err(missing()); }
                    if previous.guest_enabled != *enabled {
                        // REST is a first-party human decision. A native tool must
                        // consume an owner delegation card for these exact args,
                        // including both revisions, before setting this flag.
                        if *enabled && !delegation_approved {
                            return Err(AppError::Forbidden("Enabling guest access requires the agent owner's approval".into()));
                        }
                        if db.collection::<model::AgentAccess>(model::AGENT_ACCESS).update_one(doc! {"_id":previous.id,"revision":guest_revision},doc! {"$set":{"guest_enabled":enabled,"consenting_owner_id":if *enabled {bson::Bson::String(actor.into())} else {bson::Bson::Null},"updated_at":bson::DateTime::now()},"$inc":{"revision":1}}).session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
                        invalidate(db,id,session).await?;
                    }
                }
                Change::Rename {name:title} => { group.name = name(title)?; }
                Change::Lead {agent_id} => {
                    if !group.member_agent_ids.contains(agent_id) { return Err(invalid("Choose a current group member as the lead")); }
                    group.lead_agent_id = agent_id.clone();
                }
            }
            if changed {
                group.collaboration.as_mut().expect("validated").membership_revision += 1;
                invalidate(db,id,session).await?;
                db.collection::<model::Transfer>(model::TRANSFERS).delete_one(doc! {"_id":id}).session(&mut *session).await?;
            }
            marker(&group)?;
            group.updated_at = Utc::now();
            if db.collection::<AssistantGroup>(GROUPS).update_one(doc! {"_id":id,"collaboration.membership_revision":expected_revision},doc! {"$set":{
                "user_id":&group.user_id,"collaboration":bson::to_bson(&group.collaboration).map_err(|e| AppError::Internal(e.to_string()))?,
                "participant_user_ids":&group.participant_user_ids,"member_agent_ids":&group.member_agent_ids,"lead_agent_id":&group.lead_agent_id,
                "last_recipient_ids_by_actor":bson::to_bson(&group.last_recipient_ids_by_actor).map_err(|e| AppError::Internal(e.to_string()))?,"name":&group.name,"updated_at":bson::DateTime::from_chrono(group.updated_at)
            }}).session(session).await?.matched_count != 1 { return Err(stale()); }
            Ok((Some(group),None))
        }.await;
        transactions::transaction_result(result)
    })}).await.map_err(transactions::map_transaction_error)?;
    audit(db, actor, id, "membership_updated").await;
    Ok(Mutation {
        group,
        invite: invite.map(|invite| IssuedInvite {
            id: invite.id,
            token: raw_token,
            expires_at: invite.expires_at,
            history_disclosure: HISTORY_DISCLOSURE,
        }),
    })
}

async fn common_org(
    db: &Database,
    creator: &str,
    candidate: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    use crate::models::org_membership::{COLLECTION_NAME, OrgMembership};
    let memberships: Vec<OrgMembership> = rows(
        db,
        COLLECTION_NAME,
        doc! {"member_user_id":{"$in":[creator,candidate]},"revoked_at":null},
        session,
    )
    .await?;
    let orgs: Vec<_> = memberships
        .iter()
        .filter(|m| {
            m.member_user_id == creator
                && memberships.iter().any(|other| {
                    other.member_user_id == candidate && other.org_user_id == m.org_user_id
                })
        })
        .map(|m| &m.org_user_id)
        .collect();
    let org = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id":{"$in":orgs},"is_active":true,"user_type":"org"})
        .session(&mut *session)
        .await?
        .ok_or_else(|| invalid("Quick-add requires a shared active organization"))?;
    db.collection::<User>(USERS)
        .update_one(
            doc! {"_id":&org.id,"is_active":true},
            doc! {"$inc":{"collaboration_fence":1}},
        )
        .session(&mut *session)
        .await?;
    // Viewer qualifies as a person, never as an org-agent user.
    let ids: Vec<_> = memberships
        .iter()
        .filter(|m| m.org_user_id == org.id)
        .map(|m| &m.id)
        .collect();
    db.collection::<OrgMembership>(COLLECTION_NAME)
        .update_many(
            doc! {"_id":{"$in":ids},"revoked_at":null},
            doc! {"$inc":{"collaboration_fence":1}},
        )
        .session(session)
        .await?;
    Ok(())
}

pub struct InvitePreview {
    pub creator_name: String,
    pub participants: Vec<crate::services::org_group_service::Participant>,
    pub group_id: String,
    pub group_name: String,
    pub history_disclosure: &'static str,
    pub expires_at: DateTime<Utc>,
}

async fn invite_in_session(
    db: &Database,
    actor: &str,
    hash: &str,
    session: &mut ClientSession,
) -> AppResult<(model::Invite, AssistantGroup)> {
    person(db, actor, session).await?;
    let invite = db.collection::<model::Invite>(model::INVITES).find_one(doc! {"token_hash":hash,"accepted_at":null,"cancelled_at":null,"expires_at":{"$gt":bson::DateTime::now()},"$or":[{"intended_user_id":null},{"intended_user_id":actor}]}).session(&mut *session).await?.ok_or_else(missing)?;
    let group = db.collection::<AssistantGroup>(GROUPS).find_one(doc! {"_id":&invite.group_id,"user_id":&invite.issuer_user_id,"collaboration.version":1}).session(&mut *session).await?.ok_or_else(missing)?;
    marker(&group)?;
    Ok((invite, group))
}

pub async fn preview_invite(db: &Database, actor: &str, token: &str) -> AppResult<InvitePreview> {
    let hash = token_hash(token)?;
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await?;
    let (invite, group) = invite_in_session(db, actor, &hash, &mut session).await?;
    session.commit_transaction().await?;
    Ok(InvitePreview {
        creator_name: svc::org_group_service::display_name(db, &group.user_id).await?,
        participants: super::participants(db, &group).await?,
        group_id: group.id,
        group_name: group.name,
        expires_at: invite.expires_at,
        history_disclosure: HISTORY_DISCLOSURE,
    })
}

pub async fn accept_invite(
    db: &Database,
    actor: &str,
    token: &str,
    history_disclosed: bool,
) -> AppResult<AssistantGroup> {
    require_rollout(db).await?;
    rate_limit(db, actor).await?;
    accept_ready(db, actor, token, history_disclosed).await
}

pub(super) async fn accept_ready(
    db: &Database,
    actor: &str,
    token: &str,
    history_disclosed: bool,
) -> AppResult<AssistantGroup> {
    if !history_disclosed {
        return Err(invalid(HISTORY_DISCLOSURE));
    }
    let hash = token_hash(token)?;
    let mut session = db.client().start_session().await?;
    let group = session.start_transaction().read_concern(mongodb::options::ReadConcern::snapshot()).and_run((db.clone(), actor.to_owned(), hash), |session, context| {let (db, actor, hash)=&*context;let actor=actor.as_str();Box::pin(async move {
        let result: AppResult<AssistantGroup> = async {
            let (invite, mut group) = invite_in_session(db,actor,hash,session).await?;
            fence_person(db,actor,session).await?;
            fence_person(db,&group.user_id,session).await?;
            fence_group(db,&group,session).await?;
            if group.participant_user_ids.contains(&actor.to_owned()) { return Err(invalid("You already participate in this group")); }
            if group.participant_user_ids.len() >= 16 { return Err(invalid("A group can have at most 16 people")); }
            let previous_revision = marker(&group)?.membership_revision;
            group.participant_user_ids.push(actor.into());
            identity::for_group(db,actor,group.clone(),session).await?;
            group.collaboration.as_mut().expect("validated").membership_revision += 1;
            if db.collection::<model::Invite>(model::INVITES).update_one(doc! {"_id":invite.id,"accepted_at":null,"cancelled_at":null},doc! {"$set":{"accepted_at":bson::DateTime::now(),"accepted_by_user_id":actor}}).session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
            if db.collection::<AssistantGroup>(GROUPS).update_one(doc! {"_id":&group.id,"collaboration.membership_revision":previous_revision},doc! {"$set":{"participant_user_ids":&group.participant_user_ids,"updated_at":bson::DateTime::now()},"$inc":{"collaboration.membership_revision":1}}).session(&mut *session).await?.matched_count != 1 { return Err(stale()); }
            invalidate(db,&group.id,session).await?;
            Ok(group)
        }.await;
        transactions::transaction_result(result)
    })}).await.map_err(transactions::map_transaction_error)?;
    audit(db, actor, &group.id, "invite_accepted").await;
    Ok(group)
}

pub(super) async fn delete(
    db: &Database,
    group: &AssistantGroup,
    session: &mut ClientSession,
) -> AppResult<()> {
    invalidate(db, &group.id, session).await?;
    for collection in [
        model::INVITES,
        model::AGENT_ACCESS,
        model::TRANSFERS,
        model::PROPOSALS,
        crate::models::assistant_group::REQUESTS_COLLECTION_NAME,
        crate::models::assistant_group::MESSAGES_COLLECTION_NAME,
        crate::models::assistant_message_feedback::COLLECTION_NAME,
    ] {
        db.collection::<bson::Document>(collection)
            .delete_many(if collection == model::TRANSFERS {
                doc! {"_id":&group.id}
            } else {
                doc! {"group_id":&group.id}
            })
            .session(&mut *session)
            .await?;
    }
    db.collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
        .delete_many(doc! {"group_id":&group.id})
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .delete_many(doc! {"collaboration_group_id":&group.id})
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(crate::models::assistant_upload_retention::TOMBSTONES)
        .delete_many(doc! {"$or":[{"group_id":&group.id},{"collaboration_group_id":&group.id}]})
        .session(&mut *session)
        .await?;
    // Revoked slots are retained until cancellation/settlement and artifact cleanup.
    db.collection::<AssistantGroup>(GROUPS)
        .delete_one(doc! {"_id":&group.id})
        .session(session)
        .await?;
    Ok(())
}
