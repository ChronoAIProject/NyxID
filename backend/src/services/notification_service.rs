use chrono::Utc;
use mongodb::bson::doc;
use mongodb::Database;
use reqwest::Client;

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::approval_request::ApprovalRequest;
use crate::models::notification_channel::{NotificationChannel, COLLECTION_NAME};
use crate::services::telegram_service;

/// Send an approval notification to the user via their configured channel.
/// Returns the channel name used (e.g. "telegram").
pub async fn send_approval_notification(
    db: &Database,
    config: &AppConfig,
    http_client: &Client,
    user_id: &str,
    request: &ApprovalRequest,
) -> AppResult<(String, Option<i64>, Option<i64>)> {
    let channel = get_or_create_channel(db, user_id).await?;

    // Telegram is currently the only supported channel
    if channel.telegram_enabled {
        let chat_id = channel.telegram_chat_id.ok_or_else(|| {
            AppError::BadRequest(
                "Telegram is enabled but no chat ID is linked. Please link your Telegram account."
                    .to_string(),
            )
        })?;

        let bot_token = config.telegram_bot_token.as_deref().ok_or_else(|| {
            AppError::Internal("Telegram bot token not configured".to_string())
        })?;

        let requester_label = request
            .requester_label
            .as_deref()
            .unwrap_or(&request.requester_type);

        let message_id = telegram_service::send_approval_message(
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
        .await?;

        return Ok(("telegram".to_string(), Some(chat_id), Some(message_id)));
    }

    Err(AppError::BadRequest(
        "No notification channel is configured and enabled".to_string(),
    ))
}

/// Edit the notification message after a decision is made.
pub async fn notify_decision(
    config: &AppConfig,
    http_client: &Client,
    request: &ApprovalRequest,
    approved: bool,
) -> AppResult<()> {
    if request.notification_channel.as_deref() == Some("telegram") {
        if let (Some(chat_id), Some(message_id)) =
            (request.telegram_chat_id, request.telegram_message_id)
        {
            let bot_token = config.telegram_bot_token.as_deref().ok_or_else(|| {
                AppError::Internal("Telegram bot token not configured".to_string())
            })?;

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
    }

    Ok(())
}

/// Get the user's notification channel settings, creating defaults if none exist.
pub async fn get_or_create_channel(
    db: &Database,
    user_id: &str,
) -> AppResult<NotificationChannel> {
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

fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    if let mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we)) =
        e.kind.as_ref()
    {
        return we.code == 11000;
    }
    false
}
