use super::*;
use mongodb::{ClientSession, Database};

fn identity_filter(worker: &OracleWorker) -> Document {
    doc! { "_id": &worker.id, "generation": &worker.generation, "instance_id": &worker.instance_id }
}

async fn lock_worker(
    db: &Database,
    session: &mut ClientSession,
    worker: &OracleWorker,
) -> AppResult<OracleWorker> {
    db.collection::<OracleWorker>(ORACLE_WORKERS)
        .find_one_and_update(
            identity_filter(worker),
            doc! { "$inc": { "management_epoch": 1_i64 } },
        )
        .return_document(ReturnDocument::After)
        .session(session)
        .await?
        .ok_or_else(|| {
            AppError::Conflict("worker installation changed; refresh before retrying".into())
        })
}

pub async fn enqueue_worker_command(
    db: &Database,
    pool: &OraclePool,
    worker: &OracleWorker,
    actor: &str,
    kind: OracleWorkerCommandKind,
    snapshot_id: Option<String>,
    bundle: Option<(String, String)>,
) -> AppResult<OracleWorkerCommand> {
    let transaction_db = db.clone();
    let transaction_pool = pool.clone();
    let transaction_worker = worker.clone();
    let transaction_actor = actor.to_owned();
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                transaction_pool,
                transaction_worker,
                transaction_actor,
                kind,
                snapshot_id,
                bundle,
            ),
            |mongo_session,
             (
                transaction_db,
                transaction_pool,
                transaction_worker,
                transaction_actor,
                kind,
                snapshot_id,
                bundle,
            )| {
                Box::pin(async move {
                    let kind = &*kind;
                    let snapshot_id = &*snapshot_id;
                    let bundle = &*bundle;
                    let db = &*transaction_db;
                    let pool = &*transaction_pool;
                    let worker = &*transaction_worker;
                    let actor = transaction_actor.as_str();
                    crate::services::api_key_mutation_service::transaction_result(
                        enqueue_worker_command_in_session(
                            db,
                            mongo_session,
                            pool,
                            worker,
                            actor,
                            WorkerCommandInput {
                                kind: kind.clone(),
                                snapshot_id: snapshot_id.clone(),
                                bundle: bundle.clone(),
                            },
                        )
                        .await,
                    )
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

struct WorkerCommandInput {
    pub kind: OracleWorkerCommandKind,
    pub snapshot_id: Option<String>,
    pub bundle: Option<(String, String)>,
}

async fn enqueue_worker_command_in_session(
    db: &Database,
    mongo_session: &mut mongodb::ClientSession,
    pool: &OraclePool,
    worker: &OracleWorker,
    actor: &str,
    input: WorkerCommandInput,
) -> AppResult<OracleWorkerCommand> {
    let WorkerCommandInput {
        kind,
        snapshot_id,
        bundle,
    } = input;
    crate::services::oracle_mutation_service::lock_expected_pool(db, mongo_session, pool, false)
        .await?;
    enrollment::lock_worker_manager(db, mongo_session, actor, pool, worker).await?;

    let capability = required_capability(&kind).to_string();
    let now = Utc::now();
    let (bundle_version, bundle_sha256) = bundle.unzip();
    let command = OracleWorkerCommand {
        id: uuid::Uuid::new_v4().to_string(),
        pool_id: worker.pool_id.clone(),
        worker_label: worker.worker_label.clone(),
        worker_generation: worker.generation.clone(),
        kind,
        status: OracleWorkerCommandStatus::Queued,
        created_by_user_id: actor.into(),
        required_capability: Some(capability.clone()),
        delivery_count: 0,
        result_code: None,
        snapshot_id,
        bundle_version,
        bundle_sha256,
        delivered_at: None,
        delivery_lease_expires_at: None,
        completed_at: None,
        deadline_at: now + Duration::hours(COMMAND_DEADLINE_HOURS),
        expires_at: None,
        created_at: now,
        updated_at: now,
    };

    let live = lock_worker(db, mongo_session, worker).await?;
    if command.kind == OracleWorkerCommandKind::SessionImport && live.enrollment.is_some() {
        return Err(AppError::OracleWorkerCapabilityUnsupported(
            "contributed workers use their own browser login".into(),
        ));
    }
    if !live.capabilities.contains(&capability) {
        return Err(AppError::OracleWorkerCapabilityUnsupported(format!(
            "worker '{}' does not advertise {capability}",
            live.worker_label
        )));
    }
    db.collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
        .insert_one(&command)
        .session(&mut *mongo_session)
        .await?;
    db.collection::<Document>(ORACLE_WORKERS)
        .update_one(
            identity_filter(worker),
            doc! { "$set": { "desired_state": "draining" } },
        )
        .session(&mut *mongo_session)
        .await?;
    Ok(command)
}

pub async fn forget_authorized_worker(
    db: &Database,
    pool: &OraclePool,
    worker: &OracleWorker,
    actor: &str,
    force: bool,
) -> AppResult<ForgetOutcome> {
    let transaction_db = db.clone();
    let transaction_pool = pool.clone();
    let transaction_worker = worker.clone();
    let transaction_actor = actor.to_owned();
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                transaction_pool,
                transaction_worker,
                transaction_actor,
                force,
            ),
            |mongo_session, context| {
                let (
                    transaction_db,
                    transaction_pool,
                    transaction_worker,
                    transaction_actor,
                    force,
                ) = context;
                Box::pin(async move {
                    let force = *force;
                    let db = &*transaction_db;
                    let pool = &*transaction_pool;
                    let worker = &*transaction_worker;
                    let actor = transaction_actor.as_str();
                    crate::services::api_key_mutation_service::transaction_result(
                        forget_authorized_worker_in_session(
                            db,
                            mongo_session,
                            pool,
                            worker,
                            actor,
                            force,
                        )
                        .await,
                    )
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn forget_authorized_worker_in_session(
    db: &Database,
    mongo_session: &mut mongodb::ClientSession,
    pool: &OraclePool,
    worker: &OracleWorker,
    actor: &str,
    force: bool,
) -> AppResult<ForgetOutcome> {
    crate::services::oracle_mutation_service::lock_expected_pool(db, mongo_session, pool, false)
        .await?;
    enrollment::lock_worker_manager(db, mongo_session, actor, pool, worker).await?;

    forget_in_session(db, mongo_session, worker, force).await
}

async fn forget_in_session(
    db: &Database,
    session: &mut ClientSession,
    expected: &OracleWorker,
    force: bool,
) -> AppResult<ForgetOutcome> {
    let worker = lock_worker(db, session, expected).await?;
    let now = Utc::now();
    let pool_id = &worker.pool_id;
    let label = &worker.worker_label;
    if !force {
        if (now - worker.last_seen_at).num_seconds() <= FORGET_ONLINE_WINDOW_SECS {
            return Err(AppError::Conflict(format!(
                "worker '{label}' is online; stop it first or pass force"
            )));
        }
        let inflight = db
            .collection::<Document>(ORACLE_TASKS)
            .count_documents(doc! {
                "pool_id": pool_id, "status": "dispatched", "assigned_worker_id": label,
            })
            .session(&mut *session)
            .await?;
        if inflight > 0 {
            return Err(AppError::Conflict(format!(
                "worker '{label}' has a task in flight; wait for it to settle or pass force"
            )));
        }
    }
    let commands_removed = db
        .collection::<Document>(ORACLE_WORKER_COMMANDS)
        .delete_many(doc! { "pool_id": pool_id, "worker_label": label })
        .session(&mut *session)
        .await?
        .deleted_count;
    let sessions_released = db.collection::<Document>(ORACLE_SESSIONS).update_many(
        doc! { "pool_id": pool_id, "owner_worker_label": label },
        doc! { "$set": { "owner_worker_label": null, "updated_at": bson::DateTime::from_chrono(now) } },
    ).session(&mut *session).await?.modified_count;
    let tasks_released = db.collection::<Document>(ORACLE_TASKS).update_many(
        doc! { "pool_id": pool_id, "status": "queued", "required_worker_label": label },
        doc! { "$set": { "phase": "affinity_released_by_forget", "updated_at": bson::DateTime::from_chrono(now) },
            "$unset": { "required_worker_label": "" } },
    ).session(&mut *session).await?.modified_count;
    db.collection::<Document>(crate::models::oracle_login_profile::COLLECTION_NAME)
        .update_many(
            doc! { "pool_id": pool_id, "bindings.worker_label": label },
            doc! { "$pull": { "bindings": { "worker_label": label } } },
        )
        .session(&mut *session)
        .await?;
    db.collection::<Document>(ORACLE_WORKERS)
        .delete_one(identity_filter(&worker))
        .session(&mut *session)
        .await?;
    Ok(ForgetOutcome {
        commands_removed,
        sessions_released,
        tasks_released,
    })
}

pub async fn cancel_worker_command(
    db: &Database,
    pool: &OraclePool,
    worker: &OracleWorker,
    actor: &str,
    command_id: &str,
) -> AppResult<OracleWorkerCommand> {
    let transaction_db = db.clone();
    let transaction_pool = pool.clone();
    let transaction_worker = worker.clone();
    let transaction_actor = actor.to_owned();
    let transaction_command_id = command_id.to_owned();
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                transaction_pool,
                transaction_worker,
                transaction_actor,
                transaction_command_id,
            ),
            |mongo_session,
             (
                transaction_db,
                transaction_pool,
                transaction_worker,
                transaction_actor,
                transaction_command_id,
            )| {
                Box::pin(async move {
                    let db = &*transaction_db;
                    let pool = &*transaction_pool;
                    let worker = &*transaction_worker;
                    let actor = transaction_actor.as_str();
                    let command_id = transaction_command_id.as_str();
                    crate::services::api_key_mutation_service::transaction_result(
                        cancel_worker_command_in_session(
                            db,
                            mongo_session,
                            pool,
                            worker,
                            actor,
                            command_id,
                        )
                        .await,
                    )
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn cancel_worker_command_in_session(
    db: &Database,
    mongo_session: &mut mongodb::ClientSession,
    pool: &OraclePool,
    worker: &OracleWorker,
    actor: &str,
    command_id: &str,
) -> AppResult<OracleWorkerCommand> {
    crate::services::oracle_mutation_service::lock_expected_pool(db, mongo_session, pool, false)
        .await?;
    enrollment::lock_worker_manager(db, mongo_session, actor, pool, worker).await?;

    if !valid_metadata(command_id) {
        return Err(AppError::ValidationError(
            "command_id contains unsupported characters".into(),
        ));
    }
    let command_id = command_id.to_string();

    lock_worker(db, mongo_session, worker).await?;
    let now = Utc::now();
    let commands = db.collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS);
    let mut filter = doc! { "_id": &command_id, "pool_id": &worker.pool_id,
    "worker_label": &worker.worker_label, "worker_generation": &worker.generation };
    let existing = commands
        .find_one(filter.clone())
        .session(&mut *mongo_session)
        .await?
        .ok_or_else(|| AppError::OracleWorkerCommandNotFound(command_id.clone()))?;
    if !matches!(
        existing.status,
        OracleWorkerCommandStatus::Queued | OracleWorkerCommandStatus::Delivered
    ) {
        return Err(AppError::Conflict(format!(
            "command {command_id} is already settled"
        )));
    }
    filter.insert("status", doc! { "$in": ["queued", "delivered"] });
    let command = commands.find_one_and_update(filter, doc! { "$set": {
                "status": "cancelled", "result_code": "cancelled_by_manager", "completed_at": bson::DateTime::from_chrono(now),
                "expires_at": bson::DateTime::from_chrono(now + Duration::days(COMMAND_RETENTION_DAYS)), "updated_at": bson::DateTime::from_chrono(now),
            } }).return_document(ReturnDocument::After).session(&mut *mongo_session).await?
                .ok_or_else(|| AppError::Conflict("command changed; refresh before retrying".into()))?;
    let pending = commands
        .find_one(
            doc! { "pool_id": &worker.pool_id, "worker_label": &worker.worker_label,
            "worker_generation": &worker.generation, "status": { "$in": ["queued", "delivered"] } },
        )
        .session(&mut *mongo_session)
        .await?;
    db.collection::<Document>(ORACLE_WORKERS).update_one(identity_filter(worker),
                doc! { "$set": { "desired_state": if pending.is_some() { "draining" } else { "active" } } },
            ).session(&mut *mongo_session).await?;
    Ok(command)
}

pub async fn list_worker_commands(
    db: &Database,
    pool: &OraclePool,
    worker: &OracleWorker,
    actor: &str,
) -> AppResult<Vec<OracleWorkerCommand>> {
    let transaction_db = db.clone();
    let transaction_pool = pool.clone();
    let transaction_worker = worker.clone();
    let transaction_actor = actor.to_owned();
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                transaction_pool,
                transaction_worker,
                transaction_actor,
            ),
            |mongo_session, context| {
                let (transaction_db, transaction_pool, transaction_worker, transaction_actor) =
                    context;
                Box::pin(async move {
                    let db = &*transaction_db;
                    let pool = &*transaction_pool;
                    let worker = &*transaction_worker;
                    let actor = transaction_actor.as_str();
                    crate::services::api_key_mutation_service::transaction_result(
                        list_worker_commands_in_session(db, mongo_session, pool, worker, actor)
                            .await,
                    )
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn list_worker_commands_in_session(
    db: &Database,
    mongo_session: &mut mongodb::ClientSession,
    pool: &OraclePool,
    worker: &OracleWorker,
    actor: &str,
) -> AppResult<Vec<OracleWorkerCommand>> {
    crate::services::oracle_mutation_service::lock_expected_pool(db, mongo_session, pool, false)
        .await?;
    enrollment::lock_worker_manager(db, mongo_session, actor, pool, worker).await?;

    expire_stale_commands_filtered_in_session(
        db,
        mongo_session,
        &worker.pool_id,
        Some(&worker.worker_label),
        Some(&worker.generation),
    )
    .await?;
    Ok(db.collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS).find(doc! {
        "pool_id": &worker.pool_id, "worker_label": &worker.worker_label, "worker_generation": &worker.generation,
    }).sort(doc! { "created_at": -1 }).limit(100).session(&mut *mongo_session).await?.stream(&mut *mongo_session).try_collect().await?)
}

#[cfg(test)]
mod tests;
