//! Service-owned adapter for bounded encryption verification and KEK rotation.
use super::*;

fn decode(source: &Document) -> AppResult<AssistantToolResult> {
    let row: AssistantToolResult = bson::from_document(source.clone()).map_err(|_| missing())?;
    if row.version != 1 || row.size <= 0 || row.size as usize > MAX_BYTES {
        return Err(missing());
    }
    Ok(row)
}
pub(crate) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    let row = decode(source)?;
    if row.encryption_version != Some(3) {
        return Err(missing());
    }
    let path = context_path(&row);
    keys.authenticate_current_record(
        &row.data_encrypted,
        &RecordContext::new(COLLECTION_NAME, &row.id, &row.user_id, &path)?,
    )
    .await
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
    let context = RecordContext::new(COLLECTION_NAME, &row.id, &row.user_id, &path)?;
    let plain = record_field_service::decrypt_context(
        keys,
        &context,
        row.encryption_version,
        &row.data_encrypted,
    )
    .await?;
    let text = std::str::from_utf8(&plain).map_err(|_| missing())?;
    if text.len() != row.size as usize || row.boundaries != boundaries(text) {
        return Err(missing());
    }
    let encrypted = keys.encrypt_record(&plain, &context).await?;
    keys.authenticate_current_record(&encrypted, &context)
        .await?;
    let db = super::super::transaction_service::durable_database(db);
    Ok(db.collection::<Document>(COLLECTION_NAME).update_one(super::super::record_storage::exact_document_predicate(source),doc!{"$set":{"data_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},"encryption_version":3}}).await?.matched_count==1)
}
