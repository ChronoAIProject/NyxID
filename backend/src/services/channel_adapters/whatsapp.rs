//! WhatsApp Business Platform, directly through Meta's Cloud API.
//! Consumer/Business App automation and Twilio's separate API are not supported.

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
    let message_id = message["id"].as_str().filter(|value| !value.is_empty())?;
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
        131047 => "outside the 24-hour customer service window; send an approved template",
        131026 => "message undeliverable; check the recipient and their WhatsApp availability",
        131051 => "unsupported message type",
        131053 => "media upload failed; check media format and size",
        190 => "access token is invalid or expired",
        10 | 200 => "required WhatsApp permissions are missing",
        _ => "Graph API request failed",
    };
    if status == StatusCode::TOO_MANY_REQUESTS || matches!(code, 130429 | 80007) {
        let retry = retry_after
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(|seconds| seconds.min(3600));
        return AppError::ChannelPlatformError(match retry {
            Some(seconds) => format!("WhatsApp rate limited; retry after {seconds}s"),
            None => "WhatsApp rate limited; retry later".to_string(),
        });
    }
    // Graph's free-form error.message can echo request material. Only emit
    // locally-authored causes and numeric codes, never that message or tokens.
    AppError::ChannelPlatformError(format!(
        "WhatsApp: {reason} (HTTP {}, code {code})",
        status.as_u16()
    ))
}

pub(super) async fn graph_response(response: reqwest::Response) -> AppResult<Value> {
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .map(String::from);
    let body = response.json::<Value>().await;
    if !status.is_success() {
        return Err(graph_error(
            status,
            &body.unwrap_or(Value::Null),
            retry_after.as_deref(),
        ));
    }
    let body = body.map_err(|_| {
        AppError::ChannelPlatformError("WhatsApp returned invalid JSON".to_string())
    })?;
    if body.get("error").is_some() {
        return Err(graph_error(status, &body, retry_after.as_deref()));
    }
    Ok(body)
}

#[async_trait::async_trait]
impl PlatformAdapter for WhatsAppAdapter {
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
            extra_fields: &[],
            unsupported_patch_message: None,
            create_response_status: "pending_webhook",
            preserve_subscription_on_verify: true,
            fields: &[
                RegistrationField {
                    label: "Access token",
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
                    platform_fallback: None,
                },
            ],
            webhook_secret_label: Some("Verify Token"),
            setup_instructions: &[
                "In Meta App Dashboard > WhatsApp > Configuration, enter the Callback URL and Verify Token shown here, then verify and save.",
                "Subscribe to the messages webhook field. Subscribe this app to the WhatsApp Business Account (POST /{WABA_ID}/subscribed_apps with a system user token).",
                "Use a permanent System User access token with whatsapp_business_messaging and whatsapp_business_management permissions and access to the phone number.",
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
        if let Some(entries) = payload.get_mut("entry").and_then(Value::as_array_mut) {
            for entry in entries {
                if let Some(changes) = entry.get_mut("changes").and_then(Value::as_array_mut) {
                    changes.retain(|change| {
                        change["value"]["metadata"]["phone_number_id"].as_str()
                            == Some(bot.platform_bot_id.as_str())
                    });
                }
            }
        }
        Ok(PreparedWebhook {
            body: serde_json::to_vec(&payload)
                .map_err(|_| AppError::Internal("Unable to prepare webhook".to_string()))?,
            challenge_response: None,
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

    async fn send_reply(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        conversation_id: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        let business_object_id = credentials.platform_bot_id.unwrap_or_default();
        validate_id(business_object_id, "Phone Number ID")?;
        let url = format!("{}/messages", graph_url(business_object_id));
        let mut last_id = None;
        for body in reply_bodies(conversation_id, reply)? {
            let response =
                super::whatsapp_managed::authenticate(http.post(&url).json(&body), credentials)?
                    .send()
                    .await
                    .map_err(|_| {
                        AppError::ChannelPlatformError("WhatsApp send request failed".to_string())
                    })?;
            let response = graph_response(response).await?;
            last_id = Some(
                response["messages"][0]["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| {
                        AppError::ChannelPlatformError(
                            "WhatsApp response is missing a message ID".to_string(),
                        )
                    })?
                    .to_string(),
            );
        }
        Ok(last_id)
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
        let business_object_id = credentials.platform_bot_id.unwrap_or_default();
        validate_id(business_object_id, "Phone Number ID")?;
        let response = super::whatsapp_managed::authenticate(
            http.get(graph_url(business_object_id))
                .query(&[("fields", "id,display_phone_number,verified_name")]),
            credentials,
        )?
        .send()
        .await
        .map_err(|_| {
            AppError::ChannelPlatformError("WhatsApp identity verification failed".to_string())
        })?;
        let response = graph_response(response).await?;
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
            assert!(error.contains("retry after 17s"));
        }
        for (code, cause) in [
            (131047, "24-hour"),
            (131026, "undeliverable"),
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
                .contains("retry later")
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
                .edit_reply(&reqwest::Client::new(), "token", "wamid", &edit)
                .await,
            Err(AppError::ChannelPlatformEditUnsupported)
        ));
    }
}
