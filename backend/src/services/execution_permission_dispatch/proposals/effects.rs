//! Bounded, payload-free dispatch evidence on the existing original message root.
use super::*;

const LIMIT: usize = 128;

#[cfg(test)]
tokio::task_local! {
    pub(crate) static BEFORE_SEND: std::sync::Arc<tokio::sync::Barrier>;
}

/// The same personal owner roots as automatic continuation. Evidence is not
/// consent: a later ordinary admission may still use live reusable authority.
fn task(chat: &acks::ChatAuthority) -> Option<&TaskAdmission> {
    chat.execution_task.as_ref().filter(|task| {
        task.task.root_kind == TaskRootKind::Message
            && task.actor_user_id == chat.user_id
            && supported_chat(chat)
            && !chat.collaboration_thread
            && !chat.invocation_controlled
            && chat.automation_work.is_none()
            && chat.execution_group_id.is_none()
            && chat.voice_parent_conversation_id.is_none()
            && chat.report_to.is_none()
            && chat.org_agent_access.is_none()
    })
}

pub(in crate::services::execution_permission_dispatch) async fn record(
    db: &Database,
    session: &mut ClientSession,
    chat: &acks::ChatAuthority,
    connection: &str,
    digest: &str,
    operation: &str,
    grant: &str,
) -> AppResult<()> {
    let Some(task) = task(chat) else {
        return Ok(());
    };
    let released = match continuation::release_filter(chat)? {
        Some(filter) => db
            .collection::<Document>(REQUESTS)
            .find_one(filter)
            .projection(doc! {"_id": 1})
            .session(&mut *session)
            .await?
            .is_some(),
        None => false,
    };
    let mut filter = tasks::live_filter(task)?;
    filter.extend(doc! {
        "user_id": &chat.user_id, "conversation_id": &chat.conversation_id,
        "execution_dispatches": {"$not": {"$elemMatch": {"operation": operation}}},
    });
    if released {
        // This atomic predicate, on the same root every writer updates, prevents
        // concurrent new occurrence IDs from spending the same effect twice.
        filter.insert("execution_dispatch_evidence_version", 2);
        filter.insert(
            "$and",
            vec![doc! {"execution_dispatches": {"$not": {"$elemMatch": {
                "epoch": task.epoch, "key": &chat.api_key_id,
                "connection": connection, "digest": digest,
            }}}}],
        );
    }
    filter.insert(
        format!("execution_dispatches.{}", LIMIT - 1),
        doc! {"$exists": false},
    );
    if db
        .collection::<Document>("assistant_messages")
        .update_one(
            filter,
            doc! {"$push": {
                "execution_dispatches": {
                    "version": 1, "epoch": task.epoch,
                    "key": &chat.api_key_id, "conversation": &chat.conversation_id,
                    "connection": connection, "digest": digest,
                    "operation": operation, "grant": grant, "turn": &chat.turn_id,
                }
            }},
        )
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(refused());
    }
    Ok(())
}

pub(super) async fn prior_dispatch(
    db: &Database,
    chat: &acks::ChatAuthority,
    connection: &str,
    digest: &str,
) -> AppResult<bool> {
    let Some(task) = task(chat) else {
        return Ok(false);
    };
    let root = db
        .collection::<Document>("assistant_messages")
        .find_one(tasks::live_filter(task)?)
        .projection(doc! {"execution_dispatch_evidence_version": 1})
        .await?
        .ok_or_else(refused)?;
    if root.get_i32("execution_dispatch_evidence_version").ok() != Some(2) {
        return Err(refused());
    }
    let mut filter = tasks::live_filter(task)?;
    filter.extend(doc! {
        "user_id": &chat.user_id, "conversation_id": &chat.conversation_id,
        "execution_dispatches": {"$elemMatch": {
            "version": 1, "epoch": task.epoch,
            "key": &chat.api_key_id, "conversation": &chat.conversation_id,
            "connection": connection, "digest": digest,
            "operation": {"$type": "string"}, "grant": {"$type": "string"},
        }},
    });
    Ok(db
        .collection::<Document>("assistant_messages")
        .find_one(filter)
        .projection(doc! {"_id": 1})
        .await?
        .is_some())
}
