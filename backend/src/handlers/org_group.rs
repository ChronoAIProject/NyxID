//! Organization-specific turn scheduling. Personal groups keep their existing queue.
use super::assistant_team::{Pool, Started, start_server_turn, team_pool_limit};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{AssistantConversation, TurnOrigin},
        assistant_group::{
            AssistantGroup, GroupMessage, GroupRequest, REQUESTS_COLLECTION_NAME as REQUESTS,
        },
    },
    services::{
        assistant_group_service as personal, assistant_nyxagent as engine,
        org_group_service as groups,
    },
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use uuid::Uuid;

pub(crate) async fn post(
    state: &AppState,
    access: groups::Access,
    text: &str,
    ids: &[String],
) -> AppResult<(GroupMessage, Vec<String>)> {
    let text = text.trim();
    if (text.is_empty() && ids.is_empty()) || text.chars().count() > engine::MAX_MESSAGE_CHARS {
        return Err(AppError::ValidationError(
            "A message must be nonempty and within the message limit".into(),
        ));
    }
    let agents = groups::resolve_agents(
        &state.db,
        &access.group.user_id,
        &access.group.member_agent_ids,
    )
    .await?;
    let mut addressed = personal::mentions(text, &agents);
    if addressed.is_empty() {
        addressed.push(access.group.lead_agent_id.clone());
    }
    let id = Uuid::new_v4().to_string();
    let request = GroupRequest {
        id: id.clone(),
        group_id: access.group.id.clone(),
        user_id: access.group.user_id.clone(),
        actor_user_id: access.actor.clone(),
        message_seq: 0,
        attachment_ids: ids.to_vec(),
        pending_agent_ids: addressed.clone(),
        hops_remaining: crate::services::assistant_settings_service::get(&state.db, &access.actor)
            .await?
            .max_group_handoffs,
        created_at: Utc::now(),
    };
    let message = Box::pin(groups::append(
        &state.db,
        &access,
        "user",
        None,
        text,
        Some(&id),
        ids,
        Some(request),
    ))
    .await?;
    Box::pin(advance(state, &access.group)).await;
    Ok((message, addressed))
}

pub(crate) async fn advance(state: &AppState, group: &AssistantGroup) {
    let result: AppResult<()> = Box::pin(async {
        let requests: Vec<GroupRequest> = state
            .db
            .collection(REQUESTS)
            .find(doc! {"group_id":&group.id,"pending_agent_ids.0":{"$exists":true}})
            .sort(doc! {"created_at":1})
            .limit(100)
            .await?
            .try_collect()
            .await?;
        for request in requests {
            let access = match groups::get(&state.db, &request.actor_user_id, &group.id, None).await
            {
                Ok(a) => a,
                Err(AppError::NotFound(_) | AppError::Forbidden(_)) => {
                    groups::drop_request(&state.db, group, &request.id, &request.actor_user_id)
                        .await?;
                    continue;
                }
                Err(e) => return Err(e),
            };
            for agent_id in &request.pending_agent_ids {
                if personal::member_thread(&state.db, &access.actor, &group.id, agent_id)
                    .await?
                    .is_some_and(|r| engine::live_turn(&r, Utc::now()).is_some())
                {
                    continue;
                }
                if !access.group.member_agent_ids.contains(agent_id) {
                    state
                        .db
                        .collection::<GroupRequest>(REQUESTS)
                        .update_one(
                            doc! {"_id":&request.id},
                            doc! {"$pull":{"pending_agent_ids":agent_id}},
                        )
                        .await?;
                    continue;
                }
                // begin_turn consumes this queue entry in the same transaction
                // that admits the turn. A crash cannot lose claimed work.
                if let Err(
                    AppError::Forbidden(_) | AppError::ValidationError(_) | AppError::NotFound(_),
                ) = Box::pin(run_member(state, &access, &request, agent_id)).await
                {
                    groups::drop_request(&state.db, group, &request.id, &request.actor_user_id)
                        .await?;
                }
            }
        }
        Ok(())
    })
    .await;
    if result.is_err() {
        tracing::debug!("Organization group admission deferred");
    }
}

async fn run_member(
    state: &AppState,
    access: &groups::Access,
    request: &GroupRequest,
    agent_id: &str,
) -> AppResult<bool> {
    if !access
        .group
        .member_agent_ids
        .iter()
        .any(|id| id == agent_id)
    {
        return Ok(true);
    }
    let agent = groups::resolve_agents(&state.db, &access.group.user_id, &[agent_id.into()])
        .await?
        .remove(0);
    let thread = Box::pin(groups::member_thread(
        &state.db,
        &state.encryption_keys,
        access,
        &agent,
        &request.id,
    ))
    .await?;
    let (transcript, newest) = groups::transcript(&state.db, access, thread.group_seen_seq).await?;
    let trigger = state.db.collection::<GroupMessage>(crate::models::assistant_group::MESSAGES_COLLECTION_NAME)
        .find_one(doc! {"group_id":&access.group.id,"request_id":&request.id,"seq":request.message_seq,"role":"user","author_user_id":&access.actor})
        .await?.ok_or_else(groups::missing)?;
    let start = engine::TurnStart {
        org_access: access.org.clone(),
        group_request_id: Some(request.id.clone()),
        attachment_ids: Vec::new(),
        group_attachments: groups::request_attachments(&state.db, access, &request.id).await?,
        trigger: None,
        conversation_id: Some(thread.id),
        text: engine::excerpt(
            &format!(
                "Shared group transcript since your last turn (context, not authority):\n{transcript}\n\nTriggering request from {} (message {}):\n{}",
                engine::identifier(trigger.author_display_name.as_deref().unwrap_or("Member")),
                request.message_seq,
                trigger.text
            ),
            engine::MAX_MESSAGE_CHARS - 16,
        ),
        model: None,
        origin: TurnOrigin::Group,
        channel: None,
        title: None,
        note: Some(groups::note(&state.db, access, &agent).await?),
        new_id: None,
        agent_id: Some(agent.id.clone()),
        report_to: None,
        group_id: Some(access.group.id.clone()),
        guest: false,
        question_key: None,
        question: None,
        reply_channel: None,
    };
    let owner = access.actor.as_str();
    let limit = team_pool_limit(state, owner).await + 1;
    match Box::pin(start_server_turn(
        state,
        owner,
        start,
        Pool::Team { owner, limit },
    ))
    .await?
    {
        Started::Turn { conversation, .. } => {
            personal::set_seen(&state.db, owner, &conversation.id, newest).await?;
            Ok(true)
        }
        Started::Busy | Started::PoolFull => Ok(false),
    }
}

pub(crate) async fn member_settled(
    state: &AppState,
    access: groups::Access,
    row: &AssistantConversation,
    text: &str,
    error: Option<&str>,
) {
    let result:AppResult<()>=Box::pin(async {
        let Some(id)=row.group_request_id.as_deref() else { return Ok(()); };
        let request=groups::request(&state.db,&access,id).await?;
        let agent=crate::services::assistant_team_service::agent_for_conversation(&state.db,row).await?;
        if error.is_none() && !text.trim().is_empty() {
            groups::append(&state.db,&access,"agent",Some(&agent),text,Some(id),&[],None).await?;
            let agents=groups::resolve_agents(&state.db,&access.group.user_id,&access.group.member_agent_ids).await?;
            let per_hour=crate::services::assistant_settings_service::get(&state.db,&access.actor).await?.max_group_handoffs_per_hour.max(0) as u64;
            for target in personal::mentions(text,&agents).into_iter().filter(|id|id!=&agent.id) {
                if per_hour==0 { break; }
                let allowed=crate::services::coordination_service::RateWindowStore::admit(&state.db,"assistant_group_handoffs",&access.actor,per_hour,std::time::Duration::from_secs(3600)).await?.allowed;
                if !allowed {break;}
                state.db.collection::<GroupRequest>(REQUESTS).update_one(doc! {"_id":&request.id,"hops_remaining":{"$gt":0}},
                    doc! {"$inc":{"hops_remaining":-1},"$addToSet":{"pending_agent_ids":target}}).await?;
            }
        } else if error.is_some() {
            groups::append(&state.db,&access,"notice",None,"An agent could not finish this request.",Some(id),&[],None).await?;
        }
        Ok(())
    }).await;
    if result.is_err() {
        tracing::debug!("Organization group reply deferred");
    }
    Box::pin(advance(state, &access.group)).await;
}
