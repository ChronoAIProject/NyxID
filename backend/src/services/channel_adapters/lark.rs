//! Lark / Feishu platform adapter for the Channel Bot Relay system.
//!
//! A single [`LarkFamilyAdapter`] struct serves both Lark (international) and
//! Feishu (China mainland) by parameterising the API base URL and platform
//! identifier. The two platforms share the same webhook format, event schema,
//! and REST API shape -- only the hostname differs.
//!
//! Webhook verification uses HMAC-SHA256 over the request body, with the
//! verification token stored on the [`ChannelBot`] document.
//!
//! Message parsing handles the standard `im.message.receive_v1` event schema
//! and the `url_verification` challenge flow.
//!
//! Tenant token acquisition goes through the generic
//! [`provider_token_exchange_service`] helpers so the channel adapter and
//! the proxy's `token_exchange` auth method share one in-memory cache with
//! per-key single-flight.

use std::sync::Arc;

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::models::downstream_service::{CredentialFieldSpec, TokenExchangeConfig};
use crate::services::channel_platform::{
    BotIdentity, InboundMessage, OutboundReply, PlatformAdapter,
};
use crate::services::provider_token_exchange_service::{self, TokenExchangeCache};

type HmacSha256 = Hmac<Sha256>;

/// Build the `TokenExchangeConfig` that matches Lark / Feishu's tenant
/// token endpoint. Shared with the proxy catalog seeds so there is exactly
/// one definition in the tree.
pub fn lark_family_token_exchange_config() -> TokenExchangeConfig {
    TokenExchangeConfig {
        endpoint: "{base_url}/open-apis/auth/v3/tenant_access_token/internal".to_string(),
        request_encoding: "json".to_string(),
        request_template: serde_json::json!({
            "app_id": "$app_id",
            "app_secret": "$app_secret",
        }),
        token_response_path: "tenant_access_token".to_string(),
        ttl_response_path: Some("expire".to_string()),
        default_ttl_secs: 7200,
        injection: "bearer".to_string(),
        error_code_path: Some("code".to_string()),
        error_message_path: Some("msg".to_string()),
        credential_fields: vec![
            CredentialFieldSpec {
                name: "app_id".to_string(),
                label: "App ID".to_string(),
                placeholder: Some("cli_a940e30bf3b89eea".to_string()),
                secret: false,
            },
            CredentialFieldSpec {
                name: "app_secret".to_string(),
                label: "App Secret".to_string(),
                placeholder: None,
                secret: true,
            },
        ],
    }
}

/// Lark / Feishu platform adapter.
///
/// Created via [`LarkFamilyAdapter::lark()`] or [`LarkFamilyAdapter::feishu()`].
pub struct LarkFamilyAdapter {
    base_url: String,
    platform: String,
    token_exchange_cache: Arc<TokenExchangeCache>,
}

impl LarkFamilyAdapter {
    /// Create an adapter for the international Lark platform.
    pub fn lark(token_exchange_cache: Arc<TokenExchangeCache>) -> Self {
        Self {
            base_url: "https://open.larksuite.com".to_string(),
            platform: "lark".to_string(),
            token_exchange_cache,
        }
    }

    /// Create an adapter for the China mainland Feishu platform.
    pub fn feishu(token_exchange_cache: Arc<TokenExchangeCache>) -> Self {
        Self {
            base_url: "https://open.feishu.cn".to_string(),
            platform: "feishu".to_string(),
            token_exchange_cache,
        }
    }

    /// Exchange app credentials for a tenant access token via the shared
    /// process-wide cache. Multiple concurrent callers for the same app
    /// coalesce into a single HTTP round-trip (see `TokenExchangeCache`).
    async fn get_tenant_access_token(
        &self,
        http: &reqwest::Client,
        app_id: &str,
        app_secret: &str,
    ) -> AppResult<String> {
        let config = lark_family_token_exchange_config();
        let credential_json = serde_json::json!({
            "app_id": app_id,
            "app_secret": app_secret,
        })
        .to_string();
        let mut credential_map = serde_json::Map::new();
        credential_map.insert("app_id".to_string(), serde_json::json!(app_id));
        credential_map.insert("app_secret".to_string(), serde_json::json!(app_secret));

        provider_token_exchange_service::get_cached_exchange_token(
            &self.token_exchange_cache,
            http,
            &self.base_url,
            &credential_json,
            &config,
            &credential_map,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Map Lark `chat_type` string to our normalized conversation type.
fn map_conversation_type(chat_type: &str) -> &'static str {
    match chat_type {
        "p2p" => "private",
        "group" => "group",
        _ => "group",
    }
}

/// Extract the text content from a Lark message content JSON string.
///
/// Lark sends `message.content` as a JSON-encoded string, e.g.
/// `"{\"text\":\"hello\"}"`. This helper double-parses and extracts the
/// `text` field.
fn extract_text_content(content_str: &str) -> Option<String> {
    let inner: serde_json::Value = serde_json::from_str(content_str).ok()?;
    inner.get("text").and_then(|v| v.as_str()).map(String::from)
}

/// Detect the content type from the Lark `message_type` field.
fn detect_content_type(message_type: &str) -> &'static str {
    match message_type {
        "text" => "text",
        "image" => "image",
        "file" => "file",
        "audio" => "audio",
        "video" => "video",
        "interactive" => "text",
        _ => "unknown",
    }
}

/// Parse an `im.message.receive_v1` event into an [`InboundMessage`].
fn parse_message_event(
    event: &serde_json::Value,
    raw: serde_json::Value,
) -> Option<InboundMessage> {
    let message = event.get("message")?;
    let message_id = message.get("message_id")?.as_str()?;
    let chat_id = message.get("chat_id")?.as_str()?;
    let chat_type = message
        .get("chat_type")
        .and_then(|v| v.as_str())
        .unwrap_or("group");

    let message_type = message
        .get("message_type")
        .and_then(|v| v.as_str())
        .unwrap_or("text");

    let content_str = message.get("content").and_then(|v| v.as_str());
    let text = content_str.and_then(extract_text_content);

    let sender = event.get("sender");
    let sender_id = sender
        .and_then(|s| s.get("sender_id"))
        .and_then(|s| s.get("open_id"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let sender_name = sender
        .and_then(|s| s.get("sender_id"))
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let reply_to = message
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    let thread_id = message
        .get("thread_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    Some(InboundMessage {
        platform_message_id: message_id.to_string(),
        conversation_id: chat_id.to_string(),
        conversation_type: map_conversation_type(chat_type).to_string(),
        sender_platform_id: sender_id,
        sender_display_name: sender_name,
        content_type: detect_content_type(message_type).to_string(),
        text,
        attachments: Vec::new(),
        reply_to_platform_message_id: reply_to,
        thread_id,
        raw_data: raw,
    })
}

// ---------------------------------------------------------------------------
// PlatformAdapter implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl PlatformAdapter for LarkFamilyAdapter {
    fn platform_id(&self) -> &str {
        &self.platform
    }

    async fn verify_webhook(
        &self,
        bot: &ChannelBot,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<()> {
        // Lark sends signature in X-Lark-Signature header
        let header_signature = headers
            .get("x-lark-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError::ChannelWebhookVerificationFailed(
                    "missing X-Lark-Signature header".to_string(),
                )
            })?;

        // Parse the body to extract timestamp and nonce for verification
        let payload: serde_json::Value = serde_json::from_slice(body).map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "invalid JSON body for signature verification".to_string(),
            )
        })?;

        let timestamp = payload
            .get("header")
            .and_then(|h| h.get("create_time"))
            .and_then(|v| v.as_str())
            .or_else(|| payload.get("ts").and_then(|v| v.as_str()))
            .unwrap_or("");

        let nonce = payload
            .get("header")
            .and_then(|h| h.get("nonce"))
            .and_then(|v| v.as_str())
            .or_else(|| payload.get("nonce").and_then(|v| v.as_str()))
            .unwrap_or("");

        // The webhook_secret_hash stores the SHA-256 hash of the verification
        // token. For HMAC verification we compute:
        // HMAC-SHA256(verification_token, timestamp + nonce + body)
        // However, since we only store the hash, we use the hash itself as the
        // HMAC key (consistent with how the token was registered).
        let stored_hash = &bot.webhook_secret_hash;

        // Build the HMAC message: timestamp + nonce + encrypt_key + body_string
        let body_str = std::str::from_utf8(body).unwrap_or("");
        let hmac_message = format!("{timestamp}{nonce}{stored_hash}{body_str}");

        let mut mac = HmacSha256::new_from_slice(stored_hash.as_bytes()).map_err(|_| {
            AppError::ChannelWebhookVerificationFailed("failed to create HMAC verifier".to_string())
        })?;
        mac.update(hmac_message.as_bytes());
        let computed = hex::encode(mac.finalize().into_bytes());

        if computed
            .as_bytes()
            .ct_eq(header_signature.as_bytes())
            .into()
        {
            Ok(())
        } else {
            Err(AppError::ChannelWebhookVerificationFailed(
                "Lark signature verification failed".to_string(),
            ))
        }
    }

    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| AppError::BadRequest(format!("invalid Lark/Feishu webhook JSON: {e}")))?;

        // Check if this is a challenge (url_verification) -- should be handled
        // by handle_challenge first, but be defensive.
        if payload.get("type").and_then(|v| v.as_str()) == Some("url_verification") {
            return Ok(Vec::new());
        }

        // Lark Event API v2 wraps the event data in an `event` field
        let event = match payload.get("event") {
            Some(e) => e,
            None => return Ok(Vec::new()),
        };

        // Only handle im.message.receive_v1 events
        let event_type = payload
            .get("header")
            .and_then(|h| h.get("event_type"))
            .and_then(|v| v.as_str());

        if event_type != Some("im.message.receive_v1") {
            return Ok(Vec::new());
        }

        match parse_message_event(event, payload.clone()) {
            Some(msg) => Ok(vec![msg]),
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
        // For Lark/Feishu, bot_token is stored as "app_id:app_secret".
        // We must exchange it for a tenant_access_token first.
        let (app_id, app_secret) = bot_token.split_once(':').ok_or_else(|| {
            AppError::ChannelPlatformError(format!(
                "{} bot_token must be in app_id:app_secret format",
                self.platform
            ))
        })?;

        let tenant_token = self
            .get_tenant_access_token(http, app_id, app_secret)
            .await?;

        let text = reply.text.as_deref().unwrap_or("");

        // Lark requires content as a JSON string inside the body
        let content = serde_json::json!({ "text": text }).to_string();

        let body = serde_json::json!({
            "receive_id": conversation_id,
            "msg_type": "text",
            "content": content,
        });

        let url = format!(
            "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
            self.base_url
        );

        let resp: serde_json::Value = http
            .post(&url)
            .header("Authorization", format!("Bearer {tenant_token}"))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "{} send message request failed: {e}",
                    self.platform
                ))
            })?
            .json()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "{} send message response parse failed: {e}",
                    self.platform
                ))
            })?;

        // Lark success: { "code": 0, "data": { "message_id": "..." } }
        let code = resp.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = resp
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(AppError::ChannelPlatformError(format!(
                "{} send message failed (code {code}): {msg}",
                self.platform
            )));
        }

        let message_id = resp
            .get("data")
            .and_then(|d| d.get("message_id"))
            .and_then(|v| v.as_str())
            .map(String::from);

        Ok(message_id)
    }

    async fn register_webhook(
        &self,
        _http: &reqwest::Client,
        _bot_token: &str,
        _webhook_url: &str,
        _secret: &str,
    ) -> AppResult<()> {
        // Lark/Feishu webhook URLs are configured in the Developer Console,
        // not via API. This is a no-op.
        Ok(())
    }

    async fn verify_bot_token(
        &self,
        http: &reqwest::Client,
        bot_token: &str,
    ) -> AppResult<BotIdentity> {
        // For Lark/Feishu, bot_token is "app_id:app_secret". Verify the
        // credentials by attempting to obtain a tenant_access_token.
        let (app_id, app_secret) = bot_token.split_once(':').ok_or_else(|| {
            AppError::ChannelPlatformError(format!(
                "{} bot_token must be in app_id:app_secret format (provide both app_id and app_secret)",
                self.platform
            ))
        })?;

        // This will fail with an API error if credentials are invalid
        let _token = self
            .get_tenant_access_token(http, app_id, app_secret)
            .await?;

        Ok(BotIdentity {
            platform_bot_id: app_id.to_string(),
            platform_bot_username: format!("{}_bot", self.platform),
        })
    }

    fn handle_challenge(&self, body: &[u8]) -> Option<serde_json::Value> {
        let payload: serde_json::Value = serde_json::from_slice(body).ok()?;

        // Lark url_verification: { "type": "url_verification", "challenge": "..." }
        if payload.get("type").and_then(|v| v.as_str()) == Some("url_verification") {
            let challenge = payload.get("challenge")?.as_str()?;
            return Some(serde_json::json!({ "challenge": challenge }));
        }

        // Also handle the schema field variant used in some Lark versions
        if let Some(challenge) = payload.get("challenge").and_then(|v| v.as_str())
            && payload.get("token").is_some()
        {
            return Some(serde_json::json!({ "challenge": challenge }));
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests only exercise parsing/signature verification paths, so they
    /// never actually hit the cache. We still need a concrete instance to
    /// pass to the adapter constructors.
    fn test_cache() -> Arc<TokenExchangeCache> {
        Arc::new(TokenExchangeCache::new())
    }

    // -- platform_id ---------------------------------------------------------

    #[test]
    fn platform_id_lark() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        assert_eq!(adapter.platform_id(), "lark");
    }

    #[test]
    fn platform_id_feishu() {
        let adapter = LarkFamilyAdapter::feishu(test_cache());
        assert_eq!(adapter.platform_id(), "feishu");
    }

    // -- handle_challenge ----------------------------------------------------

    #[test]
    fn handle_challenge_url_verification() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let body = serde_json::json!({
            "type": "url_verification",
            "challenge": "abc123def456",
            "token": "verify_token"
        });
        let result = adapter.handle_challenge(serde_json::to_vec(&body).unwrap().as_slice());
        assert!(result.is_some());
        let resp = result.unwrap();
        assert_eq!(resp["challenge"], "abc123def456");
    }

    #[test]
    fn handle_challenge_non_verification_returns_none() {
        let adapter = LarkFamilyAdapter::feishu(test_cache());
        let body = serde_json::json!({
            "schema": "2.0",
            "header": { "event_type": "im.message.receive_v1" },
            "event": {}
        });
        let result = adapter.handle_challenge(serde_json::to_vec(&body).unwrap().as_slice());
        assert!(result.is_none());
    }

    #[test]
    fn handle_challenge_invalid_json_returns_none() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        assert!(adapter.handle_challenge(b"not json").is_none());
    }

    // -- parse_inbound -------------------------------------------------------

    #[tokio::test]
    async fn parse_text_message() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let body = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "ev_123",
                "event_type": "im.message.receive_v1",
                "create_time": "1700000000",
                "nonce": "abc123"
            },
            "event": {
                "sender": {
                    "sender_id": {
                        "open_id": "ou_user123",
                        "name": "Alice"
                    }
                },
                "message": {
                    "message_id": "om_msg456",
                    "chat_id": "oc_chat789",
                    "chat_type": "p2p",
                    "message_type": "text",
                    "content": "{\"text\":\"Hello bot\"}"
                }
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.platform_message_id, "om_msg456");
        assert_eq!(m.conversation_id, "oc_chat789");
        assert_eq!(m.conversation_type, "private");
        assert_eq!(m.sender_platform_id, "ou_user123");
        assert_eq!(m.sender_display_name.as_deref(), Some("Alice"));
        assert_eq!(m.content_type, "text");
        assert_eq!(m.text.as_deref(), Some("Hello bot"));
    }

    #[tokio::test]
    async fn parse_group_message() {
        let adapter = LarkFamilyAdapter::feishu(test_cache());
        let body = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "ev_group",
                "event_type": "im.message.receive_v1",
                "create_time": "1700000001"
            },
            "event": {
                "sender": {
                    "sender_id": {
                        "open_id": "ou_bob"
                    }
                },
                "message": {
                    "message_id": "om_grp",
                    "chat_id": "oc_grp",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"Group message\"}"
                }
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].conversation_type, "group");
    }

    #[tokio::test]
    async fn parse_url_verification_returns_empty() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let body = serde_json::json!({
            "type": "url_verification",
            "challenge": "test_challenge",
            "token": "verify_token"
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_non_message_event_returns_empty() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let body = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.chat.member.bot.added_v1"
            },
            "event": {
                "chat_id": "oc_xxx"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_no_event_field_returns_empty() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let body = serde_json::json!({ "schema": "2.0" });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_invalid_json_returns_error() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let result = adapter.parse_inbound(b"not json").await;
        assert!(result.is_err());
    }

    // -- conversation_type mapping -------------------------------------------

    #[test]
    fn conversation_type_mapping() {
        assert_eq!(map_conversation_type("p2p"), "private");
        assert_eq!(map_conversation_type("group"), "group");
        assert_eq!(map_conversation_type("unknown"), "group");
    }

    // -- content_type detection ----------------------------------------------

    #[test]
    fn content_type_detection() {
        assert_eq!(detect_content_type("text"), "text");
        assert_eq!(detect_content_type("image"), "image");
        assert_eq!(detect_content_type("file"), "file");
        assert_eq!(detect_content_type("audio"), "audio");
        assert_eq!(detect_content_type("video"), "video");
        assert_eq!(detect_content_type("interactive"), "text");
        assert_eq!(detect_content_type("sticker"), "unknown");
    }

    // -- text extraction -----------------------------------------------------

    #[test]
    fn extract_text_from_json_string() {
        assert_eq!(
            extract_text_content(r#"{"text":"Hello"}"#),
            Some("Hello".to_string())
        );
    }

    #[test]
    fn extract_text_missing_field() {
        assert_eq!(extract_text_content(r#"{"image_key":"abc"}"#), None);
    }

    #[test]
    fn extract_text_invalid_json() {
        assert_eq!(extract_text_content("not json"), None);
    }

    // -- base_url check ------------------------------------------------------

    #[test]
    fn lark_base_url() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        assert_eq!(adapter.base_url, "https://open.larksuite.com");
    }

    #[test]
    fn feishu_base_url() {
        let adapter = LarkFamilyAdapter::feishu(test_cache());
        assert_eq!(adapter.base_url, "https://open.feishu.cn");
    }

    // -- message with reply and thread ---------------------------------------

    #[tokio::test]
    async fn parse_message_with_reply_and_thread() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let body = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": {
                        "open_id": "ou_reply_user"
                    }
                },
                "message": {
                    "message_id": "om_reply_msg",
                    "chat_id": "oc_chat",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"reply text\"}",
                    "parent_id": "om_parent",
                    "thread_id": "ot_thread"
                }
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.reply_to_platform_message_id.as_deref(), Some("om_parent"));
        assert_eq!(m.thread_id.as_deref(), Some("ot_thread"));
    }
}
