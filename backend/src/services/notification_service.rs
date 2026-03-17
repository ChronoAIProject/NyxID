use std::collections::{HashMap, HashSet};

use chrono::Utc;
use mongodb::Database;
use mongodb::bson::{self, doc};
use reqwest::Client;

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::approval_request::ApprovalRequest;
use crate::models::notification_channel::{COLLECTION_NAME, DeviceToken, NotificationChannel};
use crate::services::push_service::{self, ApnsAuth, ApnsSendResult, FcmAuth, FcmSendResult};
use crate::services::telegram_service;

/// Result of a multi-channel notification delivery attempt.
pub struct NotificationResult {
    /// Channel names that successfully delivered (e.g. "telegram", "fcm", "apns")
    pub channels: Vec<String>,
    pub telegram_chat_id: Option<i64>,
    pub telegram_message_id: Option<i64>,
}

/// Result of sending push to a single device.
enum PushResult {
    Success,
    TokenInvalid,
}

/// Send an approval notification to the user via all enabled channels.
/// Returns which channels succeeded and Telegram metadata.
pub async fn send_approval_notification(
    db: &Database,
    config: &AppConfig,
    http_client: &Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    user_id: &str,
    request: &ApprovalRequest,
) -> AppResult<NotificationResult> {
    let channel = get_or_create_channel(db, user_id).await?;

    let mut channels_used: Vec<String> = Vec::new();
    let mut telegram_chat_id = None;
    let mut telegram_message_id = None;
    let mut tokens_to_remove: Vec<String> = Vec::new();

    // 1. Telegram (existing behavior)
    if channel.telegram_enabled
        && let Some(chat_id) = channel.telegram_chat_id
    {
        let bot_token = config.telegram_bot_token.as_deref();

        if let Some(bot_token) = bot_token {
            let requester_label = request
                .requester_label
                .as_deref()
                .unwrap_or(&request.requester_type);

            match telegram_service::send_approval_message(
                http_client,
                bot_token,
                chat_id,
                &request.id,
                &request.service_name,
                &request.service_slug,
                requester_label,
                &request.operation_summary,
                channel.approval_timeout_secs,
            )
            .await
            {
                Ok(message_id) => {
                    channels_used.push("telegram".to_string());
                    telegram_chat_id = Some(chat_id);
                    telegram_message_id = Some(message_id);
                }
                Err(e) => tracing::warn!("Telegram notification failed: {e}"),
            }
        }
    }

    // 2. Push notifications (FCM + APNs) -- fire in parallel
    if channel.push_enabled && !channel.push_devices.is_empty() {
        let unique_devices = unique_devices_by_token(&channel.push_devices);
        let mut data = HashMap::new();
        data.insert("type".to_string(), "approval_request".to_string());
        data.insert("request_id".to_string(), request.id.clone());
        data.insert("challenge_id".to_string(), request.id.clone());
        data.insert(
            "deeplink".to_string(),
            format!("nyxid://challenge/{}", request.id),
        );

        let push_futures: Vec<_> = unique_devices
            .iter()
            .map(|device| {
                send_push_to_device(
                    http_client,
                    fcm_auth,
                    apns_auth,
                    config,
                    device,
                    "Approval Required",
                    "A service is requesting access",
                    &data,
                )
            })
            .collect();

        let results = futures::future::join_all(push_futures).await;

        let mut successful_device_ids: Vec<String> = Vec::new();
        for (i, result) in results.into_iter().enumerate() {
            match result {
                Ok(PushResult::Success) => {
                    let platform = &unique_devices[i].platform;
                    if !channels_used.contains(platform) {
                        channels_used.push(platform.clone());
                    }
                    successful_device_ids.push(unique_devices[i].device_id.clone());
                }
                Ok(PushResult::TokenInvalid) => {
                    tokens_to_remove.push(unique_devices[i].device_id.clone());
                }
                Err(e) => {
                    tracing::warn!(
                        device_id = %unique_devices[i].device_id,
                        "Push notification failed: {e}"
                    );
                }
            }
        }

        // Update last_used_at for successfully delivered devices (fire-and-forget)
        if !successful_device_ids.is_empty() {
            let db_clone = db.clone();
            let channel_id = channel.id.clone();
            tokio::spawn(async move {
                update_device_last_used(&db_clone, &channel_id, &successful_device_ids).await;
            });
        }
    }

    // 3. Remove invalid tokens (fire-and-forget)
    if !tokens_to_remove.is_empty() {
        let db = db.clone();
        let channel_id = channel.id.clone();
        tokio::spawn(async move {
            remove_stale_device_tokens(&db, &channel_id, &tokens_to_remove).await;
        });
    }

    if channels_used.is_empty() {
        return Err(AppError::BadRequest(
            "No notification channel is configured and enabled".to_string(),
        ));
    }

    Ok(NotificationResult {
        channels: channels_used,
        telegram_chat_id,
        telegram_message_id,
    })
}

/// Edit the notification message after a decision is made.
/// Also sends a silent push to update mobile app UI.
pub async fn notify_decision(
    config: &AppConfig,
    http_client: &Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    db: &Database,
    request: &ApprovalRequest,
    approved: bool,
) -> AppResult<()> {
    // 1. Edit Telegram message (existing behavior)
    if request
        .notification_channel
        .as_deref()
        .is_some_and(|ch| ch.contains("telegram"))
        && let (Some(chat_id), Some(message_id)) =
            (request.telegram_chat_id, request.telegram_message_id)
    {
        let bot_token = config
            .telegram_bot_token
            .as_deref()
            .ok_or_else(|| AppError::Internal("Telegram bot token not configured".to_string()))?;

        telegram_service::edit_message_after_decision(
            http_client,
            bot_token,
            chat_id,
            message_id,
            approved,
            &request.service_name,
        )
        .await?;
    }

    // 2. Send silent push to update mobile app UI
    let channel = get_or_create_channel(db, &request.user_id).await?;
    if channel.push_enabled && !channel.push_devices.is_empty() {
        let unique_devices = unique_devices_by_token(&channel.push_devices);
        let decision_str = if approved { "approved" } else { "rejected" };
        let mut data = HashMap::new();
        data.insert("type".to_string(), "approval_decision".to_string());
        data.insert("request_id".to_string(), request.id.clone());
        data.insert("decision".to_string(), decision_str.to_string());

        for device in unique_devices {
            let _ = send_silent_push(http_client, fcm_auth, apns_auth, config, device, &data).await;
        }
    }

    Ok(())
}

/// Send silent push notifications to a user's devices with custom data.
/// Used by the expiry task to notify mobile apps of expired requests.
pub async fn send_silent_push_to_user(
    db: &Database,
    config: &AppConfig,
    http_client: &Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    user_id: &str,
    data: &HashMap<String, String>,
) -> AppResult<()> {
    let channel = get_or_create_channel(db, user_id).await?;
    if channel.push_enabled && !channel.push_devices.is_empty() {
        let unique_devices = unique_devices_by_token(&channel.push_devices);
        for device in unique_devices {
            let _ = send_silent_push(http_client, fcm_auth, apns_auth, config, device, data).await;
        }
    }
    Ok(())
}

/// Get the user's notification channel settings, creating defaults if none exist.
pub async fn get_or_create_channel(db: &Database, user_id: &str) -> AppResult<NotificationChannel> {
    let collection = db.collection::<NotificationChannel>(COLLECTION_NAME);

    if let Some(channel) = collection.find_one(doc! { "user_id": user_id }).await? {
        return Ok(channel);
    }

    let now = Utc::now();
    let channel = NotificationChannel {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        telegram_chat_id: None,
        telegram_username: None,
        telegram_enabled: false,
        telegram_link_code: None,
        telegram_link_code_expires_at: None,
        approval_timeout_secs: 30,
        grant_expiry_days: 30,
        approval_required: false,
        push_enabled: false,
        push_devices: vec![],
        created_at: now,
        updated_at: now,
    };

    match collection.insert_one(&channel).await {
        Ok(_) => Ok(channel),
        Err(e) if is_duplicate_key_error(&e) => {
            // Another request created it first; fetch the existing channel
            collection
                .find_one(doc! { "user_id": user_id })
                .await?
                .ok_or_else(|| AppError::Internal("Channel creation conflict".to_string()))
        }
        Err(e) => Err(AppError::DatabaseError(e)),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Send a push notification to a single device via the appropriate platform.
#[allow(clippy::too_many_arguments)]
async fn send_push_to_device(
    http_client: &Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    config: &AppConfig,
    device: &DeviceToken,
    title: &str,
    body: &str,
    data: &HashMap<String, String>,
) -> AppResult<PushResult> {
    match device.platform.as_str() {
        "fcm" => {
            let fcm = fcm_auth
                .ok_or_else(|| AppError::Internal("FCM auth not configured".to_string()))?;
            let project_id = config
                .fcm_project_id
                .as_deref()
                .ok_or_else(|| AppError::Internal("FCM project ID not configured".to_string()))?;

            match push_service::send_fcm_notification(
                http_client,
                fcm,
                project_id,
                &device.token,
                title,
                body,
                data,
            )
            .await?
            {
                FcmSendResult::Success { .. } => Ok(PushResult::Success),
                FcmSendResult::TokenInvalid => Ok(PushResult::TokenInvalid),
                FcmSendResult::Failed { reason } => Err(AppError::Internal(reason)),
            }
        }
        "apns" => {
            let apns = apns_auth
                .ok_or_else(|| AppError::Internal("APNs auth not configured".to_string()))?;
            let topic = device
                .app_id
                .as_deref()
                .or(config.apns_topic.as_deref())
                .ok_or_else(|| AppError::Internal("APNs topic not configured".to_string()))?;

            match push_service::send_apns_notification(
                http_client,
                apns,
                &device.token,
                topic,
                config.apns_sandbox,
                title,
                body,
                data,
            )
            .await?
            {
                ApnsSendResult::Success => Ok(PushResult::Success),
                ApnsSendResult::TokenInvalid => Ok(PushResult::TokenInvalid),
                ApnsSendResult::Failed { reason } => Err(AppError::Internal(reason)),
            }
        }
        other => {
            tracing::warn!(platform = %other, "Unknown push platform, skipping");
            Err(AppError::Internal(format!(
                "Unknown push platform: {other}"
            )))
        }
    }
}

/// Send a silent push notification to a single device (for UI refresh).
async fn send_silent_push(
    http_client: &Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    config: &AppConfig,
    device: &DeviceToken,
    data: &HashMap<String, String>,
) -> AppResult<()> {
    match device.platform.as_str() {
        "fcm" => {
            if let (Some(fcm), Some(project_id)) = (fcm_auth, config.fcm_project_id.as_deref()) {
                let _ = push_service::send_fcm_silent(
                    http_client,
                    fcm,
                    project_id,
                    &device.token,
                    data,
                )
                .await;
            }
        }
        "apns" => {
            if let Some(apns) = apns_auth {
                let topic = device
                    .app_id
                    .as_deref()
                    .or(config.apns_topic.as_deref())
                    .unwrap_or("fun.chrono-ai.nyxid");

                let _ = push_service::send_apns_silent(
                    http_client,
                    apns,
                    &device.token,
                    topic,
                    config.apns_sandbox,
                    data,
                )
                .await;
            }
        }
        _ => {}
    }

    Ok(())
}

/// Update `last_used_at` timestamp for devices that successfully received a push.
async fn update_device_last_used(db: &Database, channel_id: &str, device_ids: &[String]) {
    let now = bson::DateTime::from_chrono(chrono::Utc::now());
    for device_id in device_ids {
        let _ = db
            .collection::<NotificationChannel>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": channel_id, "push_devices.device_id": device_id },
                doc! { "$set": { "push_devices.$.last_used_at": now } },
            )
            .await;
    }
}

/// Remove device tokens that FCM/APNs reported as invalid.
async fn remove_stale_device_tokens(db: &Database, channel_id: &str, device_ids: &[String]) {
    let now = bson::DateTime::from_chrono(chrono::Utc::now());
    let result = db
        .collection::<NotificationChannel>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": channel_id },
            doc! {
                "$pull": {
                    "push_devices": {
                        "device_id": { "$in": device_ids }
                    }
                },
                "$set": {
                    "updated_at": now,
                },
            },
        )
        .await;

    match result {
        Ok(r) => {
            if r.modified_count > 0 {
                tracing::info!(count = device_ids.len(), "Removed stale device tokens");

                // Keep push_enabled consistent with device availability.
                let _ = db
                    .collection::<NotificationChannel>(COLLECTION_NAME)
                    .update_one(
                        doc! {
                            "_id": channel_id,
                            "push_enabled": true,
                            "push_devices.0": { "$exists": false },
                        },
                        doc! {
                            "$set": {
                                "push_enabled": false,
                                "updated_at": bson::DateTime::from_chrono(chrono::Utc::now()),
                            }
                        },
                    )
                    .await;
            }
        }
        Err(e) => tracing::warn!("Failed to remove stale device tokens: {e}"),
    }
}

fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    if let mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we)) =
        e.kind.as_ref()
    {
        return we.code == 11000;
    }
    false
}

fn unique_devices_by_token(devices: &[DeviceToken]) -> Vec<&DeviceToken> {
    let mut seen_tokens: HashSet<&str> = HashSet::new();
    let mut unique = Vec::with_capacity(devices.len());

    for device in devices {
        if seen_tokens.insert(device.token.as_str()) {
            unique.push(device);
        }
    }

    unique
}
