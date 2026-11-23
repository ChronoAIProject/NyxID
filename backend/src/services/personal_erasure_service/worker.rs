//! Bounded, restartable domain scheduling. Every checkpoint compares the
//! current lease token and server time; an expired worker cannot advance it.
use super::*;
use mongodb::options::ReturnDocument;

const LEASE_SECONDS: i64 = 120;
const RECONCILIATION_PAGE_CAP: usize = 64;
const RECONCILIATION_REMOTE_RETRY_SECONDS: i64 = 5 * 60;
const RECONCILIATION_QUIET_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10 * 60);
const RECONCILIATION_ERROR_BACKOFF_START: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Default)]
struct CompletedReconciliationSchedule {
    next_scan: Option<std::time::Instant>,
    failures: u32,
}

impl CompletedReconciliationSchedule {
    fn due(&self, now: std::time::Instant) -> bool {
        self.next_scan.is_none_or(|next| now >= next)
    }

    fn scanned(&mut self, now: std::time::Instant, found: bool) {
        self.failures = 0;
        self.next_scan = (!found).then_some(now + RECONCILIATION_QUIET_INTERVAL);
    }

    fn failed(&mut self, now: std::time::Instant) {
        let backoff = RECONCILIATION_ERROR_BACKOFF_START
            .saturating_mul(2_u32.saturating_pow(self.failures))
            .min(RECONCILIATION_QUIET_INTERVAL);
        self.next_scan = Some(now + backoff);
        self.failures = self.failures.saturating_add(1);
    }
}

async fn reconcile_completed_if_due(
    state: &crate::AppState,
    schedule: &mut CompletedReconciliationSchedule,
    now: std::time::Instant,
) -> AppResult<()> {
    if schedule.due(now) {
        let found = match reconcile_completed_once(state).await {
            Ok(found) => found,
            Err(error) => {
                schedule.failed(now);
                return Err(error);
            }
        };
        schedule.scanned(now, found);
    }
    Ok(())
}

async fn claim(db: &Database) -> AppResult<Option<PersonalErasureJob>> {
    let durable = super::super::transaction_service::durable_database(db);
    let db = &durable;
    Ok(db.collection::<PersonalErasureJob>(JOBS).find_one_and_update(
        doc! {"inventory_version":INVENTORY_VERSION,"status":{"$in":["accepted","pending"]},
            "$expr":{"$and":[
                {"$lte":[{"$ifNull":["$lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]},
                {"$lte":[{"$ifNull":["$retry_at",bson::DateTime::from_millis(0)]},"$$NOW"]}
            ]}},
        vec![doc! {"$set":{"status":"pending","lease_token":uuid::Uuid::new_v4().to_string(),
            "updated_at":"$$NOW","lease_expires_at":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":LEASE_SECONDS}}}}],
    ).sort(doc! {"updated_at":1,"_id":1}).return_document(ReturnDocument::After)
        .write_concern(WriteConcern::majority()).await?)
}

/// Claim one already-completed job whose local pass predates the current
/// inventory. The account tombstone remains in place, so new writers stay
/// fenced while this repair runs.
async fn claim_completed_reconciliation(db: &Database) -> AppResult<Option<PersonalErasureJob>> {
    let durable = super::super::transaction_service::durable_database(db);
    let db = &durable;
    Ok(db
        .collection::<PersonalErasureJob>(JOBS)
        .find_one_and_update(
            doc! {
                "inventory_version": INVENTORY_VERSION,
                "status": "complete",
                "$expr": {"$and": [
                    {"$lt": [{"$ifNull": ["$local_inventory_revision", 0]}, super::local::LOCAL_INVENTORY_REVISION]},
                    {"$lte": [{"$ifNull": ["$lease_expires_at", bson::DateTime::from_millis(0)]}, "$$NOW"]},
                    {"$lte": [{"$ifNull": ["$retry_at", bson::DateTime::from_millis(0)]}, "$$NOW"]}
                ]}
            },
            vec![doc! {"$set": {
                "lease_token": uuid::Uuid::new_v4().to_string(),
                "updated_at": "$$NOW",
                "lease_expires_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": LEASE_SECONDS}}
            }}],
        )
        .sort(doc! {"updated_at": 1, "_id": 1})
        .return_document(ReturnDocument::After)
        .write_concern(WriteConcern::majority())
        .await?)
}

async fn reconcile_completed_once(state: &crate::AppState) -> AppResult<bool> {
    let durable = super::super::transaction_service::durable_database(&state.db);
    let db = &durable;
    let Some(job) = claim_completed_reconciliation(db).await? else {
        return Ok(false);
    };
    let lease = job
        .lease_token
        .as_deref()
        .ok_or_else(|| AppError::Conflict("Erasure reconciliation lease unavailable".into()))?;
    if super::local::remote_cleanup_pending(db, &job.user_id).await? {
        return defer_remote_reconciliation(db, &job, lease).await;
    }
    // Observing a clear gate ends the persisted blocked episode, even if the
    // bounded inventory needs more than one lease to finish.
    db.collection::<Document>(JOBS)
        .update_one(
            doc! {"_id": &job.id, "status": "complete", "lease_token": lease,
            "$expr": {"$gt": ["$lease_expires_at", "$$NOW"]}},
            doc! {"$unset": {"reconciliation_remote_blocked": ""}},
        )
        .write_concern(WriteConcern::majority())
        .await?;
    let mut clean = false;
    for _ in 0..RECONCILIATION_PAGE_CAP {
        // Do not continue a repair after another worker has taken the lease.
        // The local pass is itself bounded; this check fences the next page
        // and, most importantly, the eventual revision stamp.
        let lease_alive = db
            .collection::<Document>(JOBS)
            .find_one(doc! {
                "_id": &job.id,
                "status": "complete",
                "lease_token": lease,
                "$expr": {"$gt": ["$lease_expires_at", "$$NOW"]},
            })
            .projection(doc! {"_id": 1})
            .await?
            .is_some();
        if !lease_alive {
            return Ok(true);
        }
        if super::local::pass(state, &job).await? {
            clean = true;
            break;
        }
        if super::local::remote_cleanup_pending(db, &job.user_id).await? {
            return defer_remote_reconciliation(db, &job, lease).await;
        }
    }
    if !clean {
        // Release the lease; the next tick can continue the bounded pass.
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id": &job.id, "status": "complete", "lease_token": lease},
                vec![doc! {"$set": {"lease_token": bson::Bson::Null, "lease_expires_at": bson::Bson::Null, "updated_at": "$$NOW"}}],
            )
            .write_concern(WriteConcern::majority())
            .await?;
        return Ok(true);
    }
    // The lease token is part of the stamp fence. A stale holder can never
    // advance a job after another replica has claimed the repair.
    stamp_reconciled(db, &job.id, lease).await?;
    // A lost lease is still a found candidate, not an empty repair scan.
    Ok(true)
}

async fn defer_remote_reconciliation(
    db: &Database,
    job: &PersonalErasureJob,
    lease: &str,
) -> AppResult<bool> {
    // The gate is checked first. Still allow one bounded cancellation of
    // unstarted/confirmed Telegram requests, which can itself clear that gate.
    // This performs no inventory pass and cannot churn through the page cap.
    super::local::settle_telegram_requests(db, &job.user_id).await?;
    let before = db
        .collection::<Document>(JOBS)
        .find_one_and_update(
            doc! {"_id": &job.id, "status": "complete", "lease_token": lease,
                "$expr": {"$gt": ["$lease_expires_at", "$$NOW"]}},
            vec![doc! {"$set": {
                "reconciliation_remote_blocked": true,
                "retry_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": RECONCILIATION_REMOTE_RETRY_SECONDS}},
                "lease_token": bson::Bson::Null,
                "lease_expires_at": bson::Bson::Null,
                "updated_at": "$$NOW",
            }}],
        )
        .projection(doc! {"reconciliation_remote_blocked": 1})
        .return_document(ReturnDocument::Before)
        .write_concern(WriteConcern::majority())
        .await?;
    if let Some(before) = &before
        && before.get_bool("reconciliation_remote_blocked") != Ok(true)
    {
        tracing::warn!(job_id = %job.id, "Completed erasure inventory repair waits for remote cleanup");
    }
    Ok(true)
}

async fn stamp_reconciled(db: &Database, job_id: &str, lease: &str) -> AppResult<bool> {
    let result = db
        .collection::<Document>(JOBS)
        .update_one(
            doc! {"_id": job_id, "status": "complete", "lease_token": lease, "$expr": {"$gt": ["$lease_expires_at", "$$NOW"]}},
            vec![doc! {"$set": {"local_inventory_revision": super::local::LOCAL_INVENTORY_REVISION, "lease_token": bson::Bson::Null, "lease_expires_at": bson::Bson::Null, "updated_at": "$$NOW"}}, doc! {"$unset": "reconciliation_remote_blocked"}],
        )
        .write_concern(WriteConcern::majority())
        .await?;
    Ok(result.matched_count == 1)
}

fn lease_filter(job: &PersonalErasureJob) -> AppResult<Document> {
    let lease = job
        .lease_token
        .as_deref()
        .ok_or_else(|| AppError::Conflict("Erasure lease unavailable".into()))?;
    Ok(
        doc! {"_id":&job.id,"user_id":&job.user_id,"inventory_version":INVENTORY_VERSION,"lease_token":lease,
        "$expr":{"$gt":["$lease_expires_at","$$NOW"]}},
    )
}

#[derive(Clone)]
enum Outcome {
    Complete,
    CompleteWithException(&'static str),
    CompleteWithEvidence(String),
    Progress(&'static str, String),
    Pending(&'static str),
    RetryableFailure(&'static str),
    OperatorAction(&'static str),
}

async fn checkpoint(
    db: &Database,
    job: &PersonalErasureJob,
    index: usize,
    outcome: Outcome,
) -> AppResult<bool> {
    let durable = super::super::transaction_service::durable_database(db);
    let db = &durable;
    let mut domains = job.domains.clone();
    let domain = domains
        .get_mut(index)
        .ok_or_else(|| AppError::Internal("Erasure domain is unavailable".into()))?;
    // Claim.updated_at and the persisted wait date both originate on MongoDB.
    let remote_wait = matches!(outcome, Outcome::Pending("remote_cleanup_pending"));
    let outcome = if remote_wait
        && domain.remote_wait_started_at.is_some_and(|started| {
            job.updated_at.signed_duration_since(started) >= chrono::Duration::hours(24)
        }) {
        Outcome::OperatorAction("remote_cleanup_wait_exceeded")
    } else {
        outcome
    };
    if !remote_wait && !matches!(outcome, Outcome::RetryableFailure(_)) {
        domain.remote_wait_started_at = None;
    }
    let start_remote_wait = remote_wait && domain.remote_wait_started_at.is_none();
    let outcome = match outcome {
        Outcome::RetryableFailure(reason) => {
            domain.consecutive_failures = domain.consecutive_failures.saturating_add(1);
            if domain.consecutive_failures >= 8 {
                Outcome::OperatorAction(reason)
            } else {
                Outcome::Pending(reason)
            }
        }
        other => {
            domain.consecutive_failures = 0;
            other
        }
    };
    let retry_seconds = if domain.consecutive_failures == 0 {
        1_i64
    } else {
        (5_i64 * (1_i64 << domain.consecutive_failures.min(6))).min(300)
    };
    match &outcome {
        Outcome::Complete
        | Outcome::CompleteWithException(_)
        | Outcome::CompleteWithEvidence(_) => {
            domain.status = DomainStatus::Complete;
            domain.reason = match &outcome {
                Outcome::CompleteWithException(reason) => Some((*reason).to_owned()),
                _ => None,
            };
            if let Outcome::CompleteWithEvidence(evidence) = &outcome {
                domain.cursor = Some(evidence.clone());
            }
            // The update pipeline stamps this field using MongoDB server time.
            domain.completed_at = None;
        }
        Outcome::Pending(reason) | Outcome::Progress(reason, _) => {
            domain.status = DomainStatus::Pending;
            domain.reason = Some((*reason).to_owned());
            if let Outcome::Progress(_, cursor) = &outcome {
                domain.cursor = Some(cursor.clone());
            }
        }
        Outcome::RetryableFailure(_) => unreachable!("failure classified before checkpoint"),
        Outcome::OperatorAction(reason) => {
            domain.status = DomainStatus::OperatorAction;
            domain.reason = Some((*reason).to_owned());
        }
    }
    if domains
        .iter()
        .all(|domain| domain.status == DomainStatus::Complete)
        && db
            .collection::<Document>(provider::EFFECTS)
            .find_one(doc! {"user_id":&job.user_id,
            "$or":[{"status":{"$ne":"complete"}},{"capability_encrypted":{"$exists":true}}]})
            .await?
            .is_some()
        && let Some(domain) = domains
            .iter_mut()
            .find(|domain| domain.domain == "provider_credentials")
    {
        domain.status = DomainStatus::Pending;
        domain.completed_at = None;
        domain.reason = Some("provider_revocation_requires_reconciliation".into());
    }
    let complete = domains
        .iter()
        .all(|domain| domain.status == DomainStatus::Complete);
    let action = domains
        .iter()
        .any(|domain| domain.status == DomainStatus::OperatorAction);
    let status = if complete {
        "complete"
    } else if action {
        "operator_action"
    } else {
        "pending"
    };
    // Aggregation dotted paths do not index arrays. Build explicit array
    // expressions, interpreting only this server-selected timestamp as code.
    let domain_expressions: Vec<bson::Bson> = domains
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            let literal = bson::to_bson(entry)
                .map_err(|_| AppError::Internal("Erasure checkpoint encoding failed".into()))?;
            Ok(bson::Bson::Document(
                if position == index && entry.status == DomainStatus::Complete {
                    doc! {"$mergeObjects":[{"$literal":literal},{"completed_at":"$$NOW"}]}
                } else if position == index && start_remote_wait {
                    doc! {"$mergeObjects":[{"$literal":literal},{"remote_wait_started_at":"$$NOW"}]}
                } else {
                    doc! {"$literal":literal}
                },
            ))
        })
        .collect::<AppResult<_>>()?;
    let changed = db.collection::<Document>(JOBS).update_one(lease_filter(job)?, vec![
        doc! {"$set":{
            "domains":domain_expressions,
            "status":status,"updated_at":"$$NOW",
            "completed_at":if complete {bson::Bson::String("$$NOW".into())} else {bson::Bson::Null},
            // The inventory revision records the revision that actually ran
            // the local inventory domain. A later domain completing cannot
            // make an old local pass look current during a mixed-fleet rollout.
            "local_inventory_revision": if index < domains.len()
                && domains[index].domain == "local_inventory"
                && domains[index].status == DomainStatus::Complete
            {bson::Bson::Int32(super::local::LOCAL_INVENTORY_REVISION as i32)} else {bson::Bson::Int32(job.local_inventory_revision as i32)},
            "lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,
            "retry_at":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":retry_seconds}},
        }},
    ]).write_concern(WriteConcern::majority()).await?;
    Ok(changed.matched_count == 1)
}

// Select by the current dependency order, including jobs whose retained
// domain array predates an ordering correction.
fn next_domain_index(job: &PersonalErasureJob) -> Option<usize> {
    job.domains
        .iter()
        .enumerate()
        .filter(|(_, domain)| {
            domain.status != DomainStatus::Complete
                || (domain.domain == "local_inventory"
                    && job.local_inventory_revision < super::local::LOCAL_INVENTORY_REVISION)
        })
        .min_by_key(|(_, domain)| {
            DOMAINS
                .iter()
                .position(|name| *name == domain.domain)
                .unwrap_or(usize::MAX)
        })
        .map(|(index, _)| index)
}

/// One claim, one domain and bounded adapter work. Existing lifecycle runners
/// may also help; only durable lifecycle evidence advances a checkpoint.
pub(crate) fn process_one(
    state: &crate::AppState,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<bool>> + Send + '_>> {
    Box::pin(process_one_inner(state))
}

async fn process_one_inner(state: &crate::AppState) -> AppResult<bool> {
    let Some(_work) = state.shutdown.background() else {
        return Ok(false);
    };
    let keys = &state.encryption_keys;
    let durable = super::super::transaction_service::durable_database(&state.db);
    let db = &durable;
    Box::pin(super::vendor::revisit(db)).await?;
    Box::pin(super::issuance::expose_uncertain(db)).await?;
    Box::pin(super::provider::drain_one(db, keys)).await?;
    let Some(job) = claim(db).await? else {
        return Ok(false);
    };
    let release = state.shutdown.register_release(crate::services::shutdown::Release {
        safe_after_stop: false,
        db: db.clone(), collection: JOBS,
        filter: doc! {"_id": &job.id, "lease_token": &job.lease_token, "inventory_version": INVENTORY_VERSION},
        update: doc! {"$set": {"lease_expires_at": null, "lease_token": null}},
    });
    let Some(index) = next_domain_index(&job) else {
        return Err(AppError::Internal(
            "Erasure checkpoint has no pending domain".into(),
        ));
    };
    let result = run_domain(state, db, &job, index).await;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(AppError::Conflict(_)) => {
            Outcome::RetryableFailure("domain_authority_changed_repair_required")
        }
        Err(AppError::Internal(_)) => {
            Outcome::OperatorAction("domain_material_or_configuration_invalid")
        }
        Err(_) => Outcome::RetryableFailure("domain_operation_failed_repair_required"),
    };
    checkpoint(db, &job, index, outcome).await?;
    release.completed();
    Ok(true)
}

async fn run_domain(
    state: &crate::AppState,
    db: &Database,
    job: &PersonalErasureJob,
    index: usize,
) -> AppResult<Outcome> {
    use super::super::personal_data_registry::ErasureDomain;
    let keys = &state.encryption_keys;
    let domain = &job.domains[index];
    // Defend jobs accepted by an earlier writer too. Provider credentials
    // must remain intact until publication reconciliation has settled.
    if !Box::pin(super::learning::clear(db, &job.user_id)).await? {
        return Ok(Outcome::OperatorAction(super::learning::REASON));
    }
    match domain.domain.as_str() {
        "platform" => Ok(
            if Box::pin(
                super::super::platform_personal_owner_lifecycle::retire_batch(
                    db,
                    &job.user_id,
                    true,
                ),
            )
            .await?
            {
                Outcome::Complete
            } else {
                Outcome::Pending("platform_lifecycle_pending")
            },
        ),
        name if name == ErasureDomain::Oracle.execution().0 => Ok(
            if Box::pin(super::super::oracle_pool_service::personal_erasure_pass(
                db,
                &job.user_id,
                &job.requested_by,
            ))
            .await?
            {
                Outcome::Complete
            } else {
                Outcome::Pending("oracle_lifecycle_pending")
            },
        ),
        // Adapters are integrated one domain at a time. Never convert a
        // missing adapter or a scheduled effect into completion evidence.
        "provider_credentials" => Ok(
            match Box::pin(super::provider::pass(db, keys, job)).await? {
                super::provider::Pass::Complete => Outcome::Complete,
                super::provider::Pass::CompleteWithException(reason) => {
                    Outcome::CompleteWithException(reason)
                }
                super::provider::Pass::Pending => Outcome::Pending("provider_revocation_pending"),
                super::provider::Pass::Action(reason) => Outcome::OperatorAction(reason),
            },
        ),
        "vendor" => Ok(
            match Box::pin(super::vendor::pass(db, &job.user_id)).await? {
                super::vendor::Pass::Complete(evidence) => Outcome::CompleteWithEvidence(evidence),
                super::vendor::Pass::Pending => Outcome::Pending("vendor_erasure_pending"),
                super::vendor::Pass::Action => {
                    Outcome::OperatorAction("vendor_reconciliation_required")
                }
            },
        ),
        name if name == ErasureDomain::ChannelBots.execution().0 => {
            Ok(if Box::pin(super::channel::pass(state, job)).await? {
                Outcome::Complete
            } else {
                Outcome::Pending("channel_webhook_cleanup_pending")
            })
        }
        "machine" => Ok(match Box::pin(super::machine::pass(state, job)).await? {
            super::machine::Pass::Complete => Outcome::Complete,
            super::machine::Pass::Retained => {
                Outcome::CompleteWithException("admin_retained_machine_workspace")
            }
            super::machine::Pass::Progress(cursor) => {
                Outcome::Progress("machine_inventory_pending", cursor)
            }
            super::machine::Pass::Pending => Outcome::Pending("machine_lifecycle_pending"),
            super::machine::Pass::Action => {
                Outcome::OperatorAction("machine_lifecycle_reconciliation_required")
            }
        }),
        name if name == ErasureDomain::LocalEvidence.execution().0 => {
            Ok(if Box::pin(super::local::pass(state, job)).await? {
                Outcome::Complete
            } else if super::local::remote_cleanup_pending(&state.db, &job.user_id).await? {
                Outcome::Pending("remote_cleanup_pending")
            } else {
                Outcome::Pending("local_inventory_cleanup_pending")
            })
        }
        name if name == ErasureDomain::Source.execution().0 => {
            Ok(if Box::pin(super::local::source(db, job)).await? {
                Outcome::Complete
            } else if super::local::remote_cleanup_pending(db, &job.user_id).await? {
                Outcome::Pending("remote_cleanup_pending")
            } else {
                Outcome::Pending("source_cleanup_pending")
            })
        }
        _ => Ok(Outcome::OperatorAction("unknown_inventory_domain")),
    }
}

/// The authenticated operator API supplies the user and domain. The job ID is
/// never an access capability. Requeue retries the same durable domain; it
/// cannot attest completion or bypass any lifecycle/ownership guard.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DomainRetryRequest {
    pub domain: String,
    pub job_id: String,
    pub expected_retry_generation: i64,
    pub idempotency_key: String,
}

pub(crate) const REMOTE_CLEANUP_RETRY_MESSAGE: &str = "Erasure domain queued for evidence re-evaluation. Resolve the remaining remote cleanup dependencies. For a node-blocked Stop, bring the node online or, if permanently gone, have an administrator delete it via DELETE /api/v1/admin/nodes/{id}, then retry. Unresolved cleanup immediately returns to operator_action (remote_cleanup_wait_exceeded)";

pub(crate) async fn retry_domain(
    db: &Database,
    actor: &str,
    user: &str,
    request: &DomainRetryRequest,
) -> AppResult<&'static str> {
    let domain = request.domain.as_str();
    let durable = super::super::transaction_service::durable_database(db);
    let db = &durable;
    if !DOMAINS.contains(&domain) {
        return Err(AppError::ValidationError("Unknown erasure domain".into()));
    }
    if request.expected_retry_generation < 0
        || uuid::Uuid::parse_str(&request.job_id).is_err()
        || uuid::Uuid::parse_str(&request.idempotency_key).is_err()
    {
        return Err(AppError::ValidationError(
            "Invalid erasure retry identity or generation".into(),
        ));
    }
    use sha2::{Digest, Sha256};
    let key_hash = hex::encode(Sha256::digest(request.idempotency_key.as_bytes()));
    let request_digest = hex::encode(Sha256::digest(
        serde_json::to_vec(&serde_json::json!({
            "actor":actor,"owner":user,"domain":domain,"job_id":request.job_id,
            "retry_generation":request.expected_retry_generation,
        }))
        .map_err(|_| AppError::Internal("Erasure retry binding failed".into()))?,
    ));
    let context = (db, actor, user, domain, request, &key_hash, &request_digest);
    super::super::transaction_service::run_db_only(db, &context, |tx, context| {
        let &(db, actor, user, domain, request, key_hash, request_digest) = context;
        Box::pin(async move {
    let result: AppResult<&'static str> = async {
        let job = db.collection::<PersonalErasureJob>(JOBS).find_one(doc! {"_id":&request.job_id,"user_id":user}).session(&mut *tx).await?
            .ok_or_else(||AppError::Conflict("Erasure retry job changed".into()))?;
        let previous = job.domains.iter().find(|entry| entry.domain == domain)
            .ok_or_else(||AppError::Conflict("Erasure retry domain changed".into()))?;
        let remote_cleanup_retry = matches!(previous.reason.as_deref(),
            Some("remote_cleanup_wait_exceeded" | "remote_cleanup_verification_required"));
        let retry_message = if remote_cleanup_retry { REMOTE_CLEANUP_RETRY_MESSAGE }
            else { "Erasure domain queued for evidence re-evaluation" };
        if previous.last_retry_key_hash.as_deref() == Some(key_hash) {
            if previous.last_retry_request_digest.as_deref() != Some(request_digest) {
                return Err(AppError::Conflict("Erasure retry idempotency key was reused".into()));
            }
            return Ok(retry_message);
        }
        if job.status != ErasureStatus::OperatorAction || previous.status != DomainStatus::OperatorAction
            || previous.retry_generation != request.expected_retry_generation {
            return Err(AppError::Conflict("Erasure retry round changed; reload current evidence".into()));
        }
        if domain == "machine" { super::machine::retry_in_session(db, &job, &mut *tx).await?; }
        let mut domains = job.domains;
        let checkpoint = domains.iter_mut().find(|entry|entry.domain == domain && entry.status == DomainStatus::OperatorAction)
            .ok_or_else(||AppError::Conflict("This erasure domain does not await operator action".into()))?;
        checkpoint.retry_generation = checkpoint.retry_generation.checked_add(1)
            .ok_or_else(||AppError::Conflict("Erasure retry generation exhausted".into()))?;
        checkpoint.last_retry_key_hash = Some(key_hash.clone());
        checkpoint.last_retry_request_digest = Some(request_digest.clone());
        checkpoint.status = DomainStatus::Pending;
        checkpoint.reason = remote_cleanup_retry.then(|| "remote_cleanup_verification_required".into());
        checkpoint.consecutive_failures = 0;
        if domain == "provider_credentials" {
            db.collection::<Document>(super::provider::EFFECTS).update_many(
                doc! {"user_id":user,"kind":"oauth_token_revocation","single_attempt":false,"status":"operator_action","capability_encrypted":{"$type":"binData"},
                    "$expr":{"$lte":[{"$ifNull":["$lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]}},
                vec![doc! {"$set":{"status":"pending","reason":bson::Bson::Null,"lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,"attempts":0_i64,"retry_generation":{"$add":[{"$ifNull":["$retry_generation",0_i64]},1_i64]},"retry_at":"$$NOW","updated_at":"$$NOW"}}],
            ).session(&mut *tx).await?;
        }
        db.collection::<Document>(JOBS).update_one(doc! {"_id":job.id,"status":"operator_action"},vec![doc! {"$set":{
            "domains":{"$literal":bson::to_bson(&domains).map_err(|_|AppError::Internal("Erasure checkpoint encoding failed".into()))?},
            "status":"pending","retry_at":bson::Bson::Null,"updated_at":"$$NOW",
        }}]).session(&mut *tx).await?;
        let key = super::super::audit_service::audit_chain_hmac_key()
            .ok_or_else(||AppError::Internal("Audit chain key is not initialized".into()))?;
        super::super::audit_service::log_actor_event_in_session(db, &mut *tx, key,
            &super::super::audit_service::AuditActor {user_id:actor.to_owned(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},
            "user.account.erasure_domain_retry",serde_json::json!({"target_user_id":user,"domain":domain,"job_id":request.job_id,"expected_retry_generation":request.expected_retry_generation,"inventory_version":INVENTORY_VERSION})).await?;
        Ok(retry_message)
    }.await;
    result
        })
    }).await
}

pub(crate) fn spawn_worker(state: crate::AppState) -> tokio::task::JoinHandle<()> {
    state.shutdown.clone().spawn_worker(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut reconciliation = CompletedReconciliationSchedule::default();
        loop {
            let Some(_work) = state.shutdown.tick(&mut timer).await else {
                break;
            };
            if super::preparation::process_one(&state.db, &state.encryption_keys)
                .await
                .is_err()
            {
                tracing::warn!("Personal erasure dependency preparation remains pending");
            }
            if process_one(&state).await.is_err() {
                tracing::warn!("Personal erasure pass remains pending; durable lease will retry");
            }
            if reconcile_completed_if_due(&state, &mut reconciliation, std::time::Instant::now())
                .await
                .is_err()
            {
                tracing::warn!(
                    "Completed personal erasure inventory reconciliation remains pending"
                );
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_reconciliation_quiet_interval_starts_only_after_empty_scan() {
        let start = std::time::Instant::now();
        let mut schedule = CompletedReconciliationSchedule::default();
        assert!(schedule.due(start));
        schedule.scanned(start, true);
        assert!(schedule.due(start + std::time::Duration::from_secs(1)));
        schedule.scanned(start, false);
        assert!(
            !schedule
                .due(start + RECONCILIATION_QUIET_INTERVAL - std::time::Duration::from_nanos(1))
        );
        assert!(schedule.due(start + RECONCILIATION_QUIET_INTERVAL));
        schedule.scanned(start + RECONCILIATION_QUIET_INTERVAL, true);
        assert!(
            schedule.due(start + RECONCILIATION_QUIET_INTERVAL + std::time::Duration::from_secs(1))
        );
        assert!(CompletedReconciliationSchedule::default().due(start));
    }

    #[test]
    fn completed_reconciliation_errors_back_off_and_success_resets_the_streak() {
        let mut now = std::time::Instant::now();
        let mut schedule = CompletedReconciliationSchedule::default();
        let mut attempts = 0;
        for seconds in [30, 60, 120, 240, 480, 600, 600, 600] {
            assert!(schedule.due(now));
            let before = attempts;
            for elapsed in 0..seconds {
                let tick = now + std::time::Duration::from_secs(elapsed);
                if schedule.due(tick) {
                    // Each attempted failure produces one warning in the worker loop.
                    attempts += 1;
                    schedule.failed(tick);
                }
                assert!(!schedule.due(tick));
            }
            assert_eq!(attempts, before + 1);
            now += std::time::Duration::from_secs(seconds);
            assert!(!schedule.due(now - std::time::Duration::from_nanos(1)));
            assert!(schedule.due(now));
        }

        schedule.failures = u32::MAX;
        schedule.failed(now);
        assert_eq!(schedule.failures, u32::MAX);
        assert_eq!(
            schedule.next_scan,
            Some(now + RECONCILIATION_QUIET_INTERVAL)
        );

        schedule.scanned(now, false);
        assert_eq!(schedule.failures, 0);
        now += RECONCILIATION_QUIET_INTERVAL;
        assert!(!schedule.due(now - std::time::Duration::from_nanos(1)));
        assert!(schedule.due(now));
        schedule.failed(now);
        assert_eq!(schedule.failures, 1);
        now += RECONCILIATION_ERROR_BACKOFF_START;
        assert!(!schedule.due(now - std::time::Duration::from_nanos(1)));
        assert!(schedule.due(now));

        schedule.scanned(now, true);
        assert_eq!(schedule.failures, 0);
        assert!(schedule.due(now));
    }

    async fn cleanup_revision(db: &Database, user: &str) -> i64 {
        db.collection::<Document>(crate::services::erasure_admission::COLLECTION)
            .find_one(doc! {"_id": user})
            .await
            .unwrap()
            .unwrap()
            .get_i64("cleanup_revision")
            .unwrap_or(0)
    }

    async fn retry_now(db: &Database, id: &str) {
        db.collection::<Document>(JOBS)
            .update_one(doc! {"_id": id}, vec![doc! {"$set": {"retry_at": "$$NOW"}}])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn completed_reconciliation_backs_off_remote_blocker_and_logs_once_per_episode() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use tracing::instrument::WithSubscriber;
        use tracing_subscriber::prelude::*;
        struct BlockedLogs(Arc<AtomicUsize>);
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for BlockedLogs {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _: tracing_subscriber::layer::Context<'_, S>,
            ) {
                if event.metadata().target() == module_path!().trim_end_matches("::tests")
                    && event.metadata().fields().field("job_id").is_some()
                {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        let logs = Arc::new(AtomicUsize::new(0));
        let subscriber =
            tracing::Dispatch::new(tracing_subscriber::registry().with(BlockedLogs(logs.clone())));
        let (db, user) = super::super::tests::fixture().await;
        let accepted = super::super::accept(&db, &user, &user, None).await.unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        db.collection::<Document>(JOBS).update_one(doc! {"_id": &accepted.id},
            vec![doc! {"$set": {"status": "complete", "local_inventory_revision": 0_i32, "retry_at": "$$NOW"}}])
            .await.unwrap();
        db.collection::<Document>("platform_bot_cleanup")
            .insert_one(doc! {"_id": "remote-blocker", "owner_user_id": &user, "completed": false})
            .await
            .unwrap();
        let before = cleanup_revision(&db, &user).await;
        assert!(
            reconcile_completed_once(&state)
                .with_subscriber(subscriber.clone())
                .await
                .unwrap()
        );
        assert_eq!(cleanup_revision(&db, &user).await - before, 1);
        let blocked = db
            .collection::<Document>(JOBS)
            .find_one(doc! {"_id": &accepted.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(blocked.get_i32("local_inventory_revision"), Ok(0));
        assert_eq!(blocked.get_bool("reconciliation_remote_blocked"), Ok(true));
        assert_eq!(blocked.get("lease_token"), Some(&bson::Bson::Null));
        assert_eq!(blocked.get("lease_expires_at"), Some(&bson::Bson::Null));
        assert_eq!(
            blocked.get_datetime("retry_at").unwrap().timestamp_millis()
                - blocked
                    .get_datetime("updated_at")
                    .unwrap()
                    .timestamp_millis(),
            RECONCILIATION_REMOTE_RETRY_SECONDS * 1000
        );
        assert_eq!(logs.load(Ordering::SeqCst), 1);
        assert!(
            !reconcile_completed_once(&state)
                .with_subscriber(subscriber.clone())
                .await
                .unwrap()
        );
        assert_eq!(cleanup_revision(&db, &user).await - before, 1);
        retry_now(&db, &accepted.id).await;
        assert!(
            reconcile_completed_once(&state)
                .with_subscriber(subscriber.clone())
                .await
                .unwrap()
        );
        assert_eq!(cleanup_revision(&db, &user).await - before, 2);
        assert_eq!(logs.load(Ordering::SeqCst), 1);
        db.collection::<Document>("platform_bot_cleanup")
            .update_one(
                doc! {"_id": "remote-blocker"},
                doc! {"$set": {"completed": true}},
            )
            .await
            .unwrap();
        assert!(!reconcile_completed_once(&state).await.unwrap());
        retry_now(&db, &accepted.id).await;
        assert!(
            reconcile_completed_once(&state)
                .with_subscriber(subscriber.clone())
                .await
                .unwrap()
        );
        let repaired = super::super::status(&db, &user).await.unwrap().unwrap();
        assert_eq!(
            repaired.local_inventory_revision,
            super::local::LOCAL_INVENTORY_REVISION
        );
        assert!(
            !db.collection::<Document>(JOBS)
                .find_one(doc! {"_id": &accepted.id})
                .await
                .unwrap()
                .unwrap()
                .contains_key("reconciliation_remote_blocked")
        );
        // A later blocked episode emits a new event, including across workers.
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id": &accepted.id},
                doc! {"$set": {"local_inventory_revision": 0_i32}},
            )
            .await
            .unwrap();
        db.collection::<Document>("platform_bot_cleanup")
            .update_one(
                doc! {"_id": "remote-blocker"},
                doc! {"$set": {"completed": false}},
            )
            .await
            .unwrap();
        retry_now(&db, &accepted.id).await;
        assert!(
            reconcile_completed_once(&state)
                .with_subscriber(subscriber)
                .await
                .unwrap()
        );
        assert_eq!(logs.load(Ordering::SeqCst), 2);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn completed_reconciliation_cancels_unstarted_telegram_before_retry() {
        let (db, user) = super::super::tests::fixture().await;
        let accepted = super::super::accept(&db, &user, &user, None).await.unwrap();
        db.collection::<Document>(JOBS).update_one(doc! {"_id": &accepted.id},
            vec![doc! {"$set": {"status": "complete", "local_inventory_revision": 0_i32, "retry_at": "$$NOW"}}]).await.unwrap();
        db.collection::<Document>("telegram_bot_requests").insert_one(
            doc! {"_id": "unstarted-request", "owner_user_id": &user, "active": true, "status": "waiting_telegram", "revision": 0_i64})
            .await.unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        assert!(reconcile_completed_once(&state).await.unwrap());
        let cancelled = db
            .collection::<Document>("telegram_bot_requests")
            .find_one(doc! {"_id": "unstarted-request"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cancelled.get_bool("active"), Ok(false));
        assert_eq!(cancelled.get_str("status"), Ok("cancelled"));
        assert!(!reconcile_completed_once(&state).await.unwrap());
        retry_now(&db, &accepted.id).await;
        assert!(reconcile_completed_once(&state).await.unwrap());
        assert_eq!(
            super::super::status(&db, &user)
                .await
                .unwrap()
                .unwrap()
                .local_inventory_revision,
            super::local::LOCAL_INVENTORY_REVISION
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn pending_remote_cleanup_runs_one_pass_and_honors_checkpoint_retry() {
        let (db, user) = super::super::tests::fixture().await;
        let mut accepted = super::super::accept(&db, &user, &user, None).await.unwrap();
        for domain in &mut accepted.domains {
            if domain.domain == "local_inventory" {
                break;
            }
            domain.status = DomainStatus::Complete;
        }
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id": &accepted.id},
                doc! {"$set": {"domains": bson::to_bson(&accepted.domains).unwrap()}},
            )
            .await
            .unwrap();
        db.collection::<Document>("platform_bot_cleanup")
            .insert_one(doc! {"_id": "remote-blocker", "owner_user_id": &user, "completed": false})
            .await
            .unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        let before = cleanup_revision(&db, &user).await;
        assert!(process_one(&state).await.unwrap());
        assert_eq!(cleanup_revision(&db, &user).await - before, 1);
        let blocked = super::super::status(&db, &user).await.unwrap().unwrap();
        let local = blocked
            .domains
            .iter()
            .find(|d| d.domain == "local_inventory")
            .unwrap();
        assert_eq!(local.status, DomainStatus::Pending);
        assert_eq!(local.reason.as_deref(), Some("remote_cleanup_pending"));
        assert!(local.remote_wait_started_at.is_some());
        assert_eq!(
            blocked.retry_at.unwrap() - blocked.updated_at,
            chrono::Duration::seconds(1)
        );
        assert!(blocked.lease_token.is_none() && blocked.lease_expires_at.is_none());
        assert_eq!(blocked.local_inventory_revision, 0);
        assert!(!process_one(&state).await.unwrap());
        assert_eq!(cleanup_revision(&db, &user).await - before, 1);
        db.collection::<Document>("platform_bot_cleanup")
            .update_one(
                doc! {"_id": "remote-blocker"},
                doc! {"$set": {"completed": true}},
            )
            .await
            .unwrap();
        retry_now(&db, &accepted.id).await;
        assert!(process_one(&state).await.unwrap());
        let repaired = super::super::status(&db, &user).await.unwrap().unwrap();
        assert_eq!(
            repaired.local_inventory_revision,
            super::local::LOCAL_INVENTORY_REVISION
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn process_start_reconciles_immediately_and_current_completion_skips_quiet_window() {
        let (db, user) = super::super::tests::fixture().await;
        let accepted = super::super::accept(&db, &user, &user, None).await.unwrap();
        db.collection::<Document>(JOBS).update_one(doc! {"_id": &accepted.id},
            vec![doc! {"$set": {"status": "complete", "local_inventory_revision": 0_i32, "retry_at": "$$NOW"}}]).await.unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        let mut schedule = CompletedReconciliationSchedule::default();
        let now = std::time::Instant::now();
        reconcile_completed_if_due(&state, &mut schedule, now)
            .await
            .unwrap();
        assert_eq!(
            super::super::status(&db, &user)
                .await
                .unwrap()
                .unwrap()
                .local_inventory_revision,
            super::local::LOCAL_INVENTORY_REVISION
        );
        assert!(schedule.due(now));
        reconcile_completed_if_due(&state, &mut schedule, now)
            .await
            .unwrap();
        assert!(!schedule.due(now + std::time::Duration::from_secs(1)));
        // Ordinary completion runs the real domains while repair scanning is quiet.
        db.collection::<Document>(JOBS).update_one(doc! {"_id": &accepted.id},
            doc! {"$set": {"status": "pending", "domains": bson::to_bson(&accepted.domains).unwrap(), "local_inventory_revision": 0_i32}}).await.unwrap();
        for _ in 0..64 {
            retry_now(&db, &accepted.id).await;
            assert!(process_one(&state).await.unwrap());
            if db
                .collection::<Document>(USERS)
                .find_one(doc! {"_id": &user})
                .await
                .unwrap()
                .is_none()
            {
                break;
            }
        }
        assert!(
            db.collection::<Document>(USERS)
                .find_one(doc! {"_id": &user})
                .await
                .unwrap()
                .is_none()
        );
        crate::services::telemetry_erasure_service::drain_once(&db, &Default::default())
            .await
            .unwrap();
        crate::services::telemetry_erasure_service::operator_reconcile_empty_inventory(
            &db,
            &user,
            "operator",
            "evidence://deployment-inventory",
            "Verified deployment history",
        )
        .await
        .unwrap();
        for _ in 0..16 {
            retry_now(&db, &accepted.id).await;
            assert!(process_one(&state).await.unwrap());
            if super::super::status(&db, &user)
                .await
                .unwrap()
                .unwrap()
                .status
                == ErasureStatus::Complete
            {
                break;
            }
        }
        let complete = super::super::status(&db, &user).await.unwrap().unwrap();
        assert_eq!(complete.status, ErasureStatus::Complete, "{complete:?}");
        assert_eq!(
            complete.local_inventory_revision,
            super::local::LOCAL_INVENTORY_REVISION
        );
        assert!(!schedule.due(now + std::time::Duration::from_secs(1)));
        db.drop().await.unwrap();
    }
    async fn server_time(db: &Database) -> chrono::DateTime<chrono::Utc> {
        db.run_command(doc! {"hello":1})
            .await
            .unwrap()
            .get_datetime("localTime")
            .unwrap()
            .to_chrono()
    }

    #[tokio::test]
    async fn continuous_remote_wait_escalates_after_twenty_four_hours() {
        let db =
            crate::test_utils::connect_transaction_test_database("erasure_remote_wait_age").await;
        let user = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>("users")
            .insert_one(crate::test_utils::test_user(
                &user,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        crate::test_utils::init_test_audit_chain_hmac_key();
        super::super::accept(&db, &user, &user, None).await.unwrap();
        let job = claim(&db).await.unwrap().unwrap();
        let index = job
            .domains
            .iter()
            .position(|d| d.domain == "local_inventory")
            .unwrap();
        checkpoint(&db, &job, index, Outcome::Pending("remote_cleanup_pending"))
            .await
            .unwrap();
        let raw = db
            .collection::<Document>(JOBS)
            .find_one(doc! {"_id":&job.id})
            .await
            .unwrap()
            .unwrap();
        let mut current: PersonalErasureJob = bson::from_document(raw).unwrap();
        assert!(current.domains[index].remote_wait_started_at.is_some());
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id":&job.id},
                doc! {"$set":{"retry_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        current = claim(&db).await.unwrap().unwrap();
        current.domains[index].remote_wait_started_at =
            Some(current.updated_at - chrono::Duration::hours(24));
        checkpoint(
            &db,
            &current,
            index,
            Outcome::Pending("remote_cleanup_pending"),
        )
        .await
        .unwrap();
        let stored = db
            .collection::<PersonalErasureJob>(JOBS)
            .find_one(doc! {"_id":&job.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, ErasureStatus::OperatorAction);
        assert_eq!(
            stored.domains[index].reason.as_deref(),
            Some("remote_cleanup_wait_exceeded")
        );
        let request = DomainRetryRequest {
            domain: "local_inventory".into(),
            job_id: stored.id.clone(),
            expected_retry_generation: stored.domains[index].retry_generation,
            idempotency_key: uuid::Uuid::new_v4().to_string(),
        };
        for _ in 0..2 {
            assert_eq!(
                retry_domain(&db, &user, &user, &request).await.unwrap(),
                REMOTE_CLEANUP_RETRY_MESSAGE
            );
        }
        let retried = super::super::status(&db, &user).await.unwrap().unwrap();
        assert_eq!(
            retried.domains[index].reason.as_deref(),
            Some("remote_cleanup_verification_required")
        );
        assert_eq!(
            retried.domains[index].remote_wait_started_at,
            stored.domains[index].remote_wait_started_at
        );
        let claimed = claim(&db).await.unwrap().unwrap();
        checkpoint(
            &db,
            &claimed,
            index,
            Outcome::Pending("remote_cleanup_pending"),
        )
        .await
        .unwrap();
        let escalated = super::super::status(&db, &user).await.unwrap().unwrap();
        assert_eq!(escalated.status, ErasureStatus::OperatorAction);
        assert_eq!(
            escalated.domains[index].reason.as_deref(),
            Some("remote_cleanup_wait_exceeded")
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn repeated_domain_failures_exhaust_budget_and_authenticated_retry_preserves_identity() {
        let (db, user) = super::super::tests::fixture().await;
        let accepted = super::super::accept(&db, &user, &user, None).await.unwrap();
        for expected in 1..=8 {
            db.collection::<Document>(JOBS)
                .update_one(
                    doc! {"_id":&accepted.id},
                    vec![doc! {"$set":{"retry_at":"$$NOW"}}],
                )
                .await
                .unwrap();
            let job = claim(&db).await.unwrap().unwrap();
            checkpoint(
                &db,
                &job,
                0,
                Outcome::RetryableFailure("domain_operation_failed_repair_required"),
            )
            .await
            .unwrap();
            let current = super::super::status(&db, &user).await.unwrap().unwrap();
            assert_eq!(current.domains[0].consecutive_failures, expected);
            assert!(current.domains[0].completed_at.is_none());
            assert!(current.completed_at.is_none());
            assert_eq!(
                current.domains[0].status,
                if expected == 8 {
                    DomainStatus::OperatorAction
                } else {
                    DomainStatus::Pending
                }
            );
            if expected < 8 {
                assert!(claim(&db).await.unwrap().is_none());
            }
        }
        assert!(claim(&db).await.unwrap().is_none());
        let before = super::super::status(&db, &user).await.unwrap().unwrap();
        retry_domain(
            &db,
            &user,
            &user,
            &DomainRetryRequest {
                domain: before.domains[0].domain.clone(),
                job_id: before.id.clone(),
                expected_retry_generation: before.domains[0].retry_generation,
                idempotency_key: uuid::Uuid::new_v4().to_string(),
            },
        )
        .await
        .unwrap();
        let retried = super::super::status(&db, &user).await.unwrap().unwrap();
        assert_eq!(retried.id, accepted.id);
        assert_eq!(retried.domains[0].consecutive_failures, 0);
        assert_eq!(retried.domains[0].status, DomainStatus::Pending);
        assert!(retried.domains[0].completed_at.is_none());
        assert!(retried.completed_at.is_none());
        assert!(claim(&db).await.unwrap().is_some());
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn checkpoint_and_operator_retry_use_server_time_and_metadata_audit() {
        let (db, user) = super::super::tests::fixture().await;
        super::super::accept(&db, &user, &user, None).await.unwrap();
        let mut job = claim(&db).await.unwrap().unwrap();
        // A skewed worker snapshot must not supply durable scheduling dates.
        job.updated_at = chrono::Utc::now() + chrono::Duration::days(365);
        job.domains[0].completed_at = Some(job.updated_at);
        let before = server_time(&db).await;
        assert!(checkpoint(&db, &job, 0, Outcome::Complete).await.unwrap());
        let after = server_time(&db).await;
        let current = super::super::status(&db, &user).await.unwrap().unwrap();
        let raw = db
            .collection::<Document>(JOBS)
            .find_one(doc! {"_id":&job.id})
            .await
            .unwrap()
            .unwrap();
        let stored_domains = raw.get_array("domains").unwrap();
        assert!(
            stored_domains[0]
                .as_document()
                .unwrap()
                .get_datetime("completed_at")
                .is_ok()
        );
        for (position, stored) in stored_domains.iter().enumerate() {
            let document = stored.as_document().unwrap();
            assert!(
                document.keys().all(|key| key.parse::<usize>().is_err()),
                "numeric nested array key"
            );
            if position != 0 {
                assert_eq!(
                    document,
                    &bson::to_document(&job.domains[position]).unwrap()
                );
            }
        }
        let completed = current.domains[0].completed_at.unwrap();
        assert!(completed >= before && completed <= after);
        assert!(current.updated_at >= before && current.updated_at <= after);
        assert_eq!(
            current.retry_at.unwrap(),
            current.updated_at + chrono::Duration::seconds(1)
        );
        // Use server time to make the next pass immediately eligible.
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id":&job.id},
                vec![doc! {"$set":{"retry_at":"$$NOW"}}],
            )
            .await
            .unwrap();
        let job = claim(&db).await.unwrap().unwrap();
        checkpoint(
            &db,
            &job,
            1,
            Outcome::OperatorAction("domain_adapter_not_ready"),
        )
        .await
        .unwrap();
        let operator = uuid::Uuid::new_v4().to_string();
        let before = server_time(&db).await;
        let observed = super::super::status(&db, &user).await.unwrap().unwrap();
        retry_domain(
            &db,
            &operator,
            &user,
            &DomainRetryRequest {
                domain: "oracle".into(),
                job_id: observed.id,
                expected_retry_generation: observed.domains[1].retry_generation,
                idempotency_key: uuid::Uuid::new_v4().to_string(),
            },
        )
        .await
        .unwrap();
        let after = server_time(&db).await;
        let current = super::super::status(&db, &user).await.unwrap().unwrap();
        assert!(current.updated_at >= before && current.updated_at <= after);
        assert!(current.retry_at.is_none());
        assert_eq!(current.domains[1].status, DomainStatus::Pending);
        assert!(current.domains[1].completed_at.is_none());
        let audit = db
            .collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! {"event_type":"user.account.erasure_domain_retry"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(audit.get_str("user_id").unwrap(), operator);
        assert_eq!(
            audit
                .get_document("event_data")
                .unwrap()
                .get_str("domain")
                .unwrap(),
            "oracle"
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn expired_or_replaced_worker_cannot_advance_domain_checkpoint() {
        let (db, user) = super::super::tests::fixture().await;
        super::super::accept(&db, &user, &user, None).await.unwrap();
        let first = claim(&db).await.unwrap().unwrap();
        assert!(claim(&db).await.unwrap().is_none());
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id":&first.id},
                doc! {"$set":{"lease_expires_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        let successor = claim(&db).await.unwrap().unwrap();
        assert_ne!(first.lease_token, successor.lease_token);
        assert!(!checkpoint(&db, &first, 0, Outcome::Complete).await.unwrap());
        assert!(
            checkpoint(&db, &successor, 0, Outcome::Complete)
                .await
                .unwrap()
        );
        let current = super::super::status(&db, &user).await.unwrap().unwrap();
        assert_eq!(current.domains[0].status, DomainStatus::Complete);
        assert_eq!(current.status, ErasureStatus::Pending);
        assert!(current.completed_at.is_none());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn worker_replays_each_erasure_domain_after_uncheckpointed_work() {
        use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
        use crate::services::{personal_erasure_registry, telemetry_erasure_service};
        use crate::test_utils::test_user;
        use futures::TryStreamExt;
        let (db, user) = super::super::tests::fixture().await;
        let other = uuid::Uuid::new_v4().to_string();
        let org = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&other, UserType::Person),
                test_user(&org, UserType::Org),
            ])
            .await
            .unwrap();
        let accepted = super::super::accept(&db, &user, &user, None).await.unwrap();
        let original = crate::test_utils::test_app_state(db.clone());
        let restarted = crate::test_utils::test_app_state(db.clone());
        let jobs = db.collection::<Document>(JOBS);
        let phase_rows = [
            (
                "local_inventory",
                db.collection::<Document>("assistant_wire_logs"),
            ),
            (
                "source",
                db.collection::<Document>(personal_erasure_registry::REVOCATION_RECEIPTS),
            ),
        ];
        for (_, rows) in &phase_rows {
            rows.insert_many([&user, &other, &org].into_iter().flat_map(|owner| {
                (0..40)
                    .map(move |index| doc! {"_id": format!("{owner}-{index:02}"), "user_id": owner})
            }))
            .await
            .unwrap();
        }

        for (index, domain) in DOMAINS.iter().enumerate() {
            if *domain == "vendor" {
                telemetry_erasure_service::drain_once(&db, &Default::default())
                    .await
                    .unwrap();
                telemetry_erasure_service::operator_reconcile_empty_inventory(
                    &db,
                    &user,
                    "operator",
                    "evidence://synthetic-provider-inventory",
                    "Reviewed the synthetic deployment inventory",
                )
                .await
                .unwrap();
            }
            retry_now(&db, &accepted.id).await;
            let interrupted = claim(&db).await.unwrap().expect("original worker claim");
            assert_eq!(next_domain_index(&interrupted), Some(index), "{domain}");
            // Execute the production pass and discard its result at the boundary
            // between durable adapter work and the worker checkpoint.
            run_domain(&original, &db, &interrupted, index)
                .await
                .unwrap();
            if let Some((_, rows)) = phase_rows.iter().find(|(name, _)| name == domain) {
                let remaining = rows.count_documents(doc! {"user_id": &user}).await.unwrap();
                assert!(
                    (1..40).contains(&remaining),
                    "{domain} must leave a partial page"
                );
            }
            let persisted = super::super::status(&db, &user).await.unwrap().unwrap();
            assert_eq!(
                bson::to_bson(&persisted.domains).unwrap(),
                bson::to_bson(&interrupted.domains).unwrap(),
                "{domain} lost worker did not write a checkpoint"
            );
            assert!(
                !process_one(&restarted).await.unwrap(),
                "{domain} lease still held"
            );
            let expired = jobs
                .update_one(
                    doc! {"_id": &accepted.id, "lease_token": &interrupted.lease_token,
                    "inventory_version": INVENTORY_VERSION},
                    doc! {"$set": {"lease_expires_at": bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
            assert_eq!(expired.modified_count, 1);
            assert!(
                !checkpoint(&db, &interrupted, index, Outcome::Complete)
                    .await
                    .unwrap(),
                "{domain} expired worker checkpoint"
            );
            for _ in 0..32 {
                retry_now(&db, &accepted.id).await;
                assert!(process_one(&restarted).await.unwrap(), "{domain}");
                let current = super::super::status(&db, &user).await.unwrap().unwrap();
                if current.domains[index].status == DomainStatus::Complete {
                    assert_eq!(current.id, accepted.id);
                    assert_eq!(
                        bson::to_bson(&current.domains[..index]).unwrap(),
                        bson::to_bson(&interrupted.domains[..index]).unwrap(),
                        "{domain} changed an earlier checkpoint"
                    );
                    break;
                }
                assert_eq!(next_domain_index(&current), Some(index), "{domain}");
                assert_eq!(current.status, ErasureStatus::Pending, "{domain}");
            }
            let current = super::super::status(&db, &user).await.unwrap().unwrap();
            assert_eq!(
                current.domains[index].status,
                DomainStatus::Complete,
                "{domain}"
            );
        }
        let finished = super::super::status(&db, &user).await.unwrap().unwrap();
        assert_eq!(finished.status, ErasureStatus::Complete);
        assert!(finished.completed_at.is_some());
        for (_, rows) in &phase_rows {
            assert_eq!(
                rows.count_documents(doc! {"user_id": &user}).await.unwrap(),
                0
            );
            for owner in [&other, &org] {
                let expected: Vec<_> = (0..40)
                    .map(|index| {
                        doc! {
                            "_id": format!("{owner}-{index:02}"), "user_id": owner
                        }
                    })
                    .collect();
                let actual: Vec<_> = rows
                    .find(doc! {"user_id": owner})
                    .sort(doc! {"_id": 1})
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                assert_eq!(actual, expected);
            }
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn revision_four_completed_job_erases_crash_reports_without_touching_other_owners() {
        let (db, user) = super::super::tests::fixture().await;
        let accepted = super::super::accept(&db, &user, &user, None).await.unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        let reports =
            db.collection::<Document>(crate::models::client_error_report::COLLECTION_NAME);
        let bystander = doc! {
            "_id": "other-owner-report",
            "user_id": uuid::Uuid::new_v4().to_string(),
        };
        reports
            .insert_many([
                doc! {"_id": "revision-four-leftover", "user_id": &user},
                bystander.clone(),
            ])
            .await
            .unwrap();
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id": &accepted.id},
                vec![doc! {"$set": {
                    "status": "complete",
                    "completed_at": "$$NOW",
                    "local_inventory_revision": 4_i32,
                    "retry_at": "$$NOW",
                }}],
            )
            .await
            .unwrap();

        assert!(reconcile_completed_once(&state).await.unwrap());
        assert_eq!(
            reports
                .count_documents(doc! {"user_id": &user})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            reports
                .find_one(doc! {"_id": "other-owner-report"})
                .await
                .unwrap(),
            Some(bystander)
        );
        let repaired = db
            .collection::<Document>(JOBS)
            .find_one(doc! {"_id": &accepted.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repaired.get_i32("local_inventory_revision"), Ok(5));
        assert!(!reconcile_completed_once(&state).await.unwrap());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn completed_job_reconciliation_cleans_new_inventory_once_and_stamps_revision() {
        let (db, user) = super::super::tests::fixture().await;
        let accepted = super::super::accept(&db, &user, &user, None).await.unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        db.collection::<Document>("assistant_wire_logs")
            .insert_one(doc! {
                "_id": "reconciliation-leftover",
                "user_id": &user,
                "payload": "must-be-erased",
            })
            .await
            .unwrap();
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id": &accepted.id},
                vec![doc! {"$set": {
                    "status": "complete",
                    "completed_at": "$$NOW",
                    "local_inventory_revision": 2_i32,
                    "domains": bson::to_bson(&accepted.domains).unwrap(),
                    "retry_at": "$$NOW",
                }}],
            )
            .await
            .unwrap();
        db.collection::<Document>("user_services").insert_one(doc! {
            "_id": "reconciliation-shared-attribution", "user_id": uuid::Uuid::new_v4().to_string(),
            "created_by": {"actor": {"person_id": &user, "name": "Old completed account", "email": "old@example.test"}},
        }).await.unwrap();
        assert!(reconcile_completed_once(&state).await.unwrap());
        let shared = db
            .collection::<Document>("user_services")
            .find_one(doc! {"_id": "reconciliation-shared-attribution"})
            .await
            .unwrap()
            .unwrap();
        let actor = shared
            .get_document("created_by")
            .unwrap()
            .get_document("actor")
            .unwrap();
        assert_eq!(actor.get_str("name"), Ok("Deleted account"));
        assert_eq!(actor.get_str("person_id"), Ok(user.as_str()));
        assert!(!actor.contains_key("email"));
        assert!(
            db.collection::<Document>("assistant_wire_logs")
                .find_one(doc! {"_id": "reconciliation-leftover"})
                .await
                .unwrap()
                .is_none()
        );
        let repaired = db
            .collection::<Document>(JOBS)
            .find_one(doc! {"_id": &accepted.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            repaired.get_i32("local_inventory_revision").unwrap(),
            super::local::LOCAL_INVENTORY_REVISION as i32
        );
        assert!(
            repaired
                .get("lease_token")
                .is_some_and(|value| value.as_null().is_some())
        );
        assert!(!reconcile_completed_once(&state).await.unwrap());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn mixed_fleet_completion_does_not_stamp_revision_for_old_local_pass() {
        let (db, user) = super::super::tests::fixture().await;
        let accepted = super::super::accept(&db, &user, &user, None).await.unwrap();
        let old_domains: Vec<_> = accepted
            .domains
            .iter()
            .enumerate()
            .map(|(index, domain)| {
                let mut domain = domain.clone();
                domain.status = if domain.domain == "vendor" {
                    DomainStatus::Pending
                } else {
                    DomainStatus::Complete
                };
                domain.completed_at =
                    (domain.status == DomainStatus::Complete).then_some(chrono::Utc::now());
                if index == 4 {
                    domain.completed_at = Some(chrono::Utc::now());
                }
                domain
            })
            .collect();
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id": &accepted.id},
                vec![doc! {"$set": {
                    "status": "pending",
                    "domains": bson::to_bson(&old_domains).unwrap(),
                    "local_inventory_revision": 0_i32,
                    "retry_at": "$$NOW",
                }}],
            )
            .await
            .unwrap();
        let job = claim(&db).await.unwrap().unwrap();
        let vendor = job
            .domains
            .iter()
            .position(|domain| domain.domain == "vendor")
            .unwrap();
        assert!(
            checkpoint(&db, &job, vendor, Outcome::Complete)
                .await
                .unwrap()
        );
        let current = db
            .collection::<Document>(JOBS)
            .find_one(doc! {"_id": &accepted.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.get_i32("local_inventory_revision"), Ok(0));
        assert_eq!(current.get_str("status"), Ok("complete"));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn stale_reconciliation_lease_cannot_stamp_revision() {
        let (db, user) = super::super::tests::fixture().await;
        let accepted = super::super::accept(&db, &user, &user, None).await.unwrap();
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id": &accepted.id},
                vec![doc! {"$set": {
                    "status": "complete",
                    "local_inventory_revision": 0_i32,
                    "retry_at": "$$NOW",
                }}],
            )
            .await
            .unwrap();
        let claimed = claim_completed_reconciliation(&db).await.unwrap().unwrap();
        let lease = claimed.lease_token.clone().unwrap();
        db.collection::<Document>(JOBS)
            .update_one(
                doc! {"_id": &accepted.id, "lease_token": &lease},
                doc! {"$set": {"lease_expires_at": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        assert!(!stamp_reconciled(&db, &accepted.id, &lease).await.unwrap());
        assert_eq!(
            db.collection::<Document>(JOBS)
                .find_one(doc! {"_id": &accepted.id})
                .await
                .unwrap()
                .unwrap()
                .get_i32("local_inventory_revision"),
            Ok(0)
        );
        db.drop().await.unwrap();
    }
}
