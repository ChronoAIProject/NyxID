//! Binary record fields with the historical unbound envelope contract. Services
//! resolve trusted identities after their existing ACL/lifecycle checks. Bound
//! legacy formats use their own adapters and must not enter this decoder.
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
};
use zeroize::Zeroizing;

pub(crate) async fn encrypt(
    keys: &EncryptionKeys,
    collection: &str,
    id: &str,
    owner: &str,
    field: &str,
    bytes: &[u8],
) -> AppResult<Vec<u8>> {
    encrypt_context(
        keys,
        &RecordContext::new(collection, id, owner, &[field])?,
        bytes,
    )
    .await
}

pub(crate) async fn encrypt_context(
    keys: &EncryptionKeys,
    context: &RecordContext<'_>,
    bytes: &[u8],
) -> AppResult<Vec<u8>> {
    if keys.contextual_record_writes() {
        keys.encrypt_record(bytes, context).await
    } else {
        keys.encrypt(bytes).await
    }
}

pub(crate) async fn decrypt(
    keys: &EncryptionKeys,
    collection: &str,
    id: &str,
    owner: &str,
    field: &str,
    version: Option<u8>,
    ciphertext: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    decrypt_context(
        keys,
        &RecordContext::new(collection, id, owner, &[field])?,
        version,
        ciphertext,
    )
    .await
}

pub(crate) async fn decrypt_context(
    keys: &EncryptionKeys,
    context: &RecordContext<'_>,
    version: Option<u8>,
    ciphertext: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    if version == Some(3) {
        keys.decrypt_record(ciphertext, context).await
    } else if version.is_none() && keys.allow_legacy_record_reads() {
        keys.decrypt(ciphertext).await.map(Zeroizing::new)
    } else {
        Err(AppError::Internal(
            "Record field requires contextual migration".into(),
        ))
    }
}

pub(crate) async fn verify_current(
    keys: &EncryptionKeys,
    collection: &str,
    id: &str,
    owner: &str,
    field: &str,
    version: Option<u8>,
    ciphertext: &[u8],
) -> AppResult<()> {
    if version != Some(3) {
        return Err(AppError::Internal(
            "Record field requires contextual migration".into(),
        ));
    }
    keys.authenticate_current_record(
        ciphertext,
        &RecordContext::new(collection, id, owner, &[field])?,
    )
    .await
}

/// Exact-source, majority-durable storage conversion for unbound binary fields.
/// No upsert, replacement or material-authority change is permitted. The caller
/// validates the decoded domain payload before invoking this storage mutation.
pub(crate) async fn migrate_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    collection: &str,
    source: &bson::Document,
    owner: &str,
    fields: &[&str],
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let invalid = || AppError::Internal("Invalid record field storage".into());
    let id = source.get_str("_id").map_err(|_| invalid())?;
    let mut set = bson::Document::new();
    let mut versions = stored_versions(source)?;
    if versions.iter().any(|(field, version)| {
        *version != 3
            || !fields.contains(&field.as_str())
            || source
                .get(field)
                .is_none_or(|value| value == &bson::Bson::Null)
    }) {
        return Err(invalid());
    }
    for field in fields {
        let Some(value) = source.get(*field) else {
            continue;
        };
        if value == &bson::Bson::Null {
            continue;
        }
        #[derive(serde::Deserialize)]
        struct Bytes {
            #[serde(with = "crate::models::bson_bytes::required")]
            bytes: Vec<u8>,
        }
        let bytes = bson::from_document::<Bytes>(bson::doc! {"bytes":value})
            .map_err(|_| invalid())?
            .bytes;
        if verify_current(
            keys,
            collection,
            id,
            owner,
            field,
            versions.get(*field).copied(),
            &bytes,
        )
        .await
        .is_ok()
        {
            continue;
        }
        let plain = decrypt(
            keys,
            collection,
            id,
            owner,
            field,
            versions.get(*field).copied(),
            &bytes,
        )
        .await?;
        let encrypted = encrypt(keys, collection, id, owner, field, &plain).await?;
        verify_current(keys, collection, id, owner, field, Some(3), &encrypted).await?;
        versions.insert((*field).to_owned(), 3);
        set.insert(
            *field,
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: encrypted,
            },
        );
    }
    if set.is_empty() {
        return Ok(false);
    }
    set.insert(
        "secret_context_versions",
        bson::to_bson(&versions).map_err(|_| invalid())?,
    );
    let filter = super::record_storage::exact_document_predicate(source);
    let db = super::transaction_service::durable_database(db);
    Ok(db
        .collection::<bson::Document>(collection)
        .update_one(filter, bson::doc! {"$set":set})
        .await?
        .matched_count
        == 1)
}

pub(crate) fn stored_versions(
    source: &bson::Document,
) -> AppResult<std::collections::BTreeMap<String, u8>> {
    match source.get("secret_context_versions") {
        None => Ok(Default::default()),
        Some(value) => bson::from_bson(value.clone())
            .map_err(|_| AppError::Internal("Invalid record field versions".into())),
    }
}

/// Accept historical array-backed bytes as well as current BSON Binary without
/// normalizing the raw source used by a migration's CAS predicate.
pub(crate) fn stored_bytes(value: &bson::Bson) -> AppResult<Vec<u8>> {
    #[derive(serde::Deserialize)]
    struct Bytes {
        #[serde(with = "crate::models::bson_bytes::required")]
        bytes: Vec<u8>,
    }
    bson::from_document::<Bytes>(bson::doc! {"bytes":value})
        .map(|row| row.bytes)
        .map_err(|_| AppError::Internal("Invalid record field storage".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct HoldFirstWrap {
        inner: crate::crypto::local_key_provider::LocalKeyProvider,
        held: std::sync::atomic::AtomicBool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    #[async_trait::async_trait]
    impl crate::crypto::key_provider::KeyProvider for HoldFirstWrap {
        async fn wrap_dek(&self, key: &[u8]) -> AppResult<crate::crypto::key_provider::WrappedKey> {
            if !self.held.swap(true, std::sync::atomic::Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            self.inner.wrap_dek(key).await
        }
        async fn unwrap_dek(
            &self,
            wrapped: &crate::crypto::key_provider::WrappedKey,
        ) -> AppResult<Zeroizing<Vec<u8>>> {
            self.inner.unwrap_dek(wrapped).await
        }
        fn current_key_id(&self) -> u8 {
            self.inner.current_key_id()
        }
        fn has_key_id(&self, id: u8) -> bool {
            self.inner.has_key_id(id)
        }
        fn has_previous_key(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn held_migration_preserves_formerly_absent_field_and_version_additions() {
        use bson::{Binary, doc, spec::BinarySubtype};
        let db = crate::test_utils::connect_test_database("record_field_absent_cas")
            .await
            .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        let collection = "record_field_cas_fixtures";
        let rows = db.collection::<bson::Document>(collection);
        // Exercise both an absent map and an existing map without field B.
        for existing_map in [false, true] {
            let id = uuid::Uuid::new_v4().to_string();
            let mut source = doc! {"_id":&id,"user_id":"owner",
            "field_a":Binary{subtype:BinarySubtype::Generic,
                bytes:keys.encrypt(b"legacy-a").await.unwrap()}};
            if existing_map {
                source.insert("secret_context_versions", doc! {});
            }
            rows.insert_one(&source).await.unwrap();
            let held = std::sync::Arc::new(HoldFirstWrap {
                inner: crate::crypto::local_key_provider::LocalKeyProvider::new([0x11; 32], None),
                held: Default::default(),
                entered: Default::default(),
                release: Default::default(),
            });
            let held_keys = EncryptionKeys::with_provider(held.clone());
            let migrate = migrate_snapshot(
                &db,
                &held_keys,
                collection,
                &source,
                "owner",
                &["field_a", "field_b"],
            );
            let writer = async {
                held.entered.notified().await;
                let ciphertext =
                    encrypt(&keys, collection, &id, "owner", "field_b", b"concurrent-b")
                        .await
                        .unwrap();
                rows.update_one(
                    doc! {"_id":&id},
                    doc! {"$set":{
                    "field_b":Binary{subtype:BinarySubtype::Generic,bytes:ciphertext},
                    "secret_context_versions.field_b":3_i32}},
                )
                .await
                .unwrap();
                let winner = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
                held.release.notify_one();
                winner
            };
            let (result, winner) =
                tokio::time::timeout(std::time::Duration::from_secs(20), async {
                    tokio::join!(migrate, writer)
                })
                .await
                .expect("held CAS must finish");
            assert!(
                !result.unwrap(),
                "stale migration must miss its exact source CAS"
            );
            let live = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
            assert_eq!(
                live, winner,
                "migration must preserve both newly added owners"
            );
            assert_eq!(stored_versions(&live).unwrap().get("field_b"), Some(&3));
            // Reloading the current source migrates A without losing B's marker.
            assert!(
                migrate_snapshot(
                    &db,
                    &keys,
                    collection,
                    &live,
                    "owner",
                    &["field_a", "field_b"]
                )
                .await
                .unwrap()
            );
            let current = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
            let versions = stored_versions(&current).unwrap();
            assert_eq!(versions.get("field_a"), Some(&3));
            assert_eq!(versions.get("field_b"), Some(&3));
            assert_eq!(current.get("field_b"), winner.get("field_b"));
            assert!(
                !migrate_snapshot(
                    &db,
                    &keys,
                    collection,
                    &current,
                    "owner",
                    &["field_a", "field_b"]
                )
                .await
                .unwrap()
            );
        }
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn explicit_dispatch_accepts_authenticated_v0_magic_nonce_and_never_downgrades_v3() {
        use aes_gcm::{
            Aes256Gcm, Nonce,
            aead::{Aead, KeyInit},
        };
        let mut keys = crate::test_utils::test_encryption_keys();
        let nonce = *b"\x03NYXnonce123";
        let cipher = Aes256Gcm::new_from_slice(&[0x11; 32]).unwrap();
        let mut legacy = nonce.to_vec();
        legacy.extend(
            cipher
                .encrypt(
                    Nonce::from_slice(&nonce),
                    b"distinctive-legacy-material".as_slice(),
                )
                .unwrap(),
        );
        assert!(legacy.starts_with(b"\x03NYX"));
        assert_eq!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                None,
                &legacy
            )
            .await
            .unwrap()
            .as_slice(),
            b"distinctive-legacy-material"
        );
        assert!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                Some(3),
                &legacy
            )
            .await
            .is_err()
        );
        let contextual = encrypt(
            &keys,
            "assistant_attachments",
            "record",
            "owner",
            "text_encrypted",
            b"current-material",
        )
        .await
        .unwrap();
        assert!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                Some(9),
                &contextual
            )
            .await
            .is_err()
        );
        let mut corrupt = contextual.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                Some(3),
                &corrupt
            )
            .await
            .is_err()
        );
        keys.set_allow_legacy_record_reads(false);
        assert!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                None,
                &legacy
            )
            .await
            .is_err()
        );
        assert_eq!(
            decrypt(
                &keys,
                "assistant_attachments",
                "record",
                "owner",
                "text_encrypted",
                Some(3),
                &contextual
            )
            .await
            .unwrap()
            .as_slice(),
            b"current-material"
        );
    }
}
