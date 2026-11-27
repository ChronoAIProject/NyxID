//! The two retained CMA bodies have independent row contexts and the same
//! exact-wire hash contract. Storage operations never enter CMA admission.
use bson::{Binary, Bson, Document, doc, spec::BinarySubtype};
use mongodb::Database;
use zeroize::Zeroizing;

use super::{
    assistant_cma_sandbox_service::prepare_creation_request, record_field_service, record_storage,
    transaction_service,
};
use crate::{
    crypto::{
        aes::EncryptionKeys,
        record_field::{self, RecordContext},
    },
    errors::{AppError, AppResult},
    models::{assistant_cma_creation, assistant_cma_sandbox},
};

const FIELD: &str = "request_ciphertext";
const VERSION: &str = "request_context_version";

fn unavailable() -> AppError {
    AppError::Conflict("cma_creation_recovery_unavailable".into())
}

pub(super) async fn open(
    keys: &EncryptionKeys,
    context: &RecordContext<'_>,
    version: Option<i32>,
    ciphertext: Option<&[u8]>,
    hash: Option<&str>,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let version = match version {
        None => None,
        Some(3) => Some(3),
        Some(_) => return Err(unavailable()),
    };
    let body =
        record_field::decrypt_context(keys, context, version, ciphertext.ok_or_else(unavailable)?)
            .await?;
    if Some(prepare_creation_request(&body)?.hash.as_str()) != hash {
        return Err(unavailable());
    }
    Ok(body)
}

pub(super) struct Request<'a> {
    pub(super) context: RecordContext<'a>,
    pub(super) ciphertext: Vec<u8>,
    pub(super) version: Option<i32>,
    pub(super) hash: &'a str,
}

pub(super) fn snapshot<'a>(
    collection: &'a str,
    source: &'a Document,
) -> AppResult<Option<Request<'a>>> {
    let text = |key| source.get_str(key).map_err(|_| unavailable());
    let context = RecordContext::new(collection, text("_id")?, text("user_id")?, &[FIELD])?;
    let version = match source.get(VERSION) {
        None | Some(Bson::Null) => None,
        Some(Bson::Int32(3) | Bson::Int64(3)) => Some(3),
        _ => return Err(unavailable()),
    };
    let Some(value) = source.get(FIELD).filter(|v| **v != Bson::Null) else {
        let state = source.get_str("submission_state").ok();
        let cleared = match collection {
            assistant_cma_creation::COLLECTION_NAME => {
                matches!(state, Some("confirmed" | "confirmed_not_created"))
            }
            assistant_cma_sandbox::COLLECTION_NAME => {
                let managed_clear = source.get_bool("occupies_account_slot") == Ok(false)
                    && source.get_str("admission_state") == Ok("retired")
                    && matches!(state, Some("confirmed_retired" | "confirmed_not_created"));
                let external = matches!(
                    source.get_str("source"),
                    Ok("discovered" | "conflict" | "claimed")
                ) && ["request_hash", "idempotency_key", "operation_id"]
                    .iter()
                    .all(|key| source.get(*key).is_none_or(|v| *v == Bson::Null));
                managed_clear || external
            }
            _ => false,
        };
        return if version.is_none() && cleared {
            Ok(None)
        } else {
            Err(unavailable())
        };
    };
    Ok(Some(Request {
        context,
        version,
        ciphertext: record_field_service::stored_bytes(value)?,
        hash: text("request_hash")?,
    }))
}

pub(super) async fn verify(
    keys: &EncryptionKeys,
    collection: &str,
    source: &Document,
) -> AppResult<()> {
    if let Some(request) = snapshot(collection, source)? {
        if request.version != Some(3) {
            return Err(unavailable());
        }
        keys.authenticate_current_record(&request.ciphertext, &request.context)
            .await?;
        open(
            keys,
            &request.context,
            request.version,
            Some(&request.ciphertext),
            Some(request.hash),
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn migrate(
    db: &Database,
    keys: &EncryptionKeys,
    collection: &str,
    source: &Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let Some(request) = snapshot(collection, source)? else {
        return Ok(false);
    };
    let body = open(
        keys,
        &request.context,
        request.version,
        Some(&request.ciphertext),
        Some(request.hash),
    )
    .await?;
    if request.version == Some(3)
        && keys
            .authenticate_current_record(&request.ciphertext, &request.context)
            .await
            .is_ok()
    {
        return Ok(false);
    }
    let bytes = keys.encrypt_record(&body, &request.context).await?;
    keys.authenticate_current_record(&bytes, &request.context)
        .await?;
    let set = doc! {
        FIELD: Binary { subtype: BinarySubtype::Generic, bytes },
        VERSION: 3_i32,
    };
    Ok(transaction_service::durable_database(db)
        .collection::<Document>(collection)
        .update_one(
            record_storage::exact_document_predicate(source),
            doc! {"$set": set},
        )
        .await?
        .matched_count
        == 1)
}
