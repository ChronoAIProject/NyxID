//! Record-bound storage of the native and relay gateway replay payload.
use super::{GatewayResponse, Payload, RESPONSES, decode, unavailable};
use crate::{
    crypto::{
        aes::EncryptionKeys,
        record_field::{self, RecordContext},
    },
    errors::{AppError, AppResult},
    services::{record_storage, transaction_service},
};
use bson::{Document, doc};
use mongodb::Database;
use zeroize::Zeroizing;

const FIELD: &str = "payload_ciphertext";
const VERSION: &str = "payload_context_version";

fn field(row: &GatewayResponse) -> [&str; 4] {
    [FIELD, &row.channel_id, &row.binding_id, &row.response_id]
}

pub(super) async fn open(
    keys: &EncryptionKeys,
    row: &GatewayResponse,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let parts = field(row);
    let context = RecordContext::new(RESPONSES, &row.id, &row.user_id, &parts)?;
    record_field::decrypt_context(
        keys,
        &context,
        row.payload_context_version,
        &row.payload_ciphertext,
    )
    .await
}

async fn seal(keys: &EncryptionKeys, row: &GatewayResponse, body: &Payload) -> AppResult<Vec<u8>> {
    let parts = field(row);
    let context = RecordContext::new(RESPONSES, &row.id, &row.user_id, &parts)?;
    record_field::encrypt_context(keys, &context, &body.encoded()?).await
}

pub(crate) async fn install(
    keys: &EncryptionKeys,
    row: &mut GatewayResponse,
    body: &Payload,
) -> AppResult<()> {
    row.payload_ciphertext = seal(keys, row, body).await?;
    row.payload_context_version = keys.contextual_record_writes().then_some(3);
    Ok(())
}

pub(super) async fn revision_update(
    keys: &EncryptionKeys,
    row: &GatewayResponse,
    body: &Payload,
    mut set: Document,
) -> AppResult<Document> {
    set.insert(
        FIELD,
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: seal(keys, row, body).await?,
        },
    );
    let mut update = doc! {"$inc": {"revision": 1_i64}};
    if keys.contextual_record_writes() {
        set.insert(VERSION, 3_i32);
    } else {
        update.insert("$unset", doc! {VERSION: ""});
    }
    update.insert("$set", set);
    Ok(update)
}

fn checked(row: &GatewayResponse, bytes: &[u8]) -> AppResult<()> {
    let body = decode(bytes)?;
    let identity = super::conversation_identity(&row.channel_id, &row.binding_id, &row.partition);
    let status = body.snapshot["status"].as_str();
    if body.snapshot["id"] != row.response_id
        || body.snapshot["conversation"]["id"] != identity
        || !matches!(
            (row.terminal, status),
            (true, Some("completed" | "failed")) | (false, Some("queued" | "in_progress"))
        )
        || body.frames.last().and_then(|f| f.get("response")) != Some(&body.snapshot)
        || body.frames.iter().enumerate().any(|(i, f)| {
            f["sequence_number"].as_u64() != Some(i as u64)
                || f.get("response").is_some_and(|s| {
                    s["id"] != row.response_id
                        || s["conversation"] != body.snapshot["conversation"]
                        || s["metadata"] != body.snapshot["metadata"]
                })
        })
    {
        return Err(unavailable());
    }
    Ok(())
}

pub(crate) async fn verify(keys: &EncryptionKeys, source: &Document) -> AppResult<()> {
    let row: GatewayResponse = bson::from_document(source.clone()).map_err(|_| unavailable())?;
    let parts = field(&row);
    let context = RecordContext::new(RESPONSES, &row.id, &row.user_id, &parts)?;
    if row.payload_context_version != Some(3) {
        return Err(unavailable());
    }
    keys.authenticate_current_record(&row.payload_ciphertext, &context)
        .await?;
    checked(&row, &open(keys, &row).await?)
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
    let row: GatewayResponse = bson::from_document(source.clone()).map_err(|_| unavailable())?;
    let parts = field(&row);
    let context = RecordContext::new(RESPONSES, &row.id, &row.user_id, &parts)?;
    let bytes = open(keys, &row).await?;
    checked(&row, &bytes)?;
    if row.payload_context_version == Some(3)
        && keys
            .authenticate_current_record(&row.payload_ciphertext, &context)
            .await
            .is_ok()
    {
        return Ok(false);
    }
    let sealed = keys.encrypt_record(&bytes, &context).await?;
    keys.authenticate_current_record(&sealed, &context).await?;
    let update = doc! {"$set": {
        FIELD: bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:sealed},
        VERSION: 3_i32,
    }};
    // A retained payload can reach 15 MiB. Put the old and new ciphertext in
    // separate commands so the update stays within the BSON document limit.
    let args = (
        db.collection::<Document>(RESPONSES),
        row.id,
        record_storage::exact_document_predicate(source),
        update,
    );
    transaction_service::run_db_only(db, &args, |tx, (rows, id, source, update)| {
        Box::pin(async move {
            if rows
                .find_one(source.clone())
                .projection(doc! {"_id":1})
                .session(&mut *tx)
                .await?
                .is_none()
            {
                return Ok(false);
            }
            Ok(rows
                .update_one(doc! {"_id":id}, update.clone())
                .session(tx)
                .await?
                .matched_count
                == 1)
        })
    })
    .await
}
