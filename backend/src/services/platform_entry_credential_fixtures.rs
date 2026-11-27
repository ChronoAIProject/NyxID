use crate::crypto::aes::EncryptionKeys;
use crate::models::channel_bot::ChannelBot;
use crate::services::{channel_bot_record, credential_record_service};
use bson::{Binary, Document, doc, spec::BinarySubtype};

pub(crate) enum BotSecret {
    Token,
    AppSecret,
    LarkVerificationToken,
    LarkEncryptKey,
}

impl BotSecret {
    pub(crate) async fn set(self, keys: &EncryptionKeys, bot: &mut ChannelBot, value: &str) {
        let field = match self {
            Self::Token => "bot_token_encrypted",
            Self::AppSecret => "app_secret_encrypted",
            Self::LarkVerificationToken => "lark_verification_token_encrypted",
            Self::LarkEncryptKey => "lark_encrypt_key_encrypted",
        };
        let bytes = channel_bot_record::encrypt(keys, &bot.id, &bot.user_id, field, value)
            .await
            .unwrap();
        match self {
            Self::Token => bot.bot_token_encrypted = bytes,
            Self::AppSecret => bot.app_secret_encrypted = Some(bytes),
            Self::LarkVerificationToken => bot.lark_verification_token_encrypted = Some(bytes),
            Self::LarkEncryptKey => bot.lark_encrypt_key_encrypted = Some(bytes),
        }
        credential_record_service::mark_version(&mut bot.secret_context_versions, keys, field);
    }
}

pub(crate) async fn token_update(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    token: &str,
    mut set: Document,
) -> Document {
    let field = "bot_token_encrypted";
    let bytes = channel_bot_record::encrypt(keys, id, owner, field, token)
        .await
        .unwrap();
    set.insert(
        field,
        Binary {
            subtype: BinarySubtype::Generic,
            bytes,
        },
    );
    marked_update(keys, field, set)
}

pub(crate) async fn connection_token(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    field: &str,
    value: &[u8],
) -> Binary {
    let bytes = credential_record_service::encrypt(
        keys,
        crate::models::user_api_key::COLLECTION_NAME,
        id,
        owner,
        field,
        value,
    )
    .await
    .unwrap();
    Binary {
        subtype: BinarySubtype::Generic,
        bytes,
    }
}

pub(crate) async fn seed_x_access_token(keys: &EncryptionKeys, row: &mut Document) {
    let field = "access_token_encrypted";
    let token = connection_token(
        keys,
        row.get_str("_id").unwrap(),
        row.get_str("user_id").unwrap(),
        field,
        b"synthetic-user-token",
    )
    .await;
    row.insert(field, token);
    let versions = credential_record_service::VERSIONS;
    if keys.contextual_record_writes() {
        if !row.contains_key(versions) {
            row.insert(versions, Document::new());
        }
        row.get_document_mut(versions).unwrap().insert(field, 3);
    } else if row.contains_key(versions) {
        row.get_document_mut(versions).unwrap().remove(field);
    }
}

pub(crate) async fn refresh_update(
    keys: &EncryptionKeys,
    bot: &ChannelBot,
    mut set: Document,
) -> Document {
    let field = "refresh_token_encrypted";
    set.insert(
        field,
        connection_token(
            keys,
            bot.connection_id.as_deref().unwrap(),
            &bot.user_id,
            field,
            b"synthetic-refresh-token",
        )
        .await,
    );
    marked_update(keys, field, set)
}

fn marked_update(keys: &EncryptionKeys, field: &str, mut set: Document) -> Document {
    let mut unset = Document::new();
    credential_record_service::mark_classic_update(&mut set, &mut unset, keys, field);
    let mut update = doc! {"$set": set};
    if !unset.is_empty() {
        update.insert("$unset", unset);
    }
    update
}
