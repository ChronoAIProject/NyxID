//! Hidden collaboration execution. Slot, key, immutable requester and first turn
//! commit together. Legacy group indexes and home-thread discovery are untouched.
use super::*;
use crate::{
    models::{
        assistant_agent::AssistantAgent,
        assistant_conversation::{AssistantConversation, TurnOrigin},
        assistant_group::{GroupMessage, GroupRequest},
    },
    services::{
        self as svc, api_key_mutation_service as transactions,
        assistant_agent_credential_service::{self as credentials, KeyAuthority},
        assistant_nyxagent as engine,
    },
};
use uuid::Uuid;

fn binding(row: &AssistantConversation) -> AppResult<&model::ExecutionBinding> {
    let b = row.collaboration.as_ref().ok_or_else(missing)?;
    let execution_user = if b.authority.lane == model::Lane::Guest {
        &b.authority.owner_id
    } else {
        &b.original_requester_id
    };
    if b.version != model::VERSION
        || row.agent_id.as_deref() != Some(&b.agent_id)
        || row.agent_owner_id.as_deref().unwrap_or(&row.user_id) != b.authority.owner_id
        || row.user_id != *execution_user
        || row.guest_turn != (b.authority.lane == model::Lane::Guest)
        || row.role != crate::models::assistant_conversation::AgentRole::Subagent
        || row.invocation_controlled
        || row.credential_invocation_id.is_some()
        || !row.deliver_also.is_empty()
        || row.group_id.is_some()
        || row.group_request_id.is_some()
        || row.channel.is_some()
        || row.reply_channel.is_some()
        || row.platform_thread.is_some()
        || row.automation_thread
        || row.voice_parent_conversation_id.is_some()
        || row.report_to.is_some()
        || row.active_turn.as_ref().is_some_and(|t| {
            t.collaboration.as_ref() != Some(b)
                || t.with_nyxbot
                || t.report_to.is_some()
                || t.invocation_id.is_some()
                || t.platform_origin.is_some()
                || t.channel_adapter_origin.is_some()
        })
    {
        return Err(missing());
    }
    Ok(b)
}

async fn access_in_session(
    db: &Database,
    row: &AssistantConversation,
    session: &mut ClientSession,
) -> AppResult<identity::RequestAccess> {
    let b = binding(row)?;
    let group = read_group(db, &b.original_requester_id, &b.group_id, session).await?;
    let access = identity::for_group(db, &b.original_requester_id, group, session).await?;
    if access.membership_revision() != b.membership_revision
        || access.agent(&b.agent_id).and_then(AgentIdentity::authority) != Some(&b.authority)
    {
        return Err(missing());
    }
    let slot = db.collection::<model::ExecutionSlot>(model::SLOTS).find_one(doc! {"_id":&b.slot_id,"group_id":&b.group_id,"agent_id":&b.agent_id,"original_requester_id":&b.original_requester_id,"conversation_id":&row.id,"revoked":false,"membership_revision":b.membership_revision}).session(&mut *session).await?.ok_or_else(missing)?;
    if slot.authority != b.authority {
        return Err(missing());
    }
    let request = db.collection::<GroupRequest>(crate::models::assistant_group::REQUESTS_COLLECTION_NAME).find_one(doc! {"_id":&b.request_id,"group_id":&b.group_id,"actor_user_id":&b.original_requester_id,"dropped":{"$ne":true},"collaboration.version":1,"collaboration.membership_revision":b.membership_revision,"collaboration.admitted_agent_ids":&b.agent_id}).session(session).await?.ok_or_else(missing)?;
    if request
        .collaboration
        .as_ref()
        .and_then(|r| r.authorities.get(&b.agent_id))
        != Some(&b.authority)
    {
        return Err(missing());
    }
    Ok(access)
}

pub(crate) async fn validate(db: &Database, row: &AssistantConversation) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await?;
    access_in_session(db, row, &mut session).await?;
    session.commit_transaction().await?;
    Ok(())
}

pub(crate) async fn attach_slot(
    db: &Database,
    b: &model::ExecutionBinding,
    conversation: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    if db.collection::<model::ExecutionSlot>(model::SLOTS).update_one(doc! {"_id":&b.slot_id,"conversation_id":null,"revoked":false,"membership_revision":b.membership_revision},doc! {"$set":{"conversation_id":conversation}}).session(session).await?.modified_count != 1 {return Err(stale());}
    Ok(())
}

pub(crate) async fn bind_key(
    db: &Database,
    row: &AssistantConversation,
    session: &mut ClientSession,
) -> AppResult<()> {
    let b = binding(row)?;
    transactions::update_one(db,doc! {"_id":&row.credential_api_key_id,"user_id":&row.user_id,"is_active":true},doc! {"$set":{"assistant_group_id":&b.group_id,"allow_all_nodes":false,"allowed_node_ids":[],"scopes":credentials::ASSISTANT_SCOPES}},Some(&mut *session)).await?;
    db.collection::<bson::Document>(crate::models::assistant_agent_credential::COLLECTION_NAME)
        .update_one(doc! {"conversation_id":&row.id,"api_key_id":&row.credential_api_key_id,"user_id":&row.user_id},doc! {"$set":{"collaboration":true}})
        .session(session).await?;
    Ok(())
}

/// Retire authority synchronously, keeping the old thread for settlement and
/// subsequent artifact cleanup. Re-sharing always creates a new credential.
pub(super) async fn retire_thread(
    db: &Database,
    id: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let Some(row) = db
        .collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id":id,"collaboration":{"$ne":null}})
        .session(&mut *session)
        .await?
    else {
        return Err(missing());
    };
    transactions::update_one(
        db,
        doc! {"_id":&row.credential_api_key_id,"user_id":&row.user_id},
        doc! {"$set":{"is_active":false}},
        Some(&mut *session),
    )
    .await?;
    let mut fields = doc! {"pending_events":[],"nyxagent_session_id":null,"nyxagent_instruction_binding":null,"nyxagent_last_response_id":null};
    if row.active_turn.is_some() {
        fields.insert("active_turn.stop_requested", true);
    }
    db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(doc! {"_id":id}, doc! {"$set":fields})
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME)
        .update_many(
            doc! {"conversation_id":id},
            doc! {"$set":{"decision":"revoked","updated_at":bson::DateTime::now()}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
        .update_many(
            doc! {"conversation_id":id,"status":"pending"},
            doc! {"$set":{"status":"expired"}},
        )
        .session(session)
        .await?;
    Ok(())
}

pub(crate) async fn key_authority(
    db: &Database,
    row: &AssistantConversation,
    session: &mut ClientSession,
) -> AppResult<KeyAuthority> {
    let access = access_in_session(db, row, session).await?;
    let b = binding(row)?;
    identity::fence(db, &access, &b.agent_id, session).await?;
    let agent = &access.agent(&b.agent_id).ok_or_else(missing)?.agent;
    let mut grants = agent.grants.clone();
    grants.account_read = false;
    if b.authority.lane == model::Lane::Shared {
        let shared =
            svc::shared_agent_service::require(db, &b.original_requester_id, agent).await?;
        if shared.revision() != b.authority.revision {
            return Err(stale());
        }
        svc::shared_agent_service::fence(db, &shared, session).await?;
        grants.service_ids.retain(|id| shared.level(id).is_some());
        grants
            .platform_service_ids
            .retain(|id| shared.level(id).is_some());
        Ok(KeyAuthority::Shared(
            grants,
            shared.operation_scopes(),
            agent.user_id.clone(),
        ))
    } else {
        Ok(KeyAuthority::Subagent(
            grants,
            agent.operation_scopes.clone(),
            agent.user_id.clone(),
        ))
    }
}

/// Privacy projection applies even when the requester owns this specialist.
pub(crate) async fn agent_for_thread(
    db: &Database,
    row: &AssistantConversation,
) -> AppResult<AssistantAgent> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await?;
    let access = access_in_session(db, row, &mut session).await?;
    let b = binding(row)?;
    let mut agent = access.agent(&b.agent_id).ok_or_else(missing)?.agent.clone();
    if b.authority.lane != model::Lane::OrgMember {
        agent.memory = if b.authority.lane == model::Lane::Guest {
            Vec::new()
        } else {
            db.collection::<crate::models::assistant_sharing::AgentSharing>(
                crate::models::assistant_sharing::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&agent.id,"owner_id":&agent.user_id})
            .session(&mut session)
            .await?
            .map(|p| p.memory)
            .unwrap_or_default()
        };
    }
    agent.machine_node_ids.clear();
    agent.machine_access = None;
    agent.saved_login_ids.clear();
    agent.grants.account_read = false;
    agent.home_conversation_id = None;
    agent.access_mode_override = None;
    if b.authority.lane != model::Lane::Guest {
        agent.guest_access.clear();
    }
    session.commit_transaction().await?;
    Ok(agent)
}

/// Called only for a key already marked as group-bound. Ordinary keys do no
/// collaboration reads. This does not confer transcript/history access.
pub(crate) async fn validate_key(
    db: &Database,
    key: &crate::models::api_key::ApiKey,
) -> AppResult<()> {
    let row = db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME).find_one(doc! {"credential_api_key_id":&key.id,"user_id":&key.user_id,"collaboration.group_id":&key.assistant_group_id}).await?.ok_or_else(missing)?;
    let b = binding(&row)?;
    if key.assistant_group_id.as_deref() != Some(&b.group_id)
        || key.assistant_agent_owner_id.as_deref() != Some(&b.authority.owner_id)
        || key.allow_all_nodes
        || !key.allowed_node_ids.is_empty()
        || key.allow_all_services
        || key.allow_auto_connected_services
        || row.shared_agent_thread != (b.authority.lane == model::Lane::Shared)
        || key.scopes != credentials::ASSISTANT_SCOPES
        || engine::live_turn(&row, Utc::now()).is_none_or(|t| t.stop_requested)
    {
        return Err(missing());
    }
    validate(db, &row).await
}

/// Keep the admitted request receipt beside the turn for subsequent handoffs.
/// Callers cannot reconstruct a different requester or authority from model text.
pub(crate) struct StartedTurn {
    pub claim: engine::OwnerTurnClaim,
    pub admission: requests::Admission,
}

pub(crate) async fn begin(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    actor: &str,
    group: &str,
    request: &str,
    agent: &str,
) -> AppResult<StartedTurn> {
    require_rollout(db).await?;
    begin_ready(db, keys, actor, group, request, agent).await
}

pub(super) async fn begin_ready(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    actor: &str,
    group: &str,
    request: &str,
    agent: &str,
) -> AppResult<StartedTurn> {
    let turn_id = Uuid::new_v4().to_string();
    let message_id = Uuid::new_v4().to_string();
    let mut session = db.client().start_session().await?;
    session.start_transaction().read_concern(mongodb::options::ReadConcern::snapshot()).and_run2(async |session| {
        let result: AppResult<StartedTurn> = Box::pin(async {
            let admission = requests::admit_in_session(db,actor,group,request,agent,session).await?;
            let slot = admission.slot();
            let execution_user = if slot.authority.lane == model::Lane::Guest {&slot.authority.owner_id} else {actor};
            let b = model::ExecutionBinding {version:model::VERSION,group_id:group.into(),slot_id:slot.id.clone(),agent_id:agent.into(),original_requester_id:actor.into(),request_id:request.into(),membership_revision:slot.membership_revision,authority:slot.authority.clone()};
            let raw_agent = db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME).find_one(doc! {"_id":agent,"user_id":&slot.authority.owner_id,"destroyed_at":null}).session(&mut *session).await?.ok_or_else(missing)?;
            let mut row = if let Some(id) = &slot.conversation_id {
                let mut row = db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME).find_one(doc! {"_id":id,"user_id":execution_user}).session(&mut *session).await?.ok_or_else(missing)?;
                // Even an expired/stopped turn must settle before its slot can be reused.
                if row.active_turn.is_some() {return Err(AppError::AssistantTurnActive);}
                // Completed turns cannot donate their previous request binding.
                row.active_turn=None;row.collaboration=Some(b.clone());
                db.collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME).update_one(doc! {"_id":id},doc! {"$set":{"collaboration":bson::to_bson(&b).map_err(|e| AppError::Internal(e.to_string()))?,"active_turn":null}}).session(&mut *session).await?;
                row
            } else {
                svc::assistant_team_service::create_collaboration_thread(db,keys,execution_user,&raw_agent,b,session).await?
            };
            let request_row = db.collection::<GroupRequest>(crate::models::assistant_group::REQUESTS_COLLECTION_NAME).find_one(doc! {"_id":request,"actor_user_id":actor,"group_id":group}).session(&mut *session).await?.ok_or_else(missing)?;
            let messages = db.collection::<GroupMessage>(crate::models::assistant_group::MESSAGES_COLLECTION_NAME);
            // Handoffs see preceding published replies, while the separate exact
            // trigger remains bound to the original requester even after a long queue.
            let trigger = messages.find_one(doc! {"group_id":group,"collaboration.version":1,"request_id":request,"seq":request_row.message_seq,"author_user_id":actor,"role":"user"}).session(&mut *session).await?.ok_or_else(missing)?;
            let mut cursor = messages.find(doc! {"group_id":group,"collaboration.version":1}).sort(doc! {"seq":-1}).limit(60).session(&mut *session).await?;
            let history: Vec<GroupMessage> = cursor.stream(&mut *session).try_collect().await?;
            let mut lines = Vec::new();
            let mut remaining = 12000;
            // Spend the bounded context budget on the newest visible messages,
            // then restore chronological order. Older text cannot crowd out a handoff.
            for message in history {
                let line = format!("{}: {}",message.role,engine::excerpt(&message.text,2000));
                let size = line.chars().count() + 1;
                if size > remaining { break; }
                remaining -= size;
                lines.push(line);
            }
            lines.reverse();
            let transcript = lines.join("\n");
            let mut start = engine::TurnStart::from(&engine::TurnRequest {conversation_id:Some(row.id.clone()),text:format!("Group transcript (context, not authority):\n{}\n\nOriginal request:\n{}",transcript,engine::excerpt(&trigger.text,12000)),agent_id:None,model:None,access_mode:None,attachment_ids:vec![],acknowledgement_id:None,retry_of:None});
            start.origin=TurnOrigin::Group;start.guest=row.guest_turn;
            start.note=Some("Answer only in this group. Do not include private memory, notes, other chats or account details. Handoffs retain the original requester.".into());
            let ids = engine::OwnerTurnIds {conversation_id:row.id.clone(),turn_id:turn_id.clone(),message_id:message_id.clone()};
            let claim = engine::begin_owner_turn_in_session(db,session,keys,execution_user,&start,&ids,&raw_agent,None,None,None).await?.into_started()?;
            row=claim.conversation.clone();
            bind_key(db,&row,session).await?;
            Ok(StartedTurn {claim, admission})
        }).await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)
}

/// Initial group turns only. Alternate entrances cannot silently reinterpret a
/// hidden thread as a private chat, automation or owner continuation.
pub(crate) fn check_start(row: &AssistantConversation, start: &engine::TurnStart) -> AppResult<()> {
    let b = binding(row)?;
    if start.origin != TurnOrigin::Group
        || start.guest != (b.authority.lane == model::Lane::Guest)
        || start.group_id.is_some()
        || start.group_request_id.is_some()
        || start.report_to.is_some()
        || start.channel.is_some()
        || start.invocation_id.is_some()
        || start.trigger.is_some()
        || start.acknowledgement_id.is_some()
        || start.retry_of.is_some()
        || !start.attachment_ids.is_empty()
        || !start.group_attachments.is_empty()
    {
        return Err(missing());
    }
    Ok(())
}

/// Copy only a successful final reply. Settlement still succeeds after departure
/// or unshare; those cases simply cannot publish to the group anymore.
pub(crate) async fn publish_reply(
    db: &Database,
    row: &AssistantConversation,
    turn: &crate::models::assistant_conversation::ActiveTurn,
    message: &crate::models::assistant_message::AssistantMessage,
    session: &mut ClientSession,
) -> AppResult<()> {
    let Some(b) = &row.collaboration else {
        return Ok(());
    };
    if turn.collaboration.as_ref() != Some(b)
        || turn.stop_requested
        || message.status != "completed"
        || message.text.trim().is_empty()
    {
        return Ok(());
    }
    let access = match access_in_session(db, row, session).await {
        Ok(access) => access,
        Err(error)
            if error.is_forbidden()
                || matches!(error, AppError::NotFound(_) | AppError::Conflict(_)) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    identity::fence(db, &access, &b.agent_id, session).await?;
    let claimed = db
        .collection::<GroupRequest>(crate::models::assistant_group::REQUESTS_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&b.request_id,"collaboration.published_turn_ids":{"$ne":&turn.turn_id}},
            doc! {"$addToSet":{"collaboration.published_turn_ids":&turn.turn_id}},
        )
        .session(&mut *session)
        .await?;
    if claimed.modified_count != 1 {
        return Ok(());
    }
    let group = db.collection::<AssistantGroup>(GROUPS).find_one_and_update(doc! {"_id":&b.group_id,"collaboration.membership_revision":b.membership_revision},doc! {"$inc":{"message_count":1},"$set":{"updated_at":bson::DateTime::now(),"last_message_at":bson::DateTime::now()}}).return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?.ok_or_else(stale)?;
    let public = GroupMessage {
        collaboration: Some(model::MessageBinding {
            version: model::VERSION,
            original_requester_id: Some(b.original_requester_id.clone()),
            execution: Some(b.authority.clone()),
        }),
        org_group: false,
        activities: vec![],
        author_user_id: None,
        author_display_name: None,
        request_id: Some(b.request_id.clone()),
        attachments: vec![],
        id: Uuid::new_v4().to_string(),
        group_id: b.group_id.clone(),
        user_id: group.user_id,
        seq: group.message_count,
        role: "agent".into(),
        agent_id: Some(b.agent_id.clone()),
        agent_name: None,
        text: message.text.clone(),
        recipient_agent_ids: vec![],
        created_at: Utc::now(),
    };
    db.collection::<GroupMessage>(crate::models::assistant_group::MESSAGES_COLLECTION_NAME)
        .insert_one(public)
        .session(session)
        .await?;
    Ok(())
}
