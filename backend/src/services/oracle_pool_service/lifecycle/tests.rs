use super::*;
use crate::models::{
    oracle_session::COLLECTION_NAME as SESSIONS,
    oracle_task::COLLECTION_NAME as TASKS,
    oracle_worker::COLLECTION_NAME as WORKERS,
    org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole},
};
use crate::services::{
    oracle_session_service as sessions, oracle_task_service as tasks,
    oracle_worker_enrollment_service as enrollment, oracle_worker_service as workers,
};
use crate::test_utils::{
    connect_transaction_test_database, init_test_audit_chain_hmac_key, test_membership, test_user,
};

struct Fixture {
    db: Database,
    actor: AuditActor,
    pool: OraclePool,
    token: String,
    org: String,
}
async fn fixture(name: &str) -> Fixture {
    init_test_audit_chain_hmac_key();
    let db = connect_transaction_test_database(name).await;
    ensure_lifecycle_indexes(&db).await.unwrap();
    enrollment::ensure_indexes(&db).await.unwrap();
    for (collection, keys, partial) in [
        (
            TASKS,
            doc! {"pool_id":1,"submitter_user_id":1,"client_ref":1},
            doc! {"client_ref":{"$exists":true}},
        ),
        (
            crate::models::oracle_worker_command::COLLECTION_NAME,
            doc! {"pool_id":1,"worker_label":1,"status":1},
            doc! {"status":"delivered"},
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(keys)
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(partial)
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
    }
    let person = uuid::Uuid::new_v4().to_string();
    let org = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_many([
            test_user(&person, UserType::Person),
            test_user(&org, UserType::Org),
        ])
        .await
        .unwrap();
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(&org, &person, OrgRole::Admin, None))
        .await
        .unwrap();
    db.collection::<Document>(ORACLE_POOLS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"slug":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let (pool, token) = create_pool(
        &db,
        &person,
        CreatePoolInput {
            slug: "lifecycle".into(),
            name: "Lifecycle".into(),
            visibility: Some(OraclePoolVisibility::Platform),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    Fixture {
        db,
        actor: AuditActor {
            user_id: person,
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        pool,
        token,
        org,
    }
}
fn identity(f: &Fixture) -> tasks::SubmitterIdentity {
    tasks::SubmitterIdentity {
        user_id: f.actor.user_id.clone(),
        api_key_id: None,
        api_key_name: None,
    }
}
fn transfer_input(f: &Fixture) -> TransferPoolInput {
    TransferPoolInput {
        owner_user_id: f.org.clone(),
        ..Default::default()
    }
}
fn prompt() -> tasks::SubmitTaskInput {
    tasks::SubmitTaskInput {
        prompt: "test body must not enter audit".into(),
        conversation_id: Some("".into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn oracle_lifecycle_transfer_preserves_records_and_shared_token_but_revokes_installations() {
    let f = fixture("oracle_lifecycle_transfer").await;
    let credential = format!("nyx_owi_{}", "a".repeat(64));
    let instance = uuid::Uuid::new_v4().to_string();
    let worker = enrollment::enroll(
        &f.db,
        &f.actor.user_id,
        &f.pool,
        &instance,
        Some("contributor"),
        &credential,
    )
    .await
    .unwrap();
    let task = tasks::submit_task(&f.db, &f.pool, &identity(&f), prompt())
        .await
        .unwrap()
        .task;
    let result = transfer_pool(&f.db, &f.actor, &f.pool.slug, transfer_input(&f))
        .await
        .unwrap();
    assert_eq!(result.pool.id, f.pool.id);
    assert_eq!(result.pool.slug, f.pool.slug);
    assert_eq!(result.pool.user_id, f.org);
    assert_eq!(result.pool.visibility, OraclePoolVisibility::Platform);
    assert!(result.worker_token.is_none());
    assert!(result.can_enroll);
    assert_eq!(
        validate_worker_token(&f.db, &f.token).await.unwrap().id,
        f.pool.id
    );
    assert!(enrollment::authenticate(&f.db, &credential).await.is_err());
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        enrollment::ensure_current_worker_in_session(&f.db, &mut session, &worker)
            .await
            .is_err()
    );
    session.abort_transaction().await.unwrap();
    assert!(matches!(
        enrollment::enroll(
            &f.db,
            &f.actor.user_id,
            &result.pool,
            &instance,
            Some("contributor"),
            &credential
        )
        .await,
        Err(AppError::OracleWorkerCredentialRenewalRequired)
    ));
    let fresh = format!("nyx_owi_{}", "b".repeat(64));
    enrollment::enroll(
        &f.db,
        &f.actor.user_id,
        &result.pool,
        &instance,
        Some("contributor"),
        &fresh,
    )
    .await
    .unwrap();
    assert!(enrollment::authenticate(&f.db, &fresh).await.is_ok());
    assert_eq!(
        tasks::get_task_for_consumer(&f.db, &f.actor.user_id, &task.id)
            .await
            .unwrap()
            .0
            .id,
        task.id
    );
    assert!(
        sessions::get_session_for_consumer(
            &f.db,
            &f.actor.user_id,
            task.conversation_id.as_ref().unwrap()
        )
        .await
        .is_ok()
    );
    assert!(
        tasks::submit_task(&f.db, &f.pool, &identity(&f), prompt())
            .await
            .is_err()
    );
    assert!(
        workers::forget_authorized_worker(&f.db, &f.pool, &worker, &f.actor.user_id, true)
            .await
            .is_err()
    );
    let audits: Vec<Document> =
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find(doc! {"event_type":"oracle_pool_transferred"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(audits.len(), 1);
    let audit = format!("{:?}", audits[0]);
    assert!(!audit.contains(&f.token));
    assert!(!audit.contains("test body"));
    assert!(!audit.contains("credential_hash"));
    assert!(audits[0].contains_key("seq"));
}

#[tokio::test]
async fn oracle_lifecycle_transfer_acl_visibility_rotation_and_destination_limit() {
    let f = fixture("oracle_lifecycle_acl").await;
    let outsider = uuid::Uuid::new_v4().to_string();
    f.db.collection::<User>(USERS)
        .insert_one(test_user(&outsider, UserType::Person))
        .await
        .unwrap();
    let mut actor = f.actor.clone();
    actor.user_id = outsider.clone();
    assert!(matches!(
        transfer_pool(&f.db, &actor, &f.pool.id, transfer_input(&f)).await,
        Err(AppError::Forbidden(_))
    ));
    let wrong_person = TransferPoolInput {
        owner_user_id: outsider,
        ..Default::default()
    };
    assert!(matches!(
        transfer_pool(&f.db, &f.actor, &f.pool.id, wrong_person).await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        transfer_pool(
            &f.db,
            &f.actor,
            &f.pool.id,
            TransferPoolInput {
                owner_user_id: "platform".into(),
                ..Default::default()
            }
        )
        .await,
        Err(AppError::ValidationError(_))
    ));
    for n in 0..MAX_POOLS_PER_OWNER {
        create_pool(
            &f.db,
            &f.org,
            CreatePoolInput {
                slug: format!("limit-{n}"),
                name: "Limit".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    assert!(matches!(
        transfer_pool(&f.db, &f.actor, &f.pool.id, transfer_input(&f)).await,
        Err(AppError::Conflict(_))
    ));
    let limit_pool = get_pool(&f.db, "limit-0").await.unwrap();
    delete_pool(&f.db, &f.actor, &limit_pool.id).await.unwrap();
    let rotated = transfer_pool(
        &f.db,
        &f.actor,
        &f.pool.id,
        TransferPoolInput {
            rotate_worker_token: true,
            ..transfer_input(&f)
        },
    )
    .await
    .unwrap();
    assert!(validate_worker_token(&f.db, &f.token).await.is_err());
    assert!(
        validate_worker_token(&f.db, rotated.worker_token.as_ref().unwrap())
            .await
            .is_ok()
    );
    let org_pool = update_pool(
        &f.db,
        &f.actor.user_id,
        &f.pool.id,
        UpdatePoolInput {
            visibility: Some(OraclePoolVisibility::Org),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let personal = TransferPoolInput {
        owner_user_id: f.actor.user_id.clone(),
        ..Default::default()
    };
    assert!(matches!(
        transfer_pool(&f.db, &f.actor, &org_pool.id, personal.clone()).await,
        Err(AppError::ValidationError(_))
    ));
    let result = transfer_pool(
        &f.db,
        &f.actor,
        &org_pool.id,
        TransferPoolInput {
            visibility: Some(OraclePoolVisibility::Private),
            ..personal
        },
    )
    .await
    .unwrap();
    assert_eq!(result.pool.visibility, OraclePoolVisibility::Private);
    f.db.collection::<Document>(USERS)
        .update_one(doc! {"_id":&f.org}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(
        transfer_pool(&f.db, &f.actor, &f.pool.id, transfer_input(&f))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn oracle_lifecycle_delete_conflicts_then_retains_history_and_stably_acknowledges() {
    let f = fixture("oracle_lifecycle_delete").await;
    let identity = identity(&f);
    let task = tasks::submit_task(&f.db, &f.pool, &identity, prompt())
        .await
        .unwrap()
        .task;
    assert!(matches!(
        delete_pool(&f.db, &f.actor, &f.pool.id).await,
        Err(AppError::OraclePoolActiveWork)
    ));
    let cancelled = tasks::cancel_task(&f.db, &f.actor.user_id, &task.id, 30)
        .await
        .unwrap();
    let instance = uuid::Uuid::new_v4().to_string();
    workers::report_presence(
        &f.db,
        &f.pool,
        workers::WorkerPresenceInput {
            worker_label: "idle".into(),
            instance_id: Some(instance),
            capabilities: vec!["commands_v1".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let worker = workers::get_worker(&f.db, &f.pool.id, "idle")
        .await
        .unwrap();
    let command = workers::enqueue_worker_command(
        &f.db,
        &f.pool,
        &worker,
        &f.actor.user_id,
        crate::models::oracle_worker_command::OracleWorkerCommandKind::Drain,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(matches!(
        delete_pool(&f.db, &f.actor, &f.pool.id).await,
        Err(AppError::OraclePoolActiveWork)
    ));
    workers::cancel_worker_command(&f.db, &f.pool, &worker, &f.actor.user_id, &command.id)
        .await
        .unwrap();
    for collection in [
        crate::models::oracle_login_snapshot::COLLECTION_NAME,
        crate::models::oracle_login_profile::COLLECTION_NAME,
    ] {
        f.db.collection::<Document>(collection).insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"pool_id":&f.pool.id,"private_material":"retire"}).await.unwrap();
    }
    let first = delete_pool(&f.db, &f.actor, &f.pool.slug).await.unwrap();
    let second = delete_pool(&f.db, &f.actor, &f.pool.id).await.unwrap();
    assert_eq!(
        first.deleted_at.to_rfc3339(),
        second.deleted_at.to_rfc3339()
    );
    assert_eq!(first.id, second.id);
    assert_eq!(first.slug, second.slug);
    assert!(get_pool(&f.db, &f.pool.id).await.is_err());
    assert!(
        list_visible_pools(&f.db, &f.actor.user_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(validate_worker_token(&f.db, &f.token).await.is_err());
    assert!(matches!(
        create_pool(
            &f.db,
            &f.actor.user_id,
            CreatePoolInput {
                slug: f.pool.slug.clone(),
                name: "Recreate".into(),
                ..Default::default()
            }
        )
        .await,
        Err(AppError::OraclePoolSlugTaken(_))
    ));
    assert!(
        tasks::pool_status_with_retention(&f.db, &f.pool, 30)
            .await
            .is_err()
    );
    assert!(
        workers::report_presence(
            &f.db,
            &f.pool,
            workers::WorkerPresenceInput {
                worker_label: "idle".into(),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert!(
        tasks::claim_task_with_retention(&f.db, &f.pool, "stale", None, None, 30, None)
            .await
            .is_err()
    );
    for collection in [
        WORKERS,
        crate::models::oracle_login_snapshot::COLLECTION_NAME,
        crate::models::oracle_login_profile::COLLECTION_NAME,
    ] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {"pool_id":&f.pool.id})
                .await
                .unwrap(),
            0
        );
    }
    let retained = tasks::get_task_for_consumer(&f.db, &f.actor.user_id, &task.id)
        .await
        .unwrap()
        .0;
    assert_eq!(retained.expires_at, cancelled.expires_at);
    assert!(
        sessions::get_session_for_consumer(
            &f.db,
            &f.actor.user_id,
            task.conversation_id.as_ref().unwrap()
        )
        .await
        .unwrap()
        .closed_at
        .is_some()
    );
    assert_eq!(
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"oracle_pool_deleted"})
            .await
            .unwrap(),
        1
    );
    // Manager history uses the tombstone even for another person's submission.
    let reader = uuid::Uuid::new_v4().to_string();
    f.db.collection::<Document>(TASKS)
        .update_one(
            doc! {"_id":&task.id},
            doc! {"$set":{"submitter_user_id":&reader}},
        )
        .await
        .unwrap();
    f.db.collection::<Document>(SESSIONS)
        .update_one(
            doc! {"_id":task.conversation_id.as_ref().unwrap()},
            doc! {"$set":{"owner_user_id":&reader}},
        )
        .await
        .unwrap();
    assert!(
        tasks::get_task_for_consumer(&f.db, &f.actor.user_id, &task.id)
            .await
            .is_ok()
    );
    assert!(
        sessions::get_session_for_consumer(
            &f.db,
            &f.actor.user_id,
            task.conversation_id.as_ref().unwrap()
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn oracle_lifecycle_delete_audit_failure_rolls_back_and_retry_succeeds() {
    let f = fixture("oracle_lifecycle_rollback").await;
    f.db.create_collection(crate::models::audit_log::COLLECTION_NAME)
        .validator(doc! {"must_not_exist":true})
        .await
        .unwrap();
    workers::report_presence(
        &f.db,
        &f.pool,
        workers::WorkerPresenceInput {
            worker_label: "worker".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(delete_pool(&f.db, &f.actor, &f.pool.id).await.is_err());
    assert!(get_pool(&f.db, &f.pool.id).await.is_ok());
    assert!(validate_worker_token(&f.db, &f.token).await.is_ok());
    assert_eq!(
        f.db.collection::<Document>(WORKERS)
            .count_documents(doc! {"pool_id":&f.pool.id})
            .await
            .unwrap(),
        1
    );
    f.db.run_command(doc! {"collMod":crate::models::audit_log::COLLECTION_NAME,"validator":{}})
        .await
        .unwrap();
    delete_pool(&f.db, &f.actor, &f.pool.id).await.unwrap();
}

#[tokio::test]
async fn oracle_lifecycle_concurrent_submission_delete_and_idempotent_new_session() {
    let f = fixture("oracle_lifecycle_race").await;
    let identity = identity(&f);
    let input = tasks::SubmitTaskInput {
        client_ref: Some("one-request".into()),
        ..prompt()
    };
    let (one, two) = tokio::join!(
        tasks::submit_task(&f.db, &f.pool, &identity, input.clone()),
        tasks::submit_task(&f.db, &f.pool, &identity, input)
    );
    let one = one.unwrap();
    let two = two.unwrap();
    assert_eq!(one.task.id, two.task.id);
    assert_ne!(one.deduplicated, two.deduplicated);
    assert_eq!(
        f.db.collection::<Document>(SESSIONS)
            .count_documents(doc! {"pool_id":&f.pool.id})
            .await
            .unwrap(),
        1
    );
    tasks::cancel_task(&f.db, &f.actor.user_id, &one.task.id, 30)
        .await
        .unwrap();
    let (deleted, submitted) = tokio::join!(
        delete_pool(&f.db, &f.actor, &f.pool.id),
        tasks::submit_task(&f.db, &f.pool, &identity, prompt())
    );
    match (deleted, submitted) {
        (Ok(_), Err(_)) => assert_eq!(
            f.db.collection::<Document>(TASKS)
                .count_documents(doc! {"pool_id":&f.pool.id,"status":"queued"})
                .await
                .unwrap(),
            0
        ),
        (Err(AppError::OraclePoolActiveWork), Ok(_)) => {
            assert!(get_pool(&f.db, &f.pool.id).await.is_ok())
        }
        _ => panic!("submission and deletion must have one serial outcome"),
    }
}

#[tokio::test]
async fn oracle_lifecycle_cleanup_is_bounded_resumable_and_failure_safe() {
    let f = fixture("oracle_lifecycle_cleanup").await;
    let profiles = crate::models::oracle_login_profile::COLLECTION_NAME;
    let snapshots = crate::models::oracle_login_snapshot::COLLECTION_NAME;
    for n in 0..40 {
        for collection in [profiles, snapshots, WORKERS] {
            f.db.collection::<Document>(collection).insert_one(doc! {"_id":format!("{collection}-{n}"),"pool_id":&f.pool.id,"secret":"removed"}).await.unwrap();
        }
        f.db.collection::<Document>(SESSIONS)
            .insert_one(doc! {"_id":format!("session-{n}"),"pool_id":&f.pool.id,"closed_at":null})
            .await
            .unwrap();
    }
    let first = delete_pool(&f.db, &f.actor, &f.pool.id).await.unwrap();
    assert!(
        get_pool_for_history(&f.db, &f.pool.id)
            .await
            .unwrap()
            .deletion_cleanup_pending
    );
    assert_eq!(
        f.db.collection::<Document>(profiles)
            .count_documents(doc! {"pool_id":&f.pool.id})
            .await
            .unwrap(),
        24
    );
    // Fail a bounded chunk after other collections have been touched. Its
    // transaction rolls them all back and retains the pending marker.
    f.db.run_command(doc! {"collMod":SESSIONS,"validator":{"closed_at":null}})
        .await
        .unwrap();
    let before =
        f.db.collection::<Document>(profiles)
            .count_documents(doc! {})
            .await
            .unwrap();
    assert!(cleanup_chunk(&f.db, &f.pool.id).await.is_err());
    assert_eq!(
        f.db.collection::<Document>(profiles)
            .count_documents(doc! {})
            .await
            .unwrap(),
        before
    );
    f.db.run_command(doc! {"collMod":SESSIONS,"validator":{}})
        .await
        .unwrap();
    // A fresh handle/stateless invocation resumes solely from durable database state.
    let restarted = f.db.clone();
    cleanup_deleted_pools(&restarted).await.unwrap();
    assert!(
        get_pool_for_history(&f.db, &f.pool.id)
            .await
            .unwrap()
            .deletion_cleanup_pending
    );
    cleanup_deleted_pools(&restarted).await.unwrap();
    assert!(
        !get_pool_for_history(&f.db, &f.pool.id)
            .await
            .unwrap()
            .deletion_cleanup_pending
    );
    for collection in [profiles, snapshots, WORKERS] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {"pool_id":&f.pool.id})
                .await
                .unwrap(),
            0
        );
    }
    assert_eq!(
        f.db.collection::<Document>(SESSIONS)
            .count_documents(doc! {"pool_id":&f.pool.id,"closed_at":null})
            .await
            .unwrap(),
        0
    );
    let again = delete_pool(&restarted, &f.actor, &f.pool.id).await.unwrap();
    assert_eq!(first.deleted_at, again.deleted_at);
}

#[tokio::test]
async fn oracle_lifecycle_transfer_requires_both_orgs_active_actor_and_atomic_audit() {
    let f = fixture("oracle_lifecycle_both_orgs").await;
    transfer_pool(&f.db, &f.actor, &f.pool.id, transfer_input(&f))
        .await
        .unwrap();
    let other = uuid::Uuid::new_v4().to_string();
    f.db.collection::<User>(USERS)
        .insert_one(test_user(&other, UserType::Org))
        .await
        .unwrap();
    let input = TransferPoolInput {
        owner_user_id: other.clone(),
        ..Default::default()
    };
    assert!(matches!(
        transfer_pool(&f.db, &f.actor, &f.pool.id, input.clone()).await,
        Err(AppError::Forbidden(_))
    ));
    f.db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(test_membership(
            &other,
            &f.actor.user_id,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    f.db.collection::<Document>(MEMBERSHIPS)
        .update_one(doc! {"org_user_id":&f.org}, doc! {"$set":{"role":"member"}})
        .await
        .unwrap();
    assert!(matches!(
        transfer_pool(&f.db, &f.actor, &f.pool.id, input.clone()).await,
        Err(AppError::Forbidden(_))
    ));
    f.db.collection::<Document>(MEMBERSHIPS)
        .update_one(doc! {"org_user_id":&f.org}, doc! {"$set":{"role":"admin"}})
        .await
        .unwrap();
    f.db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&f.actor.user_id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    assert!(matches!(
        transfer_pool(&f.db, &f.actor, &f.pool.id, input.clone()).await,
        Err(AppError::Forbidden(_))
    ));
    f.db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&f.actor.user_id},
            doc! {"$set":{"is_active":true}},
        )
        .await
        .unwrap();
    let old = get_pool(&f.db, &f.pool.id).await.unwrap();
    f.db.run_command(
        doc! {"collMod":crate::models::audit_log::COLLECTION_NAME,"validator":{"reject":true}},
    )
    .await
    .unwrap();
    assert!(
        transfer_pool(
            &f.db,
            &f.actor,
            &f.pool.id,
            TransferPoolInput {
                rotate_worker_token: true,
                ..input.clone()
            }
        )
        .await
        .is_err()
    );
    let unchanged = get_pool(&f.db, &f.pool.id).await.unwrap();
    assert_eq!(old.user_id, unchanged.user_id);
    assert_eq!(old.worker_token_hash, unchanged.worker_token_hash);
    assert_eq!(old.lifecycle_version, unchanged.lifecycle_version);
    f.db.run_command(doc! {"collMod":crate::models::audit_log::COLLECTION_NAME,"validator":{}})
        .await
        .unwrap();
    assert_eq!(
        transfer_pool(&f.db, &f.actor, &f.pool.id, input)
            .await
            .unwrap()
            .pool
            .user_id,
        other
    );
}

#[tokio::test]
async fn oracle_lifecycle_dispatched_and_delivered_work_conflicts() {
    let f = fixture("oracle_lifecycle_active").await;
    let task = tasks::submit_task(&f.db, &f.pool, &identity(&f), prompt())
        .await
        .unwrap()
        .task;
    tasks::claim_task_with_retention(&f.db, &f.pool, "busy", None, None, 30, None)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        delete_pool(&f.db, &f.actor, &f.pool.id).await,
        Err(AppError::OraclePoolActiveWork)
    ));
    tasks::cancel_task(&f.db, &f.actor.user_id, &task.id, 30)
        .await
        .unwrap();
    let worker = workers::report_presence(
        &f.db,
        &f.pool,
        workers::WorkerPresenceInput {
            worker_label: "busy".into(),
            capabilities: vec!["commands_v1".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let first = workers::enqueue_worker_command(
        &f.db,
        &f.pool,
        &worker,
        &f.actor.user_id,
        crate::models::oracle_worker_command::OracleWorkerCommandKind::Drain,
        None,
        None,
    )
    .await
    .unwrap();
    let second = workers::enqueue_worker_command(
        &f.db,
        &f.pool,
        &worker,
        &f.actor.user_id,
        crate::models::oracle_worker_command::OracleWorkerCommandKind::Resume,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        workers::deliver_next_command_for_worker(&f.db, &f.pool, &worker, &worker.capabilities)
            .await
            .unwrap()
            .unwrap()
            .id,
        first.id
    );
    assert!(
        workers::deliver_next_command_for_worker(&f.db, &f.pool, &worker, &worker.capabilities)
            .await
            .unwrap()
            .is_none()
    );
    workers::cancel_worker_command(&f.db, &f.pool, &worker, &f.actor.user_id, &second.id)
        .await
        .unwrap();
    assert!(matches!(
        delete_pool(&f.db, &f.actor, &f.pool.id).await,
        Err(AppError::OraclePoolActiveWork)
    ));
    workers::cancel_worker_command(&f.db, &f.pool, &worker, &f.actor.user_id, &first.id)
        .await
        .unwrap();
    delete_pool(&f.db, &f.actor, &f.pool.id).await.unwrap();
}

#[tokio::test]
async fn oracle_lifecycle_held_transfer_rejects_stale_enrollment_dispatch_heartbeat_and_manager() {
    let f = fixture("oracle_lifecycle_held").await;
    let credential = format!("nyx_owi_{}", "c".repeat(64));
    let instance = uuid::Uuid::new_v4().to_string();
    let worker = enrollment::enroll(
        &f.db,
        &f.actor.user_id,
        &f.pool,
        &instance,
        Some("installed"),
        &credential,
    )
    .await
    .unwrap();
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    transfer_in_session(
        &f.db,
        &mut session,
        &f.actor,
        &f.pool.id,
        &transfer_input(&f),
    )
    .await
    .unwrap();
    // All operations start from authenticated pre-transfer snapshots while the
    // uncommitted lifecycle transaction owns the database fence.
    let db = f.db.clone();
    let pool = f.pool.clone();
    let actor = f.actor.user_id.clone();
    let mut requests = tokio::spawn(async move {
        let fresh = format!("nyx_owi_{}", "d".repeat(64));
        let new_instance = uuid::Uuid::new_v4().to_string();
        tokio::join!(
            enrollment::enroll(&db, &actor, &pool, &new_instance, Some("new"), &fresh),
            tasks::claim_task_with_retention(
                &db,
                &pool,
                "installed",
                None,
                None,
                30,
                Some(&worker)
            ),
            workers::report_presence_for_worker(
                &db,
                &pool,
                workers::WorkerPresenceInput {
                    worker_label: "installed".into(),
                    ..Default::default()
                },
                Some(&worker)
            ),
            workers::enqueue_worker_command(
                &db,
                &pool,
                &worker,
                &actor,
                crate::models::oracle_worker_command::OracleWorkerCommandKind::Drain,
                None,
                None
            )
        )
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut requests)
            .await
            .is_err()
    );
    session.commit_transaction().await.unwrap();
    let (enrolled, dispatched, heartbeat, managed) = requests.await.unwrap();
    assert!(enrolled.is_err());
    assert!(dispatched.is_err());
    assert!(heartbeat.is_err());
    assert!(managed.is_err());
    assert_eq!(
        f.db.collection::<Document>(WORKERS)
            .count_documents(doc! {"pool_id":&f.pool.id})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.db.collection::<Document>(crate::models::oracle_worker_command::COLLECTION_NAME)
            .count_documents(doc! {"pool_id":&f.pool.id})
            .await
            .unwrap(),
        0
    );
}

mod http;

#[tokio::test]
async fn oracle_lifecycle_held_delete_fences_all_stale_writers_and_transfer() {
    let f = fixture("oracle_lifecycle_held_delete").await;
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    delete_in_session(&f.db, &mut session, &f.actor, &f.pool.id)
        .await
        .unwrap();
    let db = f.db.clone();
    let pool = f.pool.clone();
    let actor = f.actor.clone();
    let input = transfer_input(&f);
    let mut requests = tokio::spawn(async move {
        let credential = format!("nyx_owi_{}", "e".repeat(64));
        let instance = uuid::Uuid::new_v4().to_string();
        tokio::join!(
            enrollment::enroll(
                &db,
                &actor.user_id,
                &pool,
                &instance,
                Some("new"),
                &credential
            ),
            tasks::claim_task_with_retention(&db, &pool, "new", None, None, 30, None),
            workers::report_presence(
                &db,
                &pool,
                workers::WorkerPresenceInput {
                    worker_label: "new".into(),
                    ..Default::default()
                }
            ),
            transfer_pool(&db, &actor, &pool.id, input)
        )
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut requests)
            .await
            .is_err()
    );
    session.commit_transaction().await.unwrap();
    let (enrolled, dispatched, heartbeat, transferred) = requests.await.unwrap();
    assert!(enrolled.is_err());
    assert!(dispatched.is_err());
    assert!(heartbeat.is_err());
    assert!(transferred.is_err());
    cleanup_deleted_pools(&f.db).await.unwrap();
    assert_eq!(
        f.db.collection::<Document>(WORKERS)
            .count_documents(doc! {"pool_id":&f.pool.id})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn oracle_lifecycle_pin_revalidates_held_membership_revocation() {
    let f = fixture("oracle_lifecycle_pin_revoke").await;
    let pool = transfer_pool(&f.db, &f.actor, &f.pool.id, transfer_input(&f))
        .await
        .unwrap()
        .pool;
    let credential = format!("nyx_owi_{}", "f".repeat(64));
    let instance = uuid::Uuid::new_v4().to_string();
    let worker = enrollment::enroll(
        &f.db,
        &f.actor.user_id,
        &pool,
        &instance,
        Some("pin"),
        &credential,
    )
    .await
    .unwrap();
    let task = tasks::submit_task(&f.db, &pool, &identity(&f), prompt())
        .await
        .unwrap()
        .task;
    let claim =
        tasks::claim_task_with_retention(&f.db, &pool, "pin", None, None, 30, Some(&worker))
            .await
            .unwrap()
            .unwrap();
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    f.db.collection::<Document>(MEMBERSHIPS)
        .update_one(
            doc! {"org_user_id":&f.org,"member_user_id":&f.actor.user_id},
            doc! {"$set":{"revoked_at":bson::DateTime::now()}},
        )
        .session(&mut session)
        .await
        .unwrap();
    let db = f.db.clone();
    let task_id = task.id.clone();
    let mut request = tokio::spawn(async move {
        tasks::pin_conversation_url_fenced(
            &db,
            &pool,
            tasks::PinConversationInput {
                worker_label: "pin",
                task_id: &task_id,
                chatgpt_url: "https://chatgpt.com/c/stale-membership",
                dispatch_attempt_id: claim.dispatch_attempt_id.as_deref(),
                authorized_worker: Some(&worker),
            },
        )
        .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut request)
            .await
            .is_err()
    );
    session.commit_transaction().await.unwrap();
    assert!(request.await.unwrap().is_err());
    let stored =
        f.db.collection::<crate::models::oracle_task::OracleTask>(TASKS)
            .find_one(doc! {"_id":&task.id})
            .await
            .unwrap()
            .unwrap();
    assert!(stored.chatgpt_url.is_none());
    let session =
        f.db.collection::<Document>(SESSIONS)
            .find_one(doc! {"_id":task.conversation_id.unwrap()})
            .await
            .unwrap()
            .unwrap();
    assert!(
        session
            .get("chatgpt_url")
            .is_none_or(|value| value.as_str() != Some("https://chatgpt.com/c/stale-membership"))
    );
}

#[tokio::test]
async fn oracle_lifecycle_duplicate_submission_requires_current_active_authority() {
    let f = fixture("oracle_lifecycle_dedupe_authority").await;
    let input = tasks::SubmitTaskInput {
        client_ref: Some("duplicate-authority".into()),
        ..prompt()
    };
    let original = tasks::submit_task(&f.db, &f.pool, &identity(&f), input.clone())
        .await
        .unwrap();
    f.db.collection::<Document>(USERS)
        .update_one(
            doc! {"_id":&f.actor.user_id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    assert!(matches!(
        tasks::submit_task(&f.db, &f.pool, &identity(&f), input).await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(
        f.db.collection::<Document>(TASKS)
            .count_documents(doc! {"pool_id":&f.pool.id})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.db.collection::<Document>(SESSIONS)
            .count_documents(doc! {"pool_id":&f.pool.id})
            .await
            .unwrap(),
        1
    );
    assert!(!original.deduplicated);
}

#[tokio::test]
async fn oracle_lifecycle_cleanup_scheduler_does_not_starve_later_tombstones() {
    let f = fixture("oracle_lifecycle_cleanup_fairness").await;
    let mut ids = Vec::new();
    // Seed tombstones directly: this tests global scheduling, independent of
    // the per-owner live-inventory cap (tombstones do not count against it).
    for n in 0..18 {
        let id = uuid::Uuid::new_v4().to_string();
        ids.push(id.clone());
        let mut pool = f.pool.clone();
        pool.id = id.clone();
        pool.slug = format!("pending-{n}");
        pool.deleted_at = Some(Utc::now());
        pool.is_active = false;
        pool.deletion_cleanup_pending = true;
        f.db.collection::<OraclePool>(ORACLE_POOLS)
            .insert_one(pool)
            .await
            .unwrap();
        f.db.collection::<Document>(WORKERS)
            .insert_many((0..40).map(|row| doc! {"_id":format!("{id}:{row}"),"pool_id":&id}))
            .await
            .unwrap();
    }
    cleanup_deleted_pools(&f.db).await.unwrap();
    assert_eq!(
        f.db.collection::<Document>(ORACLE_POOLS)
            .count_documents(
                doc! {"deletion_cleanup_pending":true,"cleanup_attempted_at":{"$exists":true}}
            )
            .await
            .unwrap(),
        16
    );
    cleanup_deleted_pools(&f.db).await.unwrap();
    for id in ids {
        assert!(
            f.db.collection::<Document>(WORKERS)
                .count_documents(doc! {"pool_id":id})
                .await
                .unwrap()
                < 40
        );
    }
}

#[tokio::test]
async fn oracle_lifecycle_delete_durably_expires_orphan_commands_but_preserves_unexpired_work() {
    let f = fixture("oracle_lifecycle_orphan_expiry").await;
    let commands =
        f.db.collection::<Document>(crate::models::oracle_worker_command::COLLECTION_NAME);
    let past = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(1));
    let future = bson::DateTime::from_chrono(Utc::now() + chrono::Duration::hours(1));
    // No worker row remains for this generation. More than the whole request budget
    // proves expiry progresses independently of the later active-work conflict.
    let budget = DELETE_COMMAND_EXPIRY_BATCH_SIZE as usize * DELETE_COMMAND_EXPIRY_MAX_BATCHES;
    let orphan_count = budget + 1;
    let mut rows: Vec<_> = (0..orphan_count)
        .map(|i| {
            doc! {
                "_id":format!("orphan-{i:03}"),"pool_id":&f.pool.id,"worker_label":"removed",
                "worker_generation":"old-generation","status":"queued","deadline_at":past,
            }
        })
        .collect();
    rows.push(
        doc! {"_id":"live-orphan","pool_id":&f.pool.id,"worker_label":"removed",
        "worker_generation":"old-generation","status":"delivered","deadline_at":future},
    );
    commands.insert_many(rows).await.unwrap();
    let stranger = uuid::Uuid::new_v4().to_string();
    f.db.collection::<User>(USERS)
        .insert_one(test_user(&stranger, UserType::Person))
        .await
        .unwrap();
    let mut actor = f.actor.clone();
    actor.user_id = stranger;
    assert!(matches!(
        delete_pool(&f.db, &actor, &f.pool.id).await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(
        commands
            .count_documents(doc! {"status":"expired"})
            .await
            .unwrap(),
        0
    );
    assert!(matches!(
        delete_pool(&f.db, &f.actor, &f.pool.id).await,
        Err(AppError::OraclePoolActiveWork)
    ));
    assert_eq!(
        commands
            .count_documents(doc! {"status":"expired","expires_at":{"$type":"date"}})
            .await
            .unwrap(),
        budget as u64
    );
    assert!(matches!(
        delete_pool(&f.db, &f.actor, &f.pool.id).await,
        Err(AppError::OraclePoolActiveWork)
    ));
    assert_eq!(
        commands
            .count_documents(doc! {"status":"expired"})
            .await
            .unwrap(),
        orphan_count as u64
    );
    assert_eq!(
        commands
            .find_one(doc! {"_id":"live-orphan"})
            .await
            .unwrap()
            .unwrap()
            .get_str("status")
            .unwrap(),
        "delivered"
    );
    assert!(get_pool(&f.db, &f.pool.id).await.is_ok());
    assert!(validate_worker_token(&f.db, &f.token).await.is_ok());
    // Prior batches remain committed after the active-work conflict. A fresh
    // invocation handles the final deadline; this edit simulates time passing.
    commands
        .update_one(
            doc! {"_id":"live-orphan"},
            doc! {"$set":{"deadline_at":past}},
        )
        .await
        .unwrap();
    delete_pool(&f.db.clone(), &f.actor, &f.pool.id)
        .await
        .unwrap();
    assert_eq!(
        commands
            .count_documents(doc! {"status":"expired"})
            .await
            .unwrap(),
        (orphan_count + 1) as u64
    );
    assert_eq!(
        commands
            .count_documents(doc! {"status":{"$in":["queued","delivered"]}})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn cleanup_evidence_is_majority_durable_before_an_independent_retry() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let commands = Arc::new(Mutex::new(Vec::<Document>::new()));
    let recorded = commands.clone();
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "oracle_cleanup_durability",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                recorded.lock().unwrap().push(event.command);
            }
        }),
    )
    .await
    .expect("isolated MongoDB required");
    let owner = uuid::Uuid::new_v4().to_string();
    let pool = uuid::Uuid::new_v4().to_string();
    let worker = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>(ORACLE_POOLS)
        .insert_one(doc! {
            "_id":&pool,"user_id":&owner,"deleted_at":bson::DateTime::now(),
            "deletion_cleanup_pending":true,"worker_enrollment_epoch":0_i64,
        })
        .await
        .unwrap();
    db.collection::<Document>(WORKERS)
        .insert_one(doc! {"_id":&worker,"pool_id":&pool})
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    // Model loss of the scheduler after an effect commits: the next pass uses
    // a new database handle and derives completion from durable lifecycle rows.
    assert!(!personal_erasure_pass(&db, &owner, &owner).await.unwrap());
    let next = db.client().database(db.name());
    assert!(!personal_erasure_pass(&next, &owner, &owner).await.unwrap());
    assert!(personal_erasure_pass(&next, &owner, &owner).await.unwrap());
    let observed = commands.lock().unwrap().clone();
    let commits: Vec<_> = observed
        .iter()
        .filter(|c| c.contains_key("commitTransaction"))
        .collect();
    assert_eq!(commits.len(), 2);
    for commit in commits {
        assert_eq!(
            commit
                .get_document("writeConcern")
                .unwrap()
                .get_str("w")
                .unwrap(),
            "majority"
        );
    }
    for first in observed
        .iter()
        .filter(|c| c.get_bool("startTransaction") == Ok(true))
    {
        assert_eq!(
            first
                .get_document("readConcern")
                .unwrap()
                .get_str("level")
                .unwrap(),
            "snapshot"
        );
    }
    let reads: Vec<_> = observed
        .iter()
        .filter(|c| c.contains_key("find") && !c.contains_key("txnNumber"))
        .collect();
    assert!(!reads.is_empty());
    for read in reads {
        assert_eq!(
            read.get_document("readConcern")
                .unwrap()
                .get_str("level")
                .unwrap(),
            "majority"
        );
    }
    drop(observed);
    db.drop().await.unwrap();
}
