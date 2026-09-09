use bson::{Document, doc};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::options::ReturnDocument;
use zeroize::Zeroizing;

use super::oracle_login_snapshot_service::{
    CreateLoginSnapshotInput, LOGIN_SNAPSHOT_FORMAT_VERSION, MAX_LOGIN_SNAPSHOT_BYTES,
    verify_worker_token_hash,
};
use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::oracle_login_profile::{
    COLLECTION_NAME, OracleLoginBinding, OracleLoginProfile,
};
use crate::models::oracle_pool::OraclePool;
use crate::models::oracle_worker::{COLLECTION_NAME as WORKERS, worker_doc_id};
use crate::services::api_key_mutation_service as transactions;
use crate::services::{oracle_pool_service, oracle_worker_service};

pub const SAVED_LOGIN_CAPABILITY: &str = "saved_login_v1";
pub const RETENTION_DAYS: i64 = 30;
pub const MAX_BINDINGS: usize = 256;

#[derive(serde::Deserialize)]
pub struct LoginProfileMetadata {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub generation: String,
    pub revision: String,
    pub worker_token_hash: String,
    pub envelope_size: u64,
    pub bindings: Vec<OracleLoginBinding>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: chrono::DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: chrono::DateTime<Utc>,
}

impl From<&OracleLoginProfile> for LoginProfileMetadata {
    fn from(profile: &OracleLoginProfile) -> Self {
        Self {
            id: profile.id.clone(),
            name: profile.name.clone(),
            generation: profile.generation.clone(),
            revision: profile.revision.clone(),
            worker_token_hash: profile.worker_token_hash.clone(),
            envelope_size: profile.envelope_size,
            bindings: profile.bindings.clone(),
            updated_at: profile.updated_at,
            expires_at: profile.expires_at,
        }
    }
}

pub async fn ensure_indexes(db: &mongodb::Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<Document>(COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! { "pool_id": 1, "name": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! { "pool_id": 1, "bindings.worker_label": 1 })
                .build(),
            IndexModel::builder().keys(doc! { "expires_at": 1 }).build(),
        ])
        .await?;
    Ok(())
}

pub async fn purge_expired(db: &mongodb::Database) -> AppResult<u64> {
    Ok(db.collection::<Document>(COLLECTION_NAME).update_many(
        doc! { "expires_at": { "$lte": bson::DateTime::from_chrono(Utc::now()) }, "envelope_size": { "$gt": 0_i64 } },
        doc! { "$set": { "encrypted_envelope": bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic, bytes: vec![] }, "envelope_size": 0_i64 } },
    ).await?.modified_count)
}

fn document(value: &impl serde::Serialize) -> AppResult<Document> {
    bson::to_document(value)
        .map_err(|_| AppError::Internal("saved login serialization failed".into()))
}

async fn check_pool(db: &mongodb::Database, pool: &OraclePool) -> AppResult<()> {
    let live = oracle_pool_service::get_pool(db, &pool.id).await?;
    if !live.is_active || live.worker_token_hash != pool.worker_token_hash {
        return Err(AppError::OracleWorkerTokenInvalid);
    }
    Ok(())
}

pub fn validate_name(name: &str) -> AppResult<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(AppError::ValidationError(
            "login profile name must be 1-64 letters, digits, '-' or '_'".into(),
        ));
    }
    Ok(())
}

fn conflict() -> AppError {
    AppError::Conflict("saved login changed; fetch the current profile before retrying".into())
}

fn aad(profile: &OracleLoginProfile) -> String {
    format!(
        "oracle-login-profile:{}:{}:{}:{}:v{}",
        profile.pool_id, profile.id, profile.generation, profile.revision, profile.format_version
    )
}

fn validate_envelope(version: u32, bytes: &[u8]) -> AppResult<()> {
    if version != LOGIN_SNAPSHOT_FORMAT_VERSION {
        return Err(AppError::ValidationError(
            "unsupported login format_version".into(),
        ));
    }
    if bytes.is_empty() || bytes.len() > MAX_LOGIN_SNAPSHOT_BYTES {
        return Err(AppError::OraclePayloadTooLarge(
            "login envelope exceeds size limit".into(),
        ));
    }
    Ok(())
}

pub async fn list(db: &mongodb::Database, pool_id: &str) -> AppResult<Vec<LoginProfileMetadata>> {
    Ok(db
        .collection::<LoginProfileMetadata>(COLLECTION_NAME)
        .find(doc! { "pool_id": pool_id })
        .projection(doc! { "encrypted_envelope": 0 })
        .sort(doc! { "name": 1 })
        .await?
        .try_collect()
        .await?)
}

pub async fn save(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    pool: &OraclePool,
    name: &str,
    expected_generation: Option<&str>,
    input: CreateLoginSnapshotInput,
) -> AppResult<OracleLoginProfile> {
    validate_name(name)?;
    validate_envelope(input.format_version, &input.sealed_envelope)?;
    verify_worker_token_hash(pool, &input.worker_token_sha256)?;
    let collection = db.collection::<OracleLoginProfile>(COLLECTION_NAME);
    let prior = collection
        .find_one(doc! { "pool_id": &pool.id, "name": name })
        .await?;
    if prior.as_ref().map(|p| p.generation.as_str()) != expected_generation {
        return Err(conflict());
    }
    let now = Utc::now();
    let mut profile = OracleLoginProfile {
        id: prior
            .as_ref()
            .map(|p| p.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        pool_id: pool.id.clone(),
        name: name.into(),
        generation: uuid::Uuid::new_v4().to_string(),
        revision: uuid::Uuid::new_v4().to_string(),
        worker_token_hash: pool.worker_token_hash.clone(),
        format_version: input.format_version,
        encrypted_envelope: vec![],
        envelope_size: input.sealed_envelope.len() as u64,
        bindings: vec![],
        created_at: now,
        updated_at: now,
        expires_at: now + Duration::days(RETENTION_DAYS),
    };
    profile.encrypted_envelope = keys
        .encrypt_with_aad(&input.sealed_envelope, aad(&profile).as_bytes())
        .await?;
    check_pool(db, pool).await?;
    // Preserve concurrent binding changes. The ID fence prevents a deleted profile
    // from being resurrected by an upload that began before revocation.
    if let Some(prior) = prior {
        let mut update = document(&profile)?;
        for key in ["_id", "bindings", "created_at"] {
            update.remove(key);
        }
        collection
            .find_one_and_update(
                doc! { "_id": &prior.id, "generation": &prior.generation },
                doc! { "$set": update },
            )
            .return_document(ReturnDocument::After)
            .await?
            .ok_or_else(conflict)
    } else {
        match collection.insert_one(&profile).await {
            Ok(_) => Ok(profile),
            Err(error) if oracle_pool_service::is_duplicate_key(&error) => Err(conflict()),
            Err(error) => Err(error.into()),
        }
    }
}

pub async fn delete(db: &mongodb::Database, pool_id: &str, name: &str) -> AppResult<()> {
    validate_name(name)?;
    db.collection::<OracleLoginProfile>(COLLECTION_NAME)
        .delete_one(doc! { "pool_id": pool_id, "name": name })
        .await?;
    Ok(())
}

pub async fn unbind(db: &mongodb::Database, pool_id: &str, label: &str) -> AppResult<()> {
    remove_binding(db, pool_id, label, false).await
}

pub async fn forget_binding_and_worker(
    db: &mongodb::Database,
    pool_id: &str,
    label: &str,
) -> AppResult<()> {
    remove_binding(db, pool_id, label, true).await
}

async fn remove_binding(
    db: &mongodb::Database,
    pool_id: &str,
    label: &str,
    forget: bool,
) -> AppResult<()> {
    let db = db.clone();
    let pool_id = pool_id.to_string();
    let label = label.to_string();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation: AppResult<()> = async {
                db.collection::<Document>(WORKERS)
                    .update_one(
                        doc! { "_id": worker_doc_id(&pool_id, &label) },
                        doc! { "$inc": { "login_binding_epoch": 1_i64 } },
                    )
                    .session(&mut *session)
                    .await?;
                db.collection::<Document>(COLLECTION_NAME)
                    .update_many(
                        doc! { "pool_id": &pool_id, "bindings.worker_label": &label },
                        doc! { "$pull": { "bindings": { "worker_label": &label } } },
                    )
                    .session(&mut *session)
                    .await?;
                if forget {
                    db.collection::<Document>(WORKERS)
                        .delete_one(doc! { "_id": worker_doc_id(&pool_id, &label) })
                        .session(&mut *session)
                        .await?;
                }
                Ok(())
            }
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

pub async fn bind(
    db: &mongodb::Database,
    pool: &OraclePool,
    name: &str,
    label: &str,
    installation_id: Option<&str>,
    replace_existing: bool,
) -> AppResult<OracleLoginBinding> {
    validate_name(name)?;
    let worker = oracle_worker_service::get_worker(db, &pool.id, label).await?;
    let instance_id = match (worker.instance_id.as_deref(), installation_id) {
        (Some(existing), Some(requested)) if existing != requested => return Err(conflict()),
        (Some(existing), _) => existing,
        (None, Some(requested)) if uuid::Uuid::parse_str(requested).is_ok() => requested,
        _ => {
            return Err(AppError::ValidationError(
                "worker installation ID is required".into(),
            ));
        }
    };
    let collection = db.collection::<OracleLoginProfile>(COLLECTION_NAME);
    let profile = collection
        .find_one(doc! { "pool_id": &pool.id, "name": name })
        .await?
        .ok_or_else(|| AppError::NotFound("saved login profile not found".into()))?;
    if profile.worker_token_hash != pool.worker_token_hash || profile.expires_at <= Utc::now() {
        return Err(AppError::Conflict(
            "saved login expired or token changed; save a fresh login".into(),
        ));
    }
    let binding = OracleLoginBinding {
        worker_label: label.into(),
        instance_id: instance_id.into(),
        binding_id: uuid::Uuid::new_v4().to_string(),
        replace_existing,
    };
    let binding_doc = document(&binding)?;
    let db = db.clone();
    let pool = pool.clone();
    let label = label.to_string();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation: AppResult<()> = async {
                // Serialize all account switches for this worker, including ABA rebinds.
                let locked = db
                    .collection::<Document>(WORKERS)
                    .update_one(
                        doc! { "_id": &worker.id, "instance_id": &worker.instance_id },
                        doc! { "$inc": { "login_binding_epoch": 1_i64 } },
                    )
                    .session(&mut *session)
                    .await?;
                if locked.matched_count != 1 {
                    return Err(conflict());
                }
                db.collection::<Document>(COLLECTION_NAME)
                    .update_many(
                        doc! { "pool_id": &pool.id, "bindings.worker_label": &label },
                        doc! { "$pull": { "bindings": { "worker_label": &label } } },
                    )
                    .session(&mut *session)
                    .await?;
                let result = collection
                    .update_one(
                        doc! {
                            "_id": &profile.id, "generation": &profile.generation,
                            "worker_token_hash": &pool.worker_token_hash,
                            "expires_at": { "$gt": bson::DateTime::from_chrono(Utc::now()) },
                            "$expr": { "$lt": [{ "$size": "$bindings" }, MAX_BINDINGS as i64] },
                        },
                        doc! { "$push": { "bindings": &binding_doc } },
                    )
                    .session(&mut *session)
                    .await?;
                if result.modified_count != 1 {
                    return Err(conflict());
                }
                Ok(())
            }
            .await;
            transactions::transaction_result(operation)
        })
        .await
        .map_err(transactions::map_transaction_error)?;
    Ok(binding)
}

async fn validate_worker(
    db: &mongodb::Database,
    pool: &OraclePool,
    label: &str,
    instance: &str,
) -> AppResult<()> {
    let worker = oracle_worker_service::get_worker(db, &pool.id, label).await?;
    if worker.instance_id.as_deref() != Some(instance) {
        return Err(AppError::OracleWorkerLabelUnavailable(label.into()));
    }
    if !worker
        .capabilities
        .iter()
        .any(|c| c == SAVED_LOGIN_CAPABILITY)
    {
        return Err(AppError::OracleWorkerCapabilityUnsupported(
            SAVED_LOGIN_CAPABILITY.into(),
        ));
    }
    Ok(())
}

pub async fn current(
    db: &mongodb::Database,
    pool: &OraclePool,
    label: &str,
    instance: &str,
) -> AppResult<Option<(OracleLoginProfile, OracleLoginBinding)>> {
    validate_worker(db, pool, label, instance).await?;
    let profile = db
        .collection::<OracleLoginProfile>(COLLECTION_NAME)
        .find_one(doc! {
            "pool_id": &pool.id,
            "bindings": { "$elemMatch": { "worker_label": label, "instance_id": instance } },
        })
        .await?;
    Ok(profile.and_then(|profile| {
        let binding = profile
            .bindings
            .iter()
            .find(|b| b.worker_label == label && b.instance_id == instance)?
            .clone();
        Some((profile, binding))
    }))
}

pub fn availability(profile: &OracleLoginProfile, pool: &OraclePool) -> &'static str {
    availability_fields(&profile.worker_token_hash, profile.expires_at, pool)
}

pub fn metadata_availability(profile: &LoginProfileMetadata, pool: &OraclePool) -> &'static str {
    availability_fields(&profile.worker_token_hash, profile.expires_at, pool)
}

fn availability_fields(
    token_hash: &str,
    expires_at: chrono::DateTime<Utc>,
    pool: &OraclePool,
) -> &'static str {
    if token_hash != pool.worker_token_hash {
        "token_changed"
    } else if expires_at <= Utc::now() {
        "expired"
    } else {
        "available"
    }
}

pub async fn decrypt(
    keys: &EncryptionKeys,
    profile: &OracleLoginProfile,
) -> AppResult<Zeroizing<Vec<u8>>> {
    Ok(Zeroizing::new(
        keys.decrypt_with_aad(&profile.encrypted_envelope, aad(profile).as_bytes())
            .await?,
    ))
}

pub struct RefreshLoginInput {
    pub profile_id: String,
    pub binding_id: String,
    pub generation: String,
    pub expected_revision: String,
    pub publication_id: String,
    pub format_version: u32,
    pub sealed_envelope: Zeroizing<Vec<u8>>,
}

pub async fn refresh(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    pool: &OraclePool,
    label: &str,
    instance: &str,
    input: RefreshLoginInput,
) -> AppResult<OracleLoginProfile> {
    validate_worker(db, pool, label, instance).await?;
    validate_envelope(input.format_version, &input.sealed_envelope)?;
    if uuid::Uuid::parse_str(&input.publication_id).is_err()
        || input.publication_id == input.expected_revision
    {
        return Err(AppError::ValidationError(
            "publication_id must be a fresh UUID".into(),
        ));
    }
    let now = Utc::now();
    let mut filter = doc! {
        "_id": &input.profile_id, "pool_id": &pool.id,
        "worker_token_hash": &pool.worker_token_hash,
        "generation": &input.generation,
        "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
        "bindings": { "$elemMatch": {
            "worker_label": label, "instance_id": instance, "binding_id": &input.binding_id,
        } },
    };
    let collection = db.collection::<OracleLoginProfile>(COLLECTION_NAME);
    let mut profile = collection
        .find_one(filter.clone())
        .await?
        .ok_or_else(conflict)?;
    if profile.revision == input.publication_id {
        return Ok(profile);
    }
    if profile.revision != input.expected_revision {
        return Err(conflict());
    }
    filter.insert("revision", &input.expected_revision);
    profile.revision = input.publication_id;
    profile.format_version = input.format_version;
    profile.envelope_size = input.sealed_envelope.len() as u64;
    profile.updated_at = now;
    profile.expires_at = now + Duration::days(RETENTION_DAYS);
    profile.encrypted_envelope = keys
        .encrypt_with_aad(&input.sealed_envelope, aad(&profile).as_bytes())
        .await?;
    check_pool(db, pool).await?;
    let mut update = document(&profile)?;
    for key in ["_id", "bindings", "created_at"] {
        update.remove(key);
    }
    collection
        .find_one_and_update(filter, doc! { "$set": update })
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(conflict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::oracle_worker_service::WorkerPresenceInput;
    use crate::test_utils::{connect_transaction_test_database, test_encryption_keys};

    async fn setup() -> (mongodb::Database, OraclePool, EncryptionKeys) {
        let db = connect_transaction_test_database("oracle_login_profile").await;
        ensure_indexes(&db).await.unwrap();
        let (pool, _) = oracle_pool_service::create_pool(
            &db,
            &uuid::Uuid::new_v4().to_string(),
            oracle_pool_service::CreatePoolInput {
                slug: "saved-login-test".into(),
                name: "Test".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        (db, pool, test_encryption_keys())
    }

    fn upload(pool: &OraclePool, value: u8) -> CreateLoginSnapshotInput {
        CreateLoginSnapshotInput {
            format_version: 1,
            worker_token_sha256: Zeroizing::new(pool.worker_token_hash.clone()),
            sealed_envelope: Zeroizing::new(vec![value; 128]),
        }
    }

    async fn worker(db: &mongodb::Database, pool: &OraclePool, label: &str) -> String {
        oracle_worker_service::allocate_worker(db, pool, Some(label))
            .await
            .unwrap();
        let instance = uuid::Uuid::new_v4().to_string();
        oracle_worker_service::report_presence(
            db,
            pool,
            WorkerPresenceInput {
                worker_label: label.into(),
                instance_id: Some(instance.clone()),
                capabilities: vec![SAVED_LOGIN_CAPABILITY.into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        instance
    }

    fn publication(
        profile: &OracleLoginProfile,
        binding: &OracleLoginBinding,
        publication_id: &str,
    ) -> RefreshLoginInput {
        RefreshLoginInput {
            profile_id: profile.id.clone(),
            binding_id: binding.binding_id.clone(),
            generation: profile.generation.clone(),
            expected_revision: profile.revision.clone(),
            publication_id: publication_id.into(),
            format_version: 1,
            sealed_envelope: Zeroizing::new(vec![7; 128]),
        }
    }

    #[tokio::test]
    async fn oracle_login_profile_late_joiner_expiry_and_relogin_preserve_bindings() {
        let (db, pool, keys) = setup().await;
        let a = save(&db, &keys, &pool, "account-a", None, upload(&pool, 1))
            .await
            .unwrap();
        save(&db, &keys, &pool, "account-b", None, upload(&pool, 2))
            .await
            .unwrap();
        assert_eq!(list(&db, &pool.id).await.unwrap().len(), 2);
        let stored = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! { "_id": &a.id })
            .await
            .unwrap()
            .unwrap();
        assert!(stored.get_binary_generic("encrypted_envelope").is_ok());
        assert!(stored.get_datetime("expires_at").is_ok());
        assert!(a.expires_at > Utc::now() + Duration::hours(1));
        let instance = worker(&db, &pool, "late-worker").await;
        assert!(
            current(&db, &pool, "late-worker", &instance)
                .await
                .unwrap()
                .is_none()
        );
        let binding = bind(&db, &pool, "account-a", "late-worker", None, false)
            .await
            .unwrap();
        let (profile, _) = current(&db, &pool, "late-worker", &instance)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            decrypt(&keys, &profile).await.unwrap().as_slice(),
            &[1; 128]
        );
        db.collection::<Document>(COLLECTION_NAME).update_one(doc! { "_id": &a.id },
            doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)) } }).await.unwrap();
        assert_eq!(purge_expired(&db).await.unwrap(), 1);
        let (expired, kept) = current(&db, &pool, "late-worker", &instance)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(availability(&expired, &pool), "expired");
        assert!(expired.encrypted_envelope.is_empty());
        assert_eq!(kept.binding_id, binding.binding_id);
        let fresh = save(
            &db,
            &keys,
            &pool,
            "account-a",
            Some(&a.generation),
            upload(&pool, 3),
        )
        .await
        .unwrap();
        assert_eq!(fresh.id, a.id);
        assert_ne!(fresh.generation, a.generation);
        assert_eq!(fresh.bindings[0].binding_id, binding.binding_id);
        assert_eq!(purge_expired(&db).await.unwrap(), 0);
        assert_eq!(decrypt(&keys, &fresh).await.unwrap().as_slice(), &[3; 128]);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn oracle_login_profile_refresh_fences_siblings_human_generation_and_lost_response() {
        let (db, pool, keys) = setup().await;
        let a = save(&db, &keys, &pool, "account", None, upload(&pool, 1))
            .await
            .unwrap();
        let first = worker(&db, &pool, "first").await;
        let second = worker(&db, &pool, "second").await;
        let first_binding = bind(&db, &pool, "account", "first", None, false)
            .await
            .unwrap();
        let second_binding = bind(&db, &pool, "account", "second", None, false)
            .await
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let refreshed = refresh(
            &db,
            &keys,
            &pool,
            "first",
            &first,
            publication(&a, &first_binding, &id),
        )
        .await
        .unwrap();
        assert_eq!(refreshed.generation, a.generation);
        assert_eq!(refreshed.revision, id);
        let retry = refresh(
            &db,
            &keys,
            &pool,
            "first",
            &first,
            publication(&a, &first_binding, &id),
        )
        .await
        .unwrap();
        assert_eq!(retry.revision, id);
        assert!(matches!(
            refresh(
                &db,
                &keys,
                &pool,
                "second",
                &second,
                publication(&a, &second_binding, &uuid::Uuid::new_v4().to_string())
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        let human = save(
            &db,
            &keys,
            &pool,
            "account",
            Some(&a.generation),
            upload(&pool, 3),
        )
        .await
        .unwrap();
        assert!(matches!(
            refresh(
                &db,
                &keys,
                &pool,
                "first",
                &first,
                publication(
                    &refreshed,
                    &first_binding,
                    &uuid::Uuid::new_v4().to_string()
                )
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            save(
                &db,
                &keys,
                &pool,
                "account",
                Some(&a.generation),
                upload(&pool, 4)
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        let (left, right) = tokio::join!(
            save(
                &db,
                &keys,
                &pool,
                "account",
                Some(&human.generation),
                upload(&pool, 5)
            ),
            save(
                &db,
                &keys,
                &pool,
                "account",
                Some(&human.generation),
                upload(&pool, 6)
            )
        );
        assert_ne!(
            left.is_ok(),
            right.is_ok(),
            "exactly one human replacement can win"
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn oracle_login_profile_binding_switch_rolls_back_and_serializes() {
        let (db, pool, keys) = setup().await;
        let a = save(&db, &keys, &pool, "a", None, upload(&pool, 1))
            .await
            .unwrap();
        let b = save(&db, &keys, &pool, "b", None, upload(&pool, 2))
            .await
            .unwrap();
        let instance = worker(&db, &pool, "worker").await;
        let original = bind(&db, &pool, "a", "worker", None, false).await.unwrap();
        let full = (0..MAX_BINDINGS).map(|i| doc! { "worker_label": format!("full-{i}"),
            "instance_id": uuid::Uuid::new_v4().to_string(), "binding_id": uuid::Uuid::new_v4().to_string(), "replace_existing": false }).collect::<Vec<_>>();
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(doc! { "_id": &b.id }, doc! { "$set": { "bindings": full } })
            .await
            .unwrap();
        assert!(bind(&db, &pool, "b", "worker", None, false).await.is_err());
        let (_, still_bound) = current(&db, &pool, "worker", &instance)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            still_bound.binding_id, original.binding_id,
            "failed switch must roll back removal"
        );
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(doc! { "_id": &b.id }, doc! { "$set": { "bindings": [] } })
            .await
            .unwrap();
        let (left, right) = tokio::join!(
            bind(&db, &pool, "a", "worker", None, false),
            bind(&db, &pool, "b", "worker", None, false)
        );
        left.unwrap();
        right.unwrap();
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! { "bindings.worker_label": "worker" })
                .await
                .unwrap(),
            1
        );
        bind(&db, &pool, "a", "worker", None, false).await.unwrap();
        assert!(matches!(
            refresh(
                &db,
                &keys,
                &pool,
                "worker",
                &instance,
                publication(&a, &original, &uuid::Uuid::new_v4().to_string())
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert!(
            bind(
                &db,
                &pool,
                "a",
                "worker",
                Some(&uuid::Uuid::new_v4().to_string()),
                true
            )
            .await
            .is_err()
        );
        let (forgot, rebinding) = tokio::join!(
            forget_binding_and_worker(&db, &pool.id, "worker"),
            bind(&db, &pool, "b", "worker", None, false)
        );
        forgot.unwrap();
        let _ = rebinding;
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! { "bindings.worker_label": "worker" })
                .await
                .unwrap(),
            0
        );
        assert!(
            oracle_worker_service::get_worker(&db, &pool.id, "worker")
                .await
                .is_err()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn oracle_login_profile_rotation_and_revocation_reject_old_material() {
        let (db, pool, keys) = setup().await;
        let a = save(&db, &keys, &pool, "a", None, upload(&pool, 1))
            .await
            .unwrap();
        let instance = worker(&db, &pool, "worker").await;
        let binding = bind(&db, &pool, "a", "worker", None, false).await.unwrap();
        let (rotated, _) = oracle_pool_service::rotate_worker_token(&db, &pool.user_id, &pool.id)
            .await
            .unwrap();
        let (old, _) = current(&db, &rotated, "worker", &instance)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(availability(&old, &rotated), "token_changed");
        assert!(
            refresh(
                &db,
                &keys,
                &rotated,
                "worker",
                &instance,
                publication(&a, &binding, &uuid::Uuid::new_v4().to_string())
            )
            .await
            .is_err()
        );
        assert!(
            save(
                &db,
                &keys,
                &pool,
                "a",
                Some(&a.generation),
                upload(&pool, 2)
            )
            .await
            .is_err()
        );
        assert!(
            refresh(
                &db,
                &keys,
                &pool,
                "worker",
                &instance,
                publication(&a, &binding, &uuid::Uuid::new_v4().to_string())
            )
            .await
            .is_err()
        );
        let fresh = save(
            &db,
            &keys,
            &rotated,
            "a",
            Some(&a.generation),
            upload(&rotated, 3),
        )
        .await
        .unwrap();
        assert_eq!(fresh.bindings[0].binding_id, binding.binding_id);
        delete(&db, &pool.id, "a").await.unwrap();
        assert!(
            current(&db, &rotated, "worker", &instance)
                .await
                .unwrap()
                .is_none()
        );
        let replacement = save(&db, &keys, &rotated, "a", None, upload(&rotated, 4))
            .await
            .unwrap();
        assert_ne!(replacement.id, a.id);
        assert!(replacement.bindings.is_empty());
        db.drop().await.unwrap();
    }
}
