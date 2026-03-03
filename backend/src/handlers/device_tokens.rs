use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::notification_channel::{COLLECTION_NAME, DeviceToken, NotificationChannel};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, notification_service};

/// Maximum number of push devices per user.
const MAX_DEVICES_PER_USER: usize = 10;

// --- Request types ---

#[derive(Debug, Deserialize)]
pub struct RegisterDeviceRequest {
    pub platform: String,
    pub token: String,
    pub device_name: Option<String>,
    pub app_id: Option<String>,
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
    Json(body): Json<RegisterDeviceRequest>,
) -> AppResult<Json<DeviceResponse>> {
    // Input validation
    validate_register_request(&body)?;

    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;
    let collection = state.db.collection::<NotificationChannel>(COLLECTION_NAME);
    let now = Utc::now();
    let bson_now = bson::DateTime::from_chrono(now);

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

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "push_device_registered".to_string(),
        Some(serde_json::json!({
            "device_id": device_id,
            "platform": body.platform,
        })),
        None,
        None,
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
    Path(device_id): Path<String>,
) -> AppResult<Json<MessageResponse>> {
    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;

    // Verify the device exists for this user
    let device_exists = channel
        .push_devices
        .iter()
        .any(|d| d.device_id == device_id);
    if !device_exists {
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
    collection
        .update_one(
            doc! {
                "_id": &channel.id,
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

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "push_device_removed".to_string(),
        Some(serde_json::json!({ "device_id": device_id })),
        None,
        None,
    );

    Ok(Json(MessageResponse {
        message: "Device removed".to_string(),
    }))
}

// --- Validation ---

fn validate_register_request(body: &RegisterDeviceRequest) -> AppResult<()> {
    if body.platform != "fcm" && body.platform != "apns" {
        return Err(AppError::ValidationError(
            "platform must be 'fcm' or 'apns'".to_string(),
        ));
    }

    if body.token.is_empty() {
        return Err(AppError::ValidationError(
            "token must not be empty".to_string(),
        ));
    }

    if body.token.len() > 4096 {
        return Err(AppError::ValidationError(
            "token must not exceed 4096 characters".to_string(),
        ));
    }

    // H-2: Validate token content by platform (defense-in-depth against URL manipulation)
    if body.platform == "apns" && !body.token.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::ValidationError(
            "APNs token must contain only hexadecimal characters".to_string(),
        ));
    }
    if body.platform == "fcm"
        && !body
            .token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '-' || c == '_')
    {
        return Err(AppError::ValidationError(
            "FCM token contains invalid characters".to_string(),
        ));
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

fn ensure_platform_matches(existing: &DeviceToken, requested_platform: &str) -> AppResult<()> {
    if existing.platform != requested_platform {
        return Err(AppError::ValidationError(format!(
            "token is already registered as platform '{}' and cannot be used as '{}'",
            existing.platform, requested_platform
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_valid_fcm() {
        let body = RegisterDeviceRequest {
            platform: "fcm".to_string(),
            token: "test-token".to_string(),
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
}
