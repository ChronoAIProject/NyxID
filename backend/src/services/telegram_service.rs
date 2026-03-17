use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::errors::{AppError, AppResult};

const TELEGRAM_API_BASE: &str = "https://api.telegram.org/bot";

// --- Public Telegram API types (shared by webhook handler and poller) ---

#[derive(Debug, Deserialize)]
pub struct TelegramUpdate {
    pub update_id: i64,
    #[serde(default)]
    pub callback_query: Option<TelegramCallbackQuery>,
    #[serde(default)]
    pub message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramCallbackQuery {
    pub id: String,
    #[allow(dead_code)]
    pub from: TelegramUser,
    pub message: Option<TelegramMessageRef>,
    pub data: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramMessageRef {
    #[allow(dead_code)]
    pub message_id: i64,
    pub chat: TelegramChat,
}

#[derive(Debug, Deserialize)]
pub struct TelegramMessage {
    pub chat: TelegramChat,
    pub from: Option<TelegramUser>,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramChat {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct TelegramUser {
    #[allow(dead_code)]
    pub id: i64,
    pub username: Option<String>,
    #[allow(dead_code)]
    pub first_name: Option<String>,
}

// --- Private request/response types ---

/// Escape user-controlled values for Telegram HTML parse mode.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[derive(Debug, Serialize)]
struct SendMessageRequest {
    chat_id: i64,
    text: String,
    parse_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_markup: Option<InlineKeyboardMarkup>,
}

#[derive(Debug, Serialize)]
struct InlineKeyboardMarkup {
    inline_keyboard: Vec<Vec<InlineKeyboardButton>>,
}

#[derive(Debug, Serialize)]
struct InlineKeyboardButton {
    text: String,
    callback_data: String,
}

#[derive(Debug, Serialize)]
struct EditMessageTextRequest {
    chat_id: i64,
    message_id: i64,
    text: String,
    parse_mode: String,
}

#[derive(Debug, Serialize)]
struct AnswerCallbackQueryRequest {
    callback_query_id: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct SetWebhookRequest {
    url: String,
    secret_token: String,
    allowed_updates: Vec<String>,
}

#[derive(Debug, Serialize)]
struct GetUpdatesRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<i64>,
    timeout: u32,
    allowed_updates: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramResponse {
    ok: bool,
    #[serde(default)]
    description: Option<String>,
    result: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GetUpdatesResponse {
    ok: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    result: Vec<TelegramUpdate>,
}

/// Send an approval request message with Approve/Reject inline keyboard.
/// Returns the Telegram message_id.
#[allow(clippy::too_many_arguments)]
pub async fn send_approval_message(
    http_client: &Client,
    bot_token: &str,
    chat_id: i64,
    request_id: &str,
    service_name: &str,
    service_slug: &str,
    requester_label: &str,
    operation_summary: &str,
    expires_in_secs: u32,
) -> AppResult<i64> {
    let svc_name = html_escape(service_name);
    let svc_slug = html_escape(service_slug);
    let req_label = html_escape(requester_label);
    let op_summary = html_escape(operation_summary);
    let text = format!(
        "<b>Access Request</b>\n\n\
         <b>Service:</b> {svc_name} (<code>{svc_slug}</code>)\n\
         <b>Requester:</b> {req_label}\n\
         <b>Action:</b> <code>{op_summary}</code>\n\
         <b>Expires:</b> {expires_in_secs}s"
    );

    // Use UUID without hyphens for callback data (32 chars + 2 prefix = 34 chars)
    let id_compact = request_id.replace('-', "");

    let body = SendMessageRequest {
        chat_id,
        text,
        parse_mode: "HTML".to_string(),
        reply_markup: Some(InlineKeyboardMarkup {
            inline_keyboard: vec![vec![
                InlineKeyboardButton {
                    text: "Approve".to_string(),
                    callback_data: format!("a:{id_compact}"),
                },
                InlineKeyboardButton {
                    text: "Reject".to_string(),
                    callback_data: format!("r:{id_compact}"),
                },
            ]],
        }),
    };

    let url = format!("{TELEGRAM_API_BASE}{bot_token}/sendMessage");
    let resp: TelegramResponse = http_client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram API request failed: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram API response parse failed: {e}")))?;

    if !resp.ok {
        return Err(AppError::Internal(format!(
            "Telegram sendMessage failed: {}",
            resp.description.unwrap_or_default()
        )));
    }

    let message_id = resp
        .result
        .and_then(|r| r.get("message_id").and_then(|v| v.as_i64()))
        .ok_or_else(|| AppError::Internal("Telegram response missing message_id".to_string()))?;

    Ok(message_id)
}

/// Edit a message to show the decision result.
pub async fn edit_message_after_decision(
    http_client: &Client,
    bot_token: &str,
    chat_id: i64,
    message_id: i64,
    approved: bool,
    service_name: &str,
) -> AppResult<()> {
    let decision = if approved { "Approved" } else { "Rejected" };
    let emoji = if approved { "+" } else { "-" };
    let svc_name = html_escape(service_name);
    let text = format!(
        "<b>Access Request [{decision}]</b>\n\n\
         <b>Service:</b> {svc_name}\n\
         <b>Decision:</b> {emoji} {decision}"
    );

    let body = EditMessageTextRequest {
        chat_id,
        message_id,
        text,
        parse_mode: "HTML".to_string(),
    };

    let url = format!("{TELEGRAM_API_BASE}{bot_token}/editMessageText");
    let resp: TelegramResponse = http_client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram editMessageText failed: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram response parse failed: {e}")))?;

    if !resp.ok {
        tracing::warn!(
            "Telegram editMessageText failed: {}",
            resp.description.unwrap_or_default()
        );
    }

    Ok(())
}

/// Answer a Telegram callback query (removes loading spinner).
pub async fn answer_callback_query(
    http_client: &Client,
    bot_token: &str,
    callback_query_id: &str,
    text: &str,
) -> AppResult<()> {
    let body = AnswerCallbackQueryRequest {
        callback_query_id: callback_query_id.to_string(),
        text: text.to_string(),
    };

    let url = format!("{TELEGRAM_API_BASE}{bot_token}/answerCallbackQuery");
    let _ = http_client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!("Telegram answerCallbackQuery failed: {e}");
            e
        });

    Ok(())
}

/// Send a simple text message (used for link confirmation, expiry notices).
pub async fn send_text_message(
    http_client: &Client,
    bot_token: &str,
    chat_id: i64,
    text: &str,
) -> AppResult<()> {
    let body = SendMessageRequest {
        chat_id,
        text: text.to_string(),
        parse_mode: "HTML".to_string(),
        reply_markup: None,
    };

    let url = format!("{TELEGRAM_API_BASE}{bot_token}/sendMessage");
    let resp: TelegramResponse = http_client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram sendMessage failed: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram response parse failed: {e}")))?;

    if !resp.ok {
        tracing::warn!(
            "Telegram sendMessage failed: {}",
            resp.description.unwrap_or_default()
        );
    }

    Ok(())
}

/// Register the webhook URL with Telegram.
pub async fn set_webhook(
    http_client: &Client,
    bot_token: &str,
    webhook_url: &str,
    secret_token: &str,
) -> AppResult<()> {
    let body = SetWebhookRequest {
        url: webhook_url.to_string(),
        secret_token: secret_token.to_string(),
        allowed_updates: vec!["callback_query".to_string(), "message".to_string()],
    };

    let url = format!("{TELEGRAM_API_BASE}{bot_token}/setWebhook");
    let resp: TelegramResponse = http_client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram setWebhook failed: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram response parse failed: {e}")))?;

    if !resp.ok {
        return Err(AppError::Internal(format!(
            "Telegram setWebhook failed: {}",
            resp.description.unwrap_or_default()
        )));
    }

    Ok(())
}

/// Remove any registered webhook (required before using getUpdates).
pub async fn delete_webhook(http_client: &Client, bot_token: &str) -> AppResult<()> {
    let url = format!("{TELEGRAM_API_BASE}{bot_token}/deleteWebhook");
    let resp: TelegramResponse = http_client
        .post(&url)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram deleteWebhook failed: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram response parse failed: {e}")))?;

    if !resp.ok {
        return Err(AppError::Internal(format!(
            "Telegram deleteWebhook failed: {}",
            resp.description.unwrap_or_default()
        )));
    }

    Ok(())
}

/// Fetch pending updates via long polling.
/// Returns the list of updates. The caller must track the offset.
pub async fn get_updates(
    http_client: &Client,
    bot_token: &str,
    offset: Option<i64>,
    timeout_secs: u32,
) -> AppResult<Vec<TelegramUpdate>> {
    let body = GetUpdatesRequest {
        offset,
        timeout: timeout_secs,
        allowed_updates: vec!["callback_query".to_string(), "message".to_string()],
    };

    let url = format!("{TELEGRAM_API_BASE}{bot_token}/getUpdates");
    let resp: GetUpdatesResponse = http_client
        .post(&url)
        .json(&body)
        // Override client timeout: long poll + buffer for network latency
        .timeout(std::time::Duration::from_secs(timeout_secs as u64 + 10))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram getUpdates failed: {e}")))?
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Telegram getUpdates parse failed: {e}")))?;

    if !resp.ok {
        return Err(AppError::Internal(format!(
            "Telegram getUpdates failed: {}",
            resp.description.unwrap_or_default()
        )));
    }

    Ok(resp.result)
}

/// Parse callback data from Telegram callback query.
/// Format: "a:<uuid_no_hyphens>" or "r:<uuid_no_hyphens>"
/// Returns (approved: bool, request_id: String with hyphens reinserted).
pub fn parse_callback_data(data: &str) -> Option<(bool, String)> {
    let (prefix, id_compact) = data.split_once(':')?;
    let approved = match prefix {
        "a" => true,
        "r" => false,
        _ => return None,
    };

    // Re-insert hyphens into UUID: 8-4-4-4-12
    if id_compact.len() != 32 {
        return None;
    }

    let request_id = format!(
        "{}-{}-{}-{}-{}",
        &id_compact[..8],
        &id_compact[8..12],
        &id_compact[12..16],
        &id_compact[16..20],
        &id_compact[20..],
    );

    Some((approved, request_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_approve_callback() {
        let data = "a:550e8400e29b41d4a716446655440000";
        let (approved, id) = parse_callback_data(data).unwrap();
        assert!(approved);
        assert_eq!(id, "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn parse_reject_callback() {
        let data = "r:550e8400e29b41d4a716446655440000";
        let (approved, id) = parse_callback_data(data).unwrap();
        assert!(!approved);
        assert_eq!(id, "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn parse_invalid_prefix() {
        assert!(parse_callback_data("x:550e8400e29b41d4a716446655440000").is_none());
    }

    #[test]
    fn parse_invalid_length() {
        assert!(parse_callback_data("a:tooshort").is_none());
    }

    #[test]
    fn parse_no_separator() {
        assert!(parse_callback_data("noseparator").is_none());
    }
}
