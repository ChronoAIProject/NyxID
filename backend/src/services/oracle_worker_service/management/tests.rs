use super::*;
use crate::models::oracle_pool::OraclePoolVisibility;
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::services::oracle_worker_enrollment_service as enrollment;
use crate::test_utils::{connect_transaction_test_database, test_membership, test_user};

async fn fixture() -> (Database, OraclePool, String, String) {
    let db = connect_transaction_test_database("oracle_worker_incarnation").await;
    enrollment::ensure_indexes(&db).await.unwrap();
    let org = uuid::Uuid::new_v4().to_string();
    let first = uuid::Uuid::new_v4().to_string();
    let second = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_many([
            test_user(&org, UserType::Org),
            test_user(&first, UserType::Person),
            test_user(&second, UserType::Person),
        ])
        .await
        .unwrap();
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_many([
            test_membership(&org, &first, OrgRole::Member, None),
            test_membership(&org, &second, OrgRole::Member, None),
        ])
        .await
        .unwrap();
    let (pool, _) = oracle_pool_service::create_pool(
        &db,
        &org,
        oracle_pool_service::CreatePoolInput {
            slug: "incarnation".into(),
            name: "Incarnation".into(),
            visibility: Some(OraclePoolVisibility::Org),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    (db, pool, first, second)
}

async fn enrolled(db: &Database, pool: &OraclePool, actor: &str, instance: &str) -> OracleWorker {
    let credential = format!("nyx_owi_{}", hex::encode(rand::random::<[u8; 32]>()));
    enrollment::enroll(db, actor, pool, instance, Some("same-label"), &credential)
        .await
        .unwrap();
    report_presence(
        db,
        pool,
        WorkerPresenceInput {
            worker_label: "same-label".into(),
            instance_id: Some(instance.into()),
            capabilities: vec!["commands_v1".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn oracle_enrollment_management_aba_cannot_control_replacement_or_migrate_commands() {
    let (db, pool, first, second) = fixture().await;
    let instance = uuid::Uuid::new_v4().to_string();
    let authorized = enrolled(&db, &pool, &first, &instance).await;
    enrollment::ensure_can_manage_worker(&db, &first, &pool, &authorized)
        .await
        .unwrap();
    let stale_command = enqueue_worker_command(
        &db,
        &authorized,
        &first,
        OracleWorkerCommandKind::Drain,
        None,
        None,
    )
    .await
    .unwrap();
    forget_authorized_worker(&db, &authorized, true)
        .await
        .unwrap();
    // Reuse both the public label and client-chosen installation ID under another member.
    let replacement = enrolled(&db, &pool, &second, &instance).await;
    assert_ne!(authorized.generation, replacement.generation);
    let command = enqueue_worker_command(
        &db,
        &replacement,
        &second,
        OracleWorkerCommandKind::Drain,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        enrollment::ensure_can_manage_worker(&db, &first, &pool, &replacement)
            .await
            .is_err()
    );
    assert!(
        enqueue_worker_command(
            &db,
            &authorized,
            &first,
            OracleWorkerCommandKind::Resume,
            None,
            None
        )
        .await
        .is_err()
    );
    assert!(
        cancel_worker_command(&db, &authorized, &command.id)
            .await
            .is_err()
    );
    assert!(
        forget_authorized_worker(&db, &authorized, true)
            .await
            .is_err()
    );
    assert!(
        list_worker_commands(&db, &authorized)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        get_worker(&db, &pool.id, "same-label")
            .await
            .unwrap()
            .generation,
        replacement.generation
    );
    assert_eq!(
        list_worker_commands(&db, &replacement).await.unwrap()[0].id,
        command.id
    );
    // Even an orphaned command from an older writer cannot migrate to the replacement.
    db.collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
        .insert_one(&stale_command)
        .await
        .unwrap();
    let delivered = deliver_next_command(&db, &pool.id, "same-label", &["commands_v1".into()])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(delivered.id, command.id);
    assert!(
        deliver_next_command(&db, &pool.id, "same-label", &["commands_v1".into()])
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
            .find_one(doc! { "_id": stale_command.id })
            .await
            .unwrap()
            .unwrap()
            .status,
        OracleWorkerCommandStatus::Queued
    );
}

#[tokio::test]
async fn oracle_enrollment_forget_rolls_back_every_collection_together() {
    let (db, pool, first, _) = fixture().await;
    let worker = enrolled(&db, &pool, &first, &uuid::Uuid::new_v4().to_string()).await;
    let command = enqueue_worker_command(
        &db,
        &worker,
        &first,
        OracleWorkerCommandKind::Drain,
        None,
        None,
    )
    .await
    .unwrap();
    db.collection::<Document>(ORACLE_SESSIONS)
        .insert_one(
            doc! { "_id":"session", "pool_id":&pool.id, "owner_worker_label":&worker.worker_label },
        )
        .await
        .unwrap();
    db.collection::<Document>(ORACLE_TASKS).insert_one(doc! { "_id":"task", "pool_id":&pool.id, "status":"queued", "required_worker_label":&worker.worker_label }).await.unwrap();
    db.collection::<Document>(crate::models::oracle_login_profile::COLLECTION_NAME).insert_one(doc! {
        "_id":"profile", "pool_id":&pool.id, "bindings":[{ "worker_label":&worker.worker_label }],
    }).await.unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let outcome = forget_in_session(&db, &mut session, &worker, true)
        .await
        .unwrap();
    assert_eq!(outcome.commands_removed, 1);
    assert_eq!(outcome.sessions_released, 1);
    assert_eq!(outcome.tasks_released, 1);
    session.abort_transaction().await.unwrap();
    assert!(
        get_worker(&db, &pool.id, &worker.worker_label)
            .await
            .is_ok()
    );
    assert!(
        db.collection::<Document>(ORACLE_WORKER_COMMANDS)
            .find_one(doc! { "_id": &command.id })
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        db.collection::<Document>(ORACLE_SESSIONS)
            .find_one(doc! { "_id":"session" })
            .await
            .unwrap()
            .unwrap()
            .get_str("owner_worker_label")
            .unwrap(),
        worker.worker_label
    );
    assert_eq!(
        db.collection::<Document>(ORACLE_TASKS)
            .find_one(doc! { "_id":"task" })
            .await
            .unwrap()
            .unwrap()
            .get_str("required_worker_label")
            .unwrap(),
        worker.worker_label
    );
    assert_eq!(
        db.collection::<Document>(crate::models::oracle_login_profile::COLLECTION_NAME)
            .find_one(doc! { "_id":"profile" })
            .await
            .unwrap()
            .unwrap()
            .get_array("bindings")
            .unwrap()
            .len(),
        1
    );
    forget_authorized_worker(&db, &worker, true).await.unwrap();
    assert!(
        get_worker(&db, &pool.id, &worker.worker_label)
            .await
            .is_err()
    );
    assert!(
        db.collection::<Document>(ORACLE_WORKER_COMMANDS)
            .find_one(doc! { "_id": &command.id })
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn oracle_enrollment_legacy_none_generation_commands_stay_compatible_without_label_aba() {
    let (db, pool, actor, _) = fixture().await;
    let mut worker = provisioned_worker(&pool, "legacy", Utc::now());
    worker.generation = None;
    worker.capabilities = vec!["commands_v1".into()];
    db.collection::<OracleWorker>(ORACLE_WORKERS)
        .insert_one(&worker)
        .await
        .unwrap();
    let command = enqueue_worker_command(
        &db,
        &worker,
        &actor,
        OracleWorkerCommandKind::Drain,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(command.worker_generation.is_none());
    assert_eq!(
        deliver_next_command(&db, &pool.id, "legacy", &worker.capabilities)
            .await
            .unwrap()
            .unwrap()
            .id,
        command.id
    );
    // Simulate a legacy writer deleting only presence, leaving its old command behind.
    db.collection::<Document>(ORACLE_WORKERS)
        .delete_one(doc! { "_id": &worker.id })
        .await
        .unwrap();
    let replacement = report_presence(
        &db,
        &pool,
        WorkerPresenceInput {
            worker_label: "legacy".into(),
            instance_id: Some(uuid::Uuid::new_v4().to_string()),
            capabilities: worker.capabilities.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(replacement.generation.is_some());
    db.collection::<Document>(ORACLE_WORKER_COMMANDS)
        .update_one(
            doc! { "_id": &command.id },
            doc! { "$set": { "status":"queued" } },
        )
        .await
        .unwrap();
    assert!(
        deliver_next_command(&db, &pool.id, "legacy", &worker.capabilities)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        get_worker(&db, &pool.id, "legacy")
            .await
            .unwrap()
            .desired_state,
        OracleWorkerDesiredState::Active
    );
}

#[tokio::test]
async fn oracle_enrollment_stale_authenticated_heartbeat_cannot_resurrect_or_reach_replacement() {
    let (db, pool, first, second) = fixture().await;
    let instance = uuid::Uuid::new_v4().to_string();
    let authorized = enrolled(&db, &pool, &first, &instance).await;
    let auth = enrollment::WorkerAuth {
        pool: pool.clone(),
        installation: Some(authorized.clone()),
    };
    forget_authorized_worker(&db, &authorized, true)
        .await
        .unwrap();
    let input = || WorkerPresenceInput {
        worker_label: authorized.worker_label.clone(),
        instance_id: Some(instance.clone()),
        capabilities: vec!["commands_v1".into()],
        logged_in: Some(false),
        ..Default::default()
    };
    assert!(
        report_presence_for_worker(&db, &pool, input(), Some(&authorized))
            .await
            .is_err()
    );
    assert!(
        auth.ensure_current_identity(&db, &authorized.worker_label, Some(&instance))
            .await
            .is_err()
    );
    assert!(
        get_worker(&db, &pool.id, &authorized.worker_label)
            .await
            .is_err()
    );
    assert!(
        crate::services::oracle_task_service::claim_task_with_retention(
            &db,
            &pool,
            &authorized.worker_label,
            None,
            None,
            30,
            Some(&authorized)
        )
        .await
        .is_err()
    );
    assert!(
        get_worker(&db, &pool.id, &authorized.worker_label)
            .await
            .is_err()
    );
    let replacement = enrolled(&db, &pool, &second, &instance).await;
    let command = enqueue_worker_command(
        &db,
        &replacement,
        &second,
        OracleWorkerCommandKind::Drain,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        report_presence_for_worker(&db, &pool, input(), Some(&authorized))
            .await
            .is_err()
    );
    assert!(
        deliver_next_command_for_worker(&db, &authorized, &authorized.capabilities)
            .await
            .is_err()
    );
    assert!(
        apply_command_reports_for_worker(
            &db,
            &authorized,
            vec![CommandReport {
                command_id: command.id.clone(),
                succeeded: true,
                result_code: None,
            }]
        )
        .await
        .is_err()
    );
    assert!(
        recently_cancelled_delivered_for_worker(&db, &authorized)
            .await
            .is_err()
    );
    let current = get_worker(&db, &pool.id, &replacement.worker_label)
        .await
        .unwrap();
    assert_eq!(current.generation, replacement.generation);
    assert_eq!(current.logged_in, replacement.logged_in);
    assert_eq!(
        db.collection::<OracleWorkerCommand>(ORACLE_WORKER_COMMANDS)
            .find_one(doc! { "_id": &command.id })
            .await
            .unwrap()
            .unwrap()
            .status,
        OracleWorkerCommandStatus::Queued
    );
    assert_eq!(
        deliver_next_command_for_worker(&db, &replacement, &replacement.capabilities)
            .await
            .unwrap()
            .unwrap()
            .id,
        command.id
    );
}
