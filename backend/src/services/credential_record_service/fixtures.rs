//! Writers for unmarked credential fixtures. The service under test can already
//! be on contextual writes while these source rows still predate cutover.
use super::{EncryptionKeys, PLATFORM_OWNER, encrypt};
use crate::models::{
    provider_config, user_api_key, user_provider_credentials, user_provider_token,
};

pub(crate) mod historical;

pub(crate) fn staged_keys() -> EncryptionKeys {
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(false);
    keys
}

pub(crate) async fn provider(id: &str, field: &str, value: &[u8]) -> Vec<u8> {
    encrypt(
        &staged_keys(),
        provider_config::COLLECTION_NAME,
        id,
        PLATFORM_OWNER,
        field,
        value,
    )
    .await
    .unwrap()
}

pub(crate) async fn key(id: &str, owner: &str, field: &str, value: &[u8]) -> Vec<u8> {
    encrypt(
        &staged_keys(),
        user_api_key::COLLECTION_NAME,
        id,
        owner,
        field,
        value,
    )
    .await
    .unwrap()
}

pub(crate) async fn token(id: &str, owner: &str, field: &str, value: &[u8]) -> Vec<u8> {
    encrypt(
        &staged_keys(),
        user_provider_token::COLLECTION_NAME,
        id,
        owner,
        field,
        value,
    )
    .await
    .unwrap()
}

pub(crate) async fn user_client(id: &str, owner: &str, field: &str, value: &[u8]) -> Vec<u8> {
    encrypt(
        &staged_keys(),
        user_provider_credentials::COLLECTION_NAME,
        id,
        owner,
        field,
        value,
    )
    .await
    .unwrap()
}
