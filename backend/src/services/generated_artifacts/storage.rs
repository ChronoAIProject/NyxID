//! Service-owned contextual-encryption adapter for the operator's bounded rewrap walk.
use super::*;
use sha2::{Digest, Sha256};

fn decode(source: &Document) -> AppResult<PendingArtifact> {
    bson::from_document(source.clone()).map_err(|_| invalid())
}
fn validate(row: &PendingArtifact, bytes: &[u8]) -> AppResult<()> {
    if row.state != "pending"
        || row.size <= 0
        || row.size as usize > sandbox::FILE_BYTES
        || bytes.len() != row.size as usize
        || hex::encode(Sha256::digest(bytes)) != row.sha256
        || row
            .secret_context_versions
            .iter()
            .any(|(field, version)| field != "data_encrypted" || *version != 3)
    {
        return Err(invalid());
    }
    Ok(())
}
/// Select the format only from the stored marker, never from ciphertext bytes.
pub(super) async fn decrypt(
    keys: &EncryptionKeys,
    row: &PendingArtifact,
) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
    let path = context_path(row);
    let context = RecordContext::new(ARTIFACTS, &row.id, &row.user_id, &path)?;
    let bytes = super::super::record_field_service::decrypt_context(
        keys,
        &context,
        row.secret_context_versions.get("data_encrypted").copied(),
        row.data_encrypted.as_deref().ok_or_else(invalid)?,
    )
    .await?;
    validate(row, &bytes)?;
    Ok(bytes)
}
pub(crate) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    let row = decode(source)?;
    let Some(ciphertext) = row.data_encrypted.as_deref() else {
        return if matches!(
            row.state.as_str(),
            "materialized" | "expired" | "discarded" | "missing"
        ) && row.secret_context_versions.is_empty()
        {
            Ok(())
        } else {
            Err(invalid())
        };
    };
    if row.secret_context_versions.get("data_encrypted") != Some(&3) {
        return Err(invalid());
    }
    let path = context_path(&row);
    let context = RecordContext::new(ARTIFACTS, &row.id, &row.user_id, &path)?;
    keys.authenticate_current_record(ciphertext, &context)
        .await?;
    let plain = keys.decrypt_record(ciphertext, &context).await?;
    validate(&row, &plain)
}
pub(crate) async fn rewrap(
    db: &Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    if verify(keys, source).await.is_ok() {
        return Ok(false);
    }
    let row = decode(source)?;
    let path = context_path(&row);
    let context = RecordContext::new(ARTIFACTS, &row.id, &row.user_id, &path)?;
    let plain = decrypt(keys, &row).await?;
    let ciphertext = keys.encrypt_record(&plain, &context).await?;
    keys.authenticate_current_record(&ciphertext, &context)
        .await?;
    Ok(db.collection::<Document>(ARTIFACTS)
        .update_one(super::super::record_storage::exact_document_predicate(source),
            doc! {"$set":{"data_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext},"secret_context_versions":{"data_encrypted":3}}})
        .await?.matched_count == 1)
}
