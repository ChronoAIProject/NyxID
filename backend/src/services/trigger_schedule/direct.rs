//! Direct specialist assignment receipts, committed with child admission.
use super::*;
use crate::models::{
    assistant_conversation::{AssistantConversation, COLLECTION_NAME as THREADS, TurnOrigin},
    assistant_direct_handoff::{COLLECTION_NAME as HANDOFFS, DirectHandoff},
    assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
};
use crate::services::{assistant_nyxagent as engine, assistant_team_service as team};

/// Server-only parent identity copied from the authenticated tool turn.
#[derive(Clone)]
pub struct Parent {
    pub run_id: String,
    pub thread_id: String,
    pub turn_id: String,
}

pub async fn register(
    db: &Database,
    session: &mut ClientSession,
    parent: &Parent,
    child: &AssistantConversation,
) -> AppResult<()> {
    continuation::ready_in_session(db, session).await?;
    let turn = child
        .active_turn
        .as_ref()
        .ok_or(AppError::AssistantTurnRequired)?;
    if !child.is_subagent()
        || child.guest_turn
        || turn.origin != TurnOrigin::Orchestrator
        || turn.trigger_run_id.is_some()
        || turn.report_to.as_deref() != Some(parent.thread_id.as_str())
        || child.id == parent.thread_id
    {
        return Err(AppError::AssistantTurnRequired);
    }
    let row = db
        .collection::<AssistantConversation>(THREADS)
        .find_one_and_update(
            doc! {"_id":&parent.thread_id,"user_id":&child.user_id,
            "active_turn.turn_id":&parent.turn_id,"active_turn.trigger_run_id":&parent.run_id,
            "active_turn.stop_requested":false},
            doc! {"$inc":{"automation_handoff_version":1_i64}},
        )
        .session(&mut *session)
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    if engine::live_turn(&row, Utc::now()).is_none() {
        return Err(AppError::AssistantTurnRequired);
    }
    // Same thread -> run write order as the completion barrier and Stop.
    let run = db
        .collection::<TriggerRun>(RUNS)
        .find_one_and_update(
            doc! {"_id":&parent.run_id,"user_id":&child.user_id,"thread_id":&parent.thread_id,
            "turn_id":&parent.turn_id,"outcome":"started","continuation_protocol_version":2,
            "coordinator_handoff_closed":{"$ne":true},"result_claimed":{"$ne":true},
            "terminal_result_recorded":{"$ne":true}},
            doc! {"$inc":{"handoff_version":1_i64}},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Conflict("trigger_run_unavailable".into()))?;
    if db
        .collection::<DirectHandoff>(HANDOFFS)
        .count_documents(doc! {"run_id":&run.id,"state":"pending"})
        .session(&mut *session)
        .await?
        >= 32
    {
        return Err(AppError::Conflict("automation_handoff_capacity".into()));
    }
    db.collection::<DirectHandoff>(HANDOFFS)
        .insert_one(DirectHandoff {
            id: turn.turn_id.clone(),
            user_id: child.user_id.clone(),
            run_id: run.id,
            parent_thread_id: parent.thread_id.clone(),
            parent_turn_id: parent.turn_id.clone(),
            child_thread_id: child.id.clone(),
            child_turn_id: turn.turn_id.clone(),
            agent_id: child
                .agent_id
                .clone()
                .ok_or(AppError::AssistantTurnRequired)?,
            invocation_id: turn.invocation_id.clone(),
            state: "pending".into(),
            retry_at: Utc::now(),
            expires_at: run.expires_at,
        })
        .session(session)
        .await?;
    Ok(())
}

/// Some means this child belongs to the durable path, including an already
/// delivered/cancelled receipt. Never fall back to an unbound duplicate notice.
pub async fn settled(
    db: &Database,
    owner: &str,
    child_thread: &str,
    child_turn: &str,
) -> AppResult<Option<String>> {
    let Some(receipt) = db
        .collection::<DirectHandoff>(HANDOFFS)
        .find_one(doc! {"_id":child_turn,"user_id":owner,"child_thread_id":child_thread})
        .await?
    else {
        return Ok(None);
    };
    if receipt.state == "pending" {
        deliver(db, &receipt).await?;
    }
    Ok(Some(receipt.parent_thread_id))
}

async fn deliver(db: &Database, saved: &DirectHandoff) -> AppResult<()> {
    let db = db.clone();
    let saved = saved.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = Box::pin(async {
            let receipts = db.collection::<DirectHandoff>(HANDOFFS);
            let Some(receipt) = receipts.find_one(doc! {"_id":&saved.id,"state":"pending"})
                .session(&mut *session).await? else { return Ok(()) };
            let run = db.collection::<TriggerRun>(RUNS).find_one(doc! {
                "_id":&receipt.run_id,"user_id":&receipt.user_id,
                "thread_id":&receipt.parent_thread_id,"continuation_protocol_version":2,
                "outcome":{"$in":["started","waiting"]},
                "result_claimed":{"$ne":true},"terminal_result_recorded":{"$ne":true},
            }).session(&mut *session).await?;
            let parent = db.collection::<Document>(THREADS).find_one(
                doc! {"_id":&receipt.parent_thread_id,"user_id":&receipt.user_id},
            ).projection(doc! {"_id":1}).session(&mut *session).await?;
            if run.is_none() || parent.is_none() {
                receipts.update_one(doc! {"_id":&receipt.id},doc! {"$set":{"state":"cancelled"}})
                    .session(session).await?;
                return Ok(());
            }
            // Read the exact durable reply, so a crash between settlement and
            // notification is recoverable without replaying the specialist.
            let reply = db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
                "user_id":&receipt.user_id,"conversation_id":&receipt.child_thread_id,
                "turn_id":&receipt.child_turn_id,"role":"assistant",
            }).session(&mut *session).await?;
            let (status, text) = match reply {
                Some(reply) => (reply.error_code.unwrap_or_else(|| "replied".into()), reply.text),
                None => {
                    if db.collection::<Document>(THREADS).find_one(
                        doc! {"_id":&receipt.child_thread_id,"user_id":&receipt.user_id},
                    ).projection(doc! {"_id":1}).session(&mut *session).await?.is_some() {
                        return Ok(());
                    }
                    ("thread_deleted".into(), String::new())
                }
            };
            let mut event = team::bound_event("subagent_settled", format!(
                "Specialist {} {status}. Reply excerpt: \"{}\" Read more with nyxid__read_subagent (agent ID {}).",
                receipt.agent_id, engine::excerpt(&text, 1200).replace('"', "'"), receipt.agent_id,
            ), Some(&receipt.agent_id), None)
                .for_trigger(Some(&receipt.run_id)).for_invocation(receipt.invocation_id.as_deref());
            event.id = Uuid::new_v5(&Uuid::NAMESPACE_URL, format!("direct-handoff:{}",receipt.id).as_bytes()).to_string();
            if !engine::append_events_in_session(&db,&receipt.user_id,&receipt.parent_thread_id,&[event],session).await? {
                return Err(AppError::AssistantTurnRequired);
            }
            // Conflicts with terminal cleanup/Stop even if this was a stale
            // preterminal snapshot. Queue + receipt transition commit together.
            if db.collection::<Document>(RUNS).update_one(
                doc! {"_id":&receipt.run_id,"outcome":{"$in":["started","waiting"]},
                    "result_claimed":{"$ne":true},"terminal_result_recorded":{"$ne":true}},
                doc! {"$inc":{"handoff_version":1_i64}},
            ).session(&mut *session).await?.matched_count != 1 {
                return Err(AppError::Conflict("trigger_run_unavailable".into()));
            }
            receipts.update_one(doc! {"_id":&receipt.id,"state":"pending"},doc! {"$set":{"state":"delivered"}})
                .session(session).await?;
            Ok(())
        }).await;
        transactions::transaction_result(result)
    }).await.map_err(transactions::map_transaction_error)
}

/// Bounded retry of committed receipts after a lost callback or queue refusal.
pub async fn recover(db: &Database) -> AppResult<()> {
    let receipts: Vec<DirectHandoff> = db
        .collection::<DirectHandoff>(HANDOFFS)
        .find(doc! {"state":"pending","retry_at":{"$lte":date(Utc::now())}})
        .sort(doc! {"retry_at":1,"_id":1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    for receipt in receipts {
        db.collection::<Document>(HANDOFFS)
            .update_one(
                doc! {"_id":&receipt.id,"state":"pending"},
                doc! {"$set":{"retry_at":date(Utc::now()+Duration::seconds(15))}},
            )
            .await?;
        if let Err(error) = deliver(db, &receipt).await {
            tracing::debug!(handoff_id=%receipt.id,%error,"Direct specialist notification deferred");
        }
    }
    Ok(())
}
