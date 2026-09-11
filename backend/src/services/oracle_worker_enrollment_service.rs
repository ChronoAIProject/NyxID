use bson::{Document, doc};
use chrono::Utc;
use mongodb::options::{IndexOptions, ReturnDocument};
use mongodb::{ClientSession, Database, IndexModel};

use crate::crypto::token::hash_token;
use crate::errors::{AppError, AppResult};
use crate::models::{
    oracle_pool::{COLLECTION_NAME as POOLS, OraclePool, OraclePoolVisibility},
    oracle_worker::{COLLECTION_NAME as WORKERS, OracleWorker, OracleWorkerEnrollment},
    org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership},
    user::{COLLECTION_NAME as USERS, User},
};
use crate::services::{
    api_key_mutation_service as transactions, oracle_pool_service, oracle_worker_service,
};

pub const CREDENTIAL_PREFIX: &str = "nyx_owi_";
pub const MAX_ENROLLED_WORKERS: u64 = 256;

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<Document>(WORKERS)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "enrollment.credential_hash": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(
                            doc! { "enrollment.credential_hash": { "$type": "string" } },
                        )
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

fn forbidden() -> AppError {
    AppError::Forbidden("Only the pool owner, an org admin, or a contributing member of an org-visible pool may enroll workers".into())
}

fn membership_allows(pool: &OraclePool, membership: &OrgMembership) -> bool {
    membership.is_active()
        && (membership.role.can_admin()
            || (pool.visibility == OraclePoolVisibility::Org && membership.role.can_proxy()))
}

pub async fn enrollment_membership(
    db: &Database,
    actor: &str,
    pool: &OraclePool,
) -> AppResult<Option<OrgMembership>> {
    let users = db.collection::<User>(USERS);
    if users
        .find_one(doc! { "_id": actor, "is_active": true, "user_type": "person" })
        .await?
        .is_none()
    {
        return Err(forbidden());
    }
    if actor == pool.user_id {
        return Ok(None);
    }
    if users
        .find_one(doc! { "_id": &pool.user_id, "is_active": true, "user_type": "org" })
        .await?
        .is_none()
    {
        return Err(forbidden());
    }
    let membership = crate::services::org_service::get_active_membership(db, &pool.user_id, actor)
        .await?
        .filter(|membership| membership_allows(pool, membership))
        .ok_or_else(forbidden)?;
    Ok(Some(membership))
}

pub async fn can_enroll(db: &Database, actor: &str, pool: &OraclePool) -> bool {
    pool.is_active && enrollment_membership(db, actor, pool).await.is_ok()
}

fn validate_credential(raw: &str) -> AppResult<()> {
    let valid = raw.strip_prefix(CREDENTIAL_PREFIX).is_some_and(|secret| {
        secret.len() == 64
            && secret
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    if !valid {
        return Err(AppError::ValidationError(
            "invalid installation credential format".into(),
        ));
    }
    Ok(())
}

async fn lock_authority(
    db: &Database,
    session: &mut ClientSession,
    actor: &str,
    pool: &OraclePool,
) -> AppResult<Option<OrgMembership>> {
    // Enrollment serializes with user disable and membership removal, not just other enrollments.
    let actor_live = db
        .collection::<User>(USERS)
        .update_one(
            doc! { "_id": actor, "is_active": true, "user_type": "person" },
            doc! { "$inc": { "oracle_enrollment_epoch": 1_i64 } },
        )
        .session(&mut *session)
        .await?;
    if actor_live.matched_count != 1 {
        return Err(forbidden());
    }
    if actor == pool.user_id {
        return Ok(None);
    }
    let org_live = db
        .collection::<User>(USERS)
        .update_one(
            doc! { "_id": &pool.user_id, "is_active": true, "user_type": "org" },
            doc! { "$inc": { "oracle_enrollment_epoch": 1_i64 } },
        )
        .session(&mut *session)
        .await?;
    if org_live.matched_count != 1 {
        return Err(forbidden());
    }
    let membership = db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .find_one_and_update(
            doc! { "org_user_id": &pool.user_id, "member_user_id": actor, "revoked_at": null },
            doc! { "$inc": { "oracle_enrollment_epoch": 1_i64 } },
        )
        .return_document(ReturnDocument::After)
        .session(&mut *session)
        .await?
        .filter(|membership| membership_allows(pool, membership))
        .ok_or_else(forbidden)?;
    Ok(Some(membership))
}

pub async fn enroll(
    db: &Database,
    actor: &str,
    pool: &OraclePool,
    installation_id: &str,
    label: Option<&str>,
    credential: &str,
) -> AppResult<OracleWorker> {
    validate_credential(credential)?;
    if uuid::Uuid::parse_str(installation_id).is_err() {
        return Err(AppError::ValidationError(
            "installation_id must be a UUID".into(),
        ));
    }
    if let Some(label) = label {
        super::oracle_task_service::validate_worker_label(label)?;
    }
    let db = db.clone();
    let actor = actor.to_string();
    let pool_id = pool.id.clone();
    let installation_id = installation_id.to_string();
    let requested_label = label.map(str::to_string);
    let candidate_label = requested_label
        .clone()
        .unwrap_or_else(|| format!("worker-{}", hex::encode(rand::random::<[u8; 8]>())));
    let credential_hash = hash_token(credential);
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let operation: AppResult<OracleWorker> = async {
            // All new enrollments contend on the pool, so the inventory bound is atomic.
            let pool = db.collection::<OraclePool>(POOLS).find_one_and_update(
                doc! { "_id": &pool_id, "is_active": true },
                doc! { "$inc": { "worker_enrollment_epoch": 1_i64 } },
            ).return_document(ReturnDocument::After).session(&mut *session).await?
                .ok_or_else(|| AppError::OraclePoolInactive(pool_id.clone()))?;
            let membership = lock_authority(&db, session, &actor, &pool).await?;
            let enrollment = OracleWorkerEnrollment {
                owner_user_id: actor.clone(),
                credential_hash: credential_hash.clone(),
                pool_token_hash: pool.worker_token_hash.clone(),
                membership_id: membership.as_ref().map(|membership| membership.id.clone()),
                membership_created_at: membership.as_ref().map(|membership| membership.created_at),
            };
            let workers = db.collection::<OracleWorker>(WORKERS);
            let existing = workers.find_one(doc! { "pool_id": &pool.id, "instance_id": &installation_id })
                .session(&mut *session).await?;
            if let Some(mut worker) = existing {
                let Some(previous) = worker.enrollment.as_ref().filter(|previous| previous.owner_user_id == actor) else {
                    return Err(AppError::OracleWorkerLabelUnavailable("installation belongs to another worker".into()));
                };
                if requested_label.as_ref().is_some_and(|label| *label != worker.worker_label) {
                    return Err(AppError::OracleWorkerLabelUnavailable("installation already has a different label".into()));
                }
                if previous.credential_hash == credential_hash
                    && (previous.pool_token_hash != enrollment.pool_token_hash
                        || previous.membership_id != enrollment.membership_id
                        || previous.membership_created_at != enrollment.membership_created_at) {
                    return Err(AppError::OracleWorkerCredentialRenewalRequired);
                }
                workers.update_one(doc! { "_id": &worker.id }, doc! { "$set": {
                    "enrollment": bson::to_document(&enrollment).map_err(|_| AppError::Internal("enrollment serialization failed".into()))?,
                } }).session(&mut *session).await?;
                worker.enrollment = Some(enrollment);
                return Ok(worker);
            }
            let count = workers.count_documents(doc! { "pool_id": &pool.id, "enrollment": { "$ne": null } })
                .session(&mut *session).await?;
            if count >= MAX_ENROLLED_WORKERS {
                return Err(AppError::Conflict("pool installation limit reached; forget an unused worker before enrolling another".into()));
            }
            let mut worker = oracle_worker_service::provisioned_worker(&pool, &candidate_label, Utc::now());
            worker.instance_id = Some(installation_id.clone());
            worker.enrollment = Some(enrollment);
            workers.insert_one(&worker).session(&mut *session).await?;
            Ok(worker)
        }.await;
        transactions::transaction_result(operation)
    }).await.map_err(|error| {
        if oracle_pool_service::is_duplicate_key(&error) {
            AppError::OracleWorkerLabelUnavailable("worker label or credential is already enrolled".into())
        } else { transactions::map_transaction_error(error) }
    })
}

pub struct WorkerAuth {
    pub pool: OraclePool,
    pub installation: Option<OracleWorker>,
}

pub fn worker_filter(worker: &OracleWorker) -> Document {
    let mut filter = doc! { "_id": &worker.id, "generation": &worker.generation, "instance_id": &worker.instance_id };
    if let Some(enrollment) = &worker.enrollment {
        filter.insert("enrollment.credential_hash", &enrollment.credential_hash);
        filter.insert("enrollment.owner_user_id", &enrollment.owner_user_id);
    }
    filter
}

pub async fn ensure_current_worker(db: &Database, worker: &OracleWorker) -> AppResult<()> {
    if db
        .collection::<Document>(WORKERS)
        .find_one(worker_filter(worker))
        .await?
        .is_none()
    {
        return Err(AppError::OracleWorkerTokenInvalid);
    }
    Ok(())
}

impl WorkerAuth {
    pub async fn ensure_current_identity(
        &self,
        db: &Database,
        worker: &str,
        instance: Option<&str>,
    ) -> AppResult<()> {
        self.ensure_identity(worker, instance)?;
        if let Some(bound) = &self.installation {
            ensure_current_worker(db, bound).await
        } else {
            oracle_worker_service::ensure_instance_matches(db, &self.pool, worker, instance).await
        }
    }
    pub fn ensure_identity(&self, worker: &str, instance: Option<&str>) -> AppResult<()> {
        if let Some(bound) = &self.installation
            && (bound.worker_label != worker || bound.instance_id.as_deref() != instance)
        {
            return Err(AppError::OracleWorkerTokenInvalid);
        }
        Ok(())
    }

    pub fn ensure_pool_credential(&self) -> AppResult<()> {
        if self.installation.is_some() {
            return Err(AppError::Forbidden(
                "installation credentials cannot access pool login material".into(),
            ));
        }
        Ok(())
    }
}

pub async fn authenticate(db: &Database, credential: &str) -> AppResult<WorkerAuth> {
    if !credential.starts_with(CREDENTIAL_PREFIX) {
        return Ok(WorkerAuth {
            pool: oracle_pool_service::validate_worker_token(db, credential).await?,
            installation: None,
        });
    }
    validate_credential(credential).map_err(|_| AppError::OracleWorkerTokenInvalid)?;
    let worker = db
        .collection::<OracleWorker>(WORKERS)
        .find_one(doc! { "enrollment.credential_hash": hash_token(credential) })
        .await?
        .ok_or(AppError::OracleWorkerTokenInvalid)?;
    let enrollment = worker
        .enrollment
        .as_ref()
        .ok_or(AppError::OracleWorkerTokenInvalid)?;
    let pool = db
        .collection::<OraclePool>(POOLS)
        .find_one(doc! { "_id": &worker.pool_id,
        "is_active": true, "worker_token_hash": &enrollment.pool_token_hash })
        .await?
        .ok_or(AppError::OracleWorkerTokenInvalid)?;
    let membership = enrollment_membership(db, &enrollment.owner_user_id, &pool)
        .await
        .map_err(|error| match error {
            AppError::DatabaseError(_) => error,
            _ => AppError::OracleWorkerTokenInvalid,
        })?;
    if membership.as_ref().map(|membership| &membership.id) != enrollment.membership_id.as_ref()
        || membership.as_ref().map(|membership| membership.created_at)
            != enrollment.membership_created_at
    {
        return Err(AppError::OracleWorkerTokenInvalid);
    }
    Ok(WorkerAuth {
        pool,
        installation: Some(worker),
    })
}

pub async fn ensure_can_manage_worker(
    db: &Database,
    actor: &str,
    pool: &OraclePool,
    worker: &OracleWorker,
) -> AppResult<()> {
    if oracle_pool_service::ensure_can_manage(db, actor, pool)
        .await
        .is_ok()
    {
        return Ok(());
    }
    enrollment_membership(db, actor, pool).await?;
    if !worker
        .enrollment
        .as_ref()
        .is_some_and(|enrollment| enrollment.owner_user_id == actor)
    {
        return Err(AppError::Forbidden(
            "members may manage only their own contributed workers".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
