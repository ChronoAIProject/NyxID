//! Positive provenance is produced within authenticated adapter preparation.
//! A deserialized InboundMessage is never an authority constructor.
use axum::http::HeaderMap;
use serde_json::Value;
use zeroize::{Zeroize, Zeroizing};

use super::{
    channel_platform::{InboundMessage, PlatformAdapter, PlatformVerifySecrets, PreparedWebhook},
    platform_entry_identity::{canonical_text, opaque, refused, validate_configuration},
};
use crate::{
    errors::AppResult,
    models::{
        channel_bot::ChannelBot,
        platform_entry_identity::{
            EntryPlatform, NativeControlSurface, NativeEntryConfiguration, NativeSenderIdentity,
        },
    },
};

pub(crate) struct VerifiedNativeEvent {
    pub(crate) sender: NativeSenderIdentity,
    pub(crate) event_id: String,
    pub(crate) message_id: String,
    text: Zeroizing<String>,
    reply: Option<AuthenticatedDiscordReply>,
}

// Only the authenticated classifier can construct reply material. Generic
// adapter thread strings and deserialized gateway activity cannot mint it.
struct AuthenticatedDiscordReply {
    application_id: String,
    token: Zeroizing<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// Discord requires signature rejection on the HTTP acknowledgement itself.
/// No model, proof or delivery work runs before this bounded authentication.
pub(crate) async fn authenticate_discord_ack(
    db: &mongodb::Database,
    adapter: &dyn PlatformAdapter,
    bot_id: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> AppResult<()> {
    let bot = super::channel_bot_service::get_bot(db, bot_id).await?;
    if adapter.platform_id() != "discord" || bot.platform != "discord" {
        return Err(refused());
    }
    adapter.verify_webhook(&bot, None, headers, body).await?;
    let timestamp = headers
        .get("x-signature-timestamp")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .ok_or_else(refused)?;
    if chrono::Utc::now().timestamp().abs_diff(timestamp) > 300 {
        return Err(refused());
    }
    Ok(())
}

impl VerifiedNativeEvent {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

impl std::fmt::Debug for VerifiedNativeEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VerifiedNativeEvent([REDACTED])")
    }
}

pub(crate) struct AuthenticatedNativeBatch {
    configuration: NativeEntryConfiguration,
    prepared: PreparedWebhook,
    messages: Vec<InboundMessage>,
    facts: Vec<Option<VerifiedNativeEvent>>,
    credentials: Option<super::platform_entry_credentials::EntryCredentialSnapshot>,
}

impl AuthenticatedNativeBatch {
    pub(crate) fn configuration(&self) -> &NativeEntryConfiguration {
        &self.configuration
    }

    pub(crate) async fn fence_credentials(
        &self,
        db: &mongodb::Database,
        session: &mut mongodb::ClientSession,
    ) -> AppResult<bool> {
        match &self.credentials {
            Some(snapshot) => snapshot.fence_in_session(db, session).await,
            None => Ok(false),
        }
    }

    pub(crate) fn prepared(&self) -> &PreparedWebhook {
        &self.prepared
    }
    pub(crate) fn messages(&self) -> &[InboundMessage] {
        &self.messages
    }
    pub(crate) fn fact(&self, index: usize) -> Option<&VerifiedNativeEvent> {
        self.facts.get(index).and_then(Option::as_ref)
    }

    pub(crate) async fn seal_reply_handle(
        &self,
        keys: &crate::crypto::aes::EncryptionKeys,
        receipt: &crate::models::platform_entry_identity::NativeEntryReceipt,
        index: usize,
    ) -> AppResult<Option<crate::models::platform_native_reply_handle::NativeReplyHandle>> {
        let fact = self.fact(index).ok_or_else(refused)?;
        if receipt.configuration != self.configuration
            || receipt.sender != fact.sender
            || receipt.event_id != fact.event_id
            || receipt.message_id != fact.message_id
        {
            return Err(refused());
        }
        let Some(reply) = &fact.reply else {
            if self.configuration.identity.platform == EntryPlatform::Discord {
                return Err(refused());
            }
            return Ok(None);
        };
        let now = chrono::Utc::now();
        let expires_at = receipt
            .expires_at
            .min(reply.created_at + chrono::Duration::minutes(14));
        if expires_at <= now {
            return Err(refused());
        }
        let digest = super::platform_native_reply_service::binding_digest(receipt)?;
        #[derive(serde::Serialize)]
        struct ReplyMaterial<'a> {
            application_id: &'a str,
            token: &'a str,
        }
        let material = Zeroizing::new(
            serde_json::to_vec(&ReplyMaterial {
                application_id: &reply.application_id,
                token: &reply.token,
            })
            .map_err(|_| refused())?,
        );
        let ciphertext = keys
            .encrypt_with_aad(&material, digest.as_bytes())
            .await
            .map_err(|_| refused())?;
        Ok(Some(
            crate::models::platform_native_reply_handle::NativeReplyHandle {
                id: uuid::Uuid::new_v4().to_string(),
                receipt_id: receipt.id.clone(),
                binding_digest: digest,
                ciphertext,
                created_at: now,
                expires_at,
            },
        ))
    }
}

/// Production construction resolves the real stored signing source. The private
/// parser below is available only inside this module and its adapter unit tests.
pub(crate) async fn prepare(
    db: &mongodb::Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    headers: &HeaderMap,
    body: &[u8],
    configuration: &NativeEntryConfiguration,
) -> AppResult<AuthenticatedNativeBatch> {
    let signing_snapshot =
        super::platform_entry_credentials::EntryCredentialSnapshot::load(db, adapter, bot).await?;
    if signing_snapshot.fingerprint()? != configuration.credential_fingerprint {
        return Err(refused());
    }
    let secrets = signing_snapshot.secrets(keys, adapter).await?;
    let mut prepared =
        prepare_with_secrets(adapter, bot, Some(&secrets), headers, body, configuration).await?;
    // Authentication precedes any OAuth side effect. Then use the ordinary
    // refresh producer before taking the live admission snapshot; expired but
    // refreshable connections retain their entry identity and epoch.
    if bot.credential_source == "connection" {
        super::channel_credentials::resolve_bot_token(db, keys, adapter, bot).await?;
    }
    let admission_snapshot =
        super::platform_entry_credentials::EntryCredentialSnapshot::load(db, adapter, bot).await?;
    if admission_snapshot.fingerprint()? != configuration.credential_fingerprint {
        return Err(refused());
    }
    prepared.credentials = Some(admission_snapshot);
    Ok(prepared)
}

async fn prepare_with_secrets(
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    secrets: Option<&PlatformVerifySecrets>,
    headers: &HeaderMap,
    body: &[u8],
    configuration: &NativeEntryConfiguration,
) -> AppResult<AuthenticatedNativeBatch> {
    validate_configuration(configuration)?;
    if configuration.identity.channel_bot_id != bot.id
        || configuration.identity.platform_bot_id != bot.platform_bot_id
        || platform_name(configuration.identity.platform) != bot.platform
        || adapter.platform_id() != bot.platform
    {
        return Err(refused());
    }
    let mut prepared = adapter.prepare_webhook(bot, secrets, headers, body).await?;
    if prepared.challenge_response.is_some() {
        return Ok(AuthenticatedNativeBatch {
            configuration: configuration.clone(),
            prepared,
            messages: Vec::new(),
            facts: Vec::new(),
            credentials: None,
        });
    }
    // The adapter may filter or decrypt the envelope. Classification sees only
    // that authenticated result and the messages parsed from the same result.
    let mut messages = adapter.parse_inbound(&prepared.body).await?;
    let mut authenticated_envelope: Value =
        serde_json::from_slice(&prepared.body).map_err(|_| refused())?;
    let facts: Vec<Option<VerifiedNativeEvent>> = messages
        .iter()
        .map(|message| {
            classify(
                configuration,
                headers,
                &authenticated_envelope,
                message,
                chrono::Utc::now().timestamp(),
            )
            .ok()
        })
        .collect();
    // The selected gateway receives exactly the text committed by the native
    // classifier. Ordinary adapters retain their existing normalization.
    for (message, fact) in messages.iter_mut().zip(&facts) {
        if let Some(fact) = fact {
            message.text = Some(fact.text().to_owned());
        }
        if configuration.identity.platform == EntryPlatform::Discord {
            // The native follow-up token has a separate encrypted lifetime.
            // Neither callbacks nor ordinary metadata rows receive it.
            if let Some(mut thread) = message.thread_id.take() {
                thread.zeroize();
            }
            scrub_discord_token(&mut message.raw_data);
        }
    }
    if configuration.identity.platform == EntryPlatform::Discord {
        scrub_discord_token(&mut authenticated_envelope);
        prepared.body.zeroize();
        prepared.body.clear();
    }
    Ok(AuthenticatedNativeBatch {
        configuration: configuration.clone(),
        prepared,
        messages,
        facts,
        credentials: None,
    })
}

fn scrub_discord_token(value: &mut Value) {
    if let Some(mut token) = value.as_object_mut().and_then(|map| map.remove("token"))
        && let Value::String(token) = &mut token
    {
        token.zeroize();
    }
}

pub(crate) fn platform_name(platform: EntryPlatform) -> &'static str {
    match platform {
        EntryPlatform::Slack => "slack",
        EntryPlatform::Discord => "discord",
        EntryPlatform::Whatsapp => "whatsapp",
        EntryPlatform::Lark => "lark",
        EntryPlatform::Feishu => "feishu",
        EntryPlatform::X => "x",
    }
}

fn string<'a>(value: &'a Value, key: &str) -> AppResult<&'a str> {
    opaque(value.get(key).and_then(Value::as_str).ok_or_else(refused)?)
}

fn absent_or_false(value: &Value, key: &str) -> bool {
    value
        .get(key)
        .is_none_or(|value| value == &Value::Bool(false))
}

fn no_forwarding(value: &Value) -> bool {
    [
        "forwarded",
        "frequently_forwarded",
        "is_forwarded",
        "is_share",
    ]
    .iter()
    .all(|key| absent_or_false(value, key))
        && [
            "forward_origin",
            "forward_from",
            "forward_from_chat",
            "forwarded_message",
            "edited",
        ]
        .iter()
        .all(|key| value.get(key).is_none())
}

fn event(
    message: &InboundMessage,
    event_id: &str,
    sender: &str,
    conversation: &str,
    text: String,
) -> AppResult<VerifiedNativeEvent> {
    opaque(event_id)?;
    opaque(sender)?;
    opaque(conversation)?;
    opaque(&message.platform_message_id)?;
    canonical_text(&text)?;
    if message.sender_platform_id != sender
        || message.conversation_id != conversation
        || message.conversation_type != "private"
    {
        return Err(refused());
    }
    Ok(VerifiedNativeEvent {
        sender: NativeSenderIdentity {
            sender_id: sender.into(),
            private_conversation_id: conversation.into(),
        },
        event_id: event_id.into(),
        message_id: message.platform_message_id.clone(),
        text: Zeroizing::new(text),
        reply: None,
    })
}

fn text_controls(text: &str) -> String {
    let mut tokens = text.split_whitespace();
    let Some(first) = tokens.next() else {
        return text.into();
    };
    // Only opted-in DM controls are adapted. Ordinary prose, including text
    // beginning with a similar word, retains its exact substantive bytes.
    match (first, tokens.next(), tokens.next()) {
        ("login" | "link" | "status" | "logout" | "unlink" | "help", None, None) => {
            format!("/{first}")
        }
        ("confirm", Some(code), None) if code.to_ascii_uppercase().starts_with("P-") => {
            format!("/confirm {code}")
        }
        _ => text.into(),
    }
}

fn classify(
    configuration: &NativeEntryConfiguration,
    headers: &HeaderMap,
    envelope: &Value,
    message: &InboundMessage,
    now_seconds: i64,
) -> AppResult<VerifiedNativeEvent> {
    let raw = &message.raw_data;
    if !message.attachments.is_empty() || message.content_type != "text" {
        return Err(refused());
    }
    match (
        &configuration.identity.platform,
        &configuration.control_surface,
    ) {
        (
            EntryPlatform::Slack,
            NativeControlSurface::SlackDirectMessage {
                app_id,
                team_id,
                bot_user_id,
            },
        ) => {
            if raw["type"] != "event_callback"
                || raw["api_app_id"] != app_id.as_str()
                || raw["team_id"] != team_id.as_str()
            {
                return Err(refused());
            }
            let authorizations = raw["authorizations"].as_array().ok_or_else(refused)?;
            if authorizations.len() != 1
                || authorizations[0]["team_id"] != team_id.as_str()
                || authorizations[0]["user_id"] != bot_user_id.as_str()
                || authorizations[0]["is_bot"] != true
            {
                return Err(refused());
            }
            let native = &raw["event"];
            if native["type"] != "message"
                || native["channel_type"] != "im"
                || native.get("subtype").is_some()
                || native.get("bot_id").is_some()
                || native.get("attachments").is_some()
                || native.get("files").is_some()
                || !no_forwarding(native)
            {
                return Err(refused());
            }
            let sender = string(native, "user")?;
            if sender == bot_user_id {
                return Err(refused());
            }
            let channel = string(native, "channel")?;
            let text = native["text"].as_str().ok_or_else(refused)?;
            if message.text.as_deref() != Some(text)
                || message.platform_message_id != string(native, "ts")?
            {
                return Err(refused());
            }
            event(
                message,
                string(raw, "event_id")?,
                sender,
                channel,
                text_controls(text),
            )
        }
        (
            EntryPlatform::Discord,
            NativeControlSurface::DiscordCommand {
                application_id,
                command_id,
                ..
            },
        ) => {
            let timestamp = headers
                .get("x-signature-timestamp")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<i64>().ok())
                .ok_or_else(refused)?;
            if now_seconds.abs_diff(timestamp) > 300
                || raw["type"] != 2
                || raw["application_id"] != application_id.as_str()
                || raw["context"] != 1
                || raw["channel"]["type"] != 1
                || raw.get("guild_id").is_some()
                || raw.get("member").is_some()
                || raw["data"]["id"] != command_id.as_str()
                || raw["data"]["type"] != 1
                || raw["data"]["name"] != "nyx"
                || !absent_or_false(&raw["user"], "bot")
                || !absent_or_false(&raw["user"], "system")
            {
                return Err(refused());
            }
            let text = discord_command_text(&raw["data"])?;
            if message.platform_message_id != string(raw, "id")? {
                return Err(refused());
            }
            let mut fact = event(
                message,
                string(raw, "id")?,
                string(&raw["user"], "id")?,
                string(raw, "channel_id")?,
                text,
            )?;
            let token = raw["token"].as_str().ok_or_else(refused)?;
            if token.is_empty()
                || token.len() > 2048
                || !token
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
                || !application_id.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(refused());
            }
            // Discord's snowflake binds the interaction creation time, so a
            // valid current signature cannot extend an old follow-up lifetime.
            let snowflake = fact.event_id.parse::<u64>().map_err(|_| refused())?;
            let created_ms = i64::try_from(snowflake >> 22)
                .map_err(|_| refused())?
                .checked_add(1_420_070_400_000)
                .ok_or_else(refused)?;
            let created_at =
                chrono::DateTime::from_timestamp_millis(created_ms).ok_or_else(refused)?;
            let now = chrono::DateTime::from_timestamp(now_seconds, 0).ok_or_else(refused)?;
            if created_at > now + chrono::Duration::seconds(5)
                || created_at + chrono::Duration::minutes(14) <= now
            {
                return Err(refused());
            }
            fact.reply = Some(AuthenticatedDiscordReply {
                application_id: application_id.clone(),
                token: Zeroizing::new(token.into()),
                created_at,
            });
            Ok(fact)
        }
        (EntryPlatform::Lark | EntryPlatform::Feishu, NativeControlSurface::PrivateMessage) => {
            if raw["header"]["event_type"] != "im.message.receive_v1"
                || raw["header"]["app_id"] != configuration.identity.platform_bot_id
                || raw["event"]["sender"]["sender_type"] != "user"
                || raw["event"]["message"]["chat_type"] != "p2p"
                || raw["event"]["message"]["message_type"] != "text"
                || !no_forwarding(&raw["event"]["message"])
            {
                return Err(refused());
            }
            let native = &raw["event"]["message"];
            let content: Value =
                serde_json::from_str(native["content"].as_str().ok_or_else(refused)?)
                    .map_err(|_| refused())?;
            let text = content["text"].as_str().ok_or_else(refused)?;
            if message.text.as_deref() != Some(text)
                || message.platform_message_id != string(native, "message_id")?
            {
                return Err(refused());
            }
            event(
                message,
                string(&raw["header"], "event_id")?,
                string(&raw["event"]["sender"]["sender_id"], "open_id")?,
                string(native, "chat_id")?,
                text_controls(text),
            )
        }
        (EntryPlatform::Whatsapp, NativeControlSurface::PrivateMessage) => {
            // prepare_webhook has already authenticated the app and filtered its
            // inner messages to the exact receiving phone_number_id.
            if raw["type"] != "text" || !no_forwarding(raw) || !no_forwarding(&raw["context"]) {
                return Err(refused());
            }
            let text = raw["text"]["body"].as_str().ok_or_else(refused)?;
            if message.text.as_deref() != Some(text)
                || message.platform_message_id != string(raw, "id")?
            {
                return Err(refused());
            }
            let sender = string(raw, "from")?;
            event(
                message,
                string(raw, "id")?,
                sender,
                &message.conversation_id,
                text_controls(text),
            )
        }
        (EntryPlatform::X, NativeControlSurface::PrivateMessage) => {
            // X's paid production path is a signed Activity dm.received event.
            // prepare_webhook verifies the raw body, exact recipient account,
            // configured subscription tag and selected event before this branch.
            // The native message_create actor/destination is the evidence; a
            // normalized activity.actor/default-Human is never consulted.
            let data = &envelope["data"];
            if data["event_type"] != "dm.received"
                || data["tag"] != format!("nyxid:{}", configuration.identity.channel_bot_id)
                || data["filter"]["user_id"] != configuration.identity.platform_bot_id
                || raw["type"] != "message_create"
                || !no_forwarding(raw)
                || !no_forwarding(&raw["message_create"])
                || !no_forwarding(&raw["message_create"]["message_data"])
            {
                return Err(refused());
            }
            let events = data["payload"]["direct_message_events"]
                .as_array()
                .ok_or_else(refused)?;
            if events
                .iter()
                .filter(|event| event["id"] == raw["id"])
                .count()
                != 1
                || !events.iter().any(|event| event == raw)
            {
                return Err(refused());
            }
            let created = &raw["message_create"];
            let sender = string(created, "sender_id")?;
            let recipient = string(&created["target"], "recipient_id")?;
            if recipient != configuration.identity.platform_bot_id
                || sender == recipient
                || !sender.bytes().all(|byte| byte.is_ascii_digit())
                || !recipient.bytes().all(|byte| byte.is_ascii_digit())
                || created["message_data"].get("attachment").is_some()
                || created.get("participants").is_some()
            {
                return Err(refused());
            }
            let user = &data["payload"]["users"][sender];
            let user = user.get("data").unwrap_or(user);
            if !absent_or_false(user, "is_bot")
                || !absent_or_false(user, "bot")
                || !absent_or_false(user, "is_automated")
                || !absent_or_false(user, "system")
            {
                return Err(refused());
            }
            let expected_conversation = if (sender.len(), sender) < (recipient.len(), recipient) {
                format!("{sender}-{recipient}")
            } else {
                format!("{recipient}-{sender}")
            };
            // Merely containing two numeric IDs is insufficient. These must be
            // exactly this native actor and this authenticated receiving account.
            if message.conversation_id != expected_conversation
                || created
                    .get("dm_conversation_id")
                    .or_else(|| raw.get("dm_conversation_id"))
                    .is_some_and(|value| value.as_str() != Some(expected_conversation.as_str()))
            {
                return Err(refused());
            }
            let text = created["message_data"]["text"]
                .as_str()
                .ok_or_else(refused)?;
            if message.text.as_deref() != Some(text)
                || message.platform_message_id != string(raw, "id")?
            {
                return Err(refused());
            }
            event(
                message,
                string(raw, "id")?,
                sender,
                &expected_conversation,
                text_controls(text),
            )
        }
        _ => Err(refused()),
    }
}

fn exact_keys(value: &Value, expected: &[&str]) -> AppResult<()> {
    let map = value.as_object().ok_or_else(refused)?;
    if map.len() != expected.len() || !expected.iter().all(|key| map.contains_key(*key)) {
        return Err(refused());
    }
    Ok(())
}

fn discord_command_text(data: &Value) -> AppResult<String> {
    let options = data["options"].as_array().ok_or_else(refused)?;
    if options.len() != 1 {
        return Err(refused());
    }
    let command = &options[0];
    if command["type"] != 1 {
        return Err(refused());
    }
    match command["name"].as_str() {
        Some(name @ ("login" | "status" | "logout" | "help")) => {
            exact_keys(command, &["type", "name"])?;
            Ok(format!("/{name}"))
        }
        Some(name @ ("confirm" | "ask")) => {
            exact_keys(command, &["type", "name", "options"])?;
            let fields = command["options"].as_array().ok_or_else(refused)?;
            if fields.len() != 1 {
                return Err(refused());
            }
            let argument = &fields[0];
            exact_keys(argument, &["type", "name", "value"])?;
            let expected = if name == "confirm" {
                "code"
            } else {
                "question"
            };
            if argument["type"] != 3 || argument["name"] != expected {
                return Err(refused());
            }
            let value = argument["value"].as_str().ok_or_else(refused)?;
            canonical_text(value)?;
            if name == "confirm" {
                if value.len() > 32
                    || value.split_whitespace().count() != 1
                    || !value.to_ascii_uppercase().starts_with("P-")
                {
                    return Err(refused());
                }
                Ok(format!("/confirm {value}"))
            } else {
                Ok(value.into())
            }
        }
        _ => Err(refused()),
    }
}

#[cfg(test)]
#[path = "platform_entry_native_tests.rs"]
pub(super) mod tests;
