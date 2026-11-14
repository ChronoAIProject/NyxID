use super::*;
use crate::services::{
    api_key_mutation_service as transactions,
    audit_service::{self, AuditActor},
    oracle_mutation_service as mutation,
};
use bson::Document;
use mongodb::{ClientSession, Database};

const DELETE_COMMAND_EXPIRY_BATCH_SIZE: i64 = 256;
const DELETE_COMMAND_EXPIRY_MAX_BATCHES: usize = 16;

#[derive(Clone, Default)]
pub struct TransferPoolInput {
    pub owner_user_id: String,
    pub rotate_worker_token: bool,
    pub visibility: Option<OraclePoolVisibility>,
}

pub struct TransferPoolOutcome {
    pub pool: OraclePool,
    pub worker_token: Option<zeroize::Zeroizing<String>>,
    pub can_enroll: bool,
    pub online_workers: u32,
}

pub struct DeletePoolOutcome {
    pub id: String,
    pub slug: String,
    pub deleted_at: chrono::DateTime<Utc>,
}

pub async fn transfer_pool(
    db: &Database,
    actor: &AuditActor,
    id_or_slug: &str,
    input: TransferPoolInput,
) -> AppResult<TransferPoolOutcome> {
    let transaction_db = db.clone();
    let transaction_actor = actor.clone();
    let transaction_id_or_slug = id_or_slug.to_owned();
    uuid::Uuid::parse_str(&input.owner_user_id)
        .map_err(|_| AppError::ValidationError("owner_user_id must be a UUID".into()))?;
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                transaction_actor,
                transaction_id_or_slug,
                input,
            ),
            |session, (transaction_db, transaction_actor, transaction_id_or_slug, input)| {
                Box::pin(async move {
                    let input = &*input;
                    let db = &*transaction_db;
                    let actor = &*transaction_actor;
                    let id_or_slug = transaction_id_or_slug.as_str();
                    transactions::transaction_result(
                        transfer_in_session(db, session, actor, id_or_slug, input).await,
                    )
                })
            },
        )
        .await
        .map_err(transactions::map_transaction_error)
}

async fn transfer_in_session(
    db: &Database,
    session: &mut ClientSession,
    actor: &AuditActor,
    id_or_slug: &str,
    input: &TransferPoolInput,
) -> AppResult<TransferPoolOutcome> {
    let pool = mutation::lookup_pool(db, session, id_or_slug, false).await?;
    mutation::lock_pool(db, session, &pool.id).await?;
    mutation::lock_manager(db, session, &actor.user_id, &pool.user_id).await?;
    let destination =
        mutation::lock_manager(db, session, &actor.user_id, &input.owner_user_id).await?;
    if destination.user_type == UserType::Person && destination.id != actor.user_id {
        return Err(AppError::Forbidden(
            "personal destination must be your own active person account".into(),
        ));
    }
    let visibility = input
        .visibility
        .clone()
        .unwrap_or_else(|| pool.visibility.clone());
    if destination.user_type != UserType::Org && visibility == OraclePoolVisibility::Org {
        return Err(AppError::ValidationError(
            "moving an org-visible pool to a person requires explicit visibility=private".into(),
        ));
    }
    // A transfer never implicitly widens the audience. Audience changes beyond
    // the necessary org->person narrowing use the ordinary settings endpoint.
    if visibility != pool.visibility
        && !(pool.visibility == OraclePoolVisibility::Org
            && destination.user_type == UserType::Person
            && visibility == OraclePoolVisibility::Private)
    {
        return Err(AppError::ValidationError(
            "transfer visibility may only narrow org to private for a personal destination".into(),
        ));
    }
    if pool.user_id != destination.id {
        let count = db
            .collection::<Document>(ORACLE_POOLS)
            .count_documents(doc! {"user_id": &destination.id, "deleted_at": null})
            .session(&mut *session)
            .await?;
        if count >= MAX_POOLS_PER_OWNER {
            return Err(AppError::Conflict(format!(
                "Maximum of {MAX_POOLS_PER_OWNER} oracle pools per owner reached"
            )));
        }
    }
    if pool.user_id == destination.id && !input.rotate_worker_token {
        return transfer_outcome(db, session, actor, pool, None).await;
    }
    let token = input.rotate_worker_token.then(|| {
        let (raw, hash) = mint_worker_token();
        (zeroize::Zeroizing::new(raw), hash)
    });
    let mut set = doc! {"user_id": &destination.id, "visibility": visibility.as_str(), "updated_at": bson::DateTime::from_chrono(Utc::now())};
    if pool.user_id != destination.id {
        set.insert("installation_epoch", pool.installation_epoch + 1);
    }
    if let Some((_, hash)) = &token {
        set.insert("worker_token_hash", hash);
    }
    db.collection::<Document>(ORACLE_POOLS)
        .update_one(
            doc! {"_id": &pool.id},
            doc! {"$set": set, "$inc": {"lifecycle_version": 1_i64}},
        )
        .session(&mut *session)
        .await?;
    let key = audit_service::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("audit chain key is not initialized".into()))?;
    audit_service::log_actor_event_in_session(db, session, key, actor, "oracle_pool_transferred", serde_json::json!({"pool_id":pool.id,"pool_slug":pool.slug,"old_owner_user_id":pool.user_id,"new_owner_user_id":destination.id,"worker_token_rotated":input.rotate_worker_token})).await?;
    let updated = mutation::lookup_pool(db, session, &pool.id, false).await?;
    transfer_outcome(db, session, actor, updated, token.map(|(raw, _)| raw)).await
}

pub async fn delete_pool(
    db: &Database,
    actor: &AuditActor,
    id_or_slug: &str,
) -> AppResult<DeletePoolOutcome> {
    let mut expiry_batches = 0;
    let outcome = loop {
        let transaction_db = db.clone();
        let transaction_actor = actor.clone();
        let transaction_id_or_slug = id_or_slug.to_owned();
        let mut session = db.client().start_session().await?;
        let attempt = session
            .start_transaction()
            .and_run(
                (transaction_db, transaction_actor, transaction_id_or_slug),
                |session, (transaction_db, transaction_actor, transaction_id_or_slug)| {
                    Box::pin(async move {
                        transactions::transaction_result(
                            delete_in_session(
                                &*transaction_db,
                                session,
                                &*transaction_actor,
                                &*transaction_id_or_slug,
                            )
                            .await,
                        )
                    })
                },
            )
            .await
            .map_err(transactions::map_transaction_error)?;
        if let Some(outcome) = attempt {
            break outcome;
        }
        expiry_batches += 1;
        if expiry_batches >= DELETE_COMMAND_EXPIRY_MAX_BATCHES {
            // The bounded request made durable progress; retry settles the rest.
            return Err(AppError::OraclePoolActiveWork);
        }
        // Expiry is ordinary settlement, committed independently of deletion.
        // Interrupted requests/retries resume from remaining overdue rows.
        tokio::task::yield_now().await;
    };
    // Logical deletion is already committed. Cleanup failure must not turn its
    // acknowledgement into an ambiguous error. The durable marker retries it.
    if let Err(error) = cleanup_pool(db, &outcome.id, 16).await {
        tracing::warn!(%error, pool_id = %outcome.id, "Oracle deletion cleanup deferred");
    }
    Ok(outcome)
}

async fn delete_in_session(
    db: &Database,
    session: &mut ClientSession,
    actor: &AuditActor,
    id_or_slug: &str,
) -> AppResult<Option<DeletePoolOutcome>> {
    delete_in_session_with_owner(db, session, actor, id_or_slug, None).await
}

async fn delete_in_session_with_owner(
    db: &Database,
    session: &mut ClientSession,
    actor: &AuditActor,
    id_or_slug: &str,
    erasure_owner: Option<&str>,
) -> AppResult<Option<DeletePoolOutcome>> {
    let pool = mutation::lookup_pool(db, session, id_or_slug, true).await?;
    // The same write fence is retained on tombstones for duplicate deletion.
    db.collection::<Document>(ORACLE_POOLS)
        .update_one(
            doc! {"_id": &pool.id},
            doc! {"$inc": {"worker_enrollment_epoch": 1_i64}},
        )
        .session(&mut *session)
        .await?;
    if let Some(owner) = erasure_owner {
        if pool.user_id != owner {
            return Err(AppError::Forbidden(
                "Personal erasure cannot retire another owner's Oracle pool".into(),
            ));
        }
        let changed = db
            .collection::<Document>(USERS)
            .update_one(
                doc! {"_id":owner,"user_type":{"$ne":"org"}},
                doc! {"$inc":{"oracle_enrollment_epoch":1_i64}},
            )
            .session(&mut *session)
            .await?;
        if changed.matched_count != 1 {
            return Err(AppError::Conflict("Personal Oracle owner changed".into()));
        }
    } else {
        mutation::lock_manager(db, session, &actor.user_id, &pool.user_id).await?;
    }
    if let Some(deleted_at) = pool.deleted_at {
        return Ok(Some(DeletePoolOutcome {
            id: pool.id,
            slug: pool.slug,
            deleted_at,
        }));
    }
    // Older writers could leave commands for a retired worker generation.
    // Current worker endpoints deliberately cannot deliver/manage those rows.
    // Expire only deadlines already reached, at most 256 IDs per transaction;
    // return progress so this settlement commits even if active work later
    // prevents deletion. Never force-abandon an unexpired command.
    let now = bson::DateTime::now();
    let commands = db.collection::<Document>(crate::models::oracle_worker_command::COLLECTION_NAME);
    let overdue: Vec<Document> = commands
        .find(doc! {"pool_id": &pool.id, "status": {"$in": ["queued", "delivered"]}, "deadline_at": {"$lte": now}})
        .projection(doc! {"_id": 1})
        .sort(doc! {"deadline_at": 1, "_id": 1})
        .limit(DELETE_COMMAND_EXPIRY_BATCH_SIZE)
        .session(&mut *session).await?
        .stream(&mut *session).try_collect().await?;
    if !overdue.is_empty() {
        let ids: Vec<_> = overdue
            .iter()
            .map(|row| {
                row.get("_id")
                    .cloned()
                    .ok_or_else(|| AppError::Internal("invalid Oracle command ID".into()))
            })
            .collect::<AppResult<_>>()?;
        commands.update_many(doc! {"_id": {"$in": ids}}, doc! {"$set": {
            "status": "expired", "completed_at": now, "updated_at": now,
            "expires_at": bson::DateTime::from_chrono(now.to_chrono() + chrono::Duration::days(crate::services::oracle_worker_service::COMMAND_RETENTION_DAYS)),
        }}).session(&mut *session).await?;
        return Ok(None);
    }
    for (collection, statuses) in [
        (
            crate::models::oracle_task::COLLECTION_NAME,
            vec!["queued", "dispatched"],
        ),
        (
            crate::models::oracle_worker_command::COLLECTION_NAME,
            vec!["queued", "delivered"],
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(doc! {"pool_id": &pool.id,"status":{"$in":statuses}})
            .session(&mut *session)
            .await?
            .is_some()
        {
            return Err(AppError::OraclePoolActiveWork);
        }
    }
    let now = bson::DateTime::from_chrono(Utc::now()).to_chrono();
    db.collection::<Document>(ORACLE_POOLS).update_one(doc! {"_id": &pool.id},doc! {"$set":{"deleted_at":bson::DateTime::from_chrono(now),"updated_at":bson::DateTime::from_chrono(now),"is_active":false,"deletion_cleanup_pending":true,"worker_token_hash":format!("deleted:{}",uuid::Uuid::new_v4())},"$inc":{"lifecycle_version":1_i64}}).session(&mut *session).await?;
    let key = audit_service::audit_chain_hmac_key()
        .ok_or_else(|| AppError::Internal("audit chain key is not initialized".into()))?;
    audit_service::log_actor_event_in_session(
        db,
        session,
        key,
        actor,
        "oracle_pool_deleted",
        serde_json::json!({"pool_id":pool.id,"pool_slug":pool.slug,"owner_user_id":pool.user_id}),
    )
    .await?;
    Ok(Some(DeletePoolOutcome {
        id: pool.id,
        slug: pool.slug,
        deleted_at: now,
    }))
}

/// Personal-erasure acceptance reuses the pool's exact tombstone/audit/work
/// guards while the owner is still active. All effects roll back if ANY later
/// account prerequisite rejects; creator attribution is never an owner filter.
pub(crate) async fn accept_personal_erasure_in_session(
    db: &Database,
    session: &mut ClientSession,
    owner: &str,
    requested_by: &str,
) -> AppResult<()> {
    let pools: Vec<Document> = db
        .collection::<Document>(ORACLE_POOLS)
        .find(doc! {"user_id":owner,"deleted_at":null})
        .projection(doc! {"_id":1})
        .sort(doc! {"_id":1})
        .limit(MAX_POOLS_PER_OWNER as i64 + 1)
        .session(&mut *session)
        .await?
        .stream(&mut *session)
        .try_collect()
        .await?;
    if pools.len() > MAX_POOLS_PER_OWNER as usize {
        return Err(AppError::Conflict(
            "Retire excess legacy Oracle pools before deleting this account".into(),
        ));
    }
    let actor = AuditActor {
        user_id: requested_by.to_owned(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    for pool in pools {
        let id = pool
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid Oracle pool identity".into()))?;
        // None means overdue command settlement needs the ordinary pool API.
        // Do not commit partial acceptance or silently abandon active work.
        if delete_in_session_with_owner(db, session, &actor, id, Some(owner))
            .await?
            .is_none()
        {
            return Err(AppError::OraclePoolActiveWork);
        }
    }
    Ok(())
}

async fn transfer_outcome(
    db: &Database,
    session: &mut ClientSession,
    actor: &AuditActor,
    pool: OraclePool,
    worker_token: Option<zeroize::Zeroizing<String>>,
) -> AppResult<TransferPoolOutcome> {
    let is_person = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &actor.user_id,"is_active":true,"user_type":"person"})
        .session(&mut *session)
        .await?
        .is_some();
    let cutoff = Utc::now()
        - chrono::Duration::seconds(crate::services::oracle_task_service::WORKER_RECENT_SECS);
    let online_workers = db
        .collection::<Document>(crate::models::oracle_worker::COLLECTION_NAME)
        .count_documents(
            doc! {"pool_id": &pool.id,"last_seen_at":{"$gte":bson::DateTime::from_chrono(cutoff)}},
        )
        .session(session)
        .await? as u32;
    Ok(TransferPoolOutcome {
        can_enroll: pool.is_active && is_person,
        pool,
        worker_token,
        online_workers,
    })
}

/// Bounded physical cleanup. Each transaction handles at most one document
/// from each collection (even legacy records may approach Mongo's 16 MiB cap).
/// The remaining rows are the durable cursor; no offsets or process leases can
/// skip work. All replicas contend on the retained pool fence.
async fn cleanup_chunk(db: &Database, pool_id: &str) -> AppResult<bool> {
    let transaction_db = db.clone();
    let transaction_pool_id = pool_id.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run(
            (transaction_db, transaction_pool_id),
            |session, (db, pool_id)| {
                Box::pin(async move {
                    let result: AppResult<bool> = async {
                        let Some(pool) = db
                            .collection::<Document>(ORACLE_POOLS)
                            .find_one_and_update(
                                doc! {
                                    "_id": &*pool_id,
                                    "deleted_at": {"$ne": null},
                                    "deletion_cleanup_pending": true,
                                },
                                doc! {"$inc": {"worker_enrollment_epoch": 1_i64}},
                            )
                            .session(&mut *session)
                            .await?
                        else {
                            return Ok(true);
                        };
                        let deleted_at = *pool
                            .get_datetime("deleted_at")
                            .map_err(|_| AppError::Internal("invalid Oracle tombstone".into()))?;
                        let mut changed = false;
                        for collection in [
                            crate::models::oracle_worker::COLLECTION_NAME,
                            crate::models::oracle_login_profile::COLLECTION_NAME,
                            crate::models::oracle_login_snapshot::COLLECTION_NAME,
                        ] {
                            let rows = db.collection::<Document>(collection);
                            // Project IDs so cleanup never loads secret envelopes.
                            if let Some(row) = rows
                                .find_one(doc! {"pool_id": &*pool_id})
                                .projection(doc! {"_id": 1})
                                .session(&mut *session)
                                .await?
                            {
                                rows.delete_one(doc! {"_id": row.get("_id").unwrap()})
                                    .session(&mut *session)
                                    .await?;
                                changed = true;
                            }
                        }
                        let sessions = db
                            .collection::<Document>(crate::models::oracle_session::COLLECTION_NAME);
                        if let Some(row) = sessions
                            .find_one(doc! {"pool_id": &*pool_id, "closed_at": null})
                            .projection(doc! {"_id": 1})
                            .session(&mut *session)
                            .await?
                        {
                            sessions.update_one(
                                doc! {"_id": row.get("_id").unwrap()},
                                doc! {"$set": {"closed_at": deleted_at, "updated_at": deleted_at}},
                            )
                            .session(&mut *session)
                            .await?;
                            changed = true;
                        }
                        if !changed {
                            db.collection::<Document>(ORACLE_POOLS)
                                .update_one(
                                    doc! {"_id": &*pool_id},
                                    doc! {"$set": {"deletion_cleanup_pending": false}},
                                )
                                .session(&mut *session)
                                .await?;
                        }
                        Ok(!changed)
                    }
                    .await;
                    transactions::transaction_result(result)
                })
            },
        )
        .await
        .map_err(transactions::map_transaction_error)
}

async fn cleanup_pool(db: &Database, pool_id: &str, chunks: usize) -> AppResult<()> {
    for _ in 0..chunks {
        if cleanup_chunk(db, pool_id).await? {
            break;
        }
    }
    Ok(())
}

/// One bounded personal-erasure pass. Owned pool tombstones/history retain
/// their lifecycle contract. A contributor owns its installation credential,
/// never the organization pool or other contributors' workers.
pub(crate) async fn personal_erasure_pass(
    db: &Database,
    owner: &str,
    actor: &str,
) -> AppResult<bool> {
    let durable = crate::services::transaction_service::durable_database(db);
    let db = &durable;
    if let Some(pool) = db
        .collection::<Document>(ORACLE_POOLS)
        .find_one(doc! {"user_id":owner,"deleted_at":{"$ne":null},"deletion_cleanup_pending":true})
        .projection(doc! {"_id":1})
        .sort(doc! {"_id":1})
        .await?
    {
        let id = pool
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid Oracle pool identity".into()))?;
        cleanup_chunk(db, id).await?;
        return Ok(false);
    }
    let workers = db.collection::<Document>(crate::models::oracle_worker::COLLECTION_NAME);
    if let Some(worker) = workers
        .find_one(doc! {
            "enrollment.owner_user_id":owner,"enrollment.retired_for_owner_erasure":{"$ne":true}
        })
        .projection(doc! {"_id":1,"pool_id":1,"enrollment.credential_hash":1})
        .sort(doc! {"_id":1})
        .await?
    {
        let id = worker
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid Oracle worker identity".into()))?;
        let pool = worker
            .get_str("pool_id")
            .map_err(|_| AppError::Internal("Invalid Oracle worker pool".into()))?;
        let mut tx = db.client().start_session().await?;
        tx.start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(mongodb::options::WriteConcern::majority())
            .await?;
        let operation: AppResult<()> = async {
            // Contend with enrollment/transfer and every live worker mutation.
            db.collection::<Document>(ORACLE_POOLS).update_one(doc! {"_id":pool},doc! {"$inc":{"worker_enrollment_epoch":1_i64}}).session(&mut tx).await?;
            let changed = workers.update_one(doc! {
                "_id":id,"pool_id":pool,"enrollment.owner_user_id":owner,
                "enrollment.credential_hash":worker.get_document("enrollment").ok().and_then(|e|e.get("credential_hash")).cloned().unwrap_or(bson::Bson::Null),
                "enrollment.retired_for_owner_erasure":{"$ne":true}
            },doc! {"$set":{"enrollment.credential_hash":format!("erased:{}",uuid::Uuid::new_v4()),"enrollment.retired_for_owner_erasure":true,"desired_state":"draining"}}).session(&mut tx).await?;
            if changed.matched_count == 1 {
                let key = super::super::audit_service::audit_chain_hmac_key().ok_or_else(||AppError::Internal("Audit key unavailable".into()))?;
                super::super::audit_service::log_actor_event_in_session(db,&mut tx,key,&AuditActor {user_id:actor.to_owned(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},"oracle_worker_owner_erasure",serde_json::json!({"owner_id":owner,"pool_id":pool,"worker_id":id})).await?;
            }
            Ok(())
        }.await;
        crate::services::org_membership_authority::finish(tx, operation).await?;
        return Ok(false);
    }
    Ok(true)
}

/// One bounded fair pass; pending state survives failures and server restarts.
pub async fn cleanup_deleted_pools(db: &Database) -> AppResult<()> {
    let pools: Vec<Document> = db
        .collection::<Document>(ORACLE_POOLS)
        .find(doc! {"deleted_at":{"$ne":null},"deletion_cleanup_pending":true})
        .projection(doc! {"_id":1})
        .sort(doc! {"cleanup_attempted_at":1,"_id":1})
        .limit(16)
        .await?
        .try_collect()
        .await?;
    for pool in pools {
        let id = pool
            .get_str("_id")
            .map_err(|_| AppError::Internal("invalid Oracle pool ID".into()))?;
        // Scheduling metadata is independent of the lifecycle fence and never
        // authorizes writes. Rotating oldest attempts avoids a failed pool
        // starving other pending deletions.
        db.collection::<Document>(ORACLE_POOLS)
            .update_one(
                doc! {"_id":id},
                doc! {"$set":{"cleanup_attempted_at":bson::DateTime::now()}},
            )
            .await?;
        if let Err(error) = cleanup_pool(db, id, 16).await {
            tracing::warn!(%error, pool_id = id, "Oracle deletion cleanup will retry");
        }
    }
    Ok(())
}

/// Indexes keep global scheduling and incremental session closure proportional
/// to pending work, even when most pools/history are already terminal.
pub async fn ensure_lifecycle_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<Document>(ORACLE_POOLS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"deletion_cleanup_pending":1,"cleanup_attempted_at":1,"_id":1})
                .options(
                    IndexOptions::builder()
                        .name("oracle_pool_pending_cleanup".to_owned())
                        .partial_filter_expression(doc! {"deletion_cleanup_pending":true})
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(crate::models::oracle_worker_command::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"pool_id":1,"status":1,"deadline_at":1,"_id":1})
                .options(
                    IndexOptions::builder()
                        .name("oracle_pool_command_deadlines".to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Document>(crate::models::oracle_session::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"pool_id":1,"closed_at":1,"_id":1})
                .options(
                    IndexOptions::builder()
                        .name("oracle_pool_open_sessions_cleanup".to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
