//! A personal crew's public reply and hand-offs share its engine settlement.
use super::*;
use crate::models::{
    assistant_conversation::{ActiveTurn, AssistantConversation},
    assistant_group::{
        AssistantGroup, COLLECTION_NAME as GROUPS, GroupMessage,
        MESSAGES_COLLECTION_NAME as MESSAGES,
    },
    assistant_message::AssistantMessage,
};
use mongodb::options::ReturnDocument;

/// The admitted turn owns publication. Its callback only advances the queue.
/// Ordinary conversations return without a database command.
pub async fn bind(
    db: &Database,
    row: &mut AssistantConversation,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let Some(group_id) = row.group_id.as_deref() else {
        return Ok(());
    };
    if row.collaboration.is_some()
        || row.group_request_id.is_some()
        || row.active_turn.as_ref().is_none_or(|turn| {
            turn.automation_work
                .as_ref()
                .is_some_and(|work| work.role == "group")
        })
    {
        return Ok(());
    }
    let personal = db.collection::<AssistantGroup>(GROUPS)
        .find_one(doc! {"_id":group_id,"user_id":&row.user_id,"collaboration":null,"created_by_user_id":null})
        .session(tx).await?.is_some();
    if personal
        && super::enabled(db).await?
        && let Some(turn) = row.active_turn.as_mut()
    {
        turn.group_reply_in_settlement = true;
    }
    Ok(())
}

pub async fn publish(
    db: &Database,
    row: &AssistantConversation,
    turn: &ActiveTurn,
    reply: &AssistantMessage,
    tx: &mut ClientSession,
) -> AppResult<()> {
    if !turn.group_reply_in_settlement {
        return Ok(());
    }
    let (Some(group_id), Some(agent_id)) = (row.group_id.as_deref(), row.agent_id.as_deref())
    else {
        return Ok(());
    };
    if reply.error_code.is_none() && reply.text.trim().is_empty() {
        return Ok(());
    }
    let Some(group) = db
        .collection::<AssistantGroup>(GROUPS)
        .find_one(
            doc! {"_id":group_id,"user_id":&row.user_id,"collaboration":null,
            "created_by_user_id":null,"member_agent_ids":agent_id},
        )
        .session(&mut *tx)
        .await?
    else {
        return Ok(());
    };
    let Some(agent) = settlement_result(
        super::super::assistant_team_service::agent_for_conversation(db, row).await,
    )?
    else {
        return Ok(());
    };
    if agent.destroyed_at.is_some() {
        return Ok(());
    }
    let Some(access) = settlement_result(
        super::super::org_agent_service::resolve_agent_access(db, &row.user_id, &agent).await,
    )?
    else {
        return Ok(());
    };
    if let Some(shared) = access.as_ref().and_then(|access| access.shared())
        && settlement_result(super::super::shared_agent_service::fence(db, shared, tx).await)?
            .is_none()
    {
        return Ok(());
    }
    if settlement_result(
        super::super::erasure_admission::fence_in_session(db, &row.user_id, tx).await,
    )?
    .is_none()
    {
        return Ok(());
    }
    let (role, text) = match reply.error_code.as_deref() {
        None => ("agent", reply.text.clone()),
        Some("cancelled") => ("notice", format!("{} stopped.", identifier(&agent.name))),
        Some(code) => {
            let mut text = format!("{} could not reply ({code}).", identifier(&agent.name));
            if super::super::assistant_model_observer::is_policy_failure(code) {
                text.push_str("\n\n");
                text.push_str(&super::super::assistant_model_observer::approval_message(
                    code,
                    &super::super::assistant_links::AssistantPage::ApprovalSettings.path(),
                ));
            }
            ("notice", text)
        }
    };
    // The containing transaction replaces this exact member thread under its
    // active-turn fence. This parent write also conflicts with roster removal,
    // including remove-and-readd of a member with a new hidden thread.
    let Some(group) = db.collection::<AssistantGroup>(GROUPS).find_one_and_update(
        doc! {"_id":&group.id,"user_id":&row.user_id,"collaboration":null,
            "created_by_user_id":null,"member_agent_ids":agent_id},
        doc! {"$inc":{"message_count":1},"$set":{"last_message_at":bson::DateTime::from_chrono(reply.created_at),
            "updated_at":bson::DateTime::from_chrono(reply.created_at)}},
    ).return_document(ReturnDocument::After).session(&mut *tx).await? else {return Ok(());};
    let public = GroupMessage {
        collaboration: None,
        activities: super::super::machine_activity_service::group_activities(
            reply.activities.clone(),
        ),
        org_group: false,
        author_user_id: None,
        author_display_name: None,
        request_id: None,
        attachments: if role == "agent" {
            reply
                .attachments
                .iter()
                .filter(|a| a.origin == "tool" && a.content_type.starts_with("image/"))
                .take(super::super::assistant_nyxagent::MAX_TURN_ATTACHMENTS)
                .cloned()
                .collect()
        } else {
            vec![]
        },
        id: uuid::Uuid::new_v4().to_string(),
        group_id: group.id.clone(),
        user_id: row.user_id.clone(),
        seq: group.message_count,
        role: role.into(),
        agent_id: Some(agent.id.clone()),
        agent_name: Some(agent.name.clone()),
        text,
        recipient_agent_ids: vec![],
        mentioned_participant_ids: vec![],
        created_at: reply.created_at,
    };
    db.collection::<GroupMessage>(MESSAGES)
        .insert_one(&public)
        .session(&mut *tx)
        .await?;
    settlement_result(
        super::super::assistant_notification_service::record_group_reply_in_session(
            db,
            tx,
            &row.user_id,
            &group,
            &public,
        )
        .await,
    )?;
    if role == "agent" {
        settlement_result(handoffs(db, row, &group, &agent.id, &reply.text, tx).await)?;
    }
    Ok(())
}

async fn handoffs(
    db: &Database,
    row: &AssistantConversation,
    group: &AssistantGroup,
    source: &str,
    text: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    use super::super::assistant_group_service as groups;
    if groups::mention_tokens(text).next().is_none() {
        return Ok(());
    }
    let Some(members) = settlement_result(groups::members(db, &row.user_id, group).await)? else {
        return Ok(());
    };
    let roster: Vec<_> = members.iter().map(|m| m.agent.clone()).collect();
    let targets: Vec<_> = groups::mentions(text, &roster)
        .into_iter()
        .filter(|id| id != source && members.iter().any(|m| &m.id == id && m.available))
        .collect();
    if targets.is_empty() {
        return Ok(());
    }
    let settings = db
        .collection::<crate::models::assistant_settings::AssistantSettings>(
            crate::models::assistant_settings::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&row.user_id})
        .session(&mut *tx)
        .await?
        .unwrap_or_else(|| {
            crate::models::assistant_settings::AssistantSettings::defaults(&row.user_id)
        });
    let per_hour = settings.max_group_handoffs_per_hour.max(0) as u64;
    if per_hour == 0 {
        return Ok(());
    }
    let mut addressed = Vec::new();
    for id in targets {
        // Preserve the existing order: a denied hourly admission still spends
        // the attempted group hop. Both counters now commit with publication.
        if db
            .collection::<AssistantGroup>(GROUPS)
            .update_one(
                doc! {"_id":&group.id,"user_id":&row.user_id,"hops_remaining":{"$gt":0}},
                doc! {"$inc":{"hops_remaining":-1}},
            )
            .session(&mut *tx)
            .await?
            .modified_count
            != 1
        {
            break;
        }
        if !super::super::coordination_service::RateWindowStore::admit_in_session(
            db,
            "assistant_group_handoffs",
            &row.user_id,
            per_hour,
            std::time::Duration::from_secs(3600),
            tx,
        )
        .await?
        {
            break;
        }
        addressed.push(id);
    }
    if !addressed.is_empty() {
        db.collection::<AssistantGroup>(GROUPS)
            .update_one(
                doc! {"_id":&group.id,"user_id":&row.user_id},
                groups::address_update(&row.user_id, &addressed, &members),
            )
            .session(tx)
            .await?;
    }
    Ok(())
}

/// A business refusal ends only this settlement side effect. MongoDB errors,
/// including transaction conflicts, must abort and retry the containing turn.
pub(crate) fn settlement_result<T>(result: AppResult<T>) -> AppResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error @ AppError::DatabaseError(_)) => Err(error),
        Err(_) => Ok(None),
    }
}
