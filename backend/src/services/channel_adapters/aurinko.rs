//! Aurinko's documented account-token email and signed notification contracts.
//! Production requests have one fixed origin and never follow redirects.

use crate::models::channel_thread::{ChannelThreadFacts, ThreadKind, ThreadSenderKind};
use crate::services::{
    channel_platform::*,
    channel_registration::{BOT_TOKEN_FIELD, RegistrationField},
    channel_retry_ingress::{self, IngressContext},
    coordination_service::{EventDedupClaimResult, EventDedupStore},
};
use crate::{
    errors::{AppError, AppResult},
    models::{channel_bot::ChannelBot, channel_message::ChannelMessage},
};
use axum::http::HeaderMap;
use bson::doc;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::LazyLock, time::Duration};
use zeroize::Zeroizing;

const ORIGIN: &str = "https://api.aurinko.io";
const MAX_RESPONSE: usize = 2 * 1024 * 1024;
const MAX_BODY: usize = 256 * 1024;
const MAX_BATCH: usize = 1000;
// Notifications may be re-sent with their original signature. Do not use a
// five-minute replay cutoff; completed message claims outlive this window.
const MAX_SIGNATURE_AGE: i64 = 30 * 86400;
const COMMIT_TTL: Duration = Duration::from_secs(31 * 86400);
static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("fixed Aurinko HTTP client")
});

#[derive(Default)]
pub struct AurinkoAdapter {
    #[cfg(test)]
    origin: Option<String>,
}

fn invalid() -> AppError {
    AppError::ChannelWebhookVerificationFailed("Invalid Aurinko notification".into())
}
fn upstream() -> AppError {
    AppError::ChannelPlatformError(
        "Aurinko request failed; check account permissions and retry".into(),
    )
}
fn malformed() -> AppError {
    AppError::BadRequest("Malformed Aurinko notification".into())
}
fn identity(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => number.as_u64().filter(|v| *v > 0).map(|v| v.to_string()),
        Value::String(value)
            if !value.is_empty()
                && value.len() <= 20
                && value.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Some(value.clone())
        }
        _ => None,
    }
}
fn opaque(value: &str) -> AppResult<&str> {
    if matches!(value, "" | "." | "..") || value.len() > 2048 || value.chars().any(char::is_control)
    {
        return Err(malformed());
    }
    Ok(value)
}
fn address(value: &Value) -> Option<&str> {
    value["address"].as_str().filter(|v| {
        v.len() <= 320
            && v.contains('@')
            && !v
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || matches!(c, ',' | ';' | '<' | '>'))
    })
}

fn addresses(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            entry
                .as_str()
                .or_else(|| entry["address"].as_str())
                .filter(|value| address(&json!({"address": value})).is_some())
                .map(|value| value.to_ascii_lowercase())
        })
        .collect()
}

fn header(message: &Value, name: &str) -> Option<String> {
    message["internetHeaders"]
        .as_array()?
        .iter()
        .find_map(|entry| {
            (entry["name"].as_str()?.eq_ignore_ascii_case(name))
                .then(|| {
                    entry["value"]
                        .as_str()
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                })
                .flatten()
                .map(str::to_owned)
        })
}

fn reply_parent(message: &Value) -> Option<String> {
    let value = message["inReplyTo"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| header(message, "in-reply-to"))?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn hash_email(address: &str, key: Option<&[u8]>) -> Option<String> {
    crate::services::audit_service::keyed_fingerprint(
        key,
        b"email-participant",
        address.trim().to_ascii_lowercase().as_bytes(),
    )
}

impl AurinkoAdapter {
    fn origin(&self) -> &str {
        #[cfg(test)]
        if let Some(origin) = &self.origin {
            return origin;
        }
        ORIGIN
    }
    fn url(&self, segments: &[&str]) -> AppResult<reqwest::Url> {
        let mut url = reqwest::Url::parse(self.origin()).map_err(|_| upstream())?;
        let mut path = url.path_segments_mut().map_err(|_| upstream())?;
        path.clear();
        path.push("v1");
        for segment in segments {
            path.push(opaque(segment)?);
        }
        drop(path);
        Ok(url)
    }
    async fn request(
        &self,
        method: Method,
        token: &str,
        segments: &[&str],
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> AppResult<(StatusCode, Value)> {
        let mut request = HTTP
            .request(method, self.url(segments)?)
            .bearer_auth(token)
            .query(query);
        if let Some(body) = body {
            request = request.json(body);
        }
        let mut response = request.send().await.map_err(|_| upstream())?;
        let status = response.status();
        // Provider error bodies can contain addresses, message bodies or tokens.
        if !status.is_success() {
            return Ok((status, Value::Null));
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE as u64)
        {
            return Ok((StatusCode::PAYLOAD_TOO_LARGE, Value::Null));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| upstream())? {
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                return Ok((StatusCode::PAYLOAD_TOO_LARGE, Value::Null));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).map_err(|_| upstream())?
        };
        Ok((status, value))
    }
    async fn account(&self, token: &str, expected: Option<&str>) -> AppResult<Value> {
        let (status, value) = self
            .request(
                Method::GET,
                token,
                &["account"],
                &[("pingProvider", "true")],
                None,
            )
            .await?;
        let id = identity(&value["id"]).ok_or_else(upstream)?;
        if !status.is_success() || value["tokenStatus"].as_str() != Some("active") {
            return Err(upstream());
        }
        if expected.is_some_and(|expected| expected != id) {
            return Err(AppError::ValidationError(
                "Account token belongs to a different Aurinko mailbox; create a separate bot"
                    .into(),
            ));
        }
        let scopes = value["authScopes"].as_array().ok_or_else(upstream)?;
        let has = |scope| scopes.iter().any(|value| value.as_str() == Some(scope));
        if !(has("Mail.All") || ((has("Mail.Read") || has("Mail.ReadWrite")) && has("Mail.Send"))) {
            return Err(AppError::ValidationError(
                "Aurinko channel bots require Mail.Read and Mail.Send (or Mail.All)".into(),
            ));
        }
        if mailbox_addresses(&value).is_empty() {
            return Err(upstream());
        }
        Ok(value)
    }
    async fn message(&self, token: &str, id: &str) -> AppResult<Option<Value>> {
        let (status, value) = self
            .request(
                Method::GET,
                token,
                &["email", "messages", id],
                &[
                    ("bodyType", "text"),
                    ("stripQuoted", "true"),
                    ("loadInlines", "false"),
                    ("requireThreadId", "true"),
                ],
                None,
            )
            .await?;
        if matches!(
            status,
            StatusCode::NOT_FOUND | StatusCode::PAYLOAD_TOO_LARGE
        ) {
            return Ok(None);
        }
        // 408 (thread pending), 429 and 5xx must leave the event retryable.
        if !status.is_success() {
            return Err(upstream());
        }
        if value["id"].as_str() != Some(id) {
            return Err(upstream());
        }
        Ok(Some(value))
    }
}

fn conversation_id(account: &str, thread: &str) -> String {
    format!(
        "{account}:{}",
        hex::encode(Sha256::digest(thread.as_bytes()))
    )
}

fn mailbox_addresses(account: &Value) -> Vec<String> {
    ["email", "email2", "mailboxAddress"]
        .iter()
        .filter_map(|key| account[key].as_str())
        .filter(|value| value.contains('@'))
        .map(|value| value.to_ascii_lowercase())
        .collect()
}

fn verify_signature(
    bot: &ChannelBot,
    secret: &str,
    headers: &HeaderMap,
    body: &[u8],
    now: i64,
) -> AppResult<()> {
    let timestamp = headers
        .get("x-aurinko-request-timestamp")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(invalid)?;
    if timestamp.len() > 12 || !timestamp.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let seconds: i64 = timestamp.parse().map_err(|_| invalid())?;
    if seconds < now - MAX_SIGNATURE_AGE
        || seconds < bot.created_at.timestamp() - 300
        || seconds > now + 300
    {
        return Err(invalid());
    }
    let signature = headers
        .get("x-aurinko-signature")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() == 64)
        .ok_or_else(invalid)?;
    let digest = hex::decode(signature).map_err(|_| invalid())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| invalid())?;
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(body);
    mac.verify_slice(&digest).map_err(|_| invalid())
}

fn automated(message: &Value) -> bool {
    if message["meetingMessageMethod"].is_string() {
        return true;
    }
    let sender = address(&message["from"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    if [
        "mailer-daemon@",
        "postmaster@",
        "no-reply@",
        "noreply@",
        "do-not-reply@",
    ]
    .iter()
    .any(|prefix| sender.starts_with(prefix))
    {
        return true;
    }
    message["internetHeaders"]
        .as_array()
        .is_some_and(|headers| {
            headers.iter().any(|header| {
                let name = header["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let value = header["value"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                name.starts_with("list-")
                    || name == "x-nyxid-auto-reply"
                    || name == "x-autoreply"
                    || name == "x-autorespond"
                    || (name == "auto-submitted" && value != "no")
                    || (name == "precedence" && matches!(value.as_str(), "bulk" | "list" | "junk"))
                    || (name == "return-path" && value == "<>")
                    || (name == "content-type"
                        && (value.contains("delivery-status")
                            || value.contains("disposition-notification")
                            || value.contains("multipart/report")))
            })
        })
}

fn normalize(
    bot: &ChannelBot,
    account: &Value,
    message: &Value,
) -> AppResult<Option<InboundMessage>> {
    normalize_with_key(
        bot,
        account,
        message,
        crate::services::audit_service::audit_chain_hmac_key(),
    )
}

fn normalize_with_key(
    bot: &ChannelBot,
    account: &Value,
    message: &Value,
    audit_key: Option<&[u8]>,
) -> AppResult<Option<InboundMessage>> {
    let labels = message["sysLabels"].as_array().ok_or_else(upstream)?;
    if labels
        .iter()
        .any(|v| matches!(v.as_str(), Some("sent" | "draft" | "junk" | "trash")))
    {
        return Ok(None);
    }
    let received: DateTime<Utc> = message["receivedAt"]
        .as_str()
        .ok_or_else(upstream)?
        .parse()
        .map_err(|_| upstream())?;
    if received < bot.created_at {
        return Ok(None);
    }
    let Some(sender) = address(&message["from"]) else {
        return Ok(None);
    };
    if mailbox_addresses(account)
        .iter()
        .any(|a| a.eq_ignore_ascii_case(sender))
        || automated(message)
    {
        return Ok(None);
    }
    if message["omitted"].as_array().is_some_and(|values| {
        values.iter().any(|v| {
            matches!(
                v.as_str(),
                Some("threadId" | "body" | "internetHeaders" | "recipients")
            )
        })
    }) {
        return Err(upstream());
    }
    let Some(thread) = message["threadId"].as_str() else {
        return Err(upstream());
    };
    if opaque(thread).is_err() {
        return Ok(None);
    }
    if !message["internetHeaders"].is_array() {
        return Err(upstream());
    }
    let id = opaque(message["id"].as_str().ok_or_else(upstream)?)?;
    let body = message["body"].as_str().ok_or_else(upstream)?;
    let subject = message["subject"].as_str().unwrap_or("(no subject)");
    if body.len() > MAX_BODY || subject.len() > 4096 {
        return Ok(None);
    }
    // Attachment bytes and recipient lists are not forwarded automatically.
    // The AI Service offers authenticated attachment reads under its own scope.
    let mailbox = mailbox_addresses(account);
    let from = sender.to_ascii_lowercase();
    let to = addresses(&message["to"]);
    let cc = addresses(&message["cc"]);
    let mut participants = vec![from.clone()];
    participants.extend(to.iter().cloned());
    participants.extend(cc.iter().cloned());
    participants.sort();
    participants.dedup();
    participants.truncate(64);
    let parent = reply_parent(message).filter(|value| opaque(value).is_ok());
    let mut raw_data = json!({"account_id": bot.platform_bot_id, "message_id": id, "thread_id": thread, "subject": subject, "has_attachments": message["hasAttachments"].as_bool().unwrap_or(false)});
    // Fingerprints are additive follow metadata. If the process has not yet
    // initialized its audit key, keep the legacy message intact and let the
    // thread-facts adapter return None.
    if let (Some(sender_hash), Some(participant_hashes)) = (
        hash_email(&from, audit_key),
        participants
            .iter()
            .map(|p| hash_email(p, audit_key))
            .collect::<Option<Vec<_>>>(),
    ) {
        raw_data["email"] = json!({"account_id": bot.platform_bot_id, "thread_id": thread,
            "reply_parent_id": parent, "mailbox_to": to.iter().any(|address| mailbox.iter().any(|own| own.eq_ignore_ascii_case(address))),
            "sender_hash": sender_hash, "participant_hashes": participant_hashes});
    }
    Ok(Some(InboundMessage {
        platform_message_id: id.into(),
        conversation_id: conversation_id(&bot.platform_bot_id, thread),
        conversation_type: "private".into(),
        sender_platform_id: sender.into(),
        sender_display_name: message["from"]["name"].as_str().map(String::from),
        content_type: "text".into(),
        text: Some(format!("Subject: {subject}\n\n{body}")),
        attachments: vec![],
        reply_to_platform_message_id: None,
        thread_id: Some(thread.into()),
        raw_data,
    }))
}

fn email_history(
    messages: &[Value],
    thread: &str,
    before: DateTime<Utc>,
    audit_key: Option<&[u8]>,
) -> crate::services::channel_thread_service::ThreadHistory {
    let mut history = crate::services::channel_thread_service::ThreadHistory::default();
    for message in messages {
        let (Some(id), Some(sender), Some(received)) = (
            message["id"].as_str(),
            address(&message["from"]),
            message["receivedAt"].as_str(),
        ) else {
            history.partial = true;
            continue;
        };
        let Ok(created_at) = received.parse::<DateTime<Utc>>() else {
            history.partial = true;
            continue;
        };
        if message["threadId"].as_str() != Some(thread) {
            history.partial = true;
            continue;
        }
        let mut participants = vec![sender.to_ascii_lowercase()];
        for field in ["to", "cc"] {
            participants.extend(addresses(&message[field]));
        }
        participants.sort();
        participants.dedup();
        participants.truncate(64);
        let Some(participant_hashes) = participants
            .iter()
            .map(|p| hash_email(p, audit_key))
            .collect::<Option<Vec<_>>>()
        else {
            history.partial = true;
            continue;
        };
        let body = message["body"].as_str().unwrap_or_default().to_owned();
        let item = crate::services::channel_thread_service::ThreadHistoryMessage {
            message_id: id.to_owned(),
            sender_id: sender.to_owned(),
            sender_kind: ThreadSenderKind::Human,
            created_at,
            text: body,
            participant_hashes,
        };
        if item.created_at >= before {
            continue;
        }
        crate::services::channel_adapters::thread_support::push(&mut history, item, before);
    }
    history
}

#[async_trait::async_trait]
impl PlatformAdapter for AurinkoAdapter {
    fn platform_id(&self) -> &str {
        "aurinko"
    }
    fn display_name(&self) -> &str {
        "Aurinko Email"
    }
    fn outbound_capabilities(&self) -> OutboundCapabilities {
        OutboundCapabilities {
            reply_to: true,
            ..OutboundCapabilities::NONE
        }
    }
    fn thread_capabilities(&self) -> crate::services::channel_platform::ThreadCapabilities {
        crate::services::channel_platform::ThreadCapabilities {
            thread_reply: true,
            thread_follow: true,
            thread_history: true,
            private_thread: true,
        }
    }
    fn thread_facts(
        &self,
        inbound: &InboundMessage,
        _: &ChannelBot,
        _: Option<&str>,
    ) -> Option<ChannelThreadFacts> {
        super::thread_facts::aurinko(inbound)
    }
    async fn resolve_thread(
        &self,
        _http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        facts: &ChannelThreadFacts,
        _ancestors: &[ChannelThreadFacts],
    ) -> AppResult<Option<ChannelThreadFacts>> {
        if facts.kind != ThreadKind::Email {
            return Ok(None);
        }
        let Some(message) = self.message(credentials.token, &facts.message_id).await? else {
            return Ok(None);
        };
        if message["threadId"].as_str() != facts.root_id.as_deref() {
            return Ok(None);
        }
        let mut resolved = facts.clone();
        resolved.root_id = message["threadId"].as_str().map(str::to_owned);
        resolved.native_thread_id = resolved.root_id.clone();
        Ok(Some(resolved))
    }
    async fn send_bound_thread_reply(
        &self,
        db: &mongodb::Database,
        _http: &reqwest::Client,
        bot: &ChannelBot,
        original: &ChannelMessage,
        credentials: &BotCredentials<'_>,
        conversation_id: &str,
        _target: &crate::services::channel_thread_service::ThreadReplyTarget,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        channel_retry_ingress::with_lifecycle(
            db,
            true,
            &bot.id,
            self.reply_once(db, bot, original, credentials, conversation_id, reply),
        )
        .await
    }
    async fn thread_history(
        &self,
        _http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        target: &crate::services::channel_thread_service::ThreadReplyTarget,
        before: DateTime<Utc>,
    ) -> AppResult<crate::services::channel_thread_service::ThreadHistory> {
        let facts = target.facts();
        let Some(thread) = facts.root_id.as_deref() else {
            return Err(crate::services::channel_thread_service::unavailable());
        };
        let (status, value) = self
            .request(
                Method::GET,
                credentials.token,
                &["email", "threads", thread],
                &[("bodyType", "text")],
                None,
            )
            .await?;
        if !status.is_success() {
            return Err(crate::services::channel_thread_service::unavailable());
        }
        let messages = value["messages"]
            .as_array()
            .or_else(|| value["records"].as_array())
            .ok_or_else(crate::services::channel_thread_service::unavailable)?;
        Ok(email_history(
            messages,
            thread,
            before,
            crate::services::audit_service::audit_chain_hmac_key(),
        ))
    }

    fn media_capabilities(&self) -> MediaCapabilities {
        MediaCapabilities::NONE
    }
    fn serializes_lifecycle(&self) -> bool {
        true
    }
    fn platform_credentials(
        &self,
    ) -> Option<crate::services::channel_managed::PlatformCredentialDescriptor> {
        use crate::services::channel_managed::{
            PlatformCredentialBacking, PlatformCredentialDescriptor, PlatformCredentialField,
        };
        Some(PlatformCredentialDescriptor {
            provider: "aurinko",
            label: "Aurinko Email",
            backing: PlatformCredentialBacking::ProviderOAuth {
                provider_slug: "aurinko",
            },
            fields: &[
                PlatformCredentialField {
                    name: "client_id",
                    label: "Application Client ID",
                    secret: true,
                    required: true,
                    numeric: false,
                    help: "Aurinko application Client ID. Shared with the Aurinko provider configuration.",
                },
                PlatformCredentialField {
                    name: "client_secret",
                    label: "Application Client Secret",
                    secret: true,
                    required: true,
                    numeric: false,
                    help: "Aurinko application Client Secret. Shared with the Aurinko provider configuration; never a mailbox account token.",
                },
                PlatformCredentialField {
                    name: "signing_secret",
                    label: "Application webhook signing secret",
                    secret: true,
                    required: true,
                    numeric: false,
                    help: "Separate signing secret from the Aurinko application webhook settings. Required for managed channel bots.",
                },
            ],
            webhook_secret_field: None,
            setup_checklist: &[
                "Use one Aurinko application for NyxID. The application owns upstream billing; each mailbox still requires its owner's authorization.",
                "The Client Secret and webhook signing secret are different credentials. Never enter a mailbox account token in these fields.",
                "Saving application credentials does not enable managed OAuth onboarding. Existing manual mailbox connections continue to use their own tokens and signing secrets.",
            ],
        })
    }
    fn persists_reply_attempt(&self) -> bool {
        true
    }
    fn registration(&self) -> RegistrationDescriptor {
        RegistrationDescriptor {
            documentation_url: Some("https://docs.aurinko.io/unified-apis/webhooks-api"),
            fields: &[
                RegistrationField {
                    label: "Account access token",
                    hint: Some(
                        "Mailbox account token from Aurinko, not an application API key. Token updates must belong to the same account.",
                    ),
                    patchable: true,
                    ..BOT_TOKEN_FIELD
                },
                RegistrationField {
                    name: "app_secret",
                    label: "Aurinko signing secret",
                    hint: Some(
                        "Separate application signing secret from the Aurinko dashboard; verifies incoming notifications.",
                    ),
                    storage: "app_secret_encrypted",
                    secret: true,
                    required: true,
                    patchable: true,
                    clearable: false,
                    webhook_secret: true,
                    platform_fallback: None,
                },
            ],
            extra_fields: &[],
            unsupported_patch_message: None,
            automatic_webhook: true,
            setup_instructions: &[
                "Use an Aurinko mailbox account token with Mail.Read and Mail.Send, plus the separate application signing secret from the Aurinko dashboard.",
                "NyxID verifies the account and creates the email subscription automatically. Configure a default agent with a callback URL to receive new mail.",
                "AI Service and channel bot tokens are stored separately. Rotate or delete each connection separately. Managed OAuth onboarding is not available.",
                "Only new incoming personal mail is forwarded. Replies target the original sender or its single Reply-To address; attachments are available through the AI Service.",
            ],
            ..Default::default()
        }
    }
    fn validate_stored_verification(&self, bot: &ChannelBot) -> AppResult<()> {
        if bot.app_secret_encrypted.is_none() {
            return Err(AppError::ValidationError(
                "Aurinko signing secret is required".into(),
            ));
        }
        Ok(())
    }
    fn reply_context(&self, _: Option<&str>, _: DateTime<Utc>, _: &mut Option<Value>) {}
    fn webhook_policy(&self, _: &[u8]) -> WebhookPolicy {
        WebhookPolicy::RetryAwareInline
    }
    async fn verify_webhook(
        &self,
        bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<()> {
        let secret = secrets
            .and_then(|s| s.get("app_secret"))
            .ok_or_else(invalid)?;
        verify_signature(bot, secret, headers, body, Utc::now().timestamp())
    }
    async fn parse_inbound(&self, _: &[u8]) -> AppResult<Vec<InboundMessage>> {
        Err(AppError::BadRequest(
            "Aurinko notifications require authenticated mailbox hydration".into(),
        ))
    }
    async fn verify_bot_token(
        &self,
        _: &reqwest::Client,
        credentials: &BotCredentials<'_>,
    ) -> AppResult<BotIdentity> {
        let account = self
            .account(credentials.token, credentials.platform_bot_id)
            .await?;
        Ok(BotIdentity {
            platform_bot_id: identity(&account["id"]).ok_or_else(upstream)?,
            display_name: None,
            platform_bot_username: account["mailboxAddress"]
                .as_str()
                .or(account["email"].as_str())
                .ok_or_else(upstream)?
                .into(),
        })
    }
    async fn register_webhook(
        &self,
        _: &reqwest::Client,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<()> {
        Err(AppError::BadRequest(
            "Aurinko subscription management requires bot context".into(),
        ))
    }
    async fn setup_bot_webhook(
        &self,
        db: &mongodb::Database,
        _: &reqwest::Client,
        bot: &ChannelBot,
        token: &str,
        url: &str,
        _: &str,
    ) -> AppResult<()> {
        self.setup_subscription(db, bot, token, url).await
    }
    async fn remove_bot_webhook(
        &self,
        db: &mongodb::Database,
        _: &reqwest::Client,
        bot: &ChannelBot,
        token: &str,
    ) -> AppResult<()> {
        self.remove_subscription(db, bot, token).await
    }
    async fn retryable_webhook(
        &self,
        context: &IngressContext<'_>,
        bot_id: &str,
        headers: &HeaderMap,
        query: &HashMap<String, String>,
        body: &[u8],
    ) -> AppResult<Option<String>> {
        self.receive(context, bot_id, headers, query, body).await
    }
    async fn send_reply(
        &self,
        _: &reqwest::Client,
        _: &BotCredentials<'_>,
        _: &str,
        _: &OutboundReply,
    ) -> AppResult<Option<String>> {
        Err(AppError::BadRequest(
            "Aurinko replies require a stored inbound message".into(),
        ))
    }
    async fn send_bound_reply(
        &self,
        db: &mongodb::Database,
        _: &reqwest::Client,
        bot: &ChannelBot,
        original: &ChannelMessage,
        credentials: &BotCredentials<'_>,
        conversation_id: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        channel_retry_ingress::with_lifecycle(
            db,
            true,
            &bot.id,
            self.reply_once(db, bot, original, credentials, conversation_id, reply),
        )
        .await
    }
}

#[path = "aurinko_lifecycle.rs"]
mod lifecycle;
#[path = "aurinko_tests.rs"]
#[cfg(test)]
mod tests;

/// Only Aurinko mailbox identity is unique across active bots; legacy adapters
/// retain their existing registration semantics.
pub async fn ensure_indexes(db: &mongodb::Database) -> Result<(), mongodb::error::Error> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "platform_bot_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("aurinko_active_account".to_string())
                        .unique(true)
                        .partial_filter_expression(
                            doc! { "platform": "aurinko", "is_active": true },
                        )
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<bson::Document>(crate::models::channel_email::SENDS)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "bot_id": 1, "platform_message_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("email_reply_once".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<bson::Document>(crate::models::channel_email::BATCHES)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "bot_id": 1, "digest": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<bson::Document>(crate::models::channel_email::BATCHES)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(IndexOptions::builder().expire_after(Duration::ZERO).build())
                .build(),
        )
        .await?;
    db.collection::<bson::Document>(crate::models::channel_email::RECEIPTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "bot_id": 1, "platform_message_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    Ok(())
}
