//! NyxID-issued refresh credentials in broker bindings. These are not external
//! provider token holders. Historical ciphertext has binding-hash AAD; v3 adds
//! the immutable owner/client and exact field to its authenticated context.
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::oauth_broker_binding::{COLLECTION_NAME, OauthBrokerBinding},
};
use bson::{Document, doc};
use zeroize::Zeroizing;

const FIELD: &str = "refresh_token_encrypted";
fn invalid() -> AppError {
    AppError::Internal("Broker credential requires authenticated repair".into())
}
fn identity(id: &str, owner: &str, client: &str) -> AppResult<()> {
    if !super::oauth_broker_service::is_binding_hash(id) || owner.is_empty() || client.is_empty() {
        return Err(invalid());
    }
    RecordContext::new(COLLECTION_NAME, id, owner, &[FIELD, client])?;
    Ok(())
}
fn validate(row: &OauthBrokerBinding) -> AppResult<()> {
    identity(&row.id, &row.user_id, &row.client_id)?;
    if row.secret_context_versions.iter().any(|(field, version)| {
        field != FIELD || *version != 3 || row.refresh_token_encrypted.is_none()
    }) || row
        .refresh_token_encrypted
        .as_ref()
        .is_some_and(Vec::is_empty)
    {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) fn versions(keys: &EncryptionKeys) -> std::collections::BTreeMap<String, u8> {
    let mut versions = std::collections::BTreeMap::new();
    super::credential_record_service::mark_version(&mut versions, keys, FIELD);
    versions
}
pub(crate) fn version_update(keys: &EncryptionKeys) -> AppResult<bson::Bson> {
    bson::to_bson(&versions(keys)).map_err(|_| invalid())
}
pub(crate) async fn encrypt(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    client: &str,
    plaintext: &[u8],
) -> AppResult<Vec<u8>> {
    identity(id, owner, client)?;
    let components = [FIELD, client];
    let context = RecordContext::new(COLLECTION_NAME, id, owner, &components)?;
    if plaintext.is_empty() || std::str::from_utf8(plaintext).is_err() {
        return Err(invalid());
    }
    if keys.contextual_record_writes() {
        keys.encrypt_record(plaintext, &context).await
    } else {
        keys.encrypt_with_aad(plaintext, id.as_bytes()).await
    }
}
pub(crate) async fn text(
    keys: &EncryptionKeys,
    row: &OauthBrokerBinding,
) -> AppResult<Zeroizing<String>> {
    validate(row)?;
    let ciphertext = row.refresh_token_encrypted.as_ref().ok_or_else(invalid)?;
    let bytes = match row.secret_context_versions.get(FIELD) {
        Some(3) => {
            keys.decrypt_record(
                ciphertext,
                &RecordContext::new(
                    COLLECTION_NAME,
                    &row.id,
                    &row.user_id,
                    &[FIELD, &row.client_id],
                )?,
            )
            .await?
        }
        None if keys.allow_legacy_record_reads() => {
            Zeroizing::new(keys.decrypt_with_aad(ciphertext, row.id.as_bytes()).await?)
        }
        _ => return Err(invalid()),
    };
    if bytes.is_empty() {
        return Err(invalid());
    }
    Ok(std::str::from_utf8(&bytes)
        .map_err(|_| invalid())?
        .to_owned()
        .into())
}
pub(crate) async fn verify_current_snapshot(
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<()> {
    let row: OauthBrokerBinding = bson::from_document(source.clone()).map_err(|_| invalid())?;
    validate(&row)?;
    if let Some(ciphertext) = &row.refresh_token_encrypted {
        if row.secret_context_versions.get(FIELD) != Some(&3) {
            return Err(invalid());
        }
        keys.authenticate_current_record(
            ciphertext,
            &RecordContext::new(
                COLLECTION_NAME,
                &row.id,
                &row.user_id,
                &[FIELD, &row.client_id],
            )?,
        )
        .await?;
        text(keys, &row).await?;
    }
    Ok(())
}
pub(crate) async fn migrate_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    let row: OauthBrokerBinding = bson::from_document(source.clone()).map_err(|_| invalid())?;
    validate(&row)?;
    if row.refresh_token_encrypted.is_none() {
        return Ok(false);
    }
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    if verify_current_snapshot(keys, source).await.is_ok() {
        return Ok(false);
    }
    let value = text(keys, &row).await?;
    let ciphertext = encrypt(
        keys,
        &row.id,
        &row.user_id,
        &row.client_id,
        value.as_bytes(),
    )
    .await?;
    let set = doc! {FIELD: bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext}, "secret_context_versions":version_update(keys)?};
    let mut candidate = source.clone();
    candidate.extend(set.clone());
    verify_current_snapshot(keys, &candidate).await?;
    let db = super::transaction_service::durable_database(db);
    Ok(db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            super::record_storage::exact_document_predicate(source),
            doc! {"$set":set},
        )
        .await?
        .matched_count
        == 1)
}
