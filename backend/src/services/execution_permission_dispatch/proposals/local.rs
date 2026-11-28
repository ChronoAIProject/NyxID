//! Local retirement is not channel execution, private-result disclosure or release.
use super::*;

pub(super) enum Outcome {
    Live,
    Receipt(String),
    Withheld,
}

pub(super) async fn expired_channel(
    state: &crate::AppState,
    row: &AssistantConversation,
    credential: &crate::services::assistant_agent_credential_service::AssistantCredential,
    ids: &[String],
) -> AppResult<Outcome> {
    let Some(task) = continuation::owner_task(row).filter(|task| task.channel_source.is_some())
    else {
        return Ok(Outcome::Live);
    };
    if !channel_expired(&state.db, row).await? {
        return Ok(Outcome::Live);
    }
    let key_hash = credential_hash(row, credential)?;
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
    }).map(Outcome::Receipt).unwrap_or(Outcome::Withheld))
}

pub(super) fn credential_hash(
    row: &AssistantConversation,
    credential: &crate::services::assistant_agent_credential_service::AssistantCredential,
) -> AppResult<String> {
    if credential.api_key_id != row.credential_api_key_id || credential.raw_key.is_empty() {
        return Err(refused());
    }
    Ok(crate::crypto::token::hash_token(
        credential.raw_key.as_str(),
    ))
}

pub(super) async fn fence_actor(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    key_hash: &str,
) -> AppResult<()> {
    super::super::fence_active_account(db, session, &row.user_id).await?;
    continuation::fence_credential(db, session, row, key_hash).await
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
    // A disabled rollout permits local retirement, never execution. Lookup
    // errors still abort, and the active personal account remains write-fenced.
    crate::services::execution_permission_enrollment::live_enabled(
        db,
        &row.user_id,
        Some(&mut *session),
    )
    .await?;
    fence_actor(db, session, row, key_hash).await?;
    let mut root = continuation::original_task_filter(row)?;
    root.insert(
        "$expr",
        doc! {"$or": [
            {"$lte": ["$execution_task.hard_deadline", "$$NOW"]},
            {"$lte": ["$execution_task.channel_source.expires_at", "$$NOW"]},
        ]},
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
        // A definitive no-send can have committed at the node/provider barrier
        // while this wake was live. Adopt only this admitted turn's receipt;
        // an older terminal observer never acquires local delivery membership.
        if request.status == "approved"
            && p.state == "not_sent"
            && p.claim_id.is_some()
            && p.delivered_turn_id.is_none()
            && !p.delivery_abandoned
            && p.resume_turn_id.as_deref() == Some(turn.turn_id.as_str())
        {
            lifecycle::retire_operations(db, session, std::slice::from_ref(&request.id)).await?;
            retired.push(request.id.clone());
            continue;
        }
        if request.status != "approved"
            || !matches!(p.state.as_str(), "queued" | "preparing" | "waiting")
        {
            continue;
        }
        if p.delivery_abandoned {
            return Err(refused());
        }
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
        lifecycle::cancel_in_session(db, session, filter, "authority_or_task_unavailable").await?;
        retired.push(request.id.clone());
    }
    let retired = if retired.is_empty() {
        prior_retirements(db, session, row, &current).await?
    } else {
        retired.sort();
        Some(record_retirements(db, session, row, &retired).await?)
    };
    if retired.is_some() {
        // Only this complete, expired-source transaction makes the local
        // receipt exclusive. Live per-member retirement stays provisional.
        let changed = db
            .collection::<Document>("assistant_conversations")
            .update_one(
                crate::services::assistant_nyxagent::WorkerFence::capture(row)?.filter(),
                doc! {"$set": {"active_turn.execution_local_only": true}},
            )
            .session(&mut *session)
            .await?;
        if changed.matched_count != 1 {
            return Err(refused());
        }
    }
    Ok(retired)
}

/// This observation grants no authority; mutation repeats every exact fence.
/// Database time avoids a gap between the app clock and the provider barrier.
pub(super) async fn channel_expired(db: &Database, row: &AssistantConversation) -> AppResult<bool> {
    let Some(task) = continuation::owner_task(row) else {
        return Ok(false);
    };
    let Some(source) = &task.channel_source else {
        return Ok(false);
    };
    let deadline = bson::DateTime::from_chrono(task.hard_deadline.min(source.expires_at));
    Ok(db
        .collection::<Document>("assistant_conversations")
        .find_one(doc! {
            "_id": &row.id, "user_id": &row.user_id,
            "$expr": {"$lte": [deadline, "$$NOW"]},
        })
        .projection(doc! {"_id": 1})
        .await?
        .is_some())
}

/// Ordinary cancellation/result updates must not consume the local expiry
/// receipt. A losing transaction aborts and the outer resume handles all IDs.
pub(super) async fn fence_live_channel(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
) -> AppResult<()> {
    if !continuation::owner_task(row).is_some_and(|task| task.channel_source.is_some()) {
        return Ok(());
    }
    let mut root = continuation::original_task_filter(row)?;
    root.insert(
        "$expr",
        doc! {"$and": [
            {"$gt": ["$execution_task.hard_deadline", "$$NOW"]},
            {"$gt": ["$execution_task.channel_source.expires_at", "$$NOW"]},
        ]},
    );
    if db
        .collection::<Document>("assistant_messages")
        .update_one(root, doc! {"$inc": {"execution_dispatch_fence": 1_i64}})
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(refused());
    }
    Ok(())
}

/// Preserve a legitimate per-member local cancellation if expiry is observed
/// while processing a later member. The caller holds complete turn/root proof.
pub(super) async fn record_retirements(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    ids: &[String],
) -> AppResult<Vec<String>> {
    let filter = crate::services::assistant_nyxagent::WorkerFence::capture(row)?.filter();
    let current = db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(filter.clone())
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    let mut recorded = current
        .active_turn
        .as_ref()
        .and_then(|turn| turn.execution_local_retirements.clone())
        .unwrap_or_default();
    recorded.extend_from_slice(ids);
    recorded.sort();
    recorded.dedup();
    let admitted: std::collections::HashSet<_> = row
        .active_turn
        .as_ref()
        .ok_or_else(refused)?
        .events
        .iter()
        .filter_map(|event| event.id.strip_prefix("execution-decision:"))
        .collect();
    if recorded.is_empty()
        || recorded.len() > 64
        || !recorded.iter().all(|id| admitted.contains(id.as_str()))
    {
        return Err(refused());
    }
    for version in AssistantVersion::ALL {
        db.collection::<Document>(REQUESTS)
            .update_many(
                version.selector(doc! {
                    "_id": {"$in": ids}, "user_id": &row.user_id,
                    "status": {"$in": ["approved", "expired"]},
                    "execution_proposal.v1.conversation_id": &row.id,
                    "execution_proposal.v1.key_id": &row.credential_api_key_id,
                    "execution_proposal.v1.state": {"$in": ["cancelled", "not_sent"]},
                    "execution_proposal.v1.owner_continuation": {"$ne": null},
                }),
                version.update(
                    doc! {"$set": {"execution_proposal.v1.owner_continuation.blocked": true}},
                ),
            )
            .session(&mut *session)
            .await?;
    }
    if db
        .collection::<Document>("assistant_conversations")
        .update_one(
            filter,
            doc! {"$set": {"active_turn.execution_local_retirements": &recorded}},
        )
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(refused());
    }
    Ok(recorded)
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
                                (p.state == "cancelled"
                                    && p.failure_code.as_deref()
                                        == Some("authority_or_task_unavailable"))
                                    || (p.state == "not_sent"
                                        && p.claim_id.is_some()
                                        && p.resume_turn_id.as_deref()
                                            == row
                                                .active_turn
                                                .as_ref()
                                                .map(|turn| turn.turn_id.as_str()))
                            })
                    })
                })
        })
        .map(Some)
        .ok_or_else(refused)
}
