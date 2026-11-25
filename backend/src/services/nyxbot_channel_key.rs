//! The installed gateway key belongs to the linking person, including on an
//! organization route. A pending key ID authorizes setup and has no stored
//! ciphertext until adoption publishes it as the installed key.
use bson::{Binary, Document, doc, spec::BinarySubtype};
use mongodb::Database;
use zeroize::Zeroizing;

use crate::{
    crypto::{
        aes::EncryptionKeys,
        record_field::{self, RecordContext},
        token::hash_token,
    },
    errors::{AppError, AppResult},
    models::{
        api_key,
        nyxbot_channel::{COLLECTION_NAME, NyxbotChannel},
    },
};

pub(crate) const FIELD: &str = "agent_key_ciphertext";
pub(crate) const VERSION_PATH: &str = "secret_context_versions.agent_key_ciphertext";

fn unavailable() -> AppError {
    AppError::Internal("Channel key unavailable".into())
}

pub(crate) async fn seal(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    key_id: &str,
    secret: &str,
) -> AppResult<Vec<u8>> {
    record_field::encrypt_context(
        keys,
        &RecordContext::new(COLLECTION_NAME, id, owner, &[FIELD, key_id])?,
        secret.as_bytes(),
    )
    .await
}

fn installed_key(row: &NyxbotChannel) -> AppResult<&str> {
    row.agent_api_key_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(unavailable)
}

pub(crate) async fn open(
    keys: &EncryptionKeys,
    row: &NyxbotChannel,
) -> AppResult<Option<Zeroizing<String>>> {
    let marker = row.secret_context_versions.get(FIELD).copied();
    let Some(bytes) = row.agent_key_ciphertext.as_deref() else {
        return if marker.is_none() {
            Ok(None)
        } else {
            Err(unavailable())
        };
    };
    let path = [FIELD, installed_key(row)?];
    let context = RecordContext::new(COLLECTION_NAME, &row.id, &row.user_id, &path)?;
    let secret = record_field::decrypt_context(keys, &context, marker, bytes).await?;
    Ok(Some(
        crate::crypto::guarded_utf8(secret).map_err(|_| unavailable())?,
    ))
}

fn snapshot(source: &Document) -> AppResult<NyxbotChannel> {
    let row: NyxbotChannel = bson::from_document(source.clone()).map_err(|_| unavailable())?;
    if row
        .secret_context_versions
        .iter()
        .any(|(name, v)| name != FIELD || *v != 3 || row.agent_key_ciphertext.is_none())
        || (row.agent_key_ciphertext.is_none()
            && row.transport == "gateway"
            && matches!(row.status.as_str(), "active" | "pending"))
    {
        return Err(unavailable());
    }
    Ok(row)
}

// Legacy envelopes authenticate plaintext without its channel scope. Before
// certifying that scope, compare the installed key's independently stored hash.
// Revocation retains the digest and does not prevent a storage-only check.
async fn checked_key(
    db: &Database,
    keys: &EncryptionKeys,
    row: &NyxbotChannel,
) -> AppResult<Zeroizing<String>> {
    let secret = open(keys, row).await?.ok_or_else(unavailable)?;
    if secret.is_empty() || db.collection::<Document>(api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":installed_key(row)?,"user_id":&row.user_id,"key_hash":hash_token(&secret)})
        .projection(doc! {"_id":1}).await?.is_none() {
        return Err(unavailable());
    }
    Ok(secret)
}

async fn authenticate_primary(
    keys: &EncryptionKeys,
    row: &NyxbotChannel,
    bytes: &[u8],
) -> AppResult<()> {
    keys.authenticate_current_record(
        bytes,
        &RecordContext::new(
            COLLECTION_NAME,
            &row.id,
            &row.user_id,
            &[FIELD, installed_key(row)?],
        )?,
    )
    .await
}

pub(crate) async fn verify(
    db: &Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<()> {
    let row = snapshot(source)?;
    if let Some(bytes) = row.agent_key_ciphertext.as_deref() {
        if row.secret_context_versions.get(FIELD) != Some(&3) {
            return Err(unavailable());
        }
        authenticate_primary(keys, &row, bytes).await?;
        checked_key(db, keys, &row).await?;
    }
    Ok(())
}

pub(crate) async fn migrate(
    db: &Database,
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let row = snapshot(source)?;
    let Some(bytes) = row.agent_key_ciphertext.as_deref() else {
        return Ok(false);
    };
    let secret = checked_key(db, keys, &row).await?;
    if row.secret_context_versions.get(FIELD) == Some(&3)
        && authenticate_primary(keys, &row, bytes).await.is_ok()
    {
        return Ok(false);
    }
    let sealed = seal(keys, &row.id, &row.user_id, installed_key(&row)?, &secret).await?;
    authenticate_primary(keys, &row, &sealed).await?;
    let result = super::transaction_service::durable_database(db).collection::<Document>(COLLECTION_NAME)
        .update_one(super::record_storage::exact_document_predicate(source),
            doc! {"$set":{FIELD:Binary{subtype:BinarySubtype::Generic, bytes:sealed},VERSION_PATH:3}}).await?;
    Ok(result.matched_count == 1)
}
