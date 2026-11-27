use crate::{
    crypto::{aes::EncryptionKeys, local_key_provider::LocalKeyProvider, record_field},
    models::{
        channel_bot::{COLLECTION_NAME as CHANNEL_BOTS, ChannelBot},
        downstream_service::DownstreamService,
        mfa_factor::{COLLECTION_NAME as MFA_FACTORS, MfaFactor},
        provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig},
        user_api_key::{COLLECTION_NAME, UserApiKey},
        user_service_connection::{COLLECTION_NAME as CONNECTIONS, UserServiceConnection},
    },
    services::{
        catalog_credential_record_service as catalog, connection_record_service,
        credential_record_service,
    },
};
use std::sync::Arc;

pub(super) fn test_encryption_keys() -> EncryptionKeys {
    EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new([0x22; 32], None)))
}

pub(super) fn unrelated_encryption_keys() -> EncryptionKeys {
    EncryptionKeys::with_provider(Arc::new(LocalKeyProvider::new([0x99; 32], None)))
}

pub(super) fn proxy_node_signing_secret() -> String {
    "11".repeat(32)
}

#[derive(Clone, Copy)]
pub(super) enum BotSecret {
    Token,
    AppSecret,
    LarkVerificationToken,
    LarkEncryptKey,
}

impl BotSecret {
    fn field(self) -> &'static str {
        match self {
            Self::Token => "bot_token_encrypted",
            Self::AppSecret => "app_secret_encrypted",
            Self::LarkVerificationToken => "lark_verification_token_encrypted",
            Self::LarkEncryptKey => "lark_encrypt_key_encrypted",
        }
    }

    async fn encrypt_for_bot(
        self,
        keys: &EncryptionKeys,
        id: &str,
        owner: &str,
        plaintext: &[u8],
    ) -> Vec<u8> {
        record_field::encrypt(keys, CHANNEL_BOTS, id, owner, self.field(), plaintext)
            .await
            .unwrap()
    }

    pub(super) async fn set(self, keys: &EncryptionKeys, bot: &mut ChannelBot, plaintext: &[u8]) {
        let bytes = self
            .encrypt_for_bot(keys, &bot.id, &bot.user_id, plaintext)
            .await;
        match self {
            Self::Token => bot.bot_token_encrypted = bytes,
            Self::AppSecret => bot.app_secret_encrypted = Some(bytes),
            Self::LarkVerificationToken => bot.lark_verification_token_encrypted = Some(bytes),
            Self::LarkEncryptKey => bot.lark_encrypt_key_encrypted = Some(bytes),
        }
        credential_record_service::mark_version(
            &mut bot.secret_context_versions,
            keys,
            self.field(),
        );
    }

    pub(super) async fn set_document(
        self,
        keys: &EncryptionKeys,
        bot: &mut bson::Document,
        plaintext: &[u8],
    ) {
        let bytes = self
            .encrypt_for_bot(
                keys,
                bot.get_str("_id").unwrap(),
                bot.get_str("user_id").unwrap(),
                plaintext,
            )
            .await;
        bot.insert(
            self.field(),
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes,
            },
        );
        if keys.contextual_record_writes() {
            if !bot.contains_key(credential_record_service::VERSIONS) {
                bot.insert(credential_record_service::VERSIONS, bson::Document::new());
            }
            bot.get_document_mut(credential_record_service::VERSIONS)
                .unwrap()
                .insert(self.field(), 3);
        } else if let Ok(versions) = bot.get_document_mut(credential_record_service::VERSIONS) {
            versions.remove(self.field());
        }
    }

    pub(super) async fn update(
        keys: &EncryptionKeys,
        id: &str,
        owner: &str,
        mut set: bson::Document,
        secrets: &[(Self, &[u8])],
    ) -> bson::Document {
        let mut unset = bson::Document::new();
        for &(secret, plaintext) in secrets {
            let destination = set.get_str("user_id").unwrap_or(owner);
            let bytes = secret
                .encrypt_for_bot(keys, id, destination, plaintext)
                .await;
            set.insert(
                secret.field(),
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes,
                },
            );
            credential_record_service::mark_classic_update(
                &mut set,
                &mut unset,
                keys,
                secret.field(),
            );
        }
        let mut update = bson::doc! {"$set":set};
        if !unset.is_empty() {
            update.insert("$unset", unset);
        }
        update
    }

    pub(super) async fn insert(
        self,
        db: &mongodb::Database,
        keys: &EncryptionKeys,
        mut bot: bson::Document,
        plaintext: &[u8],
    ) {
        self.set_document(keys, &mut bot, plaintext).await;
        db.collection::<bson::Document>(CHANNEL_BOTS)
            .insert_one(bot)
            .await
            .unwrap();
    }
}

pub(super) async fn insert_recovery_factor(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    owner: &str,
    recovery_hash: &str,
) {
    let id = uuid::Uuid::new_v4().to_string();
    let secret = record_field::encrypt(
        keys,
        MFA_FACTORS,
        &id,
        owner,
        "secret_encrypted",
        b"JBSWY3DPEHPK3PXP",
    )
    .await
    .unwrap();
    db.collection::<MfaFactor>(MFA_FACTORS)
        .insert_one(MfaFactor {
            id,
            user_id: owner.to_owned(),
            factor_type: "totp".into(),
            secret_encrypted: Some(secret),
            secret_context_version: keys.contextual_record_writes().then_some(3),
            recovery_codes: Some(serde_json::json!([recovery_hash])),
            last_totp_counter: None,
            is_verified: true,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
}

#[derive(Clone, Copy)]
pub(super) enum KeySecret {
    Credential,
    AccessToken,
    OAuthClientId,
}

impl KeySecret {
    fn field(self) -> &'static str {
        match self {
            Self::Credential => "credential_encrypted",
            Self::AccessToken => "access_token_encrypted",
            Self::OAuthClientId => "user_oauth_client_id_encrypted",
        }
    }

    pub(super) async fn set(self, keys: &EncryptionKeys, row: &mut UserApiKey, plaintext: &[u8]) {
        let field = self.field();
        let encrypted = credential_record_service::encrypt(
            keys,
            COLLECTION_NAME,
            &row.id,
            &row.user_id,
            field,
            plaintext,
        )
        .await
        .unwrap();
        let slot = match self {
            Self::Credential => &mut row.credential_encrypted,
            Self::AccessToken => &mut row.access_token_encrypted,
            Self::OAuthClientId => &mut row.user_oauth_client_id_encrypted,
        };
        *slot = Some(encrypted);
        credential_record_service::mark_version(&mut row.secret_context_versions, keys, field);
    }

    pub(super) async fn insert(
        self,
        db: &mongodb::Database,
        keys: &EncryptionKeys,
        mut row: bson::Document,
        plaintext: &[u8],
    ) {
        let field = self.field();
        let bytes = credential_record_service::encrypt(
            keys,
            COLLECTION_NAME,
            row.get_str("_id").unwrap(),
            row.get_str("user_id").unwrap(),
            field,
            plaintext,
        )
        .await
        .unwrap();
        row.insert(
            field,
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes,
            },
        );
        let mut versions = Default::default();
        credential_record_service::mark_version(&mut versions, keys, field);
        if !versions.is_empty() {
            row.insert(
                credential_record_service::VERSIONS,
                bson::to_bson(&versions).unwrap(),
            );
        }
        db.collection::<bson::Document>(COLLECTION_NAME)
            .insert_one(row)
            .await
            .unwrap();
    }
}

pub(super) async fn set_catalog_credential(
    keys: &EncryptionKeys,
    row: &mut DownstreamService,
    plaintext: &[u8],
) {
    row.credential_encrypted =
        catalog::encrypt(keys, &row.id, catalog::owner(row).unwrap(), plaintext)
            .await
            .unwrap();
    row.secret_context_versions = catalog::initial_versions(keys, true);
}

pub(super) async fn insert_bearer_connection(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    owner: &str,
    service_id: &str,
    secret: &[u8],
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let mut connection = UserServiceConnection {
        id: id.clone(),
        user_id: owner.to_string(),
        service_id: service_id.to_string(),
        credential_encrypted: None,
        secret_context_versions: Default::default(),
        credential_type: Some("bearer".into()),
        credential_label: None,
        metadata: None,
        is_active: true,
        state_version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    connection_record_service::encrypt(keys, &mut connection, secret)
        .await
        .unwrap();
    db.collection::<UserServiceConnection>(CONNECTIONS)
        .insert_one(connection)
        .await
        .unwrap();
    id
}

pub(super) async fn set_provider_client(
    keys: &EncryptionKeys,
    provider: &mut ProviderConfig,
    client: &str,
    secret: Option<&str>,
) {
    for (field, slot, value) in [
        (
            "client_id_encrypted",
            &mut provider.client_id_encrypted,
            Some(client),
        ),
        (
            "client_secret_encrypted",
            &mut provider.client_secret_encrypted,
            secret,
        ),
    ] {
        *slot = match value {
            Some(value) => {
                let encrypted = credential_record_service::encrypt(
                    keys,
                    PROVIDER_CONFIGS,
                    &provider.id,
                    credential_record_service::PLATFORM_OWNER,
                    field,
                    value.as_bytes(),
                )
                .await
                .unwrap();
                credential_record_service::mark_version(
                    &mut provider.secret_context_versions,
                    keys,
                    field,
                );
                Some(encrypted)
            }
            None => {
                provider.secret_context_versions.remove(field);
                None
            }
        };
    }
}

// Captured from the legacy-empty-public insert in the bc0516469 management test.
pub(super) const LEGACY_EMPTY_CATALOG: &str = concat!(
    "0202003c1c177d49348bf9fd52f3c205fd29686de64c19ed9192385a9707f0bf",
    "f363c08a51dff9ba127fbe44471a04bfa602a7f8fe8b33321d4f87fc82f27ccd",
    "a16882ea982ea704b95d4232fd0cc3087043f846aecdbb9c91451b1f",
);

// Captured from the legacy-private-master insert in the bc0516469 management test.
pub(super) const LEGACY_PRIVATE_MASTER: &str = concat!(
    "0202003cb9b6fb191623f45ec1ced74f00bef7ae100b12d7c9548f9f5eb472f6",
    "6db3c4e756f68f49b1169163a0a3876883cf3118301af6e30c3e4b5a9e7686f5",
    "5fb61f938300e6292492144259b1deded8fb2049d243950402cb1ceb442feea5",
    "85b2d96a9718fd316f310b",
);
