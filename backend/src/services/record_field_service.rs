//! BSON storage and exact-source migration of server-encrypted binary fields.
use crate::{
    crypto::{aes::EncryptionKeys, record_field},
    errors::{AppError, AppResult},
};

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
        if record_field::verify_current(
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
        let plain = record_field::decrypt(
            keys,
            collection,
            id,
            owner,
            field,
            versions.get(*field).copied(),
            &bytes,
        )
        .await?;
        let encrypted = record_field::encrypt(keys, collection, id, owner, field, &plain).await?;
        record_field::verify_current(keys, collection, id, owner, field, Some(3), &encrypted)
            .await?;
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
    if collection == crate::models::channel_bot::COLLECTION_NAME {
        if source.get_str("user_id").ok() != Some(owner) {
            return Err(invalid());
        }
        return super::channel_credential_continuity::publish_rewrap(db, keys, source, set).await;
    }
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
    use zeroize::Zeroizing;

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
                let ciphertext = record_field::encrypt(
                    &keys,
                    collection,
                    &id,
                    "owner",
                    "field_b",
                    b"concurrent-b",
                )
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
}
