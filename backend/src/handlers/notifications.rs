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

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct NotificationSettingsResponse {
    pub telegram_connected: bool,
    pub telegram_username: Option<String>,
    pub telegram_enabled: bool,
    pub approval_required: bool,
    pub approval_timeout_secs: u32,
    pub grant_expiry_days: u32,
    pub push_enabled: bool,
    pub push_device_count: usize,
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

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "notification_settings_updated".to_string(),
        Some(serde_json::json!({
            "telegram_enabled": updated.telegram_enabled,
            "approval_required": updated.approval_required,
            "approval_timeout_secs": updated.approval_timeout_secs,
            "grant_expiry_days": updated.grant_expiry_days,
            "push_enabled": updated.push_enabled,
        })),
        None,
        None,
    );

    Ok(Json(to_settings_response(&updated)))
}

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
) -> AppResult<Json<MessageResponse>> {
    let user_id = auth_user.user_id.to_string();
    let channel = notification_service::get_or_create_channel(&state.db, &user_id).await?;

    let now = bson::DateTime::from_chrono(Utc::now());
    state
        .db
        .collection::<NotificationChannel>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": &channel.id },
            doc! {
                "$set": {
                    "telegram_chat_id": bson::Bson::Null,
                    "telegram_username": bson::Bson::Null,
                    "telegram_enabled": false,
                    "telegram_link_code": bson::Bson::Null,
                    "telegram_link_code_expires_at": bson::Bson::Null,
                    "updated_at": now,
                }
            },
        )
        .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id),
        "telegram_disconnected".to_string(),
        None,
        None,
        None,
    );

    Ok(Json(MessageResponse {
        message: "Telegram disconnected".to_string(),
    }))
}

fn to_settings_response(channel: &NotificationChannel) -> NotificationSettingsResponse {
    NotificationSettingsResponse {
        telegram_connected: channel.telegram_chat_id.is_some(),
        telegram_username: channel.telegram_username.clone(),
        telegram_enabled: channel.telegram_enabled,
        approval_required: channel.approval_required,
        approval_timeout_secs: channel.approval_timeout_secs,
        grant_expiry_days: channel.grant_expiry_days,
        push_enabled: channel.push_enabled,
        push_device_count: channel.push_devices.len(),
    }
}
