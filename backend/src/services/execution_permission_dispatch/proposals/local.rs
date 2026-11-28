//! Local retirement is not channel execution, private-result disclosure or release.
use super::*;

pub(super) async fn expired_channel(
    state: &crate::AppState,
    row: &AssistantConversation,
    credential: &crate::services::assistant_agent_credential_service::AssistantCredential,
    ids: &[String],
) -> AppResult<Option<String>> {
    let Some(task) = continuation::owner_task(row)
        .filter(|task| task.channel_source.is_some() && task.hard_deadline <= Utc::now())
    else {
        return Ok(None);
    };
    if credential.api_key_id != row.credential_api_key_id || credential.raw_key.is_empty() {
        return Err(refused());
    }
    let key_hash = crate::crypto::token::hash_token(credential.raw_key.as_str());
    let mut session = state.db.client().start_session().await?;
    let retired = session
        .start_transaction()
        .and_run(
            (&state.db, row, task, ids, &key_hash),
            |session, (db, row, task, ids, key_hash)| {
                Box::pin(async move {
                    let result = retire_in_session(db, session, row, task, ids, key_hash).await;
                    crate::services::api_key_mutation_service::transaction_result(result)
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    Ok(retired.map(|ids| {
        if ids.len() == 1 {
            "The operation was not sent because its authority or task is no longer available.".into()
        } else {
            format!(
                "The operations were not sent because their original task expired.\nRequest IDs: {}",
                ids.join(", ")
            )
        }
    }))
}

async fn retire_in_session(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    task: &TaskAdmission,
    ids: &[String],
    key_hash: &str,
) -> AppResult<Option<Vec<String>>> {
    super::batch::fence_turn(db, session, row).await?;
    require_enrolled(db, &row.user_id, Some(&mut *session)).await?;
    continuation::fence_credential(db, session, row, key_hash).await?;
    let mut root = continuation::original_task_filter(row)?;
    root.insert(
        "$expr",
        doc! {"$lte": ["$execution_task.hard_deadline", "$$NOW"]},
    );
    if db
        .collection::<Document>("assistant_messages")
        .update_one(root, doc! {"$inc": {"execution_dispatch_fence": 1_i64}})
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(refused());
    }
    let mut cursor = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find(assistant_selector(doc! {
            "_id": {"$in": ids},
            "user_id": &row.user_id,
            "execution_proposal.v1.conversation_id": &row.id,
            "execution_proposal.v1.key_id": &row.credential_api_key_id,
        }))
        .limit(64)
        .session(&mut *session)
        .await?;
    let current: Vec<ApprovalRequest> = cursor.stream(&mut *session).try_collect().await?;
    if current.len() != ids.len()
        || current.iter().any(|request| {
            !proposal(request).is_ok_and(|p| {
                p.task == *task
                    && AssistantVersion::of(request)
                        .is_ok_and(|version| continuation::eligible(row, p, version))
            })
        })
    {
        return Err(refused());
    }
    let turn = row.active_turn.as_ref().ok_or_else(refused)?;
    for event in &turn.events {
        if let Some(id) = event.id.strip_prefix("execution-decision:")
            && (event.kind != "permission_decided"
                || event.execution_task.as_ref() != Some(task)
                || !ids.iter().any(|other| other == id))
        {
            return Err(refused());
        }
    }
    let mut retired = Vec::new();
    for request in &current {
        let p = proposal(request)?;
        if request.status != "approved" || !matches!(p.state.as_str(), "queued" | "preparing") {
            continue;
        }
        if p.delivery_abandoned {
            return Err(refused());
        }
        let version = AssistantVersion::of(request)?;
        let filter = doc! {
            "_id": &request.id,
            "user_id": &row.user_id,
            "status": "approved",
            "execution_proposal.v1.state": &p.state,
            "execution_proposal.v1.claim_id": &p.claim_id,
            "execution_proposal.v1.effect_id": &p.effect_id,
            "execution_proposal.v1.grant_id": &p.grant_id,
            "execution_proposal.v1.prepared_digest": &p.prepared_digest,
            "execution_proposal.v1.key_id": &row.credential_api_key_id,
            "execution_proposal.v1.conversation_id": &row.id,
        };
        if p.owner_continuation.is_some()
            && db
                .collection::<Document>(REQUESTS)
                .update_one(
                    version.selector(filter.clone()),
                    version.update(doc! {
                        "$set": {"execution_proposal.v1.owner_continuation.blocked": true},
                    }),
                )
                .session(&mut *session)
                .await?
                .matched_count
                != 1
        {
            return Err(refused());
        }
        lifecycle::cancel_in_session(db, session, filter, "authority_or_task_unavailable").await?;
        retired.push(request.id.clone());
    }
    if retired.is_empty() {
        return prior_retirements(db, session, row, &current).await;
    }
    retired.sort();
    // Settlement uses only these IDs. Terminal observers remain intact,
    // including an undelivered observer whose result was never opened.
    if db
        .collection::<Document>("assistant_conversations")
        .update_one(
            crate::services::assistant_nyxagent::WorkerFence::capture(row)?.filter(),
            doc! {"$set": {"active_turn.execution_local_retirements": &retired}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(refused());
    }
    Ok(Some(retired))
}

async fn prior_retirements(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    requests: &[ApprovalRequest],
) -> AppResult<Option<Vec<String>>> {
    let existing = db
        .collection::<Document>("assistant_conversations")
        .find_one(crate::services::assistant_nyxagent::WorkerFence::capture(row)?.filter())
        .projection(doc! {"active_turn.execution_local_retirements": 1})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    let Ok(marker) = existing
        .get_document("active_turn")
        .and_then(|turn| turn.get_array("execution_local_retirements"))
    else {
        return Ok(None);
    };
    let recorded: Option<Vec<String>> = marker
        .iter()
        .map(|id| id.as_str().map(str::to_owned))
        .collect();
    recorded
        .filter(|recorded| {
            !recorded.is_empty()
                && recorded.windows(2).all(|pair| pair[0] < pair[1])
                && recorded.iter().all(|id| {
                    requests.iter().any(|request| {
                        &request.id == id
                            && proposal(request).is_ok_and(|p| {
                                p.state == "cancelled"
                                    && p.failure_code.as_deref()
                                        == Some("authority_or_task_unavailable")
                            })
                    })
                })
        })
        .map(Some)
        .ok_or_else(refused)
}
