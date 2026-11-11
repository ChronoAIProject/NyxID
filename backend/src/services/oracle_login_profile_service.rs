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
    let cutoff = bson::DateTime::now();
    let mut after = None;
    let mut total = 0;
    loop {
        let (changed, scanned, last) = purge_expired_batch(db, cutoff, after.as_deref()).await?;
        total += changed;
        if scanned < 256 {
            return Ok(total);
        }
        after = Some(
            last.ok_or_else(|| AppError::Internal("invalid saved-login cleanup cursor".into()))?,
        );
        tokio::task::yield_now().await;
    }
}

async fn purge_expired_batch(
    db: &mongodb::Database,
    cutoff: bson::DateTime,
    after: Option<&str>,
) -> AppResult<(u64, usize, Option<String>)> {
    let mut filter = doc! {"expires_at":{"$lte":cutoff},"envelope_size":{"$gt":0_i64}};
    if let Some(after) = after {
        filter.insert("_id", doc! {"$gt":after});
    }
    let rows: Vec<Document> = db
        .collection::<Document>(COLLECTION_NAME)
        .find(filter)
        .projection(doc! {"_id":1,"pool_id":1})
        .sort(doc! {"_id":1})
        .limit(256)
        .await?
        .try_collect()
        .await?;
    let scanned = rows.len();
    let last = rows
        .last()
        .and_then(|row| row.get_str("_id").ok())
        .map(str::to_owned);
    let mut changed = 0;
    for row in rows {
        let (Ok(id), Ok(pool_id)) = (row.get_str("_id"), row.get_str("pool_id")) else {
            continue;
        };
        let transaction_db = db.clone();
        let id = id.to_owned();
        let pool_id = pool_id.to_owned();
        let mut session = db.client().start_session().await?;
        changed += session.start_transaction().and_run((transaction_db, id, pool_id), |session, (db, id, pool_id)| Box::pin(async move {
            let db = &*db;
            let id = id.as_str();
            let pool_id = pool_id.as_str();
            let result: AppResult<u64> = async {
                // Deletion owns physical removal; expiry can also clear envelopes
                // under the same retained pool fence while cleanup is pending.
                let pool = super::oracle_mutation_service::lookup_pool(db, session, pool_id, true).await?;
                if pool.deleted_at.is_some() {
                    db.collection::<Document>(crate::models::oracle_pool::COLLECTION_NAME).update_one(doc! {"_id":pool_id}, doc! {"$inc":{"worker_enrollment_epoch":1_i64}}).session(&mut *session).await?;
                } else {
                    super::oracle_mutation_service::lock_pool(db, session, pool_id).await?;
                }
                Ok(db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":id,"expires_at":{"$lte":bson::DateTime::now()},"envelope_size":{"$gt":0_i64}},doc! {"$set":{"encrypted_envelope":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![]},"envelope_size":0_i64}}).session(session).await?.modified_count)
            }.await;
            transactions::transaction_result(result)
        })).await.map_err(transactions::map_transaction_error)?;
    }
    Ok((changed, scanned, last))
}

fn document(value: &impl serde::Serialize) -> AppResult<Document> {
    bson::to_document(value)
        .map_err(|_| AppError::Internal("saved login serialization failed".into()))
}

pub(super) async fn check_pool_in_session(
    db: &mongodb::Database,
    mongo_session: &mut mongodb::ClientSession,
    pool: &OraclePool,
) -> AppResult<()> {
    let live =
        super::oracle_mutation_service::lookup_pool(db, mongo_session, &pool.id, false).await?;
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

pub(crate) fn aad(profile: &OracleLoginProfile) -> String {
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

#[cfg(test)]
pub async fn save(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    pool: &OraclePool,
    name: &str,
    expected_generation: Option<&str>,
    input: CreateLoginSnapshotInput,
) -> AppResult<OracleLoginProfile> {
    let transaction_db = db.clone();
    let transaction_pool = pool.clone();
    let transaction_name = name.to_owned();
    let transaction_expected_generation = expected_generation.map(str::to_owned);
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                keys,
                transaction_pool,
                transaction_name,
                transaction_expected_generation,
                input,
            ),
            |mongo_session,
             (
                transaction_db,
                keys,
                transaction_pool,
                transaction_name,
                transaction_expected_generation,
                input,
            )| {
                Box::pin(async move {
                    let keys = *keys;
                    let input = &*input;
                    let db = &*transaction_db;
                    let pool = &*transaction_pool;
                    let name = transaction_name.as_str();
                    let expected_generation = transaction_expected_generation.as_deref();
                    crate::services::api_key_mutation_service::transaction_result(
                        save_in_session(
                            db,
                            mongo_session,
                            keys,
                            pool,
                            name,
                            expected_generation,
                            input.clone(),
                        )
                        .await,
                    )
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn save_in_session(
    db: &mongodb::Database,
    mongo_session: &mut mongodb::ClientSession,
    keys: &EncryptionKeys,
    pool: &OraclePool,
    name: &str,
    expected_generation: Option<&str>,
    input: CreateLoginSnapshotInput,
) -> AppResult<OracleLoginProfile> {
    crate::services::oracle_mutation_service::lock_expected_pool(db, mongo_session, pool, true)
        .await?;

    validate_name(name)?;
    validate_envelope(input.format_version, &input.sealed_envelope)?;
    verify_worker_token_hash(pool, &input.worker_token_sha256)?;
    let collection = db.collection::<OracleLoginProfile>(COLLECTION_NAME);
    let prior = collection
        .find_one(doc! { "pool_id": &pool.id, "name": name })
        .session(&mut *mongo_session)
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
    check_pool_in_session(db, mongo_session, pool).await?;
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
            .session(&mut *mongo_session)
            .await?
            .ok_or_else(conflict)
    } else {
        match collection
            .insert_one(&profile)
            .session(&mut *mongo_session)
            .await
        {
            Ok(_) => Ok(profile),
            Err(error) if oracle_pool_service::is_duplicate_key(&error) => Err(conflict()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
pub async fn delete(db: &mongodb::Database, pool: &OraclePool, name: &str) -> AppResult<()> {
    let transaction_db = db.clone();
    let transaction_pool = pool.clone();
    let transaction_name = name.to_owned();
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (transaction_db, transaction_pool, transaction_name),
            |mongo_session, (transaction_db, transaction_pool, transaction_name)| {
                Box::pin(async move {
                    let db = &*transaction_db;
                    let pool = &*transaction_pool;
                    let name = transaction_name.as_str();
                    crate::services::api_key_mutation_service::transaction_result(
                        delete_in_session(db, mongo_session, pool, name).await,
                    )
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn delete_in_session(
    db: &mongodb::Database,
    mongo_session: &mut mongodb::ClientSession,
    pool: &OraclePool,
    name: &str,
) -> AppResult<()> {
    crate::services::oracle_mutation_service::lock_expected_pool(db, mongo_session, pool, false)
        .await?;
    let pool_id = pool.id.as_str();

    validate_name(name)?;
    db.collection::<OracleLoginProfile>(COLLECTION_NAME)
        .delete_one(doc! { "pool_id": pool_id, "name": name })
        .session(&mut *mongo_session)
        .await?;
    Ok(())
}

pub(crate) async fn unbind_in_session(
    db: &mongodb::Database,
    mongo_session: &mut mongodb::ClientSession,
    pool: &OraclePool,
    label: &str,
) -> AppResult<()> {
    crate::services::oracle_mutation_service::lock_expected_pool(db, mongo_session, pool, false)
        .await?;
    let pool_id = pool.id.as_str();

    remove_binding_in_session(db, mongo_session, pool_id, label, false).await
}

#[cfg(test)]
pub async fn forget_binding_and_worker(
    db: &mongodb::Database,
    pool_id: &str,
    label: &str,
) -> AppResult<()> {
    let transaction_db = db.clone();
    let transaction_pool_id = pool_id.to_owned();
    let transaction_label = label.to_owned();
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (transaction_db, transaction_pool_id, transaction_label),
            |mongo_session, (transaction_db, transaction_pool_id, transaction_label)| {
                Box::pin(async move {
                    let db = &*transaction_db;
                    let pool_id = transaction_pool_id.as_str();
                    let label = transaction_label.as_str();
                    crate::services::api_key_mutation_service::transaction_result(
                        forget_binding_and_worker_in_session(db, mongo_session, pool_id, label)
                            .await,
                    )
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

#[cfg(test)]
pub(crate) async fn forget_binding_and_worker_in_session(
    db: &mongodb::Database,
    mongo_session: &mut mongodb::ClientSession,
    pool_id: &str,
    label: &str,
) -> AppResult<()> {
    crate::services::oracle_mutation_service::lock_pool(db, mongo_session, pool_id).await?;

    remove_binding_in_session(db, mongo_session, pool_id, label, true).await
}

pub(super) async fn remove_binding_in_session(
    db: &mongodb::Database,
    mongo_session: &mut mongodb::ClientSession,
    pool_id: &str,
    label: &str,
    forget: bool,
) -> AppResult<()> {
    db.collection::<Document>(WORKERS)
        .update_one(
            doc! { "_id": worker_doc_id(pool_id, label) },
            doc! { "$inc": { "login_binding_epoch": 1_i64 } },
        )
        .session(&mut *mongo_session)
        .await?;
    db.collection::<Document>(COLLECTION_NAME)
        .update_many(
            doc! { "pool_id": &pool_id, "bindings.worker_label": &label },
            doc! { "$pull": { "bindings": { "worker_label": &label } } },
        )
        .session(&mut *mongo_session)
        .await?;
    if forget {
        db.collection::<Document>(WORKERS)
            .delete_one(doc! { "_id": worker_doc_id(pool_id, label) })
            .session(&mut *mongo_session)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
pub async fn bind(
    db: &mongodb::Database,
    pool: &OraclePool,
    name: &str,
    label: &str,
    installation_id: Option<&str>,
    replace_existing: bool,
) -> AppResult<OracleLoginBinding> {
    let transaction_db = db.clone();
    let transaction_pool = pool.clone();
    let transaction_name = name.to_owned();
    let transaction_label = label.to_owned();
    let transaction_installation_id = installation_id.map(str::to_owned);
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                transaction_pool,
                transaction_name,
                transaction_label,
                transaction_installation_id,
                replace_existing,
            ),
            |mongo_session,
             (
                transaction_db,
                transaction_pool,
                transaction_name,
                transaction_label,
                transaction_installation_id,
                replace_existing,
            )| {
                Box::pin(async move {
                    let replace_existing = *replace_existing;
                    let db = &*transaction_db;
                    let pool = &*transaction_pool;
                    let name = transaction_name.as_str();
                    let label = transaction_label.as_str();
                    let installation_id = transaction_installation_id.as_deref();
                    crate::services::api_key_mutation_service::transaction_result(
                        bind_in_session(
                            db,
                            mongo_session,
                            pool,
                            name,
                            label,
                            installation_id,
                            replace_existing,
                        )
                        .await,
                    )
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn bind_in_session(
    db: &mongodb::Database,
    mongo_session: &mut mongodb::ClientSession,
    pool: &OraclePool,
    name: &str,
    label: &str,
    installation_id: Option<&str>,
    replace_existing: bool,
) -> AppResult<OracleLoginBinding> {
    crate::services::oracle_mutation_service::lock_expected_pool(db, mongo_session, pool, true)
        .await?;

    validate_name(name)?;
    let worker =
        oracle_worker_service::get_worker_in_session(db, mongo_session, &pool.id, label).await?;
    if worker.enrollment.is_some() {
        return Err(AppError::OracleWorkerCapabilityUnsupported(
            "contributed workers use their own browser login".into(),
        ));
    }
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
        .session(&mut *mongo_session)
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

    // Serialize all account switches for this worker, including ABA rebinds.
    let locked = db
        .collection::<Document>(WORKERS)
        .update_one(
            doc! { "_id": &worker.id, "instance_id": &worker.instance_id, "enrollment": null },
            doc! { "$inc": { "login_binding_epoch": 1_i64 } },
        )
        .session(&mut *mongo_session)
        .await?;
    if locked.matched_count != 1 {
        return Err(conflict());
    }
    db.collection::<Document>(COLLECTION_NAME)
        .update_many(
            doc! { "pool_id": &pool.id, "bindings.worker_label": &label },
            doc! { "$pull": { "bindings": { "worker_label": &label } } },
        )
        .session(&mut *mongo_session)
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
        .session(&mut *mongo_session)
        .await?;
    if result.modified_count != 1 {
        return Err(conflict());
    }

    Ok(binding)
}

pub(super) async fn validate_worker_in_session(
    db: &mongodb::Database,
    mongo_session: &mut mongodb::ClientSession,
    pool: &OraclePool,
    label: &str,
    instance: &str,
) -> AppResult<()> {
    let worker =
        oracle_worker_service::get_worker_in_session(db, mongo_session, &pool.id, label).await?;
    if worker.enrollment.is_some() {
        super::oracle_worker_enrollment_service::ensure_current_worker_in_session(
            db,
            mongo_session,
            &worker,
        )
        .await?;
    }
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
    let transaction_db = db.clone();
    let transaction_pool = pool.clone();
    let transaction_label = label.to_owned();
    let transaction_instance = instance.to_owned();
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                transaction_pool,
                transaction_label,
                transaction_instance,
            ),
            |mongo_session, context| {
                let (transaction_db, transaction_pool, transaction_label, transaction_instance) =
                    context;
                Box::pin(async move {
                    let db = &*transaction_db;
                    let pool = &*transaction_pool;
                    let label = transaction_label.as_str();
                    let instance = transaction_instance.as_str();
                    crate::services::api_key_mutation_service::transaction_result(
                        current_in_session(db, mongo_session, pool, label, instance).await,
                    )
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn current_in_session(
    db: &mongodb::Database,
    mongo_session: &mut mongodb::ClientSession,
    pool: &OraclePool,
    label: &str,
    instance: &str,
) -> AppResult<Option<(OracleLoginProfile, OracleLoginBinding)>> {
    crate::services::oracle_mutation_service::lock_expected_pool(db, mongo_session, pool, true)
        .await?;

    validate_worker_in_session(db, mongo_session, pool, label, instance).await?;
    let profile = db
        .collection::<OracleLoginProfile>(COLLECTION_NAME)
        .find_one(doc! {
            "pool_id": &pool.id,
            "bindings": { "$elemMatch": { "worker_label": label, "instance_id": instance } },
        })
        .session(&mut *mongo_session)
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
    super::oracle_login_record_service::decrypt(
        keys,
        COLLECTION_NAME,
        &bson::to_document(profile)
            .map_err(|_| AppError::Internal("Invalid Oracle login envelope".into()))?,
    )
    .await
}

#[derive(Clone)]
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
    let transaction_db = db.clone();
    let transaction_pool = pool.clone();
    let transaction_label = label.to_owned();
    let transaction_instance = instance.to_owned();
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                keys,
                transaction_pool,
                transaction_label,
                transaction_instance,
                input,
            ),
            |mongo_session,
             (
                transaction_db,
                keys,
                transaction_pool,
                transaction_label,
                transaction_instance,
                input,
            )| {
                Box::pin(async move {
                    let keys = *keys;
                    let input = &*input;
                    let db = &*transaction_db;
                    let pool = &*transaction_pool;
                    let label = transaction_label.as_str();
                    let instance = transaction_instance.as_str();
                    crate::services::api_key_mutation_service::transaction_result(
                        refresh_in_session(
                            db,
                            mongo_session,
                            keys,
                            pool,
                            label,
                            instance,
                            input.clone(),
                        )
                        .await,
                    )
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn refresh_in_session(
    db: &mongodb::Database,
    mongo_session: &mut mongodb::ClientSession,
    keys: &EncryptionKeys,
    pool: &OraclePool,
    label: &str,
    instance: &str,
    input: RefreshLoginInput,
) -> AppResult<OracleLoginProfile> {
    crate::services::oracle_mutation_service::lock_expected_pool(db, mongo_session, pool, true)
        .await?;

    validate_worker_in_session(db, mongo_session, pool, label, instance).await?;
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
        .session(&mut *mongo_session)
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
    check_pool_in_session(db, mongo_session, pool).await?;
    let mut update = document(&profile)?;
    for key in ["_id", "bindings", "created_at"] {
        update.remove(key);
    }
    collection
        .find_one_and_update(filter, doc! { "$set": update })
        .return_document(ReturnDocument::After)
        .session(&mut *mongo_session)
        .await?
        .ok_or_else(conflict)
}

pub async fn bind_for_manager(
    db: &mongodb::Database,
    actor_user_id: &str,
    pool: &OraclePool,
    name: &str,
    label: &str,
    installation_id: Option<&str>,
    replace_existing: bool,
) -> AppResult<OracleLoginBinding> {
    let transaction_db = db.clone();
    let transaction_actor_user_id = actor_user_id.to_owned();
    let transaction_pool = pool.clone();
    let transaction_name = name.to_owned();
    let transaction_label = label.to_owned();
    let transaction_installation_id = installation_id.map(str::to_owned);
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                transaction_actor_user_id,
                transaction_pool,
                transaction_name,
                transaction_label,
                transaction_installation_id,
                replace_existing,
            ),
            |mongo_session,
             (
                transaction_db,
                transaction_actor_user_id,
                transaction_pool,
                transaction_name,
                transaction_label,
                transaction_installation_id,
                replace_existing,
            )| {
                Box::pin(async move {
                    let replace_existing = *replace_existing;
                    let db = &*transaction_db;
                    let actor_user_id = transaction_actor_user_id.as_str();
                    let pool = &*transaction_pool;
                    let name = transaction_name.as_str();
                    let label = transaction_label.as_str();
                    let installation_id = transaction_installation_id.as_deref();
                    let result = async {
                        crate::services::oracle_mutation_service::lock_manager(
                            db,
                            mongo_session,
                            actor_user_id,
                            &pool.user_id,
                        )
                        .await?;
                        bind_in_session(
                            db,
                            mongo_session,
                            pool,
                            name,
                            label,
                            installation_id,
                            replace_existing,
                        )
                        .await
                    }
                    .await;
                    crate::services::api_key_mutation_service::transaction_result(result)
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub async fn unbind_for_manager(
    db: &mongodb::Database,
    actor_user_id: &str,
    pool: &OraclePool,
    label: &str,
) -> AppResult<()> {
    let transaction_db = db.clone();
    let transaction_actor_user_id = actor_user_id.to_owned();
    let transaction_pool = pool.clone();
    let transaction_label = label.to_owned();
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                transaction_actor_user_id,
                transaction_pool,
                transaction_label,
            ),
            |mongo_session, context| {
                let (
                    transaction_db,
                    transaction_actor_user_id,
                    transaction_pool,
                    transaction_label,
                ) = context;
                Box::pin(async move {
                    let db = &*transaction_db;
                    let actor_user_id = transaction_actor_user_id.as_str();
                    let pool = &*transaction_pool;
                    let label = transaction_label.as_str();
                    let result = async {
                        crate::services::oracle_mutation_service::lock_manager(
                            db,
                            mongo_session,
                            actor_user_id,
                            &pool.user_id,
                        )
                        .await?;
                        unbind_in_session(db, mongo_session, pool, label).await
                    }
                    .await;
                    crate::services::api_key_mutation_service::transaction_result(result)
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub async fn delete_for_manager(
    db: &mongodb::Database,
    actor_user_id: &str,
    pool: &OraclePool,
    name: &str,
) -> AppResult<()> {
    let transaction_db = db.clone();
    let transaction_actor_user_id = actor_user_id.to_owned();
    let transaction_pool = pool.clone();
    let transaction_name = name.to_owned();
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                transaction_actor_user_id,
                transaction_pool,
                transaction_name,
            ),
            |mongo_session, context| {
                let (transaction_db, transaction_actor_user_id, transaction_pool, transaction_name,) = context;
            Box::pin(async move {
            let db = &*transaction_db;
            let actor_user_id = transaction_actor_user_id.as_str();
            let pool = &*transaction_pool;
            let name = transaction_name.as_str();
            let result = async {
                crate::services::oracle_mutation_service::lock_manager(
                    db,
                    mongo_session,
                    actor_user_id,
                    &pool.user_id,
                )
                .await?;
                delete_in_session(db, mongo_session, pool, name).await
            }
            .await;
            crate::services::api_key_mutation_service::transaction_result(result)
            })
        },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub async fn save_for_manager(
    db: &mongodb::Database,
    actor_user_id: &str,
    keys: &EncryptionKeys,
    pool: &OraclePool,
    name: &str,
    expected_generation: Option<&str>,
    input: CreateLoginSnapshotInput,
) -> AppResult<OracleLoginProfile> {
    let transaction_db = db.clone();
    let transaction_actor_user_id = actor_user_id.to_owned();
    let transaction_pool = pool.clone();
    let transaction_name = name.to_owned();
    let transaction_expected_generation = expected_generation.map(str::to_owned);
    let mut mongo_session = db.client().start_session().await?;
    mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                transaction_actor_user_id,
                keys,
                transaction_pool,
                transaction_name,
                transaction_expected_generation,
                input,
            ),
            |mongo_session,
             (
                transaction_db,
                transaction_actor_user_id,
                keys,
                transaction_pool,
                transaction_name,
                transaction_expected_generation,
                input,
            )| {
                Box::pin(async move {
                    let keys = *keys;
                    let input = &*input;
                    let db = &*transaction_db;
                    let actor_user_id = transaction_actor_user_id.as_str();
                    let pool = &*transaction_pool;
                    let name = transaction_name.as_str();
                    let expected_generation = transaction_expected_generation.as_deref();
                    let result = async {
                        crate::services::oracle_mutation_service::lock_manager(
                            db,
                            mongo_session,
                            actor_user_id,
                            &pool.user_id,
                        )
                        .await?;
                        save_in_session(
                            db,
                            mongo_session,
                            keys,
                            pool,
                            name,
                            expected_generation,
                            input.clone(),
                        )
                        .await
                    }
                    .await;
                    crate::services::api_key_mutation_service::transaction_result(result)
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
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
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &pool.user_id,
                crate::models::user::UserType::Person,
            ))
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
        assert!(matches!(
            delete(&db, &pool, "a").await,
            Err(AppError::Conflict(_))
        ));
        delete(&db, &rotated, "a").await.unwrap();
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
    #[tokio::test]
    async fn oracle_profile_expiry_drains_multiple_bounded_batches() {
        let (db, pool, _) = setup().await;
        let expired = bson::DateTime::from_chrono(Utc::now() - Duration::days(1));
        db.collection::<Document>(COLLECTION_NAME).insert_many((0..300).map(|n| doc! {
            "_id":format!("profile-{n:04}"),"pool_id":&pool.id,"name":format!("profile-{n}"),
            "expires_at":expired,"envelope_size":128_i64,"encrypted_envelope":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1;128]},
        })).await.unwrap();
        assert_eq!(purge_expired(&db).await.unwrap(), 300);
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {"envelope_size":{"$gt":0_i64}})
                .await
                .unwrap(),
            0
        );
    }
}
