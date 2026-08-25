use axum::{Json, extract::State};
use chrono::Utc;
use mongodb::bson::{self, doc};
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::notification_channel::{COLLECTION_NAME, NotificationChannel};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, notification_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct NotificationSettingsResponse {
    pub id: String,
    pub telegram_connected: bool,
    pub telegram_link_pending: bool,
    pub telegram_username: Option<String>,
    pub telegram_enabled: bool,
    pub approval_required: bool,
    pub approval_suspended: bool,
    pub approval_timeout_secs: u32,
    pub grant_expiry_days: u32,
    pub push_enabled: bool,
    pub push_device_count: usize,
    pub updated_at: String,
}

/// Assistant postcondition projection for notification settings.  The detail
/// response may contain an upstream Telegram username; the projection omits
/// it and retains only booleans, counters, the binding id, and a timestamp.
#[derive(Debug, Serialize)]
pub struct NotificationSettingsAuthorizationEvidenceResponse {
    pub id: String,
    pub telegram_connected: bool,
    pub telegram_link_pending: bool,
    pub telegram_enabled: bool,
    pub approval_required: bool,
    pub approval_timeout_secs: u32,
    pub grant_expiry_days: u32,
    pub push_enabled: bool,
    pub push_device_count: usize,
    pub updated_at: String,
}

impl NotificationSettingsAuthorizationEvidenceResponse {
    pub fn from_settings_response(response: &NotificationSettingsResponse) -> Self {
        Self {
            id: response.id.clone(),
            telegram_connected: response.telegram_connected,
            telegram_link_pending: response.telegram_link_pending,
            telegram_enabled: response.telegram_enabled,
            approval_required: response.approval_required,
            approval_timeout_secs: response.approval_timeout_secs,
            grant_expiry_days: response.grant_expiry_days,
            push_enabled: response.push_enabled,
            push_device_count: response.push_device_count,
            updated_at: response.updated_at.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateNotificationSettingsRequest {
    pub telegram_enabled: Option<bool>,
    pub approval_required: Option<bool>,
    pub approval_timeout_secs: Option<u32>,
    pub grant_expiry_days: Option<u32>,
    pub push_enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TelegramLinkResponse {
    pub link_code: String,
    pub bot_username: String,
    pub expires_in_secs: u32,
    pub instructions: String,
}

#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
}

// --- Handlers ---

/// GET /api/v1/notifications/settings
pub async fn get_settings(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<NotificationSettingsResponse>> {
    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;

    Ok(Json(to_settings_response(&channel)))
}

/// GET /api/v1/notifications/settings/authorization
pub async fn get_settings_authorization(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<NotificationSettingsAuthorizationEvidenceResponse>> {
    let Json(detail) = get_settings(State(state), auth_user).await?;
    Ok(Json(
        NotificationSettingsAuthorizationEvidenceResponse::from_settings_response(&detail),
    ))
}

/// PUT /api/v1/notifications/settings
pub async fn update_settings(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<UpdateNotificationSettingsRequest>,
) -> AppResult<Json<NotificationSettingsResponse>> {
    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;

    // Validate ranges
    if let Some(timeout) = body.approval_timeout_secs
        && !(10..=300).contains(&timeout)
    {
        return Err(AppError::ValidationError(
            "approval_timeout_secs must be between 10 and 300".to_string(),
        ));
    }
    if let Some(days) = body.grant_expiry_days
        && !(1..=365).contains(&days)
    {
        return Err(AppError::ValidationError(
            "grant_expiry_days must be between 1 and 365".to_string(),
        ));
    }

    // Cannot enable Telegram without a linked chat
    if body.telegram_enabled == Some(true) && channel.telegram_chat_id.is_none() {
        return Err(AppError::BadRequest(
            "Cannot enable Telegram notifications without linking your Telegram account first"
                .to_string(),
        ));
    }

    // Cannot enable push without at least one registered device
    if body.push_enabled == Some(true) && channel.push_devices.is_empty() {
        return Err(AppError::BadRequest(
            "Cannot enable push notifications without registering at least one device first"
                .to_string(),
        ));
    }

    if explicitly_enables_approval_without_active_channel(&channel, &body) {
        return Err(AppError::BadRequest(
            "Approval protection requires at least one enabled notification channel. Keep Telegram or push notifications enabled, or disable approval protection first.".to_string(),
        ));
    }

    let now = bson::DateTime::from_chrono(Utc::now());
    let mut update_doc = doc! { "updated_at": now };

    if let Some(v) = body.telegram_enabled {
        update_doc.insert("telegram_enabled", v);
    }
    if let Some(v) = body.approval_required {
        update_doc.insert("approval_required", v);
    }
    if let Some(v) = body.approval_timeout_secs {
        debug_assert!(
            v <= i32::MAX as u32,
            "approval_timeout_secs exceeds i32::MAX"
        );
        update_doc.insert("approval_timeout_secs", v as i32);
    }
    if let Some(v) = body.grant_expiry_days {
        debug_assert!(v <= i32::MAX as u32, "grant_expiry_days exceeds i32::MAX");
        update_doc.insert("grant_expiry_days", v as i32);
    }
    if let Some(v) = body.push_enabled {
        update_doc.insert("push_enabled", v);
    }

    state
        .db
        .collection::<NotificationChannel>(COLLECTION_NAME)
        .update_one(doc! { "_id": &channel.id }, doc! { "$set": update_doc })
        .await?;

    let updated = notification_service::get_or_create_channel(&state.db, &user_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "notification_settings_updated",
        Some(serde_json::json!({
            "telegram_enabled": updated.telegram_enabled,
            "approval_required": updated.approval_required,
            "approval_suspended": updated.approval_required
                && !notification_service::has_active_notification_channel(&updated),
            "approval_timeout_secs": updated.approval_timeout_secs,
            "grant_expiry_days": updated.grant_expiry_days,
            "push_enabled": updated.push_enabled,
        })),
    );

    Ok(Json(to_settings_response(&updated)))
}

// TODO(telemetry): emit `notification.channel_linked { channel: "telegram" }`
// from the Telegram webhook handler (`handlers/telegram_webhook.rs`), where
// the link is actually completed when the user sends the /start code to the
// bot. This handler only generates the link code; emitting here would misfire
// whenever a user starts but never finishes the flow. Webhook file is out of
// scope for this chunk.
//
/// POST /api/v1/notifications/telegram/link
///
/// Generate a one-time link code for connecting Telegram account.
pub async fn telegram_link(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<TelegramLinkResponse>> {
    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;

    // Generate an 8-character alphanumeric code (~41 bits of entropy)
    let code: String = {
        let mut rng = rand::thread_rng();
        (0..8)
            .map(|_| {
                let idx = rng.gen_range(0..36);
                if idx < 10 {
                    (b'0' + idx) as char
                } else {
                    (b'A' + idx - 10) as char
                }
            })
            .collect()
    };
    let link_code = format!("NYXID-{code}");

    let expires_at = Utc::now() + chrono::Duration::minutes(5);
    let now = bson::DateTime::from_chrono(Utc::now());

    state
        .db
        .collection::<NotificationChannel>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": &channel.id },
            doc! {
                "$set": {
                    "telegram_link_code": &link_code,
                    "telegram_link_code_expires_at": bson::DateTime::from_chrono(expires_at),
                    "updated_at": now,
                }
            },
        )
        .await?;

    let bot_username = state
        .config
        .telegram_bot_username
        .clone()
        .unwrap_or_else(|| "NyxIDBot".to_string());

    Ok(Json(TelegramLinkResponse {
        link_code: link_code.clone(),
        bot_username: bot_username.clone(),
        expires_in_secs: 300,
        instructions: format!("Send /start {link_code} to @{bot_username} on Telegram"),
    }))
}

/// DELETE /api/v1/notifications/telegram
///
/// Disconnect Telegram from the user's notification settings.
pub async fn telegram_disconnect(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
) -> AppResult<Json<MessageResponse>> {
    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;

    let now = bson::DateTime::from_chrono(Utc::now());
    let set_doc = doc! {
        "telegram_chat_id": bson::Bson::Null,
        "telegram_username": bson::Bson::Null,
        "telegram_enabled": false,
        "telegram_link_code": bson::Bson::Null,
        "telegram_link_code_expires_at": bson::Bson::Null,
        "updated_at": now,
    };

    state
        .db
        .collection::<NotificationChannel>(COLLECTION_NAME)
        .update_one(doc! { "_id": &channel.id }, doc! { "$set": set_doc })
        .await?;

    let updated_channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;
    let approval_suspended = updated_channel.approval_required
        && !notification_service::has_active_notification_channel(&updated_channel);

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "telegram_disconnected",
        if approval_suspended {
            Some(serde_json::json!({ "approval_suspended": true }))
        } else {
            None
        },
    );

    // Telemetry: notification.channel_unlinked (Telegram). Only emit
    // when the pre-update state actually had Telegram linked --
    // otherwise a repeated tap / retry / no-op disconnect would
    // fabricate unlink activity. The endpoint is idempotent, so this
    // handler must be too.
    let telegram_was_linked = channel.telegram_chat_id.is_some() || channel.telegram_enabled;
    if telegram_was_linked {
        emit_event(
            state.telemetry.as_deref(),
            &user_id,
            auth_user.api_key_id.as_deref(),
            &tele,
            TelemetryEvent::NotificationChannelUnlinked {
                channel: "telegram".to_string(),
            },
        );
    }

    let message = if approval_suspended {
        "Telegram disconnected. Approval protection is suspended until a notification channel is available; it resumes automatically.".to_string()
    } else {
        "Telegram disconnected".to_string()
    };

    Ok(Json(MessageResponse { message }))
}

fn to_settings_response(channel: &NotificationChannel) -> NotificationSettingsResponse {
    NotificationSettingsResponse {
        id: channel.id.clone(),
        telegram_connected: channel.telegram_chat_id.is_some(),
        telegram_link_pending: channel.telegram_link_code.is_some()
            && channel
                .telegram_link_code_expires_at
                .is_some_and(|expires_at| expires_at > Utc::now()),
        telegram_username: channel.telegram_username.clone(),
        telegram_enabled: channel.telegram_enabled,
        approval_required: channel.approval_required,
        approval_suspended: channel.approval_required
            && !notification_service::has_active_notification_channel(channel),
        approval_timeout_secs: channel.approval_timeout_secs,
        grant_expiry_days: channel.grant_expiry_days,
        push_enabled: channel.push_enabled,
        push_device_count: channel.push_devices.len(),
        updated_at: channel.updated_at.to_rfc3339(),
    }
}

fn has_enabled_notification_channel_after_update(
    channel: &NotificationChannel,
    body: &UpdateNotificationSettingsRequest,
) -> bool {
    let telegram_enabled = body.telegram_enabled.unwrap_or(channel.telegram_enabled);
    let push_enabled = body.push_enabled.unwrap_or(channel.push_enabled);

    (telegram_enabled && channel.telegram_chat_id.is_some())
        || (push_enabled && !channel.push_devices.is_empty())
}

fn explicitly_enables_approval_without_active_channel(
    channel: &NotificationChannel,
    body: &UpdateNotificationSettingsRequest,
) -> bool {
    !channel.approval_required
        && body.approval_required == Some(true)
        && !has_enabled_notification_channel_after_update(channel, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_channel() -> NotificationChannel {
        NotificationChannel {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn allows_approval_when_push_is_enabled_with_registered_device() {
        let mut channel = make_channel();
        channel
            .push_devices
            .push(crate::models::notification_channel::DeviceToken {
                device_id: uuid::Uuid::new_v4().to_string(),
                platform: "fcm".to_string(),
                token: "token".to_string(),
                device_name: None,
                app_id: None,
                registered_at: Utc::now(),
                last_used_at: None,
            });

        let body = UpdateNotificationSettingsRequest {
            telegram_enabled: None,
            approval_required: Some(true),
            approval_timeout_secs: None,
            grant_expiry_days: None,
            push_enabled: Some(true),
        };

        assert!(!explicitly_enables_approval_without_active_channel(
            &channel, &body
        ));
    }

    #[test]
    fn allows_disabling_last_channel_while_approval_preference_stays_enabled() {
        let mut channel = make_channel();
        channel.push_enabled = true;
        channel.approval_required = true;
        channel
            .push_devices
            .push(crate::models::notification_channel::DeviceToken {
                device_id: uuid::Uuid::new_v4().to_string(),
                platform: "fcm".to_string(),
                token: "token".to_string(),
                device_name: None,
                app_id: None,
                registered_at: Utc::now(),
                last_used_at: None,
            });

        let body = UpdateNotificationSettingsRequest {
            telegram_enabled: None,
            // The web form submits all values, including an unchanged `true`.
            approval_required: Some(true),
            approval_timeout_secs: None,
            grant_expiry_days: None,
            push_enabled: Some(false),
        };

        assert!(!has_enabled_notification_channel_after_update(
            &channel, &body
        ));
        assert!(!explicitly_enables_approval_without_active_channel(
            &channel, &body
        ));
    }

    #[test]
    fn allows_disabling_approval_and_last_channel_together() {
        let mut channel = make_channel();
        channel.push_enabled = true;
        channel.approval_required = true;
        channel
            .push_devices
            .push(crate::models::notification_channel::DeviceToken {
                device_id: uuid::Uuid::new_v4().to_string(),
                platform: "fcm".to_string(),
                token: "token".to_string(),
                device_name: None,
                app_id: None,
                registered_at: Utc::now(),
                last_used_at: None,
            });

        let body = UpdateNotificationSettingsRequest {
            telegram_enabled: None,
            approval_required: Some(false),
            approval_timeout_secs: None,
            grant_expiry_days: None,
            push_enabled: Some(false),
        };

        assert!(!body.approval_required.unwrap());
        assert!(!has_enabled_notification_channel_after_update(
            &channel, &body
        ));
        assert!(!explicitly_enables_approval_without_active_channel(
            &channel, &body
        ));
    }

    #[test]
    fn rejects_explicit_approval_enable_without_active_channel() {
        let channel = make_channel();
        let body = UpdateNotificationSettingsRequest {
            telegram_enabled: None,
            approval_required: Some(true),
            approval_timeout_secs: None,
            grant_expiry_days: None,
            push_enabled: None,
        };

        assert!(explicitly_enables_approval_without_active_channel(
            &channel, &body
        ));
    }

    // --- Pure function tests: to_settings_response ---

    #[test]
    fn to_settings_response_defaults_with_no_telegram_and_no_push() {
        let channel = make_channel();
        let resp = to_settings_response(&channel);

        assert!(!resp.telegram_connected);
        assert!(resp.telegram_username.is_none());
        assert!(!resp.telegram_enabled);
        assert!(!resp.approval_required);
        assert!(!resp.approval_suspended);
        assert_eq!(resp.approval_timeout_secs, 30);
        assert_eq!(resp.grant_expiry_days, 30);
        assert!(!resp.push_enabled);
        assert_eq!(resp.push_device_count, 0);
    }

    #[test]
    fn to_settings_response_with_telegram_connected() {
        let mut channel = make_channel();
        channel.telegram_chat_id = Some(12345);
        channel.telegram_username = Some("testuser".to_string());
        channel.telegram_enabled = true;

        let resp = to_settings_response(&channel);

        assert!(resp.telegram_connected);
        assert_eq!(resp.telegram_username.as_deref(), Some("testuser"));
        assert!(resp.telegram_enabled);
    }

    #[test]
    fn to_settings_response_counts_push_devices() {
        let mut channel = make_channel();
        channel.push_enabled = true;
        for i in 0..3 {
            channel
                .push_devices
                .push(crate::models::notification_channel::DeviceToken {
                    device_id: format!("device-{i}"),
                    platform: "fcm".to_string(),
                    token: format!("token-{i}"),
                    device_name: None,
                    app_id: None,
                    registered_at: Utc::now(),
                    last_used_at: None,
                });
        }

        let resp = to_settings_response(&channel);

        assert!(resp.push_enabled);
        assert_eq!(resp.push_device_count, 3);
    }

    #[test]
    fn to_settings_response_with_custom_approval_settings() {
        let mut channel = make_channel();
        channel.approval_required = true;
        channel.approval_timeout_secs = 120;
        channel.grant_expiry_days = 7;

        let resp = to_settings_response(&channel);

        assert!(resp.approval_required);
        assert!(resp.approval_suspended);
        assert_eq!(resp.approval_timeout_secs, 120);
        assert_eq!(resp.grant_expiry_days, 7);
    }

    #[test]
    fn notification_authorization_projection_omits_telegram_username() {
        let mut channel = make_channel();
        channel.telegram_chat_id = Some(42);
        channel.telegram_username = Some("Bearer nyxid_ag_abcdefghijklmnop".to_string());
        channel.updated_at = Utc::now();
        let detail = to_settings_response(&channel);
        let value = serde_json::to_value(
            NotificationSettingsAuthorizationEvidenceResponse::from_settings_response(&detail),
        )
        .unwrap();
        assert!(value.get("telegram_username").is_none());
        assert!(value.to_string().find("nyxid_").is_none());
        assert_eq!(value["id"], channel.id);
    }

    // --- Pure function tests: has_enabled_notification_channel_after_update edge cases ---

    #[test]
    fn has_enabled_channel_telegram_connected_and_enabled() {
        let mut channel = make_channel();
        channel.telegram_chat_id = Some(12345);
        channel.telegram_enabled = true;

        let body = UpdateNotificationSettingsRequest {
            telegram_enabled: None,
            approval_required: None,
            approval_timeout_secs: None,
            grant_expiry_days: None,
            push_enabled: None,
        };

        assert!(has_enabled_notification_channel_after_update(
            &channel, &body
        ));
    }

    #[test]
    fn has_enabled_channel_telegram_enabled_but_not_connected() {
        let mut channel = make_channel();
        channel.telegram_chat_id = None;
        channel.telegram_enabled = true;

        let body = UpdateNotificationSettingsRequest {
            telegram_enabled: None,
            approval_required: None,
            approval_timeout_secs: None,
            grant_expiry_days: None,
            push_enabled: None,
        };

        // telegram_enabled=true but no chat_id means not actually connected
        assert!(!has_enabled_notification_channel_after_update(
            &channel, &body
        ));
    }

    #[test]
    fn has_enabled_channel_push_enabled_but_no_devices() {
        let mut channel = make_channel();
        channel.push_enabled = true;
        // push_devices is empty

        let body = UpdateNotificationSettingsRequest {
            telegram_enabled: None,
            approval_required: None,
            approval_timeout_secs: None,
            grant_expiry_days: None,
            push_enabled: None,
        };

        assert!(!has_enabled_notification_channel_after_update(
            &channel, &body
        ));
    }

    #[test]
    fn has_enabled_channel_both_channels_active() {
        let mut channel = make_channel();
        channel.telegram_chat_id = Some(12345);
        channel.telegram_enabled = true;
        channel.push_enabled = true;
        channel
            .push_devices
            .push(crate::models::notification_channel::DeviceToken {
                device_id: "d1".to_string(),
                platform: "apns".to_string(),
                token: "tok1".to_string(),
                device_name: None,
                app_id: None,
                registered_at: Utc::now(),
                last_used_at: None,
            });

        let body = UpdateNotificationSettingsRequest {
            telegram_enabled: None,
            approval_required: None,
            approval_timeout_secs: None,
            grant_expiry_days: None,
            push_enabled: None,
        };

        assert!(has_enabled_notification_channel_after_update(
            &channel, &body
        ));
    }

    #[test]
    fn has_enabled_channel_body_overrides_take_precedence() {
        let mut channel = make_channel();
        channel.telegram_chat_id = Some(12345);
        channel.telegram_enabled = true;

        // Body disables telegram
        let body = UpdateNotificationSettingsRequest {
            telegram_enabled: Some(false),
            approval_required: None,
            approval_timeout_secs: None,
            grant_expiry_days: None,
            push_enabled: None,
        };

        assert!(!has_enabled_notification_channel_after_update(
            &channel, &body
        ));
    }

    #[test]
    fn has_enabled_channel_body_enables_telegram_on_connected_channel() {
        let mut channel = make_channel();
        channel.telegram_chat_id = Some(12345);
        channel.telegram_enabled = false; // currently disabled

        // Body enables telegram
        let body = UpdateNotificationSettingsRequest {
            telegram_enabled: Some(true),
            approval_required: None,
            approval_timeout_secs: None,
            grant_expiry_days: None,
            push_enabled: None,
        };

        assert!(has_enabled_notification_channel_after_update(
            &channel, &body
        ));
    }

    // --- Serialization tests: NotificationSettingsResponse ---

    #[test]
    fn notification_settings_response_serialization() {
        let resp = NotificationSettingsResponse {
            id: "binding-1".to_string(),
            telegram_connected: true,
            telegram_link_pending: false,
            telegram_username: Some("alice".to_string()),
            telegram_enabled: true,
            approval_required: false,
            approval_suspended: false,
            approval_timeout_secs: 60,
            grant_expiry_days: 14,
            push_enabled: true,
            push_device_count: 2,
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["telegram_connected"], true);
        assert_eq!(json["telegram_username"], "alice");
        assert_eq!(json["telegram_enabled"], true);
        assert_eq!(json["approval_required"], false);
        assert_eq!(json["approval_suspended"], false);
        assert_eq!(json["approval_timeout_secs"], 60);
        assert_eq!(json["grant_expiry_days"], 14);
        assert_eq!(json["push_enabled"], true);
        assert_eq!(json["push_device_count"], 2);
    }

    #[test]
    fn notification_settings_response_null_username() {
        let resp = NotificationSettingsResponse {
            id: "binding-1".to_string(),
            telegram_connected: false,
            telegram_link_pending: false,
            telegram_username: None,
            telegram_enabled: false,
            approval_required: false,
            approval_suspended: false,
            approval_timeout_secs: 30,
            grant_expiry_days: 30,
            push_enabled: false,
            push_device_count: 0,
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["telegram_connected"], false);
        assert!(json["telegram_username"].is_null());
    }

    // --- Serialization tests: TelegramLinkResponse ---

    #[test]
    fn telegram_link_response_serialization() {
        let resp = TelegramLinkResponse {
            link_code: "NYXID-ABC12345".to_string(),
            bot_username: "NyxIDBot".to_string(),
            expires_in_secs: 300,
            instructions: "Send /start NYXID-ABC12345 to @NyxIDBot on Telegram".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["link_code"], "NYXID-ABC12345");
        assert_eq!(json["bot_username"], "NyxIDBot");
        assert_eq!(json["expires_in_secs"], 300);
        assert!(json["instructions"].as_str().unwrap().contains("/start"));
    }

    // --- Serialization tests: MessageResponse ---

    #[test]
    fn message_response_serialization() {
        let resp = MessageResponse {
            message: "Telegram disconnected".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["message"], "Telegram disconnected");
    }

    // --- Deserialization tests: UpdateNotificationSettingsRequest ---

    #[test]
    fn update_notification_settings_request_deserialization_all_fields() {
        let json = serde_json::json!({
            "telegram_enabled": true,
            "approval_required": false,
            "approval_timeout_secs": 120,
            "grant_expiry_days": 7,
            "push_enabled": true
        });
        let req: UpdateNotificationSettingsRequest = serde_json::from_value(json).unwrap();

        assert_eq!(req.telegram_enabled, Some(true));
        assert_eq!(req.approval_required, Some(false));
        assert_eq!(req.approval_timeout_secs, Some(120));
        assert_eq!(req.grant_expiry_days, Some(7));
        assert_eq!(req.push_enabled, Some(true));
    }

    #[test]
    fn update_notification_settings_request_deserialization_empty_body() {
        let json = serde_json::json!({});
        let req: UpdateNotificationSettingsRequest = serde_json::from_value(json).unwrap();

        assert!(req.telegram_enabled.is_none());
        assert!(req.approval_required.is_none());
        assert!(req.approval_timeout_secs.is_none());
        assert!(req.grant_expiry_days.is_none());
        assert!(req.push_enabled.is_none());
    }
}
