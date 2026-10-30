//! Leased, fenced and bounded vendor erasure. Never dispatch while the source
//! account exists: deletion must succeed before a vendor person can be removed.
use crate::{
    errors::{AppError, AppResult},
    models::telemetry_erasure_job::{COLLECTION_NAME, TelemetryErasureJob},
    telemetry::erasure::ErasureClients,
};
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    options::{ReadConcern, ReturnDocument, WriteConcern},
};
use std::{sync::Arc, time::Duration};

const DESTINATIONS: &str = "telemetry_erasure_destinations";
const BATCH_SIZE: usize = 16;
const LEASE_MS: i64 = 60_000;

fn jobs(db: &Database) -> mongodb::Collection<Document> {
    super::transaction_service::durable_database(db).collection(COLLECTION_NAME)
}

fn validate_destination_id(id: &str) -> AppResult<()> {
    if id == "unresolved"
        || id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(AppError::ValidationError(
            "Invalid or reserved erasure destination identifier".into(),
        ));
    }
    Ok(())
}

pub async fn register_destinations(db: &Database, clients: &ErasureClients) -> AppResult<()> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    for destination in clients.0.values() {
        validate_destination_id(&destination.id)?;
        // A retained identifier must never silently point at another project.
        let filter = doc! {"_id":&destination.id};
        let registry = db.collection::<Document>(DESTINATIONS);
        registry.update_one(filter.clone(),doc! {"$setOnInsert":{"api_host":&destination.api_host,"project_id":&destination.project_id}}).upsert(true).await?;
        let row = registry
            .find_one(filter)
            .await?
            .ok_or_else(|| AppError::Internal("Erasure destination registration failed".into()))?;
        if row.get_str("api_host").ok() != Some(destination.api_host.as_str())
            || row.get_str("project_id").ok() != Some(destination.project_id.as_str())
        {
            return Err(AppError::ValidationError(
                "Erasure destination identifier is already bound to another project".into(),
            ));
        }
    }
    Ok(())
}

pub fn spawn_worker(db: crate::db::DbHandle, clients: ErasureClients) {
    let clients = Arc::new(clients);
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(30));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            if drain_once(&db, &clients).await.is_err() {
                tracing::warn!("Telemetry erasure pass failed; durable checkpoints retained");
            }
        }
    });
}

/// Central deletion workflow calls this even when capture is currently off.
/// Registry rows outlive disabled destinations; missing credentials remain pending.
fn job_id(user_id: &str, destination: Option<&str>) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!(
            "nyxid:telemetry-erasure:{user_id}:{}",
            destination.unwrap_or("unresolved")
        )
        .as_bytes(),
    )
    .to_string()
}
async fn ensure_job(db: &Database, user_id: &str, destination: Option<&str>) -> AppResult<String> {
    if let Some(destination) = destination {
        validate_destination_id(destination)?;
    }
    let id = job_id(user_id, destination);
    let mut row = TelemetryErasureJob::new(user_id);
    row.id = id.clone();
    row.destination_id = destination.map(str::to_owned);
    jobs(db)
        .update_one(
            doc! {"_id":&id},
            doc! {"$setOnInsert":bson::to_document(&row)
            .map_err(|_|AppError::Internal("Erasure checkpoint encoding failed".into()))?},
        )
        .upsert(true)
        .await?;
    Ok(id)
}
/// Acceptance retains vendor intent in the SAME majority transaction as the
/// account fence. A rejected deletion must leave no independently actionable job.
pub(crate) async fn enqueue_in_session(
    db: &Database,
    user_id: &str,
    tx: &mut mongodb::ClientSession,
) -> AppResult<String> {
    let id = job_id(user_id, None);
    let mut row = TelemetryErasureJob::new(user_id);
    row.id = id.clone();
    db.collection::<Document>(COLLECTION_NAME).update_one(
        doc! {"_id":&id},
        doc! {"$setOnInsert":bson::to_document(&row).map_err(|_|AppError::Internal("Erasure checkpoint encoding failed".into()))?},
    ).upsert(true).session(tx).await?;
    Ok(id)
}
pub(crate) async fn inventory(db: &Database) -> AppResult<(Vec<String>, String)> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    let rows: Vec<Document> = db
        .collection(DESTINATIONS)
        .find(doc! {})
        .sort(doc! {"_id":1})
        .limit(17)
        .await?
        .try_collect()
        .await?;
    inventory_revision(rows)
}
fn inventory_revision(rows: Vec<Document>) -> AppResult<(Vec<String>, String)> {
    use sha2::{Digest, Sha256};
    if rows.len() > 16 {
        return Err(AppError::Conflict(
            "Erasure destination inventory exceeds the bounded registry".into(),
        ));
    }
    let mut ids = Vec::new();
    let mut hash = Sha256::new();
    for row in rows {
        let id = row
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid destination registry".into()))?;
        for field in ["_id", "api_host", "project_id"] {
            let value = row
                .get_str(field)
                .map_err(|_| AppError::Internal("Invalid destination registry".into()))?;
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        validate_destination_id(id)?;
        ids.push(id.to_owned());
    }
    Ok((ids, hex::encode(hash.finalize())))
}

/// Point-in-time completion proof for a personal-erasure checkpoint. All
/// registry, dispatch and child evidence comes from one majority snapshot.
/// A retained parent timestamp alone cannot prove the current inventory.
pub(crate) async fn completion_evidence(db: &Database, user: &str) -> AppResult<Option<String>> {
    use sha2::{Digest, Sha256};
    let db = super::transaction_service::durable_database(db);
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<Option<String>> = async {
        let mut cursor = db
            .collection::<Document>(DESTINATIONS)
            .find(doc! {})
            .sort(doc! {"_id":1})
            .limit(17)
            .session(&mut tx)
            .await?;
        let rows: Vec<Document> = cursor.stream(&mut tx).try_collect().await?;
        let (destinations, revision) = inventory_revision(rows)?;
        if db
            .collection::<Document>("users")
            .find_one(doc! {"_id":user})
            .session(&mut tx)
            .await?
            .is_some()
            || db
                .collection::<Document>(super::analytics_dispatch_service::COLLECTION)
                .find_one(doc! {"user_id":user,"status":{"$in":["in_flight","uncertain"]}})
                .session(&mut tx)
                .await?
                .is_some()
        {
            return Ok(None);
        }
        let account = db
            .collection::<Document>(super::analytics_dispatch_service::ACCOUNTS)
            .find_one(doc! {"_id":user})
            .session(&mut tx)
            .await?;
        let generation = account
            .as_ref()
            .and_then(|row| row.get_str("generation").ok());
        if account.as_ref().is_some_and(|row| {
            generation.is_none() || row.get_str("provider_drained_generation").ok() != generation
        }) {
            return Ok(None);
        }
        let mut hash = Sha256::new();
        hash.update(b"personal-vendor-completion-v1");
        hash.update(revision.as_bytes());
        hash.update(generation.unwrap_or_default().as_bytes());
        for destination in
            std::iter::once(None).chain(destinations.iter().map(|id| Some(id.as_str())))
        {
            let Some(row) = jobs(&db)
                .find_one(doc! {"_id":job_id(user,destination)})
                .session(&mut tx)
                .await?
            else {
                return Ok(None);
            };
            if row.get_str("status").ok() != Some("completed")
                || row.get_datetime("completion_verified_at").is_err()
                || row.get_str("completion_dispatch_generation").ok() != generation
                || (destination.is_none()
                    && row.get_str("inventory_revision").ok() != Some(revision.as_str()))
                || (destinations.is_empty()
                    && row.get_str("completion_basis").ok()
                        != Some("operator_attested_no_historical_destinations"))
            {
                return Ok(None);
            }
            hash.update(destination.unwrap_or_default().as_bytes());
            hash.update(
                row.get_datetime("completion_verified_at")
                    .unwrap()
                    .timestamp_millis()
                    .to_be_bytes(),
            );
            hash.update(
                row.get_str("completion_basis")
                    .unwrap_or_default()
                    .as_bytes(),
            );
        }
        Ok(Some(hex::encode(hash.finalize())))
    }
    .await;
    match result {
        Ok(Some(evidence)) => {
            // Snapshot majority guarantees require a successful majority
            // COMMIT even for this read-only transaction. The bounded helper
            // never turns an indeterminate commit into completion evidence.
            super::transaction_service::commit(&mut tx).await?;
            Ok(Some(evidence))
        }
        Ok(None) => {
            tx.abort_transaction().await?;
            Ok(None)
        }
        Err(error) => {
            tx.abort_transaction().await?;
            Err(error)
        }
    }
}
/// Bounded restartable scan also discovers legacy per-destination-only jobs,
/// including completed ones. The cursor advances after idempotent parent upsert.
async fn backfill_intents(db: &Database) -> AppResult<()> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    let progress = db.collection::<Document>("security_remediation_progress");
    let key = "telemetry-erasure-parent-backfill";
    let cursor = progress
        .find_one(doc! {"_id":key})
        .await?
        .and_then(|row| row.get_str("cursor").ok().map(str::to_owned))
        .unwrap_or_default();
    let rows: Vec<Document> = jobs(db)
        .find(doc! {"_id":{"$gt":&cursor}})
        .sort(doc! {"_id":1})
        .limit(BATCH_SIZE as i64)
        .projection(doc! {"user_id":1})
        .await?
        .try_collect()
        .await?;
    let mut next = String::new();
    for row in &rows {
        let user = row
            .get_str("user_id")
            .map_err(|_| AppError::Internal("Invalid legacy erasure intent".into()))?;
        ensure_job(db, user, None).await?;
        next = row
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid erasure identifier".into()))?
            .to_owned();
    }
    // Repeating the bounded walk covers concurrent legacy inserts behind cursor.
    progress
        .update_one(doc! {"_id":key}, doc! {"$set":{"cursor":next}})
        .upsert(true)
        .await?;
    Ok(())
}
async fn dispatch_generation(db: &Database, user: &str) -> AppResult<Option<String>> {
    Ok(super::transaction_service::durable_database(db)
        .collection::<Document>(super::analytics_dispatch_service::ACCOUNTS)
        .find_one(doc! {"_id":user})
        .await?
        .and_then(|row| row.get_str("generation").ok().map(str::to_owned)))
}

/// Rechecking an invalid personal checkpoint also wakes an unchanged-inventory
/// parent whose capture generation changed after its previous receipt.
pub(crate) async fn wake_parent(db: &Database, user: &str) -> AppResult<()> {
    jobs(db).update_one(doc! {"user_id":user,"destination_id":bson::Bson::Null,"status":"completed"},
        vec![doc! {"$set":{"status":"pending","completion_verified_at":bson::Bson::Null,"retry_at":"$$NOW","updated_at":"$$NOW"}}]).await?;
    Ok(())
}

async fn reconcile_parent(db: &Database, job: &TelemetryErasureJob) -> AppResult<()> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    let (destinations, revision) = inventory(db).await?;
    if destinations.is_empty() {
        checkpoint(db, job, doc! {"status":"operator_action","last_error":"historical_destination_inventory_required","inventory_revision":revision}, true).await?;
        return Ok(());
    }
    let generation = dispatch_generation(db, &job.user_id).await?;
    let mut complete = true;
    let mut operator_action = false;
    for destination in destinations {
        let id = ensure_job(db, &job.user_id, Some(&destination)).await?;
        let child = jobs(db)
            .find_one(doc! {"_id":id})
            .await?
            .ok_or_else(|| AppError::Internal("Missing erasure child".into()))?;
        let generation_matches =
            child.get_str("completion_dispatch_generation").ok() == generation.as_deref();
        if child.get_str("status").ok() == Some("completed") && !generation_matches {
            // The old receipt covers a previous capture generation. Preserve
            // the person identity, but require a fresh delete/status baseline.
            jobs(db).update_one(doc! {"_id":child.get_str("_id").unwrap(),"status":"completed",
                "completion_dispatch_generation":child.get("completion_dispatch_generation").cloned().unwrap_or(bson::Bson::Null)},
                vec![doc! {"$set":{"status":"pending","accepted_at":bson::Bson::Null,
                    "status_baseline":bson::Bson::Null,"completion_verified_at":bson::Bson::Null,
                    "completion_basis":bson::Bson::Null,"retry_at":"$$NOW"}}]).await?;
        }
        complete &= child.get_str("status").ok() == Some("completed")
            && child.get_datetime("completion_verified_at").is_ok()
            && generation_matches;
        operator_action |= child.get_str("status").ok() == Some("operator_action");
    }
    let status = if complete {
        "completed"
    } else if operator_action {
        "operator_action"
    } else {
        "pending"
    };
    let verified = if complete {
        bson::Bson::String("$$NOW".into())
    } else {
        bson::Bson::Null
    };
    checkpoint(db, job, doc! {"status":status,"inventory_revision":revision,"completion_verified_at":verified,
        "completion_dispatch_generation":generation,
        "last_error":if operator_action {bson::Bson::String("child_requires_operator_reconciliation".into())} else {bson::Bson::Null}}, true).await?;
    Ok(())
}

/// An explicit, fenced local operator action is recorded durably in the same
/// transaction as its checkpoint. It never masquerades as a vendor status row.
pub async fn operator_reconcile(
    db: &Database,
    job_id: &str,
    person_uuid: Option<&str>,
    operator: &str,
    evidence: &str,
    reason: Option<&str>,
) -> AppResult<()> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    for value in [operator, evidence].into_iter().chain(reason) {
        if value.trim().is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
            return Err(AppError::ValidationError(
                "Bounded operator identity and evidence are required".into(),
            ));
        }
    }
    if let Some(person) = person_uuid {
        uuid::Uuid::parse_str(person)
            .map_err(|_| AppError::ValidationError("Invalid vendor person UUID".into()))?;
    } else if reason.is_none() {
        return Err(AppError::ValidationError(
            "No-data attestation requires a reason".into(),
        ));
    }
    let mut transaction = db.client().start_session().await?;
    transaction
        .start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let job = jobs(db)
        .find_one(
            doc! {"_id":job_id,"destination_id":{"$type":"string"},"status":"operator_action",
            "$or":[{"lease_expires_at":null},{"$expr":{"$lte":["$lease_expires_at","$$NOW"]}}]},
        )
        .session(&mut transaction)
        .await?
        .ok_or_else(|| AppError::Conflict("Job is not awaiting operator reconciliation".into()))?;
    let user = job
        .get_str("user_id")
        .map_err(|_| AppError::Internal("Invalid erasure account".into()))?;
    if db
        .collection::<Document>("users")
        .find_one(doc! {"_id":user})
        .session(&mut transaction)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Local account deletion has not completed".into(),
        ));
    }
    let drained_generation = if person_uuid.is_none() {
        super::analytics_dispatch_service::fence_drained_in_transaction(db, user, &mut transaction)
            .await?
    } else {
        None
    };
    let action = if person_uuid.is_some() {
        "bind_person"
    } else {
        "attest_no_data"
    };
    db.collection::<Document>("telemetry_erasure_reconciliations").insert_one(doc! {
        "_id":uuid::Uuid::new_v4().to_string(),"job_id":job_id,"user_id":user,"operator":operator,
        "evidence":evidence,"reason":reason,"action":action,"drained_generation":&drained_generation,"created_at":bson::DateTime::now()
    }).session(&mut transaction).await?;
    let set = if let Some(person) = person_uuid {
        doc! {"person_uuid":person,"status":"pending","status_baseline":bson::Bson::Null,
        "accepted_at":bson::Bson::Null,"completion_verified_at":bson::Bson::Null,"completion_basis":bson::Bson::Null}
    } else {
        doc! {"status":"completed","completion_verified_at":"$$NOW","completion_basis":"operator_attested_no_data","completion_dispatch_generation":&drained_generation}
    };
    let mut set = set;
    set.extend(doc! {"last_error":bson::Bson::Null,"retry_at":"$$NOW","lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null});
    jobs(db)
        .update_one(doc! {"_id":job_id}, vec![doc! {"$set":set}])
        .session(&mut transaction)
        .await?;
    jobs(db).update_many(doc! {"user_id":user,"destination_id":null},doc! {"$set":{"status":"pending","completion_verified_at":bson::Bson::Null,"retry_at":bson::DateTime::now()}})
        .session(&mut transaction).await?;
    super::transaction_service::commit(&mut transaction).await
}

/// Trusted local DB operators and authenticated admin routes can resolve a
/// never-configured installation after verifying its historical deployment
/// inventory. Empty current configuration by itself never proves no history.
pub async fn operator_reconcile_empty_inventory(
    db: &Database,
    user: &str,
    operator: &str,
    evidence: &str,
    reason: &str,
) -> AppResult<()> {
    for value in [operator, evidence, reason] {
        if value.trim().is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
            return Err(AppError::ValidationError(
                "Bounded operator identity, evidence and reason are required".into(),
            ));
        }
    }
    let db = super::transaction_service::durable_database(db);
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        if db.collection::<Document>("users").find_one(doc! {"_id":user}).session(&mut tx).await?.is_some() {
            return Err(AppError::Conflict("Local account deletion has not completed".into()));
        }
        let mut cursor = db.collection::<Document>(DESTINATIONS).find(doc! {}).limit(17).session(&mut tx).await?;
        let rows: Vec<Document> = cursor.stream(&mut tx).try_collect().await?;
        let (destinations, revision) = inventory_revision(rows)?;
        if !destinations.is_empty() {
            return Err(AppError::Conflict("Historical destinations require their individual erasure evidence".into()));
        }
        let generation = super::analytics_dispatch_service::fence_drained_in_transaction(&db, user, &mut tx).await?;
        let receipt = uuid::Uuid::new_v4().to_string();
        let changed = jobs(&db).update_one(doc! {"user_id":user,"destination_id":bson::Bson::Null,"status":"operator_action",
            "$expr":{"$lte":[{"$ifNull":["$lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]}},
            vec![doc! {"$set":{"status":"completed","completion_verified_at":"$$NOW",
                "completion_basis":"operator_attested_no_historical_destinations","inventory_revision":{"$literal":&revision},
                "completion_dispatch_generation":{"$literal":&generation},"inventory_reconciliation_id":{"$literal":&receipt},
                "last_error":bson::Bson::Null,"updated_at":"$$NOW","lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null}}])
            .session(&mut tx).await?;
        if changed.matched_count != 1 { return Err(AppError::Conflict("Inventory erasure job is not awaiting operator reconciliation".into())); }
        db.collection::<Document>("telemetry_erasure_reconciliations").insert_one(doc! {
            "_id":&receipt,"user_id":user,"operator":operator,"evidence":evidence,"reason":reason,
            "action":"attest_no_historical_destinations","inventory_revision":&revision,"drained_generation":&generation,
            "created_at":bson::DateTime::now(),
        }).session(&mut tx).await?;
        let key = super::audit_service::audit_chain_hmac_key().ok_or_else(||AppError::Internal("Audit chain key is not initialized".into()))?;
        super::audit_service::log_actor_event_in_session(&db, &mut tx, key,
            &super::audit_service::AuditActor {user_id:operator.into(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},
            "user.account.erasure_empty_vendor_inventory_attested",serde_json::json!({"target_user_id":user,"receipt_id":receipt,
                "inventory_revision":revision,"dispatch_generation":generation})).await?;
        Ok(())
    }.await;
    super::org_membership_authority::finish(tx, result).await
}

async fn claim(db: &Database) -> AppResult<Option<TelemetryErasureJob>> {
    let token = uuid::Uuid::new_v4().to_string();
    let (_, revision) = inventory(db).await?;
    let row=jobs(db).find_one_and_update(doc! {"$and":[
        {"$or":[{"status":{"$in":["pending","in_flight","accepted","operator_action","failed"]}},{"status":"completed","completion_verified_at":null},
            {"destination_id":null,"inventory_revision":{"$ne":revision}}]},
        {"$or":[{"lease_expires_at":null},{"$expr":{"$lte":["$lease_expires_at","$$NOW"]}}]},
        {"$or":[{"retry_at":null},{"$expr":{"$lte":["$retry_at","$$NOW"]}}]}
    ]},vec![doc! {"$set":{"status":"in_flight","lease_token":token,"lease_expires_at":{"$add":["$$NOW",LEASE_MS]},"updated_at":"$$NOW","attempts":{"$add":[{"$ifNull":["$attempts",0]},1]}}}])
        .sort(doc! {"created_at":1}).return_document(ReturnDocument::After).await?;
    row.map(bson::from_document)
        .transpose()
        .map_err(|_| AppError::Internal("Invalid erasure job checkpoint".into()))
}
fn fence(job: &TelemetryErasureJob) -> Document {
    doc! {"_id":&job.id,"lease_token":&job.lease_token,"status":"in_flight","$expr":{"$gt":["$lease_expires_at","$$NOW"]}}
}
async fn checkpoint(
    db: &Database,
    job: &TelemetryErasureJob,
    mut set: Document,
    release: bool,
) -> AppResult<bool> {
    set.insert("updated_at", "$$NOW");
    if release {
        set.insert("lease_token", bson::Bson::Null);
        set.insert("lease_expires_at", bson::Bson::Null);
        let delay = if set
            .get("last_error")
            .is_some_and(|v| !matches!(v, bson::Bson::Null))
        {
            30_000_i64
                .saturating_mul(1_i64 << job.attempts.min(7))
                .min(3_600_000)
        } else {
            30_000
        };
        set.insert("retry_at", doc! {"$add":["$$NOW",delay]});
    }
    let result = jobs(db)
        .update_one(fence(job), vec![doc! {"$set":set}])
        .await?;
    Ok(result.matched_count == 1)
}

async fn process(
    db: &Database,
    clients: &ErasureClients,
    job: &TelemetryErasureJob,
) -> AppResult<()> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    // A failed/partial local deletion must never trigger vendor erasure while
    // the same account can still generate product events. Existence fails closed.
    if db
        .collection::<Document>("users")
        .find_one(doc! {"_id":&job.user_id})
        .await?
        .is_some()
    {
        checkpoint(
            db,
            job,
            doc! {"status":"pending","last_error":"account_deletion_not_complete"},
            true,
        )
        .await?;
        return Ok(());
    }
    if super::analytics_dispatch_service::has_unsettled(db, &job.user_id).await? {
        checkpoint(db, job, doc! {"status":"operator_action","last_error":"capture_in_flight_or_uncertain_requires_drain_evidence"}, true).await?;
        return Ok(());
    }
    if super::analytics_dispatch_service::needs_provider_drain(db, &job.user_id).await? {
        checkpoint(db, job, doc! {"status":"operator_action","last_error":"provider_processing_drain_evidence_required"},true).await?;
        return Ok(());
    }
    if job.destination_id.is_none() {
        return reconcile_parent(db, job).await;
    }
    let Some(destination) = job.destination_id.as_ref().and_then(|id| clients.0.get(id)) else {
        checkpoint(db,job,doc! {"status":"operator_action","last_error":"destination_or_credentials_unconfigured"},true).await?;
        return Ok(());
    };
    let person = match job.person_uuid.as_ref() {
        Some(id) => id.clone(),
        None => match destination.lookup(&job.user_id).await {
            Ok(Some(id)) => {
                if !checkpoint(db, job, doc! {"person_uuid":&id}, false).await? {
                    return Ok(());
                }
                id
            }
            Ok(None) => {
                checkpoint(db,job,doc! {"status":"operator_action","last_error":"person_uuid_unavailable_requires_review"},true).await?;
                return Ok(());
            }
            Err(_) => {
                checkpoint(
                    db,
                    job,
                    doc! {"status":"pending","last_error":"vendor_lookup_failed"},
                    true,
                )
                .await?;
                return Ok(());
            }
        },
    };
    // Poll first even after a crash during DELETE: its UUID was persisted before
    // dispatch, so accepted work remains discoverable without a person lookup.
    match destination
        .deletion_status(&person, job.status_baseline.as_deref())
        .await
    {
        Ok(status) if status.action_required => {
            checkpoint(db, job, doc! {"status":"operator_action","last_error":"vendor_deletion_failed_or_unknown_status_requires_reconciliation"}, true).await?;
            return Ok(());
        }
        Ok(status) if status.completed => {
            // A new person at the same distinct ID is not covered by the old
            // UUID's completion. Reconcile it rather than report success.
            match destination.lookup(&job.user_id).await {
                Ok(None) => {}
                Ok(Some(person)) => {
                    checkpoint(db, job, doc! {"person_uuid":person,"status":"pending","accepted_at":bson::Bson::Null,
                        "status_baseline":bson::Bson::Null,"completion_verified_at":bson::Bson::Null,
                        "last_error":"person_present_after_deletion_requires_reconciliation"}, true).await?;
                    return Ok(());
                }
                Err(_) => {
                    checkpoint(
                        db,
                        job,
                        doc! {"status":"accepted","last_error":"post_delete_lookup_unavailable"},
                        true,
                    )
                    .await?;
                    return Ok(());
                }
            }
            let generation = dispatch_generation(db, &job.user_id).await?;
            checkpoint(db,job,doc! {"status":"completed","completion_verified_at":"$$NOW","completion_basis":"vendor_status_verified",
                "completion_dispatch_generation":generation,"last_error":bson::Bson::Null},true).await?;
            return Ok(());
        }
        Err(_) => {
            checkpoint(
                db,
                job,
                doc! {"status":"pending","last_error":"vendor_status_unavailable"},
                true,
            )
            .await?;
            return Ok(());
        }
        Ok(status) => {
            if job.status_baseline.is_none()
                && !checkpoint(db, job, doc! {"status_baseline":status.markers}, false).await?
            {
                return Ok(());
            }
        }
    }
    if job.accepted_at.is_some() {
        checkpoint(
            db,
            job,
            doc! {"status":"accepted","last_error":bson::Bson::Null},
            true,
        )
        .await?;
        return Ok(());
    }
    // Renew/check the exact lease immediately before the external operation.
    if !checkpoint(
        db,
        job,
        doc! {"lease_expires_at":{"$add":["$$NOW",LEASE_MS]}},
        false,
    )
    .await?
    {
        return Ok(());
    }
    match destination.request_delete(&person).await {
        Ok(()) => {
            checkpoint(
                db,
                job,
                doc! {"status":"accepted","accepted_at":"$$NOW","last_error":bson::Bson::Null},
                true,
            )
            .await?;
        }
        Err(_) => {
            checkpoint(
                db,
                job,
                doc! {"status":"pending","last_error":"vendor_delete_unconfirmed"},
                true,
            )
            .await?;
        }
    }
    Ok(())
}
pub(crate) async fn drain_once(db: &Database, clients: &ErasureClients) -> AppResult<()> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    backfill_intents(db).await?;
    for _ in 0..BATCH_SIZE {
        let Some(job) = claim(db).await? else {
            break;
        };
        process(db, clients, &job).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{telemetry::erasure::test_support, test_utils::connect_transaction_test_database};
    use std::sync::atomic::Ordering;
    async fn due(db: &Database, id: &str) {
        let row = jobs(db).find_one(doc! {"_id":id}).await.unwrap().unwrap();
        jobs(db)
            .update_many(
                doc! {"user_id":row.get_str("user_id").unwrap()},
                doc! {"$set":{"retry_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn actual_vendor_egress_waits_for_local_deletion_and_verifies_async_completion() {
        let db = connect_transaction_test_database("erasure_egress").await;
        let (clients, server) = test_support::clients().await;
        register_destinations(&db, &clients).await.unwrap();
        let user = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>("users")
            .insert_one(doc! {"_id":&user,"is_active":true})
            .await
            .unwrap();
        let id = ensure_job(&db, &user, None).await.unwrap();
        assert_eq!(
            ensure_job(&db, &user, None).await.unwrap(),
            id,
            "central intent is resumable and idempotent"
        );
        drain_once(&db, &clients).await.unwrap();
        assert_eq!(server.state.lookups.load(Ordering::SeqCst), 0);
        assert_eq!(server.state.deletes.load(Ordering::SeqCst), 0);
        db.collection::<Document>("users")
            .delete_one(doc! {"_id":&user})
            .await
            .unwrap();
        due(&db, &id).await;
        drain_once(&db, &clients).await.unwrap();
        let child = job_id(&user, Some("historical-project"));
        let accepted = jobs(&db)
            .find_one(doc! {"_id":&child})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(accepted.get_str("status").unwrap(), "accepted");
        assert!(accepted.get_datetime("accepted_at").is_ok());
        assert!(
            accepted
                .get("completion_verified_at")
                .is_none_or(|v| v == &bson::Bson::Null)
        );
        assert_eq!(server.state.deletes.load(Ordering::SeqCst), 1);
        due(&db, &id).await;
        drain_once(&db, &clients).await.unwrap();
        assert_eq!(
            server.state.deletes.load(Ordering::SeqCst),
            1,
            "poll accepted work rather than repeating delete"
        );
        server.state.verified.store(true, Ordering::SeqCst);
        due(&db, &id).await;
        drain_once(&db, &clients).await.unwrap();
        let completed = jobs(&db)
            .find_one(doc! {"_id":&child})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completed.get_str("status").unwrap(), "completed");
        assert!(completed.get_datetime("completion_verified_at").is_ok());
        assert!(!format!("{completed:?}").contains("distinctive-personal-api-key"));
    }
    #[tokio::test]
    async fn failed_vendor_deletion_is_actionable_and_evidenced_retry_can_complete() {
        let db = connect_transaction_test_database("erasure_vendor_failed").await;
        let (clients, server) = test_support::clients().await;
        register_destinations(&db, &clients).await.unwrap();
        let user = uuid::Uuid::new_v4().to_string();
        let intent = ensure_job(&db, &user, None).await.unwrap();
        drain_once(&db, &clients).await.unwrap();
        let child = job_id(&user, Some("historical-project"));
        server.state.deletion_failed.store(true, Ordering::SeqCst);
        due(&db, &intent).await;
        drain_once(&db, &clients).await.unwrap();
        let failed = jobs(&db)
            .find_one(doc! {"_id":&child})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed.get_str("status").unwrap(), "operator_action");
        assert_eq!(
            failed.get_str("last_error").unwrap(),
            "vendor_deletion_failed_or_unknown_status_requires_reconciliation"
        );
        assert!(
            failed
                .get("completion_verified_at")
                .is_none_or(|v| v == &bson::Bson::Null)
        );
        assert_eq!(server.state.deletes.load(Ordering::SeqCst), 1);
        // After investigating/fixing the provider, an existing explicit
        // bind-person reconciliation records evidence and starts a fresh request.
        operator_reconcile(
            &db,
            &child,
            Some(test_support::PERSON),
            "test-operator",
            "local-fixture-provider-repaired",
            None,
        )
        .await
        .unwrap();
        server.state.deletion_failed.store(false, Ordering::SeqCst);
        due(&db, &intent).await;
        drain_once(&db, &clients).await.unwrap();
        assert_eq!(server.state.deletes.load(Ordering::SeqCst), 2);
        server.state.verified.store(true, Ordering::SeqCst);
        due(&db, &intent).await;
        drain_once(&db, &clients).await.unwrap();
        let completed = jobs(&db)
            .find_one(doc! {"_id":&child})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completed.get_str("status").unwrap(), "completed");
        assert_eq!(
            db.collection::<Document>("telemetry_erasure_reconciliations")
                .count_documents(doc! {"job_id":&child,"action":"bind_person"})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn independent_workers_reclaim_crashed_lease_and_fence_stale_writes() {
        let db = connect_transaction_test_database("erasure_lease").await;
        let id = ensure_job(&db, &uuid::Uuid::new_v4().to_string(), None)
            .await
            .unwrap();
        let client = mongodb::Client::with_uri_str(
            std::env::var("NYXID_TEST_DATABASE_URL").expect("test URI required"),
        )
        .await
        .unwrap();
        let other = client.database(db.name());
        let (a, b) = tokio::join!(claim(&db), claim(&other));
        let a = a.unwrap();
        let b = b.unwrap();
        assert_eq!(usize::from(a.is_some()) + usize::from(b.is_some()), 1);
        let abandoned = a.or(b).unwrap();
        jobs(&db)
            .update_one(
                doc! {"_id":&id},
                doc! {"$set":{"lease_expires_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        let successor = claim(&other).await.unwrap().unwrap();
        assert_ne!(abandoned.lease_token, successor.lease_token);
        assert!(
            !checkpoint(&db, &abandoned, doc! {"status":"completed"}, true)
                .await
                .unwrap()
        );
        assert!(
            checkpoint(&other, &successor, doc! {"status":"pending"}, true)
                .await
                .unwrap()
        );
    }
    #[tokio::test]
    async fn unconfigured_and_missing_person_never_report_erasure_success() {
        let db = connect_transaction_test_database("erasure_pending").await;
        let id = ensure_job(&db, &uuid::Uuid::new_v4().to_string(), None)
            .await
            .unwrap();
        drain_once(&db, &ErasureClients::default()).await.unwrap();
        assert_eq!(
            jobs(&db)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "operator_action"
        );
        let (clients, server) = test_support::clients().await;
        register_destinations(&db, &clients).await.unwrap();
        let id = ensure_job(&db, &uuid::Uuid::new_v4().to_string(), None)
            .await
            .unwrap();
        server.state.missing_person.store(true, Ordering::SeqCst);
        drain_once(&db, &clients).await.unwrap();
        due(&db, &id).await;
        drain_once(&db, &clients).await.unwrap();
        assert_eq!(
            jobs(&db)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "operator_action"
        );
        assert_eq!(server.state.deletes.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn late_registry_fans_out_legacy_intents_and_reopens_completed_parents() {
        let db = connect_transaction_test_database("erasure_fanout").await;
        let user = uuid::Uuid::new_v4().to_string();
        let root = ensure_job(&db, &user, None).await.unwrap();
        drain_once(&db, &ErasureClients::default()).await.unwrap();
        assert_eq!(
            jobs(&db)
                .find_one(doc! {"_id":&root})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "operator_action"
        );
        let (clients, server) = test_support::clients().await;
        register_destinations(&db, &clients).await.unwrap();
        due(&db, &root).await;
        drain_once(&db, &clients).await.unwrap();
        let child = job_id(&user, Some("historical-project"));
        assert_eq!(
            jobs(&db)
                .find_one(doc! {"_id":&child})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "accepted"
        );
        server.state.verified.store(true, Ordering::SeqCst);
        due(&db, &root).await;
        drain_once(&db, &clients).await.unwrap();
        due(&db, &root).await;
        drain_once(&db, &clients).await.unwrap();
        assert_eq!(
            jobs(&db)
                .find_one(doc! {"_id":&root})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "completed"
        );
        let personal_evidence = completion_evidence(&db, &user)
            .await
            .unwrap()
            .expect("current vendor receipts");
        let now = chrono::Utc::now();
        let personal = crate::models::personal_erasure::PersonalErasureJob {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user.clone(),
            requested_by: user.clone(),
            owner_deletion_policy: None,
            status: crate::models::personal_erasure::ErasureStatus::Complete,
            inventory_version: 1,
            domains: vec![crate::models::personal_erasure::DomainCheckpoint {
                retry_generation: 0,
                last_retry_key_hash: None,
                last_retry_request_digest: None,
                consecutive_failures: 0,
                domain: "vendor".into(),
                status: crate::models::personal_erasure::DomainStatus::Complete,
                cursor: Some(personal_evidence),
                reason: None,
                completed_at: Some(now),
            }],
            lease_token: None,
            lease_expires_at: None,
            retry_at: None,
            accepted_at: now,
            updated_at: now,
            completed_at: Some(now),
        };
        db.collection::<crate::models::personal_erasure::PersonalErasureJob>(
            crate::models::personal_erasure::COLLECTION_NAME,
        )
        .insert_one(&personal)
        .await
        .unwrap();
        assert_eq!(
            super::super::personal_erasure_service::status(&db, &user)
                .await
                .unwrap()
                .unwrap()
                .status,
            crate::models::personal_erasure::ErasureStatus::Complete
        );
        // Simulate discovery of a historical destination while its credentials
        // are unavailable. Prior completion must reopen and remain visible.
        db.collection::<Document>(DESTINATIONS).insert_one(doc! {"_id":"older-project","api_host":"https://old.example.test","project_id":"456"}).await.unwrap();
        // The personal status reader must invalidate immediately, before the
        // telemetry worker notices the new registry generation.
        let reopened = super::super::personal_erasure_service::status(&db, &user)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            reopened.status,
            crate::models::personal_erasure::ErasureStatus::Pending
        );
        assert!(reopened.completed_at.is_none());
        assert_eq!(
            reopened.domains[0].status,
            crate::models::personal_erasure::DomainStatus::Pending
        );
        assert!(completion_evidence(&db, &user).await.unwrap().is_none());
        due(&db, &root).await;
        drain_once(&db, &clients).await.unwrap();
        let older = job_id(&user, Some("older-project"));
        assert_eq!(
            jobs(&db)
                .find_one(doc! {"_id":&older})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "operator_action"
        );
        due(&db, &root).await;
        drain_once(&db, &clients).await.unwrap();
        assert_ne!(
            jobs(&db)
                .find_one(doc! {"_id":&root})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "completed"
        );
        operator_reconcile(
            &db,
            &older,
            None,
            "local-test-operator",
            "evidence://no-historical-data",
            Some("Verified historical event and recording inventory is empty"),
        )
        .await
        .unwrap();
        drain_once(&db, &clients).await.unwrap();
        let attested = jobs(&db)
            .find_one(doc! {"_id":&older})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            attested.get_str("completion_basis").unwrap(),
            "operator_attested_no_data"
        );
        // A preexisting completed child with no parent is discovered by a
        // resumable bounded walk, rather than omitted from future destinations.
        let legacy_user = uuid::Uuid::new_v4().to_string();
        let mut legacy = TelemetryErasureJob::new(&legacy_user);
        legacy.destination_id = Some("historical-project".into());
        legacy.status = crate::models::telemetry_erasure_job::TelemetryErasureStatus::Completed;
        legacy.completion_verified_at = Some(chrono::Utc::now());
        jobs(&db)
            .insert_one(bson::to_document(&legacy).unwrap())
            .await
            .unwrap();
        for _ in 0..4 {
            backfill_intents(&db).await.unwrap();
        }
        assert!(
            jobs(&db)
                .find_one(doc! {"_id":job_id(&legacy_user,None)})
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn missing_person_can_be_bound_by_audited_operator_evidence_and_resumed() {
        let db = connect_transaction_test_database("erasure_operator").await;
        let (clients, server) = test_support::clients().await;
        register_destinations(&db, &clients).await.unwrap();
        let user = uuid::Uuid::new_v4().to_string();
        let root = ensure_job(&db, &user, None).await.unwrap();
        server.state.missing_person.store(true, Ordering::SeqCst);
        drain_once(&db, &clients).await.unwrap();
        let child = job_id(&user, Some("historical-project"));
        operator_reconcile(
            &db,
            &child,
            Some(test_support::PERSON),
            "operator",
            "evidence://historical-person",
            None,
        )
        .await
        .unwrap();
        drain_once(&db, &clients).await.unwrap();
        assert_eq!(server.state.deletes.load(Ordering::SeqCst), 1);
        server.state.verified.store(true, Ordering::SeqCst);
        due(&db, &root).await;
        drain_once(&db, &clients).await.unwrap();
        assert_eq!(
            jobs(&db)
                .find_one(doc! {"_id":&child})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "completed"
        );
        assert_eq!(
            db.collection::<Document>("telemetry_erasure_reconciliations")
                .count_documents(doc! {"job_id":&child,"action":"bind_person"})
                .await
                .unwrap(),
            1
        );
        assert!(matches!(
            operator_reconcile(&db, &child, None, "operator", "evidence", Some("no data")).await,
            Err(AppError::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn no_data_attestation_waits_for_current_transport_and_provider_drain() {
        use super::super::{
            analytics_consent_service as consent, analytics_dispatch_service as dispatch,
        };
        let db = connect_transaction_test_database("erasure_no_data_drain").await;
        let user = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>("users")
            .insert_one(crate::test_utils::test_user(
                &user,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let permission = consent::set(&db, &user, true).await.unwrap();
        // `begin` is a majority-write transaction bounded by its production
        // 250 ms deadline; require a responsive server before measuring it.
        crate::test_utils::await_responsive_majority_io(&db).await;
        let held = dispatch::begin(&db, &user, &permission.revision)
            .await
            .unwrap();
        let child = ensure_job(&db, &user, Some("historical-project"))
            .await
            .unwrap();
        jobs(&db)
            .update_one(
                doc! {"_id":&child},
                doc! {"$set":{"status":"operator_action"}},
            )
            .await
            .unwrap();
        db.collection::<Document>("users")
            .delete_one(doc! {"_id":&user})
            .await
            .unwrap();
        for transport_settled in [false, true] {
            if transport_settled {
                dispatch::settle(&db, &held, true).await.unwrap();
            }
            assert!(matches!(
                operator_reconcile(
                    &db,
                    &child,
                    None,
                    "operator",
                    "fixture://no-data",
                    Some("vendor reviewed")
                )
                .await,
                Err(AppError::Conflict(_))
            ));
            assert_eq!(
                db.collection::<Document>("telemetry_erasure_reconciliations")
                    .count_documents(doc! {"job_id":&child})
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                jobs(&db)
                    .find_one(doc! {"_id":&child})
                    .await
                    .unwrap()
                    .unwrap()
                    .get_str("status")
                    .unwrap(),
                "operator_action"
            );
        }
        dispatch::attest_account_drained(&db, &user, "operator", "fixture://provider-drained")
            .await
            .unwrap();
        operator_reconcile(
            &db,
            &child,
            None,
            "operator",
            "fixture://no-data",
            Some("vendor reviewed after processing drained"),
        )
        .await
        .unwrap();
        let completed = jobs(&db)
            .find_one(doc! {"_id":&child})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completed.get_str("status").unwrap(), "completed");
        assert_eq!(
            completed.get_str("completion_dispatch_generation").unwrap(),
            held
        );
        // A malformed or newly changed generation cannot reuse old evidence.
        db.collection::<Document>(dispatch::ACCOUNTS)
            .update_one(doc! {"_id":&user}, doc! {"$set":{"generation":"successor"}})
            .await
            .unwrap();
        jobs(&db)
            .update_one(
                doc! {"_id":&child},
                doc! {"$set":{"status":"operator_action"}},
            )
            .await
            .unwrap();
        assert!(matches!(
            operator_reconcile(
                &db,
                &child,
                None,
                "operator",
                "fixture://stale-evidence",
                Some("must reject")
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn unresolved_destination_is_rejected_before_registry_or_child_write() {
        let db = connect_transaction_test_database("erasure_reserved_id").await;
        let user = uuid::Uuid::new_v4().to_string();
        let parent = ensure_job(&db, &user, None).await.unwrap();
        assert!(ensure_job(&db, &user, Some("unresolved")).await.is_err());
        let (mut clients, _server) = test_support::clients().await;
        let mut destination = clients.0.remove("historical-project").unwrap();
        destination.id = "unresolved".into();
        clients.0.insert("unresolved".into(), destination);
        assert!(register_destinations(&db, &clients).await.is_err());
        assert_eq!(
            db.collection::<Document>(DESTINATIONS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(jobs(&db).count_documents(doc! {}).await.unwrap(), 1);
        assert!(
            jobs(&db)
                .find_one(doc! {"_id":parent,"destination_id":null})
                .await
                .unwrap()
                .is_some()
        );
        // Existing invalid registry data is surfaced, never interpreted as a child.
        db.collection::<Document>(DESTINATIONS)
            .insert_one(
                doc! {"_id":"unresolved","api_host":"https://example.test","project_id":"1"},
            )
            .await
            .unwrap();
        assert!(inventory(&db).await.is_err());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn held_capture_across_deletion_settles_before_vendor_delete() {
        use crate::{
            models::user::{User, UserType},
            telemetry::{TelemetryClient, TelemetryContext, TelemetryEvent},
            test_utils::{test_app_config, test_user},
        };
        let db = connect_transaction_test_database("erasure_capture_drain").await;
        let user = uuid::Uuid::new_v4().to_string();
        db.collection::<User>("users")
            .insert_one(test_user(&user, UserType::Person))
            .await
            .unwrap();
        super::super::analytics_consent_service::set(&db, &user, true)
            .await
            .unwrap();
        let (clients, server) = test_support::clients().await;
        register_destinations(&db, &clients).await.unwrap();
        server.state.hold_capture.store(true, Ordering::SeqCst);
        let mut config = test_app_config();
        config.telemetry_dsn = Some("test-ingest".into());
        config.telemetry_host = Some(clients.0.values().next().unwrap().api_host.clone());
        let telemetry = TelemetryClient::from_config(&config, Some(db.clone())).unwrap();
        telemetry
            .track(
                &user,
                TelemetryEvent::KeyCreated {
                    source: "catalog".into(),
                    catalog_slug: None,
                    has_node_binding: false,
                },
                &TelemetryContext {
                    surface: "ui",
                    client_version: None,
                    analytics_allowed: true,
                    analytics_account: Some(user.clone()),
                },
                None,
            )
            .await;
        tokio::time::timeout(
            Duration::from_secs(1),
            server.state.capture_started.notified(),
        )
        .await
        .unwrap();
        let root = ensure_job(&db, &user, None).await.unwrap();
        db.collection::<Document>("users")
            .delete_one(doc! {"_id":&user})
            .await
            .unwrap();
        drain_once(&db, &clients).await.unwrap();
        assert_eq!(
            server.state.deletes.load(Ordering::SeqCst),
            0,
            "DELETE raced an in-flight capture"
        );
        assert_ne!(
            jobs(&db)
                .find_one(doc! {"_id":&root})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "completed"
        );
        server.state.capture_release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), async {
            while super::super::analytics_dispatch_service::has_unsettled(&db, &user)
                .await
                .unwrap()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            super::super::analytics_dispatch_service::needs_provider_drain(&db, &user)
                .await
                .unwrap()
        );
        assert_eq!(
            db.collection::<Document>(super::super::analytics_dispatch_service::COLLECTION)
                .count_documents(doc! {"user_id":&user})
                .await
                .unwrap(),
            0,
            "accepted event rows must not accumulate"
        );
        super::super::analytics_dispatch_service::attest_account_drained(
            &db,
            &user,
            "fixture-operator",
            "fixture://capture-processed-before-acknowledgement",
        )
        .await
        .unwrap();
        due(&db, &root).await;
        drain_once(&db, &clients).await.unwrap();
        assert_eq!(server.state.deletes.load(Ordering::SeqCst), 1);
        assert!(
            server.state.missing_person.load(Ordering::SeqCst),
            "late capture left a recreated person"
        );
        server.state.verified.store(true, Ordering::SeqCst);
        due(&db, &root).await;
        drain_once(&db, &clients).await.unwrap();
        let child = job_id(&user, Some("historical-project"));
        assert_eq!(
            jobs(&db)
                .find_one(doc! {"_id":&child})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "completed"
        );
    }
}

#[cfg(test)]
mod durability_tests {
    use super::*;
    #[tokio::test]
    async fn weak_inherited_options_cannot_weaken_registry_dispatch_or_completion_evidence() {
        use mongodb::{
            event::{EventHandler, command::CommandEvent},
            options::{DatabaseOptions, ReadPreference, SelectionCriteria},
        };
        use std::sync::{Arc, Mutex};
        let commands = Arc::new(Mutex::new(Vec::<Document>::new()));
        let observed = commands.clone();
        let db = crate::test_utils::connect_replica_test_database_with_command_handler(
            "telemetry_erasure_durable",
            EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event {
                    observed.lock().unwrap().push(event.command);
                }
            }),
        )
        .await;
        let weak = db.client().database_with_options(
            db.name(),
            DatabaseOptions::builder()
                .read_concern(ReadConcern::local())
                .write_concern(
                    WriteConcern::builder()
                        .w(mongodb::options::Acknowledgment::Nodes(1))
                        .build(),
                )
                .selection_criteria(SelectionCriteria::ReadPreference(
                    ReadPreference::SecondaryPreferred { options: None },
                ))
                .build(),
        );
        let (clients, _server) = crate::telemetry::erasure::test_support::clients().await;
        let user = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>("users").insert_one(doc! {"_id":&user,"is_active":true,"profile_config":{"analytics":{"enabled":true,"revision":"revision"}}}).await.unwrap();
        commands.lock().unwrap().clear();
        register_destinations(&weak, &clients).await.unwrap();
        let dispatch = super::super::analytics_dispatch_service::begin(&weak, &user, "revision")
            .await
            .unwrap();
        assert!(
            super::super::analytics_dispatch_service::has_unsettled(&weak, &user)
                .await
                .unwrap()
        );
        assert!(
            super::super::analytics_dispatch_service::needs_provider_drain(&weak, &user)
                .await
                .unwrap()
        );
        ensure_job(&weak, &user, None).await.unwrap();
        drain_once(&weak, &clients).await.unwrap(); // source still exists: no vendor effect
        super::super::analytics_dispatch_service::settle(&weak, &dispatch, true)
            .await
            .unwrap();
        let recorded = commands.lock().unwrap().clone();
        let registry = recorded
            .iter()
            .find(|c| c.get_str("update").ok() == Some(DESTINATIONS))
            .unwrap();
        assert_eq!(
            registry
                .get_document("writeConcern")
                .unwrap()
                .get_str("w")
                .unwrap(),
            "majority"
        );
        assert!(recorded.iter().any(|c| c.contains_key("commitTransaction")
            && c.get_document("writeConcern").unwrap().get_str("w").ok() == Some("majority")));
        for command in recorded
            .iter()
            .filter(|c| c.contains_key("find") || c.contains_key("aggregate"))
        {
            if command.get_bool("autocommit").ok() == Some(false) {
                continue;
            }
            assert_eq!(
                command
                    .get_document("readConcern")
                    .unwrap()
                    .get_str("level")
                    .unwrap(),
                "majority"
            );
            assert!(
                command
                    .get_document("$readPreference")
                    .ok()
                    .is_none_or(|p| p.get_str("mode").ok() == Some("primary"))
            );
        }
        assert!(
            recorded
                .iter()
                .any(|c| c.get_bool("startTransaction").ok() == Some(true)
                    && c.get_document("readConcern").unwrap().get_str("level").ok()
                        == Some("snapshot"))
        );
        let durable = super::super::transaction_service::durable_database(&weak);
        durable
            .collection::<Document>("users")
            .delete_one(doc! {"_id":&user})
            .await
            .unwrap();
        let (destinations, revision) = inventory(&weak).await.unwrap();
        for destination in destinations {
            ensure_job(&weak, &user, Some(&destination)).await.unwrap();
        }
        let generation = dispatch_generation(&weak, &user).await.unwrap().unwrap();
        durable
            .collection::<Document>(super::super::analytics_dispatch_service::ACCOUNTS)
            .update_one(
                doc! {"_id":&user},
                doc! {"$set":{"provider_drained_generation":&generation}},
            )
            .await
            .unwrap();
        jobs(&weak).update_many(doc! {"user_id":&user},doc! {"$set":{"status":"completed",
            "completion_verified_at":bson::DateTime::now(),"completion_dispatch_generation":&generation,
            "inventory_revision":revision,"completion_basis":"fixture_verified_receipt"}}).await.unwrap();
        commands.lock().unwrap().clear();
        assert!(completion_evidence(&weak, &user).await.unwrap().is_some());
        let proof_commands = commands.lock().unwrap().clone();
        assert!(
            proof_commands
                .iter()
                .any(|command| command.contains_key("commitTransaction")
                    && command
                        .get_document("writeConcern")
                        .unwrap()
                        .get_str("w")
                        .ok()
                        == Some("majority")),
            "successful read-only completion snapshots must commit with majority"
        );
        assert!(
            !proof_commands
                .iter()
                .any(|command| command.contains_key("abortTransaction"))
        );
        assert!(proof_commands.iter().any(|command| {
            command.get_bool("startTransaction").ok() == Some(true)
                && command
                    .get_document("readConcern")
                    .unwrap()
                    .get_str("level")
                    .ok()
                    == Some("snapshot")
        }));
        db.drop().await.unwrap();
    }
}

#[cfg(test)]
mod empty_inventory_tests {
    use super::*;
    #[tokio::test]
    async fn no_history_requires_audited_disposition_and_late_registry_invalidates_it() {
        let db = crate::test_utils::connect_transaction_test_database("erasure_no_history").await;
        crate::test_utils::init_test_audit_chain_hmac_key();
        let user = uuid::Uuid::new_v4().to_string();
        let root = ensure_job(&db, &user, None).await.unwrap();
        drain_once(&db, &ErasureClients::default()).await.unwrap();
        assert!(completion_evidence(&db, &user).await.unwrap().is_none());
        // Transport uncertainty cannot be waived by an inventory attestation.
        db.collection::<Document>(super::super::analytics_dispatch_service::COLLECTION)
            .insert_one(doc! {"_id":"held-dispatch","user_id":&user,"status":"uncertain"})
            .await
            .unwrap();
        assert!(
            operator_reconcile_empty_inventory(
                &db,
                &user,
                "operator",
                "evidence://history",
                "Never configured"
            )
            .await
            .is_err()
        );
        assert_eq!(
            jobs(&db)
                .find_one(doc! {"_id":&root})
                .await
                .unwrap()
                .unwrap()
                .get_str("status")
                .unwrap(),
            "operator_action"
        );
        db.collection::<Document>(super::super::analytics_dispatch_service::COLLECTION)
            .update_one(
                doc! {"_id":"held-dispatch"},
                doc! {"$set":{"status":"drained"}},
            )
            .await
            .unwrap();
        operator_reconcile_empty_inventory(
            &db,
            &user,
            "operator",
            "evidence://history",
            "Verified deployment inventory never configured telemetry",
        )
        .await
        .unwrap();
        assert!(completion_evidence(&db, &user).await.unwrap().is_some());
        assert_eq!(
            db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(
                    doc! {"event_type":"user.account.erasure_empty_vendor_inventory_attested"}
                )
                .await
                .unwrap(),
            1
        );
        db.collection::<Document>(DESTINATIONS)
            .insert_one(
                doc! {"_id":"late-project","api_host":"https://old.example.test","project_id":"7"},
            )
            .await
            .unwrap();
        assert!(completion_evidence(&db, &user).await.unwrap().is_none());
        assert!(
            operator_reconcile_empty_inventory(
                &db,
                &user,
                "operator",
                "evidence://history",
                "Empty current config"
            )
            .await
            .is_err()
        );
        db.drop().await.unwrap();
    }
}
