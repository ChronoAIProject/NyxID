//! Direct assignments commit with child admission; a durable saved reply can
//! recover a missed callback without executing the child again.
use super::*;
use crate::models::{
    assistant_conversation::{AssistantConversation, COLLECTION_NAME as THREADS},
    assistant_direct_handoff::{COLLECTION_NAME as HANDOFFS, DirectHandoff},
    assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    trigger_run::AutomationWork,
};

#[derive(Clone)]
pub struct Parent {
    pub thread: String,
    pub turn: String,
    pub work: AutomationWork,
}

pub fn parent(
    chat: &super::super::assistant_acknowledgement_service::ChatAuthority,
) -> Option<Parent> {
    Some(Parent {
        thread: chat.conversation_id.clone(),
        turn: chat.turn_id.clone()?,
        work: chat.automation_work.clone()?,
    })
}

pub async fn register_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    parent: &Parent,
    child: &mut AssistantConversation,
) -> AppResult<()> {
    let turn = child
        .active_turn
        .as_mut()
        .ok_or(AppError::AssistantTurnRequired)?;
    if child.user_id != owner
        || child.id == parent.thread
        || child.shared_agent_thread
        || child.agent_owner_id.as_ref().is_some_and(|id| id != owner)
    {
        return Err(AppError::Forbidden(
            "Automation handoffs require a personal specialist".into(),
        ));
    }
    let run = continuation::register_in_session(
        db,
        owner,
        &parent.thread,
        &parent.turn,
        &parent.work,
        1,
        session,
    )
    .await?;
    let work = AutomationWork {
        run_id: run.id.clone(),
        work_id: turn.turn_id.clone(),
        coordinator_thread_id: parent.work.coordinator_thread_id.clone(),
        role: "direct".into(),
        confirmation_policy: parent.work.confirmation_policy,
    };
    let receipt = DirectHandoff {
        id: work.work_id.clone(),
        user_id: owner.into(),
        run_id: run.id,
        parent_thread_id: parent.thread.clone(),
        parent_turn_id: parent.turn.clone(),
        child_thread_id: child.id.clone(),
        child_turn_id: turn.turn_id.clone(),
        agent_id: child
            .agent_id
            .clone()
            .ok_or(AppError::AssistantTurnRequired)?,
        state: "pending".into(),
        assignment: turn.delegated_images.assignment.clone(),
        work: work.clone(),
        parent_work: parent.work.clone(),
        expires_at: run.expires_at,
    };
    db.collection::<DirectHandoff>(HANDOFFS)
        .insert_one(receipt)
        .session(session)
        .await?;
    turn.automation_work = Some(work);
    turn.execution_task = None;
    child.automation_v2 = true;
    Ok(())
}

pub async fn resume_in_session(
    db: &Database,
    owner: &str,
    thread: &str,
    turn: &str,
    work: &AutomationWork,
    session: &mut ClientSession,
) -> AppResult<()> {
    if work.role != "direct" {
        return Ok(());
    }
    if db
        .collection::<Document>(HANDOFFS)
        .update_one(
            doc! {
                "_id": &work.work_id, "user_id": owner, "run_id": &work.run_id,
                "child_thread_id": thread, "state": "pending",
            },
            doc! {"$set": {"child_turn_id": turn}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(AppError::AssistantTurnRequired);
    }
    if db.collection::<Document>(RUNS).update_one(doc! {
        "_id": &work.run_id, "user_id": owner, "coordinator.protocol": 2,
        "coordinator.deadline": {"$gt": date(Utc::now())}, "outcome": {"$in": ["started", "waiting"]},
    }, doc! {"$inc": {"coordinator.version": 1_i64}}).session(session).await?.matched_count != 1 {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(())
}

pub async fn settled(db: &Database, row: &AssistantConversation) -> AppResult<bool> {
    let Some(turn) = row.active_turn.as_ref() else {
        return Ok(false);
    };
    let Some(work) = turn.automation_work.as_ref().filter(|w| w.role == "direct") else {
        return Ok(false);
    };
    let ctx = (db.clone(), work.work_id.clone(), row.user_id.clone());
    super::super::transaction_service::run_db_only(db, &ctx, |session, (db, id, owner)| {
        Box::pin(async move {
            if let Some(receipt) = db
                .collection::<DirectHandoff>(HANDOFFS)
                .find_one(doc! {"_id": id, "user_id": owner, "state": "pending"})
                .session(&mut *session)
                .await?
            {
                recover_one(db, &receipt, session).await?;
            }
            Ok(())
        })
    })
    .await?;
    Ok(true)
}

pub async fn recover(db: &Database, run: &TriggerRun) -> AppResult<()> {
    let receipts: Vec<DirectHandoff> = db
        .collection::<DirectHandoff>(HANDOFFS)
        .find(doc! {"user_id": &run.user_id, "run_id": &run.id, "state": "pending"})
        .limit(i64::from(continuation::DISPATCH_LIMIT))
        .await?
        .try_collect()
        .await?;
    for receipt in receipts {
        let ctx = (db.clone(), receipt.id.clone());
        super::super::transaction_service::run_db_only(db, &ctx, |session, (db, id)| {
            Box::pin(async move {
                if let Some(receipt) = db
                    .collection::<DirectHandoff>(HANDOFFS)
                    .find_one(doc! {"_id": id, "state": "pending"})
                    .session(&mut *session)
                    .await?
                {
                    recover_one(db, &receipt, session).await?;
                }
                Ok(())
            })
        })
        .await?;
    }
    Ok(())
}

async fn recover_one(
    db: &Database,
    receipt: &DirectHandoff,
    session: &mut ClientSession,
) -> AppResult<()> {
    let child = db
        .collection::<AssistantConversation>(THREADS)
        .find_one(doc! {"_id": &receipt.child_thread_id, "user_id": &receipt.user_id})
        .session(&mut *session)
        .await?;
    if child.as_ref().is_some_and(|c| {
        super::super::assistant_nyxagent::live_turn(c, Utc::now())
            .is_some_and(|t| t.turn_id == receipt.child_turn_id)
    }) {
        return Ok(());
    }
    let reply = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {
            "conversation_id": &receipt.child_thread_id, "user_id": &receipt.user_id,
            "turn_id": &receipt.child_turn_id, "role": "assistant",
        })
        .session(&mut *session)
        .await?;
    let Some(reply) = reply else {
        if child.is_none() {
            return publish(
                db,
                receipt,
                "Child thread deleted",
                Some("thread_deleted"),
                session,
            )
            .await;
        }
        return publish(
            db,
            receipt,
            "Child turn lost before a durable reply",
            Some("turn_lost"),
            session,
        )
        .await;
    };
    if reply.error_code.is_none()
        && continuation::work_pending(
            db,
            &receipt.user_id,
            &receipt.child_thread_id,
            &receipt.work,
            session,
        )
        .await?
    {
        return Ok(());
    }
    let failure = if reply.error_code.is_none() {
        continuation::child_failure(db, &receipt.user_id, &receipt.work, session).await?
    } else {
        None
    };
    publish(
        db,
        receipt,
        &reply.text,
        reply.error_code.as_deref().or(failure),
        session,
    )
    .await
}

async fn publish(
    db: &Database,
    receipt: &DirectHandoff,
    text: &str,
    error: Option<&str>,
    session: &mut ClientSession,
) -> AppResult<()> {
    // A late callback must not fall back to an unbound notice after cancellation.
    let live = db.collection::<Document>(RUNS).update_one(doc! {"_id": &receipt.run_id, "user_id": &receipt.user_id, "coordinator.protocol": 2, "outcome": {"$in": ["started", "waiting"]}}, doc! {"$inc": {"coordinator.version": 1_i64}}).session(&mut *session).await?.matched_count == 1;
    if live {
        let mut event = super::super::assistant_team_service::event(
            "subagent_settled",
            format!(
                "Automation specialist {}. {}",
                error.unwrap_or("replied"),
                super::super::assistant_nyxagent::excerpt(text, 4000)
            ),
            Some(&receipt.agent_id),
        );
        event.id = format!("automation-direct:{}", receipt.id);
        if error.is_none()
            && let Some(edge) = receipt.assignment.as_ref()
        {
            event.id = edge.id.clone();
            event.delegated_result = Some(edge.clone());
        }
        event.automation_work = Some(receipt.parent_work.clone());
        event = event.bound_to(Some(&receipt.parent_thread_id));
        super::super::assistant_nyxagent::append_events_in_session(
            db,
            &receipt.user_id,
            &receipt.parent_thread_id,
            &[event],
            session,
        )
        .await?;
        continuation::wake_in_session(db, &receipt.run_id, session).await?;
    }
    db.collection::<Document>(HANDOFFS)
        .update_one(
            doc! {"_id": &receipt.id, "state": "pending"},
            doc! {"$set": {"state": if live {"delivered"} else {"cancelled"}}},
        )
        .session(session)
        .await?;
    Ok(())
}

/// Rare lifecycle path: always clean both roles, even if a marker was lost.
pub async fn delete_thread_in_session(
    db: &Database,
    owner: &str,
    thread: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let mut cursor = db
        .collection::<DirectHandoff>(HANDOFFS)
        .find(doc! {"user_id": owner, "child_thread_id": thread, "state": "pending"})
        .session(&mut *session)
        .await?;
    let pending: Vec<_> = cursor.stream(&mut *session).try_collect().await?;
    for receipt in pending {
        publish(
            db,
            &receipt,
            "Child thread deleted",
            Some("thread_deleted"),
            session,
        )
        .await?;
    }
    db.collection::<Document>(HANDOFFS).delete_many(doc! {"user_id": owner, "$or": [{"parent_thread_id": thread}, {"child_thread_id": thread}]}).session(session).await?;
    Ok(())
}

/// A logical direct assignment keeps its original delivery identity/audience
/// through cards and async turns, while pointing at the latest saved reply.
pub async fn adopt_lineage(
    db: &Database,
    row: &mut AssistantConversation,
    session: &mut ClientSession,
) -> AppResult<()> {
    let Some(turn) = row.active_turn.as_mut() else {
        return Ok(());
    };
    let Some(work) = turn.automation_work.as_ref().filter(|w| w.role == "direct") else {
        return Ok(());
    };
    if let Some(receipt) = db.collection::<DirectHandoff>(HANDOFFS).find_one_and_update(doc! {"_id": &work.work_id, "run_id": &work.run_id, "user_id": &row.user_id, "child_thread_id": &row.id, "state": "pending", "assignment": {"$ne": null}}, doc! {"$set": {"assignment.source_turn": &turn.turn_id}}).return_document(ReturnDocument::After).session(session).await? {
        turn.delegated_images.assignment = receipt.assignment;
        turn.report_to = Some(receipt.parent_thread_id);
    }
    Ok(())
}
