//! Oracle task queue: submit / claim / heartbeat / result, all backed by
//! MongoDB so any backend instance can serve any request (no in-memory
//! queue, no sticky routing).
//!
//! Lifecycle: `queued` → atomic claim (`find_one_and_update`, FIFO by
//! `created_at`) → `dispatched` with a lease → terminal
//! (`completed` / `failed` / `cancelled`). Worker heartbeats refresh the
//! lease; expired leases are lazily requeued on the next claim, where the
//! original `created_at` puts the task back at the front of the FIFO —
//! the Mongo equivalent of the local oracle server's `appendleft`.
//!
//! Prompt and response bodies are stored only on the task document
//! (TTL-expired via `expires_at`); tracing and audit events stay
//! metadata-only.

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

use crate::errors::{AppError, AppResult};
use crate::models::oracle_pool::OraclePool;
use crate::models::oracle_session::{COLLECTION_NAME as ORACLE_SESSIONS, OracleSession};
use crate::models::oracle_task::{COLLECTION_NAME as ORACLE_TASKS, OracleTask, OracleTaskStatus};
use crate::models::oracle_worker::{
    COLLECTION_NAME as ORACLE_WORKERS, OracleWorker, worker_doc_id,
};
use crate::services::oracle_pool_service;

pub const MAX_PROMPT_CHARS: usize = 512_000;
pub const MAX_PDF_BASE64_BYTES: usize = 12_000_000;
pub const MAX_RESPONSE_CHARS: usize = 2_000_000;
const MAX_TAG_LEN: usize = 128;
const MAX_MODEL_LABEL_LEN: usize = 128;
const MAX_CLIENT_REF_LEN: usize = 128;
const MAX_PDF_NAME_LEN: usize = 256;
const MAX_PHASE_LEN: usize = 80;
const MAX_PHASE_DETAIL_LEN: usize = 500;
const MAX_URL_LEN: usize = 2048;
const MAX_WORKER_LABEL_LEN: usize = 64;

/// Workers polling within this window count as "active" in pool status.
pub const WORKER_RECENT_SECS: i64 = 120;

#[derive(Debug, Clone)]
pub struct SubmitterIdentity {
    pub user_id: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
}

#[derive(Debug, Default)]
pub struct SubmitTaskInput {
    pub prompt: String,
    pub model_label: Option<String>,
    pub tag: Option<String>,
    /// Three-state, mirroring the local oracle protocol:
    /// - `None`: single-shot task, no session.
    /// - `Some("")`: open a new session; the minted id is returned.
    /// - `Some(id)`: continue an existing session (must be open and owned
    ///   by the submitter).
    pub conversation_id: Option<String>,
    pub pdf_base64: Option<String>,
    pub pdf_name: Option<String>,
    pub client_ref: Option<String>,
}

#[derive(Debug)]
pub struct SubmitOutcome {
    pub task: OracleTask,
    pub queue_position: u64,
    /// True when an identical `client_ref` resubmit was deduplicated.
    pub deduplicated: bool,
}

fn validate_submit_input(input: &SubmitTaskInput) -> AppResult<()> {
    if input.prompt.trim().is_empty() {
        return Err(AppError::ValidationError("prompt is required".to_string()));
    }
    if input.prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(AppError::OraclePayloadTooLarge(format!(
            "prompt exceeds {MAX_PROMPT_CHARS} chars"
        )));
    }
    if let Some(pdf) = &input.pdf_base64 {
        if pdf.len() > MAX_PDF_BASE64_BYTES {
            return Err(AppError::OraclePayloadTooLarge(format!(
                "pdf_base64 exceeds {MAX_PDF_BASE64_BYTES} bytes"
            )));
        }
        if input
            .pdf_name
            .as_deref()
            .is_none_or(|n| n.trim().is_empty())
        {
            return Err(AppError::ValidationError(
                "pdf_name is required when pdf_base64 is set".to_string(),
            ));
        }
    }
    if input
        .pdf_name
        .as_deref()
        .is_some_and(|n| n.len() > MAX_PDF_NAME_LEN)
    {
        return Err(AppError::ValidationError(format!(
            "pdf_name exceeds {MAX_PDF_NAME_LEN} chars"
        )));
    }
    if input.tag.as_deref().is_some_and(|t| t.len() > MAX_TAG_LEN) {
        return Err(AppError::ValidationError(format!(
            "tag exceeds {MAX_TAG_LEN} chars"
        )));
    }
    if input
        .model_label
        .as_deref()
        .is_some_and(|m| m.len() > MAX_MODEL_LABEL_LEN)
    {
        return Err(AppError::ValidationError(format!(
            "model exceeds {MAX_MODEL_LABEL_LEN} chars"
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
    Ok(())
}

fn mint_conversation_id() -> String {
    format!("conv_{}", hex::encode(rand::random::<[u8; 8]>()))
}

async fn count_tasks(db: &mongodb::Database, filter: Document) -> AppResult<u64> {
    Ok(db
        .collection::<OracleTask>(ORACLE_TASKS)
        .count_documents(filter)
        .await?)
}

/// Enqueue a task. The caller has already resolved the pool and passed the
/// visibility gate (`oracle_pool_service::ensure_can_submit`).
pub async fn submit_task(
    db: &mongodb::Database,
    pool: &OraclePool,
    submitter: &SubmitterIdentity,
    input: SubmitTaskInput,
) -> AppResult<SubmitOutcome> {
    validate_submit_input(&input)?;

    // Quotas. Counts are read-then-insert (no transaction); a concurrent
    // burst can overshoot by a few tasks, which is acceptable for a
    // fairness cap.
    let queued = count_tasks(db, doc! { "pool_id": &pool.id, "status": "queued" }).await?;
    if queued >= u64::from(pool.max_queue_length) {
        return Err(AppError::OracleQueueFull(format!(
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
        return Err(AppError::OracleQuotaExceeded(format!(
            "you already have {inflight} tasks in flight in pool '{}' (limit {})",
            pool.slug, pool.per_user_max_inflight
        )));
    }

    // Session resolution (three-state conversation_id).
    let now = Utc::now();
    let (conversation_id, is_followup) = match input.conversation_id.as_deref() {
        None => (None, false),
        Some("") => {
            let conv_id = mint_conversation_id();
            let session = OracleSession {
                id: conv_id.clone(),
                pool_id: pool.id.clone(),
                owner_user_id: submitter.user_id.clone(),
                api_key_id: submitter.api_key_id.clone(),
                tag: input.tag.clone(),
                chatgpt_url: None,
                turn_count: 0,
                last_task_id: None,
                closed_at: None,
                created_at: now,
                updated_at: now,
            };
            db.collection::<OracleSession>(ORACLE_SESSIONS)
                .insert_one(&session)
                .await?;
            (Some(conv_id), false)
        }
        Some(conv_id) => {
            let session = db
                .collection::<OracleSession>(ORACLE_SESSIONS)
                .find_one(doc! { "_id": conv_id })
                .await?
                .ok_or_else(|| AppError::OracleSessionNotFound(conv_id.to_string()))?;
            if session.closed_at.is_some() {
                return Err(AppError::OracleSessionClosed(conv_id.to_string()));
            }
            if session.pool_id != pool.id {
                return Err(AppError::ValidationError(
                    "conversation belongs to a different pool".to_string(),
                ));
            }
            if session.owner_user_id != submitter.user_id {
                return Err(AppError::Forbidden(
                    "only the session owner can continue it".to_string(),
                ));
            }
            (Some(conv_id.to_string()), session.turn_count > 0)
        }
    };

    let task = OracleTask {
        id: uuid::Uuid::new_v4().to_string(),
        pool_id: pool.id.clone(),
        submitter_user_id: submitter.user_id.clone(),
        api_key_id: submitter.api_key_id.clone(),
        api_key_name: submitter.api_key_name.clone(),
        prompt: input.prompt,
        model_label: input
            .model_label
            .or_else(|| pool.default_model_label.clone()),
        tag: input.tag,
        pdf_base64: input.pdf_base64,
        pdf_name: input.pdf_name,
        conversation_id,
        is_followup,
        client_ref: input.client_ref,
        status: OracleTaskStatus::Queued,
        phase: None,
        phase_detail: None,
        phase_at: None,
        assigned_worker_id: None,
        dispatched_at: None,
        lease_expires_at: None,
        response: None,
        response_chars: None,
        chatgpt_url: None,
        failure_reason: None,
        worker_script_version: None,
        completed_at: None,
        expires_at: None,
        created_at: now,
        updated_at: now,
    };

    let insert = db
        .collection::<OracleTask>(ORACLE_TASKS)
        .insert_one(&task)
        .await;
    if let Err(e) = insert {
        // Submitter-scoped idempotency: a duplicate client_ref returns the
        // original task instead of erroring, so blind retries are safe.
        if oracle_pool_service::is_duplicate_key(&e)
            && let Some(client_ref) = &task.client_ref
        {
            let existing = db
                .collection::<OracleTask>(ORACLE_TASKS)
                .find_one(doc! {
                    "submitter_user_id": &submitter.user_id,
                    "client_ref": client_ref,
                })
                .await?;
            if let Some(existing) = existing {
                let position = queue_position(db, &existing).await?;
                return Ok(SubmitOutcome {
                    task: existing,
                    queue_position: position,
                    deduplicated: true,
                });
            }
        }
        return Err(e.into());
    }

    let position = queue_position(db, &task).await?;
    Ok(SubmitOutcome {
        task,
        queue_position: position,
        deduplicated: false,
    })
}

/// 1-based position among queued tasks of the same pool (0 = not queued).
async fn queue_position(db: &mongodb::Database, task: &OracleTask) -> AppResult<u64> {
    if task.status != OracleTaskStatus::Queued {
        return Ok(0);
    }
    let ahead = count_tasks(
        db,
        doc! {
            "pool_id": &task.pool_id,
            "status": "queued",
            "created_at": { "$lt": bson::DateTime::from_chrono(task.created_at) },
        },
    )
    .await?;
    Ok(ahead + 1)
}

/// Load a task for a consumer: the submitter always may read; the pool
/// owner / org admin may too.
pub async fn get_task_for_consumer(
    db: &mongodb::Database,
    actor_user_id: &str,
    task_id: &str,
) -> AppResult<(OracleTask, u64)> {
    let task = db
        .collection::<OracleTask>(ORACLE_TASKS)
        .find_one(doc! { "_id": task_id })
        .await?
        .ok_or_else(|| AppError::OracleTaskNotFound(task_id.to_string()))?;

    if task.submitter_user_id != actor_user_id {
        let pool = oracle_pool_service::get_pool(db, &task.pool_id).await?;
        oracle_pool_service::ensure_can_manage(db, actor_user_id, &pool)
            .await
            .map_err(|_| AppError::OracleTaskNotFound(task_id.to_string()))?;
    }

    let position = queue_position(db, &task).await?;
    Ok((task, position))
}

/// Cancel a queued or dispatched task. Dispatched workers learn about the
/// cancellation through their next heartbeat ack. Idempotent for tasks
/// already cancelled; other terminal states conflict.
pub async fn cancel_task(
    db: &mongodb::Database,
    actor_user_id: &str,
    task_id: &str,
    retention_days: u32,
) -> AppResult<OracleTask> {
    let (task, _) = get_task_for_consumer(db, actor_user_id, task_id).await?;
    match task.status {
        OracleTaskStatus::Cancelled => return Ok(task),
        OracleTaskStatus::Completed | OracleTaskStatus::Failed => {
            return Err(AppError::Conflict(format!(
                "task is already {}",
                task.status.as_str()
            )));
        }
        OracleTaskStatus::Queued | OracleTaskStatus::Dispatched => {}
    }

    let now = Utc::now();
    let updated = db
        .collection::<OracleTask>(ORACLE_TASKS)
        .find_one_and_update(
            doc! {
                "_id": task_id,
                "status": { "$in": ["queued", "dispatched"] },
            },
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

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect()
    }
}

async fn upsert_worker_presence(
    db: &mongodb::Database,
    pool: &OraclePool,
    worker_label: &str,
    current_task_id: Option<&str>,
    script_version: Option<&str>,
    page_url: Option<&str>,
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
    if let Some(v) = script_version {
        set.insert("script_version", truncate_chars(v, 64));
    }
    if let Some(u) = page_url {
        set.insert("page_url", truncate_chars(u, MAX_URL_LEN));
    }
    db.collection::<Document>(ORACLE_WORKERS)
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

/// Requeue dispatched tasks whose lease expired (worker died mid-task).
/// The preserved `created_at` puts them back at the FIFO front.
async fn requeue_expired_leases(db: &mongodb::Database, pool_id: &str) -> AppResult<u64> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let result = db
        .collection::<OracleTask>(ORACLE_TASKS)
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

/// The payload a worker receives for a claimed task. Field names mirror
/// the local oracle servers' task dicts so the userscript port stays a
/// thin diff.
#[derive(Debug, serde::Serialize)]
pub struct WorkerTaskPayload {
    pub task_id: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_url: Option<String>,
    pub is_followup: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_project_url: Option<String>,
    pub assigned_worker: String,
    pub submitted_at: String,
}

async fn worker_payload(
    db: &mongodb::Database,
    pool: &OraclePool,
    task: &OracleTask,
    worker_label: &str,
) -> AppResult<WorkerTaskPayload> {
    // Follow-ups navigate back to the pinned conversation URL.
    let conversation_url = match &task.conversation_id {
        Some(conv_id) => db
            .collection::<OracleSession>(ORACLE_SESSIONS)
            .find_one(doc! { "_id": conv_id })
            .await?
            .and_then(|s| s.chatgpt_url),
        None => None,
    };
    Ok(WorkerTaskPayload {
        task_id: task.id.clone(),
        prompt: task.prompt.clone(),
        conversation_id: task.conversation_id.clone(),
        conversation_url,
        is_followup: task.is_followup,
        model: task.model_label.clone(),
        tag: task.tag.clone(),
        pdf_base64: task.pdf_base64.clone(),
        pdf_name: task.pdf_name.clone(),
        required_project_url: pool.chatgpt_project_url.clone(),
        assigned_worker: worker_label.to_string(),
        submitted_at: task.created_at.to_rfc3339(),
    })
}

/// Worker poll: requeue expired leases, resume the worker's own in-flight
/// task if any (idempotent re-claim — this is what lets a tab survive a
/// mid-task page reload), then atomically claim the oldest queued task if
/// the pool has dispatch capacity. `None` = idle.
pub async fn claim_task(
    db: &mongodb::Database,
    pool: &OraclePool,
    worker_label: &str,
    script_version: Option<&str>,
    page_url: Option<&str>,
) -> AppResult<Option<WorkerTaskPayload>> {
    validate_worker_label(worker_label)?;
    requeue_expired_leases(db, &pool.id).await?;

    let now = Utc::now();
    let lease = now + Duration::seconds(pool.task_timeout_secs as i64);

    // Idempotent resume of this worker's pending task.
    let resumed = db
        .collection::<OracleTask>(ORACLE_TASKS)
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
        upsert_worker_presence(
            db,
            pool,
            worker_label,
            Some(&task.id),
            script_version,
            page_url,
        )
        .await?;
        return Ok(Some(worker_payload(db, pool, &task, worker_label).await?));
    }

    upsert_worker_presence(db, pool, worker_label, None, script_version, page_url).await?;

    // Soft capacity gate (concurrent claims may briefly overshoot by one;
    // the cap is a fairness knob, not an invariant).
    let dispatched = count_tasks(db, doc! { "pool_id": &pool.id, "status": "dispatched" }).await?;
    if dispatched >= u64::from(pool.max_workers) {
        return Ok(None);
    }

    let claimed = db
        .collection::<OracleTask>(ORACLE_TASKS)
        .find_one_and_update(
            doc! { "pool_id": &pool.id, "status": "queued" },
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
                .sort(doc! { "created_at": 1 })
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;

    match claimed {
        Some(task) => {
            upsert_worker_presence(
                db,
                pool,
                worker_label,
                Some(&task.id),
                script_version,
                page_url,
            )
            .await?;
            Ok(Some(worker_payload(db, pool, &task, worker_label).await?))
        }
        None => Ok(None),
    }
}

/// Outcome of a worker ack/heartbeat: `Cancelled` tells the tab to abandon
/// the task (the cancellation back-channel of the local oracle protocol).
#[derive(Debug, PartialEq)]
pub enum AckOutcome {
    Ok,
    Cancelled,
}

/// Heartbeat: refresh the lease and record progress. Returns `Cancelled`
/// when the task is no longer this worker's live dispatch (cancelled by
/// the submitter, expired-and-reclaimed, or unknown).
#[allow(clippy::too_many_arguments)]
pub async fn worker_ack(
    db: &mongodb::Database,
    pool: &OraclePool,
    worker_label: &str,
    task_id: &str,
    phase: Option<&str>,
    phase_detail: Option<&str>,
    script_version: Option<&str>,
    page_url: Option<&str>,
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
        .collection::<OracleTask>(ORACLE_TASKS)
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
        script_version,
        page_url,
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
    /// The task was no longer this worker's live dispatch; result dropped.
    Ignored,
}

/// Store a worker's result. Empty or `ERROR:`-prefixed responses mark the
/// task `failed` (extraction failure), mirroring the local oracle servers.
#[allow(clippy::too_many_arguments)]
pub async fn worker_submit_result(
    db: &mongodb::Database,
    pool: &OraclePool,
    worker_label: &str,
    task_id: &str,
    response: &str,
    chatgpt_url: Option<&str>,
    model: Option<&str>,
    script_version: Option<&str>,
    retention_days: u32,
) -> AppResult<ResultOutcome> {
    validate_worker_label(worker_label)?;
    let now = Utc::now();
    let trimmed = response.trim();
    let is_failure = trimmed.is_empty() || trimmed.starts_with("ERROR:");
    let stored_response = truncate_chars(response, MAX_RESPONSE_CHARS);
    let response_chars = stored_response.chars().count() as u64;

    let mut set = doc! {
        "status": if is_failure { "failed" } else { "completed" },
        "response": &stored_response,
        "response_chars": response_chars as i64,
        "completed_at": bson::DateTime::from_chrono(now),
        "expires_at": bson::DateTime::from_chrono(terminal_expiry(retention_days)),
        "updated_at": bson::DateTime::from_chrono(now),
    };
    if is_failure {
        set.insert(
            "failure_reason",
            if trimmed.is_empty() {
                "empty_response".to_string()
            } else {
                "extraction_failure".to_string()
            },
        );
    }
    if let Some(url) = chatgpt_url {
        set.insert("chatgpt_url", truncate_chars(url, MAX_URL_LEN));
    }
    if let Some(model) = model {
        set.insert("model_label", truncate_chars(model, MAX_MODEL_LABEL_LEN));
    }
    if let Some(v) = script_version {
        set.insert("worker_script_version", truncate_chars(v, 64));
    }

    let updated = db
        .collection::<OracleTask>(ORACLE_TASKS)
        .find_one_and_update(
            doc! {
                "_id": task_id,
                "pool_id": &pool.id,
                "status": "dispatched",
                "assigned_worker_id": worker_label,
            },
            doc! { "$set": set },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;

    upsert_worker_presence(db, pool, worker_label, None, script_version, None).await?;

    let Some(task) = updated else {
        return Ok(ResultOutcome::Ignored);
    };

    // Session bookkeeping: bump the turn and pin the conversation URL.
    if let Some(conv_id) = &task.conversation_id {
        let mut session_set = doc! {
            "last_task_id": &task.id,
            "updated_at": bson::DateTime::from_chrono(now),
        };
        if let Some(url) = chatgpt_url.filter(|u| !u.is_empty()) {
            session_set.insert("chatgpt_url", truncate_chars(url, MAX_URL_LEN));
        }
        db.collection::<OracleSession>(ORACLE_SESSIONS)
            .update_one(
                doc! { "_id": conv_id },
                doc! {
                    "$set": session_set,
                    "$inc": { "turn_count": 1 },
                },
            )
            .await?;
    }

    Ok(if is_failure {
        ResultOutcome::Failed
    } else {
        ResultOutcome::Completed
    })
}

/// Pin the browser-side conversation URL mid-task (the worker calls this
/// as soon as the chat URL is known, before the result lands, so a
/// follow-up submitted concurrently can already navigate).
pub async fn pin_conversation_url(
    db: &mongodb::Database,
    pool: &OraclePool,
    worker_label: &str,
    task_id: &str,
    chatgpt_url: &str,
) -> AppResult<()> {
    validate_worker_label(worker_label)?;
    if chatgpt_url.is_empty() || chatgpt_url.len() > MAX_URL_LEN {
        return Err(AppError::ValidationError(
            "chatgpt_url must be 1-2048 chars".to_string(),
        ));
    }
    let task = db
        .collection::<OracleTask>(ORACLE_TASKS)
        .find_one(doc! {
            "_id": task_id,
            "pool_id": &pool.id,
            "assigned_worker_id": worker_label,
        })
        .await?
        .ok_or_else(|| AppError::OracleTaskNotFound(task_id.to_string()))?;

    let now = bson::DateTime::from_chrono(Utc::now());
    db.collection::<OracleTask>(ORACLE_TASKS)
        .update_one(
            doc! { "_id": &task.id },
            doc! { "$set": { "chatgpt_url": chatgpt_url, "updated_at": now } },
        )
        .await?;
    if let Some(conv_id) = &task.conversation_id {
        db.collection::<OracleSession>(ORACLE_SESSIONS)
            .update_one(
                doc! { "_id": conv_id },
                doc! { "$set": { "chatgpt_url": chatgpt_url, "updated_at": now } },
            )
            .await?;
    }
    Ok(())
}

#[derive(Debug, serde::Serialize)]
pub struct PoolStatus {
    pub queued: u64,
    pub dispatched: u64,
    pub max_workers: u32,
    pub active_workers: Vec<WorkerStatus>,
    /// "idle" | "running" | "queue_waiting_for_worker"
    pub diagnosis: String,
}

#[derive(Debug, serde::Serialize)]
pub struct WorkerStatus {
    pub worker_label: String,
    pub last_seen_secs_ago: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_version: Option<String>,
}

/// Queue/worker overview for a pool (consumer-facing; no prompt bodies).
pub async fn pool_status(db: &mongodb::Database, pool: &OraclePool) -> AppResult<PoolStatus> {
    requeue_expired_leases(db, &pool.id).await?;
    let queued = count_tasks(db, doc! { "pool_id": &pool.id, "status": "queued" }).await?;
    let dispatched = count_tasks(db, doc! { "pool_id": &pool.id, "status": "dispatched" }).await?;

    let now = Utc::now();
    let recent_cutoff = now - Duration::seconds(WORKER_RECENT_SECS);
    let workers: Vec<OracleWorker> = db
        .collection::<OracleWorker>(ORACLE_WORKERS)
        .find(doc! {
            "pool_id": &pool.id,
            "last_seen_at": { "$gte": bson::DateTime::from_chrono(recent_cutoff) },
        })
        .await?
        .try_collect()
        .await?;
    let active_workers: Vec<WorkerStatus> = workers
        .into_iter()
        .map(|w| WorkerStatus {
            worker_label: w.worker_label,
            last_seen_secs_ago: (now - w.last_seen_at).num_seconds().max(0),
            current_task_id: w.current_task_id,
            script_version: w.script_version,
        })
        .collect();

    let diagnosis = if queued > 0 && active_workers.is_empty() {
        "queue_waiting_for_worker"
    } else if queued > 0 || dispatched > 0 {
        "running"
    } else {
        "idle"
    };

    Ok(PoolStatus {
        queued,
        dispatched,
        max_workers: pool.max_workers,
        active_workers,
        diagnosis: diagnosis.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::oracle_pool::OraclePoolVisibility;
    use crate::test_utils::connect_test_database;

    fn submitter(user_id: &str) -> SubmitterIdentity {
        SubmitterIdentity {
            user_id: user_id.to_string(),
            api_key_id: None,
            api_key_name: None,
        }
    }

    fn prompt_input(prompt: &str) -> SubmitTaskInput {
        SubmitTaskInput {
            prompt: prompt.to_string(),
            ..Default::default()
        }
    }

    fn test_pool(owner: &str) -> OraclePool {
        let now = Utc::now();
        OraclePool {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: owner.to_string(),
            slug: format!("pool-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            name: "Test Pool".to_string(),
            description: None,
            visibility: OraclePoolVisibility::Platform,
            worker_token_hash: "h".repeat(64),
            chatgpt_project_url: Some("https://chatgpt.com/g/g-p-x/project".to_string()),
            default_model_label: Some("chatgpt-5.5-pro".to_string()),
            max_workers: 2,
            max_queue_length: 3,
            per_user_max_inflight: 2,
            task_timeout_secs: 3600,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    /// Persist the pool so the non-submitter ACL path (which re-fetches the
    /// pool from the DB) sees it, as it always would in production.
    async fn seed_pool(db: &mongodb::Database, pool: &OraclePool) {
        db.collection::<OraclePool>(crate::models::oracle_pool::COLLECTION_NAME)
            .insert_one(pool)
            .await
            .unwrap();
    }

    #[test]
    fn submit_validation() {
        assert!(validate_submit_input(&prompt_input("hello")).is_ok());
        assert!(validate_submit_input(&prompt_input("")).is_err());
        assert!(validate_submit_input(&prompt_input("   ")).is_err());

        let oversized_pdf = SubmitTaskInput {
            pdf_base64: Some("x".repeat(MAX_PDF_BASE64_BYTES + 1)),
            pdf_name: Some("a.pdf".to_string()),
            ..prompt_input("p")
        };
        assert!(matches!(
            validate_submit_input(&oversized_pdf),
            Err(AppError::OraclePayloadTooLarge(_))
        ));

        let pdf_without_name = SubmitTaskInput {
            pdf_base64: Some("abcd".to_string()),
            ..prompt_input("p")
        };
        assert!(validate_submit_input(&pdf_without_name).is_err());

        let long_client_ref = SubmitTaskInput {
            client_ref: Some("c".repeat(129)),
            ..prompt_input("p")
        };
        assert!(validate_submit_input(&long_client_ref).is_err());
    }

    #[test]
    fn worker_label_validation() {
        assert!(validate_worker_label("tab_1").is_ok());
        assert!(validate_worker_label("bedc-2").is_ok());
        assert!(validate_worker_label("").is_err());
        assert!(validate_worker_label("has space").is_err());
        assert!(validate_worker_label(&"x".repeat(65)).is_err());
    }

    #[test]
    fn conversation_id_shape() {
        let id = mint_conversation_id();
        assert!(id.starts_with("conv_"));
        assert_eq!(id.len(), "conv_".len() + 16);
    }

    #[test]
    fn truncate_chars_respects_char_boundaries() {
        assert_eq!(truncate_chars("héllo", 2), "hé");
        assert_eq!(truncate_chars("短", 5), "短");
    }

    #[tokio::test]
    async fn fifo_claim_lease_and_result_lifecycle() {
        let Some(db) = connect_test_database("oracle_task_lifecycle").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let pool = test_pool(&owner);
        seed_pool(&db, &pool).await;

        // Two tasks from one submitter (inflight cap is 2).
        let first = submit_task(&db, &pool, &submitter(&owner), prompt_input("first"))
            .await
            .unwrap();
        assert_eq!(first.queue_position, 1);
        let second = submit_task(&db, &pool, &submitter(&owner), prompt_input("second"))
            .await
            .unwrap();
        assert_eq!(second.queue_position, 2);

        // Per-user inflight quota blocks the third.
        let third = submit_task(&db, &pool, &submitter(&owner), prompt_input("third")).await;
        assert!(matches!(third, Err(AppError::OracleQuotaExceeded(_))));

        // FIFO: worker claims the oldest first.
        let claimed = claim_task(&db, &pool, "tab_1", Some("v1"), None)
            .await
            .unwrap()
            .expect("task available");
        assert_eq!(claimed.task_id, first.task.id);
        assert_eq!(
            claimed.required_project_url.as_deref(),
            Some("https://chatgpt.com/g/g-p-x/project")
        );
        assert_eq!(claimed.model.as_deref(), Some("chatgpt-5.5-pro"));

        // Idempotent re-claim returns the same task (tab reload survival).
        let resumed = claim_task(&db, &pool, "tab_1", Some("v1"), None)
            .await
            .unwrap()
            .expect("resume");
        assert_eq!(resumed.task_id, first.task.id);

        // Heartbeat refreshes the lease and records phase.
        let ack = worker_ack(
            &db,
            &pool,
            "tab_1",
            &first.task.id,
            Some("waiting_response"),
            Some("elapsed=60s"),
            Some("v1"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(ack, AckOutcome::Ok);

        // Second worker claims the second task.
        let claimed2 = claim_task(&db, &pool, "tab_2", None, None)
            .await
            .unwrap()
            .expect("second task");
        assert_eq!(claimed2.task_id, second.task.id);

        // Pool at max_workers=2: a third worker idles.
        let idle = claim_task(&db, &pool, "tab_3", None, None).await.unwrap();
        assert!(idle.is_none());

        // Result lands; consumer sees completed.
        let outcome = worker_submit_result(
            &db,
            &pool,
            "tab_1",
            &first.task.id,
            "The answer is 42.",
            Some("https://chatgpt.com/c/abc"),
            Some("chatgpt-5.5-pro"),
            Some("v1"),
            30,
        )
        .await
        .unwrap();
        assert_eq!(outcome, ResultOutcome::Completed);
        let (done, _) = get_task_for_consumer(&db, &owner, &first.task.id)
            .await
            .unwrap();
        assert_eq!(done.status, OracleTaskStatus::Completed);
        assert_eq!(done.response.as_deref(), Some("The answer is 42."));
        assert!(done.expires_at.is_some());

        // ERROR-prefixed result marks failed.
        let fail = worker_submit_result(
            &db,
            &pool,
            "tab_2",
            &second.task.id,
            "ERROR: Response too short or empty",
            None,
            None,
            None,
            30,
        )
        .await
        .unwrap();
        assert_eq!(fail, ResultOutcome::Failed);
        let (failed, _) = get_task_for_consumer(&db, &owner, &second.task.id)
            .await
            .unwrap();
        assert_eq!(failed.status, OracleTaskStatus::Failed);
        assert_eq!(failed.failure_reason.as_deref(), Some("extraction_failure"));

        // Late duplicate result for a terminal task is ignored.
        let late = worker_submit_result(
            &db,
            &pool,
            "tab_1",
            &first.task.id,
            "stale",
            None,
            None,
            None,
            30,
        )
        .await
        .unwrap();
        assert_eq!(late, ResultOutcome::Ignored);

        db.drop().await.ok();
    }

    #[tokio::test]
    async fn lease_expiry_requeues_to_front() {
        let Some(db) = connect_test_database("oracle_task_lease").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let mut pool = test_pool(&owner);
        pool.per_user_max_inflight = 3;
        seed_pool(&db, &pool).await;

        let old = submit_task(&db, &pool, &submitter(&owner), prompt_input("old"))
            .await
            .unwrap();
        let newer = submit_task(&db, &pool, &submitter(&owner), prompt_input("newer"))
            .await
            .unwrap();

        let claimed = claim_task(&db, &pool, "tab_1", None, None)
            .await
            .unwrap()
            .expect("claim old");
        assert_eq!(claimed.task_id, old.task.id);

        // Force the lease into the past (simulates a dead tab).
        db.collection::<OracleTask>(ORACLE_TASKS)
            .update_one(
                doc! { "_id": &old.task.id },
                doc! { "$set": { "lease_expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(5)) } },
            )
            .await
            .unwrap();

        // A different worker claims: the expired task is requeued and wins
        // (front of FIFO via original created_at), not the newer task.
        let reclaimed = claim_task(&db, &pool, "tab_2", None, None)
            .await
            .unwrap()
            .expect("reclaim");
        assert_eq!(reclaimed.task_id, old.task.id);

        // The original worker's stale heartbeat now reports Cancelled.
        let stale_ack = worker_ack(&db, &pool, "tab_1", &old.task.id, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(stale_ack, AckOutcome::Cancelled);

        // And its stale result is dropped.
        let stale_result = worker_submit_result(
            &db,
            &pool,
            "tab_1",
            &old.task.id,
            "from dead tab",
            None,
            None,
            None,
            30,
        )
        .await
        .unwrap();
        assert_eq!(stale_result, ResultOutcome::Ignored);

        // The newer task is still queued, untouched.
        let (newer_task, pos) = get_task_for_consumer(&db, &owner, &newer.task.id)
            .await
            .unwrap();
        assert_eq!(newer_task.status, OracleTaskStatus::Queued);
        assert_eq!(pos, 1);

        db.drop().await.ok();
    }

    #[tokio::test]
    async fn sessions_continue_and_pin() {
        let Some(db) = connect_test_database("oracle_task_sessions").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let stranger = uuid::Uuid::new_v4().to_string();
        let mut pool = test_pool(&owner);
        pool.per_user_max_inflight = 5;
        seed_pool(&db, &pool).await;

        // Open a session.
        let t1 = submit_task(
            &db,
            &pool,
            &submitter(&owner),
            SubmitTaskInput {
                conversation_id: Some(String::new()),
                ..prompt_input("turn one")
            },
        )
        .await
        .unwrap();
        let conv_id = t1.task.conversation_id.clone().expect("conv id minted");
        assert!(conv_id.starts_with("conv_"));
        assert!(!t1.task.is_followup);

        // Worker completes turn 1 and pins the chat URL.
        let claimed = claim_task(&db, &pool, "tab_1", None, None)
            .await
            .unwrap()
            .expect("claim");
        assert!(claimed.conversation_url.is_none());
        pin_conversation_url(
            &db,
            &pool,
            "tab_1",
            &t1.task.id,
            "https://chatgpt.com/c/xyz",
        )
        .await
        .unwrap();
        worker_submit_result(
            &db,
            &pool,
            "tab_1",
            &t1.task.id,
            "turn one answer",
            Some("https://chatgpt.com/c/xyz"),
            None,
            None,
            30,
        )
        .await
        .unwrap();

        // A stranger cannot continue the session.
        let hijack = submit_task(
            &db,
            &pool,
            &submitter(&stranger),
            SubmitTaskInput {
                conversation_id: Some(conv_id.clone()),
                ..prompt_input("hijack")
            },
        )
        .await;
        assert!(matches!(hijack, Err(AppError::Forbidden(_))));

        // Owner continues; the worker payload carries the pinned URL.
        let t2 = submit_task(
            &db,
            &pool,
            &submitter(&owner),
            SubmitTaskInput {
                conversation_id: Some(conv_id.clone()),
                ..prompt_input("turn two")
            },
        )
        .await
        .unwrap();
        assert!(t2.task.is_followup);
        let claimed2 = claim_task(&db, &pool, "tab_1", None, None)
            .await
            .unwrap()
            .expect("claim turn two");
        assert_eq!(claimed2.task_id, t2.task.id);
        assert_eq!(
            claimed2.conversation_url.as_deref(),
            Some("https://chatgpt.com/c/xyz")
        );

        // Unknown session id.
        let missing = submit_task(
            &db,
            &pool,
            &submitter(&owner),
            SubmitTaskInput {
                conversation_id: Some("conv_doesnotexist00".to_string()),
                ..prompt_input("nope")
            },
        )
        .await;
        assert!(matches!(missing, Err(AppError::OracleSessionNotFound(_))));

        db.drop().await.ok();
    }

    #[tokio::test]
    async fn cancel_and_idempotent_client_ref() {
        let Some(db) = connect_test_database("oracle_task_cancel").await else {
            return;
        };
        // The partial unique index lives in db::ensure_indexes; create the
        // equivalent here so the dedup path is exercised.
        db.collection::<Document>(ORACLE_TASKS)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! { "submitter_user_id": 1, "client_ref": 1 })
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(doc! { "client_ref": { "$exists": true } })
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();

        let owner = uuid::Uuid::new_v4().to_string();
        let stranger = uuid::Uuid::new_v4().to_string();
        let mut pool = test_pool(&owner);
        pool.per_user_max_inflight = 5;
        seed_pool(&db, &pool).await;

        let submitted = submit_task(
            &db,
            &pool,
            &submitter(&owner),
            SubmitTaskInput {
                client_ref: Some("retry-key-1".to_string()),
                ..prompt_input("idempotent")
            },
        )
        .await
        .unwrap();
        assert!(!submitted.deduplicated);

        // Blind retry with the same client_ref returns the same task.
        let retried = submit_task(
            &db,
            &pool,
            &submitter(&owner),
            SubmitTaskInput {
                client_ref: Some("retry-key-1".to_string()),
                ..prompt_input("idempotent retry")
            },
        )
        .await
        .unwrap();
        assert!(retried.deduplicated);
        assert_eq!(retried.task.id, submitted.task.id);

        // Strangers cannot read or cancel someone else's task.
        let read = get_task_for_consumer(&db, &stranger, &submitted.task.id).await;
        assert!(matches!(read, Err(AppError::OracleTaskNotFound(_))));
        let cancel = cancel_task(&db, &stranger, &submitted.task.id, 30).await;
        assert!(matches!(cancel, Err(AppError::OracleTaskNotFound(_))));

        // Owner cancels; repeat cancel is idempotent.
        let cancelled = cancel_task(&db, &owner, &submitted.task.id, 30)
            .await
            .unwrap();
        assert_eq!(cancelled.status, OracleTaskStatus::Cancelled);
        let again = cancel_task(&db, &owner, &submitted.task.id, 30)
            .await
            .unwrap();
        assert_eq!(again.status, OracleTaskStatus::Cancelled);

        // A worker that somehow claims it later acks into Cancelled.
        let ack = worker_ack(
            &db,
            &pool,
            "tab_1",
            &submitted.task.id,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(ack, AckOutcome::Cancelled);

        db.drop().await.ok();
    }

    #[tokio::test]
    async fn queue_cap_and_pool_status() {
        let Some(db) = connect_test_database("oracle_task_status").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let mut pool = test_pool(&owner);
        pool.max_queue_length = 2;
        pool.per_user_max_inflight = 10;
        seed_pool(&db, &pool).await;

        submit_task(&db, &pool, &submitter(&owner), prompt_input("a"))
            .await
            .unwrap();
        submit_task(&db, &pool, &submitter(&owner), prompt_input("b"))
            .await
            .unwrap();
        let overflow = submit_task(&db, &pool, &submitter(&owner), prompt_input("c")).await;
        assert!(matches!(overflow, Err(AppError::OracleQueueFull(_))));

        // No workers yet: diagnosis flags the waiting queue.
        let status = pool_status(&db, &pool).await.unwrap();
        assert_eq!(status.queued, 2);
        assert_eq!(status.dispatched, 0);
        assert_eq!(status.diagnosis, "queue_waiting_for_worker");
        assert!(status.active_workers.is_empty());

        // A worker claims: status shows it.
        claim_task(&db, &pool, "tab_1", Some("v1"), Some("chatgpt.com"))
            .await
            .unwrap()
            .expect("claim");
        let status = pool_status(&db, &pool).await.unwrap();
        assert_eq!(status.queued, 1);
        assert_eq!(status.dispatched, 1);
        assert_eq!(status.diagnosis, "running");
        assert_eq!(status.active_workers.len(), 1);
        assert_eq!(status.active_workers[0].worker_label, "tab_1");
        assert_eq!(
            status.active_workers[0].script_version.as_deref(),
            Some("v1")
        );

        db.drop().await.ok();
    }
}
