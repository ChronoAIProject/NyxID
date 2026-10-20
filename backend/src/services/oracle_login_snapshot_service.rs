use std::fmt;

use chrono::{Duration, Utc};
use futures::TryStreamExt;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::oracle_login_snapshot::{
    COLLECTION_NAME as ORACLE_LOGIN_SNAPSHOTS, OracleLoginSnapshot,
};
use crate::models::oracle_pool::OraclePool;
use crate::models::oracle_worker_command::OracleWorkerCommandKind;
use crate::services::oracle_worker_service;

pub const LOGIN_SNAPSHOT_FORMAT_VERSION: u32 = 1;
pub const MAX_LOGIN_SNAPSHOT_BYTES: usize = 512 * 1024;
pub const MAX_LOGIN_SNAPSHOT_BASE64_CHARS: usize = MAX_LOGIN_SNAPSHOT_BYTES.div_ceil(3) * 4;
pub const LOGIN_SNAPSHOT_TTL_SECS: i64 = 60 * 60;

#[derive(Clone)]
pub struct CreateLoginSnapshotInput {
    pub format_version: u32,
    pub worker_token_sha256: Zeroizing<String>,
    pub sealed_envelope: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for CreateLoginSnapshotInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateLoginSnapshotInput")
            .field("format_version", &self.format_version)
            .field("worker_token_sha256", &"[REDACTED]")
            .field(
                "sealed_envelope",
                &format!("[REDACTED; {} bytes]", self.sealed_envelope.len()),
            )
            .finish()
    }
}

#[derive(Debug)]
pub struct LoginSnapshotFanout {
    pub snapshot_id: String,
    pub envelope_size: u64,
    pub expires_at: chrono::DateTime<Utc>,
    pub queued_workers: Vec<(String, String)>,
    pub skipped_workers: Vec<String>,
}

pub struct LoginSnapshotPayload {
    pub format_version: u32,
    pub sealed_envelope: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for LoginSnapshotPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginSnapshotPayload")
            .field("format_version", &self.format_version)
            .field(
                "sealed_envelope",
                &format!("[REDACTED; {} bytes]", self.sealed_envelope.len()),
            )
            .finish()
    }
}

pub(crate) fn snapshot_aad(pool_id: &str, snapshot_id: &str, format_version: u32) -> String {
    format!("oracle-login-snapshot:{pool_id}:{snapshot_id}:v{format_version}")
}

pub(super) fn verify_worker_token_hash(pool: &OraclePool, verifier: &str) -> AppResult<()> {
    let valid_shape = verifier.len() == 64
        && verifier
            .bytes()
            .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase());
    if !valid_shape
        || pool
            .worker_token_hash
            .as_bytes()
            .ct_eq(verifier.as_bytes())
            .unwrap_u8()
            != 1
    {
        return Err(AppError::OracleWorkerTokenInvalid);
    }
    Ok(())
}

pub async fn create_and_fanout(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    pool: &OraclePool,
    actor_user_id: &str,
    input: CreateLoginSnapshotInput,
) -> AppResult<LoginSnapshotFanout> {
    let transaction_db = db.clone();
    let transaction_pool = pool.clone();
    let transaction_actor_user_id = actor_user_id.to_owned();
    let mut mongo_session = db.client().start_session().await?;
    let snapshot = mongo_session
        .start_transaction()
        .and_run(
            (
                transaction_db,
                encryption_keys,
                transaction_pool,
                transaction_actor_user_id,
                input,
            ),
            |mongo_session,
             (
                transaction_db,
                encryption_keys,
                transaction_pool,
                transaction_actor_user_id,
                input,
            )| {
                Box::pin(async move {
                    let encryption_keys = *encryption_keys;
                    let input = &*input;
                    let db = &*transaction_db;
                    let pool = &*transaction_pool;
                    let actor_user_id = transaction_actor_user_id.as_str();
                    crate::services::api_key_mutation_service::transaction_result(
                        create_snapshot_in_session(
                            db,
                            mongo_session,
                            encryption_keys,
                            pool,
                            actor_user_id,
                            input.clone(),
                        )
                        .await,
                    )
                })
            },
        )
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    let mut outcome = LoginSnapshotFanout {
        snapshot_id: snapshot.id.clone(),
        envelope_size: snapshot.envelope_size,
        expires_at: snapshot.expires_at,
        queued_workers: Vec::new(),
        skipped_workers: Vec::new(),
    };
    // Worker inventory is unbounded. Stream it outside the snapshot transaction;
    // each command revalidates the expected pool, manager and worker generation
    // in its own transaction. Earlier fanout remains committed on later failure.
    let mut workers = db
        .collection::<crate::models::oracle_worker::OracleWorker>(
            crate::models::oracle_worker::COLLECTION_NAME,
        )
        .find(bson::doc! {"pool_id": &pool.id})
        .sort(bson::doc! {"_id": 1})
        .batch_size(64)
        .await?;
    while let Some(worker) = workers.try_next().await? {
        let capable = worker.capabilities.iter().any(|c| c == "commands_v1")
            && worker.capabilities.iter().any(|c| c == "session_import_v1");
        if !capable || worker.enrollment.is_some() {
            outcome.skipped_workers.push(worker.worker_label);
            continue;
        }
        let command = oracle_worker_service::enqueue_worker_command(
            db,
            pool,
            &worker,
            actor_user_id,
            OracleWorkerCommandKind::SessionImport,
            Some(snapshot.id.clone()),
            None,
        )
        .await?;
        outcome
            .queued_workers
            .push((worker.worker_label, command.id));
    }
    Ok(outcome)
}

pub(crate) async fn create_snapshot_in_session(
    db: &mongodb::Database,
    mongo_session: &mut mongodb::ClientSession,
    encryption_keys: &EncryptionKeys,
    pool: &OraclePool,
    actor_user_id: &str,
    input: CreateLoginSnapshotInput,
) -> AppResult<OracleLoginSnapshot> {
    crate::services::oracle_mutation_service::lock_expected_pool(db, mongo_session, pool, true)
        .await?;

    crate::services::oracle_mutation_service::lock_manager(
        db,
        mongo_session,
        actor_user_id,
        &pool.user_id,
    )
    .await?;
    if input.format_version != LOGIN_SNAPSHOT_FORMAT_VERSION {
        return Err(AppError::ValidationError(format!(
            "login snapshot format_version must be {LOGIN_SNAPSHOT_FORMAT_VERSION}"
        )));
    }
    if input.sealed_envelope.is_empty() || input.sealed_envelope.len() > MAX_LOGIN_SNAPSHOT_BYTES {
        return Err(AppError::OraclePayloadTooLarge(format!(
            "login snapshot must be 1-{MAX_LOGIN_SNAPSHOT_BYTES} bytes"
        )));
    }
    verify_worker_token_hash(pool, input.worker_token_sha256.as_str())?;

    let id = uuid::Uuid::new_v4().to_string();
    let aad = snapshot_aad(&pool.id, &id, input.format_version);
    let encrypted_envelope = encryption_keys
        .encrypt_with_aad(input.sealed_envelope.as_slice(), aad.as_bytes())
        .await?;
    let now = Utc::now();
    let expires_at = now + Duration::seconds(LOGIN_SNAPSHOT_TTL_SECS);
    let snapshot = OracleLoginSnapshot {
        id: id.clone(),
        pool_id: pool.id.clone(),
        format_version: input.format_version,
        encrypted_envelope,
        envelope_size: input.sealed_envelope.len() as u64,
        created_by_user_id: actor_user_id.to_string(),
        created_at: now,
        expires_at,
    };
    db.collection::<OracleLoginSnapshot>(ORACLE_LOGIN_SNAPSHOTS)
        .insert_one(&snapshot)
        .session(&mut *mongo_session)
        .await?;

    Ok(snapshot)
}

pub async fn fetch_for_worker(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    pool_id: &str,
    snapshot_id: &str,
) -> AppResult<LoginSnapshotPayload> {
    let snapshot = db
        .collection::<OracleLoginSnapshot>(ORACLE_LOGIN_SNAPSHOTS)
        .find_one(bson::doc! {
            "_id": snapshot_id,
            "pool_id": pool_id,
            "expires_at": { "$gt": bson::DateTime::from_chrono(Utc::now()) },
        })
        .await?
        .ok_or_else(|| AppError::OracleLoginSnapshotNotFound(snapshot_id.to_string()))?;
    let sealed_envelope = super::oracle_login_record_service::decrypt(
        encryption_keys,
        ORACLE_LOGIN_SNAPSHOTS,
        &bson::to_document(&snapshot)
            .map_err(|_| AppError::Internal("Invalid Oracle login envelope".into()))?,
    )
    .await?;
    Ok(LoginSnapshotPayload {
        format_version: snapshot.format_version,
        sealed_envelope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::oracle_pool::OraclePoolVisibility;
    use crate::services::oracle_worker_service::WorkerPresenceInput;
    use crate::test_utils::{connect_test_database, test_encryption_keys};

    async fn pool(db: &mongodb::Database) -> OraclePool {
        oracle_worker_service::ensure_test_command_indexes(db).await;
        let now = Utc::now();
        let pool = OraclePool {
            deleted_at: None,
            lifecycle_version: 0,
            installation_epoch: 0,
            deletion_cleanup_pending: false,
            require_model_match: true,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            slug: format!("login-test-{}", uuid::Uuid::new_v4()),
            name: "Login test".to_string(),
            description: None,
            visibility: OraclePoolVisibility::Private,
            worker_token_hash: "a".repeat(64),
            chatgpt_project_url: None,
            default_model_label: None,
            allow_extract: false,
            max_workers: 3,
            max_queue_length: 10,
            per_user_max_inflight: 2,
            task_timeout_secs: 60,
            is_active: true,
            created_at: now,
            updated_at: now,
        };
        db.collection::<OraclePool>(crate::models::oracle_pool::COLLECTION_NAME)
            .insert_one(&pool)
            .await
            .unwrap();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &pool.user_id,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn snapshot_roundtrips_and_fanout_is_capability_gated() {
        let Some(db) = connect_test_database("oracle_login_snapshot").await else {
            return;
        };
        let pool = pool(&db).await;
        let capable = oracle_worker_service::allocate_worker(&db, &pool, None)
            .await
            .unwrap()
            .worker;
        oracle_worker_service::report_presence(
            &db,
            &pool,
            WorkerPresenceInput {
                worker_label: capable.worker_label.clone(),
                capabilities: vec!["commands_v1".to_string(), "session_import_v1".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let legacy = oracle_worker_service::allocate_worker(&db, &pool, None)
            .await
            .unwrap()
            .worker;
        let keys = test_encryption_keys();
        let fanout = create_and_fanout(
            &db,
            &keys,
            &pool,
            &pool.user_id,
            CreateLoginSnapshotInput {
                format_version: 1,
                worker_token_sha256: Zeroizing::new("a".repeat(64)),
                sealed_envelope: Zeroizing::new(vec![1, 2, 3, 4]),
            },
        )
        .await
        .unwrap();
        assert_eq!(fanout.queued_workers.len(), 1);
        assert_eq!(fanout.queued_workers[0].0, capable.worker_label);
        assert_eq!(fanout.skipped_workers, vec![legacy.worker_label]);
        let payload = fetch_for_worker(&db, &keys, &pool.id, &fanout.snapshot_id)
            .await
            .unwrap();
        assert_eq!(payload.format_version, 1);
        assert_eq!(payload.sealed_envelope.as_slice(), &[1, 2, 3, 4]);
        db.collection::<OracleLoginSnapshot>(ORACLE_LOGIN_SNAPSHOTS)
            .update_one(
                bson::doc! { "_id": &fanout.snapshot_id },
                bson::doc! { "$set": {
                    "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))
                } },
            )
            .await
            .unwrap();
        let expired = fetch_for_worker(&db, &keys, &pool.id, &fanout.snapshot_id)
            .await
            .unwrap_err();
        assert!(matches!(expired, AppError::OracleLoginSnapshotNotFound(_)));
        db.drop().await.ok();
    }

    #[tokio::test]
    async fn wrong_token_verifier_is_rejected_before_storage() {
        let Some(db) = connect_test_database("oracle_login_snapshot_token").await else {
            return;
        };
        let pool = pool(&db).await;
        let error = create_and_fanout(
            &db,
            &test_encryption_keys(),
            &pool,
            &pool.user_id,
            CreateLoginSnapshotInput {
                format_version: 1,
                worker_token_sha256: Zeroizing::new("b".repeat(64)),
                sealed_envelope: Zeroizing::new(vec![1, 2, 3]),
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::OracleWorkerTokenInvalid));
        assert_eq!(
            db.collection::<OracleLoginSnapshot>(ORACLE_LOGIN_SNAPSHOTS)
                .count_documents(bson::doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.ok();
    }

    #[tokio::test]
    async fn oracle_snapshot_fanout_keeps_prior_commits_on_later_worker_failure() {
        let db =
            crate::test_utils::connect_transaction_test_database("oracle_snapshot_partial").await;
        let pool = pool(&db).await;
        for label in ["a-first", "z-reject"] {
            oracle_worker_service::report_presence(
                &db,
                &pool,
                WorkerPresenceInput {
                    worker_label: label.into(),
                    capabilities: vec!["commands_v1".into(), "session_import_v1".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        let commands = crate::models::oracle_worker_command::COLLECTION_NAME;
        db.run_command(bson::doc! {"collMod":commands,"validator":{"worker_label":{"$ne":"z-reject"}},"validationLevel":"strict"}).await.unwrap();
        let keys = test_encryption_keys();
        assert!(
            create_and_fanout(
                &db,
                &keys,
                &pool,
                &pool.user_id,
                CreateLoginSnapshotInput {
                    format_version: 1,
                    worker_token_sha256: Zeroizing::new("a".repeat(64)),
                    sealed_envelope: Zeroizing::new(vec![1, 2, 3]),
                }
            )
            .await
            .is_err()
        );
        let snapshots = db.collection::<OracleLoginSnapshot>(ORACLE_LOGIN_SNAPSHOTS);
        assert_eq!(snapshots.count_documents(bson::doc! {}).await.unwrap(), 1);
        let snapshot = snapshots.find_one(bson::doc! {}).await.unwrap().unwrap();
        let rows = db.collection::<bson::Document>(commands);
        assert_eq!(
            rows.count_documents(bson::doc! {"worker_label":"a-first","snapshot_id":&snapshot.id})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            rows.count_documents(bson::doc! {"worker_label":"z-reject"})
                .await
                .unwrap(),
            0
        );
        let payload = fetch_for_worker(&db, &keys, &pool.id, &snapshot.id)
            .await
            .unwrap();
        assert_eq!(payload.sealed_envelope.as_slice(), &[1, 2, 3]);
        db.drop().await.unwrap();
    }
}
