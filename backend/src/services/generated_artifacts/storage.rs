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
        || row.secret_context_versions.len() != 1
        || row.secret_context_versions.get("data_encrypted") != Some(&3)
    {
        return Err(invalid());
    }
    Ok(())
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
    if verify(keys, source).await.is_ok() {
        return Ok(false);
    }
    let row = decode(source)?;
    let path = context_path(&row);
    let context = RecordContext::new(ARTIFACTS, &row.id, &row.user_id, &path)?;
    let plain = keys
        .decrypt_record(row.data_encrypted.as_deref().ok_or_else(invalid)?, &context)
        .await?;
    validate(&row, &plain)?;
    let ciphertext = keys.encrypt_record(&plain, &context).await?;
    keys.authenticate_current_record(&ciphertext, &context)
        .await?;
    Ok(db.collection::<Document>(ARTIFACTS)
        .update_one(super::super::record_storage::exact_document_predicate(source),
            doc! {"$set":{"data_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext}}})
        .await?.matched_count == 1)
}
