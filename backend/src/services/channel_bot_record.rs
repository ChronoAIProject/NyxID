//! The six local bot credentials share the bot's record ID and current owner.
//! A connection or manager can supply the token instead of the local row.
use super::{channel_credential_continuity as continuity, channel_registration};
use crate::{
    crypto::{aes::EncryptionKeys, record_field},
    errors::{AppError, AppResult},
    models::channel_bot::{COLLECTION_NAME, ChannelBot},
};
use bson::Document;
use sha2::{Digest, Sha256};

pub(super) const FIELDS: &[&str] = &[
    "registration_pin_encrypted",
    "webhook_secret_encrypted",
    "bot_token_encrypted",
    "app_secret_encrypted",
    "lark_verification_token_encrypted",
    "lark_encrypt_key_encrypted",
];

fn invalid() -> AppError {
    AppError::Internal("Invalid channel bot credential storage".into())
}

pub(super) fn external_token(bot: &ChannelBot, field: &str) -> bool {
    field == continuity::TOKEN
        && bot.bot_token_encrypted.is_empty()
        && matches!(
            bot.credential_source.as_str(),
            "connection" | "telegram_manager"
        )
        && !bot.secret_context_versions.contains_key(field)
}

pub(super) async fn encrypt(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    field: &str,
    value: &str,
) -> AppResult<Vec<u8>> {
    record_field::encrypt(keys, COLLECTION_NAME, id, owner, field, value.as_bytes()).await
}

pub(super) fn mark_new(bot: &mut ChannelBot, keys: &EncryptionKeys) -> AppResult<()> {
    if keys.contextual_record_writes() {
        for &field in FIELDS {
            if !external_token(bot, field)
                && channel_registration::stored_secret(bot, field)?.is_some()
            {
                bot.secret_context_versions.insert(field.to_owned(), 3);
            }
        }
    }
    Ok(())
}

pub(super) fn decode(source: &Document) -> AppResult<ChannelBot> {
    let bot: ChannelBot = bson::from_document(source.clone()).map_err(|_| invalid())?;
    for (field, version) in &bot.secret_context_versions {
        if *version != 3
            || !FIELDS.contains(&field.as_str())
            || channel_registration::stored_secret(&bot, field)?.is_none()
        {
            return Err(invalid());
        }
    }
    continuity::verified(&bot)?;
    Ok(bot)
}

pub(super) async fn plaintext(
    keys: &EncryptionKeys,
    bot: &ChannelBot,
    field: &str,
) -> AppResult<Option<zeroize::Zeroizing<Vec<u8>>>> {
    if external_token(bot, field) {
        return Ok(None);
    }
    let value = channel_registration::open_secret(keys, bot, field).await?;
    if let Some(bytes) = &value {
        std::str::from_utf8(bytes).map_err(|_| invalid())?;
        if field == "webhook_secret_encrypted"
            && hex::encode(Sha256::digest(bytes)) != bot.webhook_secret_hash
        {
            return Err(invalid());
        }
    }
    Ok(value)
}

pub(super) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    let bot = decode(source)?;
    for &field in FIELDS {
        if external_token(&bot, field) {
            continue;
        }
        if let Some(bytes) = channel_registration::stored_secret(&bot, field)? {
            record_field::verify_current(
                keys,
                COLLECTION_NAME,
                &bot.id,
                &bot.user_id,
                field,
                bot.secret_context_versions.get(field).copied(),
                bytes,
            )
            .await?;
            plaintext(keys, &bot, field).await?;
        }
    }
    Ok(())
}

pub(crate) async fn migrate(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    let bot = decode(source)?;
    let mut fields = Vec::new();
    for &field in FIELDS {
        if !external_token(&bot, field) {
            plaintext(keys, &bot, field).await?;
            fields.push(field);
        }
    }
    super::record_field_service::migrate_snapshot(
        db,
        keys,
        COLLECTION_NAME,
        source,
        &bot.user_id,
        &fields,
    )
    .await
}
