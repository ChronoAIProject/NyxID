use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::Database;
use mongodb::bson::{self, doc};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument};
use sha2::{Digest, Sha256};

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::approval_grant::{ApprovalGrant, COLLECTION_NAME as GRANTS};
use crate::models::approval_request::{ApprovalRequest, COLLECTION_NAME as REQUESTS};
use crate::models::notification_channel::{COLLECTION_NAME as CHANNELS, NotificationChannel};
use crate::models::service_approval_config::{
    COLLECTION_NAME as SERVICE_APPROVAL_CONFIGS, ServiceApprovalConfig,
};
use crate::services::notification_service;

/// Check whether a user has the global approval system enabled.
pub async fn user_requires_approval(db: &Database, user_id: &str) -> AppResult<bool> {
    Ok(user_global_approval_setting(db, user_id)
        .await?
        .unwrap_or(false))
}

/// Check whether approval is required for a specific service.
///
/// Resolution order:
/// 1. If a `ServiceApprovalConfig` exists for (user, service), use its value.
/// 2. Otherwise, fall back to the global `notification_channels.approval_required`.
pub async fn requires_approval_for_service(
    db: &Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<bool> {
    // Check per-service override first
    let per_service = db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .find_one(doc! { "user_id": user_id, "service_id": service_id })
        .await?;

    let global = user_requires_approval(db, user_id).await?;
    Ok(resolve_approval_requirement(
        per_service.map(|c| c.approval_required),
        Some(global),
    ))
}

async fn user_global_approval_setting(db: &Database, user_id: &str) -> AppResult<Option<bool>> {
    let channel = db
        .collection::<NotificationChannel>(CHANNELS)
        .find_one(doc! { "user_id": user_id })
        .await?;
    Ok(channel.map(|c| c.approval_required))
}

fn resolve_approval_requirement(per_service: Option<bool>, global: Option<bool>) -> bool {
    per_service.or(global).unwrap_or(false)
}

/// Check whether the request has a valid (non-expired, non-revoked) approval grant.
/// Returns Ok(true) if access is granted, Ok(false) if approval is needed.
pub async fn check_approval(
    db: &Database,
    user_id: &str,
    service_id: &str,
    requester_type: &str,
    requester_id: &str,
) -> AppResult<bool> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let grant = db
        .collection::<ApprovalGrant>(GRANTS)
        .find_one(doc! {
            "user_id": user_id,
            "service_id": service_id,
            "requester_type": requester_type,
            "requester_id": requester_id,
            "revoked": false,
            "expires_at": { "$gt": now },
        })
        .await?;

    Ok(grant.is_some())
}

/// Create an approval request (idempotent via idempotency_key).
/// If a pending request with the same key exists, returns it.
/// Sends notification via the configured channel.
#[allow(clippy::too_many_arguments)]
pub async fn create_approval_request(
    db: &Database,
    config: &AppConfig,
    http_client: &reqwest::Client,
    user_id: &str,
    service_id: &str,
    service_name: &str,
    service_slug: &str,
    requester_type: &str,
    requester_id: &str,
    requester_label: Option<&str>,
    operation_summary: &str,
    timeout_secs: u32,
) -> AppResult<ApprovalRequest> {
    let collection = db.collection::<ApprovalRequest>(REQUESTS);
    let idempotency_key =
        compute_idempotency_key(user_id, service_id, requester_type, requester_id);
    let mut inserted_request: Option<ApprovalRequest> = None;
    for _attempt in 0..2 {
        // Check for existing pending request with the same idempotency key.
        // This handles normal idempotent retries and the winner in concurrent inserts.
        if let Some(existing) = collection
            .find_one(doc! {
                "idempotency_key": &idempotency_key,
                "status": "pending",
            })
            .await?
        {
            return Ok(existing);
        }

        let now = Utc::now();
        let expires_at = now + Duration::seconds(i64::from(timeout_secs));

        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            service_id: service_id.to_string(),
            service_name: service_name.to_string(),
            service_slug: service_slug.to_string(),
            requester_type: requester_type.to_string(),
            requester_id: requester_id.to_string(),
            requester_label: requester_label.map(String::from),
            operation_summary: operation_summary.to_string(),
            status: "pending".to_string(),
            idempotency_key: idempotency_key.clone(),
            notification_channel: None,
            telegram_message_id: None,
            telegram_chat_id: None,
            expires_at,
            decided_at: None,
            decision_channel: None,
            created_at: now,
        };

        match collection.insert_one(&request).await {
            Ok(_) => {
                inserted_request = Some(request);
                break;
            }
            Err(e) if is_duplicate_key_error(&e) => {
                // Concurrent insert race: another request inserted/processed first.
                // Retry once to read the pending row or create a new row if no longer pending.
                continue;
            }
            Err(e) => return Err(AppError::DatabaseError(e)),
        }
    }

    let request = inserted_request
        .ok_or_else(|| AppError::Conflict("Approval request conflict, please retry".to_string()))?;

    // Send notification
    match notification_service::send_approval_notification(
        db,
        config,
        http_client,
        user_id,
        &request,
    )
    .await
    {
        Ok((channel_name, chat_id, message_id)) => {
            // Update the request with notification details
            let update = doc! {
                "$set": {
                    "notification_channel": &channel_name,
                    "telegram_chat_id": chat_id,
                    "telegram_message_id": message_id,
                }
            };
            collection
                .update_one(doc! { "_id": &request.id }, update)
                .await?;

            // Return the updated request
            let updated = collection
                .find_one(doc! { "_id": &request.id })
                .await?
                .unwrap_or(request);

            Ok(updated)
        }
        Err(e) => {
            tracing::warn!("Failed to send approval notification: {e}");
            // Still return the request even if notification failed --
            // user can approve via web UI
            Ok(request)
        }
    }
}

/// Process a user's approval decision (from Telegram callback or web UI).
/// Atomically updates status from "pending" to "approved"/"rejected".
/// On approval: creates an ApprovalGrant with the user's configured expiry.
pub async fn process_decision(
    db: &Database,
    config: &AppConfig,
    http_client: &reqwest::Client,
    request_id: &str,
    approved: bool,
    decision_channel: &str,
) -> AppResult<ApprovalRequest> {
    let now = Utc::now();
    let new_status = if approved { "approved" } else { "rejected" };

    // Atomic update: only process if status is still "pending"
    let updated = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one_and_update(
            doc! {
                "_id": request_id,
                "status": "pending",
            },
            doc! {
                "$set": {
                    "status": new_status,
                    "decided_at": bson::DateTime::from_chrono(now),
                    "decision_channel": decision_channel,
                }
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| {
            AppError::NotFound("Approval request not found or already processed".to_string())
        })?;

    // On approval: create a grant
    if approved {
        let channel = notification_service::get_or_create_channel(db, &updated.user_id).await?;
        let grant_expiry = now + Duration::days(i64::from(channel.grant_expiry_days));

        let grant = ApprovalGrant {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: updated.user_id.clone(),
            service_id: updated.service_id.clone(),
            service_name: updated.service_name.clone(),
            requester_type: updated.requester_type.clone(),
            requester_id: updated.requester_id.clone(),
            requester_label: updated.requester_label.clone(),
            approval_request_id: updated.id.clone(),
            granted_at: now,
            expires_at: grant_expiry,
            revoked: false,
        };

        db.collection::<ApprovalGrant>(GRANTS)
            .insert_one(&grant)
            .await?;
    }

    // Edit the Telegram message to show decision (non-blocking)
    let request_clone = updated.clone();
    let config_clone = config.clone();
    let http_clone = http_client.clone();
    tokio::spawn(async move {
        let _ = notification_service::notify_decision(
            &config_clone,
            &http_clone,
            &request_clone,
            approved,
        )
        .await;
    });

    Ok(updated)
}

/// Expire pending requests that have passed their expiry time.
/// Called by the background task.
pub async fn expire_pending_requests(
    db: &Database,
    config: &AppConfig,
    http_client: &reqwest::Client,
) -> AppResult<u64> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let expired: Vec<ApprovalRequest> = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find(doc! {
            "status": "pending",
            "expires_at": { "$lte": now },
        })
        .with_options(FindOptions::builder().limit(100).build())
        .await?
        .try_collect()
        .await?;

    if expired.is_empty() {
        return Ok(0);
    }

    let count = expired.len() as u64;

    // Batch update status to "expired"
    let ids: Vec<&str> = expired.iter().map(|r| r.id.as_str()).collect();
    db.collection::<ApprovalRequest>(REQUESTS)
        .update_many(
            doc! { "_id": { "$in": &ids } },
            doc! { "$set": { "status": "expired" } },
        )
        .await?;

    // Edit Telegram messages for expired requests (best-effort)
    for req in &expired {
        if req.notification_channel.as_deref() == Some("telegram") {
            if let (Some(chat_id), Some(message_id)) =
                (req.telegram_chat_id, req.telegram_message_id)
            {
                if let Some(bot_token) = config.telegram_bot_token.as_deref() {
                    let http = http_client.clone();
                    let token = bot_token.to_string();
                    let svc_name = req.service_name.clone();
                    tokio::spawn(async move {
                        let _ = crate::services::telegram_service::edit_message_after_decision(
                            &http,
                            &token,
                            chat_id,
                            message_id,
                            false,
                            &format!("{svc_name} (expired)"),
                        )
                        .await;
                    });
                }
            }
        }
    }

    Ok(count)
}

/// Block until an approval decision is made or the timeout expires.
/// Returns Ok(()) if approved, Err if rejected/expired/timeout.
pub async fn wait_for_decision(
    db: &Database,
    request_id: &str,
    timeout_secs: u32,
) -> AppResult<()> {
    let poll_interval = std::time::Duration::from_millis(1000);
    let deadline = Utc::now() + Duration::seconds(i64::from(timeout_secs));

    loop {
        tokio::time::sleep(poll_interval).await;

        let request = get_request(db, request_id).await?;

        match request.status.as_str() {
            "approved" => return Ok(()),
            "rejected" => {
                return Err(AppError::Forbidden(
                    "Approval request was rejected".to_string(),
                ));
            }
            "expired" => {
                return Err(AppError::Forbidden("Approval request expired".to_string()));
            }
            "pending" => {
                if Utc::now() >= deadline {
                    return Err(AppError::Forbidden(
                        "Approval request timed out".to_string(),
                    ));
                }
            }
            other => {
                return Err(AppError::Internal(format!(
                    "Unknown approval status: {other}"
                )));
            }
        }
    }
}

/// List approval requests for a user (for history page).
pub async fn list_requests(
    db: &Database,
    user_id: &str,
    status_filter: Option<&str>,
    page: u64,
    per_page: u64,
) -> AppResult<(Vec<ApprovalRequest>, u64)> {
    let mut filter = doc! { "user_id": user_id };
    if let Some(status) = status_filter {
        filter.insert("status", status);
    }

    let total = db
        .collection::<ApprovalRequest>(REQUESTS)
        .count_documents(filter.clone())
        .await?;

    let offset = (page.saturating_sub(1)) * per_page;
    let requests: Vec<ApprovalRequest> = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find(filter)
        .with_options(
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .skip(offset)
                .limit(i64::try_from(per_page).unwrap_or(100))
                .build(),
        )
        .await?
        .try_collect()
        .await?;

    Ok((requests, total))
}

/// Get a single approval request by ID (for status polling).
pub async fn get_request(db: &Database, request_id: &str) -> AppResult<ApprovalRequest> {
    db.collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! { "_id": request_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Approval request not found".to_string()))
}

/// List active approval grants for a user.
pub async fn list_grants(
    db: &Database,
    user_id: &str,
    page: u64,
    per_page: u64,
) -> AppResult<(Vec<ApprovalGrant>, u64)> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let filter = doc! {
        "user_id": user_id,
        "revoked": false,
        "expires_at": { "$gt": now },
    };

    let total = db
        .collection::<ApprovalGrant>(GRANTS)
        .count_documents(filter.clone())
        .await?;

    let offset = (page.saturating_sub(1)) * per_page;
    let grants: Vec<ApprovalGrant> = db
        .collection::<ApprovalGrant>(GRANTS)
        .find(filter)
        .with_options(
            FindOptions::builder()
                .sort(doc! { "granted_at": -1 })
                .skip(offset)
                .limit(i64::try_from(per_page).unwrap_or(100))
                .build(),
        )
        .await?
        .try_collect()
        .await?;

    Ok((grants, total))
}

/// Revoke a specific approval grant.
pub async fn revoke_grant(db: &Database, user_id: &str, grant_id: &str) -> AppResult<()> {
    let result = db
        .collection::<ApprovalGrant>(GRANTS)
        .update_one(
            doc! {
                "_id": grant_id,
                "user_id": user_id,
            },
            doc! { "$set": { "revoked": true } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("Grant not found".to_string()));
    }

    Ok(())
}

/// Revoke all grants for a user.
#[allow(dead_code)]
pub async fn revoke_all_grants(db: &Database, user_id: &str) -> AppResult<u64> {
    let result = db
        .collection::<ApprovalGrant>(GRANTS)
        .update_many(
            doc! { "user_id": user_id, "revoked": false },
            doc! { "$set": { "revoked": true } },
        )
        .await?;

    Ok(result.modified_count)
}

/// List per-service approval configs for a user.
pub async fn list_service_approval_configs(
    db: &Database,
    user_id: &str,
) -> AppResult<Vec<ServiceApprovalConfig>> {
    let configs: Vec<ServiceApprovalConfig> = db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .find(doc! { "user_id": user_id })
        .await?
        .try_collect()
        .await?;

    Ok(configs)
}

/// Set a per-service approval config (atomic upsert).
/// If a config already exists for (user, service), it is updated.
/// Otherwise, a new config is created. Uses `findOneAndUpdate` with
/// `upsert: true` to avoid race conditions from concurrent requests.
pub async fn set_service_approval_config(
    db: &Database,
    user_id: &str,
    service_id: &str,
    service_name: &str,
    approval_required: bool,
) -> AppResult<ServiceApprovalConfig> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let collection = db.collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS);
    let filter = doc! { "user_id": user_id, "service_id": service_id };

    for _attempt in 0..2 {
        let config = collection
            .find_one_and_update(
                filter.clone(),
                doc! {
                    "$set": {
                        "approval_required": approval_required,
                        "service_name": service_name,
                        "updated_at": now,
                    },
                    "$setOnInsert": {
                        "_id": uuid::Uuid::new_v4().to_string(),
                        "user_id": user_id,
                        "service_id": service_id,
                        "created_at": now,
                    }
                },
            )
            .with_options(
                FindOneAndUpdateOptions::builder()
                    .upsert(true)
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await;

        match config {
            Ok(Some(cfg)) => return Ok(cfg),
            Ok(None) => {
                return Err(AppError::Internal(
                    "Upsert returned no document".to_string(),
                ));
            }
            Err(e) if is_duplicate_key_error(&e) => {
                // Concurrent upserts can race on the unique (user_id, service_id) index.
                // Read-after-write resolves to the winning document.
                if let Some(existing) = collection.find_one(filter.clone()).await? {
                    return Ok(existing);
                }
                continue;
            }
            Err(e) => return Err(AppError::DatabaseError(e)),
        }
    }

    Err(AppError::Conflict(
        "Per-service approval config update conflicted, please retry".to_string(),
    ))
}

/// Delete a per-service approval config (revert to global default).
pub async fn delete_service_approval_config(
    db: &Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<()> {
    let result = db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .delete_one(doc! { "user_id": user_id, "service_id": service_id })
        .await?;

    if result.deleted_count == 0 {
        return Err(AppError::NotFound(
            "Per-service approval config not found".to_string(),
        ));
    }

    Ok(())
}

fn compute_idempotency_key(
    user_id: &str,
    service_id: &str,
    requester_type: &str,
    requester_id: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    hasher.update(b":");
    hasher.update(service_id.as_bytes());
    hasher.update(b":");
    hasher.update(requester_type.as_bytes());
    hasher.update(b":");
    hasher.update(requester_id.as_bytes());
    hex::encode(hasher.finalize())
}

fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    if let mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we)) =
        e.kind.as_ref()
    {
        return we.code == 11000;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_approval_requirement_prefers_per_service_true_over_global_false() {
        assert!(resolve_approval_requirement(Some(true), Some(false)));
    }

    #[test]
    fn resolve_approval_requirement_prefers_per_service_false_over_global_true() {
        assert!(!resolve_approval_requirement(Some(false), Some(true)));
    }

    #[test]
    fn resolve_approval_requirement_falls_back_to_global_when_no_per_service() {
        assert!(resolve_approval_requirement(None, Some(true)));
        assert!(!resolve_approval_requirement(None, Some(false)));
    }

    #[test]
    fn resolve_approval_requirement_defaults_to_false_when_no_settings() {
        assert!(!resolve_approval_requirement(None, None));
    }

    #[test]
    fn idempotency_key_deterministic() {
        let key1 = compute_idempotency_key("user1", "svc1", "sa", "req1");
        let key2 = compute_idempotency_key("user1", "svc1", "sa", "req1");
        assert_eq!(key1, key2);
    }

    #[test]
    fn idempotency_key_differs_for_different_inputs() {
        let key1 = compute_idempotency_key("user1", "svc1", "sa", "req1");
        let key2 = compute_idempotency_key("user2", "svc1", "sa", "req1");
        assert_ne!(key1, key2);
    }

    #[test]
    fn idempotency_key_is_hex_sha256() {
        let key = compute_idempotency_key("u", "s", "t", "r");
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
