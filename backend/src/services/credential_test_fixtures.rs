use super::{
    catalog_credential_record_service as catalog, credential_record_service as record,
    oauth_client_webhook_record as webhook,
};
use crate::{
    crypto::aes::EncryptionKeys,
    models::{
        downstream_service::DownstreamService,
        oauth_client::OauthClient,
        provider_config::{self, ProviderConfig},
        user_api_key,
    },
};
use bson::{Document, doc};

pub(super) async fn api_key_fields(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    value: &[u8],
) -> Document {
    let field = "credential_encrypted";
    let bytes = record::encrypt(keys, user_api_key::COLLECTION_NAME, id, owner, field, value)
        .await
        .unwrap();
    let mut versions = Default::default();
    record::mark_version(&mut versions, keys, field);
    doc! {field: bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes }, "secret_context_versions": bson::to_bson(&versions).unwrap()}
}

pub(super) async fn catalog_secret(
    keys: &EncryptionKeys,
    row: &mut DownstreamService,
    value: &[u8],
) {
    row.credential_encrypted = catalog::encrypt(keys, &row.id, catalog::owner(row).unwrap(), value)
        .await
        .unwrap();
    row.secret_context_versions = catalog::initial_versions(keys, true);
}

pub(super) async fn provider_client(
    keys: &EncryptionKeys,
    row: &mut ProviderConfig,
    client_id: &str,
    client_secret: Option<&str>,
) {
    for (field, slot, value) in [
        (
            "client_id_encrypted",
            &mut row.client_id_encrypted,
            Some(client_id),
        ),
        (
            "client_secret_encrypted",
            &mut row.client_secret_encrypted,
            client_secret,
        ),
    ] {
        if let Some(value) = value {
            *slot = Some(
                record::encrypt(
                    keys,
                    provider_config::COLLECTION_NAME,
                    &row.id,
                    record::PLATFORM_OWNER,
                    field,
                    value.as_bytes(),
                )
                .await
                .unwrap(),
            );
            record::mark_version(&mut row.secret_context_versions, keys, field);
        }
    }
}

pub(super) async fn connection_webhook_secret(
    keys: &EncryptionKeys,
    row: &mut OauthClient,
    secret: &str,
) {
    row.connection_webhook_secret_encrypted = Some(
        webhook::encrypt(
            keys,
            &row.id,
            row.created_by.as_deref().unwrap(),
            webhook::CONNECTION,
            secret,
        )
        .await
        .unwrap(),
    );
    record::mark_version(&mut row.secret_context_versions, keys, webhook::CONNECTION);
}
