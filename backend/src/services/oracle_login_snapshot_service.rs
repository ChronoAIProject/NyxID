use std::fmt;

use chrono::{Duration, Utc};
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
pub const LOGIN_SNAPSHOT_TTL_SECS: i64 = 60 * 60;

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

fn snapshot_aad(pool_id: &str, snapshot_id: &str, format_version: u32) -> String {
    format!("oracle-login-snapshot:{pool_id}:{snapshot_id}:v{format_version}")
}

fn verify_worker_token_hash(pool: &OraclePool, verifier: &str) -> AppResult<()> {
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
        .await?;

    let workers = oracle_worker_service::list_workers(db, &pool.id).await?;
    let mut queued_workers = Vec::new();
    let mut skipped_workers = Vec::new();
    for worker in workers {
        let capable = worker
            .capabilities
            .iter()
            .any(|capability| capability == "commands_v1")
            && worker
                .capabilities
                .iter()
                .any(|capability| capability == "session_import_v1");
        if !capable {
            skipped_workers.push(worker.worker_label);
            continue;
        }
        let command = oracle_worker_service::enqueue_command(
            db,
            &pool.id,
            actor_user_id,
            &worker.worker_label,
            OracleWorkerCommandKind::SessionImport,
            Some(id.clone()),
            None,
        )
        .await?;
        queued_workers.push((worker.worker_label, command.id));
    }

    Ok(LoginSnapshotFanout {
        snapshot_id: id,
        envelope_size: snapshot.envelope_size,
        expires_at,
        queued_workers,
        skipped_workers,
    })
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
    let aad = snapshot_aad(pool_id, snapshot_id, snapshot.format_version);
    let sealed_envelope = encryption_keys
        .decrypt_with_aad(&snapshot.encrypted_envelope, aad.as_bytes())
        .await?;
    Ok(LoginSnapshotPayload {
        format_version: snapshot.format_version,
        sealed_envelope: Zeroizing::new(sealed_envelope),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::oracle_pool::OraclePoolVisibility;
    use crate::services::oracle_worker_service::WorkerPresenceInput;
    use crate::test_utils::{connect_test_database, test_encryption_keys};

    fn pool() -> OraclePool {
        let now = Utc::now();
        OraclePool {
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
        }
    }

    #[tokio::test]
    async fn snapshot_roundtrips_and_fanout_is_capability_gated() {
        let Some(db) = connect_test_database("oracle_login_snapshot").await else {
            return;
        };
        let pool = pool();
        let capable = oracle_worker_service::allocate_worker(&db, &pool)
            .await
            .unwrap();
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
        let legacy = oracle_worker_service::allocate_worker(&db, &pool)
            .await
            .unwrap();
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
        let pool = pool();
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
}
