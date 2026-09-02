use std::collections::HashSet;

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument};

use crate::errors::{AppError, AppResult};
use crate::models::oracle_pool::OraclePool;
use crate::models::oracle_session::COLLECTION_NAME as ORACLE_SESSIONS;
use crate::models::oracle_task::COLLECTION_NAME as ORACLE_TASKS;
use crate::models::oracle_worker::{
    COLLECTION_NAME as ORACLE_WORKERS, OracleWorker, OracleWorkerDesiredState, worker_doc_id,
};
use crate::models::oracle_worker_command::{
    COLLECTION_NAME as ORACLE_WORKER_COMMANDS, OracleWorkerCommand, OracleWorkerCommandKind,
    OracleWorkerCommandStatus,
};
use crate::services::oracle_pool_service;

const LABEL_ALLOCATION_ATTEMPTS: usize = 16;
const MAX_CAPABILITIES: usize = 32;
const MAX_METADATA_LEN: usize = 128;
const COMMAND_DEADLINE_HOURS: i64 = 24;
const COMMAND_DELIVERY_LEASE_SECS: i64 = 60;
const COMMAND_RETENTION_DAYS: i64 = 7;
const MAX_COMMAND_DELIVERIES: u32 = 10;
/// Presence newer than this counts as online for `forget` safety checks.
const FORGET_ONLINE_WINDOW_SECS: i64 = 90;

#[derive(Default)]
pub struct WorkerPresenceInput {
    pub worker_label: String,
    pub current_task_id: Option<String>,
    pub script_version: Option<String>,
    pub instance_id: Option<String>,
    pub platform: Option<String>,
    pub capabilities: Vec<String>,
    pub logged_in: Option<bool>,
    pub chrome_alive: Option<bool>,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct CommandReport {
    pub command_id: String,
    pub succeeded: bool,
    pub result_code: Option<String>,
}

fn valid_metadata(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_METADATA_LEN
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub(crate) fn valid_script_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_METADATA_LEN
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
}

fn normalize_capabilities(values: Vec<String>) -> AppResult<Vec<String>> {
    if values.len() > MAX_CAPABILITIES {
        return Err(AppError::ValidationError(format!(
            "worker capabilities exceed {MAX_CAPABILITIES} entries"
        )));
    }
    let mut unique = HashSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if !valid_metadata(value) {
            return Err(AppError::ValidationError(
                "worker capability contains unsupported characters".to_string(),
            ));
        }
        if unique.insert(value.to_string()) {
            normalized.push(value.to_string());
        }
    }
    normalized.sort();
    Ok(normalized)
}

fn optional_metadata(value: Option<String>, field: &str) -> AppResult<Option<String>> {
    value
        .map(|value| {
            if valid_metadata(&value) {
                Ok(value)
            } else {
                Err(AppError::ValidationError(format!(
                    "{field} contains unsupported characters"
                )))
            }
        })
        .transpose()
}

#[derive(Debug)]
pub struct AllocatedWorker {
    pub worker: OracleWorker,
    /// True when an existing unbound (legacy) presence row was taken over
    /// instead of creating a fresh label.
    pub adopted: bool,
}

fn provisioned_worker(pool: &OraclePool, label: &str, now: chrono::DateTime<Utc>) -> OracleWorker {
    OracleWorker {
        id: worker_doc_id(&pool.id, label),
        pool_id: pool.id.clone(),
        worker_label: label.to_string(),
        last_seen_at: now - Duration::days(365),
        current_task_id: None,
        script_version: None,
        page_url: None,
        first_seen_at: None,
        provisioned_at: Some(now),
        instance_id: None,
        platform: None,
        capabilities: Vec::new(),
        desired_state: OracleWorkerDesiredState::Active,
        logged_in: None,
        chrome_alive: None,
        last_error: None,
    }
}

/// Reserve a worker label for a managed installation.
///
/// With `requested_label`, the label is validated and either created, adopted
/// (an existing row with no installation binding, i.e. a legacy worker, is
/// taken over; the legacy process is rejected on its next poll once the new
/// installation binds), or refused with `OracleWorkerLabelUnavailable` when it
/// is bound to another installation. Without a request a unique random label
/// is generated.
pub async fn allocate_worker(
    db: &mongodb::Database,
    pool: &OraclePool,
    requested_label: Option<&str>,
) -> AppResult<AllocatedWorker> {
    let workers = db.collection::<OracleWorker>(ORACLE_WORKERS);
    if let Some(label) = requested_label {
        super::oracle_task_service::validate_worker_label(label)?;
        let now = Utc::now();
        let adopted = workers
            .find_one_and_update(
                doc! {
                    "_id": worker_doc_id(&pool.id, label),
                    "pool_id": &pool.id,
                    "$or": [
                        { "instance_id": null },
                        { "instance_id": { "$exists": false } },
                    ],
                },
                doc! { "$set": { "provisioned_at": bson::DateTime::from_chrono(now) } },
            )
            .with_options(
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?;
        if let Some(worker) = adopted {
            return Ok(AllocatedWorker {
                worker,
                adopted: true,
            });
        }
        let worker = provisioned_worker(pool, label, now);
        return match workers.insert_one(&worker).await {
            Ok(_) => Ok(AllocatedWorker {
                worker,
                adopted: false,
            }),
            Err(error) if oracle_pool_service::is_duplicate_key(&error) => {
                Err(AppError::OracleWorkerLabelUnavailable(format!(
                    "worker label '{label}' is bound to another installation"
                )))
            }
            Err(error) => Err(error.into()),
        };
    }

    for _ in 0..LABEL_ALLOCATION_ATTEMPTS {
        let label = format!("worker-{}", hex::encode(rand::random::<[u8; 5]>()));
        let worker = provisioned_worker(pool, &label, Utc::now());
        match workers.insert_one(&worker).await {
            Ok(_) => {
                return Ok(AllocatedWorker {
                    worker,
                    adopted: false,
                });
            }
            Err(error) if oracle_pool_service::is_duplicate_key(&error) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::OracleWorkerLabelUnavailable(
        "could not allocate a unique worker label".to_string(),
    ))
}

pub async fn report_presence(
    db: &mongodb::Database,
    pool: &OraclePool,
    input: WorkerPresenceInput,
) -> AppResult<OracleWorker> {
    let capabilities = normalize_capabilities(input.capabilities)?;
    let script_version = input
        .script_version
        .map(|value| {
            if valid_script_version(&value) {
                Ok(value)
            } else {
                Err(AppError::ValidationError(
                    "script_version contains unsupported characters".to_string(),
                ))
            }
        })
        .transpose()?;
    let instance_id = optional_metadata(input.instance_id, "instance_id")?;
    let platform = optional_metadata(input.platform, "platform")?;
    let last_error = optional_metadata(input.last_error, "last_error")?;
    let current_task_id = optional_metadata(input.current_task_id, "current_task_id")?;
    ensure_instance_matches(db, pool, &input.worker_label, instance_id.as_deref()).await?;

    let now = Utc::now();
    let mut set = doc! {
        "pool_id": &pool.id,
        "worker_label": &input.worker_label,
        "last_seen_at": bson::DateTime::from_chrono(now),
        "capabilities": &capabilities,
    };
    for (key, value) in [
        ("script_version", script_version),
        ("platform", platform),
        ("last_error", last_error),
    ] {
        match value {
            Some(value) => {
                set.insert(key, value);
            }
            None => {
                set.insert(key, bson::Bson::Null);
            }
        }
    }
    if let Some(instance_id) = instance_id.as_deref() {
        set.insert("instance_id", instance_id);
    }
    match current_task_id {
        Some(task_id) => set.insert("current_task_id", task_id),
        None => set.insert("current_task_id", bson::Bson::Null),
    };
    match input.logged_in {
        Some(value) => set.insert("logged_in", value),
        None => set.insert("logged_in", bson::Bson::Null),
    };
    match input.chrome_alive {
        Some(value) => set.insert("chrome_alive", value),
        None => set.insert("chrome_alive", bson::Bson::Null),
    };

    let mut filter = doc! { "_id": worker_doc_id(&pool.id, &input.worker_label) };
    if let Some(instance_id) = instance_id.as_deref() {
        filter.insert("instance_id", instance_id);
    }
    db.collection::<OracleWorker>(ORACLE_WORKERS)
        .find_one_and_update(
            filter,
            doc! {
                "$set": set,
                "$setOnInsert": {
                    "first_seen_at": bson::DateTime::from_chrono(now),
                    "desired_state": "active",
                },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::Internal("worker presence upsert returned no row".to_string()))
}

pub async fn ensure_instance_matches(
    db: &mongodb::Database,
    pool: &OraclePool,
    worker_label: &str,
    instance_id: Option<&str>,
) -> AppResult<()> {
    super::oracle_task_service::validate_worker_label(worker_label)?;
    let instance_id = instance_id
        .map(|value| optional_metadata(Some(value.to_string()), "instance_id"))
        .transpose()?
        .flatten();
    let workers = db.collection::<OracleWorker>(ORACLE_WORKERS);

    let Some(instance_id) = instance_id else {
        let existing = workers
            .find_one(doc! { "_id": worker_doc_id(&pool.id, worker_label) })
            .await?;
        if existing.is_some_and(|worker| worker.instance_id.is_some()) {
            return Err(AppError::OracleWorkerLabelUnavailable(format!(
                "worker label '{worker_label}' is bound to another installation"
            )));
        }
        return Ok(());
    };

    let now = Utc::now();
    let result = workers
        .find_one_and_update(
            doc! {
                "_id": worker_doc_id(&pool.id, worker_label),
                "$or": [
                    { "instance_id": &instance_id },
                    { "instance_id": null },
                    { "instance_id": { "$exists": false } },
                ],
            },
            doc! {
                "$set": { "instance_id": &instance_id },
                "$setOnInsert": {
                    "pool_id": &pool.id,
                    "worker_label": worker_label,
                    "last_seen_at": bson::DateTime::from_chrono(now),
                    "first_seen_at": bson::DateTime::from_chrono(now),
                    "desired_state": "active",
                },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await;

    match result {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(AppError::Internal(
            "worker instance binding returned no row".to_string(),
        )),
        Err(error) if oracle_pool_service::is_duplicate_key(&error) => {
            Err(AppError::OracleWorkerLabelUnavailable(format!(
                "worker label '{worker_label}' is bound to another installation"
            )))
        }
        Err(error) => Err(error.into()),
    }
}

pub async fn list_workers(db: &mongodb::Database, pool_id: &str) -> AppResult<Vec<OracleWorker>> {
    Ok(db
        .collection::<OracleWorker>(ORACLE_WORKERS)
        .find(doc! { "pool_id": pool_id })
        .with_options(
            FindOptions::builder()
                .sort(doc! { "worker_label": 1 })
                .build(),
        )
        .await?
        .try_collect()
        .await?)
}

pub async fn get_worker(
    db: &mongodb::Database,
    pool_id: &str,
    label: &str,
) -> AppResult<OracleWorker> {
    db.collection::<OracleWorker>(ORACLE_WORKERS)
        .find_one(doc! { "_id": worker_doc_id(pool_id, label), "pool_id": pool_id })
        .await?
        .ok_or_else(|| AppError::OracleWorkerNotFound(label.to_string()))
}

pub async fn accepts_new_tasks(
    db: &mongodb::Database,
    pool_id: &str,
    label: &str,
) -> AppResult<bool> {
    let worker = db
        .collection::<OracleWorker>(ORACLE_WORKERS)
        .find_one(doc! { "_id": worker_doc_id(pool_id, label) })
        .await?;
    Ok(worker.is_none_or(|worker| worker.desired_state == OracleWorkerDesiredState::Active))
}

fn required_capability(kind: &OracleWorkerCommandKind) -> &'static str {
    match kind {
        OracleWorkerCommandKind::Upgrade => "upgrade_v1",
        OracleWorkerCommandKind::SessionImport => "session_import_v1",
        _ => "commands_v1",
    }
}

pub async fn enqueue_command(
    db: &mongodb::Database,
    pool_id: &str,
    actor_user_id: &str,
    worker_label: &str,
    kind: OracleWorkerCommandKind,
    snapshot_id: Option<String>,
    bundle: Option<(String, String)>,
) -> AppResult<OracleWorkerCommand> {
    let worker = get_worker(db, pool_id, worker_label).await?;
    let capability = required_capability(&kind);
    if !worker.capabilities.iter().any(|value| value == capability) {
        return Err(AppError::OracleWorkerCapabilityUnsupported(format!(
            "worker '{}' does not advertise {capability}",
            worker.worker_label
        )));
    }

    let now = Utc::now();
    let (bundle_version, bundle_sha256) = bundle.unzip();
    let command = OracleWorkerCommand {
        id: uuid::Uuid::new_v4().to_string(),
        pool_id: pool_id.to_string(),
        worker_label: worker_label.to_string(),
        kind,
        status: OracleWorkerCommandStatus::Queued,
        created_by_user_id: actor_user_id.to_string(),
        required_capability: Some(capability.to_string()),
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
    db.collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
        .insert_one(&command)
        .await?;
    if let Err(error) = db
        .collection::<Document>(ORACLE_WORKERS)
        .update_one(
            doc! { "_id": &worker.id },
            doc! { "$set": { "desired_state": "draining" } },
        )
        .await
    {
        let _ = db
            .collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
            .delete_one(doc! { "_id": &command.id, "status": "queued" })
            .await;
        return Err(error.into());
    }
    Ok(command)
}

async fn expire_stale_commands(
    db: &mongodb::Database,
    pool_id: &str,
    worker_label: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now();
    let mut filter = doc! {
        "pool_id": pool_id,
        "status": { "$in": ["queued", "delivered"] },
        "deadline_at": { "$lt": bson::DateTime::from_chrono(now) },
    };
    if let Some(label) = worker_label {
        filter.insert("worker_label", label);
    }
    db.collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
        .update_many(
            filter,
            doc! { "$set": {
                "status": "expired",
                "completed_at": bson::DateTime::from_chrono(now),
                "expires_at": bson::DateTime::from_chrono(now + Duration::days(COMMAND_RETENTION_DAYS)),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .await?;
    let mut delivery_filter = doc! {
        "pool_id": pool_id,
        "status": { "$in": ["queued", "delivered"] },
        "delivery_count": { "$gte": i64::from(MAX_COMMAND_DELIVERIES) },
    };
    if let Some(label) = worker_label {
        delivery_filter.insert("worker_label", label);
    }
    db.collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
        .update_many(
            delivery_filter,
            doc! { "$set": {
                "status": "expired",
                "result_code": "delivery_exhausted",
                "completed_at": bson::DateTime::from_chrono(now),
                "expires_at": bson::DateTime::from_chrono(now + Duration::days(COMMAND_RETENTION_DAYS)),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .await?;
    if let Some(label) = worker_label {
        if let Some(latest) = db
            .collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
            .find_one(doc! { "pool_id": pool_id, "worker_label": label })
            .sort(doc! { "created_at": -1 })
            .await?
        {
            reconcile_desired_state(db, &latest).await?;
        }
    } else {
        let draining_workers: Vec<OracleWorker> = db
            .collection::<OracleWorker>(ORACLE_WORKERS)
            .find(doc! { "pool_id": pool_id, "desired_state": "draining" })
            .await?
            .try_collect()
            .await?;
        for worker in draining_workers {
            if let Some(latest) = db
                .collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
                .find_one(doc! {
                    "pool_id": pool_id,
                    "worker_label": &worker.worker_label,
                })
                .sort(doc! { "created_at": -1 })
                .await?
            {
                reconcile_desired_state(db, &latest).await?;
            }
        }
    }
    Ok(())
}

pub async fn deliver_next_command(
    db: &mongodb::Database,
    pool_id: &str,
    worker_label: &str,
    capabilities: &[String],
) -> AppResult<Option<OracleWorkerCommand>> {
    expire_stale_commands(db, pool_id, Some(worker_label)).await?;
    if !capabilities.iter().any(|value| value == "commands_v1") {
        return Ok(None);
    }

    let now = Utc::now();
    let lease = now + Duration::seconds(COMMAND_DELIVERY_LEASE_SECS);
    let result = db
        .collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
        .find_one_and_update(
            doc! {
                "pool_id": pool_id,
                "worker_label": worker_label,
                "deadline_at": { "$gt": bson::DateTime::from_chrono(now) },
                "delivery_count": { "$lt": i64::from(MAX_COMMAND_DELIVERIES) },
                "required_capability": { "$in": capabilities },
                "$or": [
                    { "status": "queued" },
                    {
                        "status": "delivered",
                        "delivery_lease_expires_at": { "$lt": bson::DateTime::from_chrono(now) },
                    },
                ],
            },
            doc! {
                "$set": {
                    "status": "delivered",
                    "delivered_at": bson::DateTime::from_chrono(now),
                    "delivery_lease_expires_at": bson::DateTime::from_chrono(lease),
                    "updated_at": bson::DateTime::from_chrono(now),
                },
                "$inc": { "delivery_count": 1_i64 },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .sort(doc! { "created_at": 1 })
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await;
    match result {
        Ok(command) => Ok(command),
        Err(error) if oracle_pool_service::is_duplicate_key(&error) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn reconcile_desired_state(
    db: &mongodb::Database,
    command: &OracleWorkerCommand,
) -> AppResult<()> {
    let pending = db
        .collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
        .find_one(doc! {
            "pool_id": &command.pool_id,
            "worker_label": &command.worker_label,
            "status": { "$in": ["queued", "delivered"] },
        })
        .sort(doc! { "created_at": 1 })
        .await?;
    let desired = if pending.is_some() {
        OracleWorkerDesiredState::Draining
    } else {
        match (&command.kind, &command.status) {
            (OracleWorkerCommandKind::Drain, OracleWorkerCommandStatus::Succeeded) => {
                OracleWorkerDesiredState::Draining
            }
            (
                OracleWorkerCommandKind::Resume,
                OracleWorkerCommandStatus::Failed | OracleWorkerCommandStatus::Expired,
            ) => OracleWorkerDesiredState::Draining,
            _ => OracleWorkerDesiredState::Active,
        }
    };
    let desired = match desired {
        OracleWorkerDesiredState::Active => "active",
        OracleWorkerDesiredState::Draining => "draining",
    };
    db.collection::<Document>(ORACLE_WORKERS)
        .update_one(
            doc! { "_id": worker_doc_id(&command.pool_id, &command.worker_label) },
            doc! { "$set": { "desired_state": desired } },
        )
        .await?;
    Ok(())
}

pub async fn apply_command_reports(
    db: &mongodb::Database,
    pool_id: &str,
    worker_label: &str,
    reports: Vec<CommandReport>,
) -> AppResult<()> {
    for report in reports.into_iter().take(16) {
        if !valid_metadata(&report.command_id) {
            return Err(AppError::ValidationError(
                "command_id contains unsupported characters".to_string(),
            ));
        }
        let result_code = optional_metadata(report.result_code, "result_code")?;
        let now = Utc::now();
        let commands = db.collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS);
        let Some(existing) = commands
            .find_one(doc! {
                "_id": &report.command_id,
                "pool_id": pool_id,
                "worker_label": worker_label,
            })
            .await?
        else {
            continue;
        };
        if matches!(
            existing.status,
            OracleWorkerCommandStatus::Succeeded
                | OracleWorkerCommandStatus::Failed
                | OracleWorkerCommandStatus::Expired
        ) {
            reconcile_desired_state(db, &existing).await?;
            continue;
        }
        let updated = commands
            .find_one_and_update(
                doc! {
                    "_id": &report.command_id,
                    "pool_id": pool_id,
                    "worker_label": worker_label,
                    "status": "delivered",
                },
                doc! { "$set": {
                    "status": if report.succeeded { "succeeded" } else { "failed" },
                    "result_code": result_code,
                    "completed_at": bson::DateTime::from_chrono(now),
                    "expires_at": bson::DateTime::from_chrono(now + Duration::days(COMMAND_RETENTION_DAYS)),
                    "updated_at": bson::DateTime::from_chrono(now),
                } },
            )
            .with_options(
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?;
        if let Some(updated) = updated {
            reconcile_desired_state(db, &updated).await?;
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct ForgetOutcome {
    pub commands_removed: u64,
    pub sessions_released: u64,
    pub tasks_released: u64,
}

/// Remove a worker's presence row and command history. Refuses an online
/// worker or one with a task in flight unless `force` (a live worker would
/// simply re-register on its next heartbeat). Session affinity owned by the
/// label is released so follow-ups do not wait out the grace window.
pub async fn forget_worker(
    db: &mongodb::Database,
    pool: &OraclePool,
    label: &str,
    force: bool,
) -> AppResult<ForgetOutcome> {
    let worker = get_worker(db, &pool.id, label).await?;
    let now = Utc::now();
    if !force {
        if (now - worker.last_seen_at).num_seconds() <= FORGET_ONLINE_WINDOW_SECS {
            return Err(AppError::Conflict(format!(
                "worker '{label}' is online; stop it first or pass force"
            )));
        }
        let inflight = db
            .collection::<Document>(ORACLE_TASKS)
            .count_documents(doc! {
                "pool_id": &pool.id,
                "status": "dispatched",
                "assigned_worker_id": label,
            })
            .await?;
        if inflight > 0 {
            return Err(AppError::Conflict(format!(
                "worker '{label}' has a task in flight; wait for it to settle or pass force"
            )));
        }
    }
    let commands_removed = db
        .collection::<Document>(ORACLE_WORKER_COMMANDS)
        .delete_many(doc! { "pool_id": &pool.id, "worker_label": label })
        .await?
        .deleted_count;
    let sessions_released = db
        .collection::<Document>(ORACLE_SESSIONS)
        .update_many(
            doc! { "pool_id": &pool.id, "owner_worker_label": label },
            doc! { "$set": { "owner_worker_label": null, "updated_at": bson::DateTime::from_chrono(now) } },
        )
        .await?
        .modified_count;
    let tasks_released = db
        .collection::<Document>(ORACLE_TASKS)
        .update_many(
            doc! { "pool_id": &pool.id, "status": "queued", "required_worker_label": label },
            doc! {
                "$set": { "phase": "affinity_released_by_forget", "updated_at": bson::DateTime::from_chrono(now) },
                "$unset": { "required_worker_label": "" },
            },
        )
        .await?
        .modified_count;
    db.collection::<OracleWorker>(ORACLE_WORKERS)
        .delete_one(doc! { "_id": &worker.id })
        .await?;
    Ok(ForgetOutcome {
        commands_removed,
        sessions_released,
        tasks_released,
    })
}

pub async fn list_commands(
    db: &mongodb::Database,
    pool_id: &str,
    worker_label: Option<&str>,
) -> AppResult<Vec<OracleWorkerCommand>> {
    expire_stale_commands(db, pool_id, worker_label).await?;
    let mut filter = doc! { "pool_id": pool_id };
    if let Some(label) = worker_label {
        filter.insert("worker_label", label);
    }
    Ok(db
        .collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
        .find(filter)
        .with_options(
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(100)
                .build(),
        )
        .await?
        .try_collect()
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::oracle_pool::OraclePoolVisibility;
    use crate::test_utils::connect_test_database;
    use mongodb::{IndexModel, options::IndexOptions};

    fn pool() -> OraclePool {
        let now = Utc::now();
        OraclePool {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            slug: format!("worker-test-{}", uuid::Uuid::new_v4()),
            name: "Worker test".to_string(),
            description: None,
            visibility: OraclePoolVisibility::Private,
            worker_token_hash: "a".repeat(64),
            chatgpt_project_url: None,
            default_model_label: None,
            allow_extract: false,
            max_workers: 2,
            max_queue_length: 10,
            per_user_max_inflight: 2,
            task_timeout_secs: 60,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn allocation_is_unique_and_command_delivery_is_at_least_once() {
        let Some(db) = connect_test_database("oracle_worker_commands").await else {
            return;
        };
        let pool = pool();
        let first = allocate_worker(&db, &pool, None).await.unwrap().worker;
        let second = allocate_worker(&db, &pool, None).await.unwrap().worker;
        assert_ne!(first.worker_label, second.worker_label);

        let worker = report_presence(
            &db,
            &pool,
            WorkerPresenceInput {
                worker_label: first.worker_label.clone(),
                capabilities: vec!["commands_v1".to_string(), "upgrade_v1".to_string()],
                logged_in: Some(true),
                chrome_alive: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(worker.logged_in, Some(true));

        let command = enqueue_command(
            &db,
            &pool.id,
            &pool.user_id,
            &first.worker_label,
            OracleWorkerCommandKind::Drain,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            !accepts_new_tasks(&db, &pool.id, &first.worker_label)
                .await
                .unwrap()
        );

        let capabilities = vec!["commands_v1".to_string()];
        let delivered = deliver_next_command(&db, &pool.id, &first.worker_label, &capabilities)
            .await
            .unwrap()
            .expect("command delivery");
        assert_eq!(delivered.id, command.id);
        assert_eq!(delivered.delivery_count, 1);

        db.collection::<Document>(ORACLE_WORKER_COMMANDS)
            .update_one(
                doc! { "_id": &command.id },
                doc! { "$set": { "delivery_lease_expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)) } },
            )
            .await
            .unwrap();
        let redelivered = deliver_next_command(&db, &pool.id, &first.worker_label, &capabilities)
            .await
            .unwrap()
            .expect("redelivery");
        assert_eq!(redelivered.id, command.id);
        assert_eq!(redelivered.delivery_count, 2);

        apply_command_reports(
            &db,
            &pool.id,
            &first.worker_label,
            vec![CommandReport {
                command_id: command.id.clone(),
                succeeded: true,
                result_code: Some("drained".to_string()),
            }],
        )
        .await
        .unwrap();
        apply_command_reports(
            &db,
            &pool.id,
            &first.worker_label,
            vec![CommandReport {
                command_id: command.id.clone(),
                succeeded: true,
                result_code: Some("drained".to_string()),
            }],
        )
        .await
        .unwrap();
        let commands = list_commands(&db, &pool.id, Some(&first.worker_label))
            .await
            .unwrap();
        assert_eq!(commands[0].status, OracleWorkerCommandStatus::Succeeded);
        assert_eq!(commands[0].result_code.as_deref(), Some("drained"));
        assert!(
            !accepts_new_tasks(&db, &pool.id, &first.worker_label)
                .await
                .unwrap()
        );

        let resume = enqueue_command(
            &db,
            &pool.id,
            &pool.user_id,
            &first.worker_label,
            OracleWorkerCommandKind::Resume,
            None,
            None,
        )
        .await
        .unwrap();
        deliver_next_command(&db, &pool.id, &first.worker_label, &capabilities)
            .await
            .unwrap()
            .expect("resume delivery");
        apply_command_reports(
            &db,
            &pool.id,
            &first.worker_label,
            vec![
                CommandReport {
                    command_id: uuid::Uuid::new_v4().to_string(),
                    succeeded: true,
                    result_code: Some("stale_report".to_string()),
                },
                CommandReport {
                    command_id: resume.id,
                    succeeded: true,
                    result_code: Some("resumed".to_string()),
                },
            ],
        )
        .await
        .unwrap();
        assert!(
            accepts_new_tasks(&db, &pool.id, &first.worker_label)
                .await
                .unwrap()
        );

        db.drop().await.ok();
    }

    #[tokio::test]
    async fn commands_require_advertised_capability() {
        let Some(db) = connect_test_database("oracle_worker_capability").await else {
            return;
        };
        let pool = pool();
        let worker = allocate_worker(&db, &pool, None).await.unwrap().worker;
        let error = enqueue_command(
            &db,
            &pool.id,
            &pool.user_id,
            &worker.worker_label,
            OracleWorkerCommandKind::Restart,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            AppError::OracleWorkerCapabilityUnsupported(_)
        ));
        db.drop().await.ok();
    }

    #[tokio::test]
    async fn pool_wide_expiry_reconciles_worker_desired_state() {
        let Some(db) = connect_test_database("oracle_worker_pool_expiry").await else {
            return;
        };
        let pool = pool();
        let worker = allocate_worker(&db, &pool, None).await.unwrap().worker;
        report_presence(
            &db,
            &pool,
            WorkerPresenceInput {
                worker_label: worker.worker_label.clone(),
                capabilities: vec!["commands_v1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let command = enqueue_command(
            &db,
            &pool.id,
            &pool.user_id,
            &worker.worker_label,
            OracleWorkerCommandKind::Restart,
            None,
            None,
        )
        .await
        .unwrap();
        db.collection::<Document>(ORACLE_WORKER_COMMANDS)
            .update_one(
                doc! { "_id": &command.id },
                doc! { "$set": {
                    "deadline_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)),
                } },
            )
            .await
            .unwrap();

        list_commands(&db, &pool.id, None).await.unwrap();

        assert!(
            accepts_new_tasks(&db, &pool.id, &worker.worker_label)
                .await
                .unwrap()
        );
        db.drop().await.ok();
    }

    #[tokio::test]
    async fn allocated_label_binds_to_one_installation() {
        let Some(db) = connect_test_database("oracle_worker_instance_binding").await else {
            return;
        };
        let pool = pool();
        let worker = allocate_worker(&db, &pool, None).await.unwrap().worker;
        ensure_instance_matches(&db, &pool, &worker.worker_label, Some("install-a"))
            .await
            .unwrap();
        ensure_instance_matches(&db, &pool, &worker.worker_label, Some("install-a"))
            .await
            .unwrap();
        let mismatch = ensure_instance_matches(&db, &pool, &worker.worker_label, Some("install-b"))
            .await
            .unwrap_err();
        assert!(matches!(
            mismatch,
            AppError::OracleWorkerLabelUnavailable(_)
        ));
        let missing = ensure_instance_matches(&db, &pool, &worker.worker_label, None)
            .await
            .unwrap_err();
        assert!(matches!(missing, AppError::OracleWorkerLabelUnavailable(_)));
        db.drop().await.ok();
    }

    #[tokio::test]
    async fn delivery_rechecks_command_specific_capability() {
        let Some(db) = connect_test_database("oracle_worker_delivery_capability").await else {
            return;
        };
        let pool = pool();
        let worker = allocate_worker(&db, &pool, None).await.unwrap().worker;
        report_presence(
            &db,
            &pool,
            WorkerPresenceInput {
                worker_label: worker.worker_label.clone(),
                capabilities: vec!["commands_v1".to_string(), "upgrade_v1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        enqueue_command(
            &db,
            &pool.id,
            &pool.user_id,
            &worker.worker_label,
            OracleWorkerCommandKind::Upgrade,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            deliver_next_command(
                &db,
                &pool.id,
                &worker.worker_label,
                &["commands_v1".to_string()],
            )
            .await
            .unwrap()
            .is_none()
        );
        assert!(
            deliver_next_command(
                &db,
                &pool.id,
                &worker.worker_label,
                &["commands_v1".to_string(), "upgrade_v1".to_string()],
            )
            .await
            .unwrap()
            .is_some()
        );
        db.drop().await.ok();
    }

    #[tokio::test]
    async fn only_one_command_can_be_delivered_per_worker() {
        let Some(db) = connect_test_database("oracle_worker_command_slot").await else {
            return;
        };
        db.collection::<Document>(ORACLE_WORKER_COMMANDS)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "pool_id": 1, "worker_label": 1, "status": 1 })
                    .options(
                        IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(doc! { "status": "delivered" })
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
        let pool = pool();
        let worker = allocate_worker(&db, &pool, None).await.unwrap().worker;
        report_presence(
            &db,
            &pool,
            WorkerPresenceInput {
                worker_label: worker.worker_label.clone(),
                capabilities: vec!["commands_v1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let first = enqueue_command(
            &db,
            &pool.id,
            &pool.user_id,
            &worker.worker_label,
            OracleWorkerCommandKind::Drain,
            None,
            None,
        )
        .await
        .unwrap();
        let second = enqueue_command(
            &db,
            &pool.id,
            &pool.user_id,
            &worker.worker_label,
            OracleWorkerCommandKind::RelaunchBrowser,
            None,
            None,
        )
        .await
        .unwrap();
        let capabilities = vec!["commands_v1".to_string()];
        let delivered = deliver_next_command(&db, &pool.id, &worker.worker_label, &capabilities)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivered.id, first.id);
        assert!(
            deliver_next_command(&db, &pool.id, &worker.worker_label, &capabilities,)
                .await
                .unwrap()
                .is_none()
        );
        apply_command_reports(
            &db,
            &pool.id,
            &worker.worker_label,
            vec![CommandReport {
                command_id: first.id,
                succeeded: true,
                result_code: Some("drained".to_string()),
            }],
        )
        .await
        .unwrap();
        let delivered = deliver_next_command(&db, &pool.id, &worker.worker_label, &capabilities)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivered.id, second.id);
        db.drop().await.ok();
    }
    #[tokio::test]
    async fn requested_label_is_created_adopted_or_refused() {
        let Some(db) = connect_test_database("oracle_worker_requested_label").await else {
            return;
        };
        let pool = pool();
        let created = allocate_worker(&db, &pool, Some("share-account-8"))
            .await
            .unwrap();
        assert_eq!(created.worker.worker_label, "share-account-8");
        assert!(!created.adopted);

        // Unbound (never heartbeated with an installation) rows can be adopted.
        let adopted = allocate_worker(&db, &pool, Some("share-account-8"))
            .await
            .unwrap();
        assert!(adopted.adopted);
        assert!(adopted.worker.provisioned_at.is_some());

        // A legacy presence row (created by polling, no instance) is adoptable too.
        report_presence(
            &db,
            &pool,
            WorkerPresenceInput {
                worker_label: "share_account_9".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let legacy = allocate_worker(&db, &pool, Some("share_account_9"))
            .await
            .unwrap();
        assert!(legacy.adopted);

        // Once bound to an installation the label is refused.
        ensure_instance_matches(&db, &pool, "share-account-8", Some("install-a"))
            .await
            .unwrap();
        let refused = allocate_worker(&db, &pool, Some("share-account-8"))
            .await
            .unwrap_err();
        assert!(matches!(refused, AppError::OracleWorkerLabelUnavailable(_)));

        let invalid = allocate_worker(&db, &pool, Some("bad label!"))
            .await
            .unwrap_err();
        assert!(matches!(invalid, AppError::ValidationError(_)));
        db.drop().await.ok();
    }
    #[tokio::test]
    async fn forget_refuses_online_workers_unless_forced_and_cleans_up() {
        let Some(db) = connect_test_database("oracle_worker_forget").await else {
            return;
        };
        let pool = pool();
        let worker = allocate_worker(&db, &pool, Some("stale-1"))
            .await
            .unwrap()
            .worker;
        report_presence(
            &db,
            &pool,
            WorkerPresenceInput {
                worker_label: worker.worker_label.clone(),
                capabilities: vec!["commands_v1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        enqueue_command(
            &db,
            &pool.id,
            &pool.user_id,
            &worker.worker_label,
            OracleWorkerCommandKind::Drain,
            None,
            None,
        )
        .await
        .unwrap();

        let refused = forget_worker(&db, &pool, &worker.worker_label, false)
            .await
            .unwrap_err();
        assert!(matches!(refused, AppError::Conflict(_)));

        let outcome = forget_worker(&db, &pool, &worker.worker_label, true)
            .await
            .unwrap();
        assert_eq!(outcome.commands_removed, 1);
        assert!(
            list_commands(&db, &pool.id, Some(&worker.worker_label))
                .await
                .unwrap()
                .is_empty()
        );
        let gone = get_worker(&db, &pool.id, &worker.worker_label)
            .await
            .unwrap_err();
        assert!(matches!(gone, AppError::OracleWorkerNotFound(_)));

        // A stale (offline) worker is forgotten without force.
        let stale = allocate_worker(&db, &pool, Some("stale-2"))
            .await
            .unwrap()
            .worker;
        forget_worker(&db, &pool, &stale.worker_label, false)
            .await
            .unwrap();
        db.drop().await.ok();
    }
}
