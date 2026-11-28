//! Reconcile proven receipts; stored batch IDs never enter executable resume.
use super::*;

async fn block(db: &Database, requests: &[ApprovalRequest]) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    for request in requests {
        let p = proposal(request)?;
        let Some(marker) = &p.owner_continuation else {
            continue;
        };
        if marker.released_turn_id.is_some() {
            continue;
        }
        let version = AssistantVersion::of(request)?;
        db.collection::<Document>(REQUESTS).update_one(version.selector(doc! {
            "_id":&request.id, "user_id":&request.user_id,
            "execution_proposal.v1.key_id":&p.key_id,
            "execution_proposal.v1.conversation_id":&p.conversation_id,
            "execution_proposal.v1.owner_continuation.version":marker.version as i32,
            "execution_proposal.v1.owner_continuation.batch":bson::to_bson(&marker.batch).map_err(encoding)?,
            "execution_proposal.v1.owner_continuation.batch_turn_id":&marker.batch_turn_id,
            "execution_proposal.v1.owner_continuation.released_turn_id":null,
        }), version.update(doc! {"$set":{"execution_proposal.v1.owner_continuation.blocked":true}}))
            .session(&mut session).await?;
    }
    session.commit_transaction().await?;
    Ok(())
}

/// A partial wake can observe pending siblings, never send them. Only an exact,
/// originally pure admitted batch with all independently successful receipts can
/// release once. Missing or contradictory historical proof never grants a launch.
pub(super) async fn reconcile(
    state: &crate::AppState,
    row: &AssistantConversation,
    admitted: &[String],
    typed_wake: bool,
) -> AppResult<Option<continuation::OwnerContinuation>> {
    let current: Vec<ApprovalRequest> = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find(assistant_selector(
            doc! {"_id":{"$in":admitted}, "user_id":&row.user_id,
            "execution_proposal.v1.key_id":&row.credential_api_key_id,
            "execution_proposal.v1.conversation_id":&row.id},
        ))
        .limit(64)
        .await?
        .try_collect()
        .await?;
    let anchors: Vec<_> = current
        .iter()
        .filter(|request| {
            proposal(request).is_ok_and(|p| {
                p.state == "completed"
                    && p.owner_continuation
                        .as_ref()
                        .is_some_and(|marker| !marker.blocked && marker.released_turn_id.is_none())
            })
        })
        .collect();
    let Some(anchor) = anchors.first() else {
        return Ok(None);
    };
    let p = proposal(anchor)?;
    let marker = p.owner_continuation.as_ref().ok_or_else(conflict)?;
    let batch = marker.batch.as_deref().unwrap_or_default();
    let valid = typed_wake
        && current.len() == admitted.len()
        && marker.version == 2
        && marker
            .batch_turn_id
            .as_ref()
            .is_some_and(|id| !id.is_empty())
        && !batch.is_empty()
        && batch.len() <= 64
        && batch.windows(2).all(|pair| pair[0] < pair[1])
        && admitted.iter().all(|id| batch.contains(id));
    if !valid {
        block(&state.db, &current).await?;
        return Ok(None);
    }
    // This is an evidence-only read: queued/preparing/dispatched siblings are
    // observed and left for their own genuine admitted decisions/recovery.
    let receipts: Vec<ApprovalRequest> = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find(assistant_selector(
            doc! {"_id":{"$in":batch}, "user_id":&row.user_id,
            "execution_proposal.v1.key_id":&row.credential_api_key_id,
            "execution_proposal.v1.conversation_id":&row.id},
        ))
        .limit(64)
        .await?
        .try_collect()
        .await?;
    let task = row
        .active_turn
        .as_ref()
        .and_then(|turn| turn.execution_task.as_ref())
        .ok_or_else(refused)?;
    let mut ready = receipts.len() == batch.len();
    let mut invalid = receipts.len() != batch.len();
    for request in &receipts {
        let p = proposal(request)?;
        let version = AssistantVersion::of(request)?;
        invalid |= request.status != "approved"
            || !continuation::eligible(row, p, version)
            || p.task != *task
            || p.delivery_abandoned
            || p.delivered_turn_id.is_some()
            || p.owner_continuation.as_ref().is_none_or(|proof| {
                proof.version != 2
                    || proof.blocked
                    || proof.released_turn_id.is_some()
                    || proof.batch.as_deref() != Some(batch)
                    || proof.batch_turn_id != marker.batch_turn_id
            });
        match p.state.as_str() {
            "completed" => {
                invalid |= request.activity_outcome.as_deref() != Some("completed")
                    || p.result.is_none()
                    || p.claim_id.is_none();
            }
            "queued" | "preparing" | "dispatched" => {
                ready = false;
            }
            _ => {
                invalid = true;
            }
        }
    }
    if invalid {
        block(&state.db, &current).await?;
        block(&state.db, &receipts).await?;
        return Ok(None);
    }
    if !ready {
        return Ok(None);
    }
    if !continuation::original_owner_source(&state.db, row).await? {
        block(&state.db, &receipts).await?;
        return Ok(None);
    }
    let mut release = Vec::new();
    let mut notes = Vec::new();
    for id in batch {
        let request = receipts
            .iter()
            .find(|request| &request.id == id)
            .ok_or_else(missing)?;
        let p = proposal(request)?;
        let Some(text) = result_for_human(state, request).await? else {
            block(&state.db, &receipts).await?;
            return Ok(None);
        };
        notes.push(text);
        release.push(continuation::Receipt {
            id: id.clone(),
            version: AssistantVersion::of(request)?,
            claim_id: p.claim_id.clone().ok_or_else(conflict)?,
            batch_turn_id: marker.batch_turn_id.clone().ok_or_else(conflict)?,
        });
    }
    Ok(Some(
        continuation::prepare(state, row, release, notes.join("\n\n")).await?,
    ))
}
