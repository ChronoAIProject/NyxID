//! Storage for the opaque reference installed by gateway activity on a chat.
use crate::{
    crypto::{
        aes::EncryptionKeys,
        guarded_utf8,
        record_field::{self, RecordContext},
    },
    errors::{AppError, AppResult},
    models::nyxbot_channel::{NyxbotThread, THREADS_COLLECTION_NAME as THREADS},
};
use bson::{Bson, Document, doc};
use chrono::{Duration, Utc};
use mongodb::Database;
use zeroize::Zeroizing;

use super::{channel_supplemental_delivery as delivery, record_storage, transaction_service};

const FIELD: &str = "event_ref_ciphertext";
const VERSION: &str = "event_ref_context_version";

fn unavailable() -> AppError {
    AppError::Internal("Event reference unavailable".into())
}

fn context<'a>(row: &'a NyxbotThread, path: &'a [&'a str]) -> AppResult<RecordContext<'a>> {
    RecordContext::new(THREADS, &row.id, &row.user_id, path)
}

pub(crate) async fn open(
    keys: &EncryptionKeys,
    row: &NyxbotThread,
) -> AppResult<Zeroizing<String>> {
    let bytes = row
        .event_ref_ciphertext
        .as_deref()
        .ok_or_else(unavailable)?;
    let path = [FIELD, &row.channel_id];
    let plain = record_field::decrypt_context(
        keys,
        &context(row, &path)?,
        row.event_ref_context_version,
        bytes,
    )
    .await?;
    guarded_utf8(plain).map_err(|_| unavailable())
}

pub(crate) async fn record_activity(
    db: &Database,
    keys: &EncryptionKeys,
    row: &NyxbotThread,
    mut metadata: Document,
    reference: Option<&str>,
    relay: Option<&str>,
) -> AppResult<()> {
    let mut update = Document::new();
    if let Some(reference) = reference {
        let path = [FIELD, &row.channel_id];
        if let Ok(ctx) = context(row, &path)
            && let Ok(bytes) = record_field::encrypt_context(keys, &ctx, reference.as_bytes()).await
        {
            metadata.extend(delivery::reference_fields(
                bytes,
                Utc::now() + Duration::minutes(29),
                relay,
            ));
            if keys.contextual_record_writes() {
                metadata.insert(VERSION, 3_i32);
            } else {
                update.insert("$unset", doc! {VERSION: ""});
            }
        }
    }
    if !metadata.is_empty() {
        update.insert("$set", metadata);
        db.collection::<Document>(THREADS)
            .update_one(
                doc! {
                    "_id": &row.id,
                    "channel_id": &row.channel_id,
                    "user_id": &row.user_id,
                    "partition": &row.partition,
                },
                update,
            )
            .await?;
    }
    Ok(())
}

fn material(source: &Document) -> AppResult<Option<NyxbotThread>> {
    if matches!(source.get(FIELD), None | Some(Bson::Null)) {
        return if matches!(source.get(VERSION), None | Some(Bson::Null)) {
            Ok(None)
        } else {
            Err(unavailable())
        };
    }
    bson::from_document(source.clone())
        .map(Some)
        .map_err(|_| unavailable())
}

pub(crate) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    if let Some(row) = material(source)? {
        if row.event_ref_context_version != Some(3) {
            return Err(unavailable());
        }
        let path = [FIELD, &row.channel_id];
        keys.authenticate_current_record(
            row.event_ref_ciphertext
                .as_deref()
                .ok_or_else(unavailable)?,
            &context(&row, &path)?,
        )
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
    let Some(row) = material(source)? else {
        return Ok(false);
    };
    let plain = open(keys, &row).await?;
    let path = [FIELD, &row.channel_id];
    let ctx = context(&row, &path)?;
    if row.event_ref_context_version == Some(3)
        && keys
            .authenticate_current_record(
                row.event_ref_ciphertext
                    .as_deref()
                    .ok_or_else(unavailable)?,
                &ctx,
            )
            .await
            .is_ok()
    {
        return Ok(false);
    }
    let sealed = keys.encrypt_record(plain.as_bytes(), &ctx).await?;
    keys.authenticate_current_record(&sealed, &ctx).await?;
    let mut set = doc! {VERSION: 3_i32};
    if delivery::reference_anchor(&row).is_ok()
        && let Some(expires) = row.event_ref_expires_at
    {
        set.insert(
            "event_ref_relay_binding",
            delivery::reference_binding(&sealed, expires),
        );
    }
    set.insert(
        FIELD,
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: sealed,
        },
    );
    let result = transaction_service::durable_database(db)
        .collection::<Document>(THREADS)
        .update_one(
            record_storage::exact_document_predicate(source),
            doc! {"$set": set},
        )
        .await?;
    Ok(result.matched_count == 1)
}
