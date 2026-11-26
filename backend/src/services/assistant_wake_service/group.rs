//! Legacy queues remain present until the corresponding admission commits.
use super::*;
use crate::models::{
    assistant_conversation::AssistantConversation,
    assistant_group::{AssistantGroup, COLLECTION_NAME as GROUPS, GroupFollower},
};
use chrono::Utc;

pub const INPUT_CHANGED: &str = "Group wake changed before admission";

#[derive(Clone)]
pub struct Member {
    pub group_id: String,
    pub agent_id: String,
    pub upper_seq: i64,
    pub org_revision: Option<i64>,
    pub durable: bool,
}

tokio::task_local! {
    static MEMBER: Member;
}

pub async fn with_member<F: std::future::Future>(member: Member, future: F) -> F::Output {
    MEMBER.scope(member, future).await
}

pub(crate) fn admitted_member(group: &str, agent: &str, upper_seq: i64) -> bool {
    MEMBER
        .try_with(|member| {
            member.durable
                && member.group_id == group
                && member.agent_id == agent
                && member.upper_seq == upper_seq
        })
        .unwrap_or(false)
}

/// No request-local binding means no database command. The group write and
/// conversation replacement are in the engine's existing admission transaction.
pub async fn admit(
    db: &Database,
    row: &mut AssistantConversation,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let Ok(member) = MEMBER.try_with(Clone::clone) else {
        return Ok(());
    };
    if member
        .org_revision
        .is_some_and(|revision| revision != row.org_use_revision)
    {
        return Err(AppError::Forbidden("Specialist use access changed".into()));
    }
    if !member.durable {
        return Ok(());
    }
    if row.group_id.as_ref() != Some(&member.group_id)
        || row.agent_id.as_ref() != Some(&member.agent_id)
        || row.group_seen_seq >= member.upper_seq
    {
        return Err(AppError::Conflict(INPUT_CHANGED.into()));
    }
    // A newer message must stay pending. Refuse this snapshot and rebuild it
    // rather than consuming work which was not present in the model input.
    let changed = db
        .collection::<AssistantGroup>(GROUPS)
        .update_one(
            doc! {"_id":&member.group_id,"user_id":&row.user_id,"collaboration":null,
            "pending_agent_ids":&member.agent_id,"member_agent_ids":&member.agent_id,
            "message_count":member.upper_seq},
            doc! {"$pull":{"pending_agent_ids":&member.agent_id}},
        )
        .session(tx)
        .await?;
    if changed.modified_count != 1 {
        return Err(AppError::Conflict(INPUT_CHANGED.into()));
    }
    row.group_seen_seq = member.upper_seq;
    Ok(())
}

/// A refusal/empty snapshot may retire only the pending request it observed.
/// This never consumes a newer post to the same member.
pub async fn retire(
    db: &Database,
    owner: &str,
    group: &AssistantGroup,
    agent: &str,
) -> AppResult<()> {
    db.collection::<AssistantGroup>(GROUPS)
        .update_one(
            doc! {"_id":&group.id,"user_id":owner,"message_count":group.message_count},
            doc! {"$pull":{"pending_agent_ids":agent}},
        )
        .await?;
    Ok(())
}

pub async fn followers(
    db: &Database,
    owner: &str,
    group_id: &str,
) -> AppResult<Vec<GroupFollower>> {
    let context = (db.clone(), owner.to_owned(), group_id.to_owned());
    super::super::transaction_service::run_db_only(db, &context, |tx, (db, owner, id)| {
        Box::pin(async move {
            let rows = db.collection::<AssistantGroup>(GROUPS);
            let Some(group) = rows
                .find_one(doc! {"_id":id,"user_id":owner,"collaboration":null})
                .session(&mut *tx)
                .await?
            else {
                return Ok(vec![]);
            };
            if !group.pending_agent_ids.is_empty()
                || !group.followers.iter().any(|f| f.automation.is_none())
            {
                return Ok(vec![]);
            }
            let now = Utc::now();
            let conversations = db.collection::<AssistantConversation>(
                crate::models::assistant_conversation::COLLECTION_NAME,
            );
            let mut cursor = conversations
                .find(doc! {"user_id":owner,"group_id":id})
                .session(&mut *tx)
                .await?;
            let threads: Vec<_> = cursor.stream(&mut *tx).try_collect().await?;
            if threads.iter().any(|row| {
                row.active_turn.as_ref().is_some_and(|turn| {
                    super::super::assistant_turn_restart::awaiting_reply(row, &turn.turn_id)
                })
            }) {
                return Ok(vec![]);
            }
            let mut queued = Vec::new();
            let mut retained = Vec::new();
            for follower in group.followers {
                if follower.automation.is_some() {
                    retained.push(follower);
                    continue;
                }
                if follower.expiry() <= now {
                    continue;
                }
                let reference = WakeReference::GroupFollower {
                    group_id: id.clone(),
                    since_seq: follower.since_seq,
                    upper_seq: group.message_count,
                };
                let event = super::event("group_settled", reference);
                // A deleted destination deliberately ends its follow. Queue pressure
                // rolls back both the take and every enqueue.
                if super::super::assistant_nyxagent::append_events_in_session(
                    db,
                    owner,
                    &follower.conversation_id,
                    &[event],
                    tx,
                )
                .await?
                {
                    queued.push(follower);
                }
            }
            rows.update_one(
                doc! {"_id":id,"user_id":owner},
                doc! {"$set":{"followers":bson::to_bson(&retained)
                .map_err(|_|AppError::Internal("Invalid followers".into()))?}},
            )
            .session(tx)
            .await?;
            Ok(queued)
        })
    })
    .await
}
