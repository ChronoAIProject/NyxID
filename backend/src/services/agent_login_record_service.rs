//! Agent-key login delivery is bound to its one request, approving person,
//! selected parent and exact issued child. These are NyxID-issued credentials;
//! they are not external-provider OAuth holder inventory.
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::agent_key_login_request::{AgentKeyLoginRequest, AgentKeyLoginStatus, COLLECTION_NAME},
};
use bson::{Document, doc};
use zeroize::Zeroizing;
const FIELD: &str = "delivery_credential_encrypted";
fn invalid() -> AppError {
    AppError::Internal("Agent login delivery requires authenticated repair".into())
}
pub(crate) fn version(keys: &EncryptionKeys) -> Option<u8> {
    keys.contextual_record_writes().then_some(3)
}
pub(crate) async fn encrypt(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    parent: &str,
    child: &str,
    secret: &[u8],
) -> AppResult<Vec<u8>> {
    if secret.is_empty() || secret.len() > 4096 {
        return Err(invalid());
    }
    super::record_field_service::encrypt_context(
        keys,
        &RecordContext::new(COLLECTION_NAME, id, owner, &[FIELD, parent, child])?,
        secret,
    )
    .await
}
fn binding(row: &AgentKeyLoginRequest) -> AppResult<(&str, &str, &str)> {
    Ok((
        row.approved_user_id.as_deref().ok_or_else(invalid)?,
        row.api_key_id.as_deref().ok_or_else(invalid)?,
        row.credential_id.as_deref().ok_or_else(invalid)?,
    ))
}
pub(crate) async fn decrypt(
    keys: &EncryptionKeys,
    row: &AgentKeyLoginRequest,
) -> AppResult<Zeroizing<String>> {
    let (owner, parent, child) = binding(row)?;
    let encrypted = row
        .delivery_credential_encrypted
        .as_deref()
        .filter(|bytes| !bytes.is_empty())
        .ok_or_else(invalid)?;
    let bytes = super::record_field_service::decrypt_context(
        keys,
        &RecordContext::new(COLLECTION_NAME, &row.id, owner, &[FIELD, parent, child])?,
        row.delivery_context_version,
        encrypted,
    )
    .await?;
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(invalid());
    }
    Ok(Zeroizing::new(
        std::str::from_utf8(&bytes)
            .map_err(|_| invalid())?
            .to_owned(),
    ))
}
fn row(source: &Document) -> AppResult<AgentKeyLoginRequest> {
    let row: AgentKeyLoginRequest = bson::from_document(source.clone()).map_err(|_| invalid())?;
    if row
        .delivery_context_version
        .is_some_and(|version| version != 3)
        || (row.delivery_credential_encrypted.is_none() && row.delivery_context_version.is_some())
        || row
            .delivery_credential_encrypted
            .as_ref()
            .is_some_and(|bytes| bytes.is_empty())
    {
        return Err(invalid());
    }
    if row.delivery_credential_encrypted.is_some() && row.status != AgentKeyLoginStatus::Approved {
        return Err(invalid());
    }
    Ok(row)
}
pub(crate) async fn verify_current_snapshot(
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<()> {
    let row = row(source)?;
    let Some(encrypted) = row.delivery_credential_encrypted.as_deref() else {
        return Ok(());
    };
    if row.delivery_context_version != Some(3) {
        return Err(invalid());
    }
    let (owner, parent, child) = binding(&row)?;
    keys.authenticate_current_record(
        encrypted,
        &RecordContext::new(COLLECTION_NAME, &row.id, owner, &[FIELD, parent, child])?,
    )
    .await?;
    decrypt(keys, &row).await?;
    Ok(())
}
pub(crate) async fn migrate_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    let row = row(source)?;
    if row.delivery_credential_encrypted.is_none() {
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
    let plain = decrypt(keys, &row).await?;
    let (owner, parent, child) = binding(&row)?;
    let encrypted = encrypt(keys, &row.id, owner, parent, child, plain.as_bytes()).await?;
    let set = doc! {FIELD:bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},"delivery_context_version":3_i32};
    let mut candidate = source.clone();
    candidate.extend(set.clone());
    verify_current_snapshot(keys, &candidate).await?;
    Ok(super::transaction_service::durable_database(db)
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            super::record_storage::exact_document_predicate(source),
            doc! {"$set":set},
        )
        .await?
        .matched_count
        == 1)
}
