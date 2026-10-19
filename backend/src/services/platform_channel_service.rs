//! Channel-neutral addresses for shared public/personal bots. Numeric handles
//! adapt existing persisted consent proofs without changing native Telegram rows.
use super::{channel_platform::InboundMessage, platform_manager_contract::*};
use crate::{
    errors::{AppError, AppResult},
    models::{
        platform_bot_config::{PlatformBotConfig, PlatformChannelTransport},
        platform_channel_identity::{COLLECTION_NAME, PlatformChannelIdentity},
    },
};
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};

pub(crate) fn supports(platform: &str) -> bool {
    matches!(
        platform,
        "telegram" | "discord" | "lark" | "feishu" | "slack" | "whatsapp" | "x"
    )
}

pub(crate) fn authority_id() -> i64 {
    let bytes = *uuid::Uuid::new_v4().as_bytes();
    (i64::from_be_bytes(bytes[..8].try_into().expect("eight UUID bytes")) & i64::MAX).max(1)
}

pub(crate) fn platform(manager: &str) -> &str {
    manager
        .strip_prefix("channel:")
        .and_then(|value| value.split(':').next())
        .unwrap_or("telegram")
}

pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<PlatformChannelIdentity>(COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"profile_id":1,"kind":1,"external_key":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"profile_id":1,"handle":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        ])
        .await?;
    db.collection::<PlatformChannelIdentity>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at":1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

fn event_handle(profile: &str, conversation: &str, message: &str) -> i64 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(
        serde_json::to_vec(&(profile, conversation, message)).expect("string tuple"),
    );
    (i64::from_be_bytes(digest[..8].try_into().expect("eight digest bytes")) & i64::MAX).max(1)
}

async fn identify(
    db: &Database,
    profile: &str,
    external: &str,
    sender: &str,
    chat: &str,
    display: Option<String>,
) -> AppResult<i64> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db, profile, external, sender, chat, display), |session, (db, profile, external, sender, chat, display)| Box::pin(async move {
        let result: AppResult<i64> = async {
            let rows = db.collection::<PlatformChannelIdentity>(COLLECTION_NAME);
            let filter = doc! {"profile_id":*profile,"kind":"peer","external_key":*external};
            if let Some(row) = rows.find_one(filter).session(&mut *session).await? {
                if row.sender_id != *sender || row.conversation_id != *chat {return Err(AppError::Conflict("Channel identity changed".into()));}
                if row.sender_display_name!=*display {
                    rows.update_one(doc! {"_id":&row.id},doc! {"$set":{"sender_display_name":display}}).session(&mut *session).await?;
                }
                return Ok(row.handle);
            }
            if db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
                .update_one(doc! {"_id":*profile,"deleted_at":bson::Bson::Null,"channel_transport":{"$ne":bson::Bson::Null}},doc! {"$inc":{"channel_identity_revision":1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(AppError::RateLimited); }
            let row = PlatformChannelIdentity { id:uuid::Uuid::new_v4().to_string(),profile_id:(*profile).into(),kind:"peer".into(),external_key:(*external).into(),handle:authority_id(),sender_id:(*sender).into(),sender_display_name:display.clone(),conversation_id:(*chat).into(),reply_context_encrypted:vec![],expires_at:None };
            rows.insert_one(&row).session(session).await?;
            Ok(row.handle)
        }.await;
        super::api_key_mutation_service::transaction_result(result)
    })).await.map_err(super::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn peer(
    db: &Database,
    profile: &str,
    handle: i64,
) -> AppResult<PlatformChannelIdentity> {
    db.collection::<PlatformChannelIdentity>(COLLECTION_NAME)
        .find_one(doc! {"profile_id":profile,"kind":"peer","handle":handle})
        .await?
        .ok_or_else(|| AppError::Conflict("Channel identity is unavailable".into()))
}

/// Reject edits, bot/self events and unsupported payloads before allocating identities.
/// Forwarded normal text can be read as text but can never prove account control.
fn authenticated_human(platform: &str, bot_id: &str, input: &InboundMessage) -> bool {
    if input.conversation_type != "private"
        || input.sender_platform_id.is_empty()
        || input.sender_platform_id == bot_id
    {
        return false;
    }
    let raw = &input.raw_data;
    let sender = input.sender_platform_id.as_str();
    match platform {
        "telegram" => {
            raw.get("edited_message").is_none()
                && raw["message"]["from"]["is_bot"] == false
                && raw["message"].get("sender_chat").is_none()
                && raw["message"]["from"]["id"]
                    .as_i64()
                    .map(|id| id.to_string())
                    .as_deref()
                    == Some(sender)
                && raw["message"]["chat"]["id"]
                    .as_i64()
                    .map(|id| id.to_string())
                    .as_deref()
                    == Some(sender)
        }
        "lark" | "feishu" => {
            raw["header"]["event_type"] == "im.message.receive_v1"
                && raw["event"]["sender"]["sender_type"] == "user"
                && raw["event"]["sender"]["sender_id"]["open_id"] == sender
        }
        "slack" => {
            raw["event"]["type"] == "message"
                && raw["event"]["user"] == sender
                && raw["event"].get("bot_id").is_none()
                && (raw["event"].get("subtype").is_none()
                    || raw["event"]["subtype"] == "file_share")
        }
        "discord" => {
            raw["type"] == 2
                && raw.get("guild_id").is_none()
                && raw["user"]["id"] == sender
                && raw["user"]["bot"] != true
                && input
                    .thread_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with("interaction:"))
        }
        "whatsapp" => {
            raw["from"] == sender
                && matches!(
                    raw["type"].as_str(),
                    Some(
                        "text"
                            | "image"
                            | "audio"
                            | "video"
                            | "document"
                            | "sticker"
                            | "location"
                            | "contacts"
                    )
                )
        }
        "x" => {
            (raw["sender_id"] == sender
                || (raw["type"] == "message_create"
                    && raw["message_create"]["sender_id"] == sender
                    && raw["message_create"]["target"]["recipient_id"] == bot_id))
                && input.conversation_id.split_once('-').is_some_and(|(a, b)| {
                    (a == sender && b == bot_id) || (a == bot_id && b == sender)
                })
        }
        _ => false,
    }
}

fn original_private(platform: &str, input: &InboundMessage) -> bool {
    if input.content_type != "text"
        || !input.attachments.is_empty()
        || input.text.as_ref().is_none_or(|text| text.len() > 16384)
    {
        return false;
    }
    let raw = &input.raw_data;
    match platform {
        "telegram" => super::telegram_new_service::private_sender(&raw["message"]).is_some(),
        "whatsapp" => {
            raw["context"]["forwarded"] != true && raw["context"]["frequently_forwarded"] != true
        }
        "lark" | "feishu" => raw["event"]["message"].get("merge_forward").is_none(),
        "x" => raw.get("referenced_events").is_none() && raw.get("forwarded").is_none(),
        _ => true,
    }
}

fn dispatch_text(platform: &str, input: &InboundMessage) -> String {
    let supported = input.content_type == "text"
        && input.text.as_ref().is_some_and(|text| text.len() <= 16384)
        && input.attachments.is_empty();
    if !supported {
        "/help".into()
    } else if platform == "slack" {
        slack_input(input.text.as_deref().unwrap_or(""))
    } else if platform == "discord" {
        discord_input(&input.raw_data).unwrap_or_else(|| "/help".into())
    } else {
        input.text.clone().unwrap_or_default()
    }
}

pub(crate) async fn dispatch(
    state: &crate::AppState,
    producers: std::sync::Arc<PlatformManagerProducers>,
    profile: &PlatformBotConfig,
    bot: &crate::models::channel_bot::ChannelBot,
    input: &InboundMessage,
) -> AppResult<()> {
    if profile.deleted_at.is_some() {
        return Ok(());
    }
    let transport = profile
        .channel_transport
        .as_ref()
        .ok_or_else(|| AppError::Conflict("Channel transport unavailable".into()))?;
    if transport.platform != bot.platform
        || transport.platform_bot_id != bot.platform_bot_id
        || transport.owner_user_id != bot.user_id
        || transport.ownership_version != bot.ownership_version
    {
        return Err(AppError::Conflict(
            "Platform channel ownership changed".into(),
        ));
    }
    let Some(runtime) =
        super::platform_bot_runtime_control_service::get(&state.db, &profile.id).await?
    else {
        return Ok(());
    };
    if !authenticated_human(&bot.platform, &bot.platform_bot_id, input) {
        return Ok(());
    }
    if !super::coordination_service::RateWindowStore::admit(
        &state.db,
        "platform_channel_ingress",
        &profile.id,
        300,
        std::time::Duration::from_secs(60),
    )
    .await?
    .allowed
    {
        return Err(AppError::RateLimited);
    }
    let verified = verified_input(&state.db, profile, input).await?;
    // Interaction follow-up credentials are encrypted and expire with Discord's token.
    if bot.platform == "discord" {
        let context = zeroize::Zeroizing::new(input.thread_id.clone().unwrap_or_default());
        let encrypted = state
            .encryption_keys
            .encrypt_with_aad(
                context.as_bytes(),
                format!("platform-reply:{}:{}", profile.id, verified.message_id).as_bytes(),
            )
            .await?;
        let row = PlatformChannelIdentity {
            id: uuid::Uuid::new_v4().to_string(),
            profile_id: profile.id.clone(),
            kind: "message".into(),
            external_key: input.platform_message_id.clone(),
            handle: verified.message_id,
            sender_id: input.sender_platform_id.clone(),
            sender_display_name: None,
            conversation_id: input.conversation_id.clone(),
            reply_context_encrypted: encrypted,
            expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(14)),
        };
        state.db.collection::<PlatformChannelIdentity>(COLLECTION_NAME).update_one(doc! {"profile_id":&profile.id,"kind":"message","external_key":&input.platform_message_id},doc! {"$setOnInsert":bson::to_document(&row).map_err(|_|AppError::Internal("Reply context unavailable".into()))?}).upsert(true).await?;
    }
    let text = dispatch_text(&bot.platform, input);
    super::platform_manager_transport::dispatch_verified_input(
        state,
        producers,
        &runtime,
        verified,
        &bot.platform_bot_username,
        &text,
        &super::telegram_new_api::TelegramApi::new(&state.http_client),
        false,
        bot.platform != "discord" || input.raw_data["data"]["name"] != "chat",
    )
    .await?;
    Ok(())
}

/// Application command options are structural authority, not concatenated text.
fn discord_input(raw: &serde_json::Value) -> Option<String> {
    if raw["data"]["type"].as_u64().is_some_and(|kind| kind != 1) {
        return None;
    }
    let name = raw["data"]["name"].as_str()?;
    let empty = vec![];
    let options = match raw["data"].get("options") {
        None => &empty,
        Some(value) => value.as_array()?,
    };
    let argument = |field: &str, required: bool| -> Option<&str> {
        if options.is_empty() && !required {
            return Some("");
        }
        if options.len() != 1 || options[0]["name"] != field || options[0]["type"] != 3 {
            return None;
        }
        options[0]["value"]
            .as_str()
            .filter(|text| !required || !text.trim().is_empty())
    };
    match name {
        "link" | "unlink" | "help" | "start" if options.is_empty() => Some(format!("/{name}")),
        "confirm" => Some(format!("/confirm {}", argument("code", true)?)),
        "faq" => Some(format!("/faq {}", argument("topic", false)?)),
        "chat" => Some(argument("message", true)?.to_string()),
        _ => None,
    }
}

/// Slack's JSON Events API receives ordinary messages, not slash commands.
/// Reserve only exact control aliases and their deliberately defined arguments.
fn slack_input(text: &str) -> String {
    let text = text.trim();
    if matches!(
        text,
        "link" | "unlink" | "help" | "faq" | "start" | "confirm"
    ) || text.strip_prefix("faq ").is_some()
        || text.strip_prefix("confirm ").is_some()
    {
        return format!("/{text}");
    }
    text.into()
}

/// Render command labels only. URL paths and bearer bytes are never rewritten.
/// Callers use this for server-authored control/help text, never model output.
pub(crate) fn format_controls(manager: &str, text: String) -> String {
    if platform(manager) == "discord" {
        return text
            .replace(
                "Send a text message to use your NyxBot",
                "Use /chat message to use your NyxBot",
            )
            .replace(
                "send a text message to use your NyxBot",
                "use /chat message to use your NyxBot",
            ).replace("Send a text question, or /faq followed by a topic above.", "Use /faq topic to ask a public question, or /chat message for personal text after linking.");
    }
    if platform(manager) != "slack" {
        return text;
    }
    let mut result = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    let mut previous = None;
    while let Some((offset, character)) = chars.next() {
        if character == '/'
            && previous
                .is_none_or(|c: char| c.is_whitespace() || matches!(c, '`' | '(' | '"' | '\''))
        {
            let tail = &text[offset + 1..];
            if ["link", "unlink", "confirm", "faq", "help", "start"]
                .iter()
                .any(|name| {
                    tail.strip_prefix(name).is_some_and(|rest| {
                        rest.chars().next().is_none_or(|c| {
                            c.is_whitespace()
                                || matches!(c, '`' | ')' | '.' | ',' | ':' | '"' | '\'')
                        })
                    })
                })
            {
                previous = Some(character);
                continue;
            }
        }
        result.push(character);
        previous = Some(character);
    }
    result
}

pub(crate) async fn verified_input(
    db: &Database,
    profile: &PlatformBotConfig,
    input: &InboundMessage,
) -> AppResult<VerifiedManagerUpdate> {
    let transport = profile
        .channel_transport
        .as_ref()
        .ok_or_else(|| AppError::Conflict("Channel transport unavailable".into()))?;
    if input.sender_platform_id.is_empty()
        || input.sender_platform_id.len() > 512
        || input.conversation_id.is_empty()
        || input.conversation_id.len() > 512
        || input.platform_message_id.is_empty()
        || input.platform_message_id.len() > 512
    {
        return Err(AppError::ValidationError(
            "Invalid channel message identity".into(),
        ));
    }
    let peer_key = serde_json::to_string(&(&input.sender_platform_id, &input.conversation_id))
        .map_err(|_| AppError::Internal("Channel identity unavailable".into()))?;
    let display = input
        .sender_display_name
        .as_deref()
        .map(|value| {
            value
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect::<String>()
        })
        .filter(|value| !value.trim().is_empty());
    let handle = identify(
        db,
        &profile.id,
        &peer_key,
        &input.sender_platform_id,
        &input.conversation_id,
        display,
    )
    .await?;
    let event = event_handle(
        &profile.id,
        &input.conversation_id,
        &input.platform_message_id,
    );
    Ok(VerifiedManagerUpdate {
        profile_id: profile.id.clone(),
        manager_bot_id: profile.manager_bot_id.clone(),
        telegram_bot_id: transport.authority_id,
        observation_id: profile.observation_id.clone(),
        sender_id: handle,
        chat_id: handle,
        update_id: event,
        message_id: event,
        chat_kind: if input.conversation_type == "private" {
            ManagerChatKind::Private
        } else {
            ManagerChatKind::Group
        },
        command_eligibility: if original_private(&transport.platform, input) {
            ManagerCommandEligibility::OriginalPrivateMessage
        } else {
            ManagerCommandEligibility::Ineligible
        },
    })
}

pub(crate) fn bot_filter(
    profile: &PlatformBotConfig,
    transport: &PlatformChannelTransport,
) -> Document {
    doc! {"_id":&profile.channel_bot_id,"platform_profile_id":&profile.id,"platform":&transport.platform,
    "platform_bot_id":&transport.platform_bot_id,"user_id":&transport.owner_user_id,
    "ownership_version":transport.ownership_version,"is_active":true,"status":"active"}
}

/// Encrypted material captured under the same transaction as the send claim.
/// Provider calls use only this snapshot; no later resolver may adopt new secrets.
pub(crate) struct CredentialSnapshot {
    pub bot: crate::models::channel_bot::ChannelBot,
    platform: Option<crate::models::platform_credential::PlatformCredential>,
    connection: Option<crate::models::user_api_key::UserApiKey>,
    provider: Option<crate::models::provider_config::ProviderConfig>,
    material: Document,
}

impl CredentialSnapshot {
    pub(crate) async fn load(
        db: &Database,
        session: &mut ClientSession,
        bot: &crate::models::channel_bot::ChannelBot,
        write: bool,
    ) -> AppResult<Self> {
        let mut material = super::channel_verification_service::credential_filter(bot)?;
        material.remove("status");
        material.remove("is_active");
        let cache =
            std::sync::Arc::new(super::provider_token_exchange_service::TokenExchangeCache::new());
        let adapter = super::channel_adapters::resolve_adapter(&bot.platform, &cache)?;
        let mut connection = None;
        let mut provider = None;
        if bot.credential_source == "connection" {
            let filter = doc! {"_id":&bot.connection_id,"user_id":&bot.user_id,"status":"active","credential_type":"oauth2"};
            let rows = db.collection::<crate::models::user_api_key::UserApiKey>(
                crate::models::user_api_key::COLLECTION_NAME,
            );
            let key = if write {
                rows.find_one_and_update(filter, doc! {"$inc":{"platform_binding_revision":1_i64}})
                    .session(&mut *session)
                    .await?
            } else {
                rows.find_one(filter).session(&mut *session).await?
            }
            .ok_or_else(|| {
                AppError::Conflict(
                    "Reconnect the bot owner's channel account, then repair this shared bot".into(),
                )
            })?;
            if key
                .access_token_encrypted
                .as_ref()
                .is_none_or(|value| value.is_empty())
                || (key
                    .expires_at
                    .is_some_and(|expiry| expiry <= chrono::Utc::now())
                    && key.refresh_token_encrypted.is_none())
            {
                return Err(AppError::Conflict("Reconnect the bot owner's expired channel account, then repair this shared bot".into()));
            }
            let mut fields = doc! {"credential_epoch":key.credential_epoch};
            let stored = bson::to_document(&key)
                .map_err(|_| AppError::Internal("Channel credential unavailable".into()))?;
            for field in [
                "provider_config_id",
                "token_scopes",
                "credential_source",
                "last_authorized_at",
            ] {
                fields.insert(
                    field,
                    stored.get(field).cloned().unwrap_or(bson::Bson::Null),
                );
            }
            material.insert("connection", fields);
            let super::channel_platform::CredentialResolution::OAuthConnection {
                provider_slug,
                required_scopes,
            } = adapter.credential_resolution()
            else {
                return Err(AppError::Conflict("Unsupported channel connection".into()));
            };
            let filter = doc! {"slug":provider_slug,"is_active":true};
            let rows = db.collection::<crate::models::provider_config::ProviderConfig>(
                crate::models::provider_config::COLLECTION_NAME,
            );
            let row = if write {
                rows.find_one_and_update(
                    filter,
                    doc! {"$inc":{"channel_observation_version":1_i64}},
                )
                .session(&mut *session)
                .await?
            } else {
                rows.find_one(filter).session(&mut *session).await?
            }
            .ok_or_else(|| AppError::Conflict("Channel OAuth provider is unavailable".into()))?;
            super::channel_credentials::validate_connection(&key, &row.id, required_scopes)?;
            let stored = bson::to_document(&row)
                .map_err(|_| AppError::Internal("Channel provider unavailable".into()))?;
            let mut fields = doc! {};
            for field in [
                "_id",
                "is_active",
                "client_id_encrypted",
                "client_secret_encrypted",
                "authorization_url",
                "token_url",
            ] {
                fields.insert(
                    field,
                    stored.get(field).cloned().unwrap_or(bson::Bson::Null),
                );
            }
            material.insert("oauth_provider", fields);
            connection = Some(key);
            provider = Some(row);
        }
        let mut platform = None;
        if matches!(bot.credential_source.as_str(), "platform" | "connection")
            && let Some(descriptor) = adapter.platform_credentials()
        {
            let rows = db.collection::<crate::models::platform_credential::PlatformCredential>(
                crate::models::platform_credential::COLLECTION_NAME,
            );
            let filter = doc! {"provider":descriptor.provider};
            platform = if write {
                rows.find_one_and_update(
                    filter,
                    doc! {"$inc":{"channel_observation_version":1_i64}},
                )
                .session(&mut *session)
                .await?
            } else {
                rows.find_one(filter).session(&mut *session).await?
            };
            if let super::channel_managed::PlatformCredentialBacking::ProviderOAuth { .. } =
                descriptor.backing
            {
                // OAuth provider material is bound above. Auxiliary webhook secrets
                // are separately captured and fenced in their own stored row.
                if provider.is_none() {
                    return Err(AppError::Conflict(
                        "OAuth channel credentials unavailable".into(),
                    ));
                }
            }
            material.insert(
                "platform_fields",
                bson::to_bson(&platform.as_ref().map(|p| &p.fields))
                    .map_err(|_| AppError::Internal("Channel credentials unavailable".into()))?,
            );
            material.insert(
                "platform_secrets",
                bson::to_bson(&platform.as_ref().map(|p| &p.secrets))
                    .map_err(|_| AppError::Internal("Channel credentials unavailable".into()))?,
            );
        }
        Ok(Self {
            bot: bot.clone(),
            platform,
            connection,
            provider,
            material,
        })
    }
    pub(crate) fn stamp(&self) -> AppResult<String> {
        use sha2::{Digest, Sha256};
        Ok(hex::encode(Sha256::digest(
            bson::to_vec(&self.material)
                .map_err(|_| AppError::Internal("Channel authority unavailable".into()))?,
        )))
    }
    pub(crate) async fn token(
        &self,
        keys: &crate::crypto::aes::EncryptionKeys,
    ) -> AppResult<zeroize::Zeroizing<String>> {
        let encrypted = if let Some(key) = &self.connection {
            if key
                .expires_at
                .is_some_and(|expiry| expiry <= chrono::Utc::now())
            {
                return Err(AppError::Conflict(
                    "Channel OAuth token expired; retry after refresh".into(),
                ));
            }
            key.access_token_encrypted
                .as_deref()
                .ok_or_else(|| AppError::Conflict("Channel OAuth token unavailable".into()))?
        } else {
            self.bot.bot_token_encrypted.as_slice()
        };
        let bytes = zeroize::Zeroizing::new(keys.decrypt(encrypted).await?);
        Ok(zeroize::Zeroizing::new(
            std::str::from_utf8(&bytes)
                .map_err(|_| AppError::Internal("Invalid channel credentials".into()))?
                .to_string(),
        ))
    }
    pub(crate) async fn secrets(
        &self,
        keys: &crate::crypto::aes::EncryptionKeys,
        adapter: &dyn super::channel_platform::PlatformAdapter,
    ) -> AppResult<super::channel_platform::PlatformVerifySecrets> {
        let mut secrets = adapter.build_verify_secrets(keys, &self.bot).await?;
        let mut platform =
            super::platform_credential_service::decrypt_snapshot(keys, self.platform.as_ref())
                .await?;
        if let Some(provider) = &self.provider {
            for (name, encrypted) in [
                ("client_id", &provider.client_id_encrypted),
                ("client_secret", &provider.client_secret_encrypted),
            ] {
                if let Some(encrypted) = encrypted {
                    let bytes = zeroize::Zeroizing::new(keys.decrypt(encrypted).await?);
                    platform.insert(
                        name,
                        std::str::from_utf8(&bytes)
                            .map_err(|_| AppError::Internal("Invalid channel credentials".into()))?
                            .to_string(),
                    );
                }
            }
        }
        if self.bot.credential_source == "connection" && adapter.platform_webhook() {
            return Ok(platform);
        }
        if self.bot.credential_source == "platform" {
            for field in adapter.registration().fields {
                if secrets.get(field.name).is_none()
                    && let Some(fallback) = field.platform_fallback
                    && let Some(value) = platform.get(fallback)
                {
                    secrets.insert(field.name, value.to_string());
                }
            }
        }
        Ok(secrets)
    }
}

pub(crate) async fn credential_stamp(
    db: &Database,
    session: &mut ClientSession,
    bot: &crate::models::channel_bot::ChannelBot,
    write: bool,
) -> AppResult<String> {
    CredentialSnapshot::load(db, session, bot, write)
        .await?
        .stamp()
}

pub(crate) async fn fence_transport(
    db: &Database,
    session: &mut ClientSession,
    profile: &PlatformBotConfig,
    observation: Option<&str>,
    write: bool,
) -> AppResult<bool> {
    let Some(transport) = &profile.channel_transport else {
        return Ok(false);
    };
    if observation.is_some_and(|value| value != profile.observation_id)
        || !supports(&transport.platform)
    {
        return Ok(false);
    }
    let bots = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME);
    let filter = bot_filter(profile, transport);
    let bot = db
        .collection::<crate::models::channel_bot::ChannelBot>(
            crate::models::channel_bot::COLLECTION_NAME,
        )
        .find_one(filter.clone())
        .session(&mut *session)
        .await?;
    let Some(bot) = bot else {
        return Ok(false);
    };
    let stamp = match credential_stamp(db, session, &bot, write).await {
        Ok(stamp) => stamp,
        Err(AppError::Conflict(_) | AppError::ValidationError(_) | AppError::NotFound(_)) => {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    if transport.credential_stamp.is_empty() || stamp != transport.credential_stamp {
        return Ok(false);
    }
    Ok(if write {
        bots.find_one_and_update(filter, doc! {"$inc":{"personal_effect_revision":1_i64}})
            .return_document(ReturnDocument::After)
            .session(session)
            .await?
            .is_some()
    } else {
        bots.find_one(filter).session(session).await?.is_some()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::channel_bot::ChannelBot;
    use serde_json::json;

    pub(crate) async fn fixture(
        name: &str,
        platform: &str,
    ) -> (
        Database,
        VerifiedManagerUpdate,
        String,
        PlatformBotConfig,
        ChannelBot,
    ) {
        let (db, mut update, owner) =
            super::super::platform_personal_binding_service::tests::fixture(name).await;
        ensure_indexes(&db).await.unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        let mut profile = super::super::platform_bot_config_service::get(&db, &update.profile_id)
            .await
            .unwrap();
        let token = state
            .encryption_keys
            .encrypt(b"snapshot-original")
            .await
            .unwrap();
        let bot:ChannelBot=bson::from_document(doc! {"_id":&profile.channel_bot_id,"user_id":&owner,"platform":platform,"platform_bot_id":"999","platform_bot_username":"support_bot","label":"Support","bot_token_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:token},"webhook_registered":true,"webhook_secret_hash":"hash","status":"active","is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
        db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .replace_one(doc! {"_id":&bot.id}, &bot)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&bot.id},
                doc! {"$set":{"platform_profile_id":&profile.id}},
            )
            .await
            .unwrap();
        let mut session = db.client().start_session().await.unwrap();
        let stamp = credential_stamp(&db, &mut session, &bot, false)
            .await
            .unwrap();
        profile.manager_bot_id = format!("channel:{platform}:{}", bot.id);
        profile.channel_transport = Some(PlatformChannelTransport {
            platform: platform.into(),
            platform_bot_id: bot.platform_bot_id.clone(),
            owner_user_id: owner.clone(),
            ownership_version: 0,
            authority_id: 5678,
            credential_stamp: stamp,
        });
        db.collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
            .replace_one(doc! {"_id":&profile.id}, &profile)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .update_one(
                doc! {"platform_config_id":&profile.id},
                doc! {"$set":{"manager_bot_id":&profile.manager_bot_id,"telegram_bot_id":5678_i64}},
            )
            .await
            .unwrap();
        update.manager_bot_id = profile.manager_bot_id.clone();
        update.telegram_bot_id = 5678;
        (db, update, owner, profile, bot)
    }
    fn input(raw: serde_json::Value, sender: &str, chat: &str) -> InboundMessage {
        InboundMessage {
            platform_message_id: "message".into(),
            conversation_id: chat.into(),
            conversation_type: "private".into(),
            sender_platform_id: sender.into(),
            sender_display_name: Some("Alice".into()),
            content_type: "text".into(),
            text: Some("hello".into()),
            attachments: vec![],
            reply_to_platform_message_id: None,
            thread_id: None,
            raw_data: raw,
        }
    }
    #[test]
    fn slack_control_grammar_and_rendering_preserve_bearers_and_urls() {
        for (text, expected) in [
            ("link", "/link"),
            ("faq login", "/faq login"),
            ("confirm p-abcd-efgh", "/confirm p-abcd-efgh"),
            ("confirm garbage", "/confirm garbage"),
            ("please link my account", "please link my account"),
            ("link now", "link now"),
        ] {
            assert_eq!(slack_input(text), expected);
        }
        let url = "https://nyxid.test/nyxbot/link#intent_token=nyx_pli_a_b-C";
        let body = format_controls(
            "channel:slack:bot",
            format!("Send /link or /confirm P-ABCD-EFGH.\n{url}"),
        );
        assert!(body.starts_with("Send link or confirm P-ABCD-EFGH."));
        let parsed = url::Url::parse(body.lines().last().unwrap()).unwrap();
        assert_eq!(parsed.path(), "/nyxbot/link");
        assert_eq!(parsed.fragment(), Some("intent_token=nyx_pli_a_b-C"));
        assert_eq!(format_controls("channel:slack:bot", body.clone()), body);
    }
    #[test]
    fn discord_controls_require_exact_string_options() {
        assert_eq!(discord_input(&json!({"data":{"type":1,"name":"confirm","options":[{"type":3,"name":"code","value":"P-ABCD-EFGH"}]}})).as_deref(),Some("/confirm P-ABCD-EFGH"));
        for options in [
            json!([]),
            json!([{"type":4,"name":"code","value":123}]),
            json!([{"type":3,"name":"message","value":"P-ABCD-EFGH"}]),
            json!([{"type":3,"name":"code","value":"x"},{"type":3,"name":"code","value":"y"}]),
        ] {
            assert!(
                discord_input(&json!({"data":{"type":1,"name":"confirm","options":options}}))
                    .is_none()
            );
        }
        assert_eq!(discord_input(&json!({"data":{"name":"chat","options":[{"type":3,"name":"message","value":"hello"}]}})).as_deref(),Some("hello"));
    }
    #[test]
    fn authentic_human_is_distinct_from_control_eligibility_and_supported_content() {
        let mut telegram = input(
            json!({"message":{"from":{"id":123,"is_bot":false},"chat":{"id":123,"type":"private"}}}),
            "123",
            "123",
        );
        assert!(authenticated_human("telegram", "999", &telegram));
        telegram.raw_data["message"]["forward_origin"] = json!({"type":"hidden_user"});
        assert!(authenticated_human("telegram", "999", &telegram));
        assert!(!original_private("telegram", &telegram));
        telegram.raw_data["message"]["from"]["is_bot"] = json!(true);
        assert!(!authenticated_human("telegram", "999", &telegram));
        let mut slack = input(json!({"event":{"type":"message","user":"U1"}}), "U1", "D1");
        assert!(authenticated_human("slack", "B1", &slack));
        slack.content_type = "image".into();
        slack.text = None;
        assert!(authenticated_human("slack", "B1", &slack));
        slack.raw_data["event"]["subtype"] = json!("message_changed");
        assert!(!authenticated_human("slack", "B1", &slack));
        let mut discord = input(json!({"type":2,"user":{"id":"U1","bot":false}}), "U1", "D1");
        discord.thread_id = Some("interaction:app:token".into());
        assert!(authenticated_human("discord", "B1", &discord));
        discord.raw_data["guild_id"] = json!("guild");
        assert!(!authenticated_human("discord", "B1", &discord));
        for platform in ["lark", "feishu"] {
            let lark = input(
                json!({"header":{"event_type":"im.message.receive_v1"},"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"U1"}}}}),
                "U1",
                "D1",
            );
            assert!(authenticated_human(platform, "B1", &lark));
        }
        let mut whatsapp = input(json!({"from":"123","type":"text"}), "123", "123");
        assert!(authenticated_human("whatsapp", "999", &whatsapp));
        whatsapp.raw_data["context"] = json!({"forwarded":true});
        assert!(!original_private("whatsapp", &whatsapp));
        for raw in [
            json!({"sender_id":"123"}),
            json!({"type":"message_create","message_create":{"sender_id":"123","target":{"recipient_id":"999"}}}),
        ] {
            let x = input(raw, "123", "123-999");
            assert!(authenticated_human("x", "999", &x));
            let mut group = x;
            group.conversation_id = "group".into();
            assert!(!authenticated_human("x", "999", &group));
        }
    }
    #[tokio::test]
    async fn actual_slack_file_share_routes_to_help_without_control_or_personal_media() {
        use super::super::channel_platform::PlatformAdapter;
        let adapter = super::super::channel_adapters::slack::SlackAdapter::new();
        let payload = json!({"type":"event_callback","event":{
            "type":"message","subtype":"file_share","channel":"D1","channel_type":"im",
            "user":"U1","text":"confirm P-ABCD-EFGH","ts":"1700000050.000000",
            "files":[{"id":"F1","name":"image.png","mimetype":"image/png",
                "url_private":"https://files.slack.com/private/image.png","size":12345}]
        }});
        let messages = adapter
            .parse_inbound(&serde_json::to_vec(&payload).unwrap())
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert!(authenticated_human("slack", "B1", message));
        assert_eq!(message.content_type, "image");
        assert_eq!(dispatch_text("slack", message), "/help");
        let (db, _, _, profile, _) = fixture("slack_file_share_help", "slack").await;
        let verified = verified_input(&db, &profile, message).await.unwrap();
        assert_eq!(
            verified.command_eligibility,
            ManagerCommandEligibility::Ineligible
        );
        let mut bot = message.clone();
        bot.raw_data["event"]["bot_id"] = json!("B1");
        assert!(!authenticated_human("slack", "B1", &bot));
        let mut edit = message.clone();
        edit.raw_data["event"]["subtype"] = json!("message_changed");
        assert!(!authenticated_human("slack", "B1", &edit));
        let mut group = message.clone();
        group.conversation_type = "group".into();
        assert!(!authenticated_human("slack", "B1", &group));
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn peers_are_durable_exact_and_events_do_not_allocate_per_message_rows() {
        let (db, _, _, profile, _) = fixture("generic_peer_identity", "slack").await;
        let one = input(json!({"event":{"type":"message","user":"U1"}}), "U1", "D1");
        let first = verified_input(&db, &profile, &one).await.unwrap();
        let mut another = one.clone();
        another.platform_message_id = "next".into();
        let next = verified_input(&db, &profile, &another).await.unwrap();
        assert_eq!(first.sender_id, next.sender_id);
        assert_ne!(first.message_id, next.message_id);
        assert_eq!(
            first.message_id,
            verified_input(&db, &profile, &one)
                .await
                .unwrap()
                .message_id
        );
        another.conversation_id = "D2".into();
        assert_ne!(
            first.sender_id,
            verified_input(&db, &profile, &another)
                .await
                .unwrap()
                .sender_id
        );
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            peer(&db, &profile.id, first.sender_id)
                .await
                .unwrap()
                .sender_display_name
                .as_deref(),
            Some("Alice")
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn captured_send_cannot_adopt_credentials_rotated_after_the_claim() {
        let (db, _, _, profile, bot) = fixture("channel_send_snapshot_race", "slack").await;
        let state = crate::test_utils::test_app_state(db.clone());
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            fence_transport(&db, &mut session, &profile, None, true)
                .await
                .unwrap()
        );
        let snapshot = CredentialSnapshot::load(&db, &mut session, &bot, true)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        let replacement = state
            .encryption_keys
            .encrypt(b"replacement-after-claim")
            .await
            .unwrap();
        db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc! {"_id":&bot.id},doc! {"$set":{"bot_token_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:replacement}}}).await.unwrap();
        assert_eq!(
            snapshot
                .token(&state.encryption_keys)
                .await
                .unwrap()
                .as_str(),
            "snapshot-original"
        );
        assert!(
            !fence_transport(&db, &mut session, &profile, None, false)
                .await
                .unwrap()
        );
        db.drop().await.unwrap();
    }
    async fn oauth_fixture(
        name: &str,
    ) -> (
        Database,
        VerifiedManagerUpdate,
        String,
        PlatformBotConfig,
        ChannelBot,
        String,
        String,
    ) {
        let (db, mut update, owner, mut profile, mut bot) = fixture(name, "slack").await;
        let state = crate::test_utils::test_app_state(db.clone());
        let encrypted = |bytes: Vec<u8>| bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes,
        };
        let key = uuid::Uuid::new_v4().to_string();
        let provider = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>(crate::models::provider_config::COLLECTION_NAME).insert_one(doc! {
            "_id":&provider,"slug":"twitter","name":"X","provider_type":"oauth2","is_active":true,"created_by":&owner,
            "client_id_encrypted":encrypted(state.encryption_keys.encrypt(b"original-client").await.unwrap()),
            "client_secret_encrypted":encrypted(state.encryption_keys.encrypt(b"original-provider-secret").await.unwrap()),
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).await.unwrap();
        db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME).insert_one(doc! {
            "_id":&key,"user_id":&owner,"label":"X connection","credential_type":"oauth2","status":"active","credential_source":"platform","provider_config_id":&provider,
            "token_scopes":super::super::channel_adapters::x::REQUIRED_SCOPES.join(" "),"credential_epoch":1_i64,
            "access_token_encrypted":encrypted(state.encryption_keys.encrypt(b"original-oauth-token").await.unwrap()),
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).await.unwrap();
        db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME).insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"provider":"x","fields":{},
            "secrets":{"consumer_secret":encrypted(state.encryption_keys.encrypt(b"original-platform-secret").await.unwrap())},"updated_by":&owner,"updated_at":bson::DateTime::now()
        }).await.unwrap();
        bot.platform = "x".into();
        bot.credential_source = "connection".into();
        bot.connection_id = Some(key.clone());
        db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .replace_one(doc! {"_id":&bot.id}, &bot)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&bot.id},
                doc! {"$set":{"platform_profile_id":&profile.id}},
            )
            .await
            .unwrap();
        profile.manager_bot_id = format!("channel:x:{}", bot.id);
        let transport = profile.channel_transport.as_mut().unwrap();
        transport.platform = "x".into();
        let mut session = db.client().start_session().await.unwrap();
        transport.credential_stamp = credential_stamp(&db, &mut session, &bot, false)
            .await
            .unwrap();
        db.collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
            .replace_one(doc! {"_id":&profile.id}, &profile)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
            .update_one(
                doc! {"platform_config_id":&profile.id},
                doc! {"$set":{"manager_bot_id":&profile.manager_bot_id}},
            )
            .await
            .unwrap();
        update.manager_bot_id = profile.manager_bot_id.clone();
        (db, update, owner, profile, bot, key, provider)
    }
    #[tokio::test]
    async fn captured_snapshot_retains_platform_and_oauth_material_after_concurrent_rotation() {
        let (db, _, _, profile, bot, key, provider) =
            oauth_fixture("channel_oauth_snapshot_race").await;
        let state = crate::test_utils::test_app_state(db.clone());
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            fence_transport(&db, &mut session, &profile, None, true)
                .await
                .unwrap()
        );
        let snapshot = CredentialSnapshot::load(&db, &mut session, &bot, true)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        let replacement = bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: state
                .encryption_keys
                .encrypt(b"rotated-after-claim")
                .await
                .unwrap(),
        };
        db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
            .update_one(
                doc! {"provider":"x"},
                doc! {"$set":{"secrets.consumer_secret":&replacement}},
            )
            .await
            .unwrap();
        assert!(
            !fence_transport(&db, &mut session, &profile, None, false)
                .await
                .unwrap()
        );
        db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME).update_one(doc! {"_id":&key},vec![doc! {"$set":{"credential_epoch":{"$add":["$credential_epoch",1_i64]},"access_token_encrypted":&replacement}}]).await.unwrap();
        db.collection::<Document>(crate::models::provider_config::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&provider},
                doc! {"$set":{"client_secret_encrypted":&replacement}},
            )
            .await
            .unwrap();
        assert_eq!(
            snapshot
                .token(&state.encryption_keys)
                .await
                .unwrap()
                .as_str(),
            "original-oauth-token"
        );
        let adapter =
            super::super::channel_adapters::resolve_adapter("x", &state.token_exchange_cache)
                .unwrap();
        let secrets = snapshot
            .secrets(&state.encryption_keys, adapter.as_ref())
            .await
            .unwrap();
        assert_eq!(
            secrets.get("consumer_secret"),
            Some("original-platform-secret")
        );
        assert_eq!(
            secrets.get("client_secret"),
            Some("original-provider-secret")
        );
        assert!(
            !fence_transport(&db, &mut session, &profile, None, false)
                .await
                .unwrap()
        );
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn unavailable_oauth_returns_repair_status_and_disconnected_inventory() {
        use super::super::platform_personal_binding_service as bindings;
        let (db, update, owner, profile, _, key, provider) =
            oauth_fixture("channel_oauth_recovery_status").await;
        db.collection::<crate::models::platform_personal_binding::PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        )
        .insert_one(bindings::tests::binding(&update, &owner))
        .await
        .unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        for (collection, filter, changes) in [
            (
                crate::models::user_api_key::COLLECTION_NAME,
                doc! {"_id":&key},
                doc! {"status":"revoked"},
            ),
            (
                crate::models::user_api_key::COLLECTION_NAME,
                doc! {"_id":&key},
                doc! {"status":"expired"},
            ),
            (
                crate::models::user_api_key::COLLECTION_NAME,
                doc! {"_id":&key},
                doc! {"token_scopes":"users.read"},
            ),
            (
                crate::models::user_api_key::COLLECTION_NAME,
                doc! {"_id":&key},
                doc! {"expires_at":bson::DateTime::from_millis(0)},
            ),
            (
                crate::models::provider_config::COLLECTION_NAME,
                doc! {"_id":&provider},
                doc! {"is_active":false},
            ),
        ] {
            let rows = db.collection::<Document>(collection);
            let before = rows.find_one(filter.clone()).await.unwrap().unwrap();
            rows.update_one(filter.clone(), doc! {"$set":changes})
                .await
                .unwrap();
            assert_eq!(
                super::super::platform_bot_status_service::view(&state, &profile.id)
                    .await
                    .unwrap()
                    .state,
                "connection_repair_required"
            );
            assert_eq!(
                bindings::list_owned(&db, &owner, None).await.unwrap()[0].state,
                crate::models::platform_personal_binding::PersonalBindingState::Revoked
            );
            assert!(
                bindings::manager_inventory_display(
                    &db,
                    &profile.id,
                    &profile.manager_bot_id,
                    false
                )
                .await
                .is_ok()
            );
            rows.replace_one(filter, &before).await.unwrap();
        }
        db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME)
            .delete_one(doc! {"_id":&key})
            .await
            .unwrap();
        assert_eq!(
            super::super::platform_bot_status_service::view(&state, &profile.id)
                .await
                .unwrap()
                .state,
            "connection_repair_required"
        );
        assert_eq!(
            bindings::list_owned(&db, &owner, None).await.unwrap()[0].state,
            crate::models::platform_personal_binding::PersonalBindingState::Revoked
        );
        db.drop().await.unwrap();
    }
}
