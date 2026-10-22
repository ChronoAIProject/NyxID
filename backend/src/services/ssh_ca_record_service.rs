//! Catalog SSH signing authority belongs to the platform service, not the
//! requesting person. Bind the selected service and exact retained public key;
//! user/node principals and certificate admission remain with ssh_service.
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::downstream_service::{COLLECTION_NAME, DownstreamService, SshServiceConfig},
};
use bson::{Document, doc};
use zeroize::Zeroizing;
const FIELD: &str = "ssh_config.ca_private_key_encrypted";
fn invalid() -> AppError {
    AppError::Internal("SSH signing authority storage is invalid or unresolved".into())
}
fn validate_key(public: &str, private: &[u8]) -> AppResult<()> {
    if private.is_empty() || private.len() > 65_536 || public.is_empty() || public.len() > 8192 {
        return Err(invalid());
    }
    let private = ssh_key::PrivateKey::from_openssh(private).map_err(|_| invalid())?;
    let public = ssh_key::PublicKey::from_openssh(public).map_err(|_| invalid())?;
    if private.public_key().key_data() != public.key_data() {
        return Err(invalid());
    }
    Ok(())
}
fn public_digest(public: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(public.as_bytes()))
}
pub(crate) async fn encrypt(
    keys: &EncryptionKeys,
    service_id: &str,
    public: &str,
    private: &[u8],
) -> AppResult<Vec<u8>> {
    validate_key(public, private)?;
    let digest = public_digest(public);
    super::record_field_service::encrypt_context(
        keys,
        &RecordContext::new(
            COLLECTION_NAME,
            service_id,
            "platform",
            &[FIELD, "ca_public_key_sha256", &digest],
        )?,
        private,
    )
    .await
}
pub(crate) async fn private_key(
    keys: &EncryptionKeys,
    service_id: &str,
    row: &SshServiceConfig,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let public = row.ca_public_key.as_deref().ok_or_else(invalid)?;
    let digest = public_digest(public);
    let private = super::record_field_service::decrypt_context(
        keys,
        &RecordContext::new(
            COLLECTION_NAME,
            service_id,
            "platform",
            &[FIELD, "ca_public_key_sha256", &digest],
        )?,
        row.ca_context_version,
        row.ca_private_key_encrypted
            .as_deref()
            .ok_or_else(invalid)?,
    )
    .await?;
    validate_key(public, &private)?;
    Ok(private)
}
fn config(row: &DownstreamService) -> AppResult<Option<&SshServiceConfig>> {
    if row.id.is_empty() {
        return Err(invalid());
    }
    let Some(config) = row.ssh_config.as_ref() else {
        return Ok(None);
    };
    if config.ca_private_key_encrypted.is_none() {
        if config.ca_context_version.is_some()
            || config.ca_public_key.is_some()
            || config.certificate_auth_enabled
        {
            return Err(invalid());
        }
        return Ok(None);
    }
    Ok(Some(config))
}
pub(crate) async fn verify_current_snapshot(
    keys: &EncryptionKeys,
    source: &Document,
) -> AppResult<()> {
    let row: DownstreamService = bson::from_document(source.clone()).map_err(|_| invalid())?;
    let Some(config) = config(&row)? else {
        return Ok(());
    };
    if config.ca_context_version != Some(3) {
        return Err(invalid());
    }
    let digest = public_digest(config.ca_public_key.as_deref().ok_or_else(invalid)?);
    keys.authenticate_current_record(
        config
            .ca_private_key_encrypted
            .as_deref()
            .ok_or_else(invalid)?,
        &RecordContext::new(
            COLLECTION_NAME,
            &row.id,
            "platform",
            &[FIELD, "ca_public_key_sha256", &digest],
        )?,
    )
    .await?;
    private_key(keys, &row.id, config).await?;
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
    let Some(config) = config(&row)? else {
        return Ok(false);
    };
    if verify_current_snapshot(keys, source).await.is_ok() {
        return Ok(false);
    }
    let private = private_key(keys, &row.id, config).await?;
    let ciphertext = encrypt(
        keys,
        &row.id,
        config.ca_public_key.as_deref().ok_or_else(invalid)?,
        &private,
    )
    .await?;
    let mut candidate = row.clone();
    let current = candidate.ssh_config.as_mut().ok_or_else(invalid)?;
    current.ca_private_key_encrypted = Some(ciphertext.clone());
    current.ca_context_version = Some(3);
    verify_current_snapshot(keys, &bson::to_document(&candidate).map_err(|_| invalid())?).await?;
    let db = super::transaction_service::durable_database(db);
    let changed=db.collection::<Document>(COLLECTION_NAME).update_one(super::record_storage::exact_document_predicate(source),doc!{"$set":{FIELD:bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext},"ssh_config.ca_context_version":3_i32}}).await?;
    if let Some(winner) = db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"_id":&row.id})
        .await?
    {
        verify_current_snapshot(keys, &winner).await?;
    }
    Ok(changed.matched_count == 1)
}

#[cfg(test)]
mod tests {
    use super::super::ssh_service::{self, SshConfigInput};
    use super::*;
    use ssh_key::{Algorithm, HashAlg, PrivateKey, certificate::Certificate};

    async fn sign_and_verify(keys: &EncryptionKeys, service: &DownstreamService) {
        let subject = PrivateKey::random(&mut rand::rngs::OsRng, Algorithm::Ed25519).unwrap();
        let issued = ssh_service::issue_certificate(
            keys,
            service.ssh_config.as_ref().unwrap(),
            &service.id,
            "person",
            "person@example.test",
            &subject.public_key().to_openssh().unwrap(),
            "ubuntu",
        )
        .await
        .unwrap();
        let certificate = Certificate::from_openssh(&issued.certificate).unwrap();
        let public = ssh_key::PublicKey::from_openssh(&issued.ca_public_key).unwrap();
        certificate
            .validate([&public.fingerprint(HashAlg::Sha256)])
            .unwrap();
        assert!(issued.valid_before > issued.valid_after);
    }

    #[tokio::test]
    async fn actual_signing_survives_staged_migration_and_strict_cutover_but_rejects_context_swaps()
    {
        let db = crate::test_utils::connect_transaction_test_database("ssh_ca_contextual").await;
        let mut staged = crate::test_utils::test_encryption_keys();
        staged.set_contextual_record_writes(false);
        let mut service = crate::test_utils::test_auto_connected_catalog_service();
        service.service_type = "ssh".into();
        service.ssh_config = Some(
            ssh_service::build_ssh_config(
                &staged,
                &service.id,
                None,
                SshConfigInput {
                    host: "ssh.example.test",
                    port: 22,
                    certificate_auth_enabled: true,
                    ssh_auth_mode: None,
                    certificate_ttl_minutes: 30,
                    allowed_principals: &["ubuntu".into()],
                },
            )
            .await
            .unwrap(),
        );
        assert_eq!(
            service.ssh_config.as_ref().unwrap().ca_context_version,
            None
        );
        sign_and_verify(&staged, &service).await;
        let mut source = bson::to_document(&service).unwrap();
        source.insert("unknown_catalog_metadata", "retained");
        db.collection::<Document>(COLLECTION_NAME)
            .insert_one(&source)
            .await
            .unwrap();
        let writer = crate::test_utils::test_encryption_keys();
        let mut strict = crate::test_utils::test_encryption_keys();
        strict.set_contextual_record_writes(true);
        strict.set_allow_legacy_record_reads(false);
        assert!(
            private_key(&strict, &service.id, service.ssh_config.as_ref().unwrap())
                .await
                .is_err()
        );
        assert!(migrate_snapshot(&db, &writer, &source).await.unwrap());
        let current = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&service.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.get_str("unknown_catalog_metadata"), Ok("retained"));
        verify_current_snapshot(&strict, &current).await.unwrap();
        assert!(!migrate_snapshot(&db, &writer, &current).await.unwrap());
        let row: DownstreamService = bson::from_document(current.clone()).unwrap();
        sign_and_verify(&strict, &row).await;
        let mut swapped = current.clone();
        swapped.insert("_id", uuid::Uuid::new_v4().to_string());
        assert!(verify_current_snapshot(&strict, &swapped).await.is_err());
        let unrelated = PrivateKey::random(&mut rand::rngs::OsRng, Algorithm::Ed25519).unwrap();
        swapped = current.clone();
        swapped.get_document_mut("ssh_config").unwrap().insert(
            "ca_public_key",
            unrelated.public_key().to_openssh().unwrap(),
        );
        assert!(verify_current_snapshot(&strict, &swapped).await.is_err());
        // Owner transfer deliberately preserves platform signing identity.
        swapped = current;
        swapped.insert("owner_user_id", uuid::Uuid::new_v4().to_string());
        verify_current_snapshot(&strict, &swapped).await.unwrap();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn signing_readiness_rejects_dangling_marker_and_stale_migration_cannot_rewrite_source() {
        let db = crate::test_utils::connect_transaction_test_database("ssh_ca_source_cas").await;
        let mut staged = crate::test_utils::test_encryption_keys();
        staged.set_contextual_record_writes(false);
        let mut service = crate::test_utils::test_auto_connected_catalog_service();
        service.service_type = "ssh".into();
        service.ssh_config = Some(
            ssh_service::build_ssh_config(
                &staged,
                &service.id,
                None,
                SshConfigInput {
                    host: "ssh.example.test",
                    port: 22,
                    certificate_auth_enabled: true,
                    ssh_auth_mode: None,
                    certificate_ttl_minutes: 30,
                    allowed_principals: &["ubuntu".into()],
                },
            )
            .await
            .unwrap(),
        );
        let source = bson::to_document(&service).unwrap();
        db.collection::<Document>(COLLECTION_NAME)
            .insert_one(&source)
            .await
            .unwrap();
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":&service.id},
                doc! {"$set":{"label":"changed after capture"}},
            )
            .await
            .unwrap();
        let writer = crate::test_utils::test_encryption_keys();
        assert!(migrate_snapshot(&db, &writer, &source).await.is_err());
        let winner = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&service.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            winner.get_document("ssh_config").unwrap(),
            source.get_document("ssh_config").unwrap()
        );
        service
            .ssh_config
            .as_mut()
            .unwrap()
            .certificate_auth_enabled = false;
        service
            .ssh_config
            .as_mut()
            .unwrap()
            .ca_private_key_encrypted = None;
        service.ssh_config.as_mut().unwrap().ca_public_key = None;
        let mut empty = bson::to_document(&service).unwrap();
        verify_current_snapshot(&writer, &empty).await.unwrap();
        empty
            .get_document_mut("ssh_config")
            .unwrap()
            .insert("ca_context_version", 3_i32);
        assert!(verify_current_snapshot(&writer, &empty).await.is_err());
        db.drop().await.unwrap();
    }
}
