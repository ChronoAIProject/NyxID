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
///
/// `org_name` is the display name of the owning org when
/// `request.from_org_policy` is true. Pass `None` for personal requests
/// (or when the lookup failed); the resulting Telegram / push wording is
/// then byte-identical to the pre-org behavior so non-org callers are
/// unaffected.
#[allow(clippy::too_many_arguments)]
pub async fn send_approval_notification(
    db: &Database,
    config: &AppConfig,
    http_client: &Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    user_id: &str,
    request: &ApprovalRequest,
    org_name: Option<&str>,
) -> AppResult<NotificationResult> {
    let channel = get_or_create_channel(db, user_id).await?;

    // Only treat `org_name` as meaningful when the request itself carries
    // the org-policy flag. This keeps the "from_org_policy is the
    // authoritative signal" invariant shared by the DTO mapper.
    let effective_org_name = if request.from_org_policy {
        org_name
    } else {
        None
    };

    let mut channels_used: Vec<String> = Vec::new();
    let mut telegram_chat_id = None;
    let mut telegram_message_id = None;
    let mut tokens_to_remove: Vec<String> = Vec::new();

    // 1. Telegram (existing behavior; wording switches when org-scoped)
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
                request
                    .action_description
                    .as_deref()
                    .unwrap_or(&request.operation_summary),
                channel.approval_timeout_secs,
                effective_org_name,
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
        if let Some(method) = &request.http_method {
            data.insert("http_method".to_string(), method.clone());
        }
        if let Some(resource) = &request.resource {
            data.insert("resource".to_string(), resource.clone());
        }
        if let Some(verb) = &request.verb {
            data.insert("verb".to_string(), verb.clone());
        }
        // When the request is created under an org policy, inject the
        // org context so the mobile app can render the org badge on
        // the detail screen opened via the deeplink before the list
        // endpoint is re-fetched. Keys are only added when defined —
        // missing keys are tolerated by the client.
        if request.from_org_policy {
            data.insert("from_org_policy".to_string(), "true".to_string());
            data.insert("org_id".to_string(), request.user_id.clone());
            if let Some(name) = effective_org_name {
                data.insert("org_name".to_string(), name.to_string());
            }
        }

        // Push title/body switch when the request is org-scoped so admins
        // can distinguish an org decision from a personal one from the
        // lock-screen preview alone.
        let (push_title, push_body_owned): (&str, Option<String>) = match effective_org_name {
            Some(name) => (
                "Org Approval Required",
                Some(format!("{name} admins: a service is requesting access")),
            ),
            None => ("Approval Required", None),
        };
        let push_body: &str = push_body_owned
            .as_deref()
            .unwrap_or("A service is requesting access");

        let push_futures: Vec<_> = unique_devices
            .iter()
            .map(|device| {
                send_push_to_device(
                    http_client,
                    fcm_auth,
                    apns_auth,
                    config,
                    device,
                    push_title,
                    push_body,
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

    // 2. Send silent push to update mobile app UI.
    //
    // For personal requests we push to the request owner. For org-policy
    // requests the owner is the *org*, which has no notification channel of
    // its own -- the actual mobile clients waiting on a decision are the
    // org admins that were recorded on `notify_user_ids` at request time.
    // Fan out silent push to every recorded admin so every admin app clears
    // the pending state after one admin decides. Falls back to `[user_id]`
    // for legacy rows without `notify_user_ids` (see
    // ChronoAIProject/NyxID#370).
    let decision_str = if approved { "approved" } else { "rejected" };
    let mut data = HashMap::new();
    data.insert("type".to_string(), "approval_decision".to_string());
    data.insert("request_id".to_string(), request.id.clone());
    data.insert("decision".to_string(), decision_str.to_string());

    let recipients: Vec<String> = if request.notify_user_ids.is_empty() {
        vec![request.user_id.clone()]
    } else {
        request.notify_user_ids.clone()
    };

    for recipient in recipients {
        let channel = match get_or_create_channel(db, &recipient).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    recipient = %recipient,
                    error = %e,
                    "Failed to load notification channel for decision silent push"
                );
                continue;
            }
        };
        if !channel.push_enabled || channel.push_devices.is_empty() {
            continue;
        }
        let unique_devices = unique_devices_by_token(&channel.push_devices);
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

                let _ = db
                    .collection::<NotificationChannel>(COLLECTION_NAME)
                    .update_one(
                        doc! {
                            "_id": channel_id,
                            "approval_required": true,
                            "push_devices.0": { "$exists": false },
                            "$or": [
                                { "telegram_enabled": { "$ne": true } },
                                { "telegram_chat_id": bson::Bson::Null },
                            ],
                        },
                        doc! {
                            "$set": {
                                "approval_required": false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::connect_test_database;

    fn make_device(device_id: &str, platform: &str, token: &str) -> DeviceToken {
        DeviceToken {
            device_id: device_id.to_string(),
            platform: platform.to_string(),
            token: token.to_string(),
            device_name: None,
            app_id: None,
            registered_at: Utc::now(),
            last_used_at: None,
        }
    }

    #[test]
    fn unique_devices_deduplicates_by_token() {
        let devices = vec![
            make_device("d1", "fcm", "token-aaa"),
            make_device("d2", "fcm", "token-aaa"),
            make_device("d3", "apns", "token-bbb"),
        ];
        let unique = unique_devices_by_token(&devices);
        assert_eq!(unique.len(), 2);
        assert_eq!(unique[0].device_id, "d1");
        assert_eq!(unique[1].device_id, "d3");
    }

    #[test]
    fn unique_devices_preserves_order_of_first_seen() {
        let devices = vec![
            make_device("d1", "apns", "token-x"),
            make_device("d2", "fcm", "token-y"),
            make_device("d3", "apns", "token-x"),
            make_device("d4", "fcm", "token-z"),
        ];
        let unique = unique_devices_by_token(&devices);
        assert_eq!(unique.len(), 3);
        assert_eq!(unique[0].device_id, "d1");
        assert_eq!(unique[1].device_id, "d2");
        assert_eq!(unique[2].device_id, "d4");
    }

    #[test]
    fn unique_devices_empty_input() {
        let devices: Vec<DeviceToken> = vec![];
        let unique = unique_devices_by_token(&devices);
        assert!(unique.is_empty());
    }

    #[tokio::test]
    async fn get_or_create_channel_creates_default_channel() {
        let Some(db) = connect_test_database("notify_create").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let channel = get_or_create_channel(&db, &user_id).await.unwrap();

        assert_eq!(channel.user_id, user_id);
        assert!(!channel.telegram_enabled);
        assert!(channel.telegram_chat_id.is_none());
        assert!(!channel.push_enabled);
        assert!(channel.push_devices.is_empty());
        assert_eq!(channel.approval_timeout_secs, 30);
        assert_eq!(channel.grant_expiry_days, 30);
        assert!(!channel.approval_required);
    }

    #[tokio::test]
    async fn get_or_create_channel_returns_existing() {
        let Some(db) = connect_test_database("notify_existing").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let first = get_or_create_channel(&db, &user_id).await.unwrap();
        let second = get_or_create_channel(&db, &user_id).await.unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.user_id, second.user_id);
    }

    #[test]
    fn unique_devices_single_device() {
        let devices = vec![make_device("d1", "fcm", "token-aaa")];
        let unique = unique_devices_by_token(&devices);
        assert_eq!(unique.len(), 1);
        assert_eq!(unique[0].device_id, "d1");
    }

    #[test]
    fn unique_devices_all_different_tokens() {
        let devices = vec![
            make_device("d1", "fcm", "token-a"),
            make_device("d2", "apns", "token-b"),
            make_device("d3", "fcm", "token-c"),
        ];
        let unique = unique_devices_by_token(&devices);
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn unique_devices_all_same_token() {
        let devices = vec![
            make_device("d1", "fcm", "token-same"),
            make_device("d2", "apns", "token-same"),
            make_device("d3", "fcm", "token-same"),
        ];
        let unique = unique_devices_by_token(&devices);
        assert_eq!(unique.len(), 1);
        assert_eq!(unique[0].device_id, "d1");
    }

    #[test]
    fn make_device_populates_required_fields() {
        let d = make_device("d1", "fcm", "t1");
        assert_eq!(d.device_id, "d1");
        assert_eq!(d.platform, "fcm");
        assert_eq!(d.token, "t1");
        assert!(d.device_name.is_none());
        assert!(d.app_id.is_none());
        assert!(d.last_used_at.is_none());
    }

    // ================================================================
    // Integration tests (require running MongoDB)
    // ================================================================

    use mongodb::bson::doc;

    #[tokio::test]
    async fn get_or_create_channel_default_values_are_sane() {
        let Some(db) = connect_test_database("notif_svc_defaults").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let channel = get_or_create_channel(&db, &user_id).await.unwrap();

        // Verify all default values
        assert_eq!(channel.user_id, user_id);
        assert!(!channel.telegram_enabled);
        assert!(channel.telegram_chat_id.is_none());
        assert!(channel.telegram_username.is_none());
        assert!(channel.telegram_link_code.is_none());
        assert!(channel.telegram_link_code_expires_at.is_none());
        assert_eq!(channel.approval_timeout_secs, 30);
        assert_eq!(channel.grant_expiry_days, 30);
        assert!(!channel.approval_required);
        assert!(!channel.push_enabled);
        assert!(channel.push_devices.is_empty());
    }

    #[tokio::test]
    async fn get_or_create_channel_idempotent_for_same_user() {
        let Some(db) = connect_test_database("notif_svc_idempotent").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();

        let first = get_or_create_channel(&db, &user_id).await.unwrap();
        let second = get_or_create_channel(&db, &user_id).await.unwrap();
        let third = get_or_create_channel(&db, &user_id).await.unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.id, third.id);
    }

    #[tokio::test]
    async fn get_or_create_channel_different_users_get_different_channels() {
        let Some(db) = connect_test_database("notif_svc_diff_users").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_a = uuid::Uuid::new_v4().to_string();
        let user_b = uuid::Uuid::new_v4().to_string();

        let channel_a = get_or_create_channel(&db, &user_a).await.unwrap();
        let channel_b = get_or_create_channel(&db, &user_b).await.unwrap();

        assert_ne!(channel_a.id, channel_b.id);
        assert_eq!(channel_a.user_id, user_a);
        assert_eq!(channel_b.user_id, user_b);
    }

    #[tokio::test]
    async fn get_or_create_channel_returns_modified_channel() {
        let Some(db) = connect_test_database("notif_svc_modified").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();

        // Create the channel
        let channel = get_or_create_channel(&db, &user_id).await.unwrap();
        assert!(!channel.approval_required);

        // Manually update approval_required in DB
        let now = bson::DateTime::from_chrono(Utc::now());
        db.collection::<NotificationChannel>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &channel.id },
                doc! { "$set": {
                    "approval_required": true,
                    "approval_timeout_secs": 60,
                    "updated_at": now,
                }},
            )
            .await
            .unwrap();

        // Fetch again -- should return the modified state
        let updated = get_or_create_channel(&db, &user_id).await.unwrap();
        assert!(updated.approval_required);
        assert_eq!(updated.approval_timeout_secs, 60);
        assert_eq!(updated.id, channel.id);
    }

    #[tokio::test]
    async fn get_or_create_channel_with_push_devices_persisted() {
        let Some(db) = connect_test_database("notif_svc_push_devs").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();

        // Create the channel
        let channel = get_or_create_channel(&db, &user_id).await.unwrap();
        assert!(channel.push_devices.is_empty());

        // Manually add a push device in DB
        let device = DeviceToken {
            device_id: uuid::Uuid::new_v4().to_string(),
            platform: "fcm".to_string(),
            token: "fcm-test-token-12345".to_string(),
            device_name: Some("Test Phone".to_string()),
            app_id: Some("dev.nyxid.app".to_string()),
            registered_at: Utc::now(),
            last_used_at: None,
        };
        let device_doc = bson::to_bson(&device).unwrap();
        let now = bson::DateTime::from_chrono(Utc::now());
        db.collection::<NotificationChannel>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &channel.id },
                doc! {
                    "$push": { "push_devices": device_doc },
                    "$set": { "push_enabled": true, "updated_at": now },
                },
            )
            .await
            .unwrap();

        // Fetch again
        let updated = get_or_create_channel(&db, &user_id).await.unwrap();
        assert!(updated.push_enabled);
        assert_eq!(updated.push_devices.len(), 1);
        assert_eq!(updated.push_devices[0].platform, "fcm");
        assert_eq!(updated.push_devices[0].token, "fcm-test-token-12345");
        assert_eq!(
            updated.push_devices[0].device_name.as_deref(),
            Some("Test Phone")
        );
    }

    #[tokio::test]
    async fn get_or_create_channel_telegram_fields_persist() {
        let Some(db) = connect_test_database("notif_svc_telegram").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();

        let channel = get_or_create_channel(&db, &user_id).await.unwrap();

        // Simulate Telegram link
        let now = bson::DateTime::from_chrono(Utc::now());
        db.collection::<NotificationChannel>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &channel.id },
                doc! { "$set": {
                    "telegram_chat_id": 12345_i64,
                    "telegram_username": "testuser",
                    "telegram_enabled": true,
                    "updated_at": now,
                }},
            )
            .await
            .unwrap();

        let updated = get_or_create_channel(&db, &user_id).await.unwrap();
        assert!(updated.telegram_enabled);
        assert_eq!(updated.telegram_chat_id, Some(12345));
        assert_eq!(updated.telegram_username.as_deref(), Some("testuser"));
    }

    #[tokio::test]
    async fn remove_stale_device_tokens_removes_specified_tokens() {
        let Some(db) = connect_test_database("notif_svc_rm_stale").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let channel = get_or_create_channel(&db, &user_id).await.unwrap();

        // Add two devices
        let d1 = DeviceToken {
            device_id: "dev-1".to_string(),
            platform: "fcm".to_string(),
            token: "token-1".to_string(),
            device_name: None,
            app_id: None,
            registered_at: Utc::now(),
            last_used_at: None,
        };
        let d2 = DeviceToken {
            device_id: "dev-2".to_string(),
            platform: "apns".to_string(),
            token: "token-2".to_string(),
            device_name: None,
            app_id: None,
            registered_at: Utc::now(),
            last_used_at: None,
        };
        let d1_doc = bson::to_bson(&d1).unwrap();
        let d2_doc = bson::to_bson(&d2).unwrap();
        let now = bson::DateTime::from_chrono(Utc::now());
        db.collection::<NotificationChannel>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &channel.id },
                doc! {
                    "$set": {
                        "push_devices": [d1_doc, d2_doc],
                        "push_enabled": true,
                        "updated_at": now,
                    }
                },
            )
            .await
            .unwrap();

        // Remove dev-1 as stale
        remove_stale_device_tokens(&db, &channel.id, &["dev-1".to_string()]).await;

        let updated = get_or_create_channel(&db, &user_id).await.unwrap();
        assert_eq!(updated.push_devices.len(), 1);
        assert_eq!(updated.push_devices[0].device_id, "dev-2");
    }

    #[tokio::test]
    async fn remove_stale_device_tokens_disables_push_when_empty() {
        let Some(db) = connect_test_database("notif_svc_rm_stale_disable").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let channel = get_or_create_channel(&db, &user_id).await.unwrap();

        // Add one device
        let d = DeviceToken {
            device_id: "dev-only".to_string(),
            platform: "fcm".to_string(),
            token: "token-only".to_string(),
            device_name: None,
            app_id: None,
            registered_at: Utc::now(),
            last_used_at: None,
        };
        let d_doc = bson::to_bson(&d).unwrap();
        let now = bson::DateTime::from_chrono(Utc::now());
        db.collection::<NotificationChannel>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &channel.id },
                doc! {
                    "$set": {
                        "push_devices": [d_doc],
                        "push_enabled": true,
                        "updated_at": now,
                    }
                },
            )
            .await
            .unwrap();

        // Remove the only device
        remove_stale_device_tokens(&db, &channel.id, &["dev-only".to_string()]).await;

        let updated = get_or_create_channel(&db, &user_id).await.unwrap();
        assert!(updated.push_devices.is_empty());
        assert!(!updated.push_enabled);
    }

    #[tokio::test]
    async fn update_device_last_used_sets_timestamp() {
        let Some(db) = connect_test_database("notif_svc_upd_last_used").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let channel = get_or_create_channel(&db, &user_id).await.unwrap();

        // Add a device without last_used_at
        let d = DeviceToken {
            device_id: "dev-1".to_string(),
            platform: "fcm".to_string(),
            token: "token-1".to_string(),
            device_name: None,
            app_id: None,
            registered_at: Utc::now(),
            last_used_at: None,
        };
        let d_doc = bson::to_bson(&d).unwrap();
        let now = bson::DateTime::from_chrono(Utc::now());
        db.collection::<NotificationChannel>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &channel.id },
                doc! {
                    "$set": {
                        "push_devices": [d_doc],
                        "push_enabled": true,
                        "updated_at": now,
                    }
                },
            )
            .await
            .unwrap();

        // Update last_used_at
        update_device_last_used(&db, &channel.id, &["dev-1".to_string()]).await;

        let updated = get_or_create_channel(&db, &user_id).await.unwrap();
        assert!(updated.push_devices[0].last_used_at.is_some());
    }
}
