//! Lark / Feishu platform adapter for the Channel Bot Relay system.
//!
//! A single [`LarkFamilyAdapter`] struct serves both Lark (international) and
//! Feishu (China mainland) by parameterising the API base URL and platform
//! identifier. The two platforms share the same webhook format, event schema,
//! and REST API shape -- only the hostname differs.
//!
//! Webhook verification follows Lark / Feishu's actual Event Subscription
//! contract: every payload carries a Verification Token, and Encrypt Key is
//! optional. When Encrypt Key is configured, the request body is wrapped as
//! `{"encrypt":"..."}` and the raw request body must be signature-checked
//! before AES-256-CBC decryption, token validation, or event parsing.
//!
//! Message parsing handles the standard `im.message.receive_v1` event schema,
//! interactive card callbacks via `card.action.trigger`, and the
//! `url_verification` challenge flow after verification and optional
//! decryption.
//!
//! Tenant token acquisition goes through the generic
//! [`provider_token_exchange_service`] helpers so the channel adapter and
//! the proxy's `token_exchange` auth method share one in-memory cache with
//! per-key single-flight.

use std::sync::Arc;

use aes::Aes256;
use base64::Engine;
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::models::downstream_service::{CredentialFieldSpec, TokenExchangeConfig};
use crate::services::channel_platform::{
    BotIdentity, InboundMessage, OutboundReply, PlatformAdapter, PlatformVerifySecrets,
    PreparedWebhook,
};
use crate::services::provider_token_exchange_service::{self, TokenExchangeCache};

type Aes256CbcDec = cbc::Decryptor<Aes256>;

/// Lark / Feishu scope keys this adapter relies on for inbound + outbound
/// message handling. Used to pre-select the right checkboxes when we hand
/// the user a permission setup link (see `services::lark_permission`).
///
/// Keep in sync with the Lark/Feishu open APIs this file actually calls:
/// - `im.message.receive_v1` event subscription → `im:message`
/// - `POST /open-apis/im/v1/messages` (text + interactive cards) →
///   `im:message:send_as_bot`
///
/// `card.action.trigger` callbacks ride on the same event subscription as
/// inbound messages, so they don't need their own scope.
pub const REQUIRED_BOT_SCOPES: &[&str] = &["im:message", "im:message:send_as_bot"];

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

    fn prepare_lark_webhook(
        &self,
        bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<PreparedWebhook> {
        let raw_payload = parse_lark_payload(body, "invalid Lark/Feishu webhook JSON")?;
        let configured_encrypt_key = secrets.and_then(|s| s.lark_encrypt_key.as_deref());

        let (effective_body, effective_payload) = if let Some(encrypt_value) =
            extract_encrypt_value(&raw_payload)
        {
            let encrypt_key = configured_encrypt_key.ok_or_else(|| {
                AppError::ChannelWebhookVerificationFailed(
                    "encrypt key not configured for bot".to_string(),
                )
            })?;
            verify_signed_request(headers, encrypt_key, body)?;
            let decrypted_body = decrypt_event_body(encrypt_key, encrypt_value)?;
            let decrypted_payload = parse_lark_payload(
                &decrypted_body,
                "invalid decrypted Lark/Feishu webhook JSON",
            )?;
            (decrypted_body, decrypted_payload)
        } else {
            if configured_encrypt_key.is_some() {
                return Err(AppError::ChannelWebhookVerificationFailed(
                    "encrypt key configured for bot but webhook payload was plaintext".to_string(),
                ));
            }
            (body.to_vec(), raw_payload)
        };

        verify_lark_token(bot, secrets, &effective_payload)?;

        let challenge_response = if is_url_verification(&effective_payload) {
            let challenge = effective_payload
                .get("challenge")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    AppError::ChannelWebhookVerificationFailed(
                        "missing challenge in url_verification payload".to_string(),
                    )
                })?;
            Some(serde_json::json!({ "challenge": challenge }))
        } else {
            None
        };

        Ok(PreparedWebhook {
            body: effective_body,
            challenge_response,
        })
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

// Official references used for this verifier:
// - Feishu Event Subscription security docs:
//   https://open.feishu.cn/document/server-docs/event-subscription-guide/event-subscription-configure-/encrypt-key-encryption-configuration-case?lang=en-US
//   Authenticate the raw request body without decrypting first, then decrypt
//   only after the signature passes.
//   Signature = hex(SHA256(X-Lark-Request-Timestamp + X-Lark-Request-Nonce + encrypt_key + raw_body_bytes)).
// - Same doc for encrypted events:
//   the request body is {"encrypt":"..."}; base64-decode it, use the first 16 bytes as IV,
//   derive the AES-256-CBC key as SHA256(encrypt_key), then remove PKCS7 padding.

fn parse_lark_payload(body: &[u8], context: &str) -> AppResult<serde_json::Value> {
    serde_json::from_slice(body)
        .map_err(|_| AppError::ChannelWebhookVerificationFailed(context.to_string()))
}

fn extract_encrypt_value(payload: &serde_json::Value) -> Option<&str> {
    payload.get("encrypt").and_then(|value| value.as_str())
}

fn extract_verification_token(payload: &serde_json::Value) -> Option<&str> {
    payload
        .get("header")
        .and_then(|header| header.get("token"))
        .and_then(|value| value.as_str())
        .or_else(|| payload.get("token").and_then(|value| value.as_str()))
}

fn is_url_verification(payload: &serde_json::Value) -> bool {
    payload.get("type").and_then(|value| value.as_str()) == Some("url_verification")
}

fn constant_time_eq(expected: &str, actual: &str) -> bool {
    expected.as_bytes().ct_eq(actual.as_bytes()).into()
}

fn verification_token_from_secrets<'a>(
    bot: &ChannelBot,
    secrets: Option<&'a PlatformVerifySecrets>,
) -> AppResult<&'a str> {
    secrets
        .and_then(|value| value.lark_verification_token.as_deref())
        .ok_or_else(|| {
            AppError::ValidationError(format!(
                "{} verification token not configured for bot {}; PATCH /api/v1/channel-bots/{} with verification_token",
                bot.platform, bot.id, bot.id
            ))
        })
}

fn verify_lark_token(
    bot: &ChannelBot,
    secrets: Option<&PlatformVerifySecrets>,
    payload: &serde_json::Value,
) -> AppResult<()> {
    let expected = verification_token_from_secrets(bot, secrets)?;
    let provided = extract_verification_token(payload).ok_or_else(|| {
        AppError::ChannelWebhookVerificationFailed(
            "missing verification token in Lark/Feishu webhook payload".to_string(),
        )
    })?;

    if constant_time_eq(expected, provided) {
        Ok(())
    } else {
        Err(AppError::ChannelWebhookVerificationFailed(
            "Lark/Feishu verification token mismatch".to_string(),
        ))
    }
}

fn verify_signed_request(
    headers: &axum::http::HeaderMap,
    encrypt_key: &str,
    body: &[u8],
) -> AppResult<()> {
    let signature = headers
        .get("x-lark-signature")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            AppError::ChannelWebhookVerificationFailed(
                "missing X-Lark-Signature header".to_string(),
            )
        })?;

    let timestamp = headers
        .get("x-lark-request-timestamp")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            AppError::ChannelWebhookVerificationFailed(
                "missing X-Lark-Request-Timestamp header".to_string(),
            )
        })?;

    let nonce = headers
        .get("x-lark-request-nonce")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            AppError::ChannelWebhookVerificationFailed(
                "missing X-Lark-Request-Nonce header".to_string(),
            )
        })?;

    let mut signed_bytes =
        Vec::with_capacity(timestamp.len() + nonce.len() + encrypt_key.len() + body.len());
    signed_bytes.extend_from_slice(timestamp.as_bytes());
    signed_bytes.extend_from_slice(nonce.as_bytes());
    signed_bytes.extend_from_slice(encrypt_key.as_bytes());
    signed_bytes.extend_from_slice(body);
    let expected = hex::encode(Sha256::digest(&signed_bytes));

    if constant_time_eq(&expected, signature) {
        Ok(())
    } else {
        Err(AppError::ChannelWebhookVerificationFailed(
            "Lark/Feishu signature verification failed".to_string(),
        ))
    }
}

fn decrypt_event_body(encrypt_key: &str, encrypted_body: &str) -> AppResult<Vec<u8>> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encrypted_body)
        .map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "invalid base64 in encrypted Lark/Feishu payload".to_string(),
            )
        })?;

    if decoded.len() <= 16 {
        return Err(AppError::ChannelWebhookVerificationFailed(
            "encrypted Lark/Feishu payload is too short".to_string(),
        ));
    }

    let iv = &decoded[..16];
    let ciphertext = &decoded[16..];
    let key = Sha256::digest(encrypt_key.as_bytes());
    let mut buffer = ciphertext.to_vec();
    let plaintext = Aes256CbcDec::new(&key, iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "failed to decrypt Lark/Feishu payload".to_string(),
            )
        })?;

    Ok(plaintext.to_vec())
}

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

    async fn prepare_webhook(
        &self,
        bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<PreparedWebhook> {
        self.prepare_lark_webhook(bot, secrets, headers, body)
    }

    async fn verify_webhook(
        &self,
        bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<()> {
        self.prepare_lark_webhook(bot, secrets, headers, body)
            .map(|_| ())
    }

    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| AppError::BadRequest(format!("invalid Lark/Feishu webhook JSON: {e}")))?;

        // `url_verification` is answered during webhook preparation after the
        // bot lookup + verification-token check. Be defensive if it still
        // reaches parsing.
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use cbc::cipher::{BlockEncryptMut, block_padding::Pkcs7};

    type Aes256CbcEnc = cbc::Encryptor<Aes256>;

    /// Tests only exercise parsing/signature verification paths, so they
    /// never actually hit the cache. We still need a concrete instance to
    /// pass to the adapter constructors.
    fn test_cache() -> Arc<TokenExchangeCache> {
        Arc::new(TokenExchangeCache::new())
    }

    fn make_test_bot(platform: &str) -> ChannelBot {
        ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: platform.to_string(),
            label: "Test Lark Bot".to_string(),
            bot_token_encrypted: vec![0; 16],
            platform_bot_id: "cli_test".to_string(),
            platform_bot_username: "testbot".to_string(),
            webhook_registered: true,
            webhook_secret_hash: "unused_for_lark".to_string(),
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

    fn make_lark_secrets(
        verification_token: Option<&str>,
        encrypt_key: Option<&str>,
    ) -> PlatformVerifySecrets {
        PlatformVerifySecrets {
            lark_verification_token: verification_token.map(str::to_string),
            lark_encrypt_key: encrypt_key.map(str::to_string),
            ..PlatformVerifySecrets::default()
        }
    }

    fn message_event_body(token: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": "2.0",
            "header": {
                "event_id": "ev_123",
                "event_type": "im.message.receive_v1",
                "create_time": "1700000000",
                "token": token
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
        }))
        .unwrap()
    }

    fn challenge_body(token: &str, challenge: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "type": "url_verification",
            "challenge": challenge,
            "token": token
        }))
        .unwrap()
    }

    fn sign_request(timestamp: &str, nonce: &str, encrypt_key: &str, body: &[u8]) -> String {
        let mut signed =
            Vec::with_capacity(timestamp.len() + nonce.len() + encrypt_key.len() + body.len());
        signed.extend_from_slice(timestamp.as_bytes());
        signed.extend_from_slice(nonce.as_bytes());
        signed.extend_from_slice(encrypt_key.as_bytes());
        signed.extend_from_slice(body);
        hex::encode(Sha256::digest(&signed))
    }

    // Reproducible encrypted fixture recipe:
    // let iv = [0u8, 1, 2, ..., 15];
    // let key = sha256(encrypt_key);
    // body = base64(iv || aes256_cbc_pkcs7_encrypt(key, iv, plaintext_json)).
    fn encrypt_payload(encrypt_key: &str, plaintext: &[u8]) -> String {
        let iv: [u8; 16] = std::array::from_fn(|idx| idx as u8);
        let key = Sha256::digest(encrypt_key.as_bytes());
        let mut buffer = plaintext.to_vec();
        let block_size = 16;
        let pad_len = block_size - (buffer.len() % block_size);
        buffer.resize(buffer.len() + pad_len, 0);
        let ciphertext = Aes256CbcEnc::new(&key, (&iv).into())
            .encrypt_padded_mut::<Pkcs7>(&mut buffer, plaintext.len())
            .unwrap()
            .to_vec();

        let mut combined = iv.to_vec();
        combined.extend_from_slice(&ciphertext);
        base64::engine::general_purpose::STANDARD.encode(combined)
    }

    fn encrypted_request_body(encrypt_key: &str, plaintext: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "encrypt": encrypt_payload(encrypt_key, plaintext)
        }))
        .unwrap()
    }

    fn signed_headers(
        timestamp: &str,
        nonce: &str,
        signature: Option<&str>,
    ) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-lark-request-timestamp", timestamp.parse().unwrap());
        headers.insert("x-lark-request-nonce", nonce.parse().unwrap());
        if let Some(signature) = signature {
            headers.insert("x-lark-signature", signature.parse().unwrap());
        }
        headers
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

    // -- verification / preprocessing ---------------------------------------

    #[tokio::test]
    async fn verify_webhook_plaintext_without_encrypt_key_accepts_correct_token() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot("lark");
        let headers = axum::http::HeaderMap::new();
        let body = message_event_body("verify_token");
        let secrets = make_lark_secrets(Some("verify_token"), None);

        adapter
            .verify_webhook(&bot, Some(&secrets), &headers, &body)
            .await
            .expect("plaintext token verification should pass");
    }

    #[tokio::test]
    async fn verify_webhook_plaintext_without_encrypt_key_rejects_wrong_token() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot("lark");
        let headers = axum::http::HeaderMap::new();
        let body = message_event_body("wrong_token");
        let secrets = make_lark_secrets(Some("verify_token"), None);

        let err = adapter
            .verify_webhook(&bot, Some(&secrets), &headers, &body)
            .await
            .expect_err("wrong token should fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
        assert!(err.to_string().contains("verification token mismatch"));
    }

    #[tokio::test]
    async fn prepare_webhook_encrypted_body_returns_decrypted_bytes() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot("lark");
        let timestamp = "1710000000";
        let nonce = "nonce-123";
        let encrypt_key = "test-encrypt-key";
        let plaintext = message_event_body("verify_token");
        let body = encrypted_request_body(encrypt_key, &plaintext);
        let signature = sign_request(timestamp, nonce, encrypt_key, &body);
        let headers = signed_headers(timestamp, nonce, Some(&signature));
        let secrets = make_lark_secrets(Some("verify_token"), Some(encrypt_key));

        let prepared = adapter
            .prepare_webhook(&bot, Some(&secrets), &headers, &body)
            .await
            .expect("encrypted payload should verify and decrypt");

        assert_eq!(prepared.body, plaintext);
        assert!(prepared.challenge_response.is_none());
    }

    #[tokio::test]
    async fn verify_webhook_encrypted_body_rejects_wrong_decrypted_token() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot("lark");
        let timestamp = "1710000000";
        let nonce = "nonce-123";
        let encrypt_key = "test-encrypt-key";
        let plaintext = message_event_body("wrong_token");
        let body = encrypted_request_body(encrypt_key, &plaintext);
        let signature = sign_request(timestamp, nonce, encrypt_key, &body);
        let headers = signed_headers(timestamp, nonce, Some(&signature));
        let secrets = make_lark_secrets(Some("verify_token"), Some(encrypt_key));

        let err = adapter
            .verify_webhook(&bot, Some(&secrets), &headers, &body)
            .await
            .expect_err("wrong decrypted token should fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
    }

    #[tokio::test]
    async fn verify_webhook_encrypted_body_rejects_invalid_signature() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot("lark");
        let encrypt_key = "test-encrypt-key";
        let plaintext = message_event_body("verify_token");
        let body = encrypted_request_body(encrypt_key, &plaintext);
        let headers = signed_headers("1710000000", "nonce-123", Some("deadbeef"));
        let secrets = make_lark_secrets(Some("verify_token"), Some(encrypt_key));

        let err = adapter
            .verify_webhook(&bot, Some(&secrets), &headers, &body)
            .await
            .expect_err("bad signature should fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
        assert!(err.to_string().contains("signature verification failed"));
    }

    #[tokio::test]
    async fn verify_webhook_rejects_plaintext_when_encrypt_key_is_configured() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot("lark");
        let body = message_event_body("verify_token");
        let secrets = make_lark_secrets(Some("verify_token"), Some("test-encrypt-key"));

        let err = adapter
            .verify_webhook(&bot, Some(&secrets), &axum::http::HeaderMap::new(), &body)
            .await
            .expect_err("plaintext should be rejected when encrypt_key is configured");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
        assert!(err.to_string().contains("payload was plaintext"));
    }

    #[tokio::test]
    async fn prepare_webhook_plaintext_challenge_returns_challenge_response() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot("lark");
        let body = challenge_body("verify_token", "abc123def456");
        let secrets = make_lark_secrets(Some("verify_token"), None);

        let prepared = adapter
            .prepare_webhook(&bot, Some(&secrets), &axum::http::HeaderMap::new(), &body)
            .await
            .expect("challenge should be accepted");

        assert_eq!(
            prepared.challenge_response,
            Some(serde_json::json!({ "challenge": "abc123def456" }))
        );
    }

    #[tokio::test]
    async fn prepare_webhook_plaintext_challenge_rejects_wrong_token() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot("lark");
        let body = challenge_body("wrong_token", "abc123def456");
        let secrets = make_lark_secrets(Some("verify_token"), None);

        let err = adapter
            .prepare_webhook(&bot, Some(&secrets), &axum::http::HeaderMap::new(), &body)
            .await
            .expect_err("wrong challenge token should fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
    }

    #[tokio::test]
    async fn prepare_webhook_encrypted_challenge_returns_challenge_response() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot("lark");
        let timestamp = "1710000000";
        let nonce = "challenge-nonce";
        let encrypt_key = "test-encrypt-key";
        let plaintext = challenge_body("verify_token", "encrypted-challenge");
        let body = encrypted_request_body(encrypt_key, &plaintext);
        let signature = sign_request(timestamp, nonce, encrypt_key, &body);
        let headers = signed_headers(timestamp, nonce, Some(&signature));
        let secrets = make_lark_secrets(Some("verify_token"), Some(encrypt_key));

        let prepared = adapter
            .prepare_webhook(&bot, Some(&secrets), &headers, &body)
            .await
            .expect("encrypted challenge should be accepted");

        assert_eq!(
            prepared.challenge_response,
            Some(serde_json::json!({ "challenge": "encrypted-challenge" }))
        );
    }

    #[tokio::test]
    async fn prepare_webhook_encrypted_challenge_rejects_invalid_signature_before_decrypt() {
        let adapter = LarkFamilyAdapter::lark(test_cache());
        let bot = make_test_bot("lark");
        let encrypt_key = "test-encrypt-key";
        let plaintext = challenge_body("verify_token", "encrypted-challenge");
        let body = encrypted_request_body(encrypt_key, &plaintext);
        let headers = signed_headers("1710000000", "challenge-nonce", Some("deadbeef"));
        let secrets = make_lark_secrets(Some("verify_token"), Some(encrypt_key));

        let err = adapter
            .prepare_webhook(&bot, Some(&secrets), &headers, &body)
            .await
            .expect_err("invalid signature should fail before decrypt");

        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
        assert!(err.to_string().contains("signature verification failed"));
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
