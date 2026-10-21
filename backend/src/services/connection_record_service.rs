//! Per-user legacy service credentials bind the service relationship as well as
//! the captured record/owner. The envelope never supplies trusted identities.
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::user_service_connection::{COLLECTION_NAME, UserServiceConnection},
};
use bson::{Document, doc};
use zeroize::Zeroizing;

const FIELD: &str = "credential_encrypted";
fn invalid() -> AppError {
    AppError::Internal("Invalid service connection credential storage".into())
}
pub(crate) fn validate(row: &UserServiceConnection) -> AppResult<()> {
    if row.id.is_empty()
        || row.user_id.is_empty()
        || row.service_id.is_empty()
        || row
            .secret_context_versions
            .iter()
            .any(|(field, version)| field != FIELD || *version != 3)
        || (row.credential_encrypted.is_none() && !row.secret_context_versions.is_empty())
    {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) async fn encrypt(
    keys: &EncryptionKeys,
    row: &mut UserServiceConnection,
    plaintext: &[u8],
) -> AppResult<()> {
    if plaintext.is_empty() || plaintext.len() > 8192 {
        return Err(invalid());
    }
    let ciphertext = super::record_field_service::encrypt_context(
        keys,
        &RecordContext::new(
            COLLECTION_NAME,
            &row.id,
            &row.user_id,
            &[FIELD, "service_id", &row.service_id],
        )?,
        plaintext,
    )
    .await?;
    super::credential_record_service::mark_version(&mut row.secret_context_versions, keys, FIELD);
    row.credential_encrypted = Some(ciphertext);
    Ok(())
}
pub(crate) async fn text(
    keys: &EncryptionKeys,
    row: &UserServiceConnection,
) -> AppResult<Zeroizing<String>> {
    validate(row)?;
    let raw = super::record_field_service::decrypt_context(
        keys,
        &RecordContext::new(
            COLLECTION_NAME,
            &row.id,
            &row.user_id,
            &[FIELD, "service_id", &row.service_id],
        )?,
        row.secret_context_versions.get(FIELD).copied(),
        row.credential_encrypted.as_deref().ok_or_else(invalid)?,
    )
    .await?;
    let value = std::str::from_utf8(&raw).map_err(|_| invalid())?;
    if value.is_empty() || value.len() > 8192 {
        return Err(invalid());
    }
    Ok(Zeroizing::new(value.to_owned()))
}
pub(crate) async fn verify_current_snapshot(
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<()> {
    let row: UserServiceConnection = bson::from_document(source.clone()).map_err(|_| invalid())?;
    validate(&row)?;
    if let Some(ciphertext) = row.credential_encrypted.as_deref() {
        if row.secret_context_versions.get(FIELD) != Some(&3) {
            return Err(invalid());
        }
        keys.authenticate_current_record(
            ciphertext,
            &RecordContext::new(
                COLLECTION_NAME,
                &row.id,
                &row.user_id,
                &[FIELD, "service_id", &row.service_id],
            )?,
        )
        .await?;
        text(keys, &row).await?;
    }
    Ok(())
}

/// Commit the exact captured mutation behind the persistent owner barrier.
/// Unknown commit outcomes are returned, never replayed as fresh operations.
pub(crate) async fn commit(
    db: &mongodb::Database,
    source: Option<&Document>,
    row: &UserServiceConnection,
) -> AppResult<()> {
    validate(row)?;
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        super::erasure_admission::credential_change_in_session(db, &row.user_id, &mut tx).await?;
        let current = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"user_id": &row.user_id,"service_id": &row.service_id})
            .session(&mut tx)
            .await?;
        if current.as_ref() != source {
            return Err(AppError::Conflict(
                "Service connection changed before credential commit".into(),
            ));
        }
        if let Some(source) = source {
            let mut set = bson::to_document(row).map_err(|_| invalid())?;
            set.remove("_id");
            // Option fields with skip_serializing_if still have explicit clear
            // semantics. Preserve unrelated additive fields while clearing the
            // old management metadata in the same exact-source mutation.
            if row.credential_type.is_none() {
                set.insert("credential_type", bson::Bson::Null);
            }
            if row.credential_label.is_none() {
                set.insert("credential_label", bson::Bson::Null);
            }
            let result = db
                .collection::<Document>(COLLECTION_NAME)
                .update_one(
                    super::record_storage::exact_document_predicate(source),
                    doc! {"$set":set},
                )
                .session(&mut tx)
                .await?;
            if result.matched_count != 1 {
                return Err(AppError::Conflict(
                    "Service connection changed before credential commit".into(),
                ));
            }
        } else {
            db.collection::<UserServiceConnection>(COLLECTION_NAME)
                .insert_one(row)
                .session(&mut tx)
                .await?;
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => {
            super::transaction_service::commit(&mut tx).await?;
            Ok(())
        }
        Err(error) => {
            let _ = tx.abort_transaction().await;
            Err(error)
        }
    }
}

pub(crate) async fn migrate_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let mut row: UserServiceConnection =
        bson::from_document(source.clone()).map_err(|_| invalid())?;
    validate(&row)?;
    if verify_current_snapshot(keys, source).await.is_ok() {
        return Ok(false);
    }
    let plaintext = text(keys, &row).await?;
    encrypt(keys, &mut row, plaintext.as_bytes()).await?;
    verify_current_snapshot(keys, &bson::to_document(&row).map_err(|_| invalid())?).await?;
    match commit(db, Some(source), &row).await {
        Ok(()) => Ok(true),
        Err(AppError::Conflict(_)) => Ok(false),
        Err(error) => Err(error),
    }
}
