//! Durable first-party owner receipts. Admission never reopens an execution
//! claim: an uncertain commit/start is recovered by inspection and orphan settlement.
use super::{api_key_mutation_service as transactions, assistant_nyxagent as engine};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{
            ActiveTurn, AssistantConversation, COLLECTION_NAME as CONVERSATIONS,
        },
        assistant_invocation::{
            AssistantInvocation, COLLECTION_NAME as INVOCATIONS, InvocationTurn, TURNS,
        },
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    },
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, doc},
    options::IndexOptions,
};
use uuid::Uuid;

pub const MAX_TURNS: i64 = 128;
pub fn not_found() -> AppError {
    AppError::NotFound("Assistant invocation not found".into())
}
pub fn conflict() -> AppError {
    AppError::Conflict("Invocation idempotency key already binds a different request".into())
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<AssistantInvocation>(INVOCATIONS)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"user_id":1,"idempotency_key":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id":1,"conversation_id":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id":1,"conversation_ids":1,"terminal_status":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"cancel_requested":1,"terminal_status":1,"cancel_retry_at":1})
                .build(),
        ])
        .await?;
    db.collection::<InvocationTurn>(TURNS)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"user_id":1,"invocation_id":1,"created_at":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id":1,"conversation_id":1})
                .build(),
        ])
        .await?;
    for (collection, keys) in [
        (
            crate::models::assistant_agent_learning::PROPOSALS_COLLECTION_NAME,
            doc! {"owner_id":1,"publication.invocation_id":1,"status":1},
        ),
        (
            crate::models::assistant_agent_learning::AUTHORED_CLAIMS_COLLECTION_NAME,
            doc! {"claim.conversation_id":1},
        ),
        (
            CONVERSATIONS,
            doc! {"user_id":1,"pending_events.invocation_id":1},
        ),
        (
            CONVERSATIONS,
            doc! {"user_id":1,"active_turn.invocation_id":1},
        ),
        (
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
            doc! {"user_id":1,"invocation_id":1,"status":1},
        ),
        (
            crate::models::async_service_operation::COLLECTION_NAME,
            doc! {"user_id":1,"delivery.invocation_id":1,"state":1},
        ),
        (
            crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME,
            doc! {"user_id":1,"invocation_id":1,"status":1},
        ),
        (
            crate::models::machine_job::COLLECTION_NAME,
            doc! {"user_id":1,"issuing_turn_id":1,"state":1},
        ),
        (
            MESSAGES,
            doc! {"user_id":1,"turn_id":1,"role":1,"created_at":-1,"_id":-1},
        ),
    ] {
        db.collection::<bson::Document>(collection)
            .create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    Ok(())
}

pub async fn get(db: &Database, owner: &str, key: &str) -> AppResult<AssistantInvocation> {
    db.collection::<AssistantInvocation>(INVOCATIONS)
        .find_one(doc! {"user_id":owner,"idempotency_key":key})
        .await?
        .ok_or_else(not_found)
}
pub fn same_request(row: AssistantInvocation, digest: &str) -> AppResult<AssistantInvocation> {
    if row.request_digest != digest {
        return Err(conflict());
    }
    Ok(row)
}

/// Public owner invocations use private owner threads. Hidden group, voice,
/// automation and platform/channel threads have independent delivery protocols.
pub fn require_owner_conversation(row: &AssistantConversation) -> AppResult<()> {
    if row.group_id.is_some()
        || row.group_request_id.is_some()
        || row.platform_thread.is_some()
        || row.channel.is_some()
        || row.voice_parent_conversation_id.is_some()
        || row.automation_thread
    {
        return Err(AppError::ValidationError(
            "Owner invocations require a private owner conversation; choose a new conversation instead of a group, voice, automation, channel or platform thread".into(),
        ));
    }
    Ok(())
}

pub struct Admission {
    pub invocation: AssistantInvocation,
    pub claim: Option<engine::OwnerTurnClaim>,
}

pub async fn admit(
    db: &Database,
    owner: &str,
    key: &str,
    digest: &str,
    start: &engine::TurnStart,
    keys: &std::sync::Arc<crate::crypto::aes::EncryptionKeys>,
) -> AppResult<Admission> {
    // Read before provisioning, including for deleted conversations. A receipt
    // remains a receipt forever; it can never become a fresh submission.
    match get(db, owner, key).await {
        Ok(row) => {
            return Ok(Admission {
                invocation: same_request(row, digest)?,
                claim: None,
            });
        }
        Err(AppError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    let agent = if let Some(id) = &start.conversation_id {
        let conversation = engine::get(db, owner, id).await?;
        require_owner_conversation(&conversation)?;
        let agent =
            super::assistant_team_service::agent_for_conversation(db, &conversation).await?;
        if agent.user_id != owner {
            return Err(not_found());
        }
        agent
    } else if let Some(id) = start.agent_id.as_deref() {
        let agent = super::assistant_team_service::agent(db, owner, id).await?;
        if agent.user_id != owner {
            return Err(not_found());
        }
        agent
    } else {
        super::assistant_team_service::ensure_nyxbot(db, owner).await?
    };
    let learning = if start.conversation_id.is_none() {
        Box::pin(super::assistant_agent_learning::enrollment_epoch(
            db, owner, &agent, None,
        ))
        .await?
    } else {
        None
    };
    let ids = engine::OwnerTurnIds {
        conversation_id: start
            .conversation_id
            .clone()
            .unwrap_or_else(|| format!("nyxa-{}", Uuid::new_v4().simple())),
        turn_id: Uuid::new_v4().to_string(),
        message_id: Uuid::new_v4().to_string(),
    };
    let invocation = AssistantInvocation {
        id: Uuid::new_v4().to_string(),
        user_id: owner.into(),
        idempotency_key: key.into(),
        request_digest: digest.into(),
        conversation_id: ids.conversation_id.clone(),
        conversation_ids: vec![ids.conversation_id.clone()],
        turn_id: ids.turn_id.clone(),
        cancel_requested: false,
        terminal_status: None,
        conversation_deleted: false,
        continuation_blocked: false,
        unconfirmed_async_cancellations: Vec::new(),
        generation: 0,
        turn_count: 0,
        created_at: Utc::now(),
    };
    let mut start = start.clone();
    start.invocation_id = Some(invocation.id.clone());
    let mut session = db.client().start_session().await?;
    let db2 = db.clone();
    let owner2 = owner.to_owned();
    let keys = keys.clone();
    let candidate = invocation.clone();
    let result = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<_> = Box::pin(async {
                if let Some(existing) = db2
                    .collection::<AssistantInvocation>(INVOCATIONS)
                    .find_one(doc! {"user_id":&owner2,"idempotency_key":&candidate.idempotency_key})
                    .session(&mut *session)
                    .await?
                {
                    return Ok(Admission {
                        invocation: same_request(existing, &candidate.request_digest)?,
                        claim: None,
                    });
                }
                db2.collection::<AssistantInvocation>(INVOCATIONS)
                    .insert_one(&candidate)
                    .session(&mut *session)
                    .await?;
                let claim = Box::pin(engine::begin_owner_turn_in_session(
                    &db2, session, &keys, &owner2, &start, &ids, &agent, learning, None, None,
                ))
                .await?
                .into_started()?;
                Ok(Admission {
                    invocation: candidate.clone(),
                    claim: Some(claim),
                })
            })
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error);
    match result {
        Ok(value) => Ok(value),
        // Includes duplicate-key races and ambiguous commit acknowledgement. A
        // read can recover identity, but never authorizes another worker start.
        Err(error) => match get(db, owner, key).await {
            Ok(row) => Ok(Admission {
                invocation: same_request(row, digest)?,
                claim: None,
            }),
            Err(_) => Err(error),
        },
    }
}

/// Every descendant admission writes the root receipt, serializing cancellation
/// against new work. Ordinary turns return before any invocation database read.
pub async fn bind_in_session(
    db: &Database,
    session: &mut ClientSession,
    row: &AssistantConversation,
) -> AppResult<()> {
    let Some(turn) = row.active_turn.as_ref() else {
        return Ok(());
    };
    let Some(id) = turn.invocation_id.as_deref() else {
        return Ok(());
    };
    let admitted = db
        .collection::<AssistantInvocation>(INVOCATIONS)
        .update_one(
            doc! {
                "_id":id,
                "user_id":&row.user_id,
                "cancel_requested":false,
                "terminal_status":null,
                "conversation_deleted":false,
                "turn_count":{"$lt":MAX_TURNS},
            },
            doc! {
                "$inc":{"generation":1_i64,"turn_count":1_i64},
                "$addToSet":{"conversation_ids":&row.id},
            },
        )
        .session(&mut *session)
        .await?
        .matched_count
        == 1;
    if !admitted {
        return Err(AppError::Conflict(
            "Invocation is cancelled, deleted or at its turn limit".into(),
        ));
    }
    db.collection::<InvocationTurn>(TURNS)
        .insert_one(InvocationTurn {
            id: turn.turn_id.clone(),
            user_id: row.user_id.clone(),
            invocation_id: id.into(),
            conversation_id: row.id.clone(),
            state: "running".into(),
            result_message_id: None,
            error_code: None,
            hooks_pending: false,
            created_at: turn.started_at,
            settled_at: None,
            machine_scopes: Vec::new(),
        })
        .session(&mut *session)
        .await?;
    Ok(())
}

pub async fn settle_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    turn: &ActiveTurn,
    message: &AssistantMessage,
) -> AppResult<()> {
    let Some(id) = turn.invocation_id.as_deref() else {
        return Ok(());
    };
    let state = if message.error_code.is_some() {
        "failed"
    } else {
        "completed"
    };
    let machine_scopes = bson::to_bson(&turn.machine_scopes)
        .map_err(|_| AppError::Internal("Invalid machine scopes".into()))?;
    db.collection::<InvocationTurn>(TURNS)
        .update_one(
            doc! {"_id": &turn.turn_id, "user_id": owner, "invocation_id": id},
            doc! {
                "$set": {
                    "state": state,
                    "error_code": &message.error_code,
                    "result_message_id": &message.id,
                    "settled_at": bson::DateTime::now(),
                    "hooks_pending": true,
                    "machine_scopes": machine_scopes,
                },
            },
        )
        .session(&mut *session)
        .await?;
    Ok(())
}

pub async fn hooks_done(db: &Database, row: &AssistantConversation) -> AppResult<()> {
    let Some(turn) = row
        .active_turn
        .as_ref()
        .filter(|t| t.invocation_id.is_some())
    else {
        return Ok(());
    };
    db.collection::<InvocationTurn>(TURNS)
        .update_one(
            doc! {"_id":&turn.turn_id,"user_id":&row.user_id},
            doc! {"$set":{"hooks_pending":false}},
        )
        .await?;
    Ok(())
}

pub async fn turns(db: &Database, owner: &str, id: &str) -> AppResult<Vec<InvocationTurn>> {
    Ok(db
        .collection::<InvocationTurn>(TURNS)
        .find(doc! {"user_id":owner,"invocation_id":id})
        .sort(doc! {"created_at":1,"_id":1})
        .limit(MAX_TURNS)
        .await?
        .try_collect()
        .await?)
}

/// Removing either a root or an associated conversation cancels its family.
/// Capture scopes and settle the removed conversation's association before its
/// active-turn row disappears. Receipts and turn identities remain tombstones.
pub async fn deleted_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    conversation: &str,
) -> AppResult<()> {
    // Mark all affected live families without loading their historical turns.
    // The existing sweep recovers every durable cancellation, in bounded pages.
    db.collection::<bson::Document>(INVOCATIONS)
        .update_many(
            doc! {"user_id": owner, "conversation_ids": conversation, "terminal_status": null},
            doc! {
                "$set": {
                    "cancel_requested": true,
                    "cancel_retry_at": bson::DateTime::now(),
                },
                "$inc": {"generation": 1_i64},
            },
        )
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(INVOCATIONS)
        .update_many(
            doc! {"user_id":owner,"conversation_id":conversation},
            doc! {"$set":{"conversation_deleted":true}},
        )
        .session(&mut *session)
        .await?;
    // Capture the deleted thread's exact scopes before its document disappears.
    if let Some(conversation) = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":conversation,"user_id":owner})
        .session(&mut *session)
        .await?
        && let Some(turn) = conversation
            .active_turn
            .as_ref()
            .filter(|t| t.invocation_id.is_some())
    {
        let machine_scopes = bson::to_bson(&turn.machine_scopes)
            .map_err(|_| AppError::Internal("Invalid scopes".into()))?;
        db.collection::<bson::Document>(TURNS)
            .update_one(
                doc! {"_id": &turn.turn_id, "user_id": owner},
                doc! {"$set": {"machine_scopes": machine_scopes}},
            )
            .session(&mut *session)
            .await?;
    }
    db.collection::<bson::Document>(TURNS)
        .update_many(
            doc! {"user_id": owner, "conversation_id": conversation, "state": "running"},
            doc! {
                "$set": {
                    "state": "failed",
                    "error_code": "cancelled",
                    "hooks_pending": false,
                    "settled_at": bson::DateTime::now(),
                },
            },
        )
        .session(&mut *session)
        .await?;
    Ok(())
}

/// Accelerate the first bounded page after deletion. The durable sweep handles
/// remaining families and crashes between the deletion commit and this call.
pub async fn cancel_deleted_conversation(
    state: &crate::AppState,
    owner: &str,
    conversation: &str,
) -> AppResult<()> {
    let roots: Vec<AssistantInvocation> = state
        .db
        .collection(INVOCATIONS)
        .find(doc! {
            "user_id": owner,
            "conversation_ids": conversation,
            "cancel_requested": true,
            "terminal_status": null,
        })
        .limit(8)
        .await?
        .try_collect()
        .await?;
    for root in roots {
        cancel(state, owner, &root.idempotency_key).await?;
    }
    Ok(())
}

/// The action decision already owns the card and conversation transaction.
/// Queue one server-authored continuation, never auto-approve or change policy.
pub async fn decision_in_session(
    db: &Database,
    session: &mut ClientSession,
    card: &crate::models::assistant_acknowledgement::AssistantAcknowledgement,
) -> AppResult<Option<String>> {
    let Some(id) = card.invocation_id.as_deref() else {
        return Ok(None);
    };
    if !fence_in_session(db, session, &card.user_id, Some(id)).await? {
        return Err(AppError::Conflict("Invocation was stopped".into()));
    }
    // The decision and continuation commit together, including specialist work.
    let text = format!(
        "The owner {} the confirmation (acknowledgement_id {}). Continue within existing authority. Never retry a denied action. Report the result.",
        card.status, card.id,
    );
    let mut event = super::assistant_team_service::event("permission_decided", text, None)
        .bound_to(
            card.routing
                .as_ref()
                .and_then(|routing| routing.report_to()),
        )
        .for_invocation(Some(id));
    event.id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("invocation-decision:{}", card.id).as_bytes(),
    )
    .to_string();
    let entry =
        bson::to_bson(&event).map_err(|_| AppError::Internal("Invalid continuation".into()))?;
    let result = db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            protect_queue(
                doc! {"_id": &card.conversation_id, "user_id": &card.user_id},
                1,
                true,
            ),
            doc! {"$push": {"pending_events": entry}},
        )
        .session(&mut *session)
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "Continuation queue is full or unavailable".into(),
        ));
    }
    Ok(Some(event.id))
}

/// Serialize all family cancellation effects with admission and event producers.
async fn cancel_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    row: &AssistantInvocation,
) -> AppResult<()> {
    use crate::models::{
        assistant_acknowledgement as cards, async_service_operation as operations,
        nyxbot_channel as watches,
    };
    let conversations = db.collection::<bson::Document>(CONVERSATIONS);
    let mut cursor = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(doc! {"user_id": owner, "active_turn.invocation_id": &row.id})
        .limit(MAX_TURNS)
        .session(&mut *session)
        .await?;
    let active: Vec<AssistantConversation> = cursor.stream(&mut *session).try_collect().await?;
    for conversation in &active {
        let turn = conversation
            .active_turn
            .as_ref()
            .ok_or(AppError::AssistantTurnRequired)?;
        let scopes = bson::to_bson(&turn.machine_scopes)
            .map_err(|_| AppError::Internal("Invalid scopes".into()))?;
        conversations.update_one(
            doc! {
                "_id": &conversation.id, "user_id": owner,
                "active_turn.turn_id": &turn.turn_id, "active_turn.invocation_id": &row.id,
            },
            doc! {"$set": {"active_turn.stop_requested": true}, "$inc": {"authority_fence": 1_i64}},
        ).session(&mut *session).await?;
        db.collection::<bson::Document>(TURNS)
            .update_one(
                doc! {"_id": &turn.turn_id, "user_id": owner, "invocation_id": &row.id},
                doc! {"$set": {"machine_scopes": scopes}},
            )
            .session(&mut *session)
            .await?;
    }
    conversations
        .update_many(
            doc! {"user_id": owner, "pending_events.invocation_id": &row.id},
            doc! {"$pull": {"pending_events": {"invocation_id": &row.id}}},
        )
        .session(&mut *session)
        .await?;
    let acknowledgements = db.collection::<bson::Document>(cards::COLLECTION_NAME);
    acknowledgements.update_many(
        doc! {"user_id": owner, "invocation_id": &row.id, "status": {"$in": ["pending", "allowed"]}},
        doc! {"$set": {"status": "expired", "decided_at": bson::DateTime::now(), "expires_at": bson::DateTime::now()}},
    ).session(&mut *session).await?;
    acknowledgements.update_many(
        doc! {"user_id": owner, "invocation_id": &row.id},
        doc! {
            "$unset": {"skill_notification_pending": ""},
            "$set": {"skill_notification_terminal": {"reason": "invocation_stopped", "at": bson::DateTime::now()}},
        },
    ).session(&mut *session).await?;
    db.collection::<bson::Document>(operations::COLLECTION_NAME)
        .update_many(
            doc! {
                "user_id": owner, "delivery.invocation_id": &row.id,
                "state": {"$in": ["submitting", "waiting", "ready", "queued"]},
            },
            doc! {"$set": {
                "state": "cancelling", "result_encrypted": null,
                "lease_id": null, "lease_until": null, "poll_at": bson::DateTime::now(),
            }},
        )
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME).update_many(
        doc! {"user_id": owner, "invocation_id": &row.id, "status": {"$in": ["pending", "claimed"]}},
        doc! {"$set": {"status": "failed", "last_error": "cancelled"}},
    ).session(&mut *session).await?;
    Ok(())
}

/// Fence the family and capture exact scopes before contacting nodes. Repeating
/// this operation can retry cancellation, but can never re-execute a turn.
pub async fn cancel(state: &crate::AppState, owner: &str, key: &str) -> AppResult<()> {
    let db = state.db.clone();
    let owner = owner.to_owned();
    let key = key.to_owned();
    let mut session = db.client().start_session().await?;
    let db2 = db.clone();
    let owner2 = owner.clone();
    let invocation = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<_> = Box::pin(async {
                let roots = db2.collection::<AssistantInvocation>(INVOCATIONS);
                if let Some(row) = roots
                    .find_one(doc! {
                        "user_id":&owner2,
                        "idempotency_key":&key,
                        "terminal_status":{"$type":"string"},
                    })
                    .session(&mut *session)
                    .await?
                {
                    return Ok(row);
                }
                let row = roots
                    .find_one_and_update(
                        doc! {"user_id":&owner2,"idempotency_key":&key},
                        doc! {"$set":{"cancel_requested":true},"$inc":{"generation":1_i64}},
                    )
                    .return_document(mongodb::options::ReturnDocument::After)
                    .session(&mut *session)
                    .await?
                    .ok_or_else(not_found)?;
                cancel_in_session(&db2, session, &owner2, &row).await?;
                Ok(row)
            })
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    if invocation.terminal_status.is_some() {
        return Ok(());
    }
    let family = turns(&db, &owner, &invocation.id).await?;
    let turn_ids: Vec<_> = family.iter().map(|t| t.id.clone()).collect();
    let jobs: Vec<crate::models::machine_job::MachineJob> = db
        .collection(crate::models::machine_job::COLLECTION_NAME)
        .find(doc! {"user_id":&owner,"issuing_turn_id":{"$in":&turn_ids},"state":"running"})
        .limit(1024)
        .await?
        .try_collect()
        .await?;
    let mut groups =
        std::collections::BTreeMap::<(String, String, String, String), Vec<String>>::new();
    for turn in &family {
        for scope in &turn.machine_scopes {
            groups
                .entry((
                    turn.conversation_id.clone(),
                    turn.id.clone(),
                    scope.node_id.clone(),
                    scope.runtime_id.clone(),
                ))
                .or_default();
        }
    }
    for job in &jobs {
        if let Some(turn) = &job.issuing_turn_id {
            groups
                .entry((
                    job.conversation_id.clone(),
                    turn.clone(),
                    job.node_id.clone(),
                    job.runtime_id.clone(),
                ))
                .or_default()
                .push(job.id.clone());
        }
    }
    use futures::StreamExt;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let outcomes: Vec<_> = futures::stream::iter(groups)
        .map(|((conversation, turn, node_id, runtime), ids)| async move {
            let result = tokio::time::timeout_at(deadline, async {
                let nodes = state
                    .db
                    .collection::<crate::models::node::Node>(crate::models::node::COLLECTION_NAME);
                let Some(node) = nodes.find_one(doc! {"_id": &node_id}).await? else {
                    return AppResult::Ok(false);
                };
                if node
                    .machine
                    .as_ref()
                    .is_none_or(|machine| machine.runtime_id != runtime)
                {
                    return Ok(true);
                }
                let response = super::machine_service::dispatch(
                    state,
                    &node,
                    nyxid_machine::Operation::CancelExact,
                    serde_json::json!({
                        "conversation_id": conversation,
                        "turn_id": turn,
                        "job_ids": ids,
                        "runtime_id": runtime,
                    }),
                )
                .await;
                let Some(latest) = nodes.find_one(doc! {"_id": &node_id}).await? else {
                    return Ok(false);
                };
                if latest
                    .machine
                    .as_ref()
                    .is_none_or(|machine| machine.runtime_id != runtime)
                {
                    return Ok(true);
                }
                let response = response?;
                Ok(response.get("stopped").and_then(serde_json::Value::as_bool) == Some(true))
            })
            .await;
            (turn, node_id, runtime, matches!(result, Ok(Ok(true))))
        })
        .buffer_unordered(8)
        .collect()
        .await;
    for job in jobs {
        if outcomes.iter().any(|(turn, node, runtime, ok)| {
            *ok && Some(turn) == job.issuing_turn_id.as_ref()
                && node == &job.node_id
                && runtime == &job.runtime_id
        }) {
            db.collection::<bson::Document>(crate::models::machine_job::COLLECTION_NAME)
                .update_one(
                    doc! {
                        "_id": &job.id,
                        "user_id": &owner,
                        "issuing_turn_id": &job.issuing_turn_id,
                        "runtime_id": &job.runtime_id,
                        "state": "running",
                    },
                    doc! {
                        "$set": {
                            "state": "finished",
                            "finished_at": bson::DateTime::now(),
                        },
                    },
                )
                .await?;
        }
    }
    for turn in family {
        if outcomes
            .iter()
            .filter(|(id, _, _, _)| id == &turn.id)
            .all(|(_, _, _, ok)| *ok)
            && db
                .collection::<bson::Document>(crate::models::machine_job::COLLECTION_NAME)
                .find_one(doc! {"user_id":&owner,"issuing_turn_id":&turn.id,"state":"running"})
                .projection(doc! {"_id":1})
                .await?
                .is_none()
        {
            db.collection::<bson::Document>(TURNS)
                .update_one(
                    doc! {"_id":&turn.id,"user_id":&owner,"invocation_id":&invocation.id},
                    doc! {"$set":{"machine_cancel_complete":true}},
                )
                .await?;
        }
        // The shared settlement is exact-turn fenced even if a browser has
        // already claimed a successor. No conversation-scoped Stop is called.
        match engine::force_stop(&db, &owner, &turn.conversation_id, &turn.id).await {
            Ok(_) | Err(AppError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub async fn fence_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    id: Option<&str>,
) -> AppResult<bool> {
    let Some(id) = id else { return Ok(true) };
    let changed = db
        .collection::<AssistantInvocation>(INVOCATIONS)
        .update_one(
            doc! {
                "_id":id,
                "user_id":owner,
                "cancel_requested":false,
                "terminal_status":null,
                "conversation_deleted":false,
            },
            doc! {"$inc":{"generation":1_i64}},
        )
        .session(session)
        .await?;
    Ok(changed.matched_count == 1)
}

/// Approval waits release execution. A later allow must revalidate the exact
/// original turn before it is allowed to dispatch an invocation tool call.
pub async fn revalidate_after_approval(
    db: &Database,
    chat: &super::assistant_acknowledgement_service::ChatAuthority,
) -> AppResult<()> {
    if chat.invocation_id.is_none() {
        return Ok(());
    }
    let database = db.clone();
    let chat = chat.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result = Box::pin(async {
                let (row, _, _) =
                    super::assistant_acknowledgement_service::fence(&database, &chat, session)
                        .await?;
                super::assistant_access_policy::require_same_live_turn(&chat, &row)?;
                Ok(())
            })
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

/// A bounded machine job retains its issuing turn's invocation after that turn
/// settles. Its current conversation authority must never rebind it to another
/// family, and root deletion/cancellation denies service egress immediately.
pub async fn require_machine_job_active(
    db: &Database,
    job: &crate::models::machine_job::MachineJob,
    auth: &crate::mw::auth::AuthUser,
) -> AppResult<()> {
    if !auth
        .assistant_chat
        .as_ref()
        .is_some_and(|chat| chat.invocation_controlled)
    {
        return Ok(());
    }
    let Some(turn) = job.issuing_turn_id.as_deref() else {
        return Err(AppError::MachineAuthorityStale);
    };
    let invocation = db
        .collection::<InvocationTurn>(TURNS)
        .find_one(doc! {
            "_id":turn,"user_id":&job.user_id,"conversation_id":&job.conversation_id,
        })
        .await?;
    // An ordinary owner turn after a completed invocation still uses the
    // mandatory credential boundary but has no invocation receipt of its own.
    let Some(invocation) = invocation else {
        return Ok(());
    };
    if db
        .collection::<AssistantInvocation>(INVOCATIONS)
        .find_one(doc! {
            "_id":&invocation.invocation_id,"user_id":&job.user_id,
            "cancel_requested":false,"conversation_deleted":false,"terminal_status":null,
        })
        .await?
        .is_none()
    {
        return Err(AppError::MachineTurnStopped);
    }
    Ok(())
}

/// Discard ineligible lineage before selecting a turn. The caller persists
/// this queue even when nothing remains, so rejected work cannot poison it.
pub async fn filter_events_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    events: Vec<crate::models::assistant_conversation::AgentEvent>,
) -> AppResult<Vec<crate::models::assistant_conversation::AgentEvent>> {
    let mut eligible = Vec::new();
    let mut families = std::collections::HashMap::new();
    for event in events {
        let keep = if let Some(id) = event.invocation_id.as_deref() {
            if let Some(keep) = families.get(id) {
                *keep
            } else {
                let mut keep = fence_in_session(db, session, owner, Some(id)).await?;
                if keep {
                    let blocked = db
                        .collection::<AssistantInvocation>(INVOCATIONS)
                        .update_one(
                            doc! {"_id":id,"user_id":owner,"turn_count":{"$gte":MAX_TURNS}},
                            doc! {"$set":{"continuation_blocked":true}},
                        )
                        .session(&mut *session)
                        .await?
                        .matched_count
                        == 1;
                    keep = !blocked;
                }
                families.insert(id.to_owned(), keep);
                keep
            }
        } else {
            true
        };
        if keep {
            eligible.push(event);
        }
    }
    Ok(eligible)
}

/// Legacy queues may evict ordinary notices. Once a tracked event is present,
/// an append must fit without evicting its durable continuation.
pub fn protect_queue(mut filter: bson::Document, count: usize, tracked: bool) -> bson::Document {
    let fits = doc! {
        "$lte": [
            {"$add": [{"$size": {"$ifNull": ["$pending_events", []]}}, count as i64]},
            engine::MAX_PENDING_EVENTS as i64,
        ],
    };
    if tracked {
        filter.insert("$expr", fits);
    } else {
        filter.insert(
            "$and",
            vec![doc! {"$or":[{"pending_events.invocation_id":{"$exists":false}},{"$expr":fits}]}],
        );
    }
    filter
}

pub async fn push_events(
    db: &Database,
    owner: &str,
    conversation: &str,
    events: Vec<crate::models::assistant_conversation::AgentEvent>,
) -> AppResult<Option<AssistantConversation>> {
    let db = db.clone();
    let owner = owner.to_owned();
    let conversation = conversation.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<_> = Box::pin(async {
                let mut eligible = Vec::new();
                for event in &events {
                    if fence_in_session(&db, session, &owner, event.invocation_id.as_deref())
                        .await?
                    {
                        eligible.push(event.clone());
                    }
                }
                if eligible.is_empty() {
                    return Ok(None);
                }
                if !engine::append_events_in_session(&db, &owner, &conversation, &eligible, session)
                    .await?
                {
                    return Err(AppError::Conflict(
                        "Invocation continuation queue is unavailable".into(),
                    ));
                }
                let row = db
                    .collection::<AssistantConversation>(CONVERSATIONS)
                    .find_one(doc! {"_id": &conversation, "user_id": &owner})
                    .session(&mut *session)
                    .await?;
                Ok(row)
            })
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

pub struct InvocationReply {
    pub conversation_id: String,
    pub turn_id: String,
    pub message_id: String,
    pub status: String,
    pub error_code: Option<String>,
    pub text: String,
    pub text_truncated: bool,
}
pub struct RequiredAction {
    pub id: String,
    pub conversation_id: String,
    pub kind: String,
    pub expires_at: DateTime<Utc>,
}
pub struct InvocationSnapshot {
    pub invocation_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub status: String,
    pub terminal: bool,
    pub created_at: DateTime<Utc>,
    pub cancel_requested: bool,
    pub turn_count: i64,
    pub running_turns: u64,
    pub background_operations: u64,
    pub pending_continuations: u64,
    pub required_actions: Vec<RequiredAction>,
    pub required_actions_truncated: bool,
    pub replies: Vec<InvocationReply>,
    pub replies_truncated: bool,
    pub error_code: Option<String>,
    pub retry_after_ms: u32,
}

fn bounded_text(text: &str, bytes: usize) -> (String, bool) {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].into(), end < text.len())
}
pub async fn snapshot(db: &Database, owner: &str, key: &str) -> AppResult<InvocationSnapshot> {
    let database = db.clone();
    let owner = owner.to_owned();
    let key = key.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            transactions::transaction_result(
                Box::pin(snapshot_in_session(&database, session, &owner, &key)).await,
            )
        })
        .await
        .map_err(transactions::map_transaction_error)
}

async fn snapshot_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    key: &str,
) -> AppResult<InvocationSnapshot> {
    use crate::models::{
        assistant_acknowledgement as cards, async_service_operation as operations, machine_job,
        nyxbot_channel as watches,
    };
    let row = db
        .collection::<AssistantInvocation>(INVOCATIONS)
        .find_one(doc! {"user_id":owner,"idempotency_key":key})
        .session(&mut *session)
        .await?
        .ok_or_else(not_found)?;
    let mut cursor = db
        .collection::<InvocationTurn>(TURNS)
        .find(doc! {"user_id":owner,"invocation_id":&row.id})
        .sort(doc! {"created_at":1,"_id":1})
        .limit(MAX_TURNS)
        .session(&mut *session)
        .await?;
    let turns: Vec<_> = cursor.stream(&mut *session).try_collect().await?;
    let ids: Vec<_> = turns.iter().map(|t| t.id.as_str()).collect();
    let running = turns.iter().filter(|t| t.state == "running").count() as u64;
    let hooks_pending = turns.iter().filter(|t| t.hooks_pending).count() as u64;
    let hooks_lost = turns.iter().any(|t| {
        t.hooks_pending
            && t.settled_at.is_some_and(|at| {
                at + chrono::Duration::seconds(engine::ACTIVE_TURN_HEARTBEAT_STALE_SECS)
                    < Utc::now()
            })
    });
    let pending = db
        .collection::<bson::Document>(CONVERSATIONS)
        .count_documents(doc! {"user_id":owner,"pending_events.invocation_id":&row.id})
        .session(&mut *session)
        .await?;
    let paused = db
        .collection::<bson::Document>(CONVERSATIONS)
        .find_one(doc! {
            "user_id": owner,
            "pending_events.invocation_id": &row.id,
            "event_streak": {"$gte": super::assistant_team_service::MAX_EVENT_STREAK},
        })
        .session(&mut *session)
        .await?
        .is_some();
    let async_count = db
        .collection::<bson::Document>(operations::COLLECTION_NAME)
        .count_documents(doc! {
            "user_id": owner,
            "delivery.invocation_id": &row.id,
            "state": {"$in": ["submitting", "waiting", "cancelling", "ready", "queued"]},
        })
        .session(&mut *session)
        .await?;
    let watch_count = db
        .collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME)
        .count_documents(
            doc! {"user_id":owner,"invocation_id":&row.id,"status":{"$in":["pending","claimed"]}},
        )
        .session(&mut *session)
        .await?;
    let failed_setup = db
        .collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME)
        .find_one(doc! {
            "user_id": owner,
            "invocation_id": &row.id,
            "$or": [
                {"status": "pending", "expires_at": {"$lte": bson::DateTime::now()}},
                {"status": "failed"},
                {"status": "claimed", "checked_at": {"$lt": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(5))}},
            ],
        })
        .session(&mut *session)
        .await?
        .is_some();
    let jobs = db
        .collection::<bson::Document>(machine_job::COLLECTION_NAME)
        .count_documents(doc! {"user_id":owner,"issuing_turn_id":{"$in":&ids},"state":"running"})
        .session(&mut *session)
        .await?;
    let acknowledgements = db.collection::<bson::Document>(cards::COLLECTION_NAME);
    let outbox = acknowledgements
        .count_documents(
            doc! {"user_id":owner,"invocation_id":&row.id,"skill_notification_pending":true},
        )
        .session(&mut *session)
        .await?;
    let unused_approval = acknowledgements
        .find_one(doc! {
            "user_id": owner, "invocation_id": &row.id, "status": "allowed",
            "kind": {"$in": ["action", "skills"]}, "decided_by": {"$ne": "access_mode"},
            "expires_at": {"$gt": bson::DateTime::now()}
        })
        .projection(doc! {"_id":1})
        .session(&mut *session)
        .await?
        .is_some();
    let notification_failed = acknowledgements
        .find_one(doc! {
            "user_id": owner,
            "invocation_id": &row.id,
            "skill_notification_terminal.reason": {"$exists": true, "$nin": ["delivered", "invocation_stopped"]}
        })
        .projection(doc! {"_id": 1})
        .session(&mut *session)
        .await?
        .is_some();
    let mut cursor = acknowledgements
        .find(doc! {"user_id":owner,"invocation_id":&row.id,"authored_skill":{"$ne":null}})
        .projection(doc! {"_id":1})
        .limit(1025)
        .session(&mut *session)
        .await?;
    let publication_cards: Vec<bson::Document> = cursor.stream(&mut *session).try_collect().await?;
    let publication_ids: Vec<_> = publication_cards
        .iter()
        .filter_map(|c| c.get_str("_id").ok())
        .collect();
    let proposals = db.collection::<bson::Document>(
        crate::models::assistant_agent_learning::PROPOSALS_COLLECTION_NAME,
    );
    let publication_lineage = vec![
        doc! {"publication.invocation_id": &row.id},
        doc! {"publication.acknowledgement_id": {"$in": &publication_ids}},
    ];
    let active_publication_states = if row.cancel_requested {
        vec!["publishing"]
    } else {
        vec!["pending", "publishing", "published_unpinned"]
    };
    let publications = proposals
        .count_documents(doc! {
            "owner_id": owner,
            "$or": &publication_lineage,
            "status": {"$in": active_publication_states},
        })
        .session(&mut *session)
        .await?;
    let publication_failed = proposals
        .find_one(doc! {
            "owner_id": owner,
            "$or": &publication_lineage,
            "status": "publication_failed",
        })
        .projection(doc! {"_id": 1})
        .session(&mut *session)
        .await?
        .is_some();
    let publication_abandoned = proposals
        .find_one(doc! {
            "owner_id": owner,
            "$and": [
                {"$or": &publication_lineage},
                {"$or": [
                    {"status": "publishing", "$or": [
                        {"publication.lease_expires_at": null},
                        {"publication.lease_expires_at": {"$lte": bson::DateTime::now()}},
                    ]},
                    {"status": "publication_failed", "publication.verified_at": null, "$or": [
                        {"publication.started": true},
                        {"publication.uncertain_dispatch": true},
                    ]},
                ]},
            ],
        })
        .projection(doc! {"_id":1})
        .session(&mut *session)
        .await?
        .is_some();
    let expired = acknowledgements
        .find_one(doc! {
            "user_id": owner,
            "invocation_id": &row.id,
            "decider": "user",
            "$or": [
                {"status": "expired"},
                {"status": "pending", "expires_at": {"$lte": bson::DateTime::now()}},
            ],
        })
        .session(&mut *session)
        .await?
        .is_some();
    let orchestrator_decisions = acknowledgements
        .count_documents(doc! {
            "user_id": owner, "invocation_id": &row.id,
            "status": "pending", "decider": "orchestrator",
        })
        .session(&mut *session)
        .await?;
    let orchestrator_decision_expired = acknowledgements
        .find_one(doc! {
            "user_id": owner, "invocation_id": &row.id,
            "decider": "orchestrator",
            "$or": [
                {"status": "expired"},
                {"status": "pending", "expires_at": {"$lte": bson::DateTime::now()}},
            ],
        })
        .projection(doc! {"_id": 1})
        .session(&mut *session)
        .await?
        .is_some();
    let mut cursor = db
        .collection::<cards::AssistantAcknowledgement>(cards::COLLECTION_NAME)
        .find(doc! {
            "user_id": owner, "invocation_id": &row.id,
            "status": "pending", "decider": "user",
            "expires_at": {"$gt": bson::DateTime::now()},
        })
        .sort(doc! {"created_at": 1})
        .limit(51)
        .session(&mut *session)
        .await?;
    let cards: Vec<_> = cursor.stream(&mut *session).try_collect().await?;
    let mut actions: Vec<_> = cards
        .into_iter()
        .map(|c| RequiredAction {
            id: c.id,
            conversation_id: c.conversation_id,
            kind: c.kind,
            expires_at: c.expires_at,
        })
        .collect();
    let mut cursor = db
        .collection::<watches::NyxbotWatch>(watches::WATCHES_COLLECTION_NAME)
        .find(doc! {
            "user_id": owner, "invocation_id": &row.id, "status": "pending",
            "kind": {"$in": ["channel_bot", "connect_link", "trigger_created", "machine_setup", "machine_control"]},
            "expires_at": {"$gt": bson::DateTime::now()},
        })
        .sort(doc! {"created_at": 1})
        .limit(51)
        .session(&mut *session)
        .await?;
    let setup_actions: Vec<_> = cursor.stream(&mut *session).try_collect().await?;
    actions.extend(setup_actions.into_iter().map(|w| RequiredAction {
        id: w.id,
        conversation_id: w.conversation_id,
        kind: w.kind,
        expires_at: w.expires_at,
    }));
    let mut cursor = db
        .collection::<AssistantMessage>(MESSAGES)
        .find(doc! {"user_id":owner,"turn_id":{"$in":&ids},"role":"assistant"})
        .sort(doc! {"created_at":-1,"_id":-1})
        .limit(17)
        .session(&mut *session)
        .await?;
    let messages: Vec<_> = cursor.stream(&mut *session).try_collect().await?;
    let machine_pending = row.cancel_requested
        && db
            .collection::<bson::Document>(TURNS)
            .find_one(doc! {
                "user_id": owner, "invocation_id": &row.id,
                "machine_cancel_complete": {"$ne": true},
            })
            .session(&mut *session)
            .await?
            .is_some();
    let background = async_count + watch_count + jobs + outbox + publications;
    let error = turns.iter().rev().find_map(|t| t.error_code.clone());
    let review_reason = if !row.unconfirmed_async_cancellations.is_empty() {
        Some("async_cancellation_unconfirmed")
    } else if publication_abandoned {
        Some("skill_publication_unconfirmed")
    } else if row.continuation_blocked || (row.turn_count >= MAX_TURNS && pending > 0) {
        Some("turn_limit")
    } else if expired {
        Some("owner_decision_expired")
    } else if orchestrator_decision_expired {
        Some("orchestrator_decision_expired")
    } else if orchestrator_decisions > 0 && running + background + pending + hooks_pending == 0 {
        Some("orchestrator_decision_unresolved")
    } else if failed_setup {
        Some("setup_requires_review")
    } else if paused {
        Some("event_limit")
    } else if hooks_lost {
        Some("settlement_hooks_lost")
    } else if unused_approval && running + background + pending + hooks_pending == 0 {
        Some("unused_owner_approval")
    } else if notification_failed {
        Some("skill_notification_requires_review")
    } else if publication_failed {
        Some("skill_publication_requires_review")
    } else if publication_cards.len() > 1024 {
        Some("publication_limit")
    } else {
        None
    };
    let status = if let Some(status) = row.terminal_status.as_deref() {
        status
    } else if row.cancel_requested {
        if !row.unconfirmed_async_cancellations.is_empty() || publication_abandoned {
            "requires_review"
        } else if running + background + pending > 0 || machine_pending {
            "cancelling"
        } else if row.conversation_deleted {
            "deleted"
        } else {
            "cancelled"
        }
    } else if !actions.is_empty() {
        "requires_action"
    } else if running > 0 {
        "running"
    } else if review_reason.is_some() {
        "requires_review"
    } else if background + pending + hooks_pending > 0 {
        "waiting"
    } else if error.is_some() {
        "failed"
    } else {
        "completed"
    };
    let terminal = matches!(status, "completed" | "failed" | "cancelled" | "deleted");
    if row.terminal_status.is_none() && terminal {
        // Producers and admissions also write this receipt. Their concurrent
        // transaction either precedes this snapshot or conflicts and retries.
        db.collection::<AssistantInvocation>(INVOCATIONS)
            .update_one(
                doc! {"_id":&row.id,"user_id":owner,"terminal_status":null},
                doc! {"$set":{"terminal_status":status},"$inc":{"generation":1_i64}},
            )
            .session(&mut *session)
            .await?;
    }
    Ok(InvocationSnapshot {
        invocation_id: row.idempotency_key,
        conversation_id: row.conversation_id,
        turn_id: row.turn_id,
        status: status.into(),
        terminal,
        created_at: row.created_at,
        cancel_requested: row.cancel_requested,
        turn_count: row.turn_count,
        running_turns: running,
        background_operations: background,
        pending_continuations: pending + hooks_pending,
        required_actions_truncated: actions.len() > 50,
        required_actions: actions.into_iter().take(50).collect(),
        replies_truncated: messages.len() > 16,
        replies: messages
            .into_iter()
            .take(16)
            .map(|m| {
                let (text, truncated) = bounded_text(&m.text, 4096);
                InvocationReply {
                    conversation_id: m.conversation_id,
                    turn_id: m.turn_id,
                    message_id: m.id,
                    status: m.status,
                    error_code: m.error_code,
                    text,
                    text_truncated: truncated,
                }
            })
            .collect(),
        error_code: if row.cancel_requested && terminal {
            Some("cancelled".into())
        } else {
            review_reason.map(str::to_owned).or(error)
        },
        retry_after_ms: 1000,
    })
}

/// Recover a crash after the durable stop fence. Only exact, idempotent machine
/// cancellation is retried; neither turn execution nor uncertain provider RPCs
/// are replayed. Claim a bounded batch across replicas with a retry deadline.
pub async fn reconcile_cancellations(state: &crate::AppState) -> AppResult<()> {
    let mut receipts = Vec::new();
    for _ in 0..8 {
        let now = Utc::now();
        let receipt = state.db.collection::<AssistantInvocation>(INVOCATIONS)
            .find_one_and_update(
                doc! {"cancel_requested":true,"terminal_status":null,"$or":[
                    {"cancel_retry_at":null},
                    {"cancel_retry_at":{"$lte":bson::DateTime::from_chrono(now)}}
                ]},
                doc! {"$set":{"cancel_retry_at":bson::DateTime::from_chrono(now+chrono::Duration::seconds(60))}},
            )
            .sort(doc! {"cancel_retry_at":1})
            .return_document(mongodb::options::ReturnDocument::After)
            .await?;
        let Some(receipt) = receipt else { break };
        receipts.push(receipt);
    }
    use futures::StreamExt;
    futures::stream::iter(receipts)
        .for_each_concurrent(4, |receipt| async move {
            if cancel(state, &receipt.user_id, &receipt.idempotency_key)
                .await
                .is_ok()
            {
                // Persist settlement even when no polling client is connected.
                let _ = snapshot(&state.db, &receipt.user_id, &receipt.idempotency_key).await;
            }
        })
        .await;
    Ok(())
}
