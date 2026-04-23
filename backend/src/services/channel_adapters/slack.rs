//! Slack platform adapter for the Channel Bot Relay system.
//!
//! Implements [`PlatformAdapter`] over the Slack Events API (HTTP mode):
//!
//! * Webhook verification uses HMAC-SHA256 over `v0:{timestamp}:{body}` keyed
//!   on the app's signing secret, with a 5-minute replay window.
//! * `parse_inbound` normalizes `event_callback` envelopes into our
//!   platform-agnostic [`InboundMessage`] shape, focusing on `message` and
//!   `app_mention` events. Bot-authored messages are skipped to break reply
//!   loops.
//! * `send_reply` posts via `chat.postMessage` with the bot's `xoxb-` token,
//!   honoring `thread_ts` for threaded replies and returning the message `ts`.
//! * `register_webhook` is a no-op; Slack apps configure the Request URL in
//!   the App Manifest / Event Subscriptions UI (parity with Discord and
//!   Lark/Feishu).
//! * `verify_bot_token` calls `auth.test` to retrieve the bot user id and
//!   handle.
//! * `handle_challenge` answers Slack's one-time `url_verification` request.
//!
//! Field mapping for [`crate::models::channel_bot::ChannelBot`]:
//!
//! | ChannelBot field            | Slack value                           |
//! | --------------------------- | ------------------------------------- |
//! | `bot_token_encrypted`       | `xoxb-...` bot user token             |
//! | `app_secret_encrypted`      | App **signing secret** (for HMAC)     |
//! | `webhook_secret_hash`       | (unused; populated with raw signing   |
//! |                             | secret at verify time by the webhook  |
//! |                             | handler, mirroring the Lark pattern)  |
//! | `platform_bot_id`           | Bot user id from `auth.test`          |
//! | `platform_bot_username`     | Bot handle from `auth.test`           |

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::services::channel_platform::{
    BotIdentity, InboundAttachment, InboundMessage, OutboundReply, PlatformAdapter,
    PlatformVerifySecrets,
};

type HmacSha256 = Hmac<Sha256>;

const SLACK_API_BASE: &str = "https://slack.com/api";
const SIGNATURE_HEADER: &str = "x-slack-signature";
const TIMESTAMP_HEADER: &str = "x-slack-request-timestamp";
/// Replay window per Slack's recommendation
/// (https://api.slack.com/authentication/verifying-requests-from-slack).
const MAX_TIMESTAMP_SKEW_SECS: i64 = 60 * 5;

/// Slack platform adapter.
///
/// Stateless — all per-bot state (signing secret, bot token, team id) lives
/// on the [`ChannelBot`] document.
pub struct SlackAdapter;

impl SlackAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SlackAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Map a Slack channel id prefix to our normalized conversation type.
///
/// Slack channel ids are prefix-tagged: `C` = public channel, `G` = private
/// group / mpim, `D` = direct message. Anything else is treated as a group
/// to stay consistent with Lark/Feishu fallback behavior.
fn map_conversation_type(channel_id: &str) -> &'static str {
    match channel_id.chars().next() {
        Some('D') => "private",
        Some('C') => "channel",
        Some('G') => "group",
        _ => "group",
    }
}

/// Detect a content type from a Slack `message` event value.
fn detect_content_type(event: &serde_json::Value) -> &'static str {
    if let Some(files) = event.get("files").and_then(|v| v.as_array())
        && let Some(first) = files.first()
    {
        let mimetype = first.get("mimetype").and_then(|v| v.as_str()).unwrap_or("");
        if mimetype.starts_with("image/") {
            return "image";
        }
        if mimetype.starts_with("audio/") {
            return "audio";
        }
        if mimetype.starts_with("video/") {
            return "video";
        }
        return "file";
    }
    if event.get("text").and_then(|v| v.as_str()).is_some() {
        return "text";
    }
    "unknown"
}

/// Extract the `files` array on a Slack message event into our generic
/// [`InboundAttachment`] shape. The `url_private` field requires the bot
/// token to download (parity with Telegram which returns `file_id`s, not
/// pre-signed URLs).
fn extract_attachments(event: &serde_json::Value) -> Vec<InboundAttachment> {
    let Some(files) = event.get("files").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    files
        .iter()
        .map(|file| {
            let mimetype = file.get("mimetype").and_then(|v| v.as_str()).unwrap_or("");
            let category = if mimetype.starts_with("image/") {
                "image"
            } else if mimetype.starts_with("audio/") {
                "audio"
            } else if mimetype.starts_with("video/") {
                "video"
            } else {
                "file"
            };

            InboundAttachment {
                content_type: category.to_string(),
                url: file
                    .get("url_private")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                filename: file.get("name").and_then(|v| v.as_str()).map(String::from),
                mime_type: if mimetype.is_empty() {
                    None
                } else {
                    Some(mimetype.to_string())
                },
                size_bytes: file.get("size").and_then(|v| v.as_u64()),
            }
        })
        .collect()
}

/// Parse Slack's `Retry-After` response header. Slack always sends this as
/// whole seconds (never an HTTP-date), but we defensively bound the result
/// to a sensible range to avoid surprising downstream consumers.
fn parse_retry_after(header: Option<&axum::http::HeaderValue>) -> Option<u64> {
    header
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        // Cap at 1 hour — anything larger is almost certainly misconfigured.
        .map(|s| s.min(3600))
}

/// Build a rate-limit error for `chat.postMessage`. Kept as a helper so both
/// the HTTP-429 path and the `ok:false` JSON path produce consistent messages
/// and so the shape is easy to update later if we add a structured variant.
fn slack_rate_limited(source: &str, retry_after_secs: Option<u64>) -> AppError {
    match retry_after_secs {
        Some(secs) => AppError::ChannelPlatformError(format!(
            "Slack chat.postMessage rate limited ({source}); retry after {secs}s"
        )),
        None => AppError::ChannelPlatformError(format!(
            "Slack chat.postMessage rate limited ({source}); retry later"
        )),
    }
}

/// True if this `message`/`app_mention` event was produced by a bot, in which
/// case we must skip it to avoid reply loops. Slack flags bot-authored
/// messages with `bot_id` and/or `subtype: "bot_message"`.
fn is_bot_event(event: &serde_json::Value) -> bool {
    if event.get("bot_id").is_some() {
        return true;
    }
    matches!(
        event.get("subtype").and_then(|v| v.as_str()),
        Some("bot_message")
    )
}

/// Parse a Slack `event_callback` inner event into an [`InboundMessage`].
fn parse_event(event: &serde_json::Value, raw: serde_json::Value) -> Option<InboundMessage> {
    let event_type = event.get("type").and_then(|v| v.as_str())?;

    // Only chat-bound event types we care about. Other types (`reaction_added`,
    // `channel_created`, etc.) are ignored at this layer.
    if !matches!(event_type, "message" | "app_mention") {
        return None;
    }

    if is_bot_event(event) {
        return None;
    }

    // Skip non-routable message subtypes (edits, deletes, joins, leaves).
    // We deliberately allow `file_share` and the (default) absence of
    // `subtype`, which are real user messages.
    if let Some(subtype) = event.get("subtype").and_then(|v| v.as_str())
        && !matches!(subtype, "file_share" | "thread_broadcast")
    {
        return None;
    }

    let channel = event.get("channel").and_then(|v| v.as_str())?;
    let ts = event.get("ts").and_then(|v| v.as_str())?;

    let user = event
        .get("user")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let text = event.get("text").and_then(|v| v.as_str()).map(String::from);

    let thread_ts = event
        .get("thread_ts")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Slack threads everything off the parent's `ts`. If `thread_ts` exists
    // and differs from `ts`, this message is a reply within the thread.
    let reply_to = thread_ts
        .as_ref()
        .and_then(|t| if t == ts { None } else { Some(t.clone()) });

    let attachments = extract_attachments(event);

    Some(InboundMessage {
        platform_message_id: ts.to_string(),
        conversation_id: channel.to_string(),
        conversation_type: map_conversation_type(channel).to_string(),
        sender_platform_id: user,
        sender_display_name: None,
        content_type: detect_content_type(event).to_string(),
        text,
        attachments,
        reply_to_platform_message_id: reply_to,
        thread_id: thread_ts,
        raw_data: raw,
    })
}

// ---------------------------------------------------------------------------
// PlatformAdapter implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl PlatformAdapter for SlackAdapter {
    fn platform_id(&self) -> &str {
        "slack"
    }

    async fn verify_webhook(
        &self,
        _bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<()> {
        let signature = headers
            .get(SIGNATURE_HEADER)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError::ChannelWebhookVerificationFailed(
                    "missing X-Slack-Signature header".to_string(),
                )
            })?;

        let timestamp_str = headers
            .get(TIMESTAMP_HEADER)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError::ChannelWebhookVerificationFailed(
                    "missing X-Slack-Request-Timestamp header".to_string(),
                )
            })?;

        let timestamp: i64 = timestamp_str.parse().map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "invalid X-Slack-Request-Timestamp header".to_string(),
            )
        })?;

        // Replay protection: reject anything older than the configured skew.
        let now = chrono::Utc::now().timestamp();
        if (now - timestamp).abs() > MAX_TIMESTAMP_SKEW_SECS {
            return Err(AppError::ChannelWebhookVerificationFailed(
                "Slack request timestamp outside replay window".to_string(),
            ));
        }

        let signing_secret = secrets
            .and_then(|s| s.slack_signing_secret.as_deref())
            .unwrap_or_default();
        if signing_secret.is_empty() {
            return Err(AppError::ChannelWebhookVerificationFailed(
                "Slack signing secret not configured".to_string(),
            ));
        }

        let body_str = std::str::from_utf8(body).map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "Slack webhook body is not valid UTF-8".to_string(),
            )
        })?;
        let base_string = format!("v0:{timestamp}:{body_str}");

        let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes()).map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "failed to initialize Slack HMAC verifier".to_string(),
            )
        })?;
        mac.update(base_string.as_bytes());
        let expected = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

        if expected.as_bytes().ct_eq(signature.as_bytes()).into() {
            Ok(())
        } else {
            Err(AppError::ChannelWebhookVerificationFailed(
                "Slack signature verification failed".to_string(),
            ))
        }
    }

    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| AppError::BadRequest(format!("invalid Slack webhook JSON: {e}")))?;

        // url_verification is handled by `handle_challenge` upstream; defensive
        // skip here if it ever lands.
        if payload.get("type").and_then(|v| v.as_str()) == Some("url_verification") {
            return Ok(Vec::new());
        }

        if payload.get("type").and_then(|v| v.as_str()) != Some("event_callback") {
            return Ok(Vec::new());
        }

        let Some(event) = payload.get("event") else {
            return Ok(Vec::new());
        };

        match parse_event(event, payload.clone()) {
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
        let text = reply.text.as_deref().unwrap_or("");

        let mut body = serde_json::json!({
            "channel": conversation_id,
            "text": text,
        });

        // Thread the reply correctly. Slack expects `thread_ts` to be the
        // ROOT message's `ts`. The relay layer surfaces the inbound event's
        // root in `metadata.thread_ts`, so prefer that. Fall back to
        // `reply_to_platform_message_id` only when the agent is explicitly
        // replying to a specific message (Slack will auto-resolve to the
        // parent thread, but the explicit root anchor is more reliable).
        let thread_ts = reply
            .metadata
            .as_ref()
            .and_then(|m| m.get("thread_ts"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| reply.reply_to_platform_message_id.clone());
        if let Some(ts) = thread_ts {
            body["thread_ts"] = serde_json::json!(ts);
        }

        // Optional Block Kit passthrough — agents that want richer payloads
        // can set `metadata.blocks`. The array is forwarded as-is; Slack
        // validates server-side.
        if let Some(metadata) = reply.metadata.as_ref()
            && let Some(blocks) = metadata.get("blocks")
        {
            body["blocks"] = blocks.clone();
        }

        let url = format!("{SLACK_API_BASE}/chat.postMessage");
        let response = http
            .post(&url)
            .header("Authorization", format!("Bearer {bot_token}"))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Slack chat.postMessage request failed: {e}"
                ))
            })?;

        // Rate-limit signal #1: HTTP 429 with a Retry-After header.
        // https://api.slack.com/docs/rate-limits
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = parse_retry_after(response.headers().get("retry-after"));
            return Err(slack_rate_limited("HTTP 429", retry_after));
        }

        let resp: serde_json::Value = response.json().await.map_err(|e| {
            AppError::ChannelPlatformError(format!(
                "Slack chat.postMessage response parse failed: {e}"
            ))
        })?;

        if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let error = resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");

            // Rate-limit signal #2: Slack can also return `{"ok":false,
            // "error":"ratelimited"}` with HTTP 200 on some endpoints.
            // `response_metadata.messages` may carry a human-readable hint.
            if error == "ratelimited" || error == "rate_limited" {
                return Err(slack_rate_limited(error, None));
            }

            return Err(AppError::ChannelPlatformError(format!(
                "Slack chat.postMessage failed: {error}"
            )));
        }

        let message_id = resp.get("ts").and_then(|v| v.as_str()).map(String::from);
        Ok(message_id)
    }

    async fn register_webhook(
        &self,
        _http: &reqwest::Client,
        _bot_token: &str,
        _webhook_url: &str,
        _secret: &str,
    ) -> AppResult<()> {
        // Slack Event Subscriptions URLs are configured via the App Manifest
        // / Slack App settings UI, not via API. The Channel Bot service marks
        // the bot `pending_webhook` and the first verified inbound webhook
        // promotes it to `active` — same flow as Discord and Lark/Feishu.
        Ok(())
    }

    async fn verify_bot_token(
        &self,
        http: &reqwest::Client,
        bot_token: &str,
    ) -> AppResult<BotIdentity> {
        let url = format!("{SLACK_API_BASE}/auth.test");
        let resp: serde_json::Value = http
            .post(&url)
            .header("Authorization", format!("Bearer {bot_token}"))
            .send()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!("Slack auth.test request failed: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Slack auth.test response parse failed: {e}"
                ))
            })?;

        if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let error = resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("invalid_auth");
            return Err(AppError::ChannelPlatformError(format!(
                "Slack auth.test failed: {error}"
            )));
        }

        // Prefer `bot_id` (Bxxxxx) for chat bots; fall back to `user_id`
        // when called against a user token.
        let bot_id = resp
            .get("bot_id")
            .and_then(|v| v.as_str())
            .or_else(|| resp.get("user_id").and_then(|v| v.as_str()))
            .ok_or_else(|| {
                AppError::ChannelPlatformError(
                    "Slack auth.test response missing bot_id/user_id".to_string(),
                )
            })?;

        let username = resp
            .get("user")
            .and_then(|v| v.as_str())
            .unwrap_or("slack_bot");

        Ok(BotIdentity {
            platform_bot_id: bot_id.to_string(),
            platform_bot_username: username.to_string(),
        })
    }

    fn handle_challenge(&self, body: &[u8]) -> Option<serde_json::Value> {
        let payload: serde_json::Value = serde_json::from_slice(body).ok()?;
        if payload.get("type").and_then(|v| v.as_str()) != Some("url_verification") {
            return None;
        }
        let challenge = payload.get("challenge")?.as_str()?;
        Some(serde_json::json!({ "challenge": challenge }))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_bot(_signing_secret: &str) -> ChannelBot {
        ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "slack".to_string(),
            label: "Test Slack Bot".to_string(),
            bot_token_encrypted: vec![0; 16],
            platform_bot_id: "B12345".to_string(),
            platform_bot_username: "testbot".to_string(),
            webhook_registered: true,
            webhook_secret_hash: "unused_for_slack".to_string(),
            app_id: None,
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

    fn sign(secret: &str, timestamp: i64, body: &[u8]) -> String {
        let body_str = std::str::from_utf8(body).unwrap();
        let base = format!("v0:{timestamp}:{body_str}");
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(base.as_bytes());
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    // -- platform_id ---------------------------------------------------------

    #[test]
    fn platform_id_is_slack() {
        let adapter = SlackAdapter::new();
        assert_eq!(adapter.platform_id(), "slack");
    }

    // -- handle_challenge ----------------------------------------------------

    #[test]
    fn handle_challenge_url_verification() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({
            "token": "deprecated",
            "type": "url_verification",
            "challenge": "abc123"
        });
        let result = adapter.handle_challenge(serde_json::to_vec(&body).unwrap().as_slice());
        let resp = result.expect("expected challenge response");
        assert_eq!(resp["challenge"], "abc123");
    }

    #[test]
    fn handle_challenge_event_callback_returns_none() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({
            "type": "event_callback",
            "event": { "type": "message" }
        });
        let result = adapter.handle_challenge(serde_json::to_vec(&body).unwrap().as_slice());
        assert!(result.is_none());
    }

    #[test]
    fn handle_challenge_invalid_json_returns_none() {
        let adapter = SlackAdapter::new();
        assert!(adapter.handle_challenge(b"not json").is_none());
    }

    // -- verify_webhook ------------------------------------------------------

    #[tokio::test]
    async fn verify_webhook_valid_signature() {
        let adapter = SlackAdapter::new();
        let secret = "8f742231b10e8888abcd99yyyzzz85a5";
        let body = br#"{"type":"event_callback"}"#;
        let ts = chrono::Utc::now().timestamp();
        let bot = make_test_bot(secret);
        let secrets = PlatformVerifySecrets {
            slack_signing_secret: Some(secret.to_string()),
            ..PlatformVerifySecrets::default()
        };

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(SIGNATURE_HEADER, sign(secret, ts, body).parse().unwrap());
        headers.insert(TIMESTAMP_HEADER, ts.to_string().parse().unwrap());

        adapter
            .verify_webhook(&bot, Some(&secrets), &headers, body)
            .await
            .expect("valid signature should pass");
    }

    #[tokio::test]
    async fn verify_webhook_invalid_signature() {
        let adapter = SlackAdapter::new();
        let body = br#"{"type":"event_callback"}"#;
        let ts = chrono::Utc::now().timestamp();
        let bot = make_test_bot("real_secret");
        let secrets = PlatformVerifySecrets {
            slack_signing_secret: Some("real_secret".to_string()),
            ..PlatformVerifySecrets::default()
        };

        let mut headers = axum::http::HeaderMap::new();
        // Sign with a different secret -> mismatch.
        headers.insert(
            SIGNATURE_HEADER,
            sign("wrong_secret", ts, body).parse().unwrap(),
        );
        headers.insert(TIMESTAMP_HEADER, ts.to_string().parse().unwrap());

        let err = adapter
            .verify_webhook(&bot, Some(&secrets), &headers, body)
            .await
            .expect_err("mismatched signature should fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
    }

    #[tokio::test]
    async fn verify_webhook_missing_signature_header() {
        let adapter = SlackAdapter::new();
        let bot = make_test_bot("secret");
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(TIMESTAMP_HEADER, "1700000000".parse().unwrap());
        let err = adapter
            .verify_webhook(
                &bot,
                Some(&PlatformVerifySecrets {
                    slack_signing_secret: Some("secret".to_string()),
                    ..PlatformVerifySecrets::default()
                }),
                &headers,
                b"{}",
            )
            .await
            .expect_err("missing signature should fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
    }

    #[tokio::test]
    async fn verify_webhook_missing_timestamp_header() {
        let adapter = SlackAdapter::new();
        let bot = make_test_bot("secret");
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(SIGNATURE_HEADER, "v0=abc".parse().unwrap());
        let err = adapter
            .verify_webhook(
                &bot,
                Some(&PlatformVerifySecrets {
                    slack_signing_secret: Some("secret".to_string()),
                    ..PlatformVerifySecrets::default()
                }),
                &headers,
                b"{}",
            )
            .await
            .expect_err("missing timestamp should fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
    }

    #[tokio::test]
    async fn verify_webhook_stale_timestamp() {
        let adapter = SlackAdapter::new();
        let secret = "secret";
        let body = br#"{"type":"event_callback"}"#;
        // 10 minutes in the past, beyond the 5-minute replay window.
        let ts = chrono::Utc::now().timestamp() - 600;
        let bot = make_test_bot(secret);
        let secrets = PlatformVerifySecrets {
            slack_signing_secret: Some(secret.to_string()),
            ..PlatformVerifySecrets::default()
        };

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(SIGNATURE_HEADER, sign(secret, ts, body).parse().unwrap());
        headers.insert(TIMESTAMP_HEADER, ts.to_string().parse().unwrap());

        let err = adapter
            .verify_webhook(&bot, Some(&secrets), &headers, body)
            .await
            .expect_err("stale timestamp should fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
    }

    #[tokio::test]
    async fn verify_webhook_missing_signing_secret() {
        let adapter = SlackAdapter::new();
        let body = b"{}";
        let ts = chrono::Utc::now().timestamp();
        // Bot without a configured signing secret (empty string).
        let bot = make_test_bot("");

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(SIGNATURE_HEADER, "v0=00".parse().unwrap());
        headers.insert(TIMESTAMP_HEADER, ts.to_string().parse().unwrap());

        let err = adapter
            .verify_webhook(&bot, None, &headers, body)
            .await
            .expect_err("missing signing secret should fail");
        assert!(matches!(err, AppError::ChannelWebhookVerificationFailed(_)));
    }

    // -- parse_inbound -------------------------------------------------------

    #[tokio::test]
    async fn parse_message_text() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({
            "type": "event_callback",
            "team_id": "T1",
            "api_app_id": "A1",
            "event": {
                "type": "message",
                "channel": "C123",
                "user": "U999",
                "text": "Hello bot",
                "ts": "1700000000.000100"
            },
            "event_id": "Ev1",
            "event_time": 1700000000
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.platform_message_id, "1700000000.000100");
        assert_eq!(m.conversation_id, "C123");
        assert_eq!(m.conversation_type, "channel");
        assert_eq!(m.sender_platform_id, "U999");
        assert_eq!(m.content_type, "text");
        assert_eq!(m.text.as_deref(), Some("Hello bot"));
        assert!(m.thread_id.is_none());
        assert!(m.reply_to_platform_message_id.is_none());
    }

    #[tokio::test]
    async fn parse_app_mention() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({
            "type": "event_callback",
            "event": {
                "type": "app_mention",
                "channel": "C42",
                "user": "U42",
                "text": "<@U_BOT> hello",
                "ts": "1700000001.000200"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text.as_deref(), Some("<@U_BOT> hello"));
    }

    #[tokio::test]
    async fn parse_thread_reply() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel": "C7",
                "user": "U7",
                "text": "in-thread reply",
                "ts": "1700000010.000400",
                "thread_ts": "1700000005.000300"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.thread_id.as_deref(), Some("1700000005.000300"));
        assert_eq!(
            m.reply_to_platform_message_id.as_deref(),
            Some("1700000005.000300")
        );
    }

    #[tokio::test]
    async fn parse_thread_root_no_reply_to() {
        let adapter = SlackAdapter::new();
        // Slack sends `thread_ts == ts` for the parent message of an
        // existing thread; we should set thread_id but not reply_to.
        let body = serde_json::json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel": "C7",
                "user": "U7",
                "text": "thread root",
                "ts": "1700000020.000500",
                "thread_ts": "1700000020.000500"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        let m = &msgs[0];
        assert_eq!(m.thread_id.as_deref(), Some("1700000020.000500"));
        assert!(m.reply_to_platform_message_id.is_none());
    }

    #[tokio::test]
    async fn parse_skips_bot_message() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "subtype": "bot_message",
                "channel": "C1",
                "bot_id": "B1",
                "text": "from another bot",
                "ts": "1700000030.000000"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(
            msgs.is_empty(),
            "bot messages must be ignored to avoid loops"
        );
    }

    #[tokio::test]
    async fn parse_skips_message_with_bot_id_only() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel": "C1",
                "bot_id": "B1",
                "text": "no subtype but bot_id present",
                "ts": "1700000031.000000"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_skips_edits_and_deletes() {
        let adapter = SlackAdapter::new();
        for subtype in ["message_changed", "message_deleted", "channel_join"] {
            let body = serde_json::json!({
                "type": "event_callback",
                "event": {
                    "type": "message",
                    "subtype": subtype,
                    "channel": "C1",
                    "user": "U1",
                    "text": "ignored",
                    "ts": "1700000040.000000"
                }
            });
            let raw = serde_json::to_vec(&body).unwrap();
            let msgs = adapter.parse_inbound(&raw).await.unwrap();
            assert!(msgs.is_empty(), "subtype {subtype} must be ignored");
        }
    }

    #[tokio::test]
    async fn parse_file_share_with_image() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "subtype": "file_share",
                "channel": "C1",
                "user": "U1",
                "text": "look at this",
                "ts": "1700000050.000000",
                "files": [
                    {
                        "id": "F1",
                        "name": "screenshot.png",
                        "mimetype": "image/png",
                        "url_private": "https://files.slack.com/private/screenshot.png",
                        "size": 12345
                    }
                ]
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.content_type, "image");
        assert_eq!(m.attachments.len(), 1);
        assert_eq!(m.attachments[0].content_type, "image");
        assert_eq!(m.attachments[0].filename.as_deref(), Some("screenshot.png"));
        assert_eq!(m.attachments[0].mime_type.as_deref(), Some("image/png"));
        assert_eq!(m.attachments[0].size_bytes, Some(12345));
        assert_eq!(
            m.attachments[0].url,
            "https://files.slack.com/private/screenshot.png"
        );
    }

    #[tokio::test]
    async fn parse_url_verification_returns_empty() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({
            "type": "url_verification",
            "challenge": "abc"
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_non_event_callback_returns_empty() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({ "type": "block_actions" });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_invalid_json_returns_error() {
        let adapter = SlackAdapter::new();
        let result = adapter.parse_inbound(b"not json").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn parse_dm_conversation_type() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel": "D123",
                "user": "U1",
                "text": "dm",
                "ts": "1700000060.000000"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].conversation_type, "private");
    }

    #[tokio::test]
    async fn parse_group_conversation_type() {
        let adapter = SlackAdapter::new();
        let body = serde_json::json!({
            "type": "event_callback",
            "event": {
                "type": "message",
                "channel": "G555",
                "user": "U1",
                "text": "group",
                "ts": "1700000061.000000"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert_eq!(msgs[0].conversation_type, "group");
    }

    // -- conversation type / content type ------------------------------------

    #[test]
    fn conversation_type_mapping() {
        assert_eq!(map_conversation_type("D123"), "private");
        assert_eq!(map_conversation_type("C123"), "channel");
        assert_eq!(map_conversation_type("G123"), "group");
        assert_eq!(map_conversation_type("Q123"), "group");
        assert_eq!(map_conversation_type(""), "group");
    }

    #[test]
    fn content_type_detection() {
        assert_eq!(
            detect_content_type(&serde_json::json!({ "text": "hi" })),
            "text"
        );
        assert_eq!(detect_content_type(&serde_json::json!({})), "unknown");
        assert_eq!(
            detect_content_type(&serde_json::json!({
                "files": [{ "mimetype": "image/png" }]
            })),
            "image"
        );
        assert_eq!(
            detect_content_type(&serde_json::json!({
                "files": [{ "mimetype": "audio/mp3" }]
            })),
            "audio"
        );
        assert_eq!(
            detect_content_type(&serde_json::json!({
                "files": [{ "mimetype": "video/mp4" }]
            })),
            "video"
        );
        assert_eq!(
            detect_content_type(&serde_json::json!({
                "files": [{ "mimetype": "application/pdf" }]
            })),
            "file"
        );
    }

    // -- send_reply thread_ts selection --------------------------------------
    //
    // These tests pin the priority order used to populate Slack's `thread_ts`
    // field. The relay surfaces the inbound thread root in
    // `metadata.thread_ts`, so it must win over `reply_to_platform_message_id`
    // (which can carry a child reply's `ts`, not the thread root).

    /// Build the same JSON body that `send_reply` sends to chat.postMessage,
    /// without going over the network. Mirrors the priority logic exactly so
    /// regressions in the helper show up here.
    fn build_post_message_body(reply: &OutboundReply, channel: &str) -> serde_json::Value {
        let text = reply.text.as_deref().unwrap_or("");
        let mut body = serde_json::json!({ "channel": channel, "text": text });

        let thread_ts = reply
            .metadata
            .as_ref()
            .and_then(|m| m.get("thread_ts"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| reply.reply_to_platform_message_id.clone());
        if let Some(ts) = thread_ts {
            body["thread_ts"] = serde_json::json!(ts);
        }

        if let Some(metadata) = reply.metadata.as_ref()
            && let Some(blocks) = metadata.get("blocks")
        {
            body["blocks"] = blocks.clone();
        }

        body
    }

    #[test]
    fn send_reply_prefers_metadata_thread_ts_over_message_id() {
        // Inbound was a reply *inside* a thread: the relay sets
        // `metadata.thread_ts` to the root and `reply_to_platform_message_id`
        // to the child's `ts`. Slack must thread off the root, not the child.
        let reply = OutboundReply {
            text: Some("answer".to_string()),
            reply_to_platform_message_id: Some("1700000010.000400".to_string()),
            metadata: Some(serde_json::json!({ "thread_ts": "1700000005.000300" })),
        };
        let body = build_post_message_body(&reply, "C1");
        assert_eq!(body["thread_ts"], "1700000005.000300");
    }

    #[test]
    fn send_reply_falls_back_to_message_id_when_no_metadata() {
        // No metadata.thread_ts (e.g. agent ignored callback fields). Use the
        // explicit reply target so Slack at least attaches the reply to the
        // same parent thread.
        let reply = OutboundReply {
            text: Some("answer".to_string()),
            reply_to_platform_message_id: Some("1700000010.000400".to_string()),
            metadata: None,
        };
        let body = build_post_message_body(&reply, "C1");
        assert_eq!(body["thread_ts"], "1700000010.000400");
    }

    #[test]
    fn send_reply_omits_thread_ts_for_top_level_reply() {
        let reply = OutboundReply {
            text: Some("hi".to_string()),
            reply_to_platform_message_id: None,
            metadata: None,
        };
        let body = build_post_message_body(&reply, "C1");
        assert!(body.get("thread_ts").is_none());
    }

    #[test]
    fn send_reply_passes_through_blocks_metadata() {
        let blocks = serde_json::json!([{ "type": "section", "text": { "type": "mrkdwn", "text": "*hi*" } }]);
        let reply = OutboundReply {
            text: Some("fallback".to_string()),
            reply_to_platform_message_id: None,
            metadata: Some(serde_json::json!({ "blocks": blocks.clone() })),
        };
        let body = build_post_message_body(&reply, "C1");
        assert_eq!(body["blocks"], blocks);
    }

    // -- rate limit handling -------------------------------------------------
    //
    // Slack signals backpressure two different ways on chat.postMessage:
    //   1. HTTP 429 with a `Retry-After: <seconds>` header.
    //   2. HTTP 200 with `{"ok": false, "error": "ratelimited"}`.
    // Both must surface as a clearly-labeled error so the relay / agent can
    // reason about retry timing. The adapter does NOT auto-retry — that's an
    // agent-level decision since Slack replies are time-sensitive.

    #[test]
    fn parse_retry_after_reads_seconds() {
        let hv = axum::http::HeaderValue::from_static("7");
        assert_eq!(parse_retry_after(Some(&hv)), Some(7));
    }

    #[test]
    fn parse_retry_after_handles_missing_header() {
        assert_eq!(parse_retry_after(None), None);
    }

    #[test]
    fn parse_retry_after_rejects_non_numeric() {
        // Slack docs specify integer seconds; HTTP-date form isn't used.
        let hv = axum::http::HeaderValue::from_static("Tue, 15 Nov 1994 08:12:31 GMT");
        assert_eq!(parse_retry_after(Some(&hv)), None);
    }

    #[test]
    fn parse_retry_after_caps_absurd_values() {
        let hv = axum::http::HeaderValue::from_static("99999");
        assert_eq!(parse_retry_after(Some(&hv)), Some(3600));
    }

    #[test]
    fn slack_rate_limited_includes_retry_window() {
        let err = slack_rate_limited("HTTP 429", Some(30));
        match err {
            AppError::ChannelPlatformError(msg) => {
                assert!(msg.contains("rate limited"), "msg was: {msg}");
                assert!(msg.contains("30s"), "msg was: {msg}");
                assert!(msg.contains("HTTP 429"), "msg was: {msg}");
            }
            other => panic!("expected ChannelPlatformError, got {other:?}"),
        }
    }

    #[test]
    fn slack_rate_limited_without_retry_window() {
        let err = slack_rate_limited("ratelimited", None);
        match err {
            AppError::ChannelPlatformError(msg) => {
                assert!(msg.contains("rate limited"));
                assert!(msg.contains("retry later"));
                assert!(msg.contains("ratelimited"));
            }
            other => panic!("expected ChannelPlatformError, got {other:?}"),
        }
    }
}
