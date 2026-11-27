use crate::{
    crypto::{aes::EncryptionKeys, local_key_provider::LocalKeyProvider},
    models::user_api_key::{COLLECTION_NAME, UserApiKey},
    services::credential_record_service,
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
