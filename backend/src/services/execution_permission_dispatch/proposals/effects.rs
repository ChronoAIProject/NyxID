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
    // Detect a release before narrowing the owner lane. A changed key/task or
    // foreign release must never fall through as an ordinary unmarked request.
    let release = if let Some(turn) = &chat.turn_id {
        db.collection::<Document>(REQUESTS)
            .find_one(assistant_selector(doc! {
                "execution_proposal.v1.conversation_id":&chat.conversation_id,
                "execution_proposal.v1.owner_continuation.released_turn_id":turn,
            }))
            .projection(doc! {"_id":1})
            .session(&mut *session)
            .await?
    } else {
        None
    };
    let released = if let Some(release) = release {
        let mut exact = continuation::release_filter(chat)?.ok_or_else(refused)?;
        exact.insert("_id", release.get_str("_id").map_err(encoding)?);
        if db
            .collection::<Document>(REQUESTS)
            .find_one(exact)
            .projection(doc! {"_id":1})
            .session(&mut *session)
            .await?
            .is_none()
        {
            return Err(refused());
        }
        true
    } else {
        false
    };
    let Some(task) = task(chat) else {
        return if released { Err(refused()) } else { Ok(()) };
    };
    let root = db
        .collection::<Document>("assistant_messages")
        .find_one(doc! {"_id":&task.task.root_id})
        .projection(doc! {"execution_dispatch_evidence_version":1})
        .session(&mut *session)
        .await?;
    let marker = root
        .as_ref()
        .and_then(|root| root.get("execution_dispatch_evidence_version"));
    if matches!(marker, None | Some(bson::Bson::Null)) && !released {
        // Historical/readiness-off ordinary coverage does not enroll the task
        // in v2 evidence or impose a new capacity/occurrence limit.
        return Ok(());
    }
    if marker != Some(&bson::Bson::Int32(2)) {
        return Err(refused());
    }
    let mut filter = tasks::live_filter(task)?;
    filter.extend(doc! {
        "user_id": &chat.user_id, "conversation_id": &chat.conversation_id,
        "execution_dispatch_evidence_version":2,
        "execution_dispatches": {"$not": {"$elemMatch": {"operation": operation}}},
    });
    if released {
        // This atomic predicate, on the same root every writer updates, prevents
        // concurrent new occurrence IDs from spending the same effect twice.
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
