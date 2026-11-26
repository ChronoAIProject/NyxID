//! OAuth webhook signing keys are scoped to their registration and purpose.
//! Client management resolves the person or organization before calling a writer.
use bson::{Bson, Document};
use zeroize::Zeroizing;

use crate::{
    crypto::{aes::EncryptionKeys, record_field},
    errors::{AppError, AppResult},
    models::oauth_client::{COLLECTION_NAME, OauthClient},
};

pub(super) const REVOCATION: &str = "revocation_webhook_secret_encrypted";
pub(super) const CONNECTION: &str = "connection_webhook_secret_encrypted";
pub(super) const FIELDS: &[&str] = &[REVOCATION, CONNECTION];

fn invalid() -> AppError {
    AppError::Internal("Invalid OAuth client webhook credential storage".into())
}

fn owner(creator: Option<&str>) -> String {
    match creator {
        None => "registration:ownerless".into(),
        Some("system") => "registration:system".into(),
        Some("dynamic_registration") => "registration:dynamic".into(),
        Some(id) => format!("creator:{id}"),
    }
}

pub(super) async fn encrypt(
    keys: &EncryptionKeys,
    id: &str,
    creator: &str,
    field: &str,
    secret: &str,
) -> AppResult<Vec<u8>> {
    record_field::encrypt(
        keys,
        COLLECTION_NAME,
        id,
        &owner(Some(creator)),
        field,
        secret.as_bytes(),
    )
    .await
}

pub(super) async fn open(
    keys: &EncryptionKeys,
    client: &OauthClient,
    field: &str,
    ciphertext: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    record_field::decrypt(
        keys,
        COLLECTION_NAME,
        &client.id,
        &owner(client.created_by.as_deref()),
        field,
        client.secret_context_versions.get(field).copied(),
        ciphertext,
    )
    .await
}

async fn check_snapshot(
    keys: &EncryptionKeys,
    source: &Document,
    require_primary: bool,
) -> AppResult<String> {
    let id = source.get_str("_id").map_err(|_| invalid())?;
    let creator = match source.get("created_by") {
        None | Some(Bson::Null) => None,
        Some(Bson::String(value)) => Some(value.as_str()),
        _ => return Err(invalid()),
    };
    let owner = owner(creator);
    let versions = super::record_field_service::stored_versions(source)?;
    if versions.iter().any(|(field, version)| {
        *version != 3
            || !FIELDS.contains(&field.as_str())
            || source.get(field).is_none_or(|v| v == &Bson::Null)
    }) {
        return Err(invalid());
    }
    for &field in FIELDS {
        let Some(value) = source.get(field).filter(|v| *v != &Bson::Null) else {
            continue;
        };
        let bytes = super::record_field_service::stored_bytes(value)?;
        let version = versions.get(field).copied();
        if require_primary {
            record_field::verify_current(keys, COLLECTION_NAME, id, &owner, field, version, &bytes)
                .await?;
        }
        let secret =
            record_field::decrypt(keys, COLLECTION_NAME, id, &owner, field, version, &bytes)
                .await?;
        if secret.is_empty() || std::str::from_utf8(&secret).is_err() {
            return Err(invalid());
        }
    }
    Ok(owner)
}

pub(super) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    check_snapshot(keys, source, true).await.map(|_| ())
}

pub(super) async fn migrate(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    let owner = check_snapshot(keys, source, false).await?;
    super::record_field_service::migrate_snapshot(db, keys, COLLECTION_NAME, source, &owner, FIELDS)
        .await
}
