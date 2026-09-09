use super::*;
use crate::models::org_membership::OrgRole;
use crate::models::user::UserType;
use crate::test_utils::{connect_transaction_test_database, test_membership, test_user};

async fn fixture() -> (Database, OraclePool, String, String, String) {
    let db = connect_transaction_test_database("oracle_enrollment").await;
    ensure_indexes(&db).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let actor = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_many([
            test_user(&org, UserType::Org),
            test_user(&actor, UserType::Person),
        ])
        .await
        .unwrap();
    let membership = test_membership(&org, &actor, OrgRole::Member, None);
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(&membership)
        .await
        .unwrap();
    let (pool, legacy) = oracle_pool_service::create_pool(
        &db,
        &org,
        oracle_pool_service::CreatePoolInput {
            slug: "org-workers".into(),
            name: "Org workers".into(),
            visibility: Some(OraclePoolVisibility::Org),
            max_workers: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    (db, pool, actor, membership.id, legacy)
}

fn credential() -> String {
    format!(
        "{CREDENTIAL_PREFIX}{}",
        hex::encode(rand::random::<[u8; 32]>())
    )
}

#[tokio::test]
async fn oracle_enrollment_is_idempotent_scoped_and_hash_only() {
    let (db, pool, actor, _, legacy) = fixture().await;
    let instance = uuid::Uuid::new_v4().to_string();
    let secret = credential();
    let worker = enroll(&db, &actor, &pool, &instance, Some("mine"), &secret)
        .await
        .unwrap();
    let repeated = enroll(&db, &actor, &pool, &instance, None, &secret)
        .await
        .unwrap();
    assert_eq!(worker.id, repeated.id);
    assert_eq!(
        db.collection::<Document>(WORKERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let stored = db
        .collection::<Document>(WORKERS)
        .find_one(doc! { "_id": &worker.id })
        .await
        .unwrap()
        .unwrap();
    assert!(!format!("{stored:?}").contains(&secret));
    assert_eq!(
        stored
            .get_document("enrollment")
            .unwrap()
            .get_str("credential_hash")
            .unwrap(),
        hash_token(&secret)
    );
    assert!(
        stored
            .get_document("enrollment")
            .unwrap()
            .get_datetime("membership_created_at")
            .is_ok()
    );
    assert!(!format!("{:?}", worker.enrollment).contains(&hash_token(&secret)));
    let auth = authenticate(&db, &secret).await.unwrap();
    assert!(auth.ensure_identity("mine", Some(&instance)).is_ok());
    assert!(auth.ensure_identity("other", Some(&instance)).is_err());
    assert!(auth.ensure_identity("mine", None).is_err());
    assert!(auth.ensure_identity("mine", Some("different")).is_err());
    assert!(auth.ensure_pool_credential().is_err());
    assert!(
        authenticate(&db, &legacy)
            .await
            .unwrap()
            .ensure_pool_credential()
            .is_ok()
    );
    assert!(matches!(
        enroll(&db, &actor, &pool, &instance, Some("changed"), &secret).await,
        Err(AppError::OracleWorkerLabelUnavailable(_))
    ));
    assert!(matches!(
        enroll(
            &db,
            &actor,
            &pool,
            &uuid::Uuid::new_v4().to_string(),
            Some("mine"),
            &credential()
        )
        .await,
        Err(AppError::OracleWorkerLabelUnavailable(_))
    ));
    oracle_worker_service::allocate_worker(&db, &pool, Some("legacy"))
        .await
        .unwrap();
    assert!(matches!(
        enroll(
            &db,
            &actor,
            &pool,
            &uuid::Uuid::new_v4().to_string(),
            Some("legacy"),
            &credential()
        )
        .await,
        Err(AppError::OracleWorkerLabelUnavailable(_))
    ));
    let replacement = credential();
    enroll(&db, &actor, &pool, &instance, None, &replacement)
        .await
        .unwrap();
    assert!(authenticate(&db, &secret).await.is_err());
    assert!(authenticate(&db, &replacement).await.is_ok());
    // The dispatch limit remains one; enrolling another contributor still succeeds.
    enroll(
        &db,
        &actor,
        &pool,
        &uuid::Uuid::new_v4().to_string(),
        None,
        &credential(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn oracle_enrollment_revalidates_membership_rejoin_rotation_disable_and_forget() {
    let (db, pool, actor, membership_id, _) = fixture().await;
    let instance = uuid::Uuid::new_v4().to_string();
    let original = credential();
    enroll(&db, &actor, &pool, &instance, Some("mine"), &original)
        .await
        .unwrap();
    db.collection::<Document>(MEMBERSHIPS)
        .update_one(
            doc! { "_id": &membership_id },
            doc! { "$set": { "role": "viewer" } },
        )
        .await
        .unwrap();
    assert!(!can_enroll(&db, &actor, &pool).await);
    assert!(authenticate(&db, &original).await.is_err());
    assert!(
        enroll(&db, &actor, &pool, &instance, None, &credential())
            .await
            .is_err()
    );
    db.collection::<Document>(MEMBERSHIPS)
        .update_one(
            doc! { "_id": &membership_id },
            doc! { "$set": { "role": "member", "revoked_at": bson::DateTime::now() } },
        )
        .await
        .unwrap();
    assert!(authenticate(&db, &original).await.is_err());
    db.collection::<Document>(MEMBERSHIPS).update_one(doc! { "_id": &membership_id }, doc! { "$set": { "revoked_at": null, "created_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(1)) } }).await.unwrap();
    assert!(authenticate(&db, &original).await.is_err());
    assert!(matches!(
        enroll(&db, &actor, &pool, &instance, None, &original).await,
        Err(AppError::OracleWorkerCredentialRenewalRequired)
    ));
    let rejoined = credential();
    enroll(&db, &actor, &pool, &instance, None, &rejoined)
        .await
        .unwrap();
    assert!(authenticate(&db, &rejoined).await.is_ok());
    db.collection::<Document>(POOLS)
        .update_one(
            doc! { "_id": &pool.id },
            doc! { "$set": { "worker_token_hash": hash_token("new-pool-token") } },
        )
        .await
        .unwrap();
    assert!(authenticate(&db, &rejoined).await.is_err());
    assert!(matches!(
        enroll(&db, &actor, &pool, &instance, None, &rejoined).await,
        Err(AppError::OracleWorkerCredentialRenewalRequired)
    ));
    let renewed = credential();
    enroll(&db, &actor, &pool, &instance, None, &renewed)
        .await
        .unwrap();
    for (collection, id) in [(USERS, &actor), (USERS, &pool.user_id), (POOLS, &pool.id)] {
        db.collection::<Document>(collection)
            .update_one(doc! { "_id": id }, doc! { "$set": { "is_active": false } })
            .await
            .unwrap();
        assert!(authenticate(&db, &renewed).await.is_err());
        db.collection::<Document>(collection)
            .update_one(doc! { "_id": id }, doc! { "$set": { "is_active": true } })
            .await
            .unwrap();
    }
    assert!(authenticate(&db, &renewed).await.is_ok());
    oracle_worker_service::forget_worker(&db, &pool, "mine", true)
        .await
        .unwrap();
    assert!(authenticate(&db, &renewed).await.is_err());
    let fresh = credential();
    enroll(
        &db,
        &actor,
        &pool,
        &uuid::Uuid::new_v4().to_string(),
        Some("mine"),
        &fresh,
    )
    .await
    .unwrap();
    assert!(authenticate(&db, &fresh).await.is_ok());
    assert!(authenticate(&db, &renewed).await.is_err());
}

#[tokio::test]
async fn oracle_enrollment_capacity_and_installation_races_are_atomic() {
    let (db, pool, actor, _, _) = fixture().await;
    let instance = uuid::Uuid::new_v4().to_string();
    let secret = credential();
    let (first, second) = tokio::join!(
        enroll(&db, &actor, &pool, &instance, None, &secret),
        enroll(&db, &actor, &pool, &instance, None, &secret),
    );
    assert_eq!(first.unwrap().id, second.unwrap().id);
    let seed = oracle_worker_service::get_worker(
        &db,
        &pool.id,
        &authenticate(&db, &secret)
            .await
            .unwrap()
            .installation
            .unwrap()
            .worker_label,
    )
    .await
    .unwrap();
    let mut rows = Vec::new();
    for index in 1..MAX_ENROLLED_WORKERS - 1 {
        let mut row = seed.clone();
        row.worker_label = format!("reserved-{index}");
        row.id = crate::models::oracle_worker::worker_doc_id(&pool.id, &row.worker_label);
        row.instance_id = Some(uuid::Uuid::new_v4().to_string());
        row.enrollment.as_mut().unwrap().credential_hash = hash_token(&credential());
        rows.push(row);
    }
    db.collection::<OracleWorker>(WORKERS)
        .insert_many(rows)
        .await
        .unwrap();
    let a = uuid::Uuid::new_v4().to_string();
    let b = uuid::Uuid::new_v4().to_string();
    let a_token = credential();
    let b_token = credential();
    let (a, b) = tokio::join!(
        enroll(&db, &actor, &pool, &a, None, &a_token),
        enroll(&db, &actor, &pool, &b, None, &b_token)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(
        db.collection::<Document>(WORKERS)
            .count_documents(doc! { "pool_id": &pool.id })
            .await
            .unwrap(),
        MAX_ENROLLED_WORKERS
    );
    enroll(&db, &actor, &pool, &instance, None, &secret)
        .await
        .unwrap();
}

#[test]
fn oracle_enrollment_credentials_are_bounded_and_renewal_error_is_stable() {
    assert!(validate_credential(&credential()).is_ok());
    for invalid in [
        "nyx_owk_abc",
        "nyx_owi_",
        &format!("nyx_owi_{}", "f".repeat(65)),
    ] {
        assert!(validate_credential(invalid).is_err());
    }
    let error = AppError::OracleWorkerCredentialRenewalRequired;
    assert_eq!(error.error_code(), 11016);
    use axum::response::IntoResponse;
    assert_eq!(
        error.into_response().status(),
        axum::http::StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn oracle_enrollment_repeated_rejoins_advance_bson_epoch_when_clock_has_not_advanced() {
    let (db, pool, actor, membership_id, _) = fixture().await;
    // Hold the persisted epoch ahead of the clock so back-to-back reactivations exercise
    // the same-millisecond/clock-rollback branch deterministically.
    let epoch = bson::DateTime::from_chrono(Utc::now() + chrono::Duration::hours(1));
    db.collection::<Document>(MEMBERSHIPS)
        .update_one(
            doc! { "_id": &membership_id },
            doc! { "$set": { "created_at": epoch } },
        )
        .await
        .unwrap();
    let instance = uuid::Uuid::new_v4().to_string();
    let mut secret = credential();
    enroll(&db, &actor, &pool, &instance, Some("mine"), &secret)
        .await
        .unwrap();
    for step in 1..=2 {
        crate::services::org_service::revoke_membership(&db, &pool.user_id, &actor)
            .await
            .unwrap();
        let membership = crate::services::org_service::create_membership(
            &db,
            &pool.user_id,
            &actor,
            OrgRole::Member,
            crate::models::org_membership::MemberScopeSource::Inherit,
            None,
        )
        .await
        .unwrap();
        assert_eq!(membership.id, membership_id);
        assert_eq!(
            membership.created_at.timestamp_millis(),
            epoch.timestamp_millis() + step
        );
        assert!(authenticate(&db, &secret).await.is_err());
        assert!(matches!(
            enroll(&db, &actor, &pool, &instance, None, &secret).await,
            Err(AppError::OracleWorkerCredentialRenewalRequired)
        ));
        secret = credential();
        enroll(&db, &actor, &pool, &instance, None, &secret)
            .await
            .unwrap();
        assert!(authenticate(&db, &secret).await.is_ok());
    }
}

#[tokio::test]
async fn oracle_enrollment_isolated_from_saved_login_material_and_legacy_fanout() {
    let (db, pool, actor, _, _) = fixture().await;
    let instance = uuid::Uuid::new_v4().to_string();
    enroll(&db, &actor, &pool, &instance, Some("mine"), &credential())
        .await
        .unwrap();
    let capabilities = vec![
        "commands_v1".to_string(),
        "saved_login_v1".to_string(),
        "session_import_v1".to_string(),
    ];
    let contributed = oracle_worker_service::report_presence(
        &db,
        &pool,
        oracle_worker_service::WorkerPresenceInput {
            worker_label: "mine".into(),
            instance_id: Some(instance),
            capabilities: capabilities.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(contributed.capabilities, vec!["commands_v1"]);
    let legacy = oracle_worker_service::report_presence(
        &db,
        &pool,
        oracle_worker_service::WorkerPresenceInput {
            worker_label: "legacy".into(),
            instance_id: Some(uuid::Uuid::new_v4().to_string()),
            capabilities,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(legacy.capabilities.contains(&"saved_login_v1".into()));
    assert!(matches!(
        crate::services::oracle_login_profile_service::bind(
            &db, &pool, "account", "mine", None, false
        )
        .await,
        Err(AppError::OracleWorkerCapabilityUnsupported(_))
    ));
    let state = crate::test_utils::test_app_state(db.clone());
    let fanout = crate::services::oracle_login_snapshot_service::create_and_fanout(
        &db,
        &state.encryption_keys,
        &pool,
        &actor,
        crate::services::oracle_login_snapshot_service::CreateLoginSnapshotInput {
            format_version: 1,
            worker_token_sha256: zeroize::Zeroizing::new(pool.worker_token_hash.clone()),
            sealed_envelope: zeroize::Zeroizing::new(b"opaque-test-login-envelope".to_vec()),
        },
    )
    .await
    .unwrap();
    assert_eq!(fanout.skipped_workers, vec!["mine"]);
    assert_eq!(fanout.queued_workers.len(), 1);
    assert_eq!(fanout.queued_workers[0].0, "legacy");
}
