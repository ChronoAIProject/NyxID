//! Trigger secrets and retained delivery envelopes use explicit field versions.
//! Delivery contexts include the trusted parent trigger and event identities;
//! ciphertext metadata never chooses the destination or the decoder.
use crate::{
    crypto::{
        aes::{EncryptionKeys, RecordContext},
        secret_json::SecretJson,
    },
    errors::{AppError, AppResult},
    models::{
        trigger::{COLLECTION_NAME as TRIGGERS, Trigger, TriggerDelivery, TriggerVerification},
        trigger_delivery::{COLLECTION_NAME as DELIVERIES, TriggerDeliveryRecord},
    },
};
use bson::{Document, doc};
use zeroize::Zeroizing;

const TRIGGER_FIELDS: &[&str] = &["verification_secret_encrypted", "delivery_secret_encrypted"];
const ENVELOPE: &str = "envelope_encrypted";
fn invalid() -> AppError {
    AppError::Internal("Invalid encrypted trigger material".into())
}

pub(crate) async fn encrypt_secret(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    field: &str,
    plain: &[u8],
) -> AppResult<Vec<u8>> {
    if !TRIGGER_FIELDS.contains(&field) || plain.is_empty() || plain.len() > 8192 {
        return Err(invalid());
    }
    super::record_field_service::encrypt(keys, TRIGGERS, id, owner, field, plain).await
}
pub(crate) async fn secret(
    keys: &EncryptionKeys,
    row: &Trigger,
    field: &str,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let bytes = match field {
        "verification_secret_encrypted" => row.verification_secret_encrypted.as_deref(),
        "delivery_secret_encrypted" => row.delivery_secret_encrypted.as_deref(),
        _ => return Err(invalid()),
    }
    .ok_or_else(invalid)?;
    let plain = super::record_field_service::decrypt(
        keys,
        TRIGGERS,
        &row.id,
        &row.user_id,
        field,
        row.secret_context_versions.get(field).copied(),
        bytes,
    )
    .await?;
    if plain.is_empty() || plain.len() > 8192 {
        return Err(invalid());
    }
    Ok(plain)
}
pub(crate) async fn encrypt_envelope(
    keys: &EncryptionKeys,
    row: &TriggerDeliveryRecord,
    plain: &[u8],
) -> AppResult<Vec<u8>> {
    validate_envelope(row, plain)?;
    super::record_field_service::encrypt_context(
        keys,
        &RecordContext::new(
            DELIVERIES,
            &row.id,
            &row.user_id,
            &[
                ENVELOPE,
                "trigger_id",
                &row.trigger_id,
                "event_id",
                &row.event_id,
            ],
        )?,
        plain,
    )
    .await
}
fn validate_envelope(row: &TriggerDeliveryRecord, plain: &[u8]) -> AppResult<()> {
    let body: SecretJson = serde_json::from_slice(plain).map_err(|_| invalid())?;
    if body.get("trigger_id").and_then(serde_json::Value::as_str) != Some(row.trigger_id.as_str())
        || body.get("event_id").and_then(serde_json::Value::as_str) != Some(row.event_id.as_str())
        || !body.is_object()
        || body.get("payload").is_none()
    {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) async fn envelope(
    keys: &EncryptionKeys,
    row: &TriggerDeliveryRecord,
) -> AppResult<Zeroizing<Vec<u8>>> {
    if row
        .secret_context_versions
        .iter()
        .any(|(field, version)| field != ENVELOPE || *version != 3)
    {
        return Err(invalid());
    }
    let Some(ciphertext) = row.envelope_encrypted.as_deref() else {
        // Zero retention deliberately keeps only delivery metadata. A dangling
        // marker still denotes invalid storage and cannot masquerade as expiry.
        return Err(if row.secret_context_versions.is_empty() {
            AppError::TriggerDeliveryRecordNotFound
        } else {
            invalid()
        });
    };
    let plain = super::record_field_service::decrypt_context(
        keys,
        &RecordContext::new(
            DELIVERIES,
            &row.id,
            &row.user_id,
            &[
                ENVELOPE,
                "trigger_id",
                &row.trigger_id,
                "event_id",
                &row.event_id,
            ],
        )?,
        row.secret_context_versions.get(ENVELOPE).copied(),
        ciphertext,
    )
    .await?;
    validate_envelope(row, &plain)?;
    Ok(plain)
}
fn validate_versions(
    source: &Document,
    fields: &[&str],
) -> AppResult<std::collections::BTreeMap<String, u8>> {
    let versions = super::record_field_service::stored_versions(source)?;
    if versions.iter().any(|(field, version)| {
        *version != 3
            || !fields.contains(&field.as_str())
            || source
                .get(field)
                .is_none_or(|value| value == &bson::Bson::Null)
    }) {
        return Err(invalid());
    }
    Ok(versions)
}
pub(crate) async fn verify_current_snapshot(
    keys: &EncryptionKeys,
    collection: &str,
    source: &Document,
) -> AppResult<()> {
    if collection == TRIGGERS {
        let row: Trigger = bson::from_document(source.clone()).map_err(|_| invalid())?;
        validate_versions(source, TRIGGER_FIELDS)?;
        if (matches!(row.verification, TriggerVerification::HmacSha256 { .. })
            && row.verification_secret_encrypted.is_none())
            || (matches!(row.delivery, TriggerDelivery::Webhook { .. })
                && row.delivery_secret_encrypted.is_none())
        {
            return Err(invalid());
        }
        for (field, bytes) in [
            (
                TRIGGER_FIELDS[0],
                row.verification_secret_encrypted.as_deref(),
            ),
            (TRIGGER_FIELDS[1], row.delivery_secret_encrypted.as_deref()),
        ] {
            if let Some(bytes) = bytes {
                super::record_field_service::verify_current(
                    keys,
                    TRIGGERS,
                    &row.id,
                    &row.user_id,
                    field,
                    row.secret_context_versions.get(field).copied(),
                    bytes,
                )
                .await?;
                let plain = secret(keys, &row, field).await?;
                if field == TRIGGER_FIELDS[0]
                    && crate::crypto::token::hash_token(
                        std::str::from_utf8(&plain).map_err(|_| invalid())?,
                    ) != row.secret_hash
                {
                    return Err(invalid());
                }
            }
        }
    } else if collection == DELIVERIES {
        let row: TriggerDeliveryRecord =
            bson::from_document(source.clone()).map_err(|_| invalid())?;
        validate_versions(source, &[ENVELOPE])?;
        // Metadata-only retention deliberately has no encrypted envelope.
        if let Some(bytes) = row.envelope_encrypted.as_deref() {
            if row.secret_context_versions.get(ENVELOPE) != Some(&3) {
                return Err(invalid());
            }
            keys.authenticate_current_record(
                bytes,
                &RecordContext::new(
                    DELIVERIES,
                    &row.id,
                    &row.user_id,
                    &[
                        ENVELOPE,
                        "trigger_id",
                        &row.trigger_id,
                        "event_id",
                        &row.event_id,
                    ],
                )?,
            )
            .await?;
            envelope(keys, &row).await?;
        }
    } else {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) async fn migrate_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    collection: &str,
    source: &Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    if verify_current_snapshot(keys, collection, source)
        .await
        .is_ok()
    {
        return Ok(false);
    }
    let mut set = Document::new();
    let mut versions = if collection == TRIGGERS {
        validate_versions(source, TRIGGER_FIELDS)?
    } else if collection == DELIVERIES {
        validate_versions(source, &[ENVELOPE])?
    } else {
        return Err(invalid());
    };
    if collection == TRIGGERS {
        let row: Trigger = bson::from_document(source.clone()).map_err(|_| invalid())?;
        if (matches!(row.verification, TriggerVerification::HmacSha256 { .. })
            && row.verification_secret_encrypted.is_none())
            || (matches!(row.delivery, TriggerDelivery::Webhook { .. })
                && row.delivery_secret_encrypted.is_none())
        {
            return Err(invalid());
        }
        // Validate domain meaning before the common storage-only rewrap.
        // The common helper authenticates each candidate and compares the
        // complete raw source; it preserves every validated plaintext byte.
        for (field, bytes) in [
            (
                TRIGGER_FIELDS[0],
                row.verification_secret_encrypted.as_deref(),
            ),
            (TRIGGER_FIELDS[1], row.delivery_secret_encrypted.as_deref()),
        ] {
            if bytes.is_some() {
                let plain = secret(keys, &row, field).await?;
                if field == TRIGGER_FIELDS[0]
                    && crate::crypto::token::hash_token(
                        std::str::from_utf8(&plain).map_err(|_| invalid())?,
                    ) != row.secret_hash
                {
                    return Err(invalid());
                }
            }
        }
        return super::record_field_service::migrate_snapshot(
            db,
            keys,
            TRIGGERS,
            source,
            &row.user_id,
            TRIGGER_FIELDS,
        )
        .await;
    } else {
        let row: TriggerDeliveryRecord =
            bson::from_document(source.clone()).map_err(|_| invalid())?;
        if row.envelope_encrypted.is_some() {
            let plain = envelope(keys, &row).await?;
            let encrypted = encrypt_envelope(keys, &row, &plain).await?;
            versions.insert(ENVELOPE.to_owned(), 3);
            set.insert(
                ENVELOPE,
                bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: encrypted,
                },
            );
        }
    }
    set.insert(
        "secret_context_versions",
        bson::to_bson(&versions).map_err(|_| invalid())?,
    );
    let mut candidate = source.clone();
    candidate.extend(set.clone());
    verify_current_snapshot(keys, collection, &candidate).await?;
    Ok(super::transaction_service::durable_database(db)
        .collection::<Document>(collection)
        .update_one(
            super::record_storage::exact_document_predicate(source),
            doc! {"$set":set},
        )
        .await?
        .matched_count
        == 1)
}
