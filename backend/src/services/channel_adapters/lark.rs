//! Lark / Feishu platform adapter for the Channel Bot Relay system.
//!
//! A single [`LarkFamilyAdapter`] struct serves both Lark (international) and
//! Feishu (China mainland) by parameterising the API base URL and platform
//! identifier. The two platforms share the same webhook format, event schema,
//! and REST API shape -- only the hostname differs.
//!
//! Webhook verification follows the Event Subscriptions security model
//! documented by Lark / Feishu:
//! - **Verification Token** (required): Lark copies this token into every
//!   event body (`header.token` in v2 schema, top-level `token` in v1). The
//!   adapter compares it against the decrypted token stored on the bot in
//!   constant time.
//! - **Encrypt Key** (optional): when set on the platform side, Lark
//!   encrypts the body with AES-256-CBC, wraps it in `{"encrypt": "..."}`,
//!   and signs it with `X-Lark-Signature = hex(SHA-256(timestamp + nonce +
//!   encrypt_key + raw_encrypted_body))`. The adapter verifies the signature
//!   and returns the decrypted plaintext body for downstream parsing.
//!
//! Message parsing handles the standard `im.message.receive_v1` event schema,
//! interactive card callbacks via `card.action.trigger`, and the
//! `url_verification` challenge flow.
//!
//! Tenant token acquisition goes through the generic
//! [`provider_token_exchange_service`] helpers so the channel adapter and
//! the proxy's `token_exchange` auth method share one in-memory cache with
//! per-key single-flight.

use std::sync::Arc;

use aes::Aes256;
use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::models::downstream_service::{CredentialFieldSpec, TokenExchangeConfig};
use crate::services::channel_platform::{
    BotIdentity, InboundMessage, OutboundReply, PlatformAdapter, WebhookSecrets,
};
use crate::services::provider_token_exchange_service::{self, TokenExchangeCache};

type Aes256CbcDec = cbc::Decryptor<Aes256>;

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

/// Build `(msg_type, content)` for Lark's `im.v1.messages` send endpoint.
///
/// If `reply.metadata` contains a `"card"` key, sends as an interactive
/// Feishu Card (JSON 2.0 format) with `msg_type = "interactive"`. The card
/// JSON is passed through as-is; Feishu validates it server-side.
///
/// Otherwise falls back to a plain text message wrapping `reply.text`.
fn build_message_body(reply: &OutboundReply) -> (&'static str, String) {
    if let Some(metadata) = reply.metadata.as_ref()
        && let Some(card) = metadata.get("card")
    {
        return ("interactive", card.to_string());
    }

    let text = reply.text.as_deref().unwrap_or("");
    ("text", serde_json::json!({ "text": text }).to_string())
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

/// Parse a `card.action.trigger` event into an [`InboundMessage`].
fn parse_card_action_event(
    header: &serde_json::Value,
    event: &serde_json::Value,
    raw: serde_json::Value,
) -> Option<InboundMessage> {
    let context = event.get("context")?;
    let chat_id = context.get("open_chat_id").and_then(|v| v.as_str())?;
    let chat_type = context
        .get("chat_type")
        .and_then(|v| v.as_str())
        .unwrap_or("group");

    let action = event.get("action");
    let text = serde_json::to_string(&serde_json::json!({
        "tag": action.and_then(|a| a.get("tag")).and_then(|v| v.as_str()),
        "value": action.and_then(|a| a.get("value")).cloned(),
        "form_value": action.and_then(|a| a.get("form_value")).cloned(),
        "open_message_id": context.get("open_message_id").and_then(|v| v.as_str()),
    }))
    .ok()?;

    let sender_id = event
        .get("operator")
        .and_then(|o| o.get("open_id"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let sender_name = event
        .get("operator")
        .and_then(|o| o.get("name"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let reply_to = context
        .get("open_message_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    Some(InboundMessage {
        platform_message_id: header.get("event_id").and_then(|v| v.as_str())?.to_string(),
        conversation_id: chat_id.to_string(),
        conversation_type: map_conversation_type(chat_type).to_string(),
        sender_platform_id: sender_id,
        sender_display_name: sender_name,
        content_type: "card_action".to_string(),
        text: Some(text),
        attachments: Vec::new(),
        reply_to_platform_message_id: reply_to,
        thread_id: None,
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
        _bot: &ChannelBot,
        secrets: &WebhookSecrets,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<Option<Vec<u8>>> {
        let verification_token =
            secrets
                .lark_verification_token
                .as_deref()
                .ok_or_else(|| {
                    AppError::ChannelWebhookVerificationFailed(
                        "Lark/Feishu verification token not configured on the bot. \
                         Re-register the bot with its Event Subscription Verification Token."
                            .to_string(),
                    )
                })?;

        // When Encrypt Key is configured on the Lark side, the body arrives
        // as `{"encrypt": "<base64>"}` accompanied by `X-Lark-Signature`. Verify
        // the signature and decrypt the envelope before any token check.
        let (plaintext_bytes, rewrote_body) = match secrets.lark_encrypt_key.as_deref() {
            Some(key) if !key.is_empty() => {
                verify_lark_signature(key, headers, body)?;
                (decrypt_lark_envelope(key, body)?, true)
            }
            _ => (body.to_vec(), false),
        };

        let payload: serde_json::Value =
            serde_json::from_slice(&plaintext_bytes).map_err(|e| {
                AppError::ChannelWebhookVerificationFailed(format!(
                    "Lark webhook body is not valid JSON: {e}"
                ))
            })?;

        // url_verification requests are sent BEFORE the inbound-event flow
        // and carry the token at the top level; handle_challenge upstream
        // should already have short-circuited them, but if one ever lands
        // here (e.g. replay) we still accept it as long as the token matches.
        let payload_token = extract_lark_token(&payload);

        let payload_token = payload_token.ok_or_else(|| {
            AppError::ChannelWebhookVerificationFailed(
                "Lark webhook body missing `token` field; cannot verify origin".to_string(),
            )
        })?;

        let matches: bool = payload_token
            .as_bytes()
            .ct_eq(verification_token.as_bytes())
            .into();
        if !matches {
            return Err(AppError::ChannelWebhookVerificationFailed(
                "Lark verification token mismatch".to_string(),
            ));
        }

        Ok(if rewrote_body {
            Some(plaintext_bytes)
        } else {
            None
        })
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

        let header = match payload.get("header") {
            Some(h) => h,
            None => return Ok(Vec::new()),
        };

        let event_type = header.get("event_type").and_then(|v| v.as_str());

        let parsed = match event_type {
            Some("im.message.receive_v1") => parse_message_event(event, payload.clone()),
            Some("card.action.trigger") => parse_card_action_event(header, event, payload.clone()),
            _ => None,
        };

        match parsed {
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

        let (msg_type, content) = build_message_body(reply);

        let body = serde_json::json!({
            "receive_id": conversation_id,
            "msg_type": msg_type,
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
// Lark Event Subscription security helpers
// ---------------------------------------------------------------------------

/// Extract the Lark Event Subscription "Verification Token" from a decoded
/// webhook payload.
///
/// Lark delivers the token in different places depending on the schema:
/// v2 events put it at `header.token`, older v1 events and `url_verification`
/// challenges put it at the top level (`token`). We accept both.
fn extract_lark_token(payload: &serde_json::Value) -> Option<&str> {
    payload
        .get("header")
        .and_then(|h| h.get("token"))
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("token").and_then(|v| v.as_str()))
}

/// Verify the `X-Lark-Signature` header that Lark sends when Event
/// Subscription encryption is enabled.
///
/// Per Lark's docs the signature is
/// `hex(SHA256(timestamp + nonce + encrypt_key + raw_body))` -- a plain
/// SHA-256 digest, NOT HMAC. The comparison is constant-time to avoid
/// timing oracles.
fn verify_lark_signature(
    encrypt_key: &str,
    headers: &axum::http::HeaderMap,
    body: &[u8],
) -> AppResult<()> {
    let expected_sig = headers
        .get("x-lark-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AppError::ChannelWebhookVerificationFailed(
                "missing X-Lark-Signature header (Encrypt Key is configured on this bot, \
                 so Lark must sign every request)"
                    .to_string(),
            )
        })?;

    let timestamp = headers
        .get("x-lark-request-timestamp")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AppError::ChannelWebhookVerificationFailed(
                "missing X-Lark-Request-Timestamp header".to_string(),
            )
        })?;

    let nonce = headers
        .get("x-lark-request-nonce")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AppError::ChannelWebhookVerificationFailed(
                "missing X-Lark-Request-Nonce header".to_string(),
            )
        })?;

    let mut hasher = Sha256::new();
    hasher.update(timestamp.as_bytes());
    hasher.update(nonce.as_bytes());
    hasher.update(encrypt_key.as_bytes());
    hasher.update(body);
    let computed = hex::encode(hasher.finalize());

    if computed.as_bytes().ct_eq(expected_sig.as_bytes()).into() {
        Ok(())
    } else {
        Err(AppError::ChannelWebhookVerificationFailed(
            "Lark signature verification failed".to_string(),
        ))
    }
}

/// Decrypt Lark's `{"encrypt": "<base64>"}` envelope into the plaintext JSON
/// event body.
///
/// Lark uses AES-256-CBC with PKCS#7 padding. The encryption key is
/// `SHA-256(encrypt_key)` and the IV is the first 16 bytes of the ciphertext;
/// the remainder is the actual encrypted payload.
fn decrypt_lark_envelope(encrypt_key: &str, body: &[u8]) -> AppResult<Vec<u8>> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        encrypt: String,
    }

    let env: Envelope = serde_json::from_slice(body).map_err(|e| {
        AppError::ChannelWebhookVerificationFailed(format!(
            "Lark encrypted body must be JSON with an `encrypt` field: {e}"
        ))
    })?;

    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(env.encrypt.as_bytes())
        .map_err(|e| {
            AppError::ChannelWebhookVerificationFailed(format!(
                "Lark `encrypt` value is not valid base64: {e}"
            ))
        })?;

    if ciphertext.len() < 16 {
        return Err(AppError::ChannelWebhookVerificationFailed(
            "Lark encrypted payload shorter than AES block size".to_string(),
        ));
    }

    let (iv, payload) = ciphertext.split_at(16);
    let key = Sha256::digest(encrypt_key.as_bytes());

    let mut buffer = payload.to_vec();
    let plaintext = Aes256CbcDec::new(key.as_slice().into(), iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "Lark AES-256-CBC decryption failed (wrong Encrypt Key?)".to_string(),
            )
        })?;

    Ok(plaintext.to_vec())
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

    // -- verify_webhook ------------------------------------------------------

    fn make_test_bot() -> ChannelBot {
        ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "lark".to_string(),
            label: "Test Lark Bot".to_string(),
            bot_token_encrypted: vec![0; 16],
            platform_bot_id: "cli_test".to_string(),
            platform_bot_username: "lark_bot".to_string(),
            webhook_registered: true,
            webhook_secret_hash: String::new(),
            app_id: Some("cli_test".to_string()),
            app_secret_encrypted: None,
            lark_verification_token_encrypted: None,
            lark_encrypt_key_encrypted: None,
            public_key: None,
            status: "active".to_string(),
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn secrets_with(verification_token: &str, encrypt_key: Option<&str>) -> WebhookSecrets {
        WebhookSecrets {
            lark_verification_token: Some(verification_token.to_string()),
            lark_encrypt_key: encrypt_key.map(String::from),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn verify_webhook_plaintext_matching_token_v2_header() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot();
        let secrets = secrets_with("vt_abc123", None);

        let body = serde_json::to_vec(&serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.message.receive_v1",
                "token": "vt_abc123"
            },
            "event": {}
        }))
        .unwrap();

        let headers = axum::http::HeaderMap::new();
        let result = adapter
            .verify_webhook(&bot, &secrets, &headers, &body)
            .await
            .expect("matching token should pass without signature");
        assert!(result.is_none(), "plaintext body should not be rewritten");
    }

    #[tokio::test]
    async fn verify_webhook_plaintext_matching_token_v1_top_level() {
        let adapter = LarkFamilyAdapter::feishu(test_cache());
        let bot = make_test_bot();
        let secrets = secrets_with("vt_xyz", None);

        let body = serde_json::to_vec(&serde_json::json!({
            "type": "event_callback",
            "token": "vt_xyz",
            "event": {}
        }))
        .unwrap();

        let headers = axum::http::HeaderMap::new();
        adapter
            .verify_webhook(&bot, &secrets, &headers, &body)
            .await
            .expect("v1 top-level token should match");
    }

    #[tokio::test]
    async fn verify_webhook_plaintext_mismatched_token_is_rejected() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot();
        let secrets = secrets_with("expected_token", None);

        let body = serde_json::to_vec(&serde_json::json!({
            "header": { "token": "wrong_token" },
            "event": {}
        }))
        .unwrap();

        let err = adapter
            .verify_webhook(&bot, &secrets, &axum::http::HeaderMap::new(), &body)
            .await
            .expect_err("mismatched token must fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
    }

    #[tokio::test]
    async fn verify_webhook_missing_verification_token_on_bot() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot();
        // No verification token configured on the bot.
        let secrets = WebhookSecrets::default();

        let body = serde_json::to_vec(&serde_json::json!({
            "header": { "token": "anything" },
            "event": {}
        }))
        .unwrap();

        let err = adapter
            .verify_webhook(&bot, &secrets, &axum::http::HeaderMap::new(), &body)
            .await
            .expect_err("bot without verification token must not pass");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
    }

    #[tokio::test]
    async fn verify_webhook_body_missing_token_field() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot();
        let secrets = secrets_with("some_token", None);

        let body = serde_json::to_vec(&serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {}
        }))
        .unwrap();

        let err = adapter
            .verify_webhook(&bot, &secrets, &axum::http::HeaderMap::new(), &body)
            .await
            .expect_err("body without token must fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
    }

    #[tokio::test]
    async fn verify_webhook_encrypted_body_valid_roundtrip() {
        use aes::Aes256;
        use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
        type Aes256CbcEnc = cbc::Encryptor<Aes256>;

        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot();
        let encrypt_key = "bZBy7jqmK8Uy8d3C";
        let verification_token = "vt_encrypted";
        let secrets = secrets_with(verification_token, Some(encrypt_key));

        // Build a plaintext event body that carries the verification token.
        let plaintext = serde_json::to_vec(&serde_json::json!({
            "header": { "token": verification_token },
            "event": { "hello": "world" }
        }))
        .unwrap();

        // Encrypt it the way Lark does: AES-256-CBC / PKCS7 / key =
        // SHA256(encrypt_key), IV = 16 random bytes prepended to the
        // ciphertext.
        let key = Sha256::digest(encrypt_key.as_bytes());
        let iv = [0x11u8; 16];
        let block_size = 16;
        let mut buf = vec![0u8; plaintext.len() + block_size];
        buf[..plaintext.len()].copy_from_slice(&plaintext);
        let ct_len = Aes256CbcEnc::new(key.as_slice().into(), (&iv).into())
            .encrypt_padded_mut::<Pkcs7>(&mut buf, plaintext.len())
            .expect("encrypt")
            .len();
        let mut full_ct = iv.to_vec();
        full_ct.extend_from_slice(&buf[..ct_len]);
        let envelope = serde_json::to_vec(&serde_json::json!({
            "encrypt": base64::engine::general_purpose::STANDARD.encode(&full_ct)
        }))
        .unwrap();

        // Build the matching X-Lark-Signature: SHA256(ts + nonce + key + body)
        let ts = "1700000000";
        let nonce = "nonce123";
        let mut hasher = Sha256::new();
        hasher.update(ts.as_bytes());
        hasher.update(nonce.as_bytes());
        hasher.update(encrypt_key.as_bytes());
        hasher.update(&envelope);
        let sig = hex::encode(hasher.finalize());

        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-lark-signature", sig.parse().unwrap());
        headers.insert("x-lark-request-timestamp", ts.parse().unwrap());
        headers.insert("x-lark-request-nonce", nonce.parse().unwrap());

        let decoded = adapter
            .verify_webhook(&bot, &secrets, &headers, &envelope)
            .await
            .expect("valid encrypted body should pass");
        let decoded = decoded.expect("encrypted path must return plaintext body");
        assert_eq!(decoded, plaintext);
    }

    #[tokio::test]
    async fn verify_webhook_encrypted_body_bad_signature() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot();
        let encrypt_key = "k";
        let secrets = secrets_with("vt", Some(encrypt_key));

        let envelope = serde_json::to_vec(&serde_json::json!({
            "encrypt": "aaaa"
        }))
        .unwrap();

        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-lark-signature", "deadbeef".parse().unwrap());
        headers.insert("x-lark-request-timestamp", "1".parse().unwrap());
        headers.insert("x-lark-request-nonce", "n".parse().unwrap());

        let err = adapter
            .verify_webhook(&bot, &secrets, &headers, &envelope)
            .await
            .expect_err("bad signature must fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
    }

    #[tokio::test]
    async fn verify_webhook_encrypted_body_missing_signature_header() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot();
        let secrets = secrets_with("vt", Some("key"));
        let envelope = serde_json::to_vec(&serde_json::json!({ "encrypt": "x" })).unwrap();

        let err = adapter
            .verify_webhook(&bot, &secrets, &axum::http::HeaderMap::new(), &envelope)
            .await
            .expect_err("missing signature must fail when encrypt_key is configured");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
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
    async fn parse_card_action_button_click() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let body = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "ev_btn",
                "event_type": "card.action.trigger",
                "create_time": "1700000002"
            },
            "event": {
                "operator": {
                    "open_id": "ou_operator123",
                    "name": "Alice"
                },
                "action": {
                    "tag": "button",
                    "value": {
                        "button_id": "approve",
                        "step": 1
                    }
                },
                "context": {
                    "open_chat_id": "oc_chat123",
                    "chat_type": "p2p",
                    "open_message_id": "om_xxx"
                }
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.content_type, "card_action");
        assert_eq!(m.platform_message_id, "ev_btn");
        assert_eq!(m.reply_to_platform_message_id.as_deref(), Some("om_xxx"));

        let envelope: serde_json::Value = serde_json::from_str(m.text.as_deref().unwrap()).unwrap();
        assert_eq!(envelope["tag"], "button");
        assert_eq!(envelope["value"]["button_id"], "approve");
        assert_eq!(envelope["open_message_id"], "om_xxx");
    }

    #[tokio::test]
    async fn parse_card_action_form_submit() {
        let adapter = LarkFamilyAdapter::feishu(test_cache());
        let body = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "ev_form",
                "event_type": "card.action.trigger"
            },
            "event": {
                "operator": {
                    "open_id": "ou_form_user"
                },
                "action": {
                    "tag": "form_submit",
                    "value": {
                        "submission": "confirm",
                        "source": "footer"
                    },
                    "form_value": {
                        "environment": "prod",
                        "reason": "deploy ready"
                    }
                },
                "context": {
                    "open_chat_id": "oc_form_chat",
                    "open_message_id": "om_form_msg"
                }
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        let envelope: serde_json::Value = serde_json::from_str(m.text.as_deref().unwrap()).unwrap();
        assert_eq!(envelope["tag"], "form_submit");
        assert_eq!(envelope["value"]["submission"], "confirm");
        assert_eq!(envelope["value"]["source"], "footer");
        assert_eq!(envelope["form_value"]["environment"], "prod");
        assert_eq!(envelope["form_value"]["reason"], "deploy ready");
        assert_eq!(
            m.raw_data["event"]["action"]["value"]["submission"],
            "confirm"
        );
        assert_eq!(
            m.raw_data["event"]["action"]["form_value"]["environment"],
            "prod"
        );
    }

    #[tokio::test]
    async fn parse_card_action_missing_chat_id_returns_empty() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let body = serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "ev_missing_chat",
                "event_type": "card.action.trigger"
            },
            "event": {
                "operator": {
                    "open_id": "ou_missing"
                },
                "action": {
                    "tag": "button"
                },
                "context": {
                    "open_message_id": "om_missing"
                }
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

    // -- build_message_body --------------------------------------------------

    #[test]
    fn build_body_plain_text() {
        let reply = OutboundReply {
            text: Some("hello".to_string()),
            reply_to_platform_message_id: None,
            metadata: None,
        };
        let (msg_type, content) = build_message_body(&reply);
        assert_eq!(msg_type, "text");
        assert_eq!(content, r#"{"text":"hello"}"#);
    }

    #[test]
    fn build_body_text_missing_defaults_to_empty() {
        let reply = OutboundReply {
            text: None,
            reply_to_platform_message_id: None,
            metadata: None,
        };
        let (msg_type, content) = build_message_body(&reply);
        assert_eq!(msg_type, "text");
        assert_eq!(content, r#"{"text":""}"#);
    }

    #[test]
    fn build_body_interactive_card() {
        let card = serde_json::json!({
            "config": { "update_multi": true },
            "header": {
                "title": { "tag": "plain_text", "content": "Agent Created" },
                "template": "green"
            },
            "elements": [
                { "tag": "markdown", "content": "Your agent is running!" }
            ]
        });
        let reply = OutboundReply {
            text: None,
            reply_to_platform_message_id: None,
            metadata: Some(serde_json::json!({ "card": card.clone() })),
        };
        let (msg_type, content) = build_message_body(&reply);
        assert_eq!(msg_type, "interactive");
        // Content is the card JSON serialized as a string
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed, card);
    }

    #[test]
    fn build_body_card_wins_over_text() {
        let reply = OutboundReply {
            text: Some("ignored fallback".to_string()),
            reply_to_platform_message_id: None,
            metadata: Some(serde_json::json!({ "card": { "elements": [] } })),
        };
        let (msg_type, _) = build_message_body(&reply);
        assert_eq!(msg_type, "interactive");
    }

    #[test]
    fn build_body_metadata_without_card_uses_text() {
        let reply = OutboundReply {
            text: Some("plain".to_string()),
            reply_to_platform_message_id: None,
            metadata: Some(serde_json::json!({ "other": "value" })),
        };
        let (msg_type, content) = build_message_body(&reply);
        assert_eq!(msg_type, "text");
        assert_eq!(content, r#"{"text":"plain"}"#);
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
