//! Lifecycle notifications/projections and durable metadata-only investigations.
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
        let accepted_message = match stop_reason.as_deref().or(op.error_code.as_deref()) {
            Some("runtime_funding_unavailable" | "runtime_funding_expired") => {
                "Runtime funding stopped. Your workspace is retained. Restore funds or billing availability, then review a new quote and resume manually."
            }
            Some("expansion_deadline_exceeded") => {
                "Provisioning exceeded its deadline and was stopped. Your existing workspace is retained; check platform status before retrying."
            }
            Some("account_deletion_pending" | "owner_not_found")
                if op.action == LifecycleAction::Delete =>
            {
                "Account deletion stopped Machine access and queued runtime and workspace deletion. Cleanup remains pending until physical stop, storage reclamation, and accounting are verified."
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
            db.collection::<Document>(OPERATIONS).update_one(doc! {"_id":&op.id,"terminal_at":bson::Bson::Null},doc! {"$set":{"deadline_at":bson::DateTime::from_chrono(op.created_at+Duration::hours(24)),"cleanup_state":"pending","error_code":"cleanup_overdue","error_message":"Cleanup remains pending; no completion is inferred from the deadline"}}).await?;
        } else if now >= op.created_at + Duration::minutes(15) {
            record(db,&op,"reconciliation_delayed","Platform reconciliation is taking longer than expected; retained work data will not be deleted automatically",true).await?;
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
        let now = Utc::now();
        let claim = uuid::Uuid::new_v4().to_string();
        let event=state.db.collection::<Document>(EVENTS).find_one_and_update(doc! {
            "notification":true,"notification_delivered_at":bson::Bson::Null,
            "$and":[{"$or":[{"notification_retry_at":bson::Bson::Null},{"notification_retry_at":{"$lte":bson::DateTime::from_chrono(now)}}]},
              {"$or":[{"notification_claim_until":bson::Bson::Null},{"notification_claim_until":{"$lte":bson::DateTime::from_chrono(now)}}]}]
        },doc! {"$set":{"notification_claim":&claim,"notification_claim_until":bson::DateTime::from_chrono(now+Duration::minutes(2))},"$inc":{"notification_attempts":1_i64}}).sort(doc! {"created_at":1}).return_document(mongodb::options::ReturnDocument::After).await?;
        let Some(document) = event else { break };
        let event: MachineLifecycleEvent = bson::from_document(document.clone())
            .map_err(|_| AppError::Internal("lifecycle event invalid".into()))?;
        let result = super::notification_service::send_machine_lifecycle_notification(
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
        .await;
        let attempts = document.get_i64("notification_attempts").unwrap_or(1);
        let mut updates = doc! {"notification_claim":bson::Bson::Null,"notification_claim_until":bson::Bson::Null};
        if result.as_ref().is_ok_and(|r| !r.channels.is_empty()) {
            updates.insert(
                "notification_delivered_at",
                bson::DateTime::from_chrono(now),
            );
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
                bson::DateTime::from_chrono(now + retry),
            );
        }
        state
            .db
            .collection::<Document>(EVENTS)
            .update_one(
                doc! {"_id":&event.id,"notification_claim":&claim},
                doc! {"$set":updates},
            )
            .await?;
    }
    Ok(())
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
        for reason in ["account_deletion_pending", "owner_not_found"] {
            let op = account_safety_operation(&db, LifecycleAction::Delete, reason).await;
            sweep(&db, Utc::now()).await.unwrap();
            let event = db
                .collection::<MachineLifecycleEvent>(EVENTS)
                .find_one(doc! {"operation_id":&op.id,"kind":"accepted"})
                .await
                .unwrap()
                .unwrap();
            assert!(
                event
                    .message
                    .contains("queued runtime and workspace deletion")
            );
            assert!(event.message.contains("Cleanup remains pending until physical stop, storage reclamation, and accounting are verified"));
            assert!(event.notification);
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
