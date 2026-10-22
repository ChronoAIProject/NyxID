//! Durable personal-erasure acceptance. A majority transaction installs the
//! account fence and intent only after every synchronous lifecycle prerequisite.
//! Domain completion is explicit; source absence alone is never completion.
use crate::{
    errors::{AppError, AppResult},
    models::{
        personal_erasure::{
            COLLECTION_NAME as JOBS, DomainCheckpoint, DomainStatus, ErasureStatus,
            PersonalErasureJob,
        },
        user::{COLLECTION_NAME as USERS, User},
    },
};
use bson::{Document, doc};
use mongodb::{
    ClientSession, Database,
    options::{IndexOptions, ReadConcern, WriteConcern},
};

pub(super) const INVENTORY_VERSION: u32 = 1;
const DOMAINS: &[&str] = &[
    "platform",
    "oracle",
    "channel_bots",
    "provider_credentials",
    "local_inventory",
    "source",
    // Machine lifecycle, deletion consent and paid liability are durable
    // records retained after source removal. Their native cleanup checkpoint
    // must not hold the account row, but still gates overall completion.
    "machine",
    "vendor",
];

pub async fn ensure_indexes(db: &Database) -> AppResult<()> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    db.collection::<Document>(JOBS)
        .create_indexes([
            mongodb::IndexModel::builder()
                .keys(doc! {"user_id":1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .name("personal_erasure_user_unique".to_owned())
                        .build(),
                )
                .build(),
            mongodb::IndexModel::builder()
                .keys(doc! {"status":1,"retry_at":1,"lease_expires_at":1,"accepted_at":1})
                .options(
                    IndexOptions::builder()
                        .name("personal_erasure_claim".to_owned())
                        .build(),
                )
                .build(),
        ])
        .await?;
    super::personal_erasure_registry::ensure_indexes(db).await?;
    preparation::ensure_indexes(db).await?;
    issuance::ensure_indexes(db).await?;
    Ok(())
}

pub async fn status(db: &Database, user_id: &str) -> AppResult<Option<PersonalErasureJob>> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    match db
        .collection::<PersonalErasureJob>(JOBS)
        .find_one(doc! {"user_id":user_id})
        .await?
    {
        Some(job) => provider::refresh(db, vendor::refresh(db, job).await?)
            .await
            .map(Some),
        None => Ok(None),
    }
}

/// The caller establishes self/admin HTTP authority. The owner policy is a
/// separate explicit self-service authorization, never inferred for an admin.
pub async fn accept(
    db: &Database,
    user_id: &str,
    requested_by: &str,
    owner_policy: Option<&str>,
) -> AppResult<PersonalErasureJob> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    if let Some(policy) = owner_policy {
        super::admin_user_service::validate_account_deletion_policy(policy)?;
        if requested_by != user_id {
            return Err(AppError::Forbidden(
                "Only the account owner can authorize managed Machine data deletion".into(),
            ));
        }
    }
    // A retry returns the retained accepted intent, including after source
    // removal; it cannot upgrade an earlier administrator request's authority.
    if let Some(job) = status(db, user_id).await? {
        return Ok(job);
    }
    let target = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id":user_id})
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    if target.user_type.is_org() {
        return Err(AppError::ValidationError(
            "Organization accounts must use the organization lifecycle".into(),
        ));
    }
    if !preparation::ready_or_enqueue(db, user_id).await? {
        return Err(AppError::ErasurePreparing);
    }
    let claims = super::channel_bot_service::claim_owner_aurinko_lifecycle(db, user_id).await?;
    let result = accept_claimed(db, user_id, requested_by, owner_policy).await;
    for claim in claims {
        // Releasing a coordination claim after COMMIT cannot invalidate durable
        // acceptance. Expiry also releases a failed release after process loss.
        if super::coordination_service::EventDedupStore::release(db, &claim)
            .await
            .is_err()
        {
            tracing::warn!("Email lifecycle claim release deferred to expiry");
        }
    }
    result
}

/// Credential attribution is not resource ownership. Shared refresh depends on
/// this app until its owner explicitly reconnects/transfers it. Reject before
/// closing any personal authority or enqueuing remote effects.
async fn guard_shared_provider_dependencies(
    db: &Database,
    user: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    for (collection, filter) in [
        (
            crate::models::user_provider_token::COLLECTION_NAME,
            doc! {"credential_user_id":user,"user_id":{"$ne":user},"status":{"$ne":"revoked"}},
        ),
        (
            crate::models::oauth_state::COLLECTION_NAME,
            doc! {"credential_user_id":user,
            "$expr":{"$ne":[{"$ifNull":["$target_user_id","$user_id"]},user]},
            "expires_at":{"$gt":bson::DateTime::now()}},
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?
            .is_some()
        {
            return Err(AppError::Conflict("Other owners still depend on your OAuth app credentials; reconnect or transfer those shared connections before deleting your account".into()));
        }
    }
    preparation::require_ready_in_session(db, user, tx).await
}

async fn guard_unsettled_channel_effects(
    db: &Database,
    user: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    for (collection, filter) in [
        // Reserved durable operations have not dispatched; the dispatch writer
        // contends with acceptance. Dispatched/uncertain effects retain their
        // existing reconciliation authority and cannot be erased by a sweep.
        (
            crate::models::durable_operation_execution::COLLECTION_NAME,
            doc! {"user_id":user,"status":{"$in":["dispatched","outcome_uncertain"]}},
        ),
        (
            crate::models::connect_link::COLLECTION_NAME,
            doc! {"user_id":user,"$or":[{"completion_claim_id":{"$type":"string"}},
            {"webhook_event_status":"pending"}]},
        ),
        (
            crate::models::trigger_run::COLLECTION_NAME,
            doc! {"user_id":user,"outcome":{"$in":["started","waiting"]}},
        ),
        (
            crate::models::trigger_delivery::COLLECTION_NAME,
            doc! {"user_id":user,"status":"pending"},
        ),
        (
            super::trigger_schedule::WORK,
            doc! {"user_id":user,"kind":"pause_notice"},
        ),
        (
            crate::models::channel_adapter::SESSIONS,
            doc! {"user_id":user,"$or":[{"state":{"$ne":"closed"}},{"billing_finalized":false}]},
        ),
        (
            crate::models::channel_adapter::EVENTS,
            doc! {"user_id":user,"event_kind":"sms_send","billing_finalized":false},
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?
            .is_some()
        {
            return Err(AppError::Conflict("Active automation, external call or message delivery must settle before account deletion can be accepted".into()));
        }
    }
    Ok(())
}

async fn accept_claimed(
    db: &Database,
    user: &str,
    actor: &str,
    owner_policy: Option<&str>,
) -> AppResult<PersonalErasureJob> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result: AppResult<PersonalErasureJob>=async {
        if let Some(job)=db.collection::<PersonalErasureJob>(JOBS).find_one(doc! {"user_id":user}).session(&mut tx).await? {return Ok(job);}
        let live=db.collection::<User>(USERS).find_one(doc! {"_id":user}).session(&mut tx).await?
            .ok_or_else(||AppError::NotFound("User not found".into()))?;
        if live.user_type.is_org() {return Err(AppError::ValidationError("Organization accounts must use the organization lifecycle".into()));}
        // Same user write fence as membership and call admission. No destructive
        // effects before these guards; any later failure rolls everything back.
        db.collection::<Document>(USERS).update_one(doc! {"_id":user,"user_type":{"$ne":"org"}},doc! {"$inc":{"erasure_acceptance_revision":1_i64}}).session(&mut tx).await?;
        guard_shared_provider_dependencies(db,user,&mut tx).await?;
        guard_unsettled_channel_effects(db,user,&mut tx).await?;
        channel::guard_in_session(db,user,&mut tx).await?;
        super::erasure_admission::close_in_session(db,user,&mut tx).await?;
        db.collection::<Document>(crate::models::durable_operation_grant::COLLECTION_NAME)
            .update_many(doc! {"user_id":user,"revoked_at":bson::Bson::Null},
                doc! {"$set":{"revoked_at":bson::DateTime::now(),"revoked_by":actor},"$inc":{"state_version":1_i64}})
            .session(&mut tx).await?;
        // Ordinary node capabilities close with acceptance, even when a later
        // external erasure domain needs operator recovery. Remote Machine
        // transport remains available solely for its explicit stop lifecycle.
        db.collection::<Document>(crate::models::node::COLLECTION_NAME).update_many(
            doc! {"user_id":user,"metadata.provisioning_source":{"$ne":"remote_machine"}},
            vec![doc! {"$set":{"is_active":false,"status":"draining","personal_erasure_drain_required":true,
                "personal_erasure_drain_owner":{"$ifNull":["$personal_erasure_drain_owner","$connection_owner"]}}}]
        ).session(&mut tx).await?;
        if !learning::clear_in_session(db,user,&mut tx).await? {
            return Err(AppError::Conflict("Reconcile the interrupted skill publication or wait for its active attempt before deleting your account; your account and provider credentials remain available for recovery".into()));
        }
        super::oracle_pool_service::accept_personal_erasure_in_session(db,&mut tx,user,actor).await?;
        super::admin_user_service::apply_user_security_state_in_session(db,&mut tx,user,false,true,owner_policy,Some(live.is_active)).await?;
        super::session_authority::revoke_all_in_transaction(db,user,&mut tx).await?;
        db.collection::<Document>(USERS).update_one(doc! {"_id":user},doc! {"$set":{"channel_adapter_admission_closed":true},"$inc":{"channel_adapter_admission_fence":1_i64}}).session(&mut tx).await?;
        super::telemetry_erasure_service::enqueue_in_session(db,user,&mut tx).await?;
        let now=chrono::Utc::now();
        let job=PersonalErasureJob {id:uuid::Uuid::new_v4().to_string(),user_id:user.to_owned(),requested_by:actor.to_owned(),owner_deletion_policy:owner_policy.map(str::to_owned),status:ErasureStatus::Accepted,inventory_version:INVENTORY_VERSION,
            domains:DOMAINS.iter().map(|domain|DomainCheckpoint { retry_generation:0,last_retry_key_hash:None,last_retry_request_digest:None,consecutive_failures: 0,domain:(*domain).to_owned(),status:DomainStatus::Pending,reason:None,cursor:None,completed_at:None}).collect(),lease_token:None,lease_expires_at:None,retry_at:None,accepted_at:now,updated_at:now,completed_at:None};
        db.collection::<PersonalErasureJob>(JOBS).insert_one(&job).session(&mut tx).await?;
        let key=super::audit_service::audit_chain_hmac_key().ok_or_else(||AppError::Internal("Audit chain key is not initialized".into()))?;
        super::audit_service::log_actor_event_in_session(db,&mut tx,key,&super::audit_service::AuditActor{user_id:actor.to_owned(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},"user.account.deletion_accepted",serde_json::json!({"target_user_id":user,"owner_machine_data_authorized":owner_policy.is_some(),"inventory_version":INVENTORY_VERSION})).await?;
        Ok(job)
    }.await;
    let job = match result {
        Ok(job) => job,
        Err(error) => {
            tx.abort_transaction().await?;
            return Err(match error {
                AppError::DatabaseError(error) => super::transaction_service::map_error(error),
                error => error,
            });
        }
    };
    super::transaction_service::commit(&mut tx).await?;
    Ok(job)
}

mod channel;
pub(crate) mod issuance;
mod learning;
mod local;
mod machine;
pub(crate) mod nodes;
mod preparation;
pub(crate) mod provider;
mod vendor;
mod worker;
#[cfg(test)]
pub(crate) use worker::process_one;
pub(crate) use worker::{DomainRetryRequest, retry_domain, spawn_worker};
#[cfg(test)]
pub(crate) async fn prepare_one_for_test(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
) -> AppResult<bool> {
    preparation::process_one(db, keys).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::{org_membership::OrgRole, user::UserType},
        test_utils::{connect_test_database, test_membership, test_user},
    };

    pub(super) async fn fixture() -> (Database, String) {
        let db = connect_test_database("durable_personal_erasure_acceptance")
            .await
            .expect("Mongo8 fixture required");
        crate::test_utils::init_test_audit_chain_hmac_key();
        ensure_indexes(&db).await.unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        (db, owner)
    }

    /// Preparation may advance only this bookkeeping fence; all account
    /// authority, authentication material and profile data must stay identical.
    pub(super) fn assert_account_preserved(mut actual: Document, mut expected: Document) {
        let before = expected.remove("credential_preparation_fence");
        let after = actual.remove("credential_preparation_fence");
        if let Some(value) = before {
            assert!(
                after.as_ref().and_then(bson::Bson::as_i64).unwrap() >= value.as_i64().unwrap()
            );
        } else if let Some(value) = after {
            assert!(value.as_i64().unwrap() >= 1);
        }
        assert_eq!(actual, expected);
    }

    pub(super) async fn finish_preparation(db: &Database, owner: &str) {
        let keys = crate::test_utils::test_encryption_keys();
        for _ in 0..64 {
            if super::preparation::ready_or_enqueue(db, owner)
                .await
                .unwrap()
            {
                return;
            }
            assert!(super::preparation::process_one(db, &keys).await.unwrap());
        }
        panic!("preparation failed to reach its persisted ready phase");
    }

    #[tokio::test]
    async fn rejected_guards_preserve_account_data_and_vendor_intent() {
        for guard in ["last_admin", "unsettled_call"] {
            let (db, owner) = fixture().await;
            if guard == "last_admin" {
                let org = uuid::Uuid::new_v4().to_string();
                db.collection::<User>(USERS)
                    .insert_one(test_user(&org, UserType::Org))
                    .await
                    .unwrap();
                db.collection(crate::models::org_membership::COLLECTION_NAME)
                    .insert_one(test_membership(&org, &owner, OrgRole::Admin, None))
                    .await
                    .unwrap();
            } else {
                db.collection::<Document>(crate::models::channel_adapter::SESSIONS).insert_one(doc! {"_id":"held-call","user_id":&owner,"state":"active","billing_finalized":false}).await.unwrap();
            }
            let original = db
                .collection::<Document>(USERS)
                .find_one(doc! {"_id":&owner})
                .await
                .unwrap()
                .unwrap();
            db.collection::<Document>(crate::models::channel_adapter::BINDINGS)
                .insert_one(doc! {"_id":"binding","user_id":&owner,"enabled":true,"revision":1_i64})
                .await
                .unwrap();
            assert!(matches!(
                accept(
                    &db,
                    &owner,
                    &owner,
                    Some("machine-data-provider-revocation-v2")
                )
                .await,
                Err(AppError::Conflict(_))
            ));
            super::tests::assert_account_preserved(
                db.collection::<Document>(USERS)
                    .find_one(doc! {"_id":&owner})
                    .await
                    .unwrap()
                    .unwrap(),
                original,
            );
            assert!(
                db.collection::<Document>(crate::models::channel_adapter::BINDINGS)
                    .find_one(doc! {"_id":"binding","enabled":true,"revision":1_i64})
                    .await
                    .unwrap()
                    .is_some()
            );
            assert!(status(&db, &owner).await.unwrap().is_none());
            assert_eq!(
                db.collection::<Document>(crate::models::telemetry_erasure_job::COLLECTION_NAME)
                    .count_documents(doc! {"user_id":&owner})
                    .await
                    .unwrap(),
                0
            );
        }
    }

    #[tokio::test]
    async fn acceptance_is_durable_idempotent_and_does_not_claim_completion() {
        let (db, owner) = fixture().await;
        let job = accept(
            &db,
            &owner,
            &owner,
            Some("machine-data-provider-revocation-v2"),
        )
        .await
        .unwrap();
        assert_eq!(job.status, ErasureStatus::Accepted);
        assert!(job.completed_at.is_none());
        assert!(
            job.domains
                .iter()
                .all(|d| d.status == DomainStatus::Pending)
        );
        let stored = db
            .collection::<User>(USERS)
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap()
            .unwrap();
        assert!(!stored.is_active);
        assert_eq!(
            accept(
                &db,
                &owner,
                &owner,
                Some("machine-data-provider-revocation-v2")
            )
            .await
            .unwrap()
            .id,
            job.id
        );
        assert_eq!(
            db.collection::<Document>(crate::models::telemetry_erasure_job::COLLECTION_NAME)
                .count_documents(doc! {"user_id":&owner})
                .await
                .unwrap(),
            1
        );
        let org = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        assert!(matches!(
            accept(&db, &org, &owner, None).await,
            Err(AppError::ValidationError(_))
        ));
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&org,"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
    }
}

#[cfg(test)]
mod byo_dependency_tests {
    use super::*;
    #[tokio::test]
    async fn pending_byo_oauth_dependency_uses_effective_target_owner() {
        for explicit_target in [false, true] {
            let (db, person) = super::tests::fixture().await;
            let other = uuid::Uuid::new_v4().to_string();
            let states = db.collection::<Document>(crate::models::oauth_state::COLLECTION_NAME);
            let source = db
                .collection::<Document>(USERS)
                .find_one(doc! {"_id":&person})
                .await
                .unwrap()
                .unwrap();
            states.insert_one(doc! {
                "_id":"pending-byo","credential_user_id":&person,
                "user_id":if explicit_target {&person} else {&other},
                "target_user_id":if explicit_target {Some(&other)} else {None},
                "expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(10))
            }).await.unwrap();
            assert!(matches!(accept(&db, &person, &person, None).await,
                Err(AppError::Conflict(message)) if message.contains("Other owners still depend")));
            super::tests::assert_account_preserved(
                db.collection::<Document>(USERS)
                    .find_one(doc! {"_id":&person})
                    .await
                    .unwrap()
                    .unwrap(),
                source,
            );
            assert!(status(&db, &person).await.unwrap().is_none());
            for collection in [
                provider::EFFECTS,
                crate::models::telemetry_erasure_job::COLLECTION_NAME,
            ] {
                assert_eq!(
                    db.collection::<Document>(collection)
                        .count_documents(doc! {})
                        .await
                        .unwrap(),
                    0
                );
            }
            // An initiating actor is attribution. Only the target owner is a
            // dependency on the person's client, and reconnecting releases it.
            states
                .update_one(
                    doc! {"_id":"pending-byo"},
                    doc! {"$set":{"user_id":&other,"target_user_id":&person}},
                )
                .await
                .unwrap();
            assert_eq!(
                accept(&db, &person, &person, None).await.unwrap().status,
                ErasureStatus::Accepted
            );
            db.drop().await.unwrap();
        }
    }
    #[tokio::test]
    async fn shared_byo_dependency_rejects_before_account_fence_or_personal_purge() {
        let (db, person) = super::tests::fixture().await;
        let org = uuid::Uuid::new_v4().to_string();
        let tokens = db.collection::<Document>(crate::models::user_provider_token::COLLECTION_NAME);
        let encrypted = crate::test_utils::test_encryption_keys()
            .encrypt(b"personal-refresh")
            .await
            .unwrap();
        tokens.insert_one(doc! {"_id":"personal-token","user_id":&person,"credential_user_id":&person,"provider_config_id":"provider","status":"active","refresh_token_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}}).await.unwrap();
        tokens.insert_one(doc! {"_id":"shared-token","user_id":&org,"credential_user_id":&person,"provider_config_id":"provider","status":"active","refresh_token_encrypted":vec![8_i32]}).await.unwrap();
        super::tests::finish_preparation(&db, &person).await;
        let source = db
            .collection::<Document>(USERS)
            .find_one(doc! {"_id":&person})
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            accept(&db, &person, &person, Some("machine-data-provider-revocation-v2")).await,
            Err(AppError::Conflict(message)) if message.contains("Other owners still depend")
        ));
        super::tests::assert_account_preserved(
            db.collection::<Document>(USERS)
                .find_one(doc! {"_id":&person})
                .await
                .unwrap()
                .unwrap(),
            source,
        );
        assert_eq!(
            tokens
                .count_documents(doc! {"status":"active"})
                .await
                .unwrap(),
            2
        );
        assert!(status(&db, &person).await.unwrap().is_none());
        assert_eq!(
            db.collection::<Document>(crate::models::telemetry_erasure_job::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }
}

#[cfg(test)]
mod shared_copy_tests {
    use super::*;
    #[tokio::test]
    async fn platform_app_shared_connection_and_legacy_ciphertext_copies_never_block_acceptance() {
        for (same_connection, same_ciphertext) in [(true, false), (false, true)] {
            let (db, person) = super::tests::fixture().await;
            let org = uuid::Uuid::new_v4().to_string();
            let key = db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME);
            let keys = crate::test_utils::test_encryption_keys();
            let personal = keys.encrypt(b"personal-access").await.unwrap();
            let unrelated = keys.encrypt(b"unrelated-access").await.unwrap();
            let encrypted = |byte| {
                bson::Bson::Binary(bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: if byte == 7 {
                        personal.clone()
                    } else {
                        unrelated.clone()
                    },
                })
            };
            key.insert_one(doc! {"_id":"personal-key","user_id":&person,"connection_id":"connection-a","provider_config_id":"provider","status":"active","access_token_encrypted":encrypted(7)}).await.unwrap();
            key.insert_one(doc! {"_id":"org-key","user_id":&org,"connection_id":if same_connection {"connection-a"} else {"connection-b"},"provider_config_id":"provider","status":"active","access_token_encrypted":encrypted(if same_ciphertext {7} else {8})}).await.unwrap();
            super::tests::finish_preparation(&db, &person).await;
            let org_source = key.find_one(doc! {"_id":"org-key"}).await.unwrap().unwrap();
            let accepted = accept(
                &db,
                &person,
                &person,
                Some("machine-data-provider-revocation-v2"),
            )
            .await
            .unwrap();
            assert_eq!(accepted.status, ErasureStatus::Accepted);
            assert_eq!(
                key.find_one(doc! {"_id":"org-key"}).await.unwrap().unwrap(),
                org_source
            );
            assert_eq!(
                key.count_documents(doc! {"status":"active"}).await.unwrap(),
                2
            );
            assert!(status(&db, &person).await.unwrap().is_some());
            db.drop().await.unwrap();
        }
    }
}

#[cfg(test)]
mod independent_credential_tests {
    use super::*;

    #[tokio::test]
    async fn distinct_legacy_credentials_at_same_provider_do_not_block_deletion() {
        let (db, user) = super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let tokens = db.collection::<Document>(crate::models::user_provider_token::COLLECTION_NAME);
        for (owner, secret) in [
            (user.as_str(), "personal-refresh"),
            ("independent-owner", "unrelated-refresh"),
        ] {
            tokens.insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":owner,
                "provider_config_id":"same-provider","status":"active",
                "refresh_token_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:keys.encrypt(secret.as_bytes()).await.unwrap()}}).await.unwrap();
        }
        assert!(
            !super::preparation::ready_or_enqueue(&db, &user)
                .await
                .unwrap()
        );
        super::tests::finish_preparation(&db, &user).await;
        assert!(
            super::preparation::ready_or_enqueue(&db, &user)
                .await
                .unwrap()
        );
        accept(&db, &user, &user, None).await.unwrap();
        assert!(
            tokens
                .find_one(doc! {"user_id":"independent-owner","status":"active"})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }
}

#[cfg(test)]
mod integration_tests;
