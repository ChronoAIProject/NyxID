//! Telegram platform adapter for the Channel Bot Relay system.
//!
//! Implements [`PlatformAdapter`] to normalize Telegram Bot API webhooks into
//! the platform-agnostic [`InboundMessage`] format and send replies via the
//! Telegram `sendMessage` endpoint.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::services::channel_platform::{
    BotIdentity, InboundAttachment, InboundMessage, OutboundReply, PlatformAdapter,
};

const TELEGRAM_API_BASE: &str = "https://api.telegram.org/bot";

/// Header name Telegram uses to pass the webhook secret token for verification.
const SECRET_HEADER: &str = "x-telegram-bot-api-secret-token";

/// Telegram platform adapter.
///
/// Stateless -- all state lives in the [`ChannelBot`] document and the Telegram
/// API itself.
pub struct TelegramAdapter;

impl TelegramAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TelegramAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Map Telegram `chat.type` string to our normalized conversation type.
fn map_conversation_type(chat_type: &str) -> &'static str {
    match chat_type {
        "private" => "private",
        "group" | "supergroup" => "group",
        "channel" => "channel",
        _ => "private",
    }
}

/// Detect content type from a Telegram message value.
fn detect_content_type(msg: &serde_json::Value) -> &'static str {
    if msg.get("photo").is_some() {
        "image"
    } else if msg.get("document").is_some() {
        "file"
    } else if msg.get("audio").is_some() {
        "audio"
    } else if msg.get("video").is_some() {
        "video"
    } else if msg.get("voice").is_some() {
        "audio"
    } else if msg.get("sticker").is_some() {
        "image"
    } else if msg.get("location").is_some() {
        "unknown"
    } else if msg.get("text").is_some() {
        "text"
    } else {
        "unknown"
    }
}

/// Build attachment list from a Telegram message value.
fn extract_attachments(msg: &serde_json::Value) -> Vec<InboundAttachment> {
    let mut attachments = Vec::new();

    // Photo: array of PhotoSize, pick the last (largest) one
    if let Some(photos) = msg.get("photo").and_then(|v| v.as_array())
        && let Some(largest) = photos.last()
    {
        let file_id = largest
            .get("file_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        attachments.push(InboundAttachment {
            content_type: "image".to_string(),
            url: file_id.to_string(),
            filename: None,
            mime_type: None,
            size_bytes: largest.get("file_size").and_then(|v| v.as_u64()),
        });
    }

    // Document
    if let Some(doc) = msg.get("document") {
        let file_id = doc
            .get("file_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        attachments.push(InboundAttachment {
            content_type: "file".to_string(),
            url: file_id.to_string(),
            filename: doc
                .get("file_name")
                .and_then(|v| v.as_str())
                .map(String::from),
            mime_type: doc
                .get("mime_type")
                .and_then(|v| v.as_str())
                .map(String::from),
            size_bytes: doc.get("file_size").and_then(|v| v.as_u64()),
        });
    }

    // Audio
    if let Some(audio) = msg.get("audio") {
        let file_id = audio
            .get("file_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        attachments.push(InboundAttachment {
            content_type: "audio".to_string(),
            url: file_id.to_string(),
            filename: audio
                .get("file_name")
                .and_then(|v| v.as_str())
                .map(String::from),
            mime_type: audio
                .get("mime_type")
                .and_then(|v| v.as_str())
                .map(String::from),
            size_bytes: audio.get("file_size").and_then(|v| v.as_u64()),
        });
    }

    // Video
    if let Some(video) = msg.get("video") {
        let file_id = video
            .get("file_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        attachments.push(InboundAttachment {
            content_type: "video".to_string(),
            url: file_id.to_string(),
            filename: video
                .get("file_name")
                .and_then(|v| v.as_str())
                .map(String::from),
            mime_type: video
                .get("mime_type")
                .and_then(|v| v.as_str())
                .map(String::from),
            size_bytes: video.get("file_size").and_then(|v| v.as_u64()),
        });
    }

    // Voice
    if let Some(voice) = msg.get("voice") {
        let file_id = voice
            .get("file_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        attachments.push(InboundAttachment {
            content_type: "audio".to_string(),
            url: file_id.to_string(),
            filename: None,
            mime_type: voice
                .get("mime_type")
                .and_then(|v| v.as_str())
                .map(String::from),
            size_bytes: voice.get("file_size").and_then(|v| v.as_u64()),
        });
    }

    attachments
}

/// Build the sender display name from Telegram `from` object.
fn sender_display_name(from: &serde_json::Value) -> Option<String> {
    let first = from.get("first_name").and_then(|v| v.as_str())?;
    let last = from.get("last_name").and_then(|v| v.as_str());
    match last {
        Some(l) => Some(format!("{first} {l}")),
        None => Some(first.to_string()),
    }
}

/// Extract the message value from a Telegram update. Checks `message`,
/// `edited_message`, and `channel_post` in order.
fn extract_message(update: &serde_json::Value) -> Option<&serde_json::Value> {
    update
        .get("message")
        .or_else(|| update.get("edited_message"))
        .or_else(|| update.get("channel_post"))
}

/// Parse a single Telegram message value into an [`InboundMessage`].
fn parse_message(msg: &serde_json::Value, raw: serde_json::Value) -> Option<InboundMessage> {
    let message_id = msg.get("message_id")?.as_i64()?;
    let chat = msg.get("chat")?;
    let chat_id = chat.get("id")?.as_i64()?;
    let chat_type = chat
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("private");

    let from = msg.get("from");
    let sender_id = from
        .and_then(|f| f.get("id"))
        .and_then(|v| v.as_i64())
        .map(|id| id.to_string())
        .unwrap_or_default();

    let display_name = from.and_then(sender_display_name);

    let content_type = detect_content_type(msg);
    let text = msg
        .get("text")
        .and_then(|v| v.as_str())
        .or_else(|| msg.get("caption").and_then(|v| v.as_str()))
        .map(String::from);

    let reply_to = msg
        .get("reply_to_message")
        .and_then(|r| r.get("message_id"))
        .and_then(|v| v.as_i64())
        .map(|id| id.to_string());

    let thread_id = msg
        .get("message_thread_id")
        .and_then(|v| v.as_i64())
        .map(|id| id.to_string());

    let attachments = extract_attachments(msg);

    Some(InboundMessage {
        platform_message_id: message_id.to_string(),
        conversation_id: chat_id.to_string(),
        conversation_type: map_conversation_type(chat_type).to_string(),
        sender_platform_id: sender_id,
        sender_display_name: display_name,
        content_type: content_type.to_string(),
        text,
        attachments,
        reply_to_platform_message_id: reply_to,
        thread_id,
        raw_data: raw,
    })
}

// ---------------------------------------------------------------------------
// PlatformAdapter implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl PlatformAdapter for TelegramAdapter {
    fn platform_id(&self) -> &str {
        "telegram"
    }

    async fn verify_webhook(
        &self,
        bot: &ChannelBot,
        headers: &axum::http::HeaderMap,
        _body: &[u8],
    ) -> AppResult<()> {
        let header_value = headers
            .get(SECRET_HEADER)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError::ChannelWebhookVerificationFailed(
                    "missing X-Telegram-Bot-Api-Secret-Token header".to_string(),
                )
            })?;

        // Hash the incoming header value and compare with stored hash using
        // constant-time comparison to prevent timing attacks.
        let incoming_hash = hex::encode(Sha256::digest(header_value.as_bytes()));
        let stored_hash = &bot.webhook_secret_hash;

        if incoming_hash
            .as_bytes()
            .ct_eq(stored_hash.as_bytes())
            .into()
        {
            Ok(())
        } else {
            Err(AppError::ChannelWebhookVerificationFailed(
                "secret token mismatch".to_string(),
            ))
        }
    }

    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        let update: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| AppError::BadRequest(format!("invalid Telegram update JSON: {e}")))?;

        let msg = match extract_message(&update) {
            Some(m) => m,
            None => return Ok(Vec::new()),
        };

        match parse_message(msg, update.clone()) {
            Some(inbound) => Ok(vec![inbound]),
            None => Ok(Vec::new()),
        }
    }

    async fn send_reply(
        &self,
        http: &reqwest::Client,
        bot_token: &str,
        conversation_id: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        let text = reply.text.as_deref().unwrap_or("");

        let mut body = serde_json::json!({
            "chat_id": conversation_id,
            "text": text,
            "parse_mode": "Markdown",
        });

        if let Some(ref reply_to_id) = reply.reply_to_platform_message_id
            && let Ok(id) = reply_to_id.parse::<i64>()
        {
            body["reply_to_message_id"] = serde_json::json!(id);
        }

        // Honor `message_thread_id` from reply metadata when the original
        // inbound message came from a forum topic. Without this, replies
        // would fall back to the root chat. Accepted as either an integer
        // or a string that parses as i64 (handlers/channel_relay.rs
        // forwards it as a JSON string).
        if let Some(md) = reply.metadata.as_ref()
            && let Some(thread_val) = md.get("message_thread_id")
        {
            let parsed = thread_val
                .as_i64()
                .or_else(|| thread_val.as_str().and_then(|s| s.parse::<i64>().ok()));
            if let Some(thread_id) = parsed {
                body["message_thread_id"] = serde_json::json!(thread_id);
            }
        }

        let url = format!("{TELEGRAM_API_BASE}{bot_token}/sendMessage");
        let resp: serde_json::Value = http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!("Telegram sendMessage request failed: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Telegram sendMessage response parse failed: {e}"
                ))
            })?;

        if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let description = resp
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(AppError::ChannelPlatformError(format!(
                "Telegram sendMessage failed: {description}"
            )));
        }

        let message_id = resp
            .get("result")
            .and_then(|r| r.get("message_id"))
            .and_then(|v| v.as_i64())
            .map(|id| id.to_string());

        Ok(message_id)
    }

    async fn register_webhook(
        &self,
        http: &reqwest::Client,
        bot_token: &str,
        webhook_url: &str,
        secret: &str,
    ) -> AppResult<()> {
        let body = serde_json::json!({
            "url": webhook_url,
            "secret_token": secret,
            "allowed_updates": ["message", "edited_message", "channel_post"],
        });

        let url = format!("{TELEGRAM_API_BASE}{bot_token}/setWebhook");
        let resp: serde_json::Value = http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!("Telegram setWebhook request failed: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Telegram setWebhook response parse failed: {e}"
                ))
            })?;

        if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let description = resp
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(AppError::ChannelPlatformError(format!(
                "Telegram setWebhook failed: {description}"
            )));
        }

        Ok(())
    }

    async fn verify_bot_token(
        &self,
        http: &reqwest::Client,
        bot_token: &str,
    ) -> AppResult<BotIdentity> {
        let url = format!("{TELEGRAM_API_BASE}{bot_token}/getMe");
        let resp: serde_json::Value = http
            .get(&url)
            .send()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!("Telegram getMe request failed: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!("Telegram getMe response parse failed: {e}"))
            })?;

        if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let description = resp
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("invalid bot token");
            return Err(AppError::ChannelPlatformError(format!(
                "Telegram getMe failed: {description}"
            )));
        }

        let result = resp.get("result").ok_or_else(|| {
            AppError::ChannelPlatformError("Telegram getMe response missing result".to_string())
        })?;

        let bot_id = result.get("id").and_then(|v| v.as_i64()).ok_or_else(|| {
            AppError::ChannelPlatformError("Telegram getMe response missing bot id".to_string())
        })?;

        let username = result
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        Ok(BotIdentity {
            platform_bot_id: bot_id.to_string(),
            platform_bot_username: username.to_string(),
        })
    }

    fn handle_challenge(&self, _body: &[u8]) -> Option<serde_json::Value> {
        // Telegram does not use a webhook challenge mechanism.
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_inbound -------------------------------------------------------

    #[tokio::test]
    async fn parse_text_message() {
        let adapter = TelegramAdapter::new();
        let body = serde_json::json!({
            "update_id": 100,
            "message": {
                "message_id": 42,
                "from": {
                    "id": 111222333,
                    "is_bot": false,
                    "first_name": "Alice",
                    "last_name": "Smith",
                    "username": "alice"
                },
                "chat": {
                    "id": 111222333,
                    "type": "private"
                },
                "date": 1700000000,
                "text": "Hello bot"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.platform_message_id, "42");
        assert_eq!(m.conversation_id, "111222333");
        assert_eq!(m.conversation_type, "private");
        assert_eq!(m.sender_platform_id, "111222333");
        assert_eq!(m.sender_display_name.as_deref(), Some("Alice Smith"));
        assert_eq!(m.content_type, "text");
        assert_eq!(m.text.as_deref(), Some("Hello bot"));
        assert!(m.attachments.is_empty());
        assert!(m.reply_to_platform_message_id.is_none());
    }

    #[tokio::test]
    async fn parse_photo_message() {
        let adapter = TelegramAdapter::new();
        let body = serde_json::json!({
            "update_id": 101,
            "message": {
                "message_id": 43,
                "from": {
                    "id": 111222333,
                    "is_bot": false,
                    "first_name": "Bob"
                },
                "chat": {
                    "id": -100123456,
                    "type": "supergroup"
                },
                "date": 1700000001,
                "photo": [
                    { "file_id": "small_photo", "file_unique_id": "a", "width": 90, "height": 90, "file_size": 1000 },
                    { "file_id": "large_photo", "file_unique_id": "b", "width": 800, "height": 600, "file_size": 50000 }
                ],
                "caption": "Check this out"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.conversation_type, "group");
        assert_eq!(m.content_type, "image");
        assert_eq!(m.text.as_deref(), Some("Check this out"));
        assert_eq!(m.attachments.len(), 1);
        // Should pick the largest photo
        assert_eq!(m.attachments[0].url, "large_photo");
        assert_eq!(m.attachments[0].size_bytes, Some(50000));
    }

    #[tokio::test]
    async fn parse_document_message() {
        let adapter = TelegramAdapter::new();
        let body = serde_json::json!({
            "update_id": 102,
            "message": {
                "message_id": 44,
                "from": { "id": 999, "is_bot": false, "first_name": "Test" },
                "chat": { "id": 999, "type": "private" },
                "date": 1700000002,
                "document": {
                    "file_id": "doc_file_id",
                    "file_unique_id": "doc_unique",
                    "file_name": "report.pdf",
                    "mime_type": "application/pdf",
                    "file_size": 123456
                }
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.content_type, "file");
        assert_eq!(m.attachments.len(), 1);
        assert_eq!(m.attachments[0].filename.as_deref(), Some("report.pdf"));
        assert_eq!(
            m.attachments[0].mime_type.as_deref(),
            Some("application/pdf")
        );
        assert_eq!(m.attachments[0].size_bytes, Some(123456));
    }

    #[tokio::test]
    async fn parse_empty_update_returns_empty() {
        let adapter = TelegramAdapter::new();
        let body = serde_json::json!({
            "update_id": 200,
            "callback_query": {
                "id": "abc",
                "from": { "id": 1, "first_name": "X" },
                "data": "some_data"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_edited_message() {
        let adapter = TelegramAdapter::new();
        let body = serde_json::json!({
            "update_id": 300,
            "edited_message": {
                "message_id": 55,
                "from": { "id": 777, "is_bot": false, "first_name": "Eve" },
                "chat": { "id": 777, "type": "private" },
                "date": 1700000010,
                "edit_date": 1700000020,
                "text": "Edited text"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text.as_deref(), Some("Edited text"));
        assert_eq!(msgs[0].platform_message_id, "55");
    }

    #[tokio::test]
    async fn parse_channel_post() {
        let adapter = TelegramAdapter::new();
        let body = serde_json::json!({
            "update_id": 400,
            "channel_post": {
                "message_id": 66,
                "chat": { "id": -1001234567890_i64, "type": "channel" },
                "date": 1700000030,
                "text": "Channel announcement"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].conversation_type, "channel");
        assert_eq!(msgs[0].text.as_deref(), Some("Channel announcement"));
        // channel_post often has no `from` field
        assert_eq!(msgs[0].sender_platform_id, "");
    }

    #[tokio::test]
    async fn parse_reply_to_message() {
        let adapter = TelegramAdapter::new();
        let body = serde_json::json!({
            "update_id": 500,
            "message": {
                "message_id": 77,
                "from": { "id": 888, "is_bot": false, "first_name": "Reply" },
                "chat": { "id": 888, "type": "private" },
                "date": 1700000040,
                "text": "This is a reply",
                "reply_to_message": {
                    "message_id": 50,
                    "from": { "id": 999, "is_bot": true, "first_name": "Bot" },
                    "chat": { "id": 888, "type": "private" },
                    "date": 1700000030,
                    "text": "Original message"
                }
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].reply_to_platform_message_id.as_deref(), Some("50"));
    }

    // -- conversation_type mapping -------------------------------------------

    #[test]
    fn conversation_type_mapping() {
        assert_eq!(map_conversation_type("private"), "private");
        assert_eq!(map_conversation_type("group"), "group");
        assert_eq!(map_conversation_type("supergroup"), "group");
        assert_eq!(map_conversation_type("channel"), "channel");
        assert_eq!(map_conversation_type("unknown_type"), "private");
    }

    // -- content_type detection ----------------------------------------------

    #[test]
    fn content_type_detection() {
        let text_msg = serde_json::json!({ "text": "hello" });
        assert_eq!(detect_content_type(&text_msg), "text");

        let photo_msg = serde_json::json!({ "photo": [] });
        assert_eq!(detect_content_type(&photo_msg), "image");

        let doc_msg = serde_json::json!({ "document": {} });
        assert_eq!(detect_content_type(&doc_msg), "file");

        let audio_msg = serde_json::json!({ "audio": {} });
        assert_eq!(detect_content_type(&audio_msg), "audio");

        let video_msg = serde_json::json!({ "video": {} });
        assert_eq!(detect_content_type(&video_msg), "video");

        let voice_msg = serde_json::json!({ "voice": {} });
        assert_eq!(detect_content_type(&voice_msg), "audio");

        let sticker_msg = serde_json::json!({ "sticker": {} });
        assert_eq!(detect_content_type(&sticker_msg), "image");

        let location_msg = serde_json::json!({ "location": {} });
        assert_eq!(detect_content_type(&location_msg), "unknown");

        let empty_msg = serde_json::json!({});
        assert_eq!(detect_content_type(&empty_msg), "unknown");
    }

    // -- handle_challenge ----------------------------------------------------

    #[test]
    fn handle_challenge_returns_none() {
        let adapter = TelegramAdapter::new();
        assert!(adapter.handle_challenge(b"{}").is_none());
        assert!(adapter.handle_challenge(b"").is_none());
    }

    // -- platform_id ---------------------------------------------------------

    #[test]
    fn platform_id_is_telegram() {
        let adapter = TelegramAdapter::new();
        assert_eq!(adapter.platform_id(), "telegram");
    }

    // -- verify_webhook ------------------------------------------------------

    #[tokio::test]
    async fn verify_webhook_valid_secret() {
        let adapter = TelegramAdapter::new();
        let secret = "my_webhook_secret_123";
        let stored_hash = hex::encode(Sha256::digest(secret.as_bytes()));

        let bot = make_test_bot(&stored_hash);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(SECRET_HEADER, secret.parse().unwrap());

        let result = adapter.verify_webhook(&bot, &headers, b"").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn verify_webhook_invalid_secret() {
        let adapter = TelegramAdapter::new();
        let stored_hash = hex::encode(Sha256::digest(b"correct_secret"));

        let bot = make_test_bot(&stored_hash);
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(SECRET_HEADER, "wrong_secret".parse().unwrap());

        let result = adapter.verify_webhook(&bot, &headers, b"").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn verify_webhook_missing_header() {
        let adapter = TelegramAdapter::new();
        let bot = make_test_bot("somehash");
        let headers = axum::http::HeaderMap::new();

        let result = adapter.verify_webhook(&bot, &headers, b"").await;
        assert!(result.is_err());
    }

    // -- test helper ---------------------------------------------------------

    fn make_test_bot(webhook_secret_hash: &str) -> ChannelBot {
        ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "telegram".to_string(),
            label: "Test Bot".to_string(),
            bot_token_encrypted: vec![0; 16],
            platform_bot_id: "123456789".to_string(),
            platform_bot_username: "testbot".to_string(),
            webhook_registered: true,
            webhook_secret_hash: webhook_secret_hash.to_string(),
            app_id: None,
            app_secret_encrypted: None,
            public_key: None,
            status: "active".to_string(),
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    // -- parse_inbound invalid JSON ------------------------------------------

    #[tokio::test]
    async fn parse_inbound_invalid_json() {
        let adapter = TelegramAdapter::new();
        let result = adapter.parse_inbound(b"not json").await;
        assert!(result.is_err());
    }

    // -- sender_display_name -------------------------------------------------

    #[test]
    fn display_name_first_only() {
        let from = serde_json::json!({ "first_name": "Alice" });
        assert_eq!(sender_display_name(&from), Some("Alice".to_string()));
    }

    #[test]
    fn display_name_first_and_last() {
        let from = serde_json::json!({ "first_name": "Alice", "last_name": "Smith" });
        assert_eq!(sender_display_name(&from), Some("Alice Smith".to_string()));
    }

    #[test]
    fn display_name_missing() {
        let from = serde_json::json!({ "id": 123 });
        assert_eq!(sender_display_name(&from), None);
    }

    // -- extract_attachments -------------------------------------------------

    #[test]
    fn extract_voice_attachment() {
        let msg = serde_json::json!({
            "voice": {
                "file_id": "voice_file",
                "file_unique_id": "v1",
                "duration": 5,
                "mime_type": "audio/ogg",
                "file_size": 9876
            }
        });
        let attachments = extract_attachments(&msg);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].content_type, "audio");
        assert_eq!(attachments[0].url, "voice_file");
        assert_eq!(attachments[0].mime_type.as_deref(), Some("audio/ogg"));
        assert_eq!(attachments[0].size_bytes, Some(9876));
        assert!(attachments[0].filename.is_none());
    }

    #[test]
    fn extract_no_attachments() {
        let msg = serde_json::json!({ "text": "just text" });
        let attachments = extract_attachments(&msg);
        assert!(attachments.is_empty());
    }

    // -- thread_id -----------------------------------------------------------

    #[tokio::test]
    async fn parse_message_with_thread_id() {
        let adapter = TelegramAdapter::new();
        let body = serde_json::json!({
            "update_id": 600,
            "message": {
                "message_id": 88,
                "message_thread_id": 42,
                "from": { "id": 111, "is_bot": false, "first_name": "Thread" },
                "chat": { "id": -100999, "type": "supergroup" },
                "date": 1700000050,
                "text": "Topic message"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].thread_id.as_deref(), Some("42"));
    }
}
