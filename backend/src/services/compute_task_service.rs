//! MongoDB-backed compute task queue and worker scheduler.

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

use crate::errors::{AppError, AppResult};
use crate::models::compute_pool::{ComputePool, ComputeSchedulingPolicy};
use crate::models::compute_task::{
    COLLECTION_NAME as COMPUTE_TASKS, ComputeTask, ComputeTaskStatus,
};
use crate::models::compute_worker::{
    COLLECTION_NAME as COMPUTE_WORKERS, ComputeWorker, worker_doc_id,
};
use crate::services::compute_pool_service;

pub const WORKER_RECENT_SECS: i64 = 120;
const MAX_KIND_LEN: usize = 64;
const MAX_MODEL_LEN: usize = 160;
const MAX_CLIENT_REF_LEN: usize = 128;
const MAX_PHASE_LEN: usize = 80;
const MAX_PHASE_DETAIL_LEN: usize = 500;
const MAX_WORKER_LABEL_LEN: usize = 64;
const MAX_WORKER_VERSION_LEN: usize = 80;
const MAX_BACKEND_LEN: usize = 64;
const MAX_HOST_KIND_LEN: usize = 64;
const MAX_GPU_NAME_LEN: usize = 128;
const MAX_MODELS: usize = 200;
const MAX_MODEL_NAME_LEN: usize = 160;
const MAX_COMPUTE_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_COMPUTE_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SubmitterIdentity {
    pub user_id: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SubmitComputeTaskInput {
    pub kind: String,
    pub model: String,
    pub input: serde_json::Value,
    pub priority: i32,
    pub client_ref: Option<String>,
}

#[derive(Debug)]
pub struct SubmitComputeTaskOutcome {
    pub task: ComputeTask,
    pub queue_position: u64,
    pub deduplicated: bool,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct WorkerCapabilities {
    pub node_id: Option<String>,
    pub host_kind: Option<String>,
    pub gpu_name: Option<String>,
    pub backend: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    pub vram_total_mb: Option<u64>,
    pub vram_free_mb: Option<u64>,
    pub max_concurrency: Option<u32>,
    pub current_inflight: Option<u32>,
    pub avg_tokens_per_sec: Option<f64>,
    pub worker_version: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct WorkerTaskPayload {
    pub task_id: String,
    pub kind: String,
    pub model: String,
    pub input: serde_json::Value,
    pub priority: i32,
    pub assigned_worker: String,
    pub submitted_at: String,
}

#[derive(Debug, serde::Serialize)]
pub struct PoolStatus {
    pub pool_id: String,
    pub slug: String,
    pub queued: u64,
    pub dispatched: u64,
    pub max_workers: u32,
    pub active_workers: Vec<ComputeWorker>,
}

fn validate_submit_input(input: &SubmitComputeTaskInput) -> AppResult<()> {
    if input.kind.trim().is_empty() || input.kind.len() > MAX_KIND_LEN {
        return Err(AppError::ValidationError(format!(
            "kind must be 1-{MAX_KIND_LEN} chars"
        )));
    }
    if input.model.trim().is_empty() || input.model.len() > MAX_MODEL_LEN {
        return Err(AppError::ValidationError(format!(
            "model must be 1-{MAX_MODEL_LEN} chars"
        )));
    }
    if input
        .client_ref
        .as_deref()
        .is_some_and(|c| c.is_empty() || c.len() > MAX_CLIENT_REF_LEN)
    {
        return Err(AppError::ValidationError(format!(
            "client_ref must be 1-{MAX_CLIENT_REF_LEN} chars"
        )));
    }
    validate_json_size("input", &input.input, MAX_COMPUTE_INPUT_BYTES)?;
    Ok(())
}

fn validate_json_size(label: &str, value: &serde_json::Value, max_bytes: usize) -> AppResult<()> {
    let size = serde_json::to_vec(value)
        .map_err(|e| AppError::Internal(format!("failed to size compute {label}: {e}")))?
        .len();
    if size > max_bytes {
        return Err(AppError::ComputePayloadTooLarge(format!(
            "{label} is {size} bytes; limit is {max_bytes} bytes"
        )));
    }
    Ok(())
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect()
    }
}

fn validate_worker_label(label: &str) -> AppResult<()> {
    let ok = !label.is_empty()
        && label.len() <= MAX_WORKER_LABEL_LEN
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(AppError::ValidationError(format!(
            "worker label must be 1-{MAX_WORKER_LABEL_LEN} chars of letters, digits, '-', '_'"
        )));
    }
    Ok(())
}

fn normalize_models(models: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for model in models.iter().take(MAX_MODELS) {
        let model = model.trim();
        if model.is_empty() {
            continue;
        }
        let model = truncate_chars(model, MAX_MODEL_NAME_LEN);
        if !out.contains(&model) {
            out.push(model);
        }
    }
    out
}

async fn count_tasks(db: &mongodb::Database, filter: Document) -> AppResult<u64> {
    Ok(db
        .collection::<ComputeTask>(COMPUTE_TASKS)
        .count_documents(filter)
        .await?)
}

async fn enforce_submit_quotas(
    db: &mongodb::Database,
    pool: &ComputePool,
    submitter: &SubmitterIdentity,
) -> AppResult<()> {
    let queued = count_tasks(db, doc! { "pool_id": &pool.id, "status": "queued" }).await?;
    if queued >= u64::from(pool.max_queue_length) {
        return Err(AppError::ComputeQueueFull(format!(
            "pool '{}' already has {queued} queued tasks",
            pool.slug
        )));
    }
    let inflight = count_tasks(
        db,
        doc! {
            "pool_id": &pool.id,
            "submitter_user_id": &submitter.user_id,
            "status": { "$in": ["queued", "dispatched"] },
        },
    )
    .await?;
    if inflight >= u64::from(pool.per_user_max_inflight) {
        return Err(AppError::ComputeQuotaExceeded(format!(
            "you already have {inflight} tasks in flight in pool '{}' (limit {})",
            pool.slug, pool.per_user_max_inflight
        )));
    }
    Ok(())
}

pub async fn submit_task(
    db: &mongodb::Database,
    pool: &ComputePool,
    submitter: &SubmitterIdentity,
    input: SubmitComputeTaskInput,
) -> AppResult<SubmitComputeTaskOutcome> {
    validate_submit_input(&input)?;
    enforce_submit_quotas(db, pool, submitter).await?;

    let now = Utc::now();
    let task = ComputeTask {
        id: uuid::Uuid::new_v4().to_string(),
        pool_id: pool.id.clone(),
        submitter_user_id: submitter.user_id.clone(),
        api_key_id: submitter.api_key_id.clone(),
        api_key_name: submitter.api_key_name.clone(),
        kind: input.kind,
        model: input.model,
        priority: input.priority,
        input: input.input,
        client_ref: input.client_ref,
        status: ComputeTaskStatus::Queued,
        phase: None,
        phase_detail: None,
        phase_at: None,
        assigned_worker_id: None,
        dispatched_at: None,
        lease_expires_at: None,
        output: None,
        failure_reason: None,
        completed_at: None,
        expires_at: None,
        created_at: now,
        updated_at: now,
    };

    let insert = db
        .collection::<ComputeTask>(COMPUTE_TASKS)
        .insert_one(&task)
        .await;
    if let Err(e) = insert {
        if compute_pool_service::is_duplicate_key(&e)
            && let Some(client_ref) = &task.client_ref
        {
            let existing = db
                .collection::<ComputeTask>(COMPUTE_TASKS)
                .find_one(doc! {
                    "pool_id": &task.pool_id,
                    "submitter_user_id": &submitter.user_id,
                    "client_ref": client_ref,
                })
                .await?;
            if let Some(existing) = existing {
                let position = queue_position(db, &existing).await?;
                return Ok(SubmitComputeTaskOutcome {
                    task: existing,
                    queue_position: position,
                    deduplicated: true,
                });
            }
        }
        return Err(e.into());
    }

    let position = queue_position(db, &task).await?;
    Ok(SubmitComputeTaskOutcome {
        task,
        queue_position: position,
        deduplicated: false,
    })
}

async fn queue_position(db: &mongodb::Database, task: &ComputeTask) -> AppResult<u64> {
    if task.status != ComputeTaskStatus::Queued {
        return Ok(0);
    }
    let ahead = count_tasks(
        db,
        doc! {
            "pool_id": &task.pool_id,
            "status": "queued",
            "$or": [
                { "priority": { "$gt": task.priority } },
                {
                    "priority": task.priority,
                    "created_at": { "$lt": bson::DateTime::from_chrono(task.created_at) },
                },
            ],
        },
    )
    .await?;
    Ok(ahead + 1)
}

pub async fn get_task_for_consumer(
    db: &mongodb::Database,
    actor_user_id: &str,
    task_id: &str,
) -> AppResult<(ComputeTask, u64)> {
    let task = db
        .collection::<ComputeTask>(COMPUTE_TASKS)
        .find_one(doc! { "_id": task_id })
        .await?
        .ok_or_else(|| AppError::ComputeTaskNotFound(task_id.to_string()))?;

    if task.submitter_user_id != actor_user_id {
        let pool = compute_pool_service::get_pool(db, &task.pool_id).await?;
        compute_pool_service::ensure_can_manage(db, actor_user_id, &pool)
            .await
            .map_err(|_| AppError::ComputeTaskNotFound(task_id.to_string()))?;
    }

    let position = queue_position(db, &task).await?;
    Ok((task, position))
}

pub async fn cancel_task(
    db: &mongodb::Database,
    actor_user_id: &str,
    task_id: &str,
    retention_days: u32,
) -> AppResult<ComputeTask> {
    let (task, _) = get_task_for_consumer(db, actor_user_id, task_id).await?;
    match task.status {
        ComputeTaskStatus::Cancelled => return Ok(task),
        ComputeTaskStatus::Completed | ComputeTaskStatus::Failed => {
            return Err(AppError::Conflict(format!(
                "task is already {}",
                task.status.as_str()
            )));
        }
        ComputeTaskStatus::Queued | ComputeTaskStatus::Dispatched => {}
    }

    let now = Utc::now();
    let updated = db
        .collection::<ComputeTask>(COMPUTE_TASKS)
        .find_one_and_update(
            doc! { "_id": task_id, "status": { "$in": ["queued", "dispatched"] } },
            doc! { "$set": {
                "status": "cancelled",
                "completed_at": bson::DateTime::from_chrono(now),
                "expires_at": bson::DateTime::from_chrono(terminal_expiry(retention_days)),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;

    updated.ok_or_else(|| AppError::Conflict("task reached a terminal state first".to_string()))
}

fn terminal_expiry(retention_days: u32) -> chrono::DateTime<Utc> {
    Utc::now() + Duration::days(i64::from(retention_days))
}

async fn requeue_expired_leases(db: &mongodb::Database, pool_id: &str) -> AppResult<u64> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let result = db
        .collection::<ComputeTask>(COMPUTE_TASKS)
        .update_many(
            doc! {
                "pool_id": pool_id,
                "status": "dispatched",
                "lease_expires_at": { "$lt": now },
            },
            doc! {
                "$set": {
                    "status": "queued",
                    "phase": "requeued_after_lease_expiry",
                    "updated_at": now,
                },
                "$unset": {
                    "assigned_worker_id": "",
                    "dispatched_at": "",
                    "lease_expires_at": "",
                },
            },
        )
        .await?;
    Ok(result.modified_count)
}

async fn upsert_worker_presence(
    db: &mongodb::Database,
    pool: &ComputePool,
    worker_label: &str,
    current_task_id: Option<&str>,
    capabilities: Option<&WorkerCapabilities>,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let mut set = doc! {
        "pool_id": &pool.id,
        "worker_label": worker_label,
        "last_seen_at": now,
    };
    match current_task_id {
        Some(task_id) => set.insert("current_task_id", task_id),
        None => set.insert("current_task_id", bson::Bson::Null),
    };
    if let Some(c) = capabilities {
        if let Some(node_id) = &c.node_id {
            set.insert("node_id", truncate_chars(node_id, 80));
        }
        if let Some(host_kind) = &c.host_kind {
            set.insert("host_kind", truncate_chars(host_kind, MAX_HOST_KIND_LEN));
        }
        if let Some(gpu_name) = &c.gpu_name {
            set.insert("gpu_name", truncate_chars(gpu_name, MAX_GPU_NAME_LEN));
        }
        if let Some(backend) = &c.backend {
            set.insert("backend", truncate_chars(backend, MAX_BACKEND_LEN));
        }
        let models = bson::to_bson(&normalize_models(&c.models))
            .map_err(|e| AppError::Internal(format!("failed to serialize worker models: {e}")))?;
        set.insert("models", models);
        if let Some(v) = c.vram_total_mb {
            set.insert("vram_total_mb", v as i64);
        }
        if let Some(v) = c.vram_free_mb {
            set.insert("vram_free_mb", v as i64);
        }
        if let Some(v) = c.max_concurrency {
            set.insert("max_concurrency", i64::from(v));
        }
        if let Some(v) = c.current_inflight {
            set.insert("current_inflight", i64::from(v));
        }
        if let Some(v) = c.avg_tokens_per_sec
            && v.is_finite()
        {
            set.insert("avg_tokens_per_sec", v);
        }
        if let Some(v) = &c.worker_version {
            set.insert("worker_version", truncate_chars(v, MAX_WORKER_VERSION_LEN));
        }
    }

    db.collection::<Document>(COMPUTE_WORKERS)
        .update_one(
            doc! { "_id": worker_doc_id(&pool.id, worker_label) },
            doc! {
                "$set": set,
                "$setOnInsert": { "first_seen_at": now },
            },
        )
        .upsert(true)
        .await?;
    Ok(())
}

fn task_filter_for_worker(pool: &ComputePool, worker: &WorkerCapabilities) -> Document {
    let mut filter = doc! {
        "pool_id": &pool.id,
        "status": "queued",
    };
    let models = normalize_models(&worker.models);
    let accepts_any_model = models.iter().any(|model| model == "*");
    if pool.scheduling_policy == ComputeSchedulingPolicy::ModelFit && !accepts_any_model {
        filter.insert("model", doc! { "$in": models });
    }
    filter
}

pub async fn claim_task(
    db: &mongodb::Database,
    pool: &ComputePool,
    worker_label: &str,
    capabilities: WorkerCapabilities,
) -> AppResult<Option<WorkerTaskPayload>> {
    validate_worker_label(worker_label)?;
    requeue_expired_leases(db, &pool.id).await?;

    let now = Utc::now();
    let lease = now + Duration::seconds(pool.task_timeout_secs as i64);

    let resumed = db
        .collection::<ComputeTask>(COMPUTE_TASKS)
        .find_one_and_update(
            doc! {
                "pool_id": &pool.id,
                "status": "dispatched",
                "assigned_worker_id": worker_label,
            },
            doc! { "$set": {
                "lease_expires_at": bson::DateTime::from_chrono(lease),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    if let Some(task) = resumed {
        upsert_worker_presence(db, pool, worker_label, Some(&task.id), Some(&capabilities)).await?;
        return Ok(Some(worker_payload(&task, worker_label)));
    }

    upsert_worker_presence(db, pool, worker_label, None, Some(&capabilities)).await?;

    let dispatched = count_tasks(db, doc! { "pool_id": &pool.id, "status": "dispatched" }).await?;
    if dispatched >= u64::from(pool.max_workers) {
        return Ok(None);
    }

    let claimed = db
        .collection::<ComputeTask>(COMPUTE_TASKS)
        .find_one_and_update(
            task_filter_for_worker(pool, &capabilities),
            doc! { "$set": {
                "status": "dispatched",
                "assigned_worker_id": worker_label,
                "dispatched_at": bson::DateTime::from_chrono(now),
                "lease_expires_at": bson::DateTime::from_chrono(lease),
                "phase": "dispatched",
                "phase_at": bson::DateTime::from_chrono(now),
                "updated_at": bson::DateTime::from_chrono(now),
            } },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .sort(doc! { "priority": -1, "created_at": 1 })
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;

    match claimed {
        Some(task) => {
            upsert_worker_presence(db, pool, worker_label, Some(&task.id), Some(&capabilities))
                .await?;
            Ok(Some(worker_payload(&task, worker_label)))
        }
        None => Ok(None),
    }
}

fn worker_payload(task: &ComputeTask, worker_label: &str) -> WorkerTaskPayload {
    WorkerTaskPayload {
        task_id: task.id.clone(),
        kind: task.kind.clone(),
        model: task.model.clone(),
        input: task.input.clone(),
        priority: task.priority,
        assigned_worker: worker_label.to_string(),
        submitted_at: task.created_at.to_rfc3339(),
    }
}

#[derive(Debug, PartialEq)]
pub enum AckOutcome {
    Ok,
    Cancelled,
}

pub async fn worker_ack(
    db: &mongodb::Database,
    pool: &ComputePool,
    worker_label: &str,
    task_id: &str,
    phase: Option<&str>,
    phase_detail: Option<&str>,
    capabilities: Option<WorkerCapabilities>,
) -> AppResult<AckOutcome> {
    validate_worker_label(worker_label)?;
    let now = Utc::now();
    let lease = now + Duration::seconds(pool.task_timeout_secs as i64);
    let mut set = doc! {
        "lease_expires_at": bson::DateTime::from_chrono(lease),
        "updated_at": bson::DateTime::from_chrono(now),
    };
    if let Some(phase) = phase {
        set.insert("phase", truncate_chars(phase, MAX_PHASE_LEN));
        set.insert("phase_at", bson::DateTime::from_chrono(now));
    }
    if let Some(detail) = phase_detail {
        set.insert("phase_detail", truncate_chars(detail, MAX_PHASE_DETAIL_LEN));
    }

    let updated = db
        .collection::<ComputeTask>(COMPUTE_TASKS)
        .update_one(
            doc! {
                "_id": task_id,
                "pool_id": &pool.id,
                "status": "dispatched",
                "assigned_worker_id": worker_label,
            },
            doc! { "$set": set },
        )
        .await?;

    upsert_worker_presence(
        db,
        pool,
        worker_label,
        (updated.matched_count > 0).then_some(task_id),
        capabilities.as_ref(),
    )
    .await?;

    if updated.matched_count == 0 {
        return Ok(AckOutcome::Cancelled);
    }
    Ok(AckOutcome::Ok)
}

#[derive(Debug, PartialEq)]
pub enum ResultOutcome {
    Completed,
    Failed,
    Ignored,
}

pub async fn worker_submit_result(
    db: &mongodb::Database,
    pool: &ComputePool,
    worker_label: &str,
    task_id: &str,
    output: Option<serde_json::Value>,
    failure_reason: Option<&str>,
    retention_days: u32,
) -> AppResult<ResultOutcome> {
    validate_worker_label(worker_label)?;
    let now = Utc::now();
    let is_failure = failure_reason.is_some();
    let mut set = doc! {
        "status": if is_failure { "failed" } else { "completed" },
        "completed_at": bson::DateTime::from_chrono(now),
        "expires_at": bson::DateTime::from_chrono(terminal_expiry(retention_days)),
        "updated_at": bson::DateTime::from_chrono(now),
    };
    if let Some(output) = output {
        validate_json_size("output", &output, MAX_COMPUTE_OUTPUT_BYTES)?;
        let output = bson::to_bson(&output)
            .map_err(|e| AppError::Internal(format!("failed to serialize compute output: {e}")))?;
        set.insert("output", output);
    }
    if let Some(reason) = failure_reason {
        set.insert("failure_reason", truncate_chars(reason, 500));
    }

    let updated = db
        .collection::<ComputeTask>(COMPUTE_TASKS)
        .find_one_and_update(
            doc! {
                "_id": task_id,
                "pool_id": &pool.id,
                "status": "dispatched",
                "assigned_worker_id": worker_label,
            },
            doc! {
                "$set": set,
                "$unset": { "lease_expires_at": "" },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;

    upsert_worker_presence(db, pool, worker_label, None, None).await?;
    match updated {
        Some(_) if is_failure => Ok(ResultOutcome::Failed),
        Some(_) => Ok(ResultOutcome::Completed),
        None => Ok(ResultOutcome::Ignored),
    }
}

pub async fn pool_status(db: &mongodb::Database, pool: &ComputePool) -> AppResult<PoolStatus> {
    let queued = count_tasks(db, doc! { "pool_id": &pool.id, "status": "queued" }).await?;
    let dispatched = count_tasks(db, doc! { "pool_id": &pool.id, "status": "dispatched" }).await?;
    let since = bson::DateTime::from_chrono(Utc::now() - Duration::seconds(WORKER_RECENT_SECS));
    let active_workers = db
        .collection::<ComputeWorker>(COMPUTE_WORKERS)
        .find(doc! {
            "pool_id": &pool.id,
            "last_seen_at": { "$gte": since },
        })
        .await?
        .try_collect()
        .await?;
    Ok(PoolStatus {
        pool_id: pool.id.clone(),
        slug: pool.slug.clone(),
        queued,
        dispatched,
        max_workers: pool.max_workers,
        active_workers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::compute_pool::ComputePoolVisibility;
    use crate::services::compute_pool_service::{CreateComputePoolInput, create_pool};
    use crate::test_utils::connect_test_database;
    use serde_json::json;

    fn submitter(user_id: &str) -> SubmitterIdentity {
        SubmitterIdentity {
            user_id: user_id.to_string(),
            api_key_id: None,
            api_key_name: None,
        }
    }

    fn task_input(model: &str, client_ref: Option<&str>) -> SubmitComputeTaskInput {
        SubmitComputeTaskInput {
            kind: "chat_completion".to_string(),
            model: model.to_string(),
            input: json!({
                "messages": [
                    { "role": "user", "content": "say hello" }
                ]
            }),
            priority: 0,
            client_ref: client_ref.map(str::to_string),
        }
    }

    fn worker_caps(models: &[&str]) -> WorkerCapabilities {
        WorkerCapabilities {
            node_id: None,
            host_kind: Some("linux".to_string()),
            gpu_name: Some("RTX 4060".to_string()),
            backend: Some("openai-compatible".to_string()),
            models: models.iter().map(|m| m.to_string()).collect(),
            vram_total_mb: Some(8192),
            vram_free_mb: Some(4096),
            max_concurrency: Some(1),
            current_inflight: Some(0),
            avg_tokens_per_sec: Some(12.5),
            worker_version: Some("test-worker".to_string()),
        }
    }

    async fn test_pool(db: &mongodb::Database, slug: &str, owner: &str) -> ComputePool {
        crate::db::ensure_indexes(db).await.expect("ensure indexes");
        let (pool, _) = create_pool(
            db,
            owner,
            CreateComputePoolInput {
                slug: slug.to_string(),
                name: "Task Test Pool".to_string(),
                description: None,
                visibility: Some(ComputePoolVisibility::Private),
                scheduling_policy: Some(ComputeSchedulingPolicy::ModelFit),
                max_workers: Some(2),
                max_queue_length: Some(10),
                per_user_max_inflight: Some(4),
                task_timeout_secs: Some(120),
            },
        )
        .await
        .expect("create pool");
        pool
    }

    #[test]
    fn model_fit_filter_honors_wildcard_worker() {
        let pool = ComputePool {
            id: "pool-a".to_string(),
            user_id: "owner-a".to_string(),
            slug: "pool-a".to_string(),
            name: "Pool A".to_string(),
            description: None,
            visibility: ComputePoolVisibility::Private,
            scheduling_policy: ComputeSchedulingPolicy::ModelFit,
            worker_token_hash: "hash".to_string(),
            max_workers: 1,
            max_queue_length: 10,
            per_user_max_inflight: 2,
            task_timeout_secs: 120,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let specific = task_filter_for_worker(&pool, &worker_caps(&["qwen2.5-coder"]));
        assert_eq!(
            specific.get_document("model").unwrap(),
            &doc! { "$in": ["qwen2.5-coder"] }
        );

        let wildcard = task_filter_for_worker(&pool, &worker_caps(&["*"]));
        assert!(wildcard.get("model").is_none());

        let no_models = task_filter_for_worker(&pool, &worker_caps(&[]));
        assert_eq!(
            no_models.get_document("model").unwrap(),
            &doc! { "$in": Vec::<String>::new() }
        );

        let fifo_pool = ComputePool {
            scheduling_policy: ComputeSchedulingPolicy::Fifo,
            ..pool
        };
        let fifo = task_filter_for_worker(&fifo_pool, &worker_caps(&["qwen2.5-coder"]));
        assert!(fifo.get("model").is_none());
    }

    #[test]
    fn input_and_output_payload_limits_are_enforced() {
        let oversized = serde_json::Value::String("x".repeat(MAX_COMPUTE_INPUT_BYTES + 1));
        let input = SubmitComputeTaskInput {
            kind: "chat_completion".to_string(),
            model: "local-model".to_string(),
            input: oversized,
            priority: 0,
            client_ref: None,
        };
        assert!(matches!(
            validate_submit_input(&input),
            Err(AppError::ComputePayloadTooLarge(_))
        ));

        let output = serde_json::Value::String("x".repeat(MAX_COMPUTE_OUTPUT_BYTES + 1));
        assert!(matches!(
            validate_json_size("output", &output, MAX_COMPUTE_OUTPUT_BYTES),
            Err(AppError::ComputePayloadTooLarge(_))
        ));
    }

    #[tokio::test]
    async fn submit_claim_ack_and_complete_task() {
        let Some(db) = connect_test_database("compute_task_lifecycle").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let pool = test_pool(&db, "compute-task-lifecycle", &owner).await;
        let submitter = submitter(&owner);

        let submitted = submit_task(&db, &pool, &submitter, task_input("codex-local", None))
            .await
            .expect("submit task");
        assert_eq!(submitted.queue_position, 1);
        assert!(!submitted.deduplicated);

        let claimed = claim_task(&db, &pool, "worker-a", worker_caps(&["*"]))
            .await
            .expect("claim task")
            .expect("task available");
        assert_eq!(claimed.task_id, submitted.task.id);
        assert_eq!(claimed.model, "codex-local");

        let ack = worker_ack(
            &db,
            &pool,
            "worker-a",
            &claimed.task_id,
            Some("running"),
            Some("local backend accepted request"),
            Some(worker_caps(&["*"])),
        )
        .await
        .expect("ack task");
        assert_eq!(ack, AckOutcome::Ok);

        let outcome = worker_submit_result(
            &db,
            &pool,
            "worker-a",
            &claimed.task_id,
            Some(json!({ "choices": [{ "message": { "content": "hello" } }] })),
            None,
            30,
        )
        .await
        .expect("submit result");
        assert_eq!(outcome, ResultOutcome::Completed);

        let (finished, position) = get_task_for_consumer(&db, &owner, &claimed.task_id)
            .await
            .expect("get task");
        assert_eq!(finished.status, ComputeTaskStatus::Completed);
        assert_eq!(
            finished.output,
            Some(json!({ "choices": [{ "message": { "content": "hello" } }] }))
        );
        assert_eq!(position, 0);

        let status = pool_status(&db, &pool).await.expect("pool status");
        assert_eq!(status.queued, 0);
        assert_eq!(status.dispatched, 0);
        assert_eq!(status.active_workers.len(), 1);
        assert_eq!(status.active_workers[0].worker_label, "worker-a");

        db.drop().await.ok();
    }

    #[tokio::test]
    async fn cancelled_dispatched_task_is_reported_to_worker_ack() {
        let Some(db) = connect_test_database("compute_task_cancel_ack").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let pool = test_pool(&db, "compute-task-cancel-ack", &owner).await;
        let submitter = submitter(&owner);

        let submitted = submit_task(&db, &pool, &submitter, task_input("codex-local", None))
            .await
            .expect("submit task");
        let claimed = claim_task(&db, &pool, "worker-a", worker_caps(&["codex-local"]))
            .await
            .expect("claim task")
            .expect("task available");
        assert_eq!(claimed.task_id, submitted.task.id);

        let cancelled = cancel_task(&db, &owner, &claimed.task_id, 30)
            .await
            .expect("cancel task");
        assert_eq!(cancelled.status, ComputeTaskStatus::Cancelled);

        let ack = worker_ack(
            &db,
            &pool,
            "worker-a",
            &claimed.task_id,
            Some("still_running"),
            None,
            None,
        )
        .await
        .expect("ack cancelled task");
        assert_eq!(ack, AckOutcome::Cancelled);

        let result = worker_submit_result(
            &db,
            &pool,
            "worker-a",
            &claimed.task_id,
            Some(json!({ "ignored": true })),
            None,
            30,
        )
        .await
        .expect("submit late result");
        assert_eq!(result, ResultOutcome::Ignored);

        db.drop().await.ok();
    }
}
