use crate::{
    crypto::{aes::EncryptionKeys, local_key_provider::LocalKeyProvider},
    models::{
        downstream_service::DownstreamService,
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

pub(super) enum KeySecret {
    Credential,
    AccessToken,
    OAuthClientId,
}

impl KeySecret {
    pub(super) async fn set(self, keys: &EncryptionKeys, row: &mut UserApiKey, plaintext: &[u8]) {
        let (field, slot) = match self {
            Self::Credential => ("credential_encrypted", &mut row.credential_encrypted),
            Self::AccessToken => ("access_token_encrypted", &mut row.access_token_encrypted),
            Self::OAuthClientId => (
                "user_oauth_client_id_encrypted",
                &mut row.user_oauth_client_id_encrypted,
            ),
        };
        *slot = Some(
            credential_record_service::encrypt(
                keys,
                COLLECTION_NAME,
                &row.id,
                &row.user_id,
                field,
                plaintext,
            )
            .await
            .expect("encrypt fixture key field"),
        );
        credential_record_service::mark_version(&mut row.secret_context_versions, keys, field);
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
