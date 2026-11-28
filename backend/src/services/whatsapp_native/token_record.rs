//! A retained native configuration belongs to the owner and bot generation
//! that enrolled it. Serving its token also requires the current parent.
use super::*;
use crate::crypto::record_field::{self, RecordContext};
use crate::services::{record_storage, transaction_service};
use bson::{Binary, Document, spec::BinarySubtype};

pub(crate) const FIELD: &str = "native_verify_token_encrypted";

fn unavailable() -> AppError {
    refusal("native_whatsapp_verify_token_unavailable")
}

pub(in crate::services) fn version(row: &NativeWhatsapp) -> AppResult<Option<u8>> {
    if row.secret_context_versions.is_empty() {
        return Ok(None);
    }
    if row.secret_context_versions.len() == 1
        && row.secret_context_versions.get(FIELD) == Some(&3)
        && row.native_verify_token_encrypted.is_some()
    {
        return Ok(Some(3));
    }
    Err(unavailable())
}

fn context<'a>(row: &'a NativeWhatsapp, path: &'a [&'a str]) -> AppResult<RecordContext<'a>> {
    RecordContext::new(COLLECTION_NAME, &row.bot_id, &row.owner_id, path)
}

async fn open(keys: &EncryptionKeys, row: &NativeWhatsapp) -> AppResult<Zeroizing<String>> {
    let generation = row.ownership_version.to_string();
    let bytes = record_field::decrypt_context(
        keys,
        &context(row, &[FIELD, &generation])?,
        version(row)?,
        row.native_verify_token_encrypted
            .as_deref()
            .ok_or_else(unavailable)?,
    )
    .await?;
    let token = crate::crypto::guarded_utf8(bytes).map_err(|_| unavailable())?;
    if !(16..=256).contains(&token.len()) || !token.bytes().all(|b| (33..=126).contains(&b)) {
        return Err(unavailable());
    }
    Ok(token)
}

async fn seal(keys: &EncryptionKeys, row: &NativeWhatsapp, token: &str) -> AppResult<Binary> {
    let generation = row.ownership_version.to_string();
    let bytes = record_field::encrypt_context(
        keys,
        &context(row, &[FIELD, &generation])?,
        token.as_bytes(),
    )
    .await?;
    Ok(Binary {
        subtype: BinarySubtype::Generic,
        bytes,
    })
}

/// Resolve both the config and the live parent in one snapshot. Only an
/// installation writes the parent, serializing that publication with transfer.
pub async fn native_verify_token(
    db: &Database,
    keys: &EncryptionKeys,
    config: &NativeWhatsapp,
) -> AppResult<Zeroizing<String>> {
    let proposal = if config.native_verify_token_encrypted.is_none() {
        let generated = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
        Some(
            seal(keys, config, &generated)
                .await
                .map_err(|_| unavailable())?,
        )
    } else {
        None
    };
    let input = (db, config, proposal, keys.contextual_record_writes());
    let winner = transaction_service::run_db_only(
        db,
        &input,
        |session, (db, expected, proposal, contextual)| {
            Box::pin(async move {
                let rows = db.collection::<NativeWhatsapp>(COLLECTION_NAME);
                let id = doc! {"_id": &expected.bot_id};
                let mut current = rows
                    .find_one(id.clone())
                    .session(&mut *session)
                    .await?
                    .ok_or_else(unavailable)?;
                let parent = if current.native_verify_token_encrypted.is_none() {
                    fence_enrollment(db, session, &current.bot_id).await?
                } else {
                    db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
                        .find_one(id.clone())
                        .session(&mut *session)
                        .await?
                        .ok_or_else(unavailable)?
                };
                if !matches_bot(&current, &parent)
                    || !matches_bot(expected, &parent)
                    || expected.revision != current.revision
                    || expected.callback.phase != current.callback.phase
                    || expected.meta_app_id != current.meta_app_id
                    || expected.waba_id != current.waba_id
                {
                    return Err(unavailable());
                }
                version(&current)?;
                if current.native_verify_token_encrypted.is_none() {
                    let proposed = proposal.as_ref().ok_or_else(unavailable)?;
                    let mut set = doc! {FIELD:proposed.clone(), "updated_at":bson::DateTime::now()};
                    if *contextual {
                        set.insert("secret_context_versions", doc! {FIELD:3});
                        current.secret_context_versions.insert(FIELD.into(), 3);
                    }
                    // Read and write the same document in this snapshot transaction.
                    // A concurrent initializer, callback or reconfiguration conflicts.
                    rows.update_one(id, doc! {"$set":set})
                        .session(&mut *session)
                        .await?;
                    current.native_verify_token_encrypted = Some(proposed.bytes.clone());
                }
                Ok(current)
            })
        },
    )
    .await?;
    open(keys, &winner).await.map_err(|_| unavailable())
}

fn snapshot(source: &Document) -> AppResult<NativeWhatsapp> {
    bson::from_document(source.clone()).map_err(|_| unavailable())
}

pub(crate) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    let row = snapshot(source)?;
    let version = version(&row)?;
    if let Some(bytes) = row.native_verify_token_encrypted.as_deref() {
        if version != Some(3) {
            return Err(unavailable());
        }
        let generation = row.ownership_version.to_string();
        keys.authenticate_current_record(bytes, &context(&row, &[FIELD, &generation])?)
            .await?;
        open(keys, &row).await?;
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
    version(&row)?;
    if row.native_verify_token_encrypted.is_none() || verify(keys, source).await.is_ok() {
        return Ok(false);
    }
    let token = open(keys, &row).await?;
    let sealed = seal(keys, &row, &token).await?;
    let generation = row.ownership_version.to_string();
    keys.authenticate_current_record(&sealed.bytes, &context(&row, &[FIELD, &generation])?)
        .await?;
    Ok(transaction_service::durable_database(db)
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            record_storage::exact_document_predicate(source),
            doc! {"$set":{FIELD:sealed,"secret_context_versions":{FIELD:3}}},
        )
        .await?
        .matched_count
        == 1)
}
