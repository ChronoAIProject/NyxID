//! Discord platform adapter for the Channel Bot Relay system.
//!
//! Implements [`PlatformAdapter`] to normalize Discord Interactions (slash
//! commands, message components) and Gateway message events into the
//! platform-agnostic [`InboundMessage`] format. Replies are sent via the
//! Discord REST API (`POST /channels/{id}/messages`).
//!
//! Webhook verification uses Ed25519 signature validation per Discord's
//! Interactions endpoint requirements.

const UNREACHABLE_TARGET_MARKERS: &[&str] = &[
    "Missing Access",
    "Missing Permissions",
    "Unknown Channel",
    "Cannot send messages to this user",
    "Unknown Message",
    "Cannot edit a message authored by another user",
];

use crate::services::channel_media_service as media;
use crate::services::channel_platform::{FetchedMedia, MediaCapabilities};

use ed25519_dalek::{Signature, VerifyingKey};

use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::services::channel_platform::{
    BotIdentity, InboundAttachment, InboundMessage, OutboundEdit, OutboundReply, PlatformAdapter,
};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

/// Discord Interaction types.
const INTERACTION_PING: u64 = 1;
const INTERACTION_APPLICATION_COMMAND: u64 = 2;
const INTERACTION_MESSAGE_COMPONENT: u64 = 4;

/// Discord channel types.
const CHANNEL_DM: u64 = 1;
const CHANNEL_GROUP_DM: u64 = 3;

/// Discord platform adapter.
///
/// Stateless -- all state lives in the [`ChannelBot`] document and the Discord
/// API itself.
/// NyxID always edits through `PATCH /channels/{channel_id}/messages/{message_id}`
/// with the bot token and never through the interaction-webhook edit endpoint,
/// so edits that Discord only permits via the interaction token (for example
/// ephemeral interaction responses) are not supported and surface as a classified refusal.
pub struct DiscordAdapter {
    base_url: String,
}

impl Default for DiscordAdapter {
    fn default() -> Self {
        Self {
            base_url: DISCORD_API_BASE.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Map Discord channel type integer to our normalized conversation type.
fn map_conversation_type(channel_type: Option<u64>) -> &'static str {
    match channel_type {
        Some(CHANNEL_DM) => "private",
        Some(CHANNEL_GROUP_DM) => "group",
        // Guild text (0), announcement (5), forum (15), etc. are all "group"
        Some(0 | 5 | 10 | 11 | 12 | 15) => "group",
        // Default to group for unknown guild channel types
        _ => "group",
    }
}

/// Parse sender information from a Discord interaction or message payload.
fn extract_sender(payload: &serde_json::Value) -> (String, Option<String>) {
    // Interaction: member.user.id or user.id
    if let Some(member) = payload.get("member")
        && let Some(user) = member.get("user")
    {
        let id = user
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let name = user
            .get("username")
            .and_then(|v| v.as_str())
            .map(String::from);
        return (id, name);
    }

    // DM interaction: user.id
    if let Some(user) = payload.get("user") {
        let id = user
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let name = user
            .get("username")
            .and_then(|v| v.as_str())
            .map(String::from);
        return (id, name);
    }

    // Gateway message: author.id
    if let Some(author) = payload.get("author") {
        let id = author
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let name = author
            .get("username")
            .and_then(|v| v.as_str())
            .map(String::from);
        return (id, name);
    }

    (String::new(), None)
}

fn extract_attachments(payload: &serde_json::Value) -> Vec<InboundAttachment> {
    let files: Vec<_> = if let Some(files) = payload.get("attachments").and_then(|v| v.as_array()) {
        files.iter().collect()
    } else {
        payload
            .pointer("/data/resolved/attachments")
            .and_then(|v| v.as_object())
            .map(|files| files.values().collect())
            .unwrap_or_default()
    };
    files
        .into_iter()
        .filter_map(|file| {
            let mime = file["content_type"].as_str().unwrap_or("");
            let kind = if mime.starts_with("image/") {
                "image"
            } else if mime.starts_with("audio/") {
                "audio"
            } else if mime.starts_with("video/") {
                "video"
            } else {
                "file"
            };
            Some(InboundAttachment {
                content_type: kind.into(),
                url: file["url"].as_str()?.into(),
                platform_message_id: payload["id"].as_str().map(str::to_string),
                file_key: file["id"].as_str().map(str::to_string),
                image_key: None,
                filename: file["filename"].as_str().map(str::to_string),
                mime_type: file["content_type"].as_str().map(str::to_string),
                size_bytes: file["size"].as_u64(),
            })
        })
        .collect()
}

/// Parse an APPLICATION_COMMAND or MESSAGE_COMPONENT interaction into an
/// [`InboundMessage`].
fn parse_interaction(payload: &serde_json::Value) -> Option<InboundMessage> {
    let interaction_id = payload.get("id")?.as_str()?;
    let channel_id = payload.get("channel_id").and_then(|v| v.as_str())?;
    let channel_type = payload
        .get("channel")
        .and_then(|c| c.get("type"))
        .and_then(|v| v.as_u64());

    let (sender_id, sender_name) = extract_sender(payload);

    // Extract text content from interaction data
    let text = payload.get("data").and_then(|d| {
        // Slash command: concatenate options as text representation
        if let Some(name) = d.get("name").and_then(|v| v.as_str()) {
            let opts = d
                .get("options")
                .and_then(|o| o.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|opt| {
                            let k = opt.get("name")?.as_str()?;
                            let v = opt.get("value")?;
                            let v_str = match v.as_str() {
                                Some(s) => s.to_string(),
                                None => v.to_string(),
                            };
                            Some(format!("{k}={v_str}"))
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            if opts.is_empty() {
                return Some(format!("/{name}"));
            }
            return Some(format!("/{name} {opts}"));
        }
        // Button / select menu: custom_id
        d.get("custom_id")
            .and_then(|v| v.as_str())
            .map(String::from)
    });

    // Preserve the interaction token for follow-up replies (deferred interactions).
    // Stored as "interaction:{application_id}:{token}" in thread_id.
    let interaction_token = payload.get("token").and_then(|v| v.as_str());
    let application_id = payload.get("application_id").and_then(|v| v.as_str());
    let thread_id = match (application_id, interaction_token) {
        (Some(app_id), Some(token)) => Some(format!("interaction:{app_id}:{token}")),
        _ => None,
    };

    Some(InboundMessage {
        platform_message_id: interaction_id.to_string(),
        conversation_id: channel_id.to_string(),
        conversation_type: map_conversation_type(channel_type).to_string(),
        sender_platform_id: sender_id,
        sender_display_name: sender_name,
        content_type: "text".to_string(),
        text,
        attachments: extract_attachments(payload),
        reply_to_platform_message_id: None,
        thread_id,
        raw_data: payload.clone(),
    })
}

/// Parse a Gateway-style message object into an [`InboundMessage`].
fn parse_gateway_message(payload: &serde_json::Value) -> Option<InboundMessage> {
    let msg = payload.get("d").unwrap_or(payload);
    let message_id = msg.get("id")?.as_str()?;
    let channel_id = msg.get("channel_id")?.as_str()?;

    let (sender_id, sender_name) = extract_sender(msg);

    let text = msg
        .get("content")
        .and_then(|v| v.as_str())
        .map(String::from);

    let reply_to = msg
        .get("message_reference")
        .and_then(|r| r.get("message_id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let thread_id = msg
        .get("thread")
        .and_then(|t| t.get("id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let attachments = extract_attachments(msg);
    Some(InboundMessage {
        platform_message_id: message_id.to_string(),
        conversation_id: channel_id.to_string(),
        conversation_type: "group".to_string(),
        sender_platform_id: sender_id,
        sender_display_name: sender_name,
        content_type: attachments
            .first()
            .map(|a| a.content_type.clone())
            .unwrap_or_else(|| if text.is_some() { "text" } else { "unknown" }.into()),
        text,
        attachments,
        reply_to_platform_message_id: reply_to,
        thread_id,
        raw_data: payload.clone(),
    })
}

// ---------------------------------------------------------------------------
// PlatformAdapter implementation
// ---------------------------------------------------------------------------

/// Legacy relay rows can carry an interaction marker independently of the
/// conversation platform. Preserve that routing before platform thread keys.
pub(crate) fn apply_interaction_context(
    thread_id: Option<&str>,
    created_at: chrono::DateTime<chrono::Utc>,
    metadata: &mut Option<serde_json::Value>,
) -> bool {
    let Some(thread_id) = thread_id.filter(|id| id.starts_with("interaction:")) else {
        return false;
    };
    let age = chrono::Utc::now() - created_at;
    if age < chrono::Duration::minutes(14) {
        super::super::channel_platform::insert_reply_context(
            metadata,
            "interaction_thread_id",
            thread_id,
        );
    } else {
        tracing::info!(
            age_secs = age.num_seconds(),
            "Skipping Discord interaction follow-up webhook: token past TTL, falling through to regular channel message API"
        );
    }
    true
}

fn build_message_request(
    base_url: &str,
    conversation_id: &str,
    reply: &OutboundReply,
) -> (String, serde_json::Value) {
    let text = reply.text.as_deref().unwrap_or("");

    let body = serde_json::json!({ "content": text });

    // Check if this is a deferred interaction follow-up. The interaction
    // token is passed via metadata as "interaction_thread_id" =
    // "interaction:{application_id}:{token}".
    let interaction_info = reply
        .metadata
        .as_ref()
        .and_then(|m| m.get("interaction_thread_id"))
        .and_then(|v| v.as_str())
        .and_then(|s| {
            let parts: Vec<&str> = s.splitn(3, ':').collect();
            if parts.len() == 3 && parts[0] == "interaction" {
                Some((parts[1].to_string(), parts[2].to_string()))
            } else {
                None
            }
        });

    let url = if let Some((app_id, token)) = &interaction_info {
        // Interaction follow-up endpoint (for deferred responses)
        format!("{base_url}/webhooks/{app_id}/{token}")
    } else {
        // Regular channel message
        format!("{base_url}/channels/{conversation_id}/messages")
    };
    (url, body)
}

fn build_edit_message_request(
    base_url: &str,
    conversation_id: &str,
    platform_message_id: &str,
    edit: &OutboundEdit,
) -> (String, serde_json::Value) {
    (
        format!("{base_url}/channels/{conversation_id}/messages/{platform_message_id}"),
        serde_json::json!({ "content": edit.text.as_deref().unwrap_or("") }),
    )
}

#[cfg(test)]
impl DiscordAdapter {
    pub(super) fn media_test_adapter(base: &str) -> Self {
        Self {
            base_url: base.into(),
        }
    }
}

#[async_trait::async_trait]
impl PlatformAdapter for DiscordAdapter {
    fn display_name(&self) -> &str {
        "Discord"
    }
    fn media_capabilities(&self) -> MediaCapabilities {
        MediaCapabilities::ALL
    }
    /// No durable thread key. interaction_thread_id is a reply-only follow-up credential.
    fn outbound_capabilities(&self) -> crate::services::channel_platform::OutboundCapabilities {
        crate::services::channel_platform::OutboundCapabilities {
            initiated_send: true,
            reply_to: false,
            thread: false,
            edit: true,
        }
    }

    fn platform_id(&self) -> &str {
        "discord"
    }

    fn registration(&self) -> super::super::channel_platform::RegistrationDescriptor {
        use super::super::channel_registration::{
            BOT_TOKEN_FIELD, RegistrationDescriptor, RegistrationField,
        };
        RegistrationDescriptor {
            documentation_url: Some(
                "https://discord.com/developers/docs/interactions/receiving-and-responding",
            ),
            required_suffix: " for Discord",
            fields: &[
                BOT_TOKEN_FIELD,
                RegistrationField {
                    name: "public_key",
                    label: "Public Key",
                    storage: "public_key",
                    secret: false,
                    required: true,
                    patchable: false,
                    clearable: false,
                    webhook_secret: false,
                    hint: None,
                    platform_fallback: None,
                },
            ],
            ..RegistrationDescriptor::default()
        }
    }

    fn webhook_policy(&self, body: &[u8]) -> super::super::channel_platform::WebhookPolicy {
        use super::super::channel_platform::WebhookPolicy;
        if let Some(challenge) = self.handle_challenge(body) {
            return WebhookPolicy::Challenge(challenge);
        }
        let is_interaction = serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|value| value.get("type")?.as_u64())
            .is_some_and(|kind| kind == 2 || kind == 4);
        if is_interaction {
            WebhookPolicy::Immediate(Some(serde_json::json!({"type": 5})))
        } else {
            WebhookPolicy::Inline
        }
    }

    fn reply_context(
        &self,
        thread_id: Option<&str>,
        created_at: chrono::DateTime<chrono::Utc>,
        metadata: &mut Option<serde_json::Value>,
    ) {
        apply_interaction_context(thread_id, created_at, metadata);
    }

    async fn verify_webhook(
        &self,
        bot: &ChannelBot,
        _secrets: Option<&crate::services::channel_platform::PlatformVerifySecrets>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<()> {
        let public_key_hex = bot.public_key.as_deref().ok_or_else(|| {
            AppError::ChannelWebhookVerificationFailed(
                "Discord bot missing public_key configuration".to_string(),
            )
        })?;

        let signature_hex = headers
            .get("x-signature-ed25519")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError::ChannelWebhookVerificationFailed(
                    "missing X-Signature-Ed25519 header".to_string(),
                )
            })?;

        let timestamp = headers
            .get("x-signature-timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError::ChannelWebhookVerificationFailed(
                    "missing X-Signature-Timestamp header".to_string(),
                )
            })?;

        // Decode the public key from hex (32 bytes)
        let pk_bytes = hex::decode(public_key_hex).map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "invalid public key hex encoding".to_string(),
            )
        })?;

        let verifying_key =
            VerifyingKey::from_bytes(pk_bytes.as_slice().try_into().map_err(|_| {
                AppError::ChannelWebhookVerificationFailed(
                    "invalid public key length (expected 32 bytes)".to_string(),
                )
            })?)
            .map_err(|_| {
                AppError::ChannelWebhookVerificationFailed("invalid Ed25519 public key".to_string())
            })?;

        // Decode the signature from hex (64 bytes)
        let sig_bytes = hex::decode(signature_hex).map_err(|_| {
            AppError::ChannelWebhookVerificationFailed("invalid signature hex encoding".to_string())
        })?;

        let signature = Signature::from_bytes(sig_bytes.as_slice().try_into().map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "invalid signature length (expected 64 bytes)".to_string(),
            )
        })?);

        // Verify: Ed25519(public_key, timestamp + body, signature)
        let mut message = Vec::with_capacity(timestamp.len() + body.len());
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body);

        use ed25519_dalek::Verifier;
        verifying_key.verify(&message, &signature).map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "Ed25519 signature verification failed".to_string(),
            )
        })?;

        Ok(())
    }

    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| AppError::BadRequest(format!("invalid Discord webhook JSON: {e}")))?;

        let interaction_type = payload.get("type").and_then(|v| v.as_u64());

        match interaction_type {
            // PING -- handled by handle_challenge, return empty
            Some(INTERACTION_PING) => Ok(Vec::new()),
            // APPLICATION_COMMAND or MESSAGE_COMPONENT
            Some(INTERACTION_APPLICATION_COMMAND) | Some(INTERACTION_MESSAGE_COMPONENT) => {
                match parse_interaction(&payload) {
                    Some(msg) => Ok(vec![msg]),
                    None => Ok(Vec::new()),
                }
            }
            // No type field -- might be a Gateway-style message
            None => match parse_gateway_message(&payload) {
                Some(msg) => Ok(vec![msg]),
                None => Ok(Vec::new()),
            },
            // Unhandled interaction types
            _ => Ok(Vec::new()),
        }
    }

    async fn fetch_attachment(
        &self,
        _http: &reqwest::Client,
        _credentials: &crate::services::channel_platform::BotCredentials<'_>,
        attachment: &InboundAttachment,
        max_bytes: u64,
    ) -> AppResult<FetchedMedia> {
        media::download(
            &attachment.url,
            &["cdn.discordapp.com", "media.discordapp.net"],
            Some(&self.base_url),
            None,
            attachment,
            max_bytes,
        )
        .await
    }

    async fn send_reply(
        &self,
        http: &reqwest::Client,
        credentials: &crate::services::channel_platform::BotCredentials<'_>,
        conversation_id: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        let bot_token = credentials.token;
        let (url, mut body) = build_message_request(&self.base_url, conversation_id, reply);
        let request = http
            .post(&url)
            .header("Authorization", format!("Bot {bot_token}"));
        let request = if reply.attachments.is_empty() {
            request.json(&body)
        } else {
            let captions: Vec<_> = reply
                .text
                .iter()
                .map(String::as_str)
                .chain(
                    reply
                        .attachments
                        .iter()
                        .filter_map(|a| a.caption.as_deref()),
                )
                .collect();
            body["content"] = serde_json::json!(captions.join("\n"));
            body["attachments"] = serde_json::json!(reply.attachments.iter().enumerate().map(|(i, a)| {
                serde_json::json!({"id": i, "filename": media::safe_filename(a.filename.as_deref())})
            }).collect::<Vec<_>>());
            let mut form = reqwest::multipart::Form::new().text("payload_json", body.to_string());
            for (i, attachment) in reply.attachments.iter().enumerate() {
                form = form.part(format!("files[{i}]"), media::multipart_part(attachment)?);
            }
            request.multipart(form)
        };
        if !reply.attachments.is_empty() {
            let response = media::response_json(request).await?;
            return response["id"]
                .as_str()
                .map(|id| Some(id.to_string()))
                .ok_or_else(media::upload_failed);
        }
        let resp: serde_json::Value = request
            .send()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Discord create message request failed: {}",
                    e.without_url()
                ))
            })?
            .json()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Discord create message response parse failed: {}",
                    e.without_url()
                ))
            })?;

        // Discord returns the message object on success with an `id` field.
        // On error it returns `{ "code": ..., "message": "..." }`.
        if let Some(error_msg) = resp.get("message").filter(|_| resp.get("code").is_some()) {
            let desc = error_msg.as_str().unwrap_or("unknown error");
            return Err(
                crate::services::channel_platform::classify_upstream_refusal(
                    "Discord",
                    desc,
                    UNREACHABLE_TARGET_MARKERS,
                ),
            );
        }

        let message_id = resp.get("id").and_then(|v| v.as_str()).map(String::from);

        Ok(message_id)
    }

    async fn edit_reply(
        &self,
        http: &reqwest::Client,
        credentials: &crate::services::channel_platform::BotCredentials<'_>,
        conversation_id: &str,
        platform_message_id: &str,
        edit: &OutboundEdit,
    ) -> AppResult<()> {
        let (url, body) =
            build_edit_message_request(&self.base_url, conversation_id, platform_message_id, edit);
        let resp: serde_json::Value = http
            .patch(&url)
            .header("Authorization", format!("Bot {}", credentials.token))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Discord edit message request failed: {}",
                    e.without_url()
                ))
            })?
            .json()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Discord edit message response parse failed: {}",
                    e.without_url()
                ))
            })?;
        if let Some(error_msg) = resp.get("message").filter(|_| resp.get("code").is_some()) {
            return Err(
                crate::services::channel_platform::classify_upstream_refusal(
                    "Discord",
                    error_msg.as_str().unwrap_or("unknown error"),
                    UNREACHABLE_TARGET_MARKERS,
                ),
            );
        }
        Ok(())
    }

    async fn register_webhook(
        &self,
        _http: &reqwest::Client,
        _bot_token: &str,
        _webhook_url: &str,
        _secret: &str,
    ) -> AppResult<()> {
        // Discord Interactions endpoint URL is configured in the Discord
        // Developer Portal, not via API. This is a no-op.
        Ok(())
    }

    async fn verify_bot_token(
        &self,
        http: &reqwest::Client,
        credentials: &crate::services::channel_platform::BotCredentials<'_>,
    ) -> AppResult<BotIdentity> {
        let bot_token = credentials.token;
        let url = format!("{}/users/@me", self.base_url);
        let resp: serde_json::Value = http
            .get(&url)
            .header("Authorization", format!("Bot {bot_token}"))
            .send()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!("Discord users/@me request failed: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Discord users/@me response parse failed: {e}"
                ))
            })?;

        let bot_id = resp.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
            AppError::ChannelPlatformError("Discord users/@me response missing id".to_string())
        })?;

        let username = resp
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        Ok(BotIdentity {
            platform_bot_id: bot_id.to_string(),
            platform_bot_username: username.to_string(),
        })
    }

    fn handle_challenge(&self, body: &[u8]) -> Option<serde_json::Value> {
        let payload: serde_json::Value = serde_json::from_slice(body).ok()?;
        let interaction_type = payload.get("type")?.as_u64()?;

        if interaction_type == INTERACTION_PING {
            // Respond with PONG (type: 1)
            Some(serde_json::json!({ "type": 1 }))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_message_request_uses_channel_endpoint_and_content_only() {
        let edit = OutboundEdit {
            text: Some("updated".into()),
            metadata: Some(serde_json::json!({"interaction_thread_id": "interaction:app:secret"})),
        };
        assert_eq!(
            build_edit_message_request(DISCORD_API_BASE, "123", "456", &edit),
            (
                format!("{DISCORD_API_BASE}/channels/123/messages/456"),
                serde_json::json!({"content": "updated"})
            )
        );
    }

    #[tokio::test]
    async fn native_edit_success_and_classified_refusals() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, header, method, path},
        };
        let mut responses = vec![(
            200,
            serde_json::json!({"id": "456", "content": "updated"}),
            None,
        )];
        for marker in UNREACHABLE_TARGET_MARKERS {
            responses.push((
                403,
                serde_json::json!({"code": 50005, "message": format!("{marker}: private content")}),
                Some(*marker),
            ));
        }
        for (status, response, refusal) in responses {
            let server = MockServer::start().await;
            Mock::given(method("PATCH"))
                .and(path("/channels/123/messages/456"))
                .and(header("Authorization", "Bot test-token"))
                .and(body_json(serde_json::json!({"content": "updated"})))
                .respond_with(ResponseTemplate::new(status).set_body_json(response))
                .expect(1)
                .mount(&server)
                .await;
            let adapter = DiscordAdapter {
                base_url: server.uri(),
            };
            let result = adapter
                .edit_reply(
                    &reqwest::Client::new(),
                    &"test-token".into(),
                    "123",
                    "456",
                    &OutboundEdit {
                        text: Some("updated".into()),
                        metadata: None,
                    },
                )
                .await;
            if let Some(marker) = refusal {
                assert!(
                    matches!(result, Err(AppError::ChannelConversationNotReachable(reason)) if reason == format!("Discord: {marker}"))
                );
            } else {
                result.unwrap();
            }
        }
    }

    // -- platform_id ---------------------------------------------------------

    #[test]
    fn platform_id_is_discord() {
        let adapter = DiscordAdapter::default();
        assert_eq!(adapter.platform_id(), "discord");
    }

    // -- handle_challenge ----------------------------------------------------

    #[test]
    fn handle_challenge_ping_returns_pong() {
        let adapter = DiscordAdapter::default();
        let body = serde_json::json!({ "type": 1 }).to_string();
        let result = adapter.handle_challenge(body.as_bytes());
        assert!(result.is_some());
        let pong = result.unwrap();
        assert_eq!(pong["type"], 1);
    }

    #[test]
    fn handle_challenge_non_ping_returns_none() {
        let adapter = DiscordAdapter::default();
        let body = serde_json::json!({ "type": 2, "data": {} }).to_string();
        assert!(adapter.handle_challenge(body.as_bytes()).is_none());
    }

    #[test]
    fn handle_challenge_invalid_json_returns_none() {
        let adapter = DiscordAdapter::default();
        assert!(adapter.handle_challenge(b"not json").is_none());
    }

    #[test]
    fn channel_discord_attachment_arrays_and_interaction_files_are_normalized() {
        let files = serde_json::json!([
            {"id":"1", "url":"https://cdn.discordapp.com/a", "filename":"a.png", "content_type":"image/png", "size":10},
            {"id":"2", "url":"https://cdn.discordapp.com/b", "filename":"b.pdf", "content_type":"application/pdf"},
            {"id":"3", "url":"https://cdn.discordapp.com/c", "content_type":"audio/ogg"},
            {"id":"4", "url":"https://cdn.discordapp.com/d", "content_type":"video/mp4"}
        ]);
        let parsed = extract_attachments(&serde_json::json!({"id":"m1", "attachments":files}));
        assert_eq!(
            parsed
                .iter()
                .map(|a| a.content_type.as_str())
                .collect::<Vec<_>>(),
            ["image", "file", "audio", "video"]
        );
        assert_eq!(parsed[0].size_bytes, Some(10));
        assert_eq!(parsed[0].platform_message_id.as_deref(), Some("m1"));
        let interaction = extract_attachments(
            &serde_json::json!({"id":"i1", "data":{"resolved":{"attachments":{"1":files[0]}}}}),
        );
        assert_eq!(interaction.len(), 1);
        assert_eq!(interaction[0].filename.as_deref(), Some("a.png"));
    }

    // -- parse_inbound -------------------------------------------------------

    #[tokio::test]
    async fn parse_ping_returns_empty() {
        let adapter = DiscordAdapter::default();
        let body = serde_json::json!({ "type": 1 });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_application_command() {
        let adapter = DiscordAdapter::default();
        let body = serde_json::json!({
            "type": 2,
            "id": "interaction_123",
            "channel_id": "ch_456",
            "channel": { "type": 0 },
            "member": {
                "user": {
                    "id": "user_789",
                    "username": "TestUser"
                }
            },
            "data": {
                "name": "ask",
                "options": [
                    { "name": "question", "value": "hello?" }
                ]
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.platform_message_id, "interaction_123");
        assert_eq!(m.conversation_id, "ch_456");
        assert_eq!(m.conversation_type, "group");
        assert_eq!(m.sender_platform_id, "user_789");
        assert_eq!(m.sender_display_name.as_deref(), Some("TestUser"));
        assert_eq!(m.content_type, "text");
        assert_eq!(m.text.as_deref(), Some("/ask question=hello?"));
    }

    #[tokio::test]
    async fn parse_message_component() {
        let adapter = DiscordAdapter::default();
        let body = serde_json::json!({
            "type": 4,
            "id": "comp_111",
            "channel_id": "ch_222",
            "channel": { "type": 1 },
            "user": {
                "id": "dm_user",
                "username": "DMUser"
            },
            "data": {
                "custom_id": "approve_action"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.conversation_type, "private");
        assert_eq!(m.sender_platform_id, "dm_user");
        assert_eq!(m.text.as_deref(), Some("approve_action"));
    }

    #[tokio::test]
    async fn parse_gateway_message() {
        let adapter = DiscordAdapter::default();
        let body = serde_json::json!({
            "id": "msg_555",
            "channel_id": "ch_666",
            "author": {
                "id": "author_777",
                "username": "GatewayUser"
            },
            "content": "Hello from gateway",
            "message_reference": {
                "message_id": "msg_444"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.platform_message_id, "msg_555");
        assert_eq!(m.conversation_id, "ch_666");
        assert_eq!(m.sender_platform_id, "author_777");
        assert_eq!(m.text.as_deref(), Some("Hello from gateway"));
        assert_eq!(m.reply_to_platform_message_id.as_deref(), Some("msg_444"));
    }

    #[tokio::test]
    async fn parse_unhandled_interaction_returns_empty() {
        let adapter = DiscordAdapter::default();
        // Type 5 = MODAL_SUBMIT -- not handled
        let body = serde_json::json!({ "type": 5, "data": {} });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_invalid_json_returns_error() {
        let adapter = DiscordAdapter::default();
        let result = adapter.parse_inbound(b"not json").await;
        assert!(result.is_err());
    }

    // -- conversation_type mapping -------------------------------------------

    #[test]
    fn conversation_type_mapping() {
        assert_eq!(map_conversation_type(Some(CHANNEL_DM)), "private");
        assert_eq!(map_conversation_type(Some(CHANNEL_GROUP_DM)), "group");
        assert_eq!(map_conversation_type(Some(0)), "group"); // GUILD_TEXT
        assert_eq!(map_conversation_type(Some(5)), "group"); // GUILD_ANNOUNCEMENT
        assert_eq!(map_conversation_type(Some(15)), "group"); // GUILD_FORUM
        assert_eq!(map_conversation_type(None), "group");
        assert_eq!(map_conversation_type(Some(99)), "group"); // unknown
    }

    // -- verify_webhook (signature verification) -----------------------------

    #[tokio::test]
    async fn verify_webhook_valid_signature() {
        use ed25519_dalek::{Signer, SigningKey};

        let adapter = DiscordAdapter::default();

        // Generate a test key pair
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.to_bytes());

        let timestamp = "1700000000";
        let body_content = b"{\"type\":2}";

        // Build the message to sign
        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body_content);

        let signature = signing_key.sign(&message);
        let signature_hex = hex::encode(signature.to_bytes());

        let bot = make_test_bot(Some(&public_key_hex));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-signature-ed25519", signature_hex.parse().unwrap());
        headers.insert("x-signature-timestamp", timestamp.parse().unwrap());

        let result = adapter
            .verify_webhook(&bot, None, &headers, body_content)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn verify_webhook_invalid_signature() {
        let adapter = DiscordAdapter::default();

        // Use a known public key but wrong signature
        let bot = make_test_bot(Some(&hex::encode([1u8; 32])));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-signature-ed25519",
            hex::encode([0u8; 64]).parse().unwrap(),
        );
        headers.insert("x-signature-timestamp", "12345".parse().unwrap());

        let result = adapter.verify_webhook(&bot, None, &headers, b"{}").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn verify_webhook_missing_public_key() {
        let adapter = DiscordAdapter::default();
        let bot = make_test_bot(None);
        let headers = axum::http::HeaderMap::new();

        let result = adapter.verify_webhook(&bot, None, &headers, b"{}").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn verify_webhook_missing_headers() {
        let adapter = DiscordAdapter::default();
        let bot = make_test_bot(Some(&hex::encode([1u8; 32])));
        let headers = axum::http::HeaderMap::new();

        let result = adapter.verify_webhook(&bot, None, &headers, b"{}").await;
        assert!(result.is_err());
    }

    // -- test helper ---------------------------------------------------------

    fn make_test_bot(public_key: Option<&str>) -> ChannelBot {
        ChannelBot {
            x_events: None,
            last_verification: None,
            ownership_version: 0,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "discord".to_string(),
            label: "Test Discord Bot".to_string(),
            credential_source: "user".to_string(),
            connection_id: None,
            poll_cursor: None,
            poll_lease_until: None,
            last_polled_at: None,
            poll_backoff_until: None,
            poll_error_count: 0,
            last_poll_notice: None,
            error: None,
            registration_pin_encrypted: None,
            webhook_secret_encrypted: None,
            managed_setup: None,
            bot_token_encrypted: vec![0; 16],
            platform_bot_id: "bot_123".to_string(),
            platform_bot_username: "testbot".to_string(),
            webhook_registered: true,
            webhook_secret_hash: "unused_for_discord".to_string(),
            app_id: None,
            app_secret_encrypted: None,
            lark_verification_token_encrypted: None,
            lark_encrypt_key_encrypted: None,
            public_key: public_key.map(String::from),
            status: "active".to_string(),
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    // -- slash command without options ---------------------------------------

    #[tokio::test]
    async fn parse_slash_command_no_options() {
        let adapter = DiscordAdapter::default();
        let body = serde_json::json!({
            "type": 2,
            "id": "int_no_opts",
            "channel_id": "ch_1",
            "channel": { "type": 0 },
            "member": {
                "user": { "id": "u1", "username": "User1" }
            },
            "data": { "name": "help" }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text.as_deref(), Some("/help"));
    }
    #[test]
    fn initiated_request_uses_channel_without_anchor_or_thread() {
        let mut reply = OutboundReply {
            attachments: vec![],
            text: Some("hello".into()),
            reply_to_platform_message_id: None,
            metadata: None,
        };
        let (url, body) = build_message_request(DISCORD_API_BASE, "123", &reply);
        assert!(url.ends_with("/channels/123/messages"));
        assert_eq!(body, serde_json::json!({ "content": "hello" }));
        reply.reply_to_platform_message_id = Some("ignored".into());
        assert_eq!(
            build_message_request(DISCORD_API_BASE, "123", &reply),
            (url, body)
        );
        reply.metadata =
            Some(serde_json::json!({ "interaction_thread_id": "interaction:app:token" }));
        assert!(
            build_message_request(DISCORD_API_BASE, "123", &reply)
                .0
                .ends_with("/webhooks/app/token")
        );
    }

    #[test]
    fn upstream_target_refusals_are_classified_and_other_diagnostics_are_bounded() {
        for marker in UNREACHABLE_TARGET_MARKERS {
            let description = format!("{marker}: private message content");
            let error = crate::services::channel_platform::classify_upstream_refusal(
                "discord",
                &description,
                UNREACHABLE_TARGET_MARKERS,
            );
            assert!(matches!(
                error,
                AppError::ChannelConversationNotReachable(_)
            ));
            assert!(!error.to_string().contains("private message content"));
        }
        for description in ["Invalid Form Body".to_string(), "界".repeat(201)] {
            let error = crate::services::channel_platform::classify_upstream_refusal(
                "discord",
                &description,
                UNREACHABLE_TARGET_MARKERS,
            );
            let expected: String = description.chars().take(200).collect();
            assert!(matches!(error, AppError::ChannelPlatformError(detail)
                if detail == format!("discord send failed: {expected}")));
        }
    }
}
