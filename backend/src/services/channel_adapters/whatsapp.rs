//! WhatsApp Business Platform, directly through Meta's Cloud API.
//! Consumer/Business App automation and Twilio's separate API are not supported.

const UNREACHABLE_TARGET_MARKERS: &[&str] = &[
    "outside the 24-hour customer service window; an approved template message is required",
    "message undeliverable",
];

use crate::services::channel_media_service as media;
use crate::services::channel_platform::{FetchedMedia, MediaCapabilities, MediaKind};

use axum::http::{HeaderMap, StatusCode};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::services::channel_platform::{
    BotCredentials, BotIdentity, InboundAttachment, InboundMessage, OutboundReply, PlatformAdapter,
    PlatformVerifySecrets, PreparedWebhook, WebhookPolicy,
};
use crate::services::channel_registration::{
    BOT_TOKEN_FIELD, RegistrationDescriptor, RegistrationField,
};

/// Pin every Cloud API URL constructed by this adapter here.
pub const GRAPH_API_VERSION: &str = "v25.0";
const TEXT_LIMIT: usize = 4096;
const GRAPH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const GRAPH_JSON_LIMIT: usize = 256 * 1024;

pub struct WhatsAppAdapter;

fn graph_url(id: &str) -> String {
    format!("https://graph.facebook.com/{GRAPH_API_VERSION}/{id}")
}

pub(super) fn validate_id(id: &str, label: &str) -> AppResult<()> {
    if id.is_empty() || id.len() > 32 || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AppError::ValidationError(format!(
            "{label} must be a numeric Meta identifier"
        )));
    }
    Ok(())
}

pub(super) fn verification_failed() -> AppError {
    AppError::ChannelWebhookVerificationFailed("Invalid WhatsApp webhook signature".to_string())
}

fn message_text(message: &Value, kind: &str) -> Option<String> {
    let value = &message[kind];
    match kind {
        "text" => value["body"].as_str().map(String::from),
        "image" | "video" | "audio" | "document" | "sticker" => {
            value["caption"].as_str().map(String::from)
        }
        "button" => value["text"]
            .as_str()
            .or_else(|| value["payload"].as_str())
            .map(String::from),
        "interactive" => {
            let selection = value
                .get("button_reply")
                .or_else(|| value.get("list_reply"))?;
            selection["title"]
                .as_str()
                .or_else(|| selection["id"].as_str())
                .map(String::from)
        }
        "location" => {
            let latitude = value["latitude"].as_f64()?;
            let longitude = value["longitude"].as_f64()?;
            let mut text = format!("Location: {latitude}, {longitude}");
            for key in ["name", "address"] {
                if let Some(detail) = value[key].as_str() {
                    text.push_str(&format!("; {detail}"));
                }
            }
            Some(text)
        }
        "contacts" => {
            let contacts = value.as_array()?;
            let names: Vec<&str> = contacts
                .iter()
                .filter_map(|contact| contact["name"]["formatted_name"].as_str())
                .collect();
            Some(format!("Contacts: {}", names.join(", ")))
        }
        _ => None,
    }
}

fn parse_message(message: &Value, contacts: &Value) -> Option<InboundMessage> {
    let kind = message["type"].as_str()?;
    // Reactions and protocol notifications should not start agent conversations.
    if !matches!(
        kind,
        "text"
            | "image"
            | "audio"
            | "video"
            | "document"
            | "sticker"
            | "location"
            | "contacts"
            | "button"
            | "interactive"
            | "order"
    ) {
        return None;
    }
    let sender = message["from"].as_str().filter(|value| !value.is_empty())?;
    let message_id = message["id"]
        .as_str()
        .filter(|value| crate::services::channel_delivery_service::valid_message_id(value))?;
    let category = match kind {
        "image" | "sticker" => "image",
        "audio" => "audio",
        "video" => "video",
        "document" => "file",
        "order" => "unknown",
        _ => "text",
    };
    let mut attachments = Vec::new();
    if matches!(kind, "image" | "sticker" | "audio" | "video" | "document") {
        let media = &message[kind];
        if let Some(id) = media["id"]
            .as_str()
            .filter(|id| validate_id(id, "Media ID").is_ok())
        {
            attachments.push(InboundAttachment {
                content_type: category.to_string(),
                // GET this lookup URL with Bearer auth, then GET its returned
                // short-lived `url` with the same token to download the bytes.
                url: graph_url(id),
                platform_message_id: Some(message_id.to_string()),
                file_key: Some(id.to_string()),
                image_key: None,
                filename: media["filename"].as_str().map(String::from),
                mime_type: media["mime_type"].as_str().map(String::from),
                size_bytes: None,
            });
        }
    }
    let display_name = contacts
        .as_array()
        .and_then(|contacts| {
            contacts
                .iter()
                .find(|contact| contact["wa_id"].as_str() == Some(sender))
        })
        .and_then(|contact| contact["profile"]["name"].as_str())
        .map(String::from);
    Some(InboundMessage {
        platform_message_id: message_id.to_string(),
        conversation_id: sender.to_string(),
        conversation_type: "private".to_string(),
        sender_platform_id: sender.to_string(),
        sender_display_name: display_name,
        content_type: category.to_string(),
        text: message_text(message, kind),
        attachments,
        reply_to_platform_message_id: message["context"]["id"].as_str().map(String::from),
        thread_id: None,
        raw_data: message.clone(),
    })
}

/// The actual request builder, shared by production sends and unit tests.
fn reply_bodies(recipient: &str, reply: &OutboundReply) -> AppResult<Vec<Value>> {
    let recipient: String = recipient
        .chars()
        .filter(|character| *character != ' ' && *character != '-')
        .collect();
    let recipient = recipient.strip_prefix('+').unwrap_or(&recipient);
    validate_id(recipient, "Recipient")?;
    let metadata = reply.metadata.as_ref();
    let template = metadata
        .and_then(|value| value.get("template"))
        .filter(|value| value.is_object());
    let interactive = metadata
        .and_then(|value| value.get("interactive"))
        .filter(|value| value.is_object());
    if template.is_some() && interactive.is_some() {
        return Err(AppError::ValidationError(
            "Choose either template or interactive metadata".to_string(),
        ));
    }
    let mut base =
        json!({"messaging_product": "whatsapp", "recipient_type": "individual", "to": recipient});
    if let Some(message_id) = &reply.reply_to_platform_message_id {
        base["context"] = json!({"message_id": message_id});
    }
    if let Some((kind, payload)) = template
        .map(|value| ("template", value))
        .or_else(|| interactive.map(|value| ("interactive", value)))
    {
        base["type"] = json!(kind);
        base[kind] = payload.clone();
        return Ok(vec![base]);
    }
    let text = reply
        .text
        .as_deref()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            AppError::ValidationError(
                "WhatsApp reply requires text, template, or interactive content".to_string(),
            )
        })?;
    let characters: Vec<char> = text.chars().collect();
    Ok(characters
        .chunks(TEXT_LIMIT)
        .map(|chunk| {
            let mut body = base.clone();
            body["type"] = json!("text");
            body["text"] = json!({"preview_url": false, "body": chunk.iter().collect::<String>()});
            body
        })
        .collect())
}

fn graph_error(status: StatusCode, body: &Value, retry_after: Option<&str>) -> AppError {
    let code = body["error"]["code"].as_i64().unwrap_or_default();
    let reason = match code {
        131047 => {
            "outside the 24-hour customer service window; an approved template message is required"
        }
        131026 => "message undeliverable; check the recipient and their WhatsApp availability",
        131051 => "unsupported message type",
        131053 => "media upload failed; check media format and size",
        190 => "access token is invalid or expired",
        10 | 200 => "required WhatsApp permissions are missing",
        100 => "check the Phone Number ID and the token's access to that WhatsApp account",
        131030 => "recipient is not authorized for this test number; check Meta API Setup",
        _ => "Graph API request failed",
    };
    if status == StatusCode::TOO_MANY_REQUESTS || matches!(code, 130429 | 80007) {
        let retry = retry_after
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(|seconds| seconds.min(3600));
        return AppError::ChannelPlatformError(match retry {
            Some(seconds) => format!("WhatsApp rate limited; wait {seconds}s before another request. For a message send, check the conversation before sending again."),
            None => "WhatsApp rate limited; wait before another request. For a message send, check the conversation before sending again.".to_string(),
        });
    }
    if matches!(code, 131047 | 131026) {
        return crate::services::channel_platform::classify_upstream_refusal(
            "WhatsApp",
            reason,
            UNREACHABLE_TARGET_MARKERS,
        );
    }
    // Graph's free-form error.message can echo request material. Only emit
    // locally-authored causes and numeric codes, never that message or tokens.
    AppError::ChannelPlatformError(format!(
        "WhatsApp: {reason} (HTTP {}, code {code})",
        status.as_u16()
    ))
}

fn graph_transport_error(error: reqwest::Error) -> AppError {
    AppError::ChannelPlatformError(if error.is_timeout() {
        "WhatsApp Graph request timed out. Credential checks may be retried. Message acceptance may be unknown; check message history before sending again.".into()
    } else {
        "WhatsApp Graph request could not complete. Check server connectivity. Message acceptance may be unknown; check message history before sending again.".into()
    })
}

pub(super) async fn graph_send(request: reqwest::RequestBuilder) -> AppResult<Value> {
    graph_send_with_timeout(request, GRAPH_TIMEOUT).await
}

async fn graph_send_with_timeout(
    request: reqwest::RequestBuilder,
    timeout: std::time::Duration,
) -> AppResult<Value> {
    graph_request(request, timeout)
        .await
        .map_err(|failure| failure.error)
}

struct GraphRequestFailure {
    error: AppError,
    confirmed_refusal: bool,
}
impl From<AppError> for GraphRequestFailure {
    fn from(error: AppError) -> Self {
        Self {
            error,
            confirmed_refusal: false,
        }
    }
}

async fn graph_request(
    request: reqwest::RequestBuilder,
    timeout: std::time::Duration,
) -> Result<Value, GraphRequestFailure> {
    let response = request
        .timeout(timeout)
        .send()
        .await
        .map_err(graph_transport_error)?;
    classified_graph_response(response).await
}

async fn classified_graph_response(
    mut response: reqwest::Response,
) -> Result<Value, GraphRequestFailure> {
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .map(String::from);
    let too_large =
        || AppError::ChannelPlatformError("WhatsApp Graph response exceeded the size limit".into());
    if response
        .content_length()
        .is_some_and(|length| length > GRAPH_JSON_LIMIT as u64)
    {
        return Err(too_large().into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(graph_transport_error)? {
        if chunk.len() > GRAPH_JSON_LIMIT - bytes.len() {
            return Err(too_large().into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let body = serde_json::from_slice::<Value>(&bytes);
    if !status.is_success() || body.as_ref().is_ok_and(|body| body.get("error").is_some()) {
        let body = body.unwrap_or(Value::Null);
        let code = body["error"]["code"].as_i64();
        let confirmed_refusal = status.is_client_error()
            && (status == StatusCode::TOO_MANY_REQUESTS
                || code.is_some_and(|code| {
                    matches!(
                        code,
                        190 | 10
                            | 200
                            | 100
                            | 131030
                            | 131047
                            | 131026
                            | 131051
                            | 131053
                            | 130429
                            | 80007
                    )
                }));
        return Err(GraphRequestFailure {
            error: graph_error(status, &body, retry_after.as_deref()),
            confirmed_refusal,
        });
    }
    let body =
        body.map_err(|_| AppError::ChannelPlatformError("WhatsApp returned invalid JSON".into()))?;
    Ok(body)
}

async fn fetch_media_at(
    http: &reqwest::Client,
    credentials: &BotCredentials<'_>,
    attachment: &InboundAttachment,
    max_bytes: u64,
    api_base: &str,
) -> AppResult<FetchedMedia> {
    // Verify the webhook reference before using any credential. Construct lookup
    // paths from the numeric handle rather than following arbitrary Graph paths.
    media::media_client(
        &attachment.url,
        Some(&["graph.facebook.com"]),
        Some(api_base),
    )
    .await?;
    let id = attachment
        .file_key
        .as_deref()
        .ok_or_else(media::fetch_failed)?;
    validate_id(id, "Media ID").map_err(|_| media::fetch_failed())?;
    let response = graph_send(super::whatsapp_managed::authenticate(
        http.get(format!("{api_base}/{id}")),
        credentials,
    )?)
    .await
    .map_err(|_| media::fetch_failed())?;
    if response["file_size"]
        .as_u64()
        .is_some_and(|size| size > max_bytes)
    {
        return Err(AppError::ChannelMediaTooLarge);
    }
    let url = response["url"].as_str().ok_or_else(media::fetch_failed)?;
    media::download(
        url,
        &["lookaside.fbsbx.com", "graph.facebook.com"],
        Some(api_base),
        Some(credentials.token),
        attachment,
        max_bytes,
    )
    .await
}

pub(crate) async fn send_outcome_at(
    http: &reqwest::Client,
    credentials: &BotCredentials<'_>,
    conversation_id: &str,
    reply: &OutboundReply,
    api_base: &str,
) -> AppResult<crate::services::channel_platform::SendOutcome> {
    let phone = credentials.platform_bot_id.unwrap_or_default();
    validate_id(phone, "Phone Number ID")?;
    let recipient: String = conversation_id
        .chars()
        .filter(|c| *c != ' ' && *c != '-')
        .collect();
    let recipient = recipient.strip_prefix('+').unwrap_or(&recipient);
    validate_id(recipient, "Recipient")?;
    let mut text = reply.clone();
    text.attachments.clear();
    let text_bodies = if reply.text.as_deref().is_some_and(|s| !s.is_empty())
        || reply
            .metadata
            .as_ref()
            .is_some_and(|m| m.get("template").is_some() || m.get("interactive").is_some())
    {
        reply_bodies(recipient, &text)?
    } else {
        Vec::new()
    };
    let mut expected = text_bodies.len() + reply.attachments.len();
    for attachment in &reply.attachments {
        if attachment.kind == MediaKind::Audio
            && let Some(caption) = &attachment.caption
        {
            let caption_reply = OutboundReply {
                text: Some(caption.clone()),
                attachments: vec![],
                metadata: None,
                reply_to_platform_message_id: reply.reply_to_platform_message_id.clone(),
            };
            expected += reply_bodies(recipient, &caption_reply)?.len();
        }
    }
    if expected == 0 || expected > crate::services::channel_delivery_service::MAX_COMPONENTS {
        return Err(AppError::ValidationError(
            "WhatsApp reply must contain 1 to 64 message components".into(),
        ));
    }
    let mut components = Vec::new();
    let mut uncertain = false;
    let result: AppResult<()> = async {
        for body in text_bodies {
            send_component(http, credentials, &format!("{api_base}/{phone}/messages"), &body, &mut components, &mut uncertain).await?;
        }
    for attachment in &reply.attachments {
        let kind = match attachment.kind {
            MediaKind::Image => "image",
            MediaKind::File => "document",
            MediaKind::Audio => "audio",
            MediaKind::Video => "video",
        };
        let form = reqwest::multipart::Form::new()
            .text("messaging_product", "whatsapp")
            .text(
                "type",
                attachment
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".into()),
            )
            .part("file", media::multipart_part(attachment)?);
        let upload = graph_send(super::whatsapp_managed::authenticate(
            http.post(format!("{api_base}/{phone}/media"))
                .multipart(form),
            credentials,
        )?)
        .await?;
        let id = upload["id"].as_str().ok_or_else(media::upload_failed)?;
        let mut content = json!({"id": id});
        if let Some(filename) = &attachment.filename
            && attachment.kind == MediaKind::File
        {
            content["filename"] = json!(filename);
        }
        if let Some(caption) = &attachment.caption
            && attachment.kind != MediaKind::Audio
        {
            content["caption"] = json!(caption);
        }
        // WhatsApp audio has no caption field; send the caption separately.
        if let Some(caption) = &attachment.caption
            && attachment.kind == MediaKind::Audio
        {
            let text = OutboundReply {
                attachments: vec![],
                text: Some(caption.clone()),
                metadata: None,
                reply_to_platform_message_id: reply.reply_to_platform_message_id.clone(),
            };
            for body in reply_bodies(recipient, &text)? {
                send_component(http, credentials, &format!("{api_base}/{phone}/messages"), &body, &mut components, &mut uncertain).await?;
            }
        }
        let mut body = json!({"messaging_product": "whatsapp", "recipient_type": "individual", "to": recipient, "type": kind, kind: content});
        if let Some(id) = &reply.reply_to_platform_message_id {
            body["context"] = json!({"message_id": id});
        }
        send_component(http, credentials, &format!("{api_base}/{phone}/messages"), &body, &mut components, &mut uncertain).await?;
    }
        Ok(())
    }.await;
    if components.is_empty()
        && !uncertain
        && let Err(error) = result
    {
        return Err(error);
    }
    let error = result.err();
    Ok(crate::services::channel_platform::SendOutcome {
        final_id: components.last().cloned(),
        observation: Some(crate::models::channel_delivery::PlatformSendRecord {
            phone_number_id: phone.into(),
            waba_id: None,
            recipient_id: recipient.into(),
            complete: error.is_none() && components.len() == expected,
            uncertain,
            failure_code: error.as_ref().map(AppError::error_code),
            component_ids: components,
            expected_components: expected as u32,
        }),
        error,
    })
}

async fn send_component(
    http: &reqwest::Client,
    credentials: &BotCredentials<'_>,
    url: &str,
    body: &Value,
    components: &mut Vec<String>,
    uncertain: &mut bool,
) -> AppResult<()> {
    let request = super::whatsapp_managed::authenticate(http.post(url).json(body), credentials)?;
    *uncertain = true;
    let response = match graph_request(request, GRAPH_TIMEOUT).await {
        Ok(response) => response,
        Err(failure) => {
            *uncertain = !failure.confirmed_refusal;
            return Err(failure.error);
        }
    };
    let id = response["messages"][0]["id"]
        .as_str()
        .filter(|id| crate::services::channel_delivery_service::valid_message_id(id))
        .filter(|id| !components.iter().any(|old| old == id))
        .ok_or_else(|| {
            AppError::ChannelPlatformError(
                "WhatsApp response has no usable message ID; acceptance is unknown".into(),
            )
        })?;
    components.push(id.to_string());
    *uncertain = false;
    Ok(())
}

#[cfg(test)]
async fn send_media_at(
    http: &reqwest::Client,
    credentials: &BotCredentials<'_>,
    conversation_id: &str,
    reply: &OutboundReply,
    api_base: &str,
) -> AppResult<Option<String>> {
    send_outcome_at(http, credentials, conversation_id, reply, api_base)
        .await?
        .into_result()
}

#[async_trait::async_trait]
impl PlatformAdapter for WhatsAppAdapter {
    fn atomic_inbound_admission(&self) -> bool {
        true
    }
    fn receipt_observations(
        &self,
        prepared_body: &[u8],
    ) -> Vec<crate::services::channel_delivery_service::ReceiptObservation> {
        let Ok(payload) = serde_json::from_slice::<Value>(prepared_body) else {
            return Vec::new();
        };
        if payload["object"] != "whatsapp_business_account" {
            return Vec::new();
        }
        payload["entry"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|entry| entry["changes"].as_array().into_iter().flatten())
            .filter(|change| {
                change["field"] == "messages" && change["value"]["messaging_product"] == "whatsapp"
            })
            .flat_map(|change| change["value"]["statuses"].as_array().into_iter().flatten())
            .filter_map(parse_receipt)
            .take(crate::services::channel_delivery_service::MAX_RECEIPTS)
            .collect()
    }
    fn records_verification_result(&self) -> bool {
        true
    }
    fn display_name(&self) -> &str {
        "WhatsApp"
    }
    fn media_capabilities(&self) -> MediaCapabilities {
        MediaCapabilities::ALL
    }
    fn outbound_capabilities(&self) -> crate::services::channel_platform::OutboundCapabilities {
        crate::services::channel_platform::OutboundCapabilities {
            initiated_send: true,
            reply_to: true,
            thread: false,
            edit: false,
        }
    }

    fn platform_id(&self) -> &str {
        "whatsapp"
    }

    fn platform_credentials(
        &self,
    ) -> Option<crate::services::channel_managed::PlatformCredentialDescriptor> {
        Some(super::whatsapp_managed::CREDENTIALS)
    }
    fn managed_onboarding(
        &self,
    ) -> Option<crate::services::channel_managed::ManagedOnboardingDescriptor> {
        Some(super::whatsapp_managed::ONBOARDING)
    }
    fn validate_platform_subscription(
        &self,
        query: &std::collections::HashMap<String, String>,
    ) -> AppResult<()> {
        super::whatsapp_managed::validate_handshake(query)
    }
    fn managed_webhook_scope(&self, bot: &ChannelBot) -> Option<(&'static str, String)> {
        bot.app_id.clone().map(|waba| ("app_id", waba))
    }
    async fn remove_managed_webhook_override(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        bot: &ChannelBot,
    ) -> AppResult<()> {
        super::whatsapp_managed::remove_override(http, credentials, bot).await
    }
    fn platform_webhook(&self) -> bool {
        true
    }
    fn platform_subscription_handshake(
        &self,
        credentials: &PlatformVerifySecrets,
        query: &std::collections::HashMap<String, String>,
    ) -> AppResult<String> {
        super::whatsapp_managed::handshake(credentials, query)
    }
    async fn platform_webhook_targets(
        &self,
        credentials: &PlatformVerifySecrets,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<Vec<String>> {
        super::whatsapp_managed::targets(credentials, headers, body)
    }
    async fn complete_managed_onboarding(
        &self,
        http: &reqwest::Client,
        credentials: &PlatformVerifySecrets,
        input: &crate::services::channel_managed::ManagedOnboardingInput,
    ) -> AppResult<crate::services::channel_managed::ManagedOnboardingResult> {
        super::whatsapp_managed::complete(http, credentials, input).await
    }
    async fn setup_managed_bot(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        bot: &ChannelBot,
        webhook_url: &str,
        verify_token: &str,
        pin: &str,
        progress: &crate::services::channel_managed::ManagedProgress,
    ) -> AppResult<crate::models::channel_bot::ManagedBotSetup> {
        super::whatsapp_managed::setup(
            http,
            credentials,
            bot,
            webhook_url,
            verify_token,
            pin,
            progress,
        )
        .await
    }
    async fn reregister_managed_bot(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        pin: &str,
    ) -> AppResult<String> {
        super::whatsapp_managed::register(http, credentials, pin).await
    }

    fn registration(&self) -> RegistrationDescriptor {
        RegistrationDescriptor {
            documentation_url: Some(
                "https://developers.facebook.com/documentation/business-messaging/whatsapp/webhooks/create-webhook-endpoint",
            ),
            extra_fields: &[],
            unsupported_patch_message: None,
            create_response_status: "pending_webhook",
            preserve_subscription_on_verify: true,
            fields: &[
                RegistrationField {
                    label: "Access token",
                    hint: Some(
                        "For pre-review testing, use a temporary API Setup token with verified test recipients. Use a System User token for ongoing operation.",
                    ),
                    patchable: true,
                    ..BOT_TOKEN_FIELD
                },
                RegistrationField {
                    name: "phone_number_id",
                    label: "Phone Number ID",
                    storage: "platform_bot_id",
                    secret: false,
                    required: true,
                    patchable: false,
                    clearable: false,
                    webhook_secret: false,
                    hint: Some(
                        "Meta phone number identifier, not the display phone number or App ID.",
                    ),
                    platform_fallback: None,
                },
                RegistrationField {
                    name: "app_secret",
                    label: "Meta App Secret",
                    storage: "app_secret_encrypted",
                    secret: true,
                    required: true,
                    patchable: true,
                    clearable: false,
                    webhook_secret: true,
                    hint: None,
                    platform_fallback: Some("app_secret"),
                },
                RegistrationField {
                    name: "waba_id",
                    label: "WhatsApp Business Account ID",
                    storage: "app_id",
                    secret: false,
                    required: false,
                    patchable: false,
                    clearable: false,
                    webhook_secret: false,
                    hint: Some("Optional WABA ID."),
                    platform_fallback: None,
                },
            ],
            webhook_secret_label: Some("Verify Token"),
            setup_instructions: &[
                "Find the Phone Number ID in WhatsApp > API Setup and the App Secret in Meta App Dashboard > Basic settings.",
                "In Meta App Dashboard > WhatsApp > Configuration, enter the Callback URL and Verify Token shown here, then verify and save.",
                "Subscribe to the messages webhook field. Subscribe this app to the WhatsApp Business Account (POST /{WABA_ID}/subscribed_apps with a token authorized for that account).",
                "For controlled tests before App Review, use a temporary API Setup access token and verify the intended test recipients in Meta API Setup.",
                "For ongoing operation, use a System User access token with whatsapp_business_messaging and whatsapp_business_management permissions and access to the phone number.",
                "The WhatsApp Business Platform (Meta Cloud API) is supported. The consumer WhatsApp Business App has no API; Twilio-hosted WhatsApp uses a different API and is not supported.",
            ],
            ..RegistrationDescriptor::default()
        }
    }

    fn registration_token(
        &self,
        fields: &super::super::channel_platform::RegistrationValues<'_>,
    ) -> AppResult<zeroize::Zeroizing<String>> {
        validate_id(
            fields.get("phone_number_id").unwrap_or_default(),
            "Phone Number ID",
        )?;
        if let Some(waba) = fields.get("waba_id") {
            validate_id(waba, "WhatsApp Business Account ID")?;
        }
        Ok(zeroize::Zeroizing::new(
            fields.get("bot_token").unwrap_or_default().to_string(),
        ))
    }

    fn webhook_policy(&self, _body: &[u8]) -> WebhookPolicy {
        WebhookPolicy::Immediate(None)
    }

    fn dedup_inbound_by_platform_message_id(&self) -> bool {
        true
    }

    fn validate_stored_verification(&self, bot: &ChannelBot) -> AppResult<()> {
        if bot.app_secret_encrypted.is_none() && bot.credential_source != "platform" {
            return Err(AppError::ValidationError(
                "Meta App Secret is not configured".to_string(),
            ));
        }
        Ok(())
    }

    fn subscription_handshake(
        &self,
        bot: &ChannelBot,
        query: &std::collections::HashMap<String, String>,
    ) -> AppResult<String> {
        let denied =
            || AppError::Forbidden("Invalid WhatsApp subscription verification".to_string());
        if query.get("hub.mode").map(String::as_str) != Some("subscribe") {
            return Err(denied());
        }
        let token = query
            .get("hub.verify_token")
            .filter(|token| !token.is_empty())
            .ok_or_else(denied)?;
        let challenge = query
            .get("hub.challenge")
            .filter(|challenge| !challenge.is_empty())
            .ok_or_else(denied)?;
        let expected = hex::decode(&bot.webhook_secret_hash).map_err(|_| denied())?;
        let actual = Sha256::digest(token.as_bytes());
        if !bool::from(actual.as_slice().ct_eq(&expected)) {
            return Err(denied());
        }
        Ok(challenge.clone())
    }

    async fn verify_webhook(
        &self,
        _bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<()> {
        let secret = secrets
            .and_then(|secrets| secrets.get("app_secret"))
            .filter(|secret| !secret.is_empty())
            .ok_or_else(verification_failed)?;
        let signature = headers
            .get("x-hub-signature-256")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("sha256="))
            .filter(|value| value.len() == 64)
            .ok_or_else(verification_failed)?;
        let signature = hex::decode(signature).map_err(|_| verification_failed())?;
        let mut mac =
            Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| verification_failed())?;
        mac.update(body);
        mac.verify_slice(&signature)
            .map_err(|_| verification_failed())
    }

    async fn prepare_webhook(
        &self,
        bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<PreparedWebhook> {
        self.verify_webhook(bot, secrets, headers, body).await?;
        let mut payload: Value = serde_json::from_slice(body).map_err(|_| {
            AppError::ChannelPlatformError("Invalid WhatsApp webhook JSON".to_string())
        })?;
        // Meta subscriptions are app-wide. Filter before the generic parser
        // ever sees events for another registered phone number.
        let is_whatsapp = payload["object"].as_str() == Some("whatsapp_business_account");
        let mut activate_bot = false;
        if let Some(entries) = payload.get_mut("entry").and_then(Value::as_array_mut) {
            for entry in entries {
                let matches_account = bot
                    .app_id
                    .as_deref()
                    .is_none_or(|id| entry["id"].as_str() == Some(id));
                if let Some(changes) = entry.get_mut("changes").and_then(Value::as_array_mut) {
                    changes.retain(|change| {
                        is_whatsapp
                            && matches_account
                            && change["field"].as_str() == Some("messages")
                            && change["value"]["messaging_product"].as_str() == Some("whatsapp")
                            && change["value"]["metadata"]["phone_number_id"].as_str()
                                == Some(bot.platform_bot_id.as_str())
                    });
                    activate_bot |= changes
                        .iter()
                        .any(|change| has_webhook_evidence(&change["value"]));
                }
            }
        }
        Ok(PreparedWebhook {
            body: serde_json::to_vec(&payload)
                .map_err(|_| AppError::Internal("Unable to prepare webhook".to_string()))?,
            challenge_response: None,
            activate_bot,
        })
    }

    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        let payload: Value = serde_json::from_slice(body).map_err(|_| {
            AppError::ChannelPlatformError("Invalid WhatsApp webhook JSON".to_string())
        })?;
        let mut result = Vec::new();
        if payload["object"].as_str() != Some("whatsapp_business_account") {
            return Ok(result);
        }
        for entry in payload["entry"].as_array().into_iter().flatten() {
            for change in entry["changes"].as_array().into_iter().flatten() {
                let value = &change["value"];
                if change["field"].as_str() != Some("messages")
                    || value["messaging_product"].as_str() != Some("whatsapp")
                {
                    continue;
                }
                for message in value["messages"].as_array().into_iter().flatten() {
                    if let Some(message) = parse_message(message, &value["contacts"]) {
                        result.push(message);
                    }
                }
            }
        }
        Ok(result)
    }

    fn supports_reply_metadata(&self, metadata: &Value) -> bool {
        ["template", "interactive"]
            .iter()
            .any(|key| metadata[*key].is_object())
    }

    fn reply_context(
        &self,
        _thread_id: Option<&str>,
        _created_at: chrono::DateTime<chrono::Utc>,
        _metadata: &mut Option<Value>,
    ) {
    }

    async fn fetch_attachment(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        attachment: &InboundAttachment,
        max_bytes: u64,
    ) -> AppResult<FetchedMedia> {
        fetch_media_at(
            http,
            credentials,
            attachment,
            max_bytes,
            &format!("https://graph.facebook.com/{GRAPH_API_VERSION}"),
        )
        .await
    }

    async fn send_reply(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        conversation_id: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        self.send_reply_outcome(http, credentials, conversation_id, reply)
            .await?
            .into_result()
    }

    async fn send_reply_outcome(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        conversation_id: &str,
        reply: &OutboundReply,
    ) -> AppResult<crate::services::channel_platform::SendOutcome> {
        send_outcome_at(
            http,
            credentials,
            conversation_id,
            reply,
            &format!("https://graph.facebook.com/{GRAPH_API_VERSION}"),
        )
        .await
    }

    async fn send_bound_reply_outcome(
        &self,
        _db: &mongodb::Database,
        http: &reqwest::Client,
        _bot: &ChannelBot,
        _original: &crate::models::channel_message::ChannelMessage,
        credentials: &BotCredentials<'_>,
        conversation_id: &str,
        reply: &OutboundReply,
    ) -> AppResult<crate::services::channel_platform::SendOutcome> {
        self.send_reply_outcome(http, credentials, conversation_id, reply)
            .await
    }

    async fn register_webhook(
        &self,
        _http: &reqwest::Client,
        _bot_token: &str,
        _webhook_url: &str,
        _secret: &str,
    ) -> AppResult<()> {
        // App Dashboard configuration and WABA subscribed_apps are manual steps.
        Ok(())
    }

    async fn verify_bot_token(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
    ) -> AppResult<BotIdentity> {
        verify_identity_at(http, credentials, "https://graph.facebook.com").await
    }
}

async fn verify_identity_at(
    http: &reqwest::Client,
    credentials: &BotCredentials<'_>,
    base: &str,
) -> AppResult<BotIdentity> {
    let business_object_id = credentials.platform_bot_id.unwrap_or_default();
    validate_id(business_object_id, "Phone Number ID")?;
    let response = graph_send(super::whatsapp_managed::authenticate(
        http.get(format!("{base}/{GRAPH_API_VERSION}/{business_object_id}"))
            .query(&[("fields", "id,display_phone_number,verified_name")]),
        credentials,
    )?)
    .await?;
    if response["id"].as_str() != Some(business_object_id) {
        return Err(AppError::ChannelPlatformError(
            "WhatsApp phone number identity mismatch".to_string(),
        ));
    }
    let username = response["display_phone_number"]
        .as_str()
        .filter(|name| !name.is_empty())
        .or_else(|| response["verified_name"].as_str())
        .unwrap_or(business_object_id);
    Ok(BotIdentity {
        platform_bot_id: business_object_id.to_string(),
        platform_bot_username: username.to_string(),
    })
}

fn parse_receipt(
    status: &Value,
) -> Option<crate::services::channel_delivery_service::ReceiptObservation> {
    use crate::services::channel_delivery_service::{
        ReceiptObservation, ReceiptStatus, valid_message_id, valid_recipient,
    };
    // This adapter sends recipient_type=individual; a participant receipt is not
    // proof of delivery to every member of a group.
    if status
        .get("recipient_type")
        .is_some_and(|kind| kind.as_str() != Some("individual"))
        || status
            .get("recipient_participant_id")
            .is_some_and(|id| !id.is_null())
    {
        return None;
    }
    let message_id = status["id"].as_str().filter(|id| valid_message_id(id))?;
    let recipient_id = status["recipient_id"]
        .as_str()
        .filter(|id| valid_recipient(id))?;
    let kind = ReceiptStatus::parse(status["status"].as_str()?)?;
    let seconds = status["timestamp"]
        .as_str()?
        .parse::<i64>()
        .ok()
        .filter(|time| *time >= 0)?;
    let provider_at = chrono::DateTime::from_timestamp(seconds, 0)?;
    let error_codes = status["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .take(8)
        .filter_map(|error| {
            error["code"]
                .as_i64()
                .filter(|code| (0..=i64::from(i32::MAX)).contains(code))
        })
        .collect();
    Some(ReceiptObservation {
        message_id: message_id.into(),
        recipient_id: recipient_id.into(),
        status: kind,
        provider_at,
        error_codes,
    })
}

fn has_webhook_evidence(value: &Value) -> bool {
    value["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|message| {
            parse_message(message, &value["contacts"]).is_some_and(|message| {
                validate_id(&message.sender_platform_id, "Sender").is_ok()
                    && (message.text.as_deref().is_some_and(|text| !text.is_empty())
                        || !message.attachments.is_empty())
            })
        })
        || value["statuses"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|status| parse_receipt(status).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bot() -> ChannelBot {
        bson::from_document(bson::doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "user_id": uuid::Uuid::new_v4().to_string(),
            "platform": "whatsapp", "label": "WhatsApp", "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1, 2] },
            "platform_bot_id": "123456", "platform_bot_username": "+123456",
            "webhook_registered": false, "webhook_secret_hash": hex::encode(Sha256::digest(b"verify-token")),
            "status": "pending_webhook", "is_active": true,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        }).unwrap()
    }

    fn secrets() -> PlatformVerifySecrets {
        PlatformVerifySecrets::from([("app_secret", "app-secret")])
    }

    fn headers(body: &[u8]) -> HeaderMap {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"app-secret").unwrap();
        mac.update(body);
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-hub-signature-256",
            format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
                .parse()
                .unwrap(),
        );
        headers
    }

    fn message(kind: &str, content: Value) -> Value {
        let mut message = json!({"id": "wamid.test", "from": "15551234567", "type": kind, "timestamp": "1", "context": {"id": "wamid.parent"}});
        message[kind] = content;
        message
    }

    fn delivery(messages: Vec<Value>) -> Value {
        json!({"object": "whatsapp_business_account", "entry": [{"id": "654321", "changes": [{"field": "messages", "value": {
            "messaging_product": "whatsapp", "metadata": {"phone_number_id": "123456"},
            "contacts": [{"wa_id": "999", "profile": {"name": "Other"}}, {"wa_id": "15551234567", "profile": {"name": "Alice"}}],
            "messages": messages,
        }}]}]})
    }

    #[tokio::test]
    async fn signature_valid_invalid_missing_malformed_and_no_secret() {
        let body = br#"{"object":"whatsapp_business_account"}"#;
        let valid = headers(body);
        assert!(
            WhatsAppAdapter
                .verify_webhook(&bot(), Some(&secrets()), &valid, body)
                .await
                .is_ok()
        );
        assert!(
            WhatsAppAdapter
                .verify_webhook(&bot(), Some(&secrets()), &valid, b"tampered")
                .await
                .is_err()
        );
        assert!(
            WhatsAppAdapter
                .verify_webhook(&bot(), None, &valid, body)
                .await
                .is_err()
        );
        assert!(
            WhatsAppAdapter
                .verify_webhook(
                    &bot(),
                    Some(&PlatformVerifySecrets::default()),
                    &valid,
                    body
                )
                .await
                .is_err()
        );
        assert!(
            WhatsAppAdapter
                .verify_webhook(
                    &bot(),
                    Some(&PlatformVerifySecrets::from([("app_secret", "")])),
                    &valid,
                    body
                )
                .await
                .is_err()
        );
        assert!(
            WhatsAppAdapter
                .verify_webhook(&bot(), Some(&secrets()), &HeaderMap::new(), body)
                .await
                .is_err()
        );
        for malformed in [
            "sha256=deadbeef".to_string(),
            format!("sha256={}", "z".repeat(64)),
            format!("sha1={}", "0".repeat(64)),
            format!("sha256={}", "0".repeat(64)),
        ] {
            let mut invalid = HeaderMap::new();
            invalid.insert("x-hub-signature-256", malformed.parse().unwrap());
            assert!(
                WhatsAppAdapter
                    .verify_webhook(&bot(), Some(&secrets()), &invalid, body)
                    .await
                    .is_err()
            );
        }
    }

    #[test]
    fn subscription_handshake_valid_bad_token_and_missing_parameters() {
        let query: std::collections::HashMap<String, String> = [
            ("hub.mode", "subscribe"),
            ("hub.verify_token", "verify-token"),
            ("hub.challenge", "001234"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
        assert_eq!(
            WhatsAppAdapter
                .subscription_handshake(&bot(), &query)
                .unwrap(),
            "001234"
        );
        for key in ["hub.mode", "hub.verify_token", "hub.challenge"] {
            let mut missing = query.clone();
            missing.remove(key);
            assert!(matches!(
                WhatsAppAdapter.subscription_handshake(&bot(), &missing),
                Err(AppError::Forbidden(_))
            ));
            missing.insert(key.to_string(), String::new());
            assert!(
                WhatsAppAdapter
                    .subscription_handshake(&bot(), &missing)
                    .is_err()
            );
        }
        let mut bad = query.clone();
        bad.insert("hub.verify_token".to_string(), "wrong".to_string());
        assert!(matches!(
            WhatsAppAdapter.subscription_handshake(&bot(), &bad),
            Err(AppError::Forbidden(_))
        ));
        bad = query;
        bad.insert("hub.mode".to_string(), "unsubscribe".to_string());
        assert!(
            WhatsAppAdapter
                .subscription_handshake(&bot(), &bad)
                .is_err()
        );
    }

    #[tokio::test]
    async fn parse_text_contacts_context_and_multiple_messages() {
        let original = message("text", json!({"body": "Hello"}));
        let payload = delivery(vec![
            original.clone(),
            message("text", json!({"body": "Second"})),
        ]);
        let messages = WhatsAppAdapter
            .parse_inbound(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap();
        assert_eq!(messages.len(), 2);
        let first = &messages[0];
        assert_eq!(first.platform_message_id, "wamid.test");
        assert_eq!(first.conversation_id, "15551234567");
        assert_eq!(first.conversation_type, "private");
        assert_eq!(first.sender_display_name.as_deref(), Some("Alice"));
        assert_eq!(
            first.reply_to_platform_message_id.as_deref(),
            Some("wamid.parent")
        );
        assert_eq!(first.text.as_deref(), Some("Hello"));
        assert_eq!(first.content_type, "text");
        assert!(first.thread_id.is_none());
        assert_eq!(first.raw_data, original);
        assert_eq!(messages[1].text.as_deref(), Some("Second"));
    }

    #[tokio::test]
    async fn parse_all_media_categories_with_lookup_urls_and_captions() {
        for (kind, category) in [
            ("image", "image"),
            ("sticker", "image"),
            ("video", "video"),
            ("audio", "audio"),
            ("document", "file"),
        ] {
            let payload = delivery(vec![message(
                kind,
                json!({"id": "987654", "caption": "A caption", "filename": "media.bin", "mime_type": "application/octet-stream", "sha256": "hash"}),
            )]);
            let messages = WhatsAppAdapter
                .parse_inbound(&serde_json::to_vec(&payload).unwrap())
                .await
                .unwrap();
            assert_eq!(messages[0].content_type, category);
            assert_eq!(messages[0].text.as_deref(), Some("A caption"));
            let attachment = &messages[0].attachments[0];
            assert_eq!(attachment.content_type, category);
            assert_eq!(attachment.file_key.as_deref(), Some("987654"));
            assert_eq!(attachment.url, graph_url("987654"));
            assert_eq!(attachment.filename.as_deref(), Some("media.bin"));
            assert_eq!(
                attachment.mime_type.as_deref(),
                Some("application/octet-stream")
            );
        }
    }

    #[test]
    fn parse_buttons_interactive_location_contacts_and_order() {
        for (kind, payload, expected) in [
            ("button", json!({"text": "Yes", "payload": "yes-id"}), "Yes"),
            ("button", json!({"payload": "yes-id"}), "yes-id"),
            (
                "interactive",
                json!({"button_reply": {"title": "Button", "id": "b"}}),
                "Button",
            ),
            (
                "interactive",
                json!({"list_reply": {"title": "List", "id": "l"}}),
                "List",
            ),
            (
                "interactive",
                json!({"list_reply": {"id": "fallback"}}),
                "fallback",
            ),
            (
                "location",
                json!({"latitude": 1.25, "longitude": 103.5, "name": "Office", "address": "Street"}),
                "Location: 1.25, 103.5; Office; Street",
            ),
            (
                "contacts",
                json!([{ "name": {"formatted_name": "Bob"}, "phones": [{"phone": "+123"}] }]),
                "Contacts: Bob",
            ),
        ] {
            let raw = message(kind, payload);
            let parsed = parse_message(&raw, &Value::Null).unwrap();
            assert_eq!(parsed.content_type, "text");
            assert_eq!(parsed.text.as_deref(), Some(expected));
            assert_eq!(parsed.raw_data, raw);
        }
        let order = message("order", json!({"catalog_id": "123", "product_items": []}));
        assert_eq!(
            parse_message(&order, &Value::Null).unwrap().content_type,
            "unknown"
        );
        for kind in [
            "reaction",
            "system",
            "unknown",
            "unsupported",
            "future_type",
        ] {
            assert!(parse_message(&message(kind, json!({})), &Value::Null).is_none());
        }
    }

    #[tokio::test]
    async fn prepare_filters_other_phone_numbers_across_entries_and_changes() {
        let mut payload = delivery(vec![message("text", json!({"body": "Mine"}))]);
        let mut other = payload["entry"][0].clone();
        other["changes"][0]["value"]["metadata"]["phone_number_id"] = json!("999999");
        payload["entry"][0]["changes"]
            .as_array_mut()
            .unwrap()
            .push(other["changes"][0].clone());
        payload["entry"].as_array_mut().unwrap().push(other);
        let body = serde_json::to_vec(&payload).unwrap();
        let prepared = WhatsAppAdapter
            .prepare_webhook(&bot(), Some(&secrets()), &headers(&body), &body)
            .await
            .unwrap();
        assert_eq!(
            WhatsAppAdapter
                .parse_inbound(&prepared.body)
                .await
                .unwrap()
                .len(),
            1
        );
        let mut other_bot = bot();
        other_bot.platform_bot_id = "888888".to_string();
        let prepared = WhatsAppAdapter
            .prepare_webhook(&other_bot, Some(&secrets()), &headers(&body), &body)
            .await
            .unwrap();
        assert!(
            WhatsAppAdapter
                .parse_inbound(&prepared.body)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn statuses_errors_only_and_unexpected_shapes_do_not_panic() {
        let mut payload = delivery(vec![]);
        payload["entry"][0]["changes"][0]["value"]["statuses"] = json!([{"status": "delivered"}]);
        payload["entry"][0]["changes"][0]["value"]["errors"] = json!([{"code": 131026}]);
        for payload in [
            payload,
            Value::Null,
            json!(4),
            json!([]),
            json!({"entry": [null, 1, []]}),
            delivery(vec![
                json!({}),
                json!({"type":"image"}),
                message("image", json!({"id":"../../bad"})),
            ]),
        ] {
            let body = serde_json::to_vec(&payload).unwrap();
            let prepared = WhatsAppAdapter
                .prepare_webhook(&bot(), Some(&secrets()), &headers(&body), &body)
                .await
                .unwrap();
            let messages = WhatsAppAdapter.parse_inbound(&prepared.body).await.unwrap();
            assert!(
                messages
                    .iter()
                    .all(|message| message.attachments.is_empty())
            );
        }
        assert!(
            WhatsAppAdapter
                .parse_inbound(b"invalid json")
                .await
                .is_err()
        );
    }

    fn reply(text: Option<&str>, metadata: Option<Value>) -> OutboundReply {
        OutboundReply {
            attachments: vec![],
            text: text.map(String::from),
            reply_to_platform_message_id: Some("wamid.parent".to_string()),
            metadata,
        }
    }

    #[test]
    fn reply_recipient_normalizes_phone_formatting_but_rejects_invalid_ids() {
        for recipient in [
            "15551234567",
            "+15551234567",
            "+1 555-123-4567",
            "1 555 123 4567",
        ] {
            for content in [
                reply(Some("Hello"), None),
                reply(None, Some(json!({"template": {"name": "greeting"}}))),
            ] {
                assert_eq!(
                    reply_bodies(recipient, &content).unwrap()[0]["to"],
                    "15551234567"
                );
            }
        }
        for recipient in [
            "++15551234567",
            "1555+1234567",
            "abc",
            "+ - ",
            "1555/1234567",
            "(1555)1234567",
        ] {
            assert!(
                reply_bodies(recipient, &reply(Some("Hello"), None)).is_err(),
                "{recipient}"
            );
        }
    }

    #[test]
    fn reply_body_context_and_protected_routing_fields() {
        let bodies = reply_bodies(
            "15551234567",
            &reply(
                Some("Hello"),
                Some(json!({"to": "attacker", "messaging_product": "other"})),
            ),
        )
        .unwrap();
        assert_eq!(
            bodies,
            vec![
                json!({"messaging_product": "whatsapp", "recipient_type": "individual", "to": "15551234567", "type": "text", "text": {"preview_url": false, "body": "Hello"}, "context": {"message_id": "wamid.parent"}})
            ]
        );
        let mut top_level = reply(Some("Hello"), None);
        top_level.reply_to_platform_message_id = None;
        assert!(
            reply_bodies("15551234567", &top_level).unwrap()[0]
                .get("context")
                .is_none()
        );
    }

    #[test]
    fn template_and_interactive_metadata_can_send_without_text() {
        for (kind, content) in [
            (
                "template",
                json!({"name": "hello_world", "language": {"code": "en_US"}}),
            ),
            (
                "interactive",
                json!({"type": "button", "body": {"text": "Choose"}}),
            ),
        ] {
            let mut metadata = json!({"to": "attacker", "messaging_product": "other"});
            metadata[kind] = content.clone();
            assert!(WhatsAppAdapter.supports_reply_metadata(&metadata));
            let bodies = reply_bodies("12345", &reply(None, Some(metadata))).unwrap();
            assert_eq!(bodies.len(), 1);
            assert_eq!(bodies[0]["type"], kind);
            assert_eq!(bodies[0][kind], content);
            assert_eq!(bodies[0]["to"], "12345");
            assert_eq!(bodies[0]["messaging_product"], "whatsapp");
        }
        assert!(!WhatsAppAdapter.supports_reply_metadata(&json!({"template": "not an object"})));
        assert!(
            reply_bodies(
                "12345",
                &reply(None, Some(json!({"template": {}, "interactive": {}})))
            )
            .is_err()
        );
        assert!(reply_bodies("12345", &reply(None, None)).is_err());
    }

    #[test]
    fn chunks_at_4096_unicode_characters_without_losing_content() {
        for length in [1, 4095, 4096, 4097, 8193] {
            let text = "\u{1f600}".repeat(length);
            let bodies = reply_bodies("12345", &reply(Some(&text), None)).unwrap();
            assert_eq!(bodies.len(), length.div_ceil(TEXT_LIMIT));
            let parts: Vec<&str> = bodies
                .iter()
                .map(|body| body["text"]["body"].as_str().unwrap())
                .collect();
            assert!(parts.iter().all(|text| text.chars().count() <= TEXT_LIMIT));
            assert_eq!(parts.concat(), text);
        }
    }

    #[test]
    fn graph_errors_are_actionable_rate_limited_and_redacted() {
        for (status, code) in [
            (StatusCode::TOO_MANY_REQUESTS, 0),
            (StatusCode::BAD_REQUEST, 130429),
            (StatusCode::BAD_REQUEST, 80007),
        ] {
            let error =
                graph_error(status, &json!({"error": {"code": code}}), Some("17")).to_string();
            assert!(error.contains("rate limited"));
            assert!(error.contains("wait 17s"));
        }
        for (code, cause) in [
            (131047, "24-hour"),
            (131026, "not reachable"),
            (131051, "unsupported message"),
            (131053, "media upload"),
            (190, "invalid or expired"),
            (1234, "Graph API request failed"),
        ] {
            let error = graph_error(StatusCode::BAD_REQUEST, &json!({"error": {"code": code, "message": "SECRET-TOKEN", "type": "SECRET-TOKEN", "fbtrace_id": "SECRET-TOKEN"}}), None).to_string();
            assert!(error.contains(cause));
            assert!(!error.contains("SECRET-TOKEN"));
        }
        assert!(
            graph_error(StatusCode::TOO_MANY_REQUESTS, &Value::Null, None)
                .to_string()
                .contains("wait before another request")
        );
    }

    #[tokio::test]
    async fn adapter_policies_and_default_edit() {
        assert!(matches!(
            WhatsAppAdapter.webhook_policy(b"anything"),
            WebhookPolicy::Immediate(None)
        ));
        assert!(!WhatsAppAdapter.registration().automatic_webhook);
        assert_eq!(
            WhatsAppAdapter.registration().webhook_secret_label,
            Some("Verify Token")
        );
        let edit = crate::services::channel_platform::OutboundEdit {
            text: Some("edit".to_string()),
            metadata: None,
        };
        assert!(matches!(
            WhatsAppAdapter
                .edit_reply(
                    &reqwest::Client::new(),
                    &"token".into(),
                    "chat",
                    "wamid",
                    &edit
                )
                .await,
            Err(AppError::ChannelPlatformEditUnsupported)
        ));
    }
    #[test]
    fn initiated_request_has_no_reply_or_thread_context() {
        let outbound = OutboundReply {
            attachments: vec![],
            text: Some("hello".into()),
            reply_to_platform_message_id: None,
            metadata: None,
        };
        assert_eq!(
            reply_bodies("123456", &outbound).unwrap(),
            vec![
                json!({ "messaging_product": "whatsapp", "recipient_type": "individual", "to": "123456", "type": "text", "text": { "preview_url": false, "body": "hello" } })
            ]
        );
        for code in [131047, 131026] {
            assert!(matches!(
                graph_error(
                    StatusCode::BAD_REQUEST,
                    &json!({ "error": { "code": code, "message": "private content" } }),
                    None
                ),
                AppError::ChannelConversationNotReachable(_)
            ));
        }
        let error = graph_error(
            StatusCode::BAD_REQUEST,
            &json!({ "error": { "code": 131047 } }),
            None,
        );
        assert!(error.to_string().contains("template message is required"));
    }
}

#[cfg(test)]
mod media_tests {
    use super::*;
    use crate::services::channel_platform::MaterializedAttachment;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, header, method, path},
    };
    #[tokio::test]
    async fn channel_whatsapp_media_download_and_upload_contracts() {
        let server = MockServer::start().await;
        let http = reqwest::Client::new();
        let credentials = BotCredentials {
            billing: None,
            token: "token",
            platform_bot_id: Some("123"),
            platform_secrets: None,
        };
        Mock::given(method("GET"))
            .and(path("/456"))
            .and(header("Authorization", "Bearer token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"url":format!("{}/download",server.uri())})),
            )
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/download"))
            .and(header("Authorization", "Bearer token"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"document".to_vec()))
            .expect(2)
            .mount(&server)
            .await;
        let mut attachment = InboundAttachment {
            content_type: "file".into(),
            url: format!("{}/456", server.uri()),
            platform_message_id: None,
            file_key: Some("456".into()),
            image_key: None,
            filename: None,
            mime_type: None,
            size_bytes: None,
        };
        assert_eq!(
            fetch_media_at(&http, &credentials, &attachment, 20, &server.uri())
                .await
                .unwrap()
                .bytes,
            b"document"[..]
        );
        assert!(matches!(
            fetch_media_at(&http, &credentials, &attachment, 2, &server.uri()).await,
            Err(AppError::ChannelMediaTooLarge)
        ));
        attachment.url = "https://evil.example/media".into();
        assert!(matches!(
            fetch_media_at(&http, &credentials, &attachment, 20, &server.uri()).await,
            Err(AppError::ChannelMediaFetchFailed(_))
        ));
        for (kind, native) in [
            (MediaKind::Image, "image"),
            (MediaKind::File, "document"),
            (MediaKind::Audio, "audio"),
            (MediaKind::Video, "video"),
        ] {
            Mock::given(method("POST"))
                .and(path("/123/media"))
                .and(header("Authorization", "Bearer token"))
                .and(body_string_contains("media-bytes"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"789"})))
                .up_to_n_times(1)
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/123/messages"))
                .and(body_string_contains(native))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"messages":[{"id":"last"}]})),
                )
                .up_to_n_times(1)
                .expect(1)
                .mount(&server)
                .await;
            let reply = OutboundReply {
                text: None,
                metadata: None,
                reply_to_platform_message_id: Some("parent".into()),
                attachments: vec![MaterializedAttachment {
                    kind,
                    bytes: bytes::Bytes::from_static(b"media-bytes"),
                    filename: Some("report.pdf".into()),
                    mime_type: Some("application/pdf".into()),
                    caption: None,
                }],
            };
            assert_eq!(
                send_media_at(&http, &credentials, "234", &reply, &server.uri())
                    .await
                    .unwrap()
                    .as_deref(),
                Some("last")
            );
        }
        server.verify().await;
    }
}

#[cfg(test)]
mod verification_tests {
    use super::*;
    use std::time::Duration;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };

    #[tokio::test]
    async fn identity_request_checks_phone_and_sanitizes_provider_failures() {
        let server = MockServer::start().await;
        let http = reqwest::Client::new();
        let credentials = BotCredentials {
            billing: None,
            token: "fake-access-token",
            platform_bot_id: Some("123"),
            platform_secrets: None,
        };
        for (body, valid) in [
            (json!({"id":"123", "display_phone_number":"+123"}), true),
            (json!({"id":"456"}), false),
        ] {
            let mock = Mock::given(method("GET"))
                .and(path("/v25.0/123"))
                .and(header("authorization", "Bearer fake-access-token"))
                .and(query_param(
                    "fields",
                    "id,display_phone_number,verified_name",
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(1)
                .mount_as_scoped(&server)
                .await;
            let result = verify_identity_at(&http, &credentials, &server.uri()).await;
            assert_eq!(result.is_ok(), valid);
            drop(mock);
        }
        for (status, body, expected) in [
            (
                400,
                json!({"error":{"code":190,"message":"fake-access-token"}}).to_string(),
                "invalid or expired",
            ),
            (
                403,
                json!({"error":{"code":200,"message":"fake-access-token"}}).to_string(),
                "permissions",
            ),
            (502, "fake-access-token: gateway failure".into(), "HTTP 502"),
            (
                200,
                "fake-access-token: invalid json".into(),
                "invalid JSON",
            ),
        ] {
            let mock = Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .expect(1)
                .mount_as_scoped(&server)
                .await;
            let error = verify_identity_at(&http, &credentials, &server.uri())
                .await
                .err()
                .unwrap();
            let response = error.response_body();
            let json = serde_json::to_string(&response).unwrap();
            assert!(json.contains(expected), "unexpected safe response: {json}");
            assert!(!json.contains("fake-access-token"));
            drop(mock);
        }
    }

    #[tokio::test]
    async fn graph_requests_bound_slow_headers_and_declared_response_size() {
        let server = MockServer::start().await;
        Mock::given(path("/slow"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(2))
                    .set_body_json(json!({})),
            )
            .mount(&server)
            .await;
        Mock::given(path("/large"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("x".repeat(GRAPH_JSON_LIMIT + 1)),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let started = std::time::Instant::now();
        let error = graph_send_with_timeout(
            http.get(format!("{}/slow", server.uri())),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(1));
        let error = graph_send(http.get(format!("{}/large", server.uri())))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("size limit"));
    }

    #[tokio::test]
    async fn graph_requests_bound_chunked_sizes_and_stalled_bodies() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for oversized in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 4096];
                let _ = stream.read(&mut buffer).await.unwrap();
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                    .await
                    .unwrap();
                if oversized {
                    let chunk = "x".repeat(GRAPH_JSON_LIMIT + 1);
                    let _ = stream
                        .write_all(
                            format!("{:x}\r\n{}\r\n0\r\n\r\n", chunk.len(), chunk).as_bytes(),
                        )
                        .await;
                } else {
                    stream.write_all(b"1\r\n{\r\n").await.unwrap();
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            });
            let error = graph_send_with_timeout(
                reqwest::Client::new().get(format!("http://{address}")),
                Duration::from_millis(100),
            )
            .await
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(if oversized { "size limit" } else { "timed out" })
            );
            task.abort();
        }
    }

    #[test]
    fn webhook_evidence_requires_nonempty_message_and_status_identifiers() {
        let message = json!({"messages":[{"id":"wamid.1","from":"123","type":"text","text":{"body":"hello"}}]});
        assert!(has_webhook_evidence(&message));
        for field in ["id", "from"] {
            let mut invalid = message.clone();
            invalid["messages"][0][field] = json!("");
            assert!(!has_webhook_evidence(&invalid));
        }
        let status = json!({"statuses":[{"id":"wamid.1","recipient_id":"123","status":"delivered","timestamp":"1750000000"}]});
        assert!(has_webhook_evidence(&status));
        for field in ["id", "recipient_id", "status", "timestamp"] {
            let mut invalid = status.clone();
            invalid["statuses"][0][field] = json!("");
            assert!(!has_webhook_evidence(&invalid));
        }
        assert!(!has_webhook_evidence(
            &json!({"messages":[],"statuses":[],"errors":[{"code":190}]})
        ));
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    fn credentials() -> BotCredentials<'static> {
        BotCredentials {
            billing: None,
            token: "token",
            platform_bot_id: Some("123"),
            platform_secrets: None,
        }
    }
    fn reply() -> OutboundReply {
        OutboundReply {
            text: Some("hello".into()),
            attachments: vec![],
            reply_to_platform_message_id: None,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn channel_whatsapp_collects_split_text_media_and_audio_caption_ids() {
        use crate::services::channel_platform::MaterializedAttachment;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/123/media"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"456"})))
            .expect(1)
            .mount(&server)
            .await;
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = counter.clone();
        Mock::given(method("POST"))
            .and(path("/123/messages"))
            .respond_with(move |_: &wiremock::Request| {
                let index = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                ResponseTemplate::new(200)
                    .set_body_json(json!({"messages":[{"id":format!("wamid.{index}")}]}))
            })
            .expect(5)
            .mount(&server)
            .await;
        let mut reply = reply();
        reply.text = Some("t".repeat(TEXT_LIMIT + 1));
        reply.attachments.push(MaterializedAttachment {
            kind: MediaKind::Audio,
            bytes: bytes::Bytes::from_static(b"audio"),
            filename: None,
            mime_type: Some("audio/ogg".into()),
            caption: Some("c".repeat(TEXT_LIMIT + 1)),
        });
        let outcome = send_outcome_at(
            &reqwest::Client::new(),
            &credentials(),
            "+789",
            &reply,
            &server.uri(),
        )
        .await
        .unwrap();
        assert!(outcome.error.is_none());
        assert_eq!(outcome.final_id.as_deref(), Some("wamid.4"));
        let send = outcome.observation.unwrap();
        assert_eq!(
            send.component_ids,
            vec!["wamid.0", "wamid.1", "wamid.2", "wamid.3", "wamid.4"]
        );
        assert_eq!(send.expected_components, 5);
        assert!(send.complete);
        assert!(!send.uncertain);
        let requests = server.received_requests().await.unwrap();
        let messages: Vec<Value> = requests
            .iter()
            .filter(|request| request.url.path().ends_with("/messages"))
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect();
        assert_eq!(
            messages
                .iter()
                .map(|body| body["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["text", "text", "text", "text", "audio"]
        );
        assert!(
            messages
                .iter()
                .all(|body| body["recipient_type"] == "individual" && body["to"] == "789")
        );
        assert!(messages[4]["audio"].get("caption").is_none());
        server.verify().await;
    }

    #[tokio::test]
    async fn channel_whatsapp_confirmed_first_refusals_are_proven_unsent() {
        for (status, code) in [(400, 190), (400, 131030), (400, 131047), (429, 130429)] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/123/messages"))
                .respond_with(ResponseTemplate::new(status).set_body_json(
                    json!({"error":{"code":code,"message":"private provider prose token"}}),
                ))
                .expect(1)
                .mount(&server)
                .await;
            let result = send_outcome_at(
                &reqwest::Client::new(),
                &credentials(),
                "789",
                &reply(),
                &server.uri(),
            )
            .await;
            let error = result
                .err()
                .expect("confirmed first refusal allows claim release");
            assert!(!error.to_string().contains("private provider prose"));
            server.verify().await;
        }
    }

    #[tokio::test]
    async fn channel_whatsapp_partial_refusal_preserves_evidence_and_certainty() {
        for status in [400, 503] {
            let server = MockServer::start().await;
            let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            Mock::given(method("POST"))
                .and(path("/123/messages"))
                .respond_with(move |_: &wiremock::Request| {
                    if counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                        ResponseTemplate::new(200)
                            .set_body_json(json!({"messages":[{"id":"wamid.accepted"}]}))
                    } else {
                        ResponseTemplate::new(status)
                            .set_body_json(json!({"error":{"code":190,"message":"private"}}))
                    }
                })
                .expect(2)
                .mount(&server)
                .await;
            let mut reply = reply();
            reply.text = Some("t".repeat(TEXT_LIMIT * 2 + 1));
            let outcome = send_outcome_at(
                &reqwest::Client::new(),
                &credentials(),
                "789",
                &reply,
                &server.uri(),
            )
            .await
            .unwrap();
            assert!(outcome.error.is_some());
            assert_eq!(outcome.final_id.as_deref(), Some("wamid.accepted"));
            let record = outcome.observation.unwrap();
            assert_eq!(record.component_ids, vec!["wamid.accepted"]);
            assert_eq!(record.expected_components, 3);
            assert!(!record.complete);
            assert_eq!(record.uncertain, status == 503);
        }
    }

    #[tokio::test]
    async fn channel_whatsapp_ambiguous_success_and_server_errors_retain_unknown_outcome() {
        for response in [
            ResponseTemplate::new(200).set_body_json(json!({"messages":[{}]})),
            ResponseTemplate::new(200).set_body_json(json!({"messages":[{"id":"x".repeat(513)}]})),
            ResponseTemplate::new(200).set_body_string("invalid JSON private content"),
            ResponseTemplate::new(200).set_body_string("x".repeat(GRAPH_JSON_LIMIT + 1)),
            ResponseTemplate::new(503)
                .set_body_json(json!({"error":{"code":190,"message":"private"}})),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/123/messages"))
                .respond_with(response)
                .expect(1)
                .mount(&server)
                .await;
            let outcome = send_outcome_at(
                &reqwest::Client::new(),
                &credentials(),
                "789",
                &reply(),
                &server.uri(),
            )
            .await
            .unwrap();
            assert!(
                !outcome
                    .error
                    .as_ref()
                    .unwrap()
                    .to_string()
                    .contains("private")
            );
            let record = outcome.observation.unwrap();
            assert!(record.uncertain);
            assert!(!record.complete);
            assert!(record.component_ids.is_empty());
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let outcome = send_outcome_at(
            &reqwest::Client::new(),
            &credentials(),
            "789",
            &reply(),
            &format!("http://{address}"),
        )
        .await
        .unwrap();
        assert!(outcome.observation.unwrap().uncertain);
        assert!(
            outcome
                .error
                .unwrap()
                .to_string()
                .contains("before sending again")
        );
    }

    #[test]
    fn channel_whatsapp_receipt_parser_bounds_codes_and_rejects_participants() {
        let mut status = json!({"id":"wamid.one", "recipient_id":"789", "status":"failed", "timestamp":"1700000000",
            "errors":[{"code":131026,"message":"secret", "error_data":{"details":"private"}}, {"code":-1}, {"code":"190"}, {"code":i64::MAX}]});
        let parsed = parse_receipt(&status).unwrap();
        assert_eq!(parsed.error_codes, vec![131026]);
        for group in [
            json!({"recipient_type":"group"}),
            json!({"recipient_participant_id":"999"}),
        ] {
            let mut bad = status.clone();
            bad.as_object_mut()
                .unwrap()
                .extend(group.as_object().unwrap().clone());
            assert!(parse_receipt(&bad).is_none());
        }
        status["timestamp"] = json!(i64::MAX.to_string());
        assert!(parse_receipt(&status).is_none());
    }
}
