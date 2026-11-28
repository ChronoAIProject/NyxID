//! Fenced handoffs retain independent decision proofs; IDs never become sends.
use super::*;
use crate::models::assistant_conversation::{AgentEvent, TurnOrigin};
use crate::models::execution_proposal::OwnerContinuationHandoff as Handoff;

fn canonical(ids: &[String]) -> bool {
    !ids.is_empty() && ids.len() <= 64 && ids.windows(2).all(|pair| pair[0] < pair[1])
}

fn request_ids(events: &[AgentEvent]) -> Vec<String> {
    let mut ids: Vec<_> = events
        .iter()
        .filter_map(|event| {
            event
                .id
                .strip_prefix("execution-decision:")
                .map(str::to_owned)
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

pub(super) fn typed_wake(events: &[AgentEvent]) -> bool {
    events.iter().all(|event| {
        event.kind == "permission_decided" && event.id.starts_with("execution-decision:")
    })
}

fn event_set(events: &[AgentEvent]) -> AppResult<Vec<Document>> {
    let mut events: Vec<_> = events.iter().collect();
    events.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.kind.cmp(&b.kind)));
    let mut proof = events
        .into_iter()
        .map(bson::to_document)
        .collect::<Result<Vec<_>, _>>()
        .map_err(encoding)?;
    proof.dedup();
    Ok(proof)
}

/// Server-only ownership captured from the admitted snapshot. Carry the same
/// worker and event proof through preparation and the provider's send barrier.
#[derive(Clone, PartialEq)]
pub(super) struct TurnAuthority {
    filter: Document,
    events: Vec<Document>,
}

impl TurnAuthority {
    pub(super) fn capture(row: &AssistantConversation) -> AppResult<Self> {
        let turn = row.active_turn.as_ref().ok_or_else(refused)?;
        let task = turn.execution_task.as_ref().ok_or_else(refused)?;
        let mut filter = crate::services::assistant_nyxagent::WorkerFence::capture(row)?.filter();
        if let Some(worker) = &turn.worker_instance {
            filter.insert("active_turn.worker_instance.id", &worker.id);
        } else {
            filter.insert("active_turn.worker_instance", bson::Bson::Null);
        }
        filter.extend(continuation::current_task_filter(task)?);
        filter.extend(doc! {"credential_api_key_id":&row.credential_api_key_id,
        "active_turn.stop_requested":false,"$expr":continuation::unexpired_turn(),
        "model":&row.model,"agent_id":&row.agent_id});
        Ok(Self {
            filter,
            events: event_set(&turn.events)?,
        })
    }

    pub(super) async fn fence(&self, db: &Database, session: &mut ClientSession) -> AppResult<()> {
        let bound = db
            .collection::<Document>("assistant_conversations")
            .find_one_and_update(
                self.filter.clone(),
                doc! {"$inc":{"execution_dispatch_fence":1_i64}},
            )
            .return_document(mongodb::options::ReturnDocument::After)
            .projection(doc! {"active_turn.events":1})
            .session(&mut *session)
            .await?
            .ok_or_else(refused)?;
        let events: Vec<AgentEvent> = bson::from_bson(
            bound
                .get_document("active_turn")
                .map_err(encoding)?
                .get("events")
                .cloned()
                .ok_or_else(refused)?,
        )
        .map_err(encoding)?;
        if event_set(&events)? != self.events {
            return Err(refused());
        }
        Ok(())
    }
}

pub(super) async fn fence_turn(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
) -> AppResult<()> {
    TurnAuthority::capture(row)?.fence(db, session).await
}

pub(super) async fn fence_original_task(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
) -> AppResult<()> {
    if db
        .collection::<Document>("assistant_messages")
        .update_one(
            continuation::original_task_filter(row)?,
            doc! {"$inc":{"execution_dispatch_fence":1_i64}},
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

/// Drop only invalid entries in the retained-execution event namespace. This
/// read shares the source conversation's admission transaction and key snapshot;
/// no foreign proposal data is rendered or changed, and unrelated events remain.
pub(crate) async fn filter_decisions(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    mut events: Vec<AgentEvent>,
) -> AppResult<Vec<AgentEvent>> {
    if row.guest_turn
        || row.shared_agent_thread
        || row.agent_owner_id.is_some()
        || row.group_id.is_some()
        || row.collaboration.is_some()
        || row.platform_thread.is_some()
        || row.automation_thread
        || row.invocation_controlled
        || row.voice_parent_conversation_id.is_some()
    {
        return Ok(events);
    }
    let ids = request_ids(&events);
    if ids.is_empty() {
        return Ok(events);
    }
    if ids.len() > 64 {
        return Err(refused());
    }
    let current = load(db, session, row, &ids).await?;
    events.retain(|event| {
        let Some(id) = event.id.strip_prefix("execution-decision:") else {
            return true;
        };
        event.kind == "permission_decided"
            && current.iter().any(|request| {
                request.id == id
                    && proposal(request)
                        .is_ok_and(|p| event.execution_task.as_ref() == Some(&p.task))
            })
    });
    Ok(events)
}

/// A later successful subset cannot erase an ordinary, denied or waiting member
/// of the server-admitted wake. This decision needs no optional readiness read.
fn receipt_only(row: &AssistantConversation, requests: &[ApprovalRequest], typed: bool) -> bool {
    !typed
        || requests.iter().any(|request| {
            !proposal(request).is_ok_and(|p| {
                request.status == "approved"
                    && AssistantVersion::of(request)
                        .is_ok_and(|version| continuation::eligible(row, p, version))
                    && !p.delivery_abandoned
                    && p.owner_continuation
                        .as_ref()
                        .is_none_or(|proof| !proof.blocked)
                    && (matches!(p.state.as_str(), "queued" | "preparing" | "dispatched")
                        || (p.state == "completed"
                            && request.activity_outcome.as_deref() == Some("completed")
                            && p.result.is_some()
                            && p.claim_id.is_some()))
            })
        })
}

/// A confirmed success already released or delivered is an observation, not a
/// member of a fresh continuation. Every authority/outcome fact still matches.
fn settled_success(row: &AssistantConversation, request: &ApprovalRequest) -> bool {
    proposal(request).is_ok_and(|p| {
        request.status == "approved"
            && p.state == "completed"
            && request.activity_outcome.as_deref() == Some("completed")
            && p.result.is_some()
            && p.claim_id.is_some()
            && !p.delivery_abandoned
            && AssistantVersion::of(request)
                .is_ok_and(|version| continuation::eligible(row, p, version))
            && p.owner_continuation
                .as_ref()
                .is_none_or(|proof| !proof.blocked)
            && (p.delivered_turn_id.is_some()
                || p.owner_continuation
                    .as_ref()
                    .is_some_and(|proof| proof.released_turn_id.is_some()))
    })
}

/// The admission transaction owns the turn/worker claim and removes these exact
/// named events from the queue. Persist their intent before any resume/readiness
/// work can fail. Unmarked legacy roots only need durable refusal evidence.
pub(crate) async fn record_admission(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
) -> AppResult<()> {
    let Some(task) = continuation::owner_task(row) else {
        return Ok(());
    };
    let turn = row.active_turn.as_ref().ok_or_else(refused)?;
    if turn.origin != TurnOrigin::Event {
        return Ok(());
    }
    let ids = request_ids(&turn.events);
    if ids.is_empty() {
        return Ok(());
    }
    if !canonical(&ids) {
        return Err(refused());
    }
    let current = load(db, session, row, &ids).await?;
    if current.len() != ids.len() {
        // Credential resolution can adopt a successor after queue filtering.
        // Do not roll back consumption of an old-key event; resume's exact-key
        // lookup returns a safe terminal failure before any send or model call.
        return Ok(());
    }
    if current
        .iter()
        .any(|request| !proposal(request).is_ok_and(|p| p.task == *task))
    {
        return Err(refused());
    }
    if !current.iter().any(|request| {
        proposal(request).is_ok_and(|p| {
            AssistantVersion::of(request)
                .is_ok_and(|version| continuation::eligible(row, p, version))
        })
    }) {
        return Ok(());
    }
    let blocked = receipt_only(row, &current, typed_wake(&turn.events));
    let mut source = tasks::live_filter(task)?;
    source.extend(doc! {"user_id":&row.user_id,"conversation_id":&row.id});
    let root = db
        .collection::<Document>("assistant_messages")
        .find_one(source)
        .projection(doc! {"execution_dispatch_evidence_version":1})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    if !blocked && root.get_i32("execution_dispatch_evidence_version").ok() != Some(2) {
        return Ok(());
    }
    stamp_fresh(db, session, row, &current, blocked).await?;
    let current = load(db, session, row, &ids).await?;
    if blocked {
        block_in(db, session, &current).await
    } else {
        join_handoff(db, session, row, &current, false).await
    }
}

async fn stamp_fresh(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    requests: &[ApprovalRequest],
    blocked: bool,
) -> AppResult<()> {
    let mut fresh: Vec<_> = requests
        .iter()
        .filter(|request| {
            !settled_success(row, request)
                && proposal(request).is_ok_and(|p| p.owner_continuation.is_none())
        })
        .map(|request| request.id.clone())
        .collect();
    fresh.sort();
    let turn = row.active_turn.as_ref().ok_or_else(refused)?;
    for request in requests {
        let p = proposal(request)?;
        let version = AssistantVersion::of(request)?;
        let actionable =
            request.status == "approved" && matches!(p.state.as_str(), "queued" | "preparing");
        let unsent_decision = matches!(request.status.as_str(), "rejected" | "expired")
            && matches!(p.state.as_str(), "denied" | "expired" | "cancelled");
        if settled_success(row, request)
            || p.owner_continuation.is_some()
            || !(blocked || actionable || unsent_decision)
            || !continuation::eligible(row, p, version)
        {
            continue;
        }
        if db
            .collection::<Document>(REQUESTS)
            .update_one(
                version.selector(doc! {
                    "_id":&request.id,"user_id":&row.user_id,
                    "execution_proposal.v1.key_id":&row.credential_api_key_id,
                    "execution_proposal.v1.conversation_id":&row.id,
                    "execution_proposal.v1.owner_continuation":null,
                    "execution_proposal.v1.state":&p.state,"status":&request.status,
                    "execution_proposal.v1.claim_id":&p.claim_id,
                    "execution_proposal.v1.resume_turn_id":&p.resume_turn_id,
                }),
                version.update(doc! {"$set":{"execution_proposal.v1.owner_continuation":{
                    "version":0,"batch":&fresh,"batch_turn_id":&turn.turn_id,"blocked":blocked,
                    "released_turn_id":bson::Bson::Null,"handoff":bson::Bson::Null,
                    "admission_handoff":{"version":1,"request_ids":&fresh,"turn_id":&turn.turn_id},
                }}}),
            )
            .session(&mut *session)
            .await?
            .matched_count
            != 1
        {
            return Err(conflict());
        }
    }
    Ok(())
}

/// Extend only receipt evidence. Admission and resume use the same atomic
/// intent, so optional readiness failure cannot split a co-admitted group.
/// Only the ready resume path promotes that intent into an eligible handoff.
async fn join_handoff(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
    current: &[ApprovalRequest],
    promote: bool,
) -> AppResult<()> {
    let task = continuation::owner_task(row).ok_or_else(refused)?;
    let turn = row.active_turn.as_ref().ok_or_else(refused)?;
    // Keep released successes out of the new proof without dropping independent
    // fresh members. Unsafe terminal observations still block the remaining group.
    let pending: Vec<_> = current
        .iter()
        .filter(|request| !settled_success(row, request))
        .cloned()
        .collect();
    let current = pending.as_slice();
    if current.is_empty() {
        return Ok(());
    }
    if current.iter().any(|request| {
        proposal(request).is_ok_and(|p| {
            p.delivered_turn_id.is_some()
                || p.owner_continuation
                    .as_ref()
                    .is_some_and(|proof| proof.released_turn_id.is_some())
        })
    }) {
        block_in(db, session, current).await?;
        return Ok(());
    }
    let mut union = Vec::new();
    for request in current {
        let Some(group) = cohort(request)? else {
            block_in(db, session, current).await?;
            return Ok(());
        };
        union.extend(group.request_ids);
    }
    union.sort();
    union.dedup();
    if !canonical(&union) {
        block_in(db, session, current).await?;
        return Ok(());
    }
    let all = load(db, session, row, &union).await?;
    if !valid_proofs(row, &all, &union)? {
        block_in(db, session, current).await?;
        let own: Vec<_> = all
            .iter()
            .filter(|request| proposal(request).is_ok_and(|p| p.task == *task))
            .cloned()
            .collect();
        block_in(db, session, &own).await?;
        return Ok(());
    }
    let first = all.first().map(cohort).transpose()?.flatten();
    let shared = first.as_ref().filter(|first| {
        first.request_ids == union
            && all
                .iter()
                .all(|request| cohort(request).ok().flatten().as_ref() == Some(*first))
    });
    let joined = shared.cloned().unwrap_or_else(|| Handoff {
        version: 1,
        request_ids: union.clone(),
        turn_id: turn.turn_id.clone(),
    });
    if all.iter().all(|request| {
        proposal(request).is_ok_and(|p| {
            p.owner_continuation.as_ref().is_some_and(|proof| {
                proof.admission_handoff.as_ref() == Some(&joined)
                    && (!promote || (proof.version == 3 && proof.handoff.as_ref() == Some(&joined)))
            })
        })
    }) {
        return Ok(());
    }
    for request in &all {
        let p = proposal(request)?;
        let version = AssistantVersion::of(request)?;
        let proof = p.owner_continuation.as_ref().ok_or_else(conflict)?;
        let mut set = doc! {"execution_proposal.v1.owner_continuation.admission_handoff":bson::to_bson(&joined).map_err(encoding)?};
        if promote {
            set.extend(doc! {"execution_proposal.v1.owner_continuation.version":3,
                "execution_proposal.v1.owner_continuation.handoff":bson::to_bson(&joined).map_err(encoding)?});
        }
        if db.collection::<Document>(REQUESTS).update_one(version.selector(doc! {
            "_id":&request.id,"user_id":&row.user_id,
            "execution_proposal.v1.key_id":&row.credential_api_key_id,
            "execution_proposal.v1.conversation_id":&row.id,
            "execution_proposal.v1.owner_continuation.version":proof.version as i32,
            "execution_proposal.v1.owner_continuation.batch":bson::to_bson(&proof.batch).map_err(encoding)?,
            "execution_proposal.v1.owner_continuation.batch_turn_id":&proof.batch_turn_id,
            "execution_proposal.v1.owner_continuation.admission_handoff":bson::to_bson(&proof.admission_handoff).map_err(encoding)?,
            "execution_proposal.v1.owner_continuation.released_turn_id":null,
            "execution_proposal.v1.delivered_turn_id":null,
        }), version.update(doc! {"$set":set})).session(&mut *session).await?.matched_count != 1 { return Err(conflict()); }
    }
    Ok(())
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
    if let Some(intent) = &proof.admission_handoff {
        return Ok((matches!(proof.version, 0 | 2 | 3)
            && intent.version == 1
            && !intent.turn_id.is_empty()
            && canonical(&intent.request_ids)
            && original.iter().all(|id| intent.request_ids.contains(id)))
        .then(|| intent.clone()));
    }
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
            "execution_proposal.v1.owner_continuation.admission_handoff":bson::to_bson(&marker.admission_handoff).map_err(encoding)?,
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

async fn block(
    db: &Database,
    row: &AssistantConversation,
    requests: &[ApprovalRequest],
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    fence_turn(db, &mut session, row).await?;
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
    block(db, row, &own).await
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

/// A current admitted worker may settle an exact expired/completed root without
/// promoting continuation authority. Missing or changed roots are not this case.
async fn terminal_task(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
) -> AppResult<bool> {
    let mut root = continuation::original_task_filter(row)?;
    root.insert(
        "$expr",
        doc! {"$or":[
            {"$lte":["$execution_task.hard_deadline","$$NOW"]},
            {"$eq":["$execution_task_completed",true]},
        ]},
    );
    Ok(db
        .collection::<Document>("assistant_messages")
        .update_one(root, doc! {"$inc":{"execution_dispatch_fence":1_i64}})
        .session(session)
        .await?
        .matched_count
        == 1)
}

/// Verify admitted proofs and their joined handoff. Older already-admitted
/// turns also persist refusal intent before sending; mixed wakes never need
/// optional continuation readiness to retain that refusal.
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
    // Displaying a terminal receipt does not need a live root, credential,
    // optional readiness or full execution-task equality.
    let active = requests.iter().any(|request| {
        proposal(request).is_ok_and(|p| {
            request.status == "approved"
                && (matches!(p.state.as_str(), "queued" | "preparing")
                    || (p.state == "completed"
                        && request.activity_outcome.as_deref() == Some("completed")
                        && p.result.is_some()
                        && p.claim_id.is_some()
                        && p.delivered_turn_id.is_none()
                        && p.owner_continuation.as_ref().is_some_and(|proof| {
                            matches!(proof.version, 2 | 3)
                                && !proof.blocked
                                && proof.released_turn_id.is_none()
                        })))
        })
    });
    if !active {
        return Ok(None);
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
            && let Some(group) = cohort(request)?
        {
            work |= matches!(proof.version, 2 | 3)
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
    let blocked = receipt_only(row, requests, typed_wake);
    work |= groups.len() > 1
        || groups
            .iter()
            .any(|group| !admitted.iter().all(|id| group.request_ids.contains(id)));
    if !work {
        if blocked {
            block(&state.db, row, requests).await?;
        }
        return Ok(None);
    }
    if requests
        .iter()
        .any(|request| !proposal(request).is_ok_and(|p| p.task == *task))
    {
        return Err(refused());
    }
    // Receipt-only intent is persisted even on an older admitted turn and an
    // unmarked legacy root. It never depends on optional model readiness.
    if !blocked
        && state
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
    let mut session = state.db.client().start_session().await?;
    let bound = session
        .start_transaction()
        .and_run(
            (&state.db, row, task, admitted, typed_wake, blocked),
            |session, (db, row, task, admitted, typed_wake, blocked)| {
                Box::pin(async move {
                    let result: AppResult<bool> = async {
                        fence_turn(db, session, row).await?;
                        if let Err(error) =
                            require_enrolled(db, &row.user_id, Some(&mut *session)).await
                        {
                            if error.is_forbidden() {
                                return Ok(false);
                            }
                            return Err(error);
                        }
                        if request_ids(&row.active_turn.as_ref().ok_or_else(refused)?.events)
                            .as_slice()
                            != *admitted
                        {
                            return Err(refused());
                        }
                        let mut current = load(db, session, row, admitted).await?;
                        if current.len() != admitted.len()
                            || current
                                .iter()
                                .any(|request| !proposal(request).is_ok_and(|p| p.task == **task))
                        {
                            return Err(refused());
                        }
                        let live_root: AppResult<()> = async {
                            tasks::fence(db, session, task).await?;
                            let mut root = tasks::live_filter(task)?;
                            root.extend(doc! {"user_id":&row.user_id,"conversation_id":&row.id});
                            if !*blocked {
                                root.insert("execution_dispatch_evidence_version", 2);
                            }
                            if db
                                .collection::<Document>("assistant_messages")
                                .update_one(root, doc! {"$inc":{"execution_dispatch_fence":1_i64}})
                                .session(&mut *session)
                                .await?
                                .matched_count
                                != 1
                            {
                                return Err(refused());
                            }
                            Ok(())
                        }
                        .await;
                        if let Err(error) = live_root {
                            if error.is_forbidden() && terminal_task(db, session, row).await? {
                                block_in(db, session, &current).await?;
                                return Ok(true);
                            }
                            return Err(error);
                        }
                        if !*blocked {
                            let pending = current.iter().any(|request| {
                                proposal(request).is_ok_and(|p| {
                                    p.delivered_turn_id.is_none()
                                        && p.owner_continuation.as_ref().is_some_and(|proof| {
                                            matches!(proof.version, 2 | 3)
                                                && !proof.blocked
                                                && proof.released_turn_id.is_none()
                                        })
                                })
                            });
                            match crate::services::security_readiness_service::serving_report(
                                db,
                                continuation::READINESS,
                            )
                            .await
                            {
                                Ok(report) if report.serving_ready => {}
                                Err(error) if pending => return Err(error),
                                _ if pending => return Err(AppError::SecurityReadinessUnavailable),
                                _ => return Ok(false),
                            }
                        }
                        // Retry contenders reload both effect claims and refusal intent.
                        // A concurrent winner cannot have its blocked proof overwritten.
                        let blocked = *blocked || receipt_only(row, &current, *typed_wake);
                        stamp_fresh(db, session, row, &current, blocked).await?;
                        current = load(db, session, row, admitted).await?;
                        if blocked {
                            block_in(db, session, &current).await?;
                            return Ok(true);
                        }
                        join_handoff(db, session, row, &current, true).await?;
                        Ok(true)
                    }
                    .await;
                    crate::services::api_key_mutation_service::transaction_result(result)
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error);
    bound.map(Some)
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
                && p.delivered_turn_id.is_none()
                && AssistantVersion::of(request)
                    .is_ok_and(|version| continuation::eligible(row, p, version))
                && p.owner_continuation.as_ref().is_some_and(|proof| {
                    matches!(proof.version, 2 | 3)
                        && !proof.blocked
                        && proof.released_turn_id.is_none()
                })
        })
    });
    let Some(anchor) = anchor else {
        return Ok(None);
    };
    if !typed_wake {
        block(&state.db, row, &current).await?;
        return Ok(None);
    }
    let proof = proposal(anchor)?
        .owner_continuation
        .as_ref()
        .ok_or_else(conflict)?;
    let Some(handoff) = cohort(anchor)?.filter(|_| proof.version == 3) else {
        block(&state.db, row, &current).await?;
        return Ok(None);
    };
    if !admitted.iter().all(|id| {
        handoff.request_ids.contains(id)
            || current
                .iter()
                .any(|request| &request.id == id && settled_success(row, request))
    }) {
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
            block(&state.db, row, &current).await?;
            block_on_task(&state.db, row, &all).await?;
        }
        return Ok(None);
    }
    let status = validate(row, &all, &handoff.request_ids)?;
    if status == State::Invalid {
        block(&state.db, row, &current).await?;
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
