//! Catalog master credentials retain the exact authorized record identity.
//! This decoder does not perform ACL checks: execution callers must first use
//! the platform/master authorization gate, while management keeps its authority.
use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::downstream_service::{COLLECTION_NAME, DownstreamService},
};
use bson::{Document, doc};
use zeroize::Zeroizing;

pub(crate) const FIELD: &str = "credential_encrypted";
fn invalid() -> AppError {
    AppError::Internal("Catalog credential storage is invalid or unresolved".into())
}
pub(crate) fn owner(row: &DownstreamService) -> AppResult<&str> {
    let owner = row.owner_user_id.as_deref().unwrap_or(&row.created_by);
    if owner.is_empty() {
        return Err(invalid());
    }
    Ok(owner)
}
pub(crate) fn validate(row: &DownstreamService) -> AppResult<()> {
    owner(row)?;
    if row.id.is_empty()
        || row.secret_context_versions.iter().any(|(field, version)| {
            field != FIELD || *version != 3 || row.credential_encrypted.is_empty()
        })
    {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) async fn encrypt(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    plaintext: &[u8],
) -> AppResult<Vec<u8>> {
    if plaintext.is_empty() || plaintext.len() > 65_536 {
        return Err(invalid());
    }
    super::record_field_service::encrypt(keys, COLLECTION_NAME, id, owner, FIELD, plaintext).await
}
pub(crate) fn initial_versions(
    keys: &EncryptionKeys,
    material: bool,
) -> std::collections::BTreeMap<String, u8> {
    let mut versions = std::collections::BTreeMap::new();
    if material {
        super::credential_record_service::mark_version(&mut versions, keys, FIELD);
    }
    versions
}
pub(crate) fn version_update(keys: &EncryptionKeys, material: bool) -> AppResult<bson::Bson> {
    bson::to_bson(&initial_versions(keys, material)).map_err(|_| invalid())
}
pub(crate) async fn bytes(
    keys: &EncryptionKeys,
    row: &DownstreamService,
) -> AppResult<Zeroizing<Vec<u8>>> {
    validate(row)?;
    if row.credential_encrypted.is_empty() {
        return Err(invalid());
    }
    let plaintext = super::record_field_service::decrypt(
        keys,
        COLLECTION_NAME,
        &row.id,
        owner(row)?,
        FIELD,
        row.secret_context_versions.get(FIELD).copied(),
        &row.credential_encrypted,
    )
    .await?;
    // Historical creators encrypted an empty placeholder even for no-auth rows.
    // It is readable while staged and normalized to lifecycle-empty on migration.
    if plaintext.len() > 65_536 || std::str::from_utf8(&plaintext).is_err() {
        return Err(invalid());
    }
    Ok(plaintext)
}
pub(crate) async fn text(
    keys: &EncryptionKeys,
    row: &DownstreamService,
) -> AppResult<Zeroizing<String>> {
    let plaintext = bytes(keys, row).await?;
    Ok(std::str::from_utf8(&plaintext)
        .map_err(|_| invalid())?
        .to_owned()
        .into())
}
pub(crate) async fn verify_current_snapshot(
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<()> {
    let row: DownstreamService = bson::from_document(source.clone()).map_err(|_| invalid())?;
    validate(&row)?;
    if row.credential_encrypted.is_empty() {
        return Ok(());
    }
    super::record_field_service::verify_current(
        keys,
        COLLECTION_NAME,
        &row.id,
        owner(&row)?,
        FIELD,
        row.secret_context_versions.get(FIELD).copied(),
        &row.credential_encrypted,
    )
    .await?;
    if bytes(keys, &row).await?.is_empty() {
        return Err(invalid());
    }
    Ok(())
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
    let row: DownstreamService = bson::from_document(source.clone()).map_err(|_| invalid())?;
    validate(&row)?;
    if verify_current_snapshot(keys, source).await.is_ok() {
        return Ok(false);
    }
    let plaintext = bytes(keys, &row).await?;
    let ciphertext = if plaintext.is_empty() {
        Vec::new()
    } else {
        encrypt(keys, &row.id, owner(&row)?, &plaintext).await?
    };
    let mut candidate = row.clone();
    candidate.secret_context_versions = initial_versions(keys, !ciphertext.is_empty());
    candidate.credential_encrypted = ciphertext.clone();
    verify_current_snapshot(keys, &bson::to_document(&candidate).map_err(|_| invalid())?).await?;
    let db = super::transaction_service::durable_database(db);
    let set = doc! {FIELD: bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext},
    "secret_context_versions":bson::to_bson(&candidate.secret_context_versions).map_err(|_|invalid())?};
    let changed = db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            super::record_storage::exact_document_predicate(source),
            doc! {"$set":set},
        )
        .await?
        .matched_count
        == 1;
    if let Some(winner) = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await?
    {
        verify_current_snapshot(keys, &winner).await?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::test_helpers::dummy_service;

    #[tokio::test]
    async fn catalog_master_readiness_preserves_staging_and_current_noop() {
        let db = crate::test_utils::connect_test_database("catalog_master_adapter")
            .await
            .unwrap();
        let rows = db.collection::<Document>(COLLECTION_NAME);
        let mut keys = crate::test_utils::test_encryption_keys();
        keys.set_contextual_record_writes(false);
        let mut row = dummy_service();
        row.id = uuid::Uuid::new_v4().to_string();
        row.created_by = uuid::Uuid::new_v4().to_string();
        row.auth_method = "bearer".into();
        row.service_category = "internal".into();
        row.requires_user_credential = false;
        row.provider_config_id = None;
        row.visibility = "public".into();
        row.is_active = true;
        row.credential_encrypted =
            encrypt(&keys, &row.id, owner(&row).unwrap(), b"catalog-test-secret")
                .await
                .unwrap();
        row.secret_context_versions = initial_versions(&keys, true);
        let source = bson::to_document(&row).unwrap();
        rows.insert_one(&source).await.unwrap();
        let authorized =
            super::super::proxy_service::authorize_master_credential_server_chosen(&db, &row)
                .await
                .unwrap();
        assert_eq!(
            authorized.decrypt(&keys).await.unwrap().as_slice(),
            b"catalog-test-secret"
        );
        assert!(verify_current_snapshot(&keys, &source).await.is_err());
        assert!(migrate_snapshot(&db, &keys, &source).await.is_err());
        keys.set_contextual_record_writes(true);
        assert!(migrate_snapshot(&db, &keys, &source).await.unwrap());
        let current = rows.find_one(doc! {"_id":&row.id}).await.unwrap().unwrap();
        verify_current_snapshot(&keys, &current).await.unwrap();
        assert!(!migrate_snapshot(&db, &keys, &current).await.unwrap());
        assert_eq!(
            rows.find_one(doc! {"_id":&row.id}).await.unwrap().unwrap(),
            current
        );
        keys.set_allow_legacy_record_reads(false);
        assert!(authorized.decrypt(&keys).await.is_err());
        let live: DownstreamService = bson::from_document(current.clone()).unwrap();
        let authorized =
            super::super::proxy_service::authorize_master_credential_server_chosen(&db, &live)
                .await
                .unwrap();
        assert_eq!(
            authorized.decrypt(&keys).await.unwrap().as_slice(),
            b"catalog-test-secret"
        );
        for target in ["id", "owner"] {
            let mut moved = live.clone();
            if target == "id" {
                moved.id = uuid::Uuid::new_v4().to_string();
            } else {
                moved.owner_user_id = Some(uuid::Uuid::new_v4().to_string());
            }
            assert!(bytes(&keys, &moved).await.is_err());
        }
        let mut malformed = current.clone();
        malformed.insert("owner_user_id", 17_i32);
        assert!(verify_current_snapshot(&keys, &malformed).await.is_err());
        malformed = current.clone();
        malformed.insert("secret_context_versions", doc! {FIELD:4_i32});
        assert!(verify_current_snapshot(&keys, &malformed).await.is_err());
        // Absent marker on a v3 envelope is never a fallback to another context.
        malformed = current.clone();
        malformed.remove("secret_context_versions");
        assert!(verify_current_snapshot(&keys, &malformed).await.is_err());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn empty_catalog_status_distinguishes_absent_material_from_dangling_versions() {
        let keys = crate::test_utils::test_encryption_keys();
        let mut row = dummy_service();
        row.credential_encrypted.clear();
        assert_eq!(
            super::super::platform_key_service::credential_configured(&keys, &row).await,
            Some(false)
        );
        row.secret_context_versions.insert(FIELD.into(), 3);
        assert_eq!(
            super::super::platform_key_service::credential_configured(&keys, &row).await,
            None
        );
        row.secret_context_versions.clear();
        row.secret_context_versions
            .insert("unexpected_field".into(), 3);
        assert_eq!(
            super::super::platform_key_service::credential_configured(&keys, &row).await,
            None
        );
    }

    #[tokio::test]
    async fn legacy_encrypted_empty_is_normalized_without_claiming_usable_material() {
        let db = crate::test_utils::connect_test_database("catalog_master_empty")
            .await
            .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        let mut row = dummy_service();
        row.id = uuid::Uuid::new_v4().to_string();
        row.credential_encrypted = keys.encrypt(b"").await.unwrap();
        let source = bson::to_document(&row).unwrap();
        db.collection::<Document>(COLLECTION_NAME)
            .insert_one(&source)
            .await
            .unwrap();
        assert_eq!(
            super::super::platform_key_service::credential_configured(&keys, &row).await,
            Some(false)
        );
        assert!(migrate_snapshot(&db, &keys, &source).await.unwrap());
        let current = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        verify_current_snapshot(&keys, &current).await.unwrap();
        let current: DownstreamService = bson::from_document(current).unwrap();
        assert!(current.credential_encrypted.is_empty());
        assert!(current.secret_context_versions.is_empty());
        db.drop().await.unwrap();
    }
}
