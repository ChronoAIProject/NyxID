use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use mongodb::Collection;
use mongodb::bson::{self, Document, doc};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::notification_channel::{COLLECTION_NAME, DeviceToken, NotificationChannel};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, notification_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

/// Maximum number of push devices per user.
const MAX_DEVICES_PER_USER: usize = 10;

// --- Request types ---

#[derive(Debug, Deserialize)]
pub struct RegisterDeviceRequest {
    pub platform: String,
    pub token: String,
    pub previous_token: Option<String>,
    pub device_name: Option<String>,
    pub app_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UnregisterCurrentDeviceRequest {
    pub platform: String,
    pub token: String,
}

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct DeviceResponse {
    pub device_id: String,
    pub platform: String,
    pub device_name: Option<String>,
    pub registered_at: String,
}

#[derive(Debug, Serialize)]
pub struct DeviceListItem {
    pub device_id: String,
    pub platform: String,
    pub device_name: Option<String>,
    pub registered_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeviceListResponse {
    pub devices: Vec<DeviceListItem>,
    pub push_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

// --- Handlers ---

/// POST /api/v1/notifications/devices
///
/// Register a device token for push notifications.
/// If a device with the same `token` already exists, updates its metadata.
pub async fn register_device(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<RegisterDeviceRequest>,
) -> AppResult<Json<DeviceResponse>> {
    // Input validation
    validate_register_request(&body)?;

    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;
    let collection = state.db.collection::<NotificationChannel>(COLLECTION_NAME);
    let now = Utc::now();
    let bson_now = bson::DateTime::from_chrono(now);

    let resolved_prev = body
        .previous_token
        .as_deref()
        .filter(|prev| *prev != body.token.as_str());

    // Ensure one push token belongs to one user at a time (prevents account-switch leakage).
    detach_token_from_other_users(&collection, &user_id, &body.token, bson_now).await?;
    if let Some(prev) = resolved_prev {
        detach_token_from_other_users(&collection, &user_id, prev, bson_now).await?;
    }

    // Rotation path: replace `previous_token` with the new token on the same device record.
    if let Some(previous_token) = resolved_prev
        && let Some(existing) = channel
            .push_devices
            .iter()
            .find(|d| d.token == previous_token)
    {
        ensure_platform_matches(existing, &body.platform)?;

        let device_id = existing.device_id.clone();
        let mut update_doc = doc! {
            "push_devices.$.token": &body.token,
            "push_devices.$.platform": &body.platform,
            "push_devices.$.registered_at": bson_now,
            "updated_at": bson_now,
        };

        if let Some(ref name) = body.device_name {
            update_doc.insert("push_devices.$.device_name", name);
        }
        if let Some(ref app_id) = body.app_id {
            update_doc.insert("push_devices.$.app_id", app_id);
        }

        collection
            .update_one(
                doc! {
                    "_id": &channel.id,
                    "push_devices.device_id": &device_id,
                },
                doc! { "$set": update_doc },
            )
            .await?;

        remove_duplicate_token_entries(
            &collection,
            &channel.id,
            &body.token,
            &device_id,
            bson::DateTime::from_chrono(Utc::now()),
        )
        .await?;

        // NO registration emit here. This is the same-device token-rotation
        // path (APNs/FCM replaces the push token on the existing
        // `device_id`); it is not a new-device acquisition. Matches the
        // existing-token refresh path below, which also suppresses the
        // event. The emit survives only on the true new-device insert path.

        return Ok(Json(DeviceResponse {
            device_id,
            platform: body.platform.clone(),
            device_name: body.device_name.clone().or(existing.device_name.clone()),
            registered_at: now.to_rfc3339(),
        }));
    }

    // Check if device with this token already exists (token refresh)
    let existing_device = channel.push_devices.iter().find(|d| d.token == body.token);

    if let Some(existing) = existing_device {
        ensure_platform_matches(existing, &body.platform)?;

        // Update existing device metadata
        let device_id = existing.device_id.clone();
        let mut update_doc = doc! {
            "push_devices.$.registered_at": bson_now,
            "updated_at": bson_now,
        };

        if let Some(ref name) = body.device_name {
            update_doc.insert("push_devices.$.device_name", name);
        }
        if let Some(ref app_id) = body.app_id {
            update_doc.insert("push_devices.$.app_id", app_id);
        }

        collection
            .update_one(
                doc! {
                    "_id": &channel.id,
                    "push_devices.token": &body.token,
                },
                doc! { "$set": update_doc },
            )
            .await?;

        remove_duplicate_token_entries(
            &collection,
            &channel.id,
            &body.token,
            &device_id,
            bson::DateTime::from_chrono(Utc::now()),
        )
        .await?;

        // Not a new registration — the mobile app hits this path on every
        // sign-in / session restore via `activatePushAfterLogin({ forceRegister: true })`.
        // Emitting here would count every re-auth as a fresh device and inflate
        // the registration funnel. Telemetry fires only on the new-device insert
        // path below.

        return Ok(Json(DeviceResponse {
            device_id,
            platform: existing.platform.clone(),
            device_name: body.device_name.clone().or(existing.device_name.clone()),
            registered_at: now.to_rfc3339(),
        }));
    }

    // New device
    let device_token = DeviceToken {
        device_id: uuid::Uuid::new_v4().to_string(),
        platform: body.platform.clone(),
        token: body.token.clone(),
        device_name: body.device_name.clone(),
        app_id: body.app_id.clone(),
        registered_at: now,
        last_used_at: None,
    };

    let device_id = device_token.device_id.clone();

    // Atomically check array size and push to prevent TOCTOU race.
    // "push_devices.9" existing means the array already has 10+ elements (0-indexed).
    let result = collection
        .update_one(
            doc! {
                "_id": &channel.id,
                "push_devices.9": { "$exists": false },
                "push_devices.token": { "$ne": &body.token },
            },
            doc! {
                "$push": {
                    "push_devices": bson::to_bson(&device_token)
                        .map_err(|e| AppError::Internal(format!("BSON serialize failed: {e}")))?
                },
                "$set": {
                    "push_enabled": true,
                    "approval_required": true,
                    "updated_at": bson_now,
                }
            },
        )
        .await?;

    if result.matched_count == 0 {
        // Could be either: max devices reached OR concurrent registration of this token.
        let latest_channel =
            notification_service::get_or_create_channel(&state.db, &user_id).await?;
        if let Some(existing) = latest_channel
            .push_devices
            .iter()
            .find(|d| d.token == body.token)
        {
            ensure_platform_matches(existing, &body.platform)?;

            // Token already exists (likely concurrent request) -- refresh metadata and return it.
            let mut update_doc = doc! {
                "push_devices.$.registered_at": bson_now,
                "updated_at": bson_now,
            };
            if let Some(ref name) = body.device_name {
                update_doc.insert("push_devices.$.device_name", name);
            }
            if let Some(ref app_id) = body.app_id {
                update_doc.insert("push_devices.$.app_id", app_id);
            }

            collection
                .update_one(
                    doc! {
                        "_id": &latest_channel.id,
                        "push_devices.token": &body.token,
                    },
                    doc! { "$set": update_doc },
                )
                .await?;

            remove_duplicate_token_entries(
                &collection,
                &latest_channel.id,
                &body.token,
                &existing.device_id,
                bson::DateTime::from_chrono(Utc::now()),
            )
            .await?;

            // Another request already persisted this token. The "primary" emit
            // has already fired there, so skip here to avoid double-counting.

            return Ok(Json(DeviceResponse {
                device_id: existing.device_id.clone(),
                platform: existing.platform.clone(),
                device_name: body.device_name.clone().or(existing.device_name.clone()),
                registered_at: now.to_rfc3339(),
            }));
        }

        return Err(AppError::BadRequest(format!(
            "Maximum of {MAX_DEVICES_PER_USER} devices per user exceeded"
        )));
    }

    remove_duplicate_token_entries(
        &collection,
        &channel.id,
        &body.token,
        &device_id,
        bson::DateTime::from_chrono(Utc::now()),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "push_device_registered",
        Some(serde_json::json!({
            "device_id": device_id,
            "platform": body.platform,
        })),
    );

    // Telemetry: notification.device_registered (new-device path).
    emit_event(
        state.telemetry.as_deref(),
        &user_id,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::NotificationDeviceRegistered {
            platform: body.platform.clone(),
        },
    );

    Ok(Json(DeviceResponse {
        device_id,
        platform: body.platform,
        device_name: body.device_name,
        registered_at: now.to_rfc3339(),
    }))
}

/// GET /api/v1/notifications/devices
///
/// List all registered push notification devices.
/// Token values are NOT returned (they are secrets).
pub async fn list_devices(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<DeviceListResponse>> {
    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;

    let devices: Vec<DeviceListItem> = channel
        .push_devices
        .iter()
        .map(|d| DeviceListItem {
            device_id: d.device_id.clone(),
            platform: d.platform.clone(),
            device_name: d.device_name.clone(),
            registered_at: d.registered_at.to_rfc3339(),
            last_used_at: d.last_used_at.map(|t| t.to_rfc3339()),
        })
        .collect();

    Ok(Json(DeviceListResponse {
        devices,
        push_enabled: channel.push_enabled,
    }))
}

/// DELETE /api/v1/notifications/devices/{device_id}
///
/// Remove a registered push notification device.
/// If no devices remain, auto-disables push notifications.
pub async fn remove_device(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(device_id): Path<String>,
) -> AppResult<Json<MessageResponse>> {
    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;

    // Verify the device exists for this user, and capture its platform
    // pre-delete for telemetry. The DELETE path param is only a `device_id`,
    // so platform must be resolved from the existing record before the
    // `$pull` makes it unreachable.
    let removed_platform = channel
        .push_devices
        .iter()
        .find(|d| d.device_id == device_id)
        .map(|d| d.platform.clone());
    if removed_platform.is_none() {
        return Err(AppError::NotFound("Device not found".to_string()));
    }

    let collection = state.db.collection::<NotificationChannel>(COLLECTION_NAME);
    let now = bson::DateTime::from_chrono(Utc::now());

    collection
        .update_one(
            doc! { "_id": &channel.id, "user_id": &user_id },
            doc! {
                "$pull": {
                    "push_devices": { "device_id": &device_id }
                },
                "$set": { "updated_at": now }
            },
        )
        .await?;

    // Disable push only if this update still has no remaining devices.
    // The conditional filter prevents racing with a concurrent device registration.
    // Also auto-disable approval_required if Telegram is not connected either,
    // to prevent the user from being locked out of their own services.
    let should_auto_disable_approval =
        should_auto_disable_approval_after_last_push_device_removed(&channel);
    let set_on_empty = disable_push_update_doc(should_auto_disable_approval);

    collection
        .update_one(
            doc! {
                "_id": &channel.id,
                "push_enabled": true,
                "push_devices.0": { "$exists": false },
            },
            doc! { "$set": set_on_empty },
        )
        .await?;

    let approval_auto_disabled = channel.push_devices.len() == 1 && should_auto_disable_approval;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "push_device_removed",
        Some(serde_json::json!({
            "device_id": device_id,
            "approval_auto_disabled": approval_auto_disabled,
        })),
    );

    // Telemetry: notification.device_removed. Platform is captured pre-delete
    // above; if the device record ever slipped past the existence check we
    // would have already errored out, so unwrap is safe here.
    if let Some(platform) = removed_platform {
        emit_event(
            state.telemetry.as_deref(),
            &user_id,
            auth_user.api_key_id.as_deref(),
            &tele,
            TelemetryEvent::NotificationDeviceRemoved { platform },
        );
    }

    let message = if approval_auto_disabled && channel.approval_required {
        "Device removed. Approval protection has been disabled because no notification channels remain.".to_string()
    } else {
        "Device removed".to_string()
    };

    Ok(Json(MessageResponse { message }))
}

/// DELETE /api/v1/notifications/devices/current
///
/// Remove current authenticated user's device by push token.
/// Designed for sign-out to prevent old-account push leakage.
pub async fn remove_current_device(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<UnregisterCurrentDeviceRequest>,
) -> AppResult<Json<MessageResponse>> {
    validate_token_for_platform(&body.platform, &body.token, "token")?;

    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;
    let collection = state.db.collection::<NotificationChannel>(COLLECTION_NAME);
    let now = bson::DateTime::from_chrono(Utc::now());

    // Gate telemetry on whether the pull actually removed a matching device.
    // Logout fires this endpoint even when the local token is stale / already
    // rotated / never registered; we don't want to count those as real removals.
    let token_was_present = channel.push_devices.iter().any(|d| d.token == body.token);

    collection
        .update_one(
            doc! { "_id": &channel.id, "user_id": &user_id },
            doc! {
                "$pull": {
                    "push_devices": {
                        "token": &body.token
                    }
                },
                "$set": { "updated_at": now }
            },
        )
        .await?;

    collection
        .update_one(
            doc! {
                "_id": &channel.id,
                "push_enabled": true,
                "push_devices.0": { "$exists": false },
            },
            doc! {
                "$set": disable_push_update_doc(
                    should_auto_disable_approval_after_last_push_device_removed(&channel)
                )
            },
        )
        .await?;

    let approval_auto_disabled = channel.push_devices.len() == 1
        && should_auto_disable_approval_after_last_push_device_removed(&channel);

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "push_device_removed_on_logout",
        Some(serde_json::json!({
            "platform": body.platform,
            "token_removed": true,
            "approval_auto_disabled": approval_auto_disabled,
        })),
    );

    // Telemetry: notification.device_removed (sign-out path). Only fires when
    // the pull actually removed a token from this user's channel.
    if token_was_present {
        emit_event(
            state.telemetry.as_deref(),
            &user_id,
            auth_user.api_key_id.as_deref(),
            &tele,
            TelemetryEvent::NotificationDeviceRemoved {
                platform: body.platform.clone(),
            },
        );
    }

    let message = if approval_auto_disabled {
        "Current device removed. Approval protection has been disabled because no notification channels remain.".to_string()
    } else {
        "Current device removed".to_string()
    };

    Ok(Json(MessageResponse { message }))
}

async fn detach_token_from_other_users(
    collection: &Collection<NotificationChannel>,
    user_id: &str,
    token: &str,
    now: bson::DateTime,
) -> AppResult<()> {
    let removed = collection
        .update_many(
            doc! {
                "user_id": { "$ne": user_id },
                "push_devices.token": token,
            },
            doc! {
                "$pull": {
                    "push_devices": { "token": token }
                },
                "$set": {
                    "updated_at": now,
                }
            },
        )
        .await?;

    if removed.modified_count > 0 {
        // Keep push_enabled in sync for users whose last token was removed.
        collection
            .update_many(
                doc! {
                    "user_id": { "$ne": user_id },
                    "push_enabled": true,
                    "push_devices.0": { "$exists": false },
                },
                doc! {
                    "$set": {
                        "push_enabled": false,
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    }
                },
            )
            .await?;

        // Avoid leaving approvals enabled for users who no longer have any
        // active notification channels after token detachment.
        collection
            .update_many(
                doc! {
                    "user_id": { "$ne": user_id },
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
                        "updated_at": bson::DateTime::from_chrono(Utc::now()),
                    }
                },
            )
            .await?;
    }

    Ok(())
}

async fn remove_duplicate_token_entries(
    collection: &Collection<NotificationChannel>,
    channel_id: &str,
    token: &str,
    keep_device_id: &str,
    now: bson::DateTime,
) -> AppResult<()> {
    collection
        .update_one(
            doc! { "_id": channel_id },
            doc! {
                "$pull": {
                    "push_devices": {
                        "token": token,
                        "device_id": { "$ne": keep_device_id }
                    }
                },
                "$set": {
                    "updated_at": now,
                }
            },
        )
        .await?;
    Ok(())
}

// --- Validation ---

fn validate_register_request(body: &RegisterDeviceRequest) -> AppResult<()> {
    if body.platform != "fcm" && body.platform != "apns" {
        return Err(AppError::ValidationError(
            "platform must be 'fcm' or 'apns'".to_string(),
        ));
    }

    validate_token_for_platform(&body.platform, &body.token, "token")?;
    if let Some(previous_token) = body.previous_token.as_deref() {
        validate_token_for_platform(&body.platform, previous_token, "previous_token")?;
    }

    if let Some(ref name) = body.device_name
        && name.len() > 100
    {
        return Err(AppError::ValidationError(
            "device_name must not exceed 100 characters".to_string(),
        ));
    }

    // APNs requires app_id for the apns-topic header
    if body.platform == "apns" && body.app_id.as_ref().is_none_or(|s| s.is_empty()) {
        return Err(AppError::ValidationError(
            "app_id is required for APNs platform".to_string(),
        ));
    }

    // M-5: Validate app_id length
    if let Some(ref app_id) = body.app_id
        && app_id.len() > 256
    {
        return Err(AppError::ValidationError(
            "app_id must not exceed 256 characters".to_string(),
        ));
    }

    Ok(())
}

fn validate_token_for_platform(platform: &str, token: &str, field_name: &str) -> AppResult<()> {
    if token.is_empty() {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not be empty"
        )));
    }

    if token.len() > 4096 {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not exceed 4096 characters"
        )));
    }

    if platform == "apns" && !token.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::ValidationError(format!(
            "{field_name} must contain only hexadecimal characters for APNs platform"
        )));
    }

    if platform == "fcm"
        && !token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '-' || c == '_')
    {
        return Err(AppError::ValidationError(format!(
            "{field_name} contains invalid characters for FCM platform"
        )));
    }

    Ok(())
}

fn ensure_platform_matches(existing: &DeviceToken, requested_platform: &str) -> AppResult<()> {
    if existing.platform != requested_platform {
        return Err(AppError::ValidationError(format!(
            "token is already registered as platform '{}' and cannot be used as '{}'",
            existing.platform, requested_platform
        )));
    }
    Ok(())
}

fn telegram_channel_is_active(channel: &NotificationChannel) -> bool {
    channel.telegram_enabled && channel.telegram_chat_id.is_some()
}

fn should_auto_disable_approval_after_last_push_device_removed(
    channel: &NotificationChannel,
) -> bool {
    channel.approval_required && !telegram_channel_is_active(channel)
}

fn disable_push_update_doc(disable_approval: bool) -> Document {
    let mut update = doc! {
        "push_enabled": false,
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    if disable_approval {
        update.insert("approval_required", false);
    }
    update
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_valid_fcm() {
        let body = RegisterDeviceRequest {
            platform: "fcm".to_string(),
            token: "test-token".to_string(),
            previous_token: None,
            device_name: Some("Pixel 8".to_string()),
            app_id: None,
        };
        assert!(validate_register_request(&body).is_ok());
    }

    #[test]
    fn validate_valid_apns() {
        let body = RegisterDeviceRequest {
            platform: "apns".to_string(),
            token: "a1b2c3d4e5f60011223344556677889900aabbccddeeff0011223344556677".to_string(),
            previous_token: None,
            device_name: Some("iPhone".to_string()),
            app_id: Some("dev.nyxid.app".to_string()),
        };
        assert!(validate_register_request(&body).is_ok());
    }

    #[test]
    fn validate_invalid_platform() {
        let body = RegisterDeviceRequest {
            platform: "invalid".to_string(),
            token: "test".to_string(),
            previous_token: None,
            device_name: None,
            app_id: None,
        };
        assert!(validate_register_request(&body).is_err());
    }

    #[test]
    fn validate_empty_token() {
        let body = RegisterDeviceRequest {
            platform: "fcm".to_string(),
            token: "".to_string(),
            previous_token: None,
            device_name: None,
            app_id: None,
        };
        assert!(validate_register_request(&body).is_err());
    }

    #[test]
    fn validate_token_too_long() {
        let body = RegisterDeviceRequest {
            platform: "fcm".to_string(),
            token: "x".repeat(4097),
            previous_token: None,
            device_name: None,
            app_id: None,
        };
        assert!(validate_register_request(&body).is_err());
    }

    #[test]
    fn validate_previous_token_too_long() {
        let body = RegisterDeviceRequest {
            platform: "fcm".to_string(),
            token: "valid-token".to_string(),
            previous_token: Some("x".repeat(4097)),
            device_name: None,
            app_id: None,
        };
        assert!(validate_register_request(&body).is_err());
    }

    #[test]
    fn validate_previous_token_rejects_invalid_chars() {
        let body = RegisterDeviceRequest {
            platform: "fcm".to_string(),
            token: "valid-token".to_string(),
            previous_token: Some("token/with/slash".to_string()),
            device_name: None,
            app_id: None,
        };
        assert!(validate_register_request(&body).is_err());
    }

    #[test]
    fn validate_device_name_too_long() {
        let body = RegisterDeviceRequest {
            platform: "fcm".to_string(),
            token: "test".to_string(),
            previous_token: None,
            device_name: Some("x".repeat(101)),
            app_id: None,
        };
        assert!(validate_register_request(&body).is_err());
    }

    #[test]
    fn validate_apns_missing_app_id() {
        let body = RegisterDeviceRequest {
            platform: "apns".to_string(),
            token: "test".to_string(),
            previous_token: None,
            device_name: None,
            app_id: None,
        };
        assert!(validate_register_request(&body).is_err());
    }

    #[test]
    fn validate_apns_token_hex_only() {
        let body = RegisterDeviceRequest {
            platform: "apns".to_string(),
            token: "abcdef0123456789".to_string(),
            previous_token: None,
            device_name: None,
            app_id: Some("dev.nyxid.app".to_string()),
        };
        assert!(validate_register_request(&body).is_ok());
    }

    #[test]
    fn validate_apns_token_rejects_non_hex() {
        let body = RegisterDeviceRequest {
            platform: "apns".to_string(),
            token: "not-valid-hex!".to_string(),
            previous_token: None,
            device_name: None,
            app_id: Some("dev.nyxid.app".to_string()),
        };
        assert!(validate_register_request(&body).is_err());
    }

    #[test]
    fn validate_fcm_token_rejects_special_chars() {
        let body = RegisterDeviceRequest {
            platform: "fcm".to_string(),
            token: "token/with/slashes".to_string(),
            previous_token: None,
            device_name: None,
            app_id: None,
        };
        assert!(validate_register_request(&body).is_err());
    }

    #[test]
    fn validate_fcm_token_allows_valid_chars() {
        let body = RegisterDeviceRequest {
            platform: "fcm".to_string(),
            token: "abc123:def-456_ghi".to_string(),
            previous_token: None,
            device_name: None,
            app_id: None,
        };
        assert!(validate_register_request(&body).is_ok());
    }

    #[test]
    fn validate_app_id_too_long() {
        let body = RegisterDeviceRequest {
            platform: "fcm".to_string(),
            token: "valid-token".to_string(),
            previous_token: None,
            device_name: None,
            app_id: Some("x".repeat(257)),
        };
        assert!(validate_register_request(&body).is_err());
    }

    #[test]
    fn validate_app_id_at_max_length() {
        let body = RegisterDeviceRequest {
            platform: "fcm".to_string(),
            token: "valid-token".to_string(),
            previous_token: None,
            device_name: None,
            app_id: Some("x".repeat(256)),
        };
        assert!(validate_register_request(&body).is_ok());
    }

    #[test]
    fn platform_match_allows_same_platform() {
        let existing = DeviceToken {
            device_id: "dev-1".to_string(),
            platform: "fcm".to_string(),
            token: "tok-1".to_string(),
            device_name: None,
            app_id: None,
            registered_at: Utc::now(),
            last_used_at: None,
        };
        assert!(ensure_platform_matches(&existing, "fcm").is_ok());
    }

    #[test]
    fn platform_match_rejects_mismatched_platform() {
        let existing = DeviceToken {
            device_id: "dev-2".to_string(),
            platform: "apns".to_string(),
            token: "a1b2c3".to_string(),
            device_name: None,
            app_id: Some("dev.nyxid.app".to_string()),
            registered_at: Utc::now(),
            last_used_at: None,
        };
        assert!(ensure_platform_matches(&existing, "fcm").is_err());
    }

    #[test]
    fn auto_disables_approval_when_last_push_device_is_removed_without_telegram() {
        let channel = NotificationChannel {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            telegram_chat_id: None,
            telegram_username: None,
            telegram_enabled: false,
            telegram_link_code: None,
            telegram_link_code_expires_at: None,
            approval_timeout_secs: 30,
            grant_expiry_days: 30,
            approval_required: true,
            push_enabled: true,
            push_devices: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(should_auto_disable_approval_after_last_push_device_removed(
            &channel
        ));
    }

    #[test]
    fn keeps_approval_when_telegram_channel_is_active() {
        let channel = NotificationChannel {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            telegram_chat_id: Some(1234),
            telegram_username: Some("nyx".to_string()),
            telegram_enabled: true,
            telegram_link_code: None,
            telegram_link_code_expires_at: None,
            approval_timeout_secs: 30,
            grant_expiry_days: 30,
            approval_required: true,
            push_enabled: true,
            push_devices: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(!should_auto_disable_approval_after_last_push_device_removed(&channel));
    }
}
