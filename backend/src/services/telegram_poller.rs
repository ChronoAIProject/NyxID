use std::time::Duration;

use mongodb::bson::{self, Bson, doc};

use crate::AppState;
use crate::errors::AppError;
use crate::models::coordination::CoordinationHolder;
use crate::models::notification_channel::{COLLECTION_NAME as CHANNELS, NotificationChannel};
use crate::services::coordination_service::{ClusterLeaseRuntime, LeaseStore};
use crate::services::{approval_service, audit_service, notification_service, telegram_service};

const TELEGRAM_LINKED_APPROVAL_ACTIVE_MESSAGE: &str = "Your Telegram account has been linked to NyxID. Approval protection is now active and approval requests will be delivered here.";
const TELEGRAM_LINKED_APPROVAL_OFF_MESSAGE: &str = "Your Telegram account has been linked to NyxID. Approval protection is off — enable it in NyxID notification settings when you want requests delivered here.";
const TELEGRAM_POLLER_LEASE_NAME: &str = "telegram-poller";

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

    let runtime = ClusterLeaseRuntime::new(
        CoordinationHolder {
            instance_id: state.replica_identity.instance_name.clone(),
            generation_id: state.replica_identity.generation_id.clone(),
        },
        Duration::from_secs(state.config.cluster_lease_ttl_secs),
        Duration::from_secs(state.config.cluster_lease_renew_secs),
    );

    loop {
        match runtime.acquire(&state.db, TELEGRAM_POLLER_LEASE_NAME).await {
            Ok(Some(lease)) => {
                let result = runtime
                    .run_while_renewed(
                        &state.db,
                        &lease,
                        run_as_polling_leader(&state, &bot_token, &lease, &runtime),
                    )
                    .await;
                if let Err(error) = LeaseStore::release(&state.db, &lease).await {
                    tracing::warn!(error = %error, "Failed to release Telegram polling lease");
                }
                match result {
                    None => {
                        tracing::warn!("Telegram polling leadership lost; cancelling long poll")
                    }
                    Some(Ok(())) => tracing::warn!("Telegram polling leadership ended"),
                    Some(Err(error)) => {
                        tracing::warn!(error = %error, "Telegram polling leader failed")
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(error = %error, "Failed to acquire Telegram polling lease");
            }
        }
        tokio::time::sleep(runtime.contender_wait()).await;
    }
}

async fn run_as_polling_leader(
    state: &AppState,
    bot_token: &str,
    lease: &crate::services::coordination_service::LeaseToken,
    runtime: &ClusterLeaseRuntime,
) -> Result<(), AppError> {
    telegram_service::delete_webhook(&state.http_client, bot_token).await?;
    tracing::info!("Telegram polling mode active on elected replica");

    let mut offset = match LeaseStore::load_checkpoint(&state.db, lease).await? {
        Some(Bson::Int64(value)) => Some(value),
        Some(Bson::Int32(value)) => Some(i64::from(value)),
        Some(_) => {
            tracing::warn!("Ignoring invalid Telegram polling checkpoint");
            None
        }
        None => None,
    };
    let poll_timeout_secs = u32::try_from(runtime.ttl.as_secs()).unwrap_or(u32::MAX);

    loop {
        match telegram_service::get_updates(
            &state.http_client,
            bot_token,
            offset,
            poll_timeout_secs,
        )
        .await
        {
            Ok(updates) => {
                for update in updates {
                    let next_offset = update.update_id.saturating_add(1);
                    process_update(state, update).await;
                    if !LeaseStore::store_checkpoint(&state.db, lease, Bson::Int64(next_offset))
                        .await?
                    {
                        return Ok(());
                    }
                    offset = Some(next_offset);
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "Telegram getUpdates error");
                tokio::time::sleep(runtime.contender_wait()).await;
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

    let request = match approval_service::get_request(&state.db, &request_id).await {
        Ok(r) => r,
        Err(crate::errors::AppError::NotFound(_)) => {
            answer_callback(state, &callback.id, "Request not found or expired").await;
            return;
        }
        Err(e) => {
            tracing::error!("Database error fetching approval request {request_id}: {e}");
            answer_callback(state, &callback.id, "Server error, please try again").await;
            return;
        }
    };

    // Verify the chat_id matches the request's telegram_chat_id.
    // Telegram may omit `callback.message` for old messages, so fall back
    // to `callback.from.id` which is always present and represents the
    // user's private chat ID for bot conversations.
    let chat_id = callback
        .message
        .as_ref()
        .map(|m| m.chat.id)
        .unwrap_or(callback.from.id);

    if request.telegram_chat_id != Some(chat_id) {
        tracing::warn!(
            "Chat ID mismatch: expected {:?}, got {}",
            request.telegram_chat_id,
            chat_id
        );
        answer_callback(state, &callback.id, "Unauthorized").await;
        return;
    }

    // Build an idempotency key from the callback so Telegram retries are
    // handled correctly instead of being rejected as "already_decided".
    let decision_idempotency_key = format!("tg:{}:{}", callback.id, request_id);

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
        Some(decision_idempotency_key.as_str()),
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
                None,
                None,
            );
        }
        Err(e) => {
            let callback_message = decision_callback_message(&e);
            if callback_message == "Server error, please try again" {
                tracing::error!("Failed to process approval decision {request_id}: {e}");
            } else {
                tracing::warn!("Failed to process approval decision {request_id}: {e}");
            }
            answer_callback(state, &callback.id, callback_message).await;
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
    if let Some(expires_at) = channel.telegram_link_code_expires_at
        && expires_at < chrono::Utc::now()
    {
        let _ = telegram_service::send_text_message(
            &state.http_client,
            bot_token,
            chat_id,
            "This link code has expired. Please generate a new one from NyxID settings.",
        )
        .await;
        return;
    }

    // Link Telegram without changing the user's global approval preference.
    // Approval protection is enabled only through the explicit notification
    // settings endpoint; linking a delivery channel must not opt the user in.
    let now = bson::DateTime::from_chrono(chrono::Utc::now());
    let update = doc! {
        "$set": telegram_link_update_fields(chat_id, &username, now)
    };
    let link_outcome = telegram_link_outcome(&channel);

    match collection
        .update_one(doc! { "_id": &channel.id }, update)
        .await
    {
        Ok(_) => {
            let _ = telegram_service::send_text_message(
                &state.http_client,
                bot_token,
                chat_id,
                link_outcome.message,
            )
            .await;

            audit_service::log_async(
                state.db.clone(),
                Some(channel.user_id.clone()),
                "telegram_linked".to_string(),
                Some(telegram_link_audit_metadata(
                    username.as_deref(),
                    chat_id,
                    link_outcome,
                )),
                None,
                None,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TelegramLinkOutcome {
    message: &'static str,
    approval_resumed: bool,
}

fn telegram_link_outcome(channel: &NotificationChannel) -> TelegramLinkOutcome {
    TelegramLinkOutcome {
        message: if channel.approval_required {
            TELEGRAM_LINKED_APPROVAL_ACTIVE_MESSAGE
        } else {
            TELEGRAM_LINKED_APPROVAL_OFF_MESSAGE
        },
        approval_resumed: channel.approval_required
            && !notification_service::has_active_notification_channel(channel),
    }
}

fn telegram_link_audit_metadata(
    username: Option<&str>,
    chat_id: i64,
    outcome: TelegramLinkOutcome,
) -> serde_json::Value {
    serde_json::json!({
        "telegram_username": username,
        "telegram_chat_id": chat_id,
        "approval_resumed": outcome.approval_resumed,
    })
}

fn telegram_link_update_fields(
    chat_id: i64,
    username: &Option<String>,
    now: bson::DateTime,
) -> bson::Document {
    doc! {
        "telegram_chat_id": chat_id,
        "telegram_username": username,
        "telegram_enabled": true,
        "telegram_link_code": bson::Bson::Null,
        "telegram_link_code_expires_at": bson::Bson::Null,
        "updated_at": now,
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

fn decision_callback_message(error: &AppError) -> &'static str {
    match error {
        AppError::Forbidden(message) if message == "Approval request expired" => "Request expired",
        AppError::Conflict(_) => "Already processed or expired",
        AppError::NotFound(_) => "Request not found or expired",
        _ => "Server error, please try again",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification_channel(approval_required: bool) -> NotificationChannel {
        NotificationChannel {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            telegram_chat_id: None,
            telegram_username: None,
            telegram_enabled: false,
            telegram_link_code: Some("NYXID-TEST".to_string()),
            telegram_link_code_expires_at: None,
            approval_timeout_secs: 30,
            grant_expiry_days: 30,
            approval_required,
            push_enabled: false,
            push_devices: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn decision_callback_message_maps_expired_forbidden() {
        let error = AppError::Forbidden("Approval request expired".to_string());
        assert_eq!(decision_callback_message(&error), "Request expired");
    }

    #[test]
    fn decision_callback_message_maps_conflict_to_processed() {
        let error = AppError::Conflict("already_decided".to_string());
        assert_eq!(
            decision_callback_message(&error),
            "Already processed or expired"
        );
    }

    #[test]
    fn decision_callback_message_maps_not_found() {
        let error = AppError::NotFound("Approval request not found".to_string());
        assert_eq!(
            decision_callback_message(&error),
            "Request not found or expired"
        );
    }

    #[test]
    fn decision_callback_message_maps_internal_to_server_error() {
        let error = AppError::Internal("database timeout".to_string());
        assert_eq!(
            decision_callback_message(&error),
            "Server error, please try again"
        );
    }

    #[test]
    fn telegram_link_update_preserves_approval_preference() {
        let update =
            telegram_link_update_fields(1234, &Some("nyx".to_string()), bson::DateTime::now());

        assert!(!update.contains_key("approval_required"));
        for approval_required in [false, true] {
            let mut persisted = doc! { "approval_required": approval_required };
            persisted.extend(update.clone());
            assert_eq!(
                persisted.get_bool("approval_required").unwrap(),
                approval_required
            );
        }
    }

    #[test]
    fn telegram_link_reports_approval_resumption() {
        let outcome = telegram_link_outcome(&notification_channel(true));
        let metadata = telegram_link_audit_metadata(Some("nyx"), 1234, outcome);

        assert_eq!(outcome.message, TELEGRAM_LINKED_APPROVAL_ACTIVE_MESSAGE);
        assert!(outcome.approval_resumed);
        assert_eq!(metadata["approval_resumed"], true);
    }

    #[test]
    fn telegram_link_reports_approval_off_without_resumption() {
        let outcome = telegram_link_outcome(&notification_channel(false));
        let metadata = telegram_link_audit_metadata(None, 1234, outcome);

        assert_eq!(outcome.message, TELEGRAM_LINKED_APPROVAL_OFF_MESSAGE);
        assert!(!outcome.approval_resumed);
        assert_eq!(metadata["approval_resumed"], false);
    }

    #[test]
    fn telegram_link_does_not_report_resumption_when_another_channel_is_active() {
        let mut channel = notification_channel(true);
        channel.push_enabled = true;
        channel
            .push_devices
            .push(crate::models::notification_channel::DeviceToken {
                device_id: "existing-device".to_string(),
                platform: "fcm".to_string(),
                token: "existing-token".to_string(),
                device_name: None,
                app_id: None,
                registered_at: chrono::Utc::now(),
                last_used_at: None,
            });

        let outcome = telegram_link_outcome(&channel);

        assert_eq!(outcome.message, TELEGRAM_LINKED_APPROVAL_ACTIVE_MESSAGE);
        assert!(!outcome.approval_resumed);
    }
}
