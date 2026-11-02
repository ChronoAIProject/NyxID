//! Request-chain primitives. Runtime must call admission in the same transaction
//! as key/thread/turn creation; queued work retains its original requester.
use super::*;
use crate::{
    models::assistant_group::{
        GroupMessage, GroupRequest, MESSAGES_COLLECTION_NAME as MESSAGES,
        REQUESTS_COLLECTION_NAME as REQUESTS,
    },
    services::{self as svc, api_key_mutation_service as transactions},
};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone)]
pub struct Admission {
    slot: model::ExecutionSlot,
    actor: String,
    request_id: String,
    membership_revision: i64,
}
impl Admission {
    pub fn slot(&self) -> &model::ExecutionSlot {
        &self.slot
    }
    pub fn requester(&self) -> &str {
        &self.actor
    }
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
}

/// Only final, explicitly group-visible text belongs in this collection. Exact
/// card args, tool traces and private notes have no field in this API.
#[allow(clippy::too_many_arguments)]
async fn append(
    db: &Database,
    group: &AssistantGroup,
    actor: &str,
    text: &str,
    request: &str,
    refused: Option<&str>,
    recipients: &[String],
    session: &mut ClientSession,
) -> AppResult<GroupMessage> {
    let now = Utc::now();
    let current = db.collection::<AssistantGroup>(GROUPS).find_one_and_update(doc! {"_id":&group.id,"collaboration.membership_revision":marker(group)?.membership_revision},
        doc! {"$inc":{"message_count":1},"$set":{"updated_at":bson::DateTime::from_chrono(now),"last_message_at":bson::DateTime::from_chrono(now)}})
        .return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?.ok_or_else(stale)?;
    let message = GroupMessage {
        collaboration: Some(model::MessageBinding {
            version: model::VERSION,
            original_requester_id: Some(actor.into()),
            execution: None,
        }),
        org_group: false,
        activities: vec![],
        author_user_id: refused.is_none().then(|| actor.into()),
        author_display_name: None,
        request_id: Some(request.into()),
        attachments: vec![],
        id: Uuid::new_v4().to_string(),
        group_id: group.id.clone(),
        user_id: group.user_id.clone(),
        seq: current.message_count,
        role: if refused.is_some() { "notice" } else { "user" }.into(),
        agent_id: refused.map(str::to_owned),
        agent_name: None,
        text: text.into(),
        recipient_agent_ids: recipients.to_vec(),
        created_at: now,
    };
    db.collection::<GroupMessage>(MESSAGES)
        .insert_one(&message)
        .session(session)
        .await?;
    Ok(message)
}

#[cfg(test)]
pub(super) async fn enqueue_ready(
    db: &Database,
    actor: &str,
    group_id: &str,
    text: &str,
    recipients: Option<&[String]>,
) -> AppResult<GroupRequest> {
    enqueue_with_uploads_ready(db, actor, group_id, text, recipients, &[]).await
}

pub async fn enqueue_with_uploads(
    db: &Database,
    actor: &str,
    group_id: &str,
    text: &str,
    recipients: Option<&[String]>,
    uploads: &[String],
) -> AppResult<GroupRequest> {
    require_rollout(db).await?;
    enqueue_with_uploads_ready(db, actor, group_id, text, recipients, uploads).await
}

pub(super) async fn enqueue_with_uploads_ready(
    db: &Database,
    actor: &str,
    group_id: &str,
    text: &str,
    recipients: Option<&[String]>,
    uploads: &[String],
) -> AppResult<GroupRequest> {
    svc::assistant_upload_service::validate_ids(uploads)?;
    let text = text.trim();
    if (text.is_empty() && uploads.is_empty())
        || text.chars().count() > svc::assistant_nyxagent::MAX_MESSAGE_CHARS
    {
        return Err(invalid(
            "A message must be nonempty and within the message limit",
        ));
    }
    let settings = svc::assistant_settings_service::get(db, actor).await?;
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .and_run((db.clone(), actor.to_owned(), group_id.to_owned(), text.to_owned(), recipients.map(<[String]>::to_vec), uploads.to_vec(), settings), |session, context| {let (db, actor, group_id, text, recipients, uploads, settings)=&*context;let actor=actor.as_str();let group_id=group_id.as_str();let text=text.as_str();let recipients=recipients.as_deref();Box::pin(async move {
            let result: AppResult<GroupRequest> = async {
                let group = read_group(db, actor, group_id, session).await?;
                let access = identity::for_group(db, actor, group, session).await?;
                fence_person(db, actor, session).await?;
                fence_group(db, &access.group, session).await?;
                if db.collection::<GroupRequest>(REQUESTS).count_documents(doc! {"actor_user_id":actor,"collaboration.version":1,"dropped":{"$ne":true},"pending_agent_ids.0":{"$exists":true}}).session(&mut *session).await? >= 16 {
                    return Err(AppError::Conflict("Your group queue is full; wait or stop a pending request".into()));
                }
                let roster: Vec<_> = access.agents.values().map(|a| a.agent.clone()).collect();
                let selected = if let Some(ids) = recipients {
                    // Stored IDs are accepted even when their current ACL is unavailable:
                    // the addressed member receives a fixed notice, never a redirect.
                    if ids.is_empty()
                        || ids.len() > 8
                        || !unique(ids)
                        || ids
                            .iter()
                            .any(|id| !access.group.member_agent_ids.contains(id))
                    {
                        return Err(invalid("Choose 1 to 8 current group members as recipients"));
                    }
                    ids.to_vec()
                } else {
                    if svc::assistant_group_service::ambiguous_mention(text, &roster) {
                        return Err(invalid(
                            "More than one agent has that name; choose an exact recipient",
                        ));
                    }
                    let mentions = svc::assistant_group_service::mentions(text, &roster);
                    if !mentions.is_empty() {
                        mentions
                    } else {
                        svc::assistant_group_service::default_recipients(
                            access
                                .group
                                .last_recipient_ids_by_actor
                                .get(actor)
                                .map(Vec::as_slice)
                                .unwrap_or_default(),
                            &access.group.member_agent_ids,
                            &access.group.lead_agent_id,
                        )
                    }
                };
                let id = Uuid::new_v4().to_string();
                let mut message = append(
                    db,
                    &access.group,
                    actor,
                    text,
                    &id,
                    None,
                    &selected,
                    session,
                )
                .await?;
                message.attachments=svc::assistant_upload_service::bind(db,actor,group_id,&message.id,uploads,session).await?;
                db.collection::<GroupMessage>(MESSAGES).update_one(doc! {"_id":&message.id},doc! {"$set":{"attachments":bson::to_bson(&message.attachments).map_err(|e|AppError::Internal(e.to_string()))?}}).session(&mut *session).await?;
                let mut authorities = BTreeMap::new();
                for agent in &selected {
                    if let Some(authority) = access.agent(agent).and_then(AgentIdentity::authority)
                    {
                        identity::fence(db, &access, agent, session).await?;
                        authorities.insert(agent.clone(), authority.clone());
                    } else {
                        append(
                            db,
                            &access.group,
                            actor,
                            REFUSAL,
                            &id,
                            Some(agent),
                            &[],
                            session,
                        )
                        .await?;
                    }
                }
                let request = GroupRequest {
                    collaboration: Some(model::RequestBinding {
                        version: model::VERSION,
                        membership_revision: access.membership_revision(),
                        authorities: authorities.clone(),
                        admitted_agent_ids: vec![],
                        published_turn_ids: vec![],
                        refused_agent_ids: selected
                            .iter()
                            .filter(|id| !authorities.contains_key(*id))
                            .cloned()
                            .collect(),
                    }),
                    id,
                    group_id: group_id.into(),
                    user_id: access.group.user_id.clone(),
                    actor_user_id: actor.into(),
                    message_seq: message.seq,
                    attachment_ids: uploads.to_vec(),
                    pending_agent_ids: selected
                        .iter()
                        .filter(|id| authorities.contains_key(*id))
                        .cloned()
                        .collect(),
                    hops_remaining: settings.max_group_handoffs.max(0),
                    created_at: Utc::now(),
                };
                db.collection::<GroupRequest>(REQUESTS)
                    .insert_one(&request)
                    .session(&mut *session)
                    .await?;
                db.collection::<AssistantGroup>(GROUPS)
                    .update_one(
                        doc! {"_id":group_id},
                        doc! {"$set":{format!("last_recipient_ids_by_actor.{actor}"):selected}},
                    )
                    .session(session)
                    .await?;
                Ok(request)
            }
            .await;
            transactions::transaction_result(result)
        })})
        .await
        .map_err(transactions::map_transaction_error)
}

/// Intended hook for the runtime's existing atomic begin-turn transaction.
pub(super) async fn admit_in_session(
    db: &Database,
    actor: &str,
    group_id: &str,
    request_id: &str,
    agent_id: &str,
    session: &mut ClientSession,
) -> AppResult<Admission> {
    let group = read_group(db, actor, group_id, session).await?;
    let access = identity::for_group(db, actor, group, session).await?;
    let request = db.collection::<GroupRequest>(REQUESTS).find_one(doc! {"_id":request_id,"group_id":group_id,"actor_user_id":actor,"pending_agent_ids":agent_id,"dropped":{"$ne":true},"collaboration.version":1})
        .session(&mut *session).await?.ok_or_else(missing)?;
    let binding = request.collaboration.as_ref().ok_or_else(missing)?;
    if binding.membership_revision != access.membership_revision() {
        return Err(stale());
    }
    let live = access
        .agent(agent_id)
        .and_then(AgentIdentity::authority)
        .ok_or_else(|| AppError::Forbidden(REFUSAL.into()))?;
    if binding.authorities.get(agent_id) != Some(live) {
        return Err(AppError::Forbidden(
            "The queued agent's access changed; send a new request".into(),
        ));
    }
    identity::fence(db, &access, agent_id, session).await?;
    let lane = if live.lane == model::Lane::Guest {
        "guest"
    } else {
        "authenticated"
    };
    let filter =
        doc! {"group_id":group_id,"agent_id":agent_id,"original_requester_id":actor,"lane":lane};
    let previous = db
        .collection::<model::ExecutionSlot>(model::SLOTS)
        .find_one(filter.clone())
        .session(&mut *session)
        .await?;
    let slot = match previous {
        Some(slot)
            if !slot.revoked
                && slot.membership_revision == access.membership_revision()
                && slot.authority == *live =>
        {
            slot
        }
        Some(mut slot) => {
            // Never recycle a principal/session after an authority transition.
            // Runtime drains/revokes the old conversation before attaching a new one.
            if let Some(id) = slot.conversation_id.take() {
                super::runtime::retire_thread(db, &id, session).await?;
            }
            slot.authority = live.clone();
            slot.membership_revision = access.membership_revision();
            slot.revoked = false;
            db.collection::<model::ExecutionSlot>(model::SLOTS)
                .replace_one(filter.clone(), &slot)
                .session(&mut *session)
                .await?;
            slot
        }
        None => {
            let slot = model::ExecutionSlot {
                id: Uuid::new_v4().to_string(),
                group_id: group_id.into(),
                agent_id: agent_id.into(),
                original_requester_id: actor.into(),
                lane: lane.into(),
                conversation_id: None,
                membership_revision: access.membership_revision(),
                authority: live.clone(),
                revoked: false,
            };
            db.collection::<model::ExecutionSlot>(model::SLOTS)
                .insert_one(&slot)
                .session(&mut *session)
                .await?;
            slot
        }
    };
    if db.collection::<GroupRequest>(REQUESTS).update_one(doc! {"_id":request_id,"pending_agent_ids":agent_id},
        doc! {"$pull":{"pending_agent_ids":agent_id},"$addToSet":{"collaboration.admitted_agent_ids":agent_id}}).session(session).await?.matched_count != 1 {return Err(stale());}
    Ok(Admission {
        slot,
        actor: actor.into(),
        request_id: request_id.into(),
        membership_revision: access.membership_revision(),
    })
}

pub async fn handoff(db: &Database, source: &Admission, target: &str) -> AppResult<bool> {
    require_rollout(db).await?;
    handoff_ready(db, source, target).await
}

pub(super) async fn handoff_ready(
    db: &Database,
    source: &Admission,
    target: &str,
) -> AppResult<bool> {
    // Existing hourly bucket is keyed to the ORIGINAL requester, including guests.
    let settings = svc::assistant_settings_service::get(db, &source.actor).await?;
    if settings.max_group_handoffs_per_hour <= 0 {
        return Ok(false);
    }
    let mut session = db.client().start_session().await?;
    session.start_transaction().read_concern(mongodb::options::ReadConcern::snapshot()).and_run((db.clone(), source.clone(), target.to_owned(), settings), |session, context| {let (db, source, target, settings)=&*context;let target=target.as_str();Box::pin(async move {
        let result: AppResult<bool> = async {
            let group = read_group(db,&source.actor,&source.slot.group_id,session).await?;
            let access = identity::for_group(db,&source.actor,group,session).await?;
            if access.membership_revision() != source.membership_revision {return Err(stale());}
            if !access.group.member_agent_ids.iter().any(|id| id == target) || source.slot.agent_id == target {return Ok(false);}
            let request = db.collection::<GroupRequest>(REQUESTS).find_one(doc! {"_id":&source.request_id,"group_id":&source.slot.group_id,"actor_user_id":&source.actor,"dropped":{"$ne":true},"collaboration.version":1,"collaboration.admitted_agent_ids":&source.slot.agent_id}).session(&mut *session).await?.ok_or_else(missing)?;
            let binding = request.collaboration.as_ref().ok_or_else(missing)?;
            // Deduplicate across concurrent agents and repeated callbacks; a retry
            // cannot refill the chain or replace its original lane binding.
            if request.hops_remaining <= 0 || binding.authorities.contains_key(target) || binding.refused_agent_ids.iter().any(|id| id == target) {return Ok(false);}
            if access.agent(&source.slot.agent_id).and_then(AgentIdentity::authority) != Some(&source.slot.authority) {return Err(missing());}
            identity::fence(db,&access,&source.slot.agent_id,session).await?;
            fence_group(db,&access.group,session).await?;
            let Some(authority) = access.agent(target).and_then(AgentIdentity::authority) else {
                append(db,&access.group,&source.actor,REFUSAL,&source.request_id,Some(target),&[],session).await?;
                db.collection::<GroupRequest>(REQUESTS).update_one(doc! {"_id":&source.request_id},doc! {"$addToSet":{"collaboration.refused_agent_ids":target}}).session(session).await?;
                // Remember the denied target, without inventing an authority or
                // consuming a handoff. No model call can follow this notice.
                return Ok(false);
            };
            identity::fence(db,&access,target,session).await?;
            if !svc::coordination_service::RateWindowStore::admit_in_session(db,"assistant_group_handoffs",&source.actor,settings.max_group_handoffs_per_hour as u64,std::time::Duration::from_secs(3600),session).await? {return Ok(false);}
            if db.collection::<GroupRequest>(REQUESTS).update_one(doc! {"_id":&source.request_id,"hops_remaining":{"$gt":0}},doc! {
                "$inc":{"hops_remaining":-1},"$addToSet":{"pending_agent_ids":target},"$set":{format!("collaboration.authorities.{target}"):bson::to_bson(authority).map_err(|e| AppError::Internal(e.to_string()))?}
            }).session(session).await?.matched_count != 1 {return Err(stale());}
            Ok(true)
        }.await;
        transactions::transaction_result(result)
    })}).await.map_err(transactions::map_transaction_error)
}

/// Cancellation belongs to the original requester and remains available during
/// rollout disablement. It cannot stop another participant's chain or threads.
pub async fn cancel(db: &Database, actor: &str, group: &str, request: &str) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().read_concern(mongodb::options::ReadConcern::snapshot()).and_run((db.clone(), actor.to_owned(), group.to_owned(), request.to_owned()), |session, context| {let (db, actor, group, request)=&*context;let actor=actor.as_str();let group=group.as_str();let request=request.as_str();Box::pin(async move {
        let result: AppResult<()> = async {
            let current = read_group(db, actor, group, session).await?;
            fence_group(db, &current, session).await?;
            let request = db.collection::<GroupRequest>(REQUESTS).find_one(doc! {
                "_id":request,"group_id":group,"actor_user_id":actor,"collaboration.version":1
            }).session(&mut *session).await?.ok_or_else(missing)?;
            db.collection::<GroupRequest>(REQUESTS).update_one(doc! {"_id":&request.id},doc! {
                "$set":{"pending_agent_ids":[],"dropped":true}
            }).session(&mut *session).await?;
            let threads: Vec<crate::models::assistant_conversation::AssistantConversation> = rows(db,
                crate::models::assistant_conversation::COLLECTION_NAME,doc! {
                    "collaboration.version":1,"collaboration.group_id":group,
                    "collaboration.request_id":&request.id,"collaboration.original_requester_id":actor
                },session).await?;
            for thread in threads {
                super::runtime::retire_thread(db, &thread.id, session).await?;
                db.collection::<model::ExecutionSlot>(model::SLOTS).update_one(doc! {
                    "conversation_id":&thread.id,"group_id":group,"original_requester_id":actor
                },doc! {"$set":{"revoked":true}}).session(&mut *session).await?;
            }
            Ok(())
        }.await;
        transactions::transaction_result(result)
    })}).await.map_err(transactions::map_transaction_error)
}

/// Restore only a server-persisted receipt for a settled turn. The handoff
/// transaction rechecks the source, request and live authority before use.
pub(crate) async fn settled_admission(
    db: &Database,
    b: &model::ExecutionBinding,
) -> AppResult<Admission> {
    let slot = db
        .collection::<model::ExecutionSlot>(model::SLOTS)
        .find_one(
            doc! {"_id":&b.slot_id,"group_id":&b.group_id,"agent_id":&b.agent_id,
            "original_requester_id":&b.original_requester_id,"revoked":false},
        )
        .await?
        .ok_or_else(missing)?;
    if slot.authority != b.authority || slot.membership_revision != b.membership_revision {
        return Err(stale());
    }
    Ok(Admission {
        slot,
        actor: b.original_requester_id.clone(),
        request_id: b.request_id.clone(),
        membership_revision: b.membership_revision,
    })
}

pub async fn pending(db: &Database, group: Option<&str>) -> AppResult<Vec<GroupRequest>> {
    let mut filter = doc! {"collaboration.version":1,"dropped":{"$ne":true},"pending_agent_ids.0":{"$exists":true}};
    if let Some(id) = group {
        filter.insert("group_id", id);
    }
    Ok(db
        .collection::<GroupRequest>(REQUESTS)
        .find(filter)
        .sort(doc! {"created_at":1})
        .limit(100)
        .await?
        .try_collect()
        .await?)
}

/// Terminal queue failures cannot retry as a different principal or guest lane.
pub(crate) async fn refuse_queued(
    db: &Database,
    request: &GroupRequest,
    agent: &str,
) -> AppResult<()> {
    finish_queued(db, request, agent, REFUSAL).await
}
pub(crate) async fn busy_queued(
    db: &Database,
    request: &GroupRequest,
    agent: &str,
) -> AppResult<()> {
    finish_queued(
        db,
        request,
        agent,
        "This agent is busy. Please try again shortly.",
    )
    .await
}
async fn finish_queued(
    db: &Database,
    request: &GroupRequest,
    agent: &str,
    notice: &str,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().read_concern(mongodb::options::ReadConcern::snapshot()).and_run((db.clone(), request.clone(), agent.to_owned(), notice.to_owned()), |session, context| {let (db, request, agent, notice)=&*context;let agent=agent.as_str();let notice=notice.as_str();Box::pin(async move {
        let result: AppResult<()> = async {
            let removed = db.collection::<GroupRequest>(REQUESTS).update_one(doc! {"_id":&request.id,"pending_agent_ids":agent,"collaboration.version":1},
                doc! {"$pull":{"pending_agent_ids":agent}}).session(&mut *session).await?;
            if removed.modified_count != 1 {return Ok(());}
            if let Ok(group) = read_group(db,&request.actor_user_id,&request.group_id,session).await {
                append(db,&group,&request.actor_user_id,notice,&request.id,Some(agent),&[],session).await?;
            }
            Ok(())
        }.await;
        transactions::transaction_result(result)
    })}).await.map_err(transactions::map_transaction_error)
}
