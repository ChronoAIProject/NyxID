//! Bounded, restartable domain scheduling. Every checkpoint compares the
//! current lease token and server time; an expired worker cannot advance it.
use super::*;
use mongodb::options::ReturnDocument;

const LEASE_SECONDS: i64 = 120;

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
        .filter(|(_, domain)| domain.status != DomainStatus::Complete)
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
    let keys = &state.encryption_keys;
    let durable = super::super::transaction_service::durable_database(&state.db);
    let db = &durable;
    Box::pin(super::vendor::revisit(db)).await?;
    Box::pin(super::issuance::expose_uncertain(db)).await?;
    Box::pin(super::provider::drain_one(db, keys)).await?;
    let Some(job) = claim(db).await? else {
        return Ok(false);
    };
    let Some(index) = next_domain_index(&job) else {
        return Err(AppError::Internal(
            "Erasure checkpoint has no pending domain".into(),
        ));
    };
    let domain = &job.domains[index];
    let result: AppResult<Outcome> = async {
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
            "oracle" => Ok(
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
                match Box::pin(super::provider::pass(db, keys, &job)).await? {
                    super::provider::Pass::Complete => Outcome::Complete,
                    super::provider::Pass::CompleteWithException(reason) => {
                        Outcome::CompleteWithException(reason)
                    }
                    super::provider::Pass::Pending => {
                        Outcome::Pending("provider_revocation_pending")
                    }
                    super::provider::Pass::Action(reason) => Outcome::OperatorAction(reason),
                },
            ),
            "vendor" => Ok(
                match Box::pin(super::vendor::pass(db, &job.user_id)).await? {
                    super::vendor::Pass::Complete(evidence) => {
                        Outcome::CompleteWithEvidence(evidence)
                    }
                    super::vendor::Pass::Pending => Outcome::Pending("vendor_erasure_pending"),
                    super::vendor::Pass::Action => {
                        Outcome::OperatorAction("vendor_reconciliation_required")
                    }
                },
            ),
            "channel_bots" => Ok(if Box::pin(super::channel::pass(state, &job)).await? {
                Outcome::Complete
            } else {
                Outcome::Pending("channel_webhook_cleanup_pending")
            }),
            "machine" => Ok(match Box::pin(super::machine::pass(state, &job)).await? {
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
            "local_inventory" => Ok(if Box::pin(super::local::pass(state, &job)).await? {
                Outcome::Complete
            } else {
                Outcome::Pending("local_inventory_cleanup_pending")
            }),
            "source" => Ok(if Box::pin(super::local::source(db, &job)).await? {
                Outcome::Complete
            } else {
                Outcome::Pending("source_cleanup_pending")
            }),
            _ => Ok(Outcome::OperatorAction("unknown_inventory_domain")),
        }
    }
    .await;
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
    Ok(true)
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

pub(crate) async fn retry_domain(
    db: &Database,
    actor: &str,
    user: &str,
    request: &DomainRetryRequest,
) -> AppResult<()> {
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
    let result: AppResult<()> = async {
        let job = db.collection::<PersonalErasureJob>(JOBS).find_one(doc! {"_id":&request.job_id,"user_id":user}).session(&mut *tx).await?
            .ok_or_else(||AppError::Conflict("Erasure retry job changed".into()))?;
        let previous = job.domains.iter().find(|entry| entry.domain == domain)
            .ok_or_else(||AppError::Conflict("Erasure retry domain changed".into()))?;
        if previous.last_retry_key_hash.as_deref() == Some(key_hash) {
            if previous.last_retry_request_digest.as_deref() != Some(request_digest) {
                return Err(AppError::Conflict("Erasure retry idempotency key was reused".into()));
            }
            return Ok(());
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
        checkpoint.reason = None;
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
        Ok(())
    }.await;
    result
        })
    }).await
}

pub(crate) fn spawn_worker(state: crate::AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            if super::preparation::process_one(&state.db, &state.encryption_keys)
                .await
                .is_err()
            {
                tracing::warn!("Personal erasure dependency preparation remains pending");
            }
            if process_one(&state).await.is_err() {
                tracing::warn!("Personal erasure pass remains pending; durable lease will retry");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn server_time(db: &Database) -> chrono::DateTime<chrono::Utc> {
        db.run_command(doc! {"hello":1})
            .await
            .unwrap()
            .get_datetime("localTime")
            .unwrap()
            .to_chrono()
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
}
