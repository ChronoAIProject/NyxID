//! Assistant credential and attachment identities, including relationship-based
//! credential lookup and the independent BSON-literal upload-chunk variant.
use super::record_field_service as fields;
use crate::{
    crypto::aes::{EncryptionKeys, RecordContext},
    errors::{AppError, AppResult},
    models::{
        assistant_agent_credential::AssistantAgentCredential,
        assistant_attachment::AssistantAttachment,
    },
};
use zeroize::Zeroizing;
fn invalid() -> AppError {
    AppError::Internal("Assistant encrypted field authentication is invalid".into())
}

fn credential_path(row: &AssistantAgentCredential) -> [&str; 6] {
    [
        "key_ciphertext",
        "conversation",
        if row.conversation_id.is_empty() {
            "legacy_unbound"
        } else {
            "bound"
        },
        if row.conversation_id.is_empty() {
            "absent"
        } else {
            &row.conversation_id
        },
        "api_key",
        &row.api_key_id,
    ]
}
pub(crate) async fn encrypt_credential(
    keys: &EncryptionKeys,
    row: &AssistantAgentCredential,
    raw: &[u8],
) -> AppResult<Vec<u8>> {
    let path = credential_path(row);
    fields::encrypt_context(
        keys,
        &RecordContext::new(
            crate::models::assistant_agent_credential::COLLECTION_NAME,
            &row.id,
            &row.user_id,
            &path,
        )?,
        raw,
    )
    .await
}
pub(crate) async fn decrypt_credential(
    keys: &EncryptionKeys,
    row: &AssistantAgentCredential,
) -> AppResult<Zeroizing<String>> {
    if row
        .secret_context_versions
        .iter()
        .any(|(field, version)| field != "key_ciphertext" || *version != 3)
    {
        return Err(invalid());
    }
    let path = credential_path(row);
    let bytes = fields::decrypt_context(
        keys,
        &RecordContext::new(
            crate::models::assistant_agent_credential::COLLECTION_NAME,
            &row.id,
            &row.user_id,
            &path,
        )?,
        row.secret_context_versions.get("key_ciphertext").copied(),
        &row.key_ciphertext,
    )
    .await?;
    let raw = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
    if raw.is_empty() {
        return Err(invalid());
    }
    Ok(Zeroizing::new(raw.to_owned()))
}
pub(crate) async fn verify_credential_hash(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    row: &AssistantAgentCredential,
) -> AppResult<()> {
    let raw = decrypt_credential(keys, row).await?;
    let db = super::transaction_service::durable_database(db);
    let matching = db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .find_one(bson::doc! {"_id":&row.api_key_id,"user_id":&row.user_id,"key_hash":crate::crypto::token::hash_token(&raw)})
        .projection(bson::doc! {"_id":1}).await?;
    if matching.is_none() {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) async fn verify_current_credential(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &bson::Document,
) -> AppResult<()> {
    let row: AssistantAgentCredential =
        bson::from_document(source.clone()).map_err(|_| invalid())?;
    if row.secret_context_versions.len() != 1
        || row.secret_context_versions.get("key_ciphertext") != Some(&3)
    {
        return Err(invalid());
    }
    let path = credential_path(&row);
    keys.authenticate_current_record(
        &row.key_ciphertext,
        &RecordContext::new(
            crate::models::assistant_agent_credential::COLLECTION_NAME,
            &row.id,
            &row.user_id,
            &path,
        )?,
    )
    .await?;
    verify_credential_hash(db, keys, &row).await
}
pub(crate) async fn migrate_credential_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &bson::Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    if verify_current_credential(db, keys, source).await.is_ok() {
        return Ok(false);
    }
    let row: AssistantAgentCredential =
        bson::from_document(source.clone()).map_err(|_| invalid())?;
    verify_credential_hash(db, keys, &row).await?;
    let raw = decrypt_credential(keys, &row).await?;
    let ciphertext = encrypt_credential(keys, &row, raw.as_bytes()).await?;
    let mut target = source.clone();
    let mut versions = row.secret_context_versions.clone();
    versions.insert("key_ciphertext".to_owned(), 3);
    target.insert(
        "secret_context_versions",
        bson::to_bson(&versions).map_err(|_| invalid())?,
    );
    target.insert(
        "key_ciphertext",
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: ciphertext,
        },
    );
    verify_current_credential(db, keys, &target).await?;
    let filter = super::record_storage::exact_document_predicate(source);
    let db = super::transaction_service::durable_database(db);
    Ok(db.collection::<bson::Document>(crate::models::assistant_agent_credential::COLLECTION_NAME)
        .update_one(filter,bson::doc! {"$set":{"key_ciphertext":target.get("key_ciphertext").unwrap(),"secret_context_versions":target.get("secret_context_versions").unwrap()}}).await?.matched_count == 1)
}

fn image_path(row: &AssistantAttachment) -> [&str; 8] {
    [
        "data_encrypted",
        "origin",
        &row.origin,
        "conversation",
        &row.conversation_id,
        "turn",
        if row.turn_id.is_empty() {
            "legacy_unbound"
        } else {
            "bound"
        },
        if row.turn_id.is_empty() {
            "absent"
        } else {
            &row.turn_id
        },
    ]
}
pub(crate) async fn encrypt_image(
    keys: &EncryptionKeys,
    row: &AssistantAttachment,
    bytes: &[u8],
) -> AppResult<Vec<u8>> {
    let path = image_path(row);
    fields::encrypt_context(
        keys,
        &RecordContext::new(
            crate::models::assistant_attachment::COLLECTION_NAME,
            &row.id,
            &row.user_id,
            &path,
        )?,
        bytes,
    )
    .await
}
pub(crate) async fn decrypt_image(
    keys: &EncryptionKeys,
    row: &AssistantAttachment,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let path = image_path(row);
    fields::decrypt_context(
        keys,
        &RecordContext::new(
            crate::models::assistant_attachment::COLLECTION_NAME,
            &row.id,
            &row.user_id,
            &path,
        )?,
        row.secret_context_versions.get("data_encrypted").copied(),
        &row.data_encrypted,
    )
    .await
}
pub(crate) async fn encrypt_chunk(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    root: &str,
    index: i64,
    bytes: &[u8],
) -> AppResult<Vec<u8>> {
    if index < 0 {
        return Err(invalid());
    }
    let index = index.to_string();
    fields::encrypt_context(
        keys,
        &RecordContext::new(
            crate::models::assistant_attachment::COLLECTION_NAME,
            id,
            owner,
            &[
                "data_encrypted",
                "parent_attachment_id",
                root,
                "chunk_index",
                &index,
            ],
        )?,
        bytes,
    )
    .await
}
pub(crate) async fn decrypt_chunk(
    keys: &EncryptionKeys,
    id: &str,
    owner: &str,
    expected_root: &str,
    expected_index: i64,
    version: Option<u8>,
    ciphertext: &[u8],
) -> AppResult<Zeroizing<Vec<u8>>> {
    if expected_index < 0 {
        return Err(invalid());
    }
    let index = expected_index.to_string();
    fields::decrypt_context(
        keys,
        &RecordContext::new(
            crate::models::assistant_attachment::COLLECTION_NAME,
            id,
            owner,
            &[
                "data_encrypted",
                "parent_attachment_id",
                expected_root,
                "chunk_index",
                &index,
            ],
        )?,
        version,
        ciphertext,
    )
    .await
}

/// The collection has three distinct storage variants. Resolve the relationship
/// before authenticating a chunk; its own mutable parent/index are not authority.
struct AttachmentIdentity {
    id: String,
    owner: String,
    field: &'static str,
    path: Vec<String>,
    version: Option<u8>,
    ciphertext: Vec<u8>,
    expected_size: Option<usize>,
    text_content_type: Option<String>,
    preview: bool,
}

async fn attachment_identity(
    db: &mongodb::Database,
    source: &bson::Document,
) -> AppResult<AttachmentIdentity> {
    let id = source.get_str("_id").map_err(|_| invalid())?.to_owned();
    let owner = source.get_str("user_id").map_err(|_| invalid())?.to_owned();
    let (field, path, expected_size, text_content_type, preview) =
        if source.contains_key("parent_attachment_id") {
            let root_id = source
                .get_str("parent_attachment_id")
                .map_err(|_| invalid())?;
            let index = source.get_i64("chunk_index").map_err(|_| invalid())?;
            let root = db
                .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
                .find_one(bson::doc! {"_id":root_id,"user_id":&owner,"origin":"user_upload"})
                .await?
                .ok_or_else(invalid)?;
            let root: crate::models::assistant_upload::AssistantUpload =
                bson::from_document(root).map_err(|_| invalid())?;
            let size = usize::try_from(root.size).map_err(|_| invalid())?;
            const CHUNK: usize = 4 * 1024 * 1024;
            if size == 0
                || size > crate::services::attachment_extraction::MAX_BYTES
                || root.chunks != size.div_ceil(CHUNK)
                || index < 0
                || index as usize >= root.chunks
                || source.get_str("conversation_id").ok() != Some(root.conversation_id.as_str())
                || source.get("group_id")
                    != Some(&bson::to_bson(&root.group_id).map_err(|_| invalid())?)
            {
                return Err(invalid());
            }
            let expected_size = (size - (index as usize) * CHUNK).min(CHUNK);
            (
                "data_encrypted",
                vec![
                    "data_encrypted".into(),
                    "parent_attachment_id".into(),
                    root.id,
                    "chunk_index".into(),
                    index.to_string(),
                ],
                Some(expected_size),
                None,
                false,
            )
        } else if source.get_str("origin").ok() == Some("user_upload") {
            let row: crate::models::assistant_upload::AssistantUpload =
                bson::from_document(source.clone()).map_err(|_| invalid())?;
            (
                "text_encrypted",
                vec!["text_encrypted".into()],
                None,
                Some(row.content_type),
                false,
            )
        } else {
            let row: AssistantAttachment =
                bson::from_document(source.clone()).map_err(|_| invalid())?;
            if !matches!(row.origin.as_str(), "tool" | "machine_preview") {
                return Err(invalid());
            }
            let size = usize::try_from(row.size).map_err(|_| invalid())?;
            let path = image_path(&row).iter().map(|s| (*s).to_owned()).collect();
            (
                "data_encrypted",
                path,
                Some(size),
                None,
                row.origin == "machine_preview",
            )
        };
    let versions = fields::stored_versions(source)?;
    if versions
        .iter()
        .any(|(name, version)| name != field || *version != 3)
        || (field == "text_encrypted" && source.contains_key("data_encrypted"))
        || (field == "data_encrypted" && source.contains_key("text_encrypted"))
    {
        return Err(invalid());
    }
    #[derive(serde::Deserialize)]
    struct Bytes {
        #[serde(with = "crate::models::bson_bytes::required")]
        bytes: Vec<u8>,
    }
    let ciphertext =
        bson::from_document::<Bytes>(bson::doc! {"bytes":source.get(field).ok_or_else(invalid)?})
            .map_err(|_| invalid())?
            .bytes;
    Ok(AttachmentIdentity {
        id,
        owner,
        field,
        path,
        version: versions.get(field).copied(),
        ciphertext,
        expected_size,
        text_content_type,
        preview,
    })
}

fn validate_attachment_plain(identity: &AttachmentIdentity, plain: &[u8]) -> AppResult<()> {
    if identity
        .expected_size
        .is_some_and(|size| size != plain.len())
    {
        return Err(invalid());
    }
    if identity.preview {
        std::str::from_utf8(plain).map_err(|_| invalid())?;
    }
    if let Some(content_type) = &identity.text_content_type {
        let extracted: crate::services::attachment_extraction::Extracted =
            serde_json::from_slice(plain).map_err(|_| invalid())?;
        let chars = extracted.text.chars().count();
        if extracted.content_type != *content_type
            || chars > 1_000_000
            || extracted
                .sections
                .iter()
                .any(|section| section.offset > chars)
            || extracted
                .sections
                .windows(2)
                .any(|pair| pair[0].offset > pair[1].offset)
        {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(crate) async fn verify_current_attachment(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &bson::Document,
) -> AppResult<()> {
    let db = super::transaction_service::durable_database(db);
    let identity = attachment_identity(&db, source).await?;
    if identity.version != Some(3) {
        return Err(invalid());
    }
    let path: Vec<&str> = identity.path.iter().map(String::as_str).collect();
    let context = RecordContext::new(
        crate::models::assistant_attachment::COLLECTION_NAME,
        &identity.id,
        &identity.owner,
        &path,
    )?;
    keys.authenticate_current_record(&identity.ciphertext, &context)
        .await?;
    let plain =
        fields::decrypt_context(keys, &context, identity.version, &identity.ciphertext).await?;
    validate_attachment_plain(&identity, &plain)
}

pub(crate) async fn migrate_attachment_snapshot(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    source: &bson::Document,
) -> AppResult<bool> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration".into(),
        ));
    }
    let db = super::transaction_service::durable_database(db);
    if verify_current_attachment(&db, keys, source).await.is_ok() {
        return Ok(false);
    }
    let identity = attachment_identity(&db, source).await?;
    let path: Vec<&str> = identity.path.iter().map(String::as_str).collect();
    let context = RecordContext::new(
        crate::models::assistant_attachment::COLLECTION_NAME,
        &identity.id,
        &identity.owner,
        &path,
    )?;
    let plain =
        fields::decrypt_context(keys, &context, identity.version, &identity.ciphertext).await?;
    validate_attachment_plain(&identity, &plain)?;
    let ciphertext = keys.encrypt_record(&plain, &context).await?;
    keys.authenticate_current_record(&ciphertext, &context)
        .await?;
    let mut versions = fields::stored_versions(source)?;
    versions.insert(identity.field.to_owned(), 3);
    let mut set =
        bson::doc! {"secret_context_versions":bson::to_bson(&versions).map_err(|_|invalid())?};
    set.insert(
        identity.field,
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: ciphertext,
        },
    );
    Ok(db
        .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .update_one(
            super::record_storage::exact_document_predicate(source),
            bson::doc! {"$set":set},
        )
        .await?
        .matched_count
        == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{Binary, doc, spec::BinarySubtype};
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
    async fn attachment_variants_migrate_exact_arrays_and_bind_chunk_root_and_index() {
        let db = crate::test_utils::connect_test_database("assistant_attachment_variants")
            .await
            .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        let owner = uuid::Uuid::new_v4().to_string();
        let root_id = uuid::Uuid::new_v4().to_string();
        let collection = crate::models::assistant_attachment::COLLECTION_NAME;
        let rows = db.collection::<bson::Document>(collection);
        let text = crate::crypto::secret_json::SecretJson(serde_json::json!({
            "content_type":"text/plain","text":"hello","sections":[{"offset":0,"label":"Page 1"}]}));
        let encoded = crate::crypto::secret_json::to_secret_string(&text.0).unwrap();
        // The legacy storage is deliberately an integer array, not Binary.
        let root = doc! {"_id":&root_id,"user_id":&owner,"origin":"user_upload",
        "conversation_id":"nyxa-fixture","group_id":bson::Bson::Null,
        "message_id":bson::Bson::Null,"label":"fixture.txt","content_type":"text/plain",
        "size":5_i64,"pages":1_i64,"chunks":1_i64,
        "text_encrypted":keys.encrypt(encoded.as_bytes()).await.unwrap().iter().map(|b|bson::Bson::Int32(*b as i32)).collect::<Vec<_>>(),
        "created_at":bson::DateTime::now()};
        let chunk = doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,
        "conversation_id":"nyxa-fixture","group_id":bson::Bson::Null,
        "parent_attachment_id":&root_id,"chunk_index":0_i64,
        "data_encrypted":Binary{subtype:BinarySubtype::Generic,bytes:keys.encrypt(b"hello").await.unwrap()}};
        let mut sources = vec![root.clone(), chunk];
        for origin in ["tool", "machine_preview"] {
            sources.push(doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&owner,
                "origin":origin,"conversation_id":"nyxa-fixture","turn_id":"turn-fixture",
                "content_type":if origin=="tool" {"image/png"} else {"text/plain"},"size":5_i64,
                "data_encrypted":Binary{subtype:BinarySubtype::Generic,bytes:keys.encrypt(b"hello").await.unwrap()},
                "created_at":bson::DateTime::now()});
        }
        rows.insert_many(&sources).await.unwrap();
        let mut strict = crate::test_utils::test_encryption_keys();
        strict.set_allow_legacy_record_reads(false);
        for source in &sources {
            assert!(verify_current_attachment(&db, &keys, source).await.is_err());
            assert!(
                migrate_attachment_snapshot(&db, &keys, source)
                    .await
                    .unwrap()
            );
            let live = rows
                .find_one(doc! {"_id":source.get("_id").unwrap()})
                .await
                .unwrap()
                .unwrap();
            verify_current_attachment(&db, &strict, &live)
                .await
                .unwrap();
            assert!(
                !migrate_attachment_snapshot(&db, &keys, &live)
                    .await
                    .unwrap()
            );
            assert!(
                !migrate_attachment_snapshot(&db, &keys, source)
                    .await
                    .unwrap()
            );
            let mut swapped = live.clone();
            swapped.insert("user_id", "unrelated-owner");
            assert!(
                verify_current_attachment(&db, &strict, &swapped)
                    .await
                    .is_err()
            );
        }
        let live_chunk = rows
            .find_one(doc! {"parent_attachment_id":&root_id})
            .await
            .unwrap()
            .unwrap();
        let id = live_chunk.get_str("_id").unwrap();
        let encrypted = live_chunk.get_binary_generic("data_encrypted").unwrap();
        assert_eq!(
            decrypt_chunk(&strict, id, &owner, &root_id, 0, Some(3), encrypted)
                .await
                .unwrap()
                .as_slice(),
            b"hello"
        );
        assert!(
            decrypt_chunk(&strict, id, &owner, &root_id, 1, Some(3), encrypted)
                .await
                .is_err()
        );
        assert!(
            decrypt_chunk(&strict, id, &owner, "other-root", 0, Some(3), encrypted)
                .await
                .is_err()
        );
        let mut other_root = root.clone();
        let other_id = uuid::Uuid::new_v4().to_string();
        other_root.insert("_id", &other_id);
        rows.insert_one(other_root).await.unwrap();
        let mut relocated = live_chunk.clone();
        relocated.insert("parent_attachment_id", &other_id);
        assert!(
            verify_current_attachment(&db, &strict, &relocated)
                .await
                .is_err()
        );
        rows.delete_one(doc! {"_id":&root_id}).await.unwrap();
        assert!(
            verify_current_attachment(&db, &strict, &live_chunk)
                .await
                .is_err(),
            "orphan chunks cannot certify retirement"
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn held_credential_migration_cannot_overwrite_new_conversation_and_version() {
        let db = crate::test_utils::connect_test_database("assistant_credential_absent_cas")
            .await
            .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        let id = uuid::Uuid::new_v4().to_string();
        let owner = uuid::Uuid::new_v4().to_string();
        let key_id = uuid::Uuid::new_v4().to_string();
        let raw = zeroize::Zeroizing::new("distinctive-bound-assistant-key".to_owned());
        db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .insert_one(doc! {"_id":&key_id,"user_id":&owner,
            "key_hash":crate::crypto::token::hash_token(&raw)})
            .await
            .unwrap();
        let source = doc! {"_id":&id,"user_id":&owner,"api_key_id":&key_id,
        "key_ciphertext":Binary{subtype:BinarySubtype::Generic,
            bytes:keys.encrypt(raw.as_bytes()).await.unwrap()},
        "created_at":bson::DateTime::now(),"last_used_at":bson::DateTime::now()};
        assert!(!source.contains_key("conversation_id"));
        assert!(!source.contains_key("secret_context_versions"));
        let rows = db.collection::<bson::Document>(
            crate::models::assistant_agent_credential::COLLECTION_NAME,
        );
        rows.insert_one(&source).await.unwrap();
        let held = std::sync::Arc::new(HoldFirstWrap {
            inner: crate::crypto::local_key_provider::LocalKeyProvider::new([0x11; 32], None),
            held: Default::default(),
            entered: Default::default(),
            release: Default::default(),
        });
        let held_keys = EncryptionKeys::with_provider(held.clone());
        let migrate = migrate_credential_snapshot(&db, &held_keys, &source);
        let writer = async {
            held.entered.notified().await;
            let mut bound: AssistantAgentCredential = bson::from_document(source.clone()).unwrap();
            bound.conversation_id = "nyxa-concurrently-bound-conversation".into();
            let ciphertext = encrypt_credential(&keys, &bound, raw.as_bytes())
                .await
                .unwrap();
            rows.update_one(
                doc! {"_id":&id},
                doc! {"$set":{
                "conversation_id":&bound.conversation_id,
                "key_ciphertext":Binary{subtype:BinarySubtype::Generic,bytes:ciphertext},
                "secret_context_versions":{"key_ciphertext":3_i32}}},
            )
            .await
            .unwrap();
            let winner = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
            held.release.notify_one();
            winner
        };
        let (result, winner) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            tokio::join!(migrate, writer)
        })
        .await
        .expect("held credential migration must finish");
        assert!(!result.unwrap());
        assert_eq!(
            rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap(),
            winner
        );
        verify_current_credential(&db, &keys, &winner)
            .await
            .unwrap();
        assert!(
            !migrate_credential_snapshot(&db, &keys, &winner)
                .await
                .unwrap()
        );
        let mut wrong = winner.clone();
        wrong.remove("conversation_id");
        assert!(verify_current_credential(&db, &keys, &wrong).await.is_err());
        db.drop().await.unwrap();
    }
}
