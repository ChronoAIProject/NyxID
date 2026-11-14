//! Lifecycle notifications/projections and durable metadata-only investigations.
//! Optional notification delivery remains at-least-once after an accepted send
//! without a durable completion. A fresh exact claim and bounded send prevent a
//! live request from outlasting its claim; provider idempotency is not assumed.
use crate::{
    errors::{AppError, AppResult},
    models::{
        remote_machine_event::{
            COLLECTION_NAME as EVENTS, MachineLifecycleEvent, SafeLifecycleEvent,
        },
        remote_machine_operation::{
            COLLECTION_NAME as OPERATIONS, LifecycleAction, MachineOperation,
        },
    },
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, Document, doc},
    options::IndexOptions,
};

const NOTICE_CLAIM_LIFETIME: std::time::Duration = std::time::Duration::from_secs(120);
const NOTICE_DISPATCH_BOUND: std::time::Duration =
    std::time::Duration::from_secs(NOTICE_CLAIM_LIFETIME.as_secs() / 2);

pub const ORPHAN_INVENTORY: &str = "machine_orphan_inventory";

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<Document>(EVENTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"operation_id":1,"kind":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<Document>(EVENTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"owner_id":1,"machine_id":1,"created_at":-1,"_id":-1})
                .build(),
        )
        .await?;
    db.collection::<Document>(ORPHAN_INVENTORY)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"namespace":1,"kind":1,"identity_hash":1})
                .options(
                    IndexOptions::builder()
                        .name("controlled_resource_identity_v2".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! {"identity_hash":{"$type":"string"}})
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(ORPHAN_INVENTORY)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"state":1,"operator_escalated_at":1,"created_at":1,"_id":1})
                .build(),
        )
        .await?;
    // Install the new identity fence before retiring the Sandbox-only key.
    // Pod and PVC names occupy separate Kubernetes resource namespaces.
    let old = db
        .collection::<Document>(ORPHAN_INVENTORY)
        .list_indexes()
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    for index in old {
        if index.keys == doc! {"namespace":1,"name":1,"sandbox_uid":1}
            && let Some(name) = index.options.and_then(|options| options.name)
            && let Err(error) = db
                .collection::<Document>(ORPHAN_INVENTORY)
                .drop_index(name)
                .await
            && !matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == 27)
        {
            return Err(error);
        }
    }
    db.collection::<Document>(ORPHAN_INVENTORY)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"state":1,"created_at":1,"_id":1})
                .build(),
        )
        .await?;
    Ok(())
}

/// Safe correlation metadata only, never Pod specifications, annotations,
/// credentials, command output or provider assertions of physical cleanup.
pub struct InvestigationIdentity {
    pub kind: &'static str,
    pub namespace: String,
    pub name: Option<String>,
    pub resource_uid: Option<String>,
    pub sandbox_uid: Option<String>,
    pub machine_id: Option<String>,
    pub owner_id: Option<String>,
    pub operation_id: Option<String>,
    pub identity_hash: String,
}

pub async fn record_investigation(
    db: &Database,
    resource: InvestigationIdentity,
    reason: &'static str,
) -> AppResult<()> {
    let safe_name = |value: Option<String>| {
        value.filter(|value| {
            !value.is_empty()
                && value.len() <= 253
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.'))
        })
    };
    let safe_uuid = |value: Option<String>| {
        value.filter(|value| value.len() == 36 && uuid::Uuid::parse_str(value).is_ok())
    };
    let now = bson::DateTime::now();
    let result = db.collection::<Document>(ORPHAN_INVENTORY).update_one(
        doc! {"namespace":&resource.namespace,"kind":resource.kind,"identity_hash":&resource.identity_hash},
        doc! {"$set":{"last_seen_at":now,"state":"investigation_required","reason":reason,
            "name":safe_name(resource.name),"resource_uid":safe_uuid(resource.resource_uid),
            "sandbox_uid":safe_uuid(resource.sandbox_uid),"machine_id":safe_uuid(resource.machine_id),
            "owner_id":safe_uuid(resource.owner_id),"operation_id":safe_uuid(resource.operation_id)},
            "$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"created_at":now,
                "operator_deadline_at":bson::DateTime::from_chrono(Utc::now()+Duration::hours(24))}}
    ).upsert(true).await?;
    if result.upserted_id.is_some() {
        tracing::warn!(
            resource_kind = resource.kind,
            reason,
            "Remote Machine resource requires operator investigation; inspect admin remote-machine platform diagnostics; no cleanup inferred"
        );
    }
    Ok(())
}

pub struct InvestigationPage {
    pub rows: Vec<Document>,
    pub next_cursor: Option<String>,
}

/// Seek by immutable first-seen time and UUID, not mutable last-seen order.
/// Repeated sweeps cannot push an unresolved finding behind a page boundary.
pub async fn list_investigations(
    db: &Database,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> AppResult<InvestigationPage> {
    let mut filter = doc! {"state":"investigation_required"};
    if let Some(cursor) = cursor {
        let bad = || AppError::BadRequest("machine_inventory_cursor_invalid".into());
        if cursor.len() > 80 {
            return Err(bad());
        }
        let (at, id) = cursor.split_once('|').ok_or_else(bad)?;
        let at = DateTime::parse_from_rfc3339(at)
            .map_err(|_| bad())?
            .with_timezone(&Utc);
        let id = uuid::Uuid::parse_str(id).map_err(|_| bad())?.to_string();
        filter.insert(
            "$or",
            vec![
                doc! {"created_at":{"$gt":bson::DateTime::from_chrono(at)}},
                doc! {"created_at":bson::DateTime::from_chrono(at),"_id":{"$gt":id}},
            ],
        );
    }
    let limit = i64::from(limit.unwrap_or(25).clamp(1, 100));
    let mut rows = db.collection::<Document>(ORPHAN_INVENTORY).find(filter)
        .projection(doc! {"_id":1,"kind":1,"namespace":1,"name":1,"resource_uid":1,"sandbox_uid":1,
            "machine_id":1,"owner_id":1,"operation_id":1,"reason":1,"state":1,"created_at":1,"last_seen_at":1,"operator_deadline_at":1})
        .sort(doc! {"created_at":1,"_id":1}).limit(limit+1)
        .max_time(std::time::Duration::from_secs(3)).await?.try_collect::<Vec<_>>().await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if more {
        rows.last().and_then(|row| {
            Some(format!(
                "{}|{}",
                row.get_datetime("created_at")
                    .ok()?
                    .to_chrono()
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                row.get_str("_id").ok()?
            ))
        })
    } else {
        None
    };
    Ok(InvestigationPage { rows, next_cursor })
}

pub async fn record(
    db: &Database,
    operation: &MachineOperation,
    kind: &str,
    message: &str,
    notification: bool,
) -> AppResult<()> {
    let event = MachineLifecycleEvent {
        id: uuid::Uuid::new_v4().to_string(),
        owner_id: operation.owner_id.clone(),
        machine_id: operation.machine_id.clone(),
        operation_id: operation.id.clone(),
        kind: kind.into(),
        message: message.into(),
        notification,
        responsibility: "platform".into(),
        created_at: Utc::now(),
        expires_at: operation.terminal_at.map(|at| at + Duration::days(90)),
    };
    db.collection::<Document>(EVENTS).update_one(doc! {"operation_id":&operation.id,"kind":kind}, doc! {"$setOnInsert":bson::to_document(&event).map_err(|_|AppError::Internal("timeline encoding failed".into()))?}).upsert(true).await?;
    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct EventPage {
    pub events: Vec<SafeLifecycleEvent>,
    pub next_cursor: Option<String>,
}

#[cfg(test)]
pub async fn list(
    db: &Database,
    owner_id: &str,
    machine_id: &str,
    before: Option<DateTime<Utc>>,
) -> AppResult<Vec<SafeLifecycleEvent>> {
    Ok(list_page(db, owner_id, machine_id, before, None, None)
        .await?
        .events)
}

/// Owner and Machine filters precede the bounded seek. The UUID tie-breaker
/// prevents equal-millisecond lifecycle events disappearing between pages.
pub async fn list_page(
    db: &Database,
    owner_id: &str,
    machine_id: &str,
    before: Option<DateTime<Utc>>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> AppResult<EventPage> {
    let mut filter = doc! {"owner_id":owner_id,"machine_id":machine_id,"$or":[{"expires_at":bson::Bson::Null},{"expires_at":{"$gt":bson::DateTime::now()}}]};
    if before.is_some() && cursor.is_some() {
        return Err(AppError::BadRequest(
            "machine_event_cursor_conflicts_with_before".into(),
        ));
    }
    if let Some(before) = before {
        filter.insert(
            "created_at",
            doc! {"$lt":bson::DateTime::from_chrono(before)},
        );
    }
    if let Some(cursor) = cursor {
        let bad = || AppError::BadRequest("machine_event_cursor_invalid".into());
        if cursor.len() > 80 {
            return Err(bad());
        }
        let (at, id) = cursor.split_once('|').ok_or_else(bad)?;
        let at = DateTime::parse_from_rfc3339(at)
            .map_err(|_| bad())?
            .with_timezone(&Utc);
        let id = uuid::Uuid::parse_str(id).map_err(|_| bad())?.to_string();
        filter.insert(
            "$and",
            vec![doc! {"$or":[
                {"created_at":{"$lt":bson::DateTime::from_chrono(at)}},
                {"created_at":bson::DateTime::from_chrono(at),"_id":{"$lt":id}},
            ]}],
        );
    }
    let limit = i64::from(limit.unwrap_or(100).clamp(1, 100));
    let mut rows: Vec<MachineLifecycleEvent> = db
        .collection(EVENTS)
        .find(filter)
        .sort(doc! {"created_at":-1,"_id":-1})
        .limit(limit + 1)
        .max_time(std::time::Duration::from_secs(3))
        .await?
        .try_collect()
        .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = more.then(|| rows.last()).flatten().map(|row| {
        format!(
            "{}|{}",
            row.created_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            row.id
        )
    });
    let events = rows
        .into_iter()
        .map(|r| SafeLifecycleEvent {
            id: r.id,
            machine_id: r.machine_id,
            operation_id: r.operation_id,
            kind: r.kind,
            message: r.message,
            notification: r.notification,
            responsibility: r.responsibility,
            created_at: r.created_at,
        })
        .collect();
    Ok(EventPage {
        events,
        next_cursor,
    })
}

pub async fn sweep(db: &Database, now: DateTime<Utc>) -> AppResult<()> {
    // Durable operator escalation is separate from disposable owner timelines.
    // Absence in a later inventory never resolves these resource investigations.
    for _ in 0..128 {
        let row = db
            .collection::<Document>(ORPHAN_INVENTORY)
            .find_one_and_update(
                doc! {"state":"investigation_required","operator_escalated_at":bson::Bson::Null,
                "created_at":{"$lte":bson::DateTime::from_chrono(now-Duration::hours(24))}},
                doc! {"$set":{"operator_escalated_at":bson::DateTime::from_chrono(now)}},
            )
            .sort(doc! {"created_at":1,"_id":1})
            .await?;
        let Some(row) = row else { break };
        tracing::error!(
            investigation_id = row.get_str("_id").unwrap_or("unknown"),
            "Remote Machine resource investigation exceeded 24 hours; operator action required in admin remote-machine platform diagnostics"
        );
    }
    let ops: Vec<MachineOperation>=db.collection(OPERATIONS).find(doc! {"$or":[{"terminal_at":bson::Bson::Null},{"timeline_retention_finalized":{"$ne":true}}]}).sort(doc! {"timeline_projected_at":1,"_id":1}).limit(128).await?.try_collect().await?;
    for op in ops {
        // Transient controller progress may replace error_code. The durable
        // lifecycle stop reason still carries the owner's actionable cause.
        let stop_reason = if op.action == LifecycleAction::Pause {
            db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
                .find_one(doc! {"_id":&op.machine_id,"active_operation_id":&op.id})
                .projection(doc! {"stop_reason":1})
                .await?
                .and_then(|row| row.get_str("stop_reason").ok().map(str::to_owned))
        } else {
            None
        };
        // Delete has the same evidence contract for every account-safety reason
        // and after transient progress replaces that reason. Never promise
        // retained workspaces or unobserved physical storage reclamation here.
        let accepted_message = if op.action == LifecycleAction::Delete {
            "Machine access stopped and runtime and workspace deletion was queued. Cleanup remains pending until applicable control-plane stop or absence, Sandbox object-cascade checks, creation obligations, and accounting are verified. Underlying storage follows its reclaim policy; physical storage release is not verified."
        } else {
            match stop_reason.as_deref().or(op.error_code.as_deref()) {
                Some("runtime_funding_unavailable" | "runtime_funding_expired") => {
                    "Runtime funding stopped. Your workspace is retained. Restore funds or billing availability, then review a new quote and resume manually."
                }
                Some("expansion_deadline_exceeded") => {
                    "Provisioning exceeded its deadline and was stopped. Your existing workspace is retained; check platform status before retrying."
                }
                Some(
                    "account_deletion_pending" | "owner_not_found" | "owner_deletion_basis_missing",
                ) => {
                    "Machine access stopped because the account is unavailable or owner deletion consent is missing. Your workspace is retained; platform investigation is required before further action."
                }
                Some("account_inactive" | "machine_security_restriction") => {
                    "Machine access stopped because the account is unavailable or restricted. Your workspace is retained; resolve the account status before resuming."
                }
                _ => "Lifecycle operation accepted",
            }
        };
        record(db, &op, "accepted", accepted_message, true).await?;
        if let Some(terminal) = op.terminal_at {
            record(
                db,
                &op,
                "completed",
                "Lifecycle operation reached its terminal result",
                true,
            )
            .await?;
            db.collection::<Document>(EVENTS).update_many(doc! {"operation_id":&op.id,"expires_at":bson::Bson::Null},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(terminal+Duration::days(90))}}).await?;
        } else if op.action == LifecycleAction::Delete && now >= op.created_at + Duration::hours(24)
        {
            record(db,&op,"cleanup_overdue","Cleanup remains pending beyond the 24-hour target; platform investigation is required",true).await?;
            escalate_cleanup(db, &op).await?;
            db.collection::<Document>(OPERATIONS).update_one(doc! {"_id":&op.id,"terminal_at":bson::Bson::Null},doc! {"$set":{"deadline_at":bson::DateTime::from_chrono(op.created_at+Duration::hours(24)),"cleanup_state":"pending"}}).await?;
        } else if now >= op.created_at + Duration::minutes(15) {
            let message = if op.action == LifecycleAction::Delete {
                "Machine deletion reconciliation is taking longer than expected. Cleanup remains pending; platform investigation may be required."
            } else {
                "Platform reconciliation is taking longer than expected; retained work data will not be deleted automatically"
            };
            record(db, &op, "reconciliation_delayed", message, true).await?;
        }
        db.collection::<Document>(OPERATIONS)
            .update_one(
                doc! {"_id":&op.id},
                doc! {"$set":{"timeline_projected_at":bson::DateTime::from_chrono(now),"timeline_retention_finalized":op.terminal_at.is_some()}},
            )
            .await?;
    }
    // Only the disposable projection is pruned. Paused volumes, nonterminal
    // operations, idempotency, cleanup/billing evidence and audit chains survive.
    db.collection::<Document>(EVENTS)
        .delete_many(doc! {"expires_at":{"$lte":bson::DateTime::from_chrono(now)}})
        .await?;
    Ok(())
}

/// An escalation is durable in the existing operator audit surface. The marker
/// and chained audit commit together, so a crash neither loses nor duplicates it.
async fn escalate_cleanup(db: &Database, op: &MachineOperation) -> AppResult<()> {
    let db = db.clone();
    let op = op.clone();
    db.client().start_session().await?.start_transaction().write_concern(mongodb::options::WriteConcern::majority()).and_run2(async move |session| {
        let result: AppResult<()> = async {
            let updated=db.collection::<Document>(OPERATIONS).update_one(doc! {"_id":&op.id,"terminal_at":bson::Bson::Null,"operator_escalated_at":bson::Bson::Null},doc! {"$set":{"operator_escalated_at":bson::DateTime::now(),"operator_escalation":"cleanup_overdue"}}).session(&mut *session).await?;
            if updated.modified_count==0 {return Ok(());}
            super::remote_machine_service::append_owner_audit(&db,session,&op.owner_id,"machine_cleanup_overdue_operator_action",op.machine_id.as_deref().unwrap_or_default(),&op.id).await?;
            tracing::error!(operation_id=%op.id, machine_id=?op.machine_id, "Remote Machine cleanup exceeded 24 hours; inspect operator audit and retained cleanup liability");
            Ok(())
        }.await;super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::api_key_mutation_service::map_transaction_error)
}

/// Bounded optional channel delivery. No channel leaves the in-site notice
/// visible and does not block lifecycle reconciliation or physical cleanup.
pub async fn deliver_pending(state: &crate::AppState) -> AppResult<()> {
    for _ in 0..16 {
        let Some(_work) = state.shutdown.background() else {
            break;
        };
        let claim = uuid::Uuid::new_v4().to_string();
        let event=state.db.collection::<Document>(EVENTS).find_one_and_update(doc! {
            "notification":true,"notification_delivered_at":bson::Bson::Null,
            "$expr":{"$and":[
                {"$lte":[{"$ifNull":["$notification_retry_at","$$NOW"]},"$$NOW"]},
                {"$lte":[{"$ifNull":["$notification_claim_until","$$NOW"]},"$$NOW"]}
            ]}
        },vec![doc! {"$set":{
            "notification_claim":&claim,
            "notification_claim_until":{"$add":["$$NOW",NOTICE_CLAIM_LIFETIME.as_millis() as i64]},
            "notification_attempts":{"$add":[{"$ifNull":["$notification_attempts",0_i64]},1_i64]}
        }}]).sort(doc! {"created_at":1}).return_document(mongodb::options::ReturnDocument::After).await?;
        let Some(document) = event else { break };
        let event: MachineLifecycleEvent = bson::from_document(document.clone())
            .map_err(|_| AppError::Internal("lifecycle event invalid".into()))?;
        let release = state.shutdown.register_release(super::shutdown::Release {
            safe_after_stop: false,
            db: state.db.clone(),
            collection: EVENTS,
            filter: doc! {"_id": &event.id, "notification_claim": &claim},
            update: doc! {"$set": {"notification_claim_until": null, "notification_claim": null}},
        });
        #[cfg(test)]
        notice_tests::age_claim(&state.db, &event.id).await;
        let result = dispatch_lifecycle_notice(state, &event, &claim).await;
        let attempts = document.get_i64("notification_attempts").unwrap_or(1);
        let mut updates = doc! {"notification_claim":bson::Bson::Null,"notification_claim_until":bson::Bson::Null};
        if result.as_ref().is_ok_and(|r| !r.channels.is_empty()) {
            updates.insert("notification_delivered_at", "$$NOW");
        } else {
            // Optional delivery is bounded; the owner still has the in-site event.
            let retry = if attempts >= 8 {
                Duration::hours(24)
            } else {
                Duration::seconds(
                    super::remote_machine_operation_service::retry_backoff_seconds(attempts),
                )
            };
            updates.insert(
                "notification_retry_at",
                doc! {"$add":["$$NOW",retry.num_milliseconds()]},
            );
        }
        state
            .db
            .collection::<Document>(EVENTS)
            .update_one(
                doc! {"_id":&event.id,"notification_claim":&claim},
                vec![doc! {"$set":updates}],
            )
            .await?;
        release.completed();
    }
    Ok(())
}

async fn dispatch_lifecycle_notice(
    state: &crate::AppState,
    event: &MachineLifecycleEvent,
    claim: &str,
) -> AppResult<super::notification_service::NotificationResult> {
    // Include the final claim check and all channel preparation in the budget.
    // A slow claim-check response cannot grant a fresh local timeout afterward.
    let deadline = tokio::time::Instant::now() + NOTICE_DISPATCH_BOUND;
    tokio::time::timeout_at(deadline, async {
        let renewed = state
            .db
            .collection::<Document>(EVENTS)
            .update_one(
                doc! {"_id": &event.id, "notification_claim": claim,
                "notification_delivered_at": bson::Bson::Null,
                "$expr": {"$gt": ["$notification_claim_until", "$$NOW"]}},
                vec![doc! {"$set": {"notification_claim_until": {"$add": ["$$NOW", NOTICE_CLAIM_LIFETIME.as_millis() as i64]}}}],
            )
            .await?;
        if renewed.matched_count != 1 || tokio::time::Instant::now() >= deadline {
            return Err(AppError::Conflict("Lifecycle notice claim changed".into()));
        }
        send_lifecycle_notice(state, event).await
    })
    .await
    .map_err(|_| AppError::ChannelPlatformError("Lifecycle notice delivery timed out".into()))?
}

async fn send_lifecycle_notice(
    state: &crate::AppState,
    event: &MachineLifecycleEvent,
) -> AppResult<super::notification_service::NotificationResult> {
    #[cfg(test)]
    if let Ok(gate) = notice_tests::SEND.try_with(Clone::clone) {
        if gate.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            gate.started.notify_one();
            gate.resume.notified().await;
        }
        return Ok(super::notification_service::NotificationResult {
            channels: if gate.delivered {
                vec!["test".into()]
            } else {
                Vec::new()
            },
            telegram_chat_id: None,
            telegram_message_id: None,
        });
    }
    super::notification_service::send_machine_lifecycle_notification(
        &state.db,
        &state.config,
        &state.http_client,
        state.fcm_auth.as_deref(),
        state.apns_auth.as_deref(),
        &event.owner_id,
        event.machine_id.as_deref().unwrap_or_default(),
        &event.operation_id,
        &event.message,
    )
    .await
}

#[cfg(test)]
mod pagination_tests {
    use super::*;

    async fn account_safety_operation(
        db: &Database,
        action: LifecycleAction,
        reason: &str,
    ) -> MachineOperation {
        let now = Utc::now();
        let op = MachineOperation {
            id: uuid::Uuid::new_v4().to_string(),
            owner_id: uuid::Uuid::new_v4().to_string(),
            machine_id: Some(uuid::Uuid::new_v4().to_string()),
            action,
            idempotency_key_hash: "system:account-safety-fixture".into(),
            request_digest: "account-safety-fixture".into(),
            expected_revision: Some(1),
            desired_runtime_generation: Some(1),
            state: crate::models::remote_machine_operation::OperationState::Accepted,
            cleanup_state: crate::models::remote_machine_operation::CleanupState::Pending,
            effect_id: None,
            error_code: Some(if action == LifecycleAction::Pause {
                // Manager progress may replace this field; the durable stop
                // reason must still drive the pause disclosure.
                "runtime_evidence_pending".into()
            } else {
                reason.into()
            }),
            error_message: None,
            evidence_digest: None,
            retry_count: 0,
            delete_retry: None,
            next_attempt_at: Some(now),
            deadline_at: Some(now + Duration::hours(24)),
            created_at: now,
            updated_at: now,
            terminal_at: None,
        };
        db.collection::<MachineOperation>(OPERATIONS)
            .insert_one(&op)
            .await
            .unwrap();
        if action == LifecycleAction::Pause {
            db.collection::<Document>(crate::models::remote_machine::COLLECTION_NAME)
                .insert_one(doc! {"_id":op.machine_id.as_deref().unwrap(),"active_operation_id":&op.id,"stop_reason":reason})
                .await
                .unwrap();
        }
        op
    }

    async fn overdue_delete_operation(
        db: &Database,
        error: Option<(&str, &str)>,
    ) -> MachineOperation {
        let op = account_safety_operation(db, LifecycleAction::Delete, "fixture").await;
        let (code, message) = error.unzip();
        db.collection::<Document>(OPERATIONS)
            .update_one(
                doc! {"_id":&op.id},
                doc! {"$set":{
                    "created_at":bson::DateTime::from_chrono(Utc::now()-Duration::hours(25)),
                    "deadline_at":bson::Bson::Null,
                    "error_code":code,
                    "error_message":message
                }},
            )
            .await
            .unwrap();
        db.collection::<MachineOperation>(OPERATIONS)
            .find_one(doc! {"_id":&op.id})
            .await
            .unwrap()
            .unwrap()
    }

    async fn assert_overdue_projection(
        db: &Database,
        original: &MachineOperation,
        error: Option<(&str, &str)>,
    ) {
        let op = db
            .collection::<MachineOperation>(OPERATIONS)
            .find_one(doc! {"_id":&original.id})
            .await
            .unwrap()
            .unwrap();
        let (code, message) = error.unzip();
        assert_eq!(op.error_code.as_deref(), code);
        assert_eq!(op.error_message.as_deref(), message);
        assert_eq!(op.state, original.state);
        assert!(op.terminal_at.is_none());
        assert_eq!(
            op.cleanup_state,
            crate::models::remote_machine_operation::CleanupState::Pending
        );
        assert_eq!(op.deadline_at, Some(op.created_at + Duration::hours(24)));
        assert_eq!(op.retry_count, original.retry_count);
        assert_eq!(
            db.collection::<Document>(EVENTS)
                .count_documents(
                    doc! {"operation_id":&op.id,"kind":"cleanup_overdue","notification":true}
                )
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type":"machine_cleanup_overdue_operator_action","event_data.operation_id":&op.id})
                .await
                .unwrap(),
            1
        );
        let row = db
            .collection::<Document>(OPERATIONS)
            .find_one(doc! {"_id":&op.id})
            .await
            .unwrap()
            .unwrap();
        assert!(row.get_datetime("operator_escalated_at").is_ok());
        assert_eq!(row.get_str("operator_escalation"), Ok("cleanup_overdue"));
    }

    #[tokio::test]
    async fn cleanup_overdue_preserves_specific_error_before_sweep() {
        let db =
            crate::test_utils::connect_transaction_test_database("machine_overdue_specific").await;
        ensure_indexes(&db).await.unwrap();
        let error = Some((
            "delete_kubernetes_access_denied",
            "Kubernetes access denied while observing cleanup resources",
        ));
        let op = overdue_delete_operation(&db, error).await;
        for _ in 0..2 {
            sweep(&db, Utc::now()).await.unwrap();
            assert_overdue_projection(&db, &op, error).await;
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn cleanup_overdue_preserves_specific_error_after_earlier_sweep() {
        let db =
            crate::test_utils::connect_transaction_test_database("machine_overdue_order").await;
        ensure_indexes(&db).await.unwrap();
        // A pre-upgrade overdue code is replaced only by a real observation;
        // the projection does not infer what the previous failure meant.
        let legacy = Some((
            "cleanup_overdue",
            "Cleanup remains pending; no completion is inferred from the deadline",
        ));
        let op = overdue_delete_operation(&db, legacy).await;
        sweep(&db, Utc::now()).await.unwrap();
        assert_overdue_projection(&db, &op, legacy).await;
        let (code, message) = (
            "delete_kubernetes_response_invalid",
            "Kubernetes cleanup observation response is invalid",
        );
        let error = Some((code, message));
        db.collection::<Document>(OPERATIONS)
            .update_one(
                doc! {"_id":&op.id,"terminal_at":bson::Bson::Null},
                doc! {"$set":{"error_code":code,"error_message":message}},
            )
            .await
            .unwrap();
        for _ in 0..2 {
            sweep(&db, Utc::now()).await.unwrap();
            assert_overdue_projection(&db, &op, error).await;
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn cleanup_overdue_does_not_invent_an_error_when_none_is_known() {
        let db =
            crate::test_utils::connect_transaction_test_database("machine_overdue_no_error").await;
        ensure_indexes(&db).await.unwrap();
        let op = overdue_delete_operation(&db, None).await;
        for _ in 0..2 {
            sweep(&db, Utc::now()).await.unwrap();
            assert_overdue_projection(&db, &op, None).await;
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn cleanup_overdue_does_not_reopen_a_terminal_delete() {
        let db =
            crate::test_utils::connect_transaction_test_database("machine_overdue_terminal").await;
        ensure_indexes(&db).await.unwrap();
        let op = overdue_delete_operation(&db, None).await;
        let terminal = bson::DateTime::now();
        db.collection::<Document>(OPERATIONS)
            .update_one(
                doc! {"_id":&op.id},
                doc! {"$set":{"state":"succeeded","cleanup_state":"complete","terminal_at":terminal}},
            )
            .await
            .unwrap();
        for _ in 0..2 {
            sweep(&db, Utc::now()).await.unwrap();
            let retained = db
                .collection::<MachineOperation>(OPERATIONS)
                .find_one(doc! {"_id":&op.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                retained.state,
                crate::models::remote_machine_operation::OperationState::Succeeded
            );
            assert_eq!(
                retained.cleanup_state,
                crate::models::remote_machine_operation::CleanupState::Complete
            );
            assert_eq!(retained.terminal_at, Some(terminal.to_chrono()));
            assert_eq!(retained.deadline_at, None);
            assert_eq!(retained.error_code, None);
            assert_eq!(retained.error_message, None);
            assert_eq!(retained.retry_count, op.retry_count);
            assert_eq!(
                db.collection::<Document>(EVENTS)
                    .count_documents(doc! {"operation_id":&op.id,"kind":"cleanup_overdue"})
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                    .count_documents(doc! {"event_type":"machine_cleanup_overdue_operator_action","event_data.operation_id":&op.id})
                    .await
                    .unwrap(),
                0
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn account_safety_pause_projection_discloses_retention_and_replays_once() {
        let db =
            crate::test_utils::connect_transaction_test_database("machine_pause_disclosure").await;
        ensure_indexes(&db).await.unwrap();
        for reason in [
            "account_deletion_pending",
            "owner_not_found",
            "owner_deletion_basis_missing",
        ] {
            let op = account_safety_operation(&db, LifecycleAction::Pause, reason).await;
            for _ in 0..2 {
                sweep(&db, Utc::now()).await.unwrap();
                let event = db
                    .collection::<MachineLifecycleEvent>(EVENTS)
                    .find_one(doc! {"operation_id":&op.id,"kind":"accepted"})
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    event.message.contains("Your workspace is retained"),
                    "{reason}: {}",
                    event.message
                );
                assert!(
                    !event
                        .message
                        .contains("queued runtime and workspace deletion")
                );
                assert!(event.notification);
                assert_eq!(
                    db.collection::<Document>(EVENTS)
                        .count_documents(doc! {"operation_id":&op.id,"kind":"accepted"})
                        .await
                        .unwrap(),
                    1
                );
            }
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn owner_account_delete_projection_preserves_pending_cleanup_disclosure() {
        let db =
            crate::test_utils::connect_transaction_test_database("machine_delete_disclosure").await;
        ensure_indexes(&db).await.unwrap();
        // All owner_ineligibility reasons plus the retained-recovery reason.
        // Delete selection depends on deletion authority, not on the reason.
        // Include ordinary Delete, replaced progress and shared non-Delete
        // disclosures to ensure none can promise retention on a Delete.
        for reason in [
            "account_deletion_pending",
            "owner_not_found",
            "organization_not_eligible",
            "account_inactive",
            "machine_security_restriction",
            "owner_deletion_basis_missing",
            "delete_sandbox_gc_pending",
            "runtime_funding_unavailable",
            "runtime_funding_expired",
            "expansion_deadline_exceeded",
            "fixture",
            "no_error_code",
        ] {
            for phase in ["accepted", "delayed", "overdue", "succeeded", "failed"] {
                let mut op = account_safety_operation(&db, LifecycleAction::Delete, reason).await;
                if reason == "no_error_code" {
                    op.error_code = None;
                }
                let now = Utc::now();
                let age = match phase {
                    "delayed" => Duration::minutes(16),
                    "overdue" => Duration::hours(25),
                    _ => Duration::zero(),
                };
                let terminal = matches!(phase, "succeeded" | "failed");
                db.collection::<Document>(OPERATIONS).update_one(doc! {"_id":&op.id},
                    doc! {"$set":{
                        "created_at":bson::DateTime::from_chrono(now-age),
                        "error_code":&op.error_code,
                        "terminal_at":if terminal { bson::Bson::DateTime(bson::DateTime::from_chrono(now)) } else { bson::Bson::Null },
                        "state":if terminal { phase } else { "accepted" },
                        "cleanup_state":if phase == "succeeded" { "complete" } else { "pending" },
                    }}).await.unwrap();
                for _ in 0..2 {
                    sweep(&db, now).await.unwrap();
                    let events: Vec<MachineLifecycleEvent> = db
                        .collection(EVENTS)
                        .find(doc! {"operation_id":&op.id})
                        .await
                        .unwrap()
                        .try_collect()
                        .await
                        .unwrap();
                    let accepted = events.iter().find(|e| e.kind == "accepted").unwrap();
                    assert!(
                        accepted
                            .message
                            .contains("runtime and workspace deletion was queued"),
                        "{reason}/{phase}"
                    );
                    assert!(accepted.message.contains("applicable control-plane stop or absence, Sandbox object-cascade checks, creation obligations, and accounting"));
                    assert!(
                        accepted
                            .message
                            .contains("physical storage release is not verified")
                    );
                    assert!(
                        accepted
                            .message
                            .contains("Underlying storage follows its reclaim policy")
                    );
                    let expected_kind = match phase {
                        "delayed" => Some("reconciliation_delayed"),
                        "overdue" => Some("cleanup_overdue"),
                        "succeeded" | "failed" => Some("completed"),
                        _ => None,
                    };
                    assert_eq!(
                        events.len(),
                        if expected_kind.is_some() { 2 } else { 1 },
                        "{reason}/{phase}: replay must not duplicate notices"
                    );
                    if let Some(kind) = expected_kind {
                        assert!(events.iter().any(|e| e.kind == kind));
                    }
                    for event in events {
                        assert!(event.notification);
                        assert!(
                            !event.message.contains("physical stop"),
                            "{reason}/{phase}: {}",
                            event.message
                        );
                        assert!(!event.message.contains("storage reclamation"));
                        assert!(!event.message.contains("workspace is retained"));
                        assert!(!event.message.contains("will not be deleted automatically"));
                        if matches!(
                            event.kind.as_str(),
                            "cleanup_overdue" | "reconciliation_delayed"
                        ) {
                            assert!(event.message.contains("pending"));
                        }
                    }
                    let retained = db
                        .collection::<MachineOperation>(OPERATIONS)
                        .find_one(doc! {"_id":&op.id})
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(retained.terminal_at.is_some(), terminal);
                    assert_eq!(
                        retained.cleanup_state.as_str(),
                        if phase == "succeeded" {
                            "complete"
                        } else {
                            "pending"
                        }
                    );
                    assert_eq!(retained.error_code, op.error_code);
                }
            }
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn investigation_index_upgrade_retains_legacy_rows_and_separates_resource_kinds() {
        let db =
            crate::test_utils::connect_transaction_test_database("machine_inventory_index_upgrade")
                .await;
        let rows = db.collection::<Document>(ORPHAN_INVENTORY);
        rows.create_index(
            IndexModel::builder()
                .keys(doc! {"namespace":1,"name":1,"sandbox_uid":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .unwrap();
        let sandbox_uid = uuid::Uuid::new_v4().to_string();
        let legacy = uuid::Uuid::new_v4().to_string();
        rows.insert_one(doc! {"_id":&legacy,"namespace":"nyxid-machine","name":"same-name","sandbox_uid":&sandbox_uid,
            "state":"investigation_required","created_at":bson::DateTime::now(),"last_seen_at":bson::DateTime::now()}).await.unwrap();
        tokio::try_join!(ensure_indexes(&db), ensure_indexes(&db)).unwrap();
        for kind in ["Pod", "PersistentVolumeClaim"] {
            for _ in 0..2 {
                record_investigation(
                    &db,
                    InvestigationIdentity {
                        kind,
                        namespace: "nyxid-machine".into(),
                        name: Some("same-name".into()),
                        resource_uid: Some(sandbox_uid.clone()),
                        sandbox_uid: Some(sandbox_uid.clone()),
                        machine_id: None,
                        owner_id: None,
                        operation_id: None,
                        identity_hash: format!("{kind}-stable-fixture"),
                    },
                    "resource_machine_unmatched",
                )
                .await
                .unwrap();
            }
        }
        assert_eq!(rows.count_documents(doc! {}).await.unwrap(), 3);
        assert!(rows.find_one(doc! {"_id":legacy}).await.unwrap().is_some());
        let indexes = rows
            .list_indexes()
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert!(
            indexes
                .iter()
                .any(|index| index.keys == doc! {"namespace":1,"kind":1,"identity_hash":1})
        );
        assert!(
            !indexes
                .iter()
                .any(|index| index.keys == doc! {"namespace":1,"name":1,"sandbox_uid":1})
        );
        assert_eq!(
            list_investigations(&db, None, None)
                .await
                .unwrap()
                .rows
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn terminal_event_retention_recovers_after_more_than_ninety_days_offline() {
        let db = crate::test_utils::connect_transaction_test_database("machine_timeline_retention")
            .await;
        ensure_indexes(&db).await.unwrap();
        let now = Utc::now();
        let old = now - Duration::days(100);
        let op = MachineOperation {
            id: uuid::Uuid::new_v4().to_string(),
            owner_id: uuid::Uuid::new_v4().to_string(),
            machine_id: Some(uuid::Uuid::new_v4().to_string()),
            action: LifecycleAction::Create,
            idempotency_key_hash: "retained-hash".into(),
            request_digest: "retained-digest".into(),
            expected_revision: None,
            desired_runtime_generation: Some(1),
            state: crate::models::remote_machine_operation::OperationState::Succeeded,
            cleanup_state: crate::models::remote_machine_operation::CleanupState::NotRequired,
            effect_id: None,
            error_code: None,
            error_message: None,
            evidence_digest: None,
            retry_count: 0,
            delete_retry: None,
            next_attempt_at: None,
            deadline_at: None,
            created_at: old - Duration::minutes(1),
            updated_at: old,
            terminal_at: Some(old),
        };
        db.collection::<MachineOperation>(OPERATIONS)
            .insert_one(&op)
            .await
            .unwrap();
        let mut live = op.clone();
        live.terminal_at = None;
        record(&db, &live, "accepted", "Accepted", false)
            .await
            .unwrap();
        sweep(&db, now).await.unwrap();
        assert_eq!(
            db.collection::<Document>(EVENTS)
                .count_documents(doc! {"operation_id":&op.id})
                .await
                .unwrap(),
            0
        );
        let retained = db
            .collection::<Document>(OPERATIONS)
            .find_one(doc! {"_id":&op.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained.get_bool("timeline_retention_finalized"), Ok(true));
        assert_eq!(retained.get_str("request_digest"), Ok("retained-digest"));
        sweep(&db, now).await.unwrap();
        assert_eq!(
            db.collection::<Document>(EVENTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn lifecycle_cursor_keeps_timestamp_ties_and_filters_owner_before_limit() {
        let db = crate::test_utils::connect_transaction_test_database("machine_event_cursor").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let machine = uuid::Uuid::new_v4().to_string();
        let at = bson::DateTime::now().to_chrono();
        let mut expected = std::collections::BTreeSet::new();
        let mut rows = Vec::new();
        for _ in 0..105 {
            let id = uuid::Uuid::new_v4().to_string();
            expected.insert(id.clone());
            rows.push(MachineLifecycleEvent {
                id,
                owner_id: owner.clone(),
                machine_id: Some(machine.clone()),
                operation_id: uuid::Uuid::new_v4().to_string(),
                kind: "progress".into(),
                message: "Safe progress".into(),
                notification: false,
                responsibility: "platform".into(),
                created_at: at,
                expires_at: None,
            });
        }
        let mut foreign = rows[0].clone();
        foreign.id = uuid::Uuid::new_v4().to_string();
        foreign.owner_id = uuid::Uuid::new_v4().to_string();
        foreign.created_at += Duration::hours(1);
        rows.push(foreign);
        db.collection::<MachineLifecycleEvent>(EVENTS)
            .insert_many(rows)
            .await
            .unwrap();
        let first = list_page(&db, &owner, &machine, None, None, Some(100))
            .await
            .unwrap();
        assert_eq!(first.events.len(), 100);
        let second = list_page(
            &db,
            &owner,
            &machine,
            None,
            first.next_cursor.as_deref(),
            Some(100),
        )
        .await
        .unwrap();
        assert_eq!(second.events.len(), 5);
        assert!(second.next_cursor.is_none());
        let found = first
            .events
            .into_iter()
            .chain(second.events)
            .map(|row| row.id)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(found, expected);
        assert!(
            list_page(&db, &owner, &machine, None, Some("bad|cursor"), None)
                .await
                .is_err()
        );
        assert!(
            list_page(&db, &owner, &machine, Some(at), Some("bad|cursor"), None)
                .await
                .is_err()
        );
        db.drop().await.unwrap();
    }
}

#[cfg(test)]
mod notice_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[derive(Clone)]
    pub(super) struct SendGate {
        pub calls: Arc<AtomicUsize>,
        pub age_claim: bool,
        pub delivered: bool,
        pub started: Arc<tokio::sync::Notify>,
        pub resume: Arc<tokio::sync::Notify>,
    }
    tokio::task_local! {pub(super) static SEND: SendGate;}
    pub(super) async fn age_claim(db: &Database, id: &str) {
        if SEND.try_with(|gate| gate.age_claim).unwrap_or(false) {
            db.collection::<Document>(EVENTS)
                .update_one(
                    doc! {"_id":id},
                    vec![doc! {"$set":{"notification_claim_until":{"$add":["$$NOW",300_i64]}}}],
                )
                .await
                .unwrap();
        }
    }
    type ClaimCommands = Arc<std::sync::Mutex<Vec<Document>>>;
    async fn clock_fixture() -> (
        crate::AppState,
        MachineLifecycleEvent,
        SendGate,
        ClaimCommands,
    ) {
        let commands = ClaimCommands::default();
        let seen = commands.clone();
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "lifecycle_clock",
            mongodb::event::EventHandler::callback(move |event| {
                if let mongodb::event::command::CommandEvent::Started(event) = event
                    && (event.command.get_str("findAndModify") == Ok(EVENTS)
                        || event.command.get_str("update") == Ok(EVENTS))
                {
                    seen.lock().unwrap().push(event.command);
                }
            }),
        )
        .await;
        let state = crate::test_utils::test_app_state(db.clone());
        let event = MachineLifecycleEvent {
            id: uuid::Uuid::new_v4().to_string(),
            owner_id: uuid::Uuid::new_v4().to_string(),
            machine_id: None,
            operation_id: uuid::Uuid::new_v4().to_string(),
            kind: "ready".into(),
            message: "Ready".into(),
            notification: true,
            responsibility: "platform".into(),
            created_at: Utc::now(),
            expires_at: None,
        };
        db.collection::<MachineLifecycleEvent>(EVENTS)
            .insert_one(&event)
            .await
            .unwrap();
        let gate = SendGate {
            calls: Arc::new(AtomicUsize::new(0)),
            age_claim: false,
            delivered: true,
            started: Arc::new(tokio::sync::Notify::new()),
            resume: Arc::new(tokio::sync::Notify::new()),
        };
        gate.resume.notify_one();
        (state, event, gate, commands)
    }

    fn assert_server_clock_commands(commands: &ClaimCommands) {
        fn has_wall_clock(value: &bson::Bson) -> bool {
            match value {
                bson::Bson::DateTime(_) => true,
                bson::Bson::Document(doc) => doc.values().any(has_wall_clock),
                bson::Bson::Array(values) => values.iter().any(has_wall_clock),
                _ => false,
            }
        }
        let commands = commands.lock().unwrap();
        assert!(
            commands
                .iter()
                .any(|cmd| cmd.get_str("findAndModify") == Ok(EVENTS))
        );
        assert!(
            commands
                .iter()
                .any(|cmd| cmd.get_str("update") == Ok(EVENTS))
        );
        for command in commands.iter() {
            assert!(
                !has_wall_clock(&bson::Bson::Document(command.clone())),
                "client clock in {command:?}"
            );
            assert!(
                command.to_string().contains("$$NOW"),
                "no server clock in {command:?}"
            );
        }
    }

    #[tokio::test]
    async fn round2_lifecycle_claim_renews_with_process_clock_behind_database() {
        Box::pin(async {
            let (state, event, gate, commands) = clock_fixture().await;
            let legacy_claim_until = bson::DateTime::from_chrono(
                Utc::now() - Duration::minutes(5) + Duration::minutes(2),
            );
            assert!(
                state
                    .db
                    .collection::<Document>(EVENTS)
                    .find_one(doc! {
                        "_id": &event.id, "$expr": {"$lt": [legacy_claim_until, "$$NOW"]}
                    })
                    .await
                    .unwrap()
                    .is_some(),
                "a process-clock claim would fail the server-time renewal"
            );
            SEND.scope(gate.clone(), deliver_pending(&state))
                .await
                .unwrap();
            assert_eq!(
                gate.calls.load(Ordering::SeqCst),
                1,
                "a process-clock claim would already be expired in database time"
            );
            let row = state
                .db
                .collection::<Document>(EVENTS)
                .find_one(doc! {"_id":&event.id})
                .await
                .unwrap()
                .unwrap();
            assert!(row.get_datetime("notification_delivered_at").is_ok());
            assert_server_clock_commands(&commands);
            state.db.drop().await.unwrap();
        })
        .await;
    }

    #[tokio::test]
    async fn round2_lifecycle_eligibility_and_backoff_use_database_time() {
        Box::pin(async {
            let (state,event,mut gate,commands)=clock_fixture().await;
            gate.delivered=false;
            let rows=state.db.collection::<Document>(EVENTS);
            rows.update_one(doc!{"_id":&event.id},vec![doc!{"$set":{"notification_retry_at":{"$add":["$$NOW",300_000_i64]}}}]).await.unwrap();
            SEND.scope(gate.clone(),deliver_pending(&state)).await.unwrap();
            assert_eq!(gate.calls.load(Ordering::SeqCst),0,"a fast process clock must not bypass the server retry deadline");
            rows.update_one(doc!{"_id":&event.id},vec![doc!{"$set":{"notification_retry_at":{"$subtract":["$$NOW",1_i64]}}}]).await.unwrap();
            SEND.scope(gate.clone(),deliver_pending(&state)).await.unwrap();
            assert_eq!(gate.calls.load(Ordering::SeqCst),1);
            assert!(rows.find_one(doc!{"_id":&event.id,"$expr":{"$and":[
                {"$gt":["$notification_retry_at","$$NOW"]},{"$lt":["$notification_retry_at",{"$add":["$$NOW",120_000_i64]}]}
            ]}}).await.unwrap().is_some(),"backoff is scheduled from the database clock");
            assert_server_clock_commands(&commands);
            state.db.drop().await.unwrap();
        }).await;
    }

    #[tokio::test]
    async fn lifecycle_notice_lost_claim_is_never_dispatched() {
        let db =
            crate::test_utils::connect_transaction_test_database("lifecycle_notice_lost").await;
        let state = crate::test_utils::test_app_state(db.clone());
        let event = MachineLifecycleEvent {
            id: uuid::Uuid::new_v4().to_string(),
            owner_id: uuid::Uuid::new_v4().to_string(),
            machine_id: None,
            operation_id: uuid::Uuid::new_v4().to_string(),
            kind: "ready".into(),
            message: "Ready".into(),
            notification: true,
            responsibility: "platform".into(),
            created_at: Utc::now(),
            expires_at: None,
        };
        let mut document = bson::to_document(&event).unwrap();
        document.insert("notification_claim", "successor");
        document.insert(
            "notification_claim_until",
            bson::DateTime::from_chrono(Utc::now() + Duration::minutes(2)),
        );
        db.collection::<Document>(EVENTS)
            .insert_one(document.clone())
            .await
            .unwrap();
        let gate = SendGate {
            age_claim: true,
            delivered: true,
            calls: Arc::new(AtomicUsize::new(0)),
            started: Arc::new(tokio::sync::Notify::new()),
            resume: Arc::new(tokio::sync::Notify::new()),
        };
        let result = SEND
            .scope(
                gate.clone(),
                dispatch_lifecycle_notice(&state, &event, "old"),
            )
            .await;
        assert!(matches!(result, Err(AppError::Conflict(_))));
        assert_eq!(gate.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            db.collection::<Document>(EVENTS)
                .find_one(doc! {"_id":&event.id})
                .await
                .unwrap()
                .unwrap(),
            document
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn lifecycle_notice_send_outlives_original_claim_without_a_second_sender() {
        Box::pin(async {
            let db = crate::test_utils::connect_transaction_test_database("lifecycle_notice_claim")
                .await;
            let state = crate::test_utils::test_app_state(db.clone());
            let event = MachineLifecycleEvent {
                id: uuid::Uuid::new_v4().to_string(),
                owner_id: uuid::Uuid::new_v4().to_string(),
                machine_id: Some(uuid::Uuid::new_v4().to_string()),
                operation_id: uuid::Uuid::new_v4().to_string(),
                kind: "ready".into(),
                message: "Ready".into(),
                notification: true,
                responsibility: "platform".into(),
                created_at: Utc::now(),
                expires_at: None,
            };
            db.collection::<MachineLifecycleEvent>(EVENTS)
                .insert_one(&event)
                .await
                .unwrap();
            let gate = SendGate {
                age_claim: true,
                delivered: true,
                calls: Arc::new(AtomicUsize::new(0)),
                started: Arc::new(tokio::sync::Notify::new()),
                resume: Arc::new(tokio::sync::Notify::new()),
            };
            let (first, second) =
                tokio::join!(SEND.scope(gate.clone(), deliver_pending(&state)), async {
                    gate.started.notified().await;
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let result = SEND.scope(gate.clone(), deliver_pending(&state)).await;
                    gate.resume.notify_one();
                    result
                });
            first.unwrap();
            second.unwrap();
            assert_eq!(
                gate.calls.load(Ordering::SeqCst),
                1,
                "a live lifecycle sender must retain its claim past the old expiry"
            );
            let row = db
                .collection::<Document>(EVENTS)
                .find_one(doc! {"_id":&event.id})
                .await
                .unwrap()
                .unwrap();
            assert!(row.get_datetime("notification_delivered_at").is_ok());
            db.drop().await.unwrap();
        })
        .await;
    }
}
