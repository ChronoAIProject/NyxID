//! Fenced handoffs retain independent decision proofs; IDs never become sends.
use super::*;
use crate::models::execution_proposal::OwnerContinuationHandoff as Handoff;

fn canonical(ids: &[String]) -> bool {
    !ids.is_empty() && ids.len() <= 64 && ids.windows(2).all(|pair| pair[0] < pair[1])
}

fn cohort(request: &ApprovalRequest) -> AppResult<Option<Handoff>> {
    let p = proposal(request)?;
    let Some(proof) = &p.owner_continuation else {
        return Ok(None);
    };
    let Some(original) = proof
        .batch
        .as_ref()
        .filter(|ids| canonical(ids) && ids.contains(&request.id))
    else {
        return Ok(None);
    };
    let Some(turn) = proof.batch_turn_id.as_ref().filter(|turn| !turn.is_empty()) else {
        return Ok(None);
    };
    Ok(match proof.version {
        2 if proof.handoff.is_none() => Some(Handoff {
            version: 1,
            request_ids: original.clone(),
            turn_id: turn.clone(),
        }),
        3 => proof
            .handoff
            .as_ref()
            .filter(|handoff| {
                handoff.version == 1
                    && !handoff.turn_id.is_empty()
                    && canonical(&handoff.request_ids)
                    && original.iter().all(|id| handoff.request_ids.contains(id))
            })
            .cloned(),
        _ => None,
    })
}

async fn load(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    ids: &[String],
) -> AppResult<Vec<ApprovalRequest>> {
    let mut cursor = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find(assistant_selector(doc! {
            "_id":{"$in":ids}, "user_id":&row.user_id,
            "execution_proposal.v1.key_id":&row.credential_api_key_id,
            "execution_proposal.v1.conversation_id":&row.id,
        }))
        .limit(64)
        .session(&mut *session)
        .await?;
    Ok(cursor.stream(session).try_collect().await?)
}

async fn block_in(
    db: &Database,
    session: &mut ClientSession,
    requests: &[ApprovalRequest],
) -> AppResult<()> {
    for request in requests {
        let p = proposal(request)?;
        let Some(marker) = &p.owner_continuation else {
            continue;
        };
        if marker.released_turn_id.is_some() {
            continue;
        }
        let version = AssistantVersion::of(request)?;
        let mut exact = doc! {
            "_id":&request.id, "user_id":&request.user_id,
            "execution_proposal.v1.key_id":&p.key_id,
            "execution_proposal.v1.conversation_id":&p.conversation_id,
            "execution_proposal.v1.owner_continuation.version":marker.version as i32,
            "execution_proposal.v1.owner_continuation.batch":bson::to_bson(&marker.batch).map_err(encoding)?,
            "execution_proposal.v1.owner_continuation.batch_turn_id":&marker.batch_turn_id,
            "execution_proposal.v1.owner_continuation.released_turn_id":null,
        };
        if let Some(handoff) = &marker.handoff {
            exact.extend(doc! {
                "execution_proposal.v1.owner_continuation.handoff.version":handoff.version as i32,
                "execution_proposal.v1.owner_continuation.handoff.request_ids":&handoff.request_ids,
                "execution_proposal.v1.owner_continuation.handoff.turn_id":&handoff.turn_id,
            });
        } else {
            exact.insert(
                "execution_proposal.v1.owner_continuation.handoff",
                bson::Bson::Null,
            );
        }
        db.collection::<Document>(REQUESTS)
            .update_one(
                version.selector(exact),
                version.update(
                    doc! {"$set":{"execution_proposal.v1.owner_continuation.blocked":true}},
                ),
            )
            .session(&mut *session)
            .await?;
    }
    Ok(())
}

async fn block(db: &Database, requests: &[ApprovalRequest]) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    block_in(db, &mut session, requests).await?;
    session.commit_transaction().await?;
    Ok(())
}

async fn block_on_task(
    db: &Database,
    row: &AssistantConversation,
    requests: &[ApprovalRequest],
) -> AppResult<()> {
    let Some(task) = continuation::owner_task(row) else {
        return Ok(());
    };
    let own: Vec<_> = requests
        .iter()
        .filter(|request| proposal(request).is_ok_and(|p| p.task == *task))
        .cloned()
        .collect();
    block(db, &own).await
}

#[derive(PartialEq, Eq)]
enum State {
    Ready,
    Pending,
    Invalid,
}

/// Both original proof groups and any prior handoff must be complete and
/// internally consistent. Different valid original groups can share a handoff.
fn valid_proofs(
    row: &AssistantConversation,
    requests: &[ApprovalRequest],
    ids: &[String],
) -> AppResult<bool> {
    let Some(task) = continuation::owner_task(row) else {
        return Ok(false);
    };
    if requests.len() != ids.len() {
        return Ok(false);
    }
    for request in requests {
        let p = proposal(request)?;
        let version = AssistantVersion::of(request)?;
        let Some(handoff) = cohort(request)? else {
            return Ok(false);
        };
        let proof = p.owner_continuation.as_ref().ok_or_else(conflict)?;
        if !continuation::eligible(row, p, version)
            || p.task != *task
            || p.delivered_turn_id.is_some()
            || proof.released_turn_id.is_some()
        {
            return Ok(false);
        }
        for member in proof.batch.as_ref().ok_or_else(conflict)? {
            let Some(other) = requests.iter().find(|other| &other.id == member) else {
                return Ok(false);
            };
            let Some(original) = &proposal(other)?.owner_continuation else {
                return Ok(false);
            };
            if original.batch != proof.batch || original.batch_turn_id != proof.batch_turn_id {
                return Ok(false);
            }
        }
        for member in &handoff.request_ids {
            let Some(other) = requests.iter().find(|other| &other.id == member) else {
                return Ok(false);
            };
            if cohort(other)?.as_ref() != Some(&handoff) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn validate(
    row: &AssistantConversation,
    requests: &[ApprovalRequest],
    ids: &[String],
) -> AppResult<State> {
    if !valid_proofs(row, requests, ids)? {
        return Ok(State::Invalid);
    }
    let mut state = State::Ready;
    for request in requests {
        let p = proposal(request)?;
        let proof = p.owner_continuation.as_ref().ok_or_else(conflict)?;
        if request.status != "approved" || p.delivery_abandoned || proof.blocked {
            return Ok(State::Invalid);
        }
        match p.state.as_str() {
            "completed"
                if request.activity_outcome.as_deref() == Some("completed")
                    && p.result.is_some()
                    && p.claim_id.is_some() => {}
            "queued" | "preparing" | "dispatched" => {
                state = State::Pending;
            }
            _ => {
                return Ok(State::Invalid);
            }
        }
    }
    Ok(state)
}

/// Bind fresh decisions to their own original proof, then atomically join all
/// compatible, unreleased groups co-admitted on this live task. No crash can
/// expose a fresh proof without the corresponding joined handoff.
pub(super) async fn admit(
    state: &crate::AppState,
    row: &AssistantConversation,
    requests: &[ApprovalRequest],
    admitted: &[String],
    typed_wake: bool,
) -> AppResult<Option<bool>> {
    let Some(task) = continuation::owner_task(row) else {
        return Ok(None);
    };
    if requests
        .iter()
        .any(|request| !proposal(request).is_ok_and(|p| p.task == *task))
    {
        return Err(refused());
    }
    let mut groups = Vec::new();
    let mut work = false;
    for request in requests {
        let p = proposal(request)?;
        if request.status == "approved"
            && matches!(p.state.as_str(), "queued" | "preparing")
            && continuation::eligible(row, p, AssistantVersion::of(request)?)
        {
            work = true;
        }
        if let Some(proof) = p
            .owner_continuation
            .as_ref()
            .filter(|proof| !proof.blocked && proof.released_turn_id.is_none())
        {
            if let Some(group) = cohort(request)? {
                work |= proof.version == 2
                    && request.status == "approved"
                    && p.delivered_turn_id.is_none()
                    && p.state == "completed"
                    && request.activity_outcome.as_deref() == Some("completed")
                    && p.result.is_some()
                    && p.claim_id.is_some()
                    && continuation::eligible(row, p, AssistantVersion::of(request)?);
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
        }
    }
    if !typed_wake {
        // Mixed ordinary events cannot regain eligibility after a lost wake.
        block(&state.db, requests).await?;
        return Ok(None);
    }
    work |= groups.len() > 1
        || groups
            .iter()
            .any(|group| !admitted.iter().all(|id| group.request_ids.contains(id)));
    if !work {
        return Ok(None);
    }
    // The optional dependency is never consulted for an unmarked legacy root.
    if state
        .db
        .collection::<Document>("assistant_messages")
        .find_one(doc! {
            "_id":&task.task.root_id,"user_id":&row.user_id,"conversation_id":&row.id,
            "execution_dispatch_evidence_version":2,
        })
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let ready = crate::services::security_readiness_service::serving_report(
        &state.db,
        continuation::READINESS,
    )
    .await?
    .serving_ready;
    if !ready {
        if !groups.is_empty() {
            return Err(AppError::SecurityReadinessUnavailable);
        }
        return Ok(Some(false));
    }
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run((&state.db,row,task,admitted),
        |session,(db,row,task,admitted)| Box::pin(async move {
            let result: AppResult<()> = async {
                tasks::fence(db, session, task).await?;
                let mut root = tasks::live_filter(task)?;
                root.extend(doc! {"user_id":&row.user_id,"conversation_id":&row.id,"execution_dispatch_evidence_version":2});
                if db.collection::<Document>("assistant_messages").update_one(root,
                    doc! {"$inc":{"execution_dispatch_fence":1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(refused()); }
                let mut fence = crate::services::assistant_nyxagent::WorkerFence::capture(row)?.filter();
                fence.extend(continuation::current_task_filter(task)?);
                fence.extend(doc! {"credential_api_key_id":&row.credential_api_key_id,
                    "active_turn.stop_requested":false,"$expr":continuation::unexpired_turn()});
                if db.collection::<Document>("assistant_conversations").update_one(fence,
                    doc! {"$inc":{"execution_dispatch_fence":1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(refused()); }
                let mut current = load(db, session, row, admitted).await?;
                if current.len() != admitted.len() || current.iter().any(|request|
                    !proposal(request).is_ok_and(|p| p.task == **task)) { return Err(refused()); }
                let mut fresh: Vec<_> = current.iter().filter(|request| proposal(request).is_ok_and(|p| p.owner_continuation.is_none()))
                    .map(|request| request.id.clone()).collect();
                fresh.sort();
                let turn = row.active_turn.as_ref().ok_or_else(refused)?;
                for request in &current {
                    let p = proposal(request)?;
                    let version = AssistantVersion::of(request)?;
                    let actionable = request.status == "approved" && matches!(p.state.as_str(), "queued" | "preparing");
                    let unsent_decision = matches!(request.status.as_str(), "rejected" | "expired")
                        && matches!(p.state.as_str(), "denied" | "expired" | "cancelled");
                    if p.owner_continuation.is_none() && (actionable || unsent_decision)
                        && continuation::eligible(row, p, version) {
                        if db.collection::<Document>(REQUESTS).update_one(version.selector(doc! {
                            "_id":&request.id,"execution_proposal.v1.owner_continuation":null,
                            "execution_proposal.v1.state":&p.state,"status":&request.status,
                        }), version.update(doc! {"$set":{"execution_proposal.v1.owner_continuation":{
                            "version":3,"batch":&fresh,"batch_turn_id":&turn.turn_id,"blocked":false,
                            "released_turn_id":bson::Bson::Null,
                            "handoff":{"version":1,"request_ids":&fresh,"turn_id":&turn.turn_id},
                        }}})).session(&mut *session).await?.matched_count != 1 { return Err(conflict()); }
                    }
                }
                current = load(db, session, row, admitted).await?;
                // Already released/delivered observations cannot join another
                // launch. Preserve the independent new group's recovery proof.
                if current.iter().any(|request| proposal(request).is_ok_and(|p|
                    p.delivered_turn_id.is_some() || p.owner_continuation.as_ref().is_some_and(|proof| proof.released_turn_id.is_some()))) { return Ok(()); }
                let mut union = Vec::new();
                for request in &current {
                    let Some(group) = cohort(request)? else {
                        block_in(db, session, &current).await?;
                        return Ok(());
                    };
                    union.extend(group.request_ids);
                }
                union.sort(); union.dedup();
                if !canonical(&union) { block_in(db,session,&current).await?; return Ok(()); }
                let all = load(db, session, row, &union).await?;
                if !valid_proofs(row, &all, &union)? {
                    block_in(db,session,&current).await?;
                    let own: Vec<_> = all.iter().filter(|request| proposal(request).is_ok_and(|p| p.task == **task)).cloned().collect();
                    block_in(db,session,&own).await?;
                    return Ok(());
                }
                let joined = Handoff {version:1,request_ids:union.clone(),turn_id:turn.turn_id.clone()};
                let unchanged = all.iter().all(|request| proposal(request).is_ok_and(|p|
                    p.owner_continuation.as_ref().is_some_and(|proof| proof.version == 3)));
                let first = all.first().map(cohort).transpose()?.flatten();
                if unchanged && first.as_ref().is_some_and(|first| first.request_ids == union)
                    && all.iter().all(|request| cohort(request).ok().flatten() == first) { return Ok(()); }
                for request in &all {
                    let p = proposal(request)?;
                    let version = AssistantVersion::of(request)?;
                    let proof = p.owner_continuation.as_ref().ok_or_else(conflict)?;
                    if db.collection::<Document>(REQUESTS).update_one(version.selector(doc! {
                        "_id":&request.id,"user_id":&row.user_id,
                        "execution_proposal.v1.key_id":&row.credential_api_key_id,
                        "execution_proposal.v1.conversation_id":&row.id,
                        "execution_proposal.v1.owner_continuation.version":proof.version as i32,
                        "execution_proposal.v1.owner_continuation.batch":bson::to_bson(&proof.batch).map_err(encoding)?,
                        "execution_proposal.v1.owner_continuation.batch_turn_id":&proof.batch_turn_id,
                        "execution_proposal.v1.owner_continuation.released_turn_id":null,
                        "execution_proposal.v1.delivered_turn_id":null,
                    }), version.update(doc! {"$set":{"execution_proposal.v1.owner_continuation.version":3,
                        "execution_proposal.v1.owner_continuation.handoff":bson::to_bson(&joined).map_err(encoding)?}}))
                        .session(&mut *session).await?.matched_count != 1 { return Err(conflict()); }
                }
                Ok(())
            }.await;
            crate::services::api_key_mutation_service::transaction_result(result)
        })).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    Ok(Some(true))
}

/// Reconcile durable receipts only. An unrelated valid proof is never blocked
/// merely for having a different original decision batch.
pub(super) async fn reconcile(
    state: &crate::AppState,
    row: &AssistantConversation,
    admitted: &[String],
    typed_wake: bool,
) -> AppResult<Option<continuation::OwnerContinuation>> {
    let current: Vec<ApprovalRequest> = state.db.collection::<ApprovalRequest>(REQUESTS).find(assistant_selector(doc! {
        "_id":{"$in":admitted},"user_id":&row.user_id,
        "execution_proposal.v1.key_id":&row.credential_api_key_id,"execution_proposal.v1.conversation_id":&row.id,
    })).limit(64).await?.try_collect().await?;
    let anchor = current.iter().find(|request| {
        proposal(request).is_ok_and(|p| {
            p.state == "completed"
                && p.owner_continuation
                    .as_ref()
                    .is_some_and(|proof| !proof.blocked && proof.released_turn_id.is_none())
        })
    });
    let Some(anchor) = anchor else {
        return Ok(None);
    };
    if !typed_wake {
        block(&state.db, &current).await?;
        return Ok(None);
    }
    let proof = proposal(anchor)?
        .owner_continuation
        .as_ref()
        .ok_or_else(conflict)?;
    let Some(handoff) = cohort(anchor)?.filter(|_| proof.version == 3) else {
        block(&state.db, &current).await?;
        return Ok(None);
    };
    if !admitted.iter().all(|id| handoff.request_ids.contains(id)) {
        return Ok(None);
    }
    let all: Vec<ApprovalRequest> = state.db.collection::<ApprovalRequest>(REQUESTS).find(assistant_selector(doc! {
        "_id":{"$in":&handoff.request_ids},"user_id":&row.user_id,
        "execution_proposal.v1.key_id":&row.credential_api_key_id,"execution_proposal.v1.conversation_id":&row.id,
    })).limit(64).await?.try_collect().await?;
    // Another fenced admission may have extended this handoff between the
    // two reads. Preserve its valid proof and let a fresh wake observe the new
    // cohort; never block it based on an older, smaller snapshot.
    let observed: Vec<_> = all.iter().map(cohort).collect::<AppResult<_>>()?;
    if observed
        .iter()
        .any(|group| group.as_ref() != Some(&handoff))
    {
        let advanced = observed
            .first()
            .and_then(Option::as_ref)
            .is_some_and(|newer| {
                newer.turn_id != handoff.turn_id
                    && handoff
                        .request_ids
                        .iter()
                        .all(|id| newer.request_ids.contains(id))
                    && observed.iter().all(|group| group.as_ref() == Some(newer))
            });
        if !advanced {
            block(&state.db, &current).await?;
            block_on_task(&state.db, row, &all).await?;
        }
        return Ok(None);
    }
    let status = validate(row, &all, &handoff.request_ids)?;
    if status == State::Invalid {
        block(&state.db, &current).await?;
        block_on_task(&state.db, row, &all).await?;
        return Ok(None);
    }
    if status == State::Pending {
        return Ok(None);
    }
    if !continuation::original_owner_source(&state.db, row).await? {
        block_on_task(&state.db, row, &all).await?;
        return Ok(None);
    }
    let mut receipts = Vec::new();
    let mut notes = Vec::new();
    for id in &handoff.request_ids {
        let request = all
            .iter()
            .find(|request| &request.id == id)
            .ok_or_else(missing)?;
        let p = proposal(request)?;
        let original = p.owner_continuation.as_ref().ok_or_else(conflict)?;
        let Some(text) = result_for_human(state, request).await? else {
            block_on_task(&state.db, row, &all).await?;
            return Ok(None);
        };
        notes.push(text);
        receipts.push(continuation::Receipt {
            id: id.clone(),
            version: AssistantVersion::of(request)?,
            claim_id: p.claim_id.clone().ok_or_else(conflict)?,
            batch_turn_id: original.batch_turn_id.clone().ok_or_else(conflict)?,
            original_batch: original.batch.clone().ok_or_else(conflict)?,
            handoff_turn_id: handoff.turn_id.clone(),
        });
    }
    Ok(Some(
        continuation::prepare(state, row, receipts, notes.join("\n\n")).await?,
    ))
}
