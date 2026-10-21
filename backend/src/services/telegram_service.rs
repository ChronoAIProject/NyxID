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

#[derive(Serialize)]
struct SendMessageRequest {
    chat_id: i64,
    text: String,
    parse_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_markup: Option<InlineKeyboardMarkup>,
}

#[derive(Serialize)]
struct InlineKeyboardMarkup {
    inline_keyboard: Vec<Vec<InlineKeyboardButton>>,
}

#[derive(Serialize)]
struct InlineKeyboardButton {
    text: String,
    callback_data: String,
}

#[derive(Serialize)]
struct EditMessageTextRequest {
    chat_id: i64,
    message_id: i64,
    text: String,
    parse_mode: String,
}

#[derive(Serialize)]
struct AnswerCallbackQueryRequest {
    callback_query_id: String,
    text: String,
}

#[derive(Serialize)]
struct SetWebhookRequest {
    url: String,
    secret_token: String,
    allowed_updates: Vec<String>,
}

#[derive(Serialize)]
struct GetUpdatesRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<i64>,
    timeout: u32,
    allowed_updates: Vec<String>,
}

#[derive(Deserialize)]
struct TelegramResponse {
    ok: bool,
    #[serde(default)]
    error_code: Option<u16>,
    result: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct GetUpdatesResponse {
    ok: bool,
    #[serde(default)]
    error_code: Option<u16>,
    #[serde(default)]
    result: Vec<TelegramUpdate>,
}

/// Build the HTML body of an approval notification. Extracted from
/// `send_approval_message` so the template logic can be unit-tested
/// without a Telegram client.
fn format_approval_message(
    service_name: &str,
    service_slug: &str,
    requester_label: &str,
    operation_summary: &str,
    expires_in_secs: u32,
    org_name: Option<&str>,
) -> String {
    let svc_name = html_escape(service_name);
    let svc_slug = html_escape(service_slug);
    let req_label = html_escape(requester_label);
    let op_summary = html_escape(operation_summary);

    match org_name {
        Some(org) => {
            let org_escaped = html_escape(org);
            format!(
                "<b>Org Access Request — {org_escaped}</b>\n\n\
                 <b>Org:</b> {org_escaped}\n\
                 <b>Service:</b> {svc_name} (<code>{svc_slug}</code>)\n\
                 <b>Requester:</b> {req_label}\n\
                 <b>Action:</b> <code>{op_summary}</code>\n\
                 <b>Expires:</b> {expires_in_secs}s"
            )
        }
        None => format!(
            "<b>Access Request</b>\n\n\
             <b>Service:</b> {svc_name} (<code>{svc_slug}</code>)\n\
             <b>Requester:</b> {req_label}\n\
             <b>Action:</b> <code>{op_summary}</code>\n\
             <b>Expires:</b> {expires_in_secs}s"
        ),
    }
}

/// Send an approval request message with Approve/Reject inline keyboard.
/// Returns the Telegram message_id.
///
/// `org_name` is `Some(name)` when the request was created under an org's
/// approval policy. In that case the header switches to `Org Access
/// Request — {org_name}` and an extra `<b>Org:</b> {org_name}` line is
/// prepended so admins can tell at a glance that they are deciding on
/// behalf of the org rather than for themselves. Passing `None` keeps
/// the message byte-identical to the pre-org wording so non-org
/// requests are unaffected.
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
    org_name: Option<&str>,
) -> AppResult<i64> {
    let text = format_approval_message(
        service_name,
        service_slug,
        requester_label,
        operation_summary,
        expires_in_secs,
        org_name,
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
        .map_err(|_| AppError::Internal("Telegram API request failed".into()))?
        .json()
        .await
        .map_err(|_| AppError::Internal("Telegram API response parse failed".into()))?;

    if !resp.ok {
        return Err(AppError::Internal(format!(
            "Telegram sendMessage failed (code {:?})",
            resp.error_code
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
        .map_err(|_| AppError::Internal("Telegram editMessageText failed".into()))?
        .json()
        .await
        .map_err(|_| AppError::Internal("Telegram response parse failed".into()))?;

    if !resp.ok {
        tracing::warn!(error_code = ?resp.error_code, "Telegram editMessageText failed");
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
        .map_err(|_| {
            tracing::warn!("Telegram answerCallbackQuery transport failed");
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
    send_text_message_request(http_client, &url, &body).await
}

async fn send_text_message_request(
    http_client: &Client,
    url: &str,
    body: &SendMessageRequest,
) -> AppResult<()> {
    let resp: TelegramResponse = http_client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|_| AppError::Internal("Telegram sendMessage failed".into()))?
        .json()
        .await
        .map_err(|_| AppError::Internal("Telegram response parse failed".into()))?;

    if !resp.ok {
        tracing::warn!(error_code = ?resp.error_code, "Telegram sendMessage failed");
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
        .map_err(|_| AppError::Internal("Telegram setWebhook failed".into()))?
        .json()
        .await
        .map_err(|_| AppError::Internal("Telegram response parse failed".into()))?;

    if !resp.ok {
        return Err(AppError::Internal(format!(
            "Telegram setWebhook failed (code {:?})",
            resp.error_code
        )));
    }

    Ok(())
}

/// Check whether the webhook is healthy (registered, no pending errors).
/// Returns `true` if the webhook URL matches and has no undelivered errors.
pub async fn is_webhook_healthy(http_client: &Client, bot_token: &str, expected_url: &str) -> bool {
    let url = format!("{TELEGRAM_API_BASE}{bot_token}/getWebhookInfo");
    let resp = match http_client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => {
            tracing::warn!("Telegram getWebhookInfo transport failed");
            return false;
        }
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => {
            tracing::warn!("Invalid Telegram getWebhookInfo response");
            return false;
        }
    };

    let result = match body.get("result") {
        Some(r) => r,
        None => return false,
    };

    let registered_url = result.get("url").and_then(|v| v.as_str()).unwrap_or("");

    if registered_url != expected_url {
        tracing::warn!("Telegram webhook URL mismatch");
        return false;
    }

    // Check for accumulated errors that indicate delivery failures
    let pending_count = result
        .get("pending_update_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    if pending_count > 50 {
        tracing::warn!(pending_count, "Telegram webhook delivery backlog");
        return false;
    }

    true
}

/// Remove any registered webhook (required before using getUpdates).
pub async fn delete_webhook(http_client: &Client, bot_token: &str) -> AppResult<()> {
    let url = format!("{TELEGRAM_API_BASE}{bot_token}/deleteWebhook");
    let resp: TelegramResponse = http_client
        .post(&url)
        .send()
        .await
        .map_err(|_| AppError::Internal("Telegram deleteWebhook failed".into()))?
        .json()
        .await
        .map_err(|_| AppError::Internal("Telegram response parse failed".into()))?;

    if !resp.ok {
        return Err(AppError::Internal(format!(
            "Telegram deleteWebhook failed (code {:?})",
            resp.error_code
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
        .map_err(|_| AppError::Internal("Telegram getUpdates failed".into()))?
        .json()
        .await
        .map_err(|_| AppError::Internal("Telegram getUpdates parse failed".into()))?;

    if !resp.ok {
        return Err(AppError::Internal(format!(
            "Telegram getUpdates failed (code {:?})",
            resp.error_code
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

    #[tokio::test]
    async fn actual_telegram_transport_and_provider_errors_do_not_disclose_secrets() {
        use tracing::instrument::WithSubscriber;
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

        let secret = "distinctive-telegram-bot-and-description-2e79";
        let capture = tempfile::NamedTempFile::new().unwrap();
        let writer = capture.reopen().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || writer.try_clone().unwrap())
            .finish();
        let client = Client::builder().no_proxy().build().unwrap();
        let request = SendMessageRequest {
            chat_id: 42,
            text: "Recovery code used".into(),
            parse_mode: "HTML".into(),
            reply_markup: None,
        };
        async {
            let server = MockServer::start().await;
            let url = format!("{}/bot{secret}/sendMessage", server.uri());
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                    "ok": false, "error_code": 403, "description": secret
                })))
                .expect(1)
                .mount(&server)
                .await;
            // Best-effort text delivery retains its existing result contract,
            // but diagnostics contain only the numeric protocol code.
            send_text_message_request(&client, &url, &request)
                .await
                .unwrap();
            server.verify().await;
            server.reset().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ok": secret, "description": secret
                })))
                .mount(&server)
                .await;
            let malformed = send_text_message_request(&client, &url, &request)
                .await
                .err()
                .unwrap();

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let unavailable = format!(
                "http://{}/bot{secret}/sendMessage?token={secret}",
                listener.local_addr().unwrap()
            );
            drop(listener);
            let transport = send_text_message_request(&client, &unavailable, &request)
                .await
                .err()
                .unwrap();
            for error in [malformed, transport] {
                // Notification callers log Display, so redacted Debug alone
                // cannot establish containment.
                let display = error.to_string();
                assert!(!display.contains(secret));
                tracing::warn!(error = %error, "Telegram fixture notification failed");
                let response = axum::response::IntoResponse::into_response(error);
                let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
                    .await
                    .unwrap();
                assert!(!String::from_utf8_lossy(&body).contains(secret));
            }
        }
        .with_subscriber(subscriber)
        .await;
        let logs = std::fs::read_to_string(capture.path()).unwrap();
        assert!(logs.contains("403"));
        assert!(logs.contains("Telegram fixture notification failed"));
        assert!(!logs.contains(secret));
    }

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

    #[test]
    fn format_personal_approval_message_keeps_existing_wording() {
        let text = format_approval_message(
            "OpenAI",
            "openai",
            "CI bot",
            "POST /v1/chat/completions",
            30,
            None,
        );
        assert!(text.starts_with("<b>Access Request</b>"));
        assert!(!text.contains("Org Access Request"));
        assert!(!text.contains("<b>Org:</b>"));
        assert!(text.contains("<b>Service:</b> OpenAI"));
    }

    #[test]
    fn format_org_approval_message_uses_org_header_and_line() {
        let text = format_approval_message(
            "OpenAI",
            "openai",
            "CI bot",
            "POST /v1/chat/completions",
            30,
            Some("Acme Inc."),
        );
        assert!(text.starts_with("<b>Org Access Request — Acme Inc.</b>"));
        assert!(text.contains("<b>Org:</b> Acme Inc."));
        assert!(text.contains("<b>Service:</b> OpenAI"));
    }

    #[test]
    fn format_org_approval_message_escapes_html_in_org_name() {
        let text = format_approval_message(
            "OpenAI",
            "openai",
            "CI bot",
            "POST /v1/chat/completions",
            30,
            Some("<script>&"),
        );
        assert!(text.contains("&lt;script&gt;&amp;"));
        assert!(!text.contains("<script>"));
    }
}
