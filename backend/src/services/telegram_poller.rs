use mongodb::bson::{self, doc};

use crate::AppState;
use crate::models::notification_channel::{COLLECTION_NAME as CHANNELS, NotificationChannel};
use crate::services::{approval_service, audit_service, telegram_service};

/// Run the Telegram long polling loop (development mode fallback).
///
/// When TELEGRAM_WEBHOOK_URL is not configured but TELEGRAM_BOT_TOKEN is set,
/// this polls Telegram's getUpdates API to receive callback queries and link
/// messages without requiring a publicly accessible webhook endpoint.
pub async fn run_polling_loop(state: AppState) {
    let bot_token = match state.config.telegram_bot_token.as_deref() {
        Some(t) => t.to_string(),
        None => return,
    };

    // Delete any existing webhook so getUpdates works
    match telegram_service::delete_webhook(&state.http_client, &bot_token).await {
        Ok(()) => tracing::info!("Telegram webhook cleared for polling mode"),
        Err(e) => {
            tracing::error!("Failed to clear Telegram webhook: {e}");
            return;
        }
    }

    tracing::info!("Telegram polling mode active (development fallback)");

    let mut offset: Option<i64> = None;

    loop {
        match telegram_service::get_updates(&state.http_client, &bot_token, offset, 30).await {
            Ok(updates) => {
                for update in updates {
                    offset = Some(update.update_id + 1);
                    process_update(&state, update).await;
                }
            }
            Err(e) => {
                tracing::warn!("Telegram getUpdates error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

/// Process a single Telegram update (callback query or message).
///
/// Shared by both the webhook handler and the polling loop.
pub async fn process_update(state: &AppState, update: telegram_service::TelegramUpdate) {
    if let Some(callback) = update.callback_query {
        handle_callback_query(state, callback).await;
    } else if let Some(message) = update.message {
        handle_link_message(state, message).await;
    }
}

/// Handle a Telegram callback query (user pressed Approve/Reject).
async fn handle_callback_query(
    state: &AppState,
    callback: telegram_service::TelegramCallbackQuery,
) {
    let data = match callback.data.as_deref() {
        Some(d) => d,
        None => return,
    };

    let (approved, request_id) = match telegram_service::parse_callback_data(data) {
        Some(result) => result,
        None => {
            tracing::warn!("Invalid callback data: {data}");
            return;
        }
    };

    // Verify the chat_id matches the request's telegram_chat_id
    let chat_id = callback.message.as_ref().map(|m| m.chat.id).unwrap_or(0);

    let request = match approval_service::get_request(&state.db, &request_id).await {
        Ok(r) => r,
        Err(_) => {
            answer_callback(state, &callback.id, "Request not found or expired").await;
            return;
        }
    };

    // Verify chat_id binding
    if request.telegram_chat_id != Some(chat_id) {
        tracing::warn!(
            "Chat ID mismatch: expected {:?}, got {}",
            request.telegram_chat_id,
            chat_id
        );
        answer_callback(state, &callback.id, "Unauthorized").await;
        return;
    }

    // Process the decision
    match approval_service::process_decision(
        &state.db,
        &state.config,
        &state.http_client,
        state.fcm_auth.clone(),
        state.apns_auth.clone(),
        &request_id,
        approved,
        None,
        None,
        "telegram",
    )
    .await
    {
        Ok(updated) => {
            let text = if approved {
                format!("Approved access to {}", updated.service_name)
            } else {
                format!("Rejected access to {}", updated.service_name)
            };
            answer_callback(state, &callback.id, &text).await;

            audit_service::log_async(
                state.db.clone(),
                Some(updated.user_id.clone()),
                "approval_decision".to_string(),
                Some(serde_json::json!({
                    "request_id": request_id,
                    "service_id": updated.service_id,
                    "approved": approved,
                    "channel": "telegram",
                })),
                None,
                None,
            );
        }
        Err(e) => {
            tracing::warn!("Failed to process approval decision: {e}");
            answer_callback(state, &callback.id, "Already processed or expired").await;
        }
    }
}

/// Handle a Telegram /start link message.
async fn handle_link_message(state: &AppState, message: telegram_service::TelegramMessage) {
    let text = match message.text.as_deref() {
        Some(t) => t,
        None => return,
    };

    // Parse /start NYXID-XXXXXX
    let link_code = if text.starts_with("/start ") {
        text.trim_start_matches("/start ").trim()
    } else {
        return;
    };

    if !link_code.starts_with("NYXID-") {
        return;
    }

    let chat_id = message.chat.id;
    let username = message.from.as_ref().and_then(|u| u.username.clone());

    let bot_token = match state.config.telegram_bot_token.as_deref() {
        Some(t) => t,
        None => return,
    };

    // Find the notification channel with this link code
    let collection = state.db.collection::<NotificationChannel>(CHANNELS);

    let channel = match collection
        .find_one(doc! { "telegram_link_code": link_code })
        .await
    {
        Ok(Some(ch)) => ch,
        _ => {
            let _ = telegram_service::send_text_message(
                &state.http_client,
                bot_token,
                chat_id,
                "Invalid or expired link code. Please generate a new one from NyxID settings.",
            )
            .await;
            return;
        }
    };

    // Check if the link code has expired
    if let Some(expires_at) = channel.telegram_link_code_expires_at {
        if expires_at < chrono::Utc::now() {
            let _ = telegram_service::send_text_message(
                &state.http_client,
                bot_token,
                chat_id,
                "This link code has expired. Please generate a new one from NyxID settings.",
            )
            .await;
            return;
        }
    }

    // Update the channel with the Telegram details
    let now = bson::DateTime::from_chrono(chrono::Utc::now());
    let update = doc! {
        "$set": {
            "telegram_chat_id": chat_id,
            "telegram_username": &username,
            "telegram_enabled": true,
            "telegram_link_code": bson::Bson::Null,
            "telegram_link_code_expires_at": bson::Bson::Null,
            "updated_at": now,
        }
    };

    match collection
        .update_one(doc! { "_id": &channel.id }, update)
        .await
    {
        Ok(_) => {
            let _ = telegram_service::send_text_message(
                &state.http_client,
                bot_token,
                chat_id,
                "Your Telegram account has been linked to NyxID. You will now receive approval notifications here.",
            )
            .await;

            audit_service::log_async(
                state.db.clone(),
                Some(channel.user_id.clone()),
                "telegram_linked".to_string(),
                Some(serde_json::json!({
                    "telegram_username": username,
                    "telegram_chat_id": chat_id,
                })),
                None,
                None,
            );
        }
        Err(e) => {
            tracing::error!("Failed to update notification channel: {e}");
            let _ = telegram_service::send_text_message(
                &state.http_client,
                bot_token,
                chat_id,
                "Failed to link your account. Please try again.",
            )
            .await;
        }
    }
}

async fn answer_callback(state: &AppState, callback_id: &str, text: &str) {
    if let Some(bot_token) = state.config.telegram_bot_token.as_deref() {
        let _ = telegram_service::answer_callback_query(
            &state.http_client,
            bot_token,
            callback_id,
            text,
        )
        .await;
    }
}
