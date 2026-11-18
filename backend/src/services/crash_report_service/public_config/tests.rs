use super::*;
use crate::{
    models::feature_flag_override::{COLLECTION_NAME, FeatureFlagOverride, FlagTargetKind},
    services::{
        audit_service::AuditActor,
        feature_flag_admin_service::{self as flags, Mutation},
        security_readiness_service as readiness,
    },
    test_utils::*,
};
use mongodb::bson::{Document, doc};

fn global() -> FeatureFlagOverride {
    FeatureFlagOverride {
        id: uuid::Uuid::new_v4().to_string(),
        org_user_id: None,
        definition_id: None,
        flag_key: super::super::FLAG.into(),
        target_kind: FlagTargetKind::Global,
        target_key: None,
        enabled: true,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        updated_by: uuid::Uuid::new_v4().to_string(),
    }
}

async fn wait_offered(db: &Database, expected: bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while offered(db) != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn crash_public_hint_evaluates_at_startup_flips_on_and_off_and_stops_on_drain() {
    let db = connect_test_database("crash_public_hint_worker")
        .await
        .unwrap();
    let row = global();
    let overrides = db.collection::<FeatureFlagOverride>(COLLECTION_NAME);
    overrides.insert_one(&row).await.unwrap();
    assert!(
        !offered(&db),
        "false until the first successful rollout read"
    );
    let shutdown = Runtime::default();
    let worker = shutdown.spawn_worker(run_every(
        db.clone(),
        shutdown.clone(),
        Duration::from_millis(20),
    ));
    wait_offered(&db, true).await;
    // Writes from another replica do not run this process's invalidation path.
    overrides
        .update_one(doc! {"_id": &row.id}, doc! {"$set": {"enabled": false}})
        .await
        .unwrap();
    wait_offered(&db, false).await;
    overrides
        .update_one(doc! {"_id": &row.id}, doc! {"$set": {"enabled": true}})
        .await
        .unwrap();
    wait_offered(&db, true).await;
    shutdown.begin_drain();
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(shutdown.counts().background, 0);
    assert_eq!(shutdown.tracked_units(), 0);
    overrides
        .update_one(doc! {"_id": &row.id}, doc! {"$set": {"enabled": false}})
        .await
        .unwrap();
    run_every(db.clone(), shutdown, Duration::from_millis(1)).await;
    assert!(offered(&db), "drained workers never evaluate another pass");
    assert_eq!(REFRESH_INTERVAL, Duration::from_secs(60));
}

#[tokio::test]
async fn crash_public_hint_keeps_last_value_on_rollout_errors_and_timeout() {
    let db = connect_test_database("crash_public_hint_failure")
        .await
        .unwrap();
    refresh_from(
        &db,
        std::future::ready(Err(crate::errors::AppError::Internal(
            "test rollout failure".into(),
        ))),
    )
    .await;
    assert!(!offered(&db));
    let row = global();
    db.collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .insert_one(&row)
        .await
        .unwrap();
    refresh(&db).await;
    assert!(offered(&db));
    let malformed = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>(COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &malformed, "flag_key": super::super::FLAG, "enabled": "invalid"
        })
        .await
        .unwrap();
    refresh(&db).await;
    assert!(offered(&db), "a failed rollout read is not an off decision");
    tokio::time::pause();
    refresh_from(&db, std::future::pending()).await;
    assert!(offered(&db), "a deadline retains the last value too");
    tokio::time::resume();
    db.collection::<Document>(COLLECTION_NAME)
        .delete_one(doc! {"_id": malformed})
        .await
        .unwrap();
    db.collection::<Document>(COLLECTION_NAME)
        .delete_one(doc! {"_id": row.id})
        .await
        .unwrap();
    refresh(&db).await;
    assert!(!offered(&db));
}

#[tokio::test]
async fn crash_public_hint_local_admin_writes_update_immediately_including_clear_and_kill() {
    let (db, id, _) = super::super::tests::account("crash_public_hint_admin").await;
    readiness::ready_test_replica(&db).await;
    let actor = AuditActor {
        user_id: id.clone(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    assert!(!offered(&db));
    for (kind, enabled, expected) in [
        (FlagTargetKind::User, Some(true), true),
        (FlagTargetKind::Global, Some(false), false),
        (FlagTargetKind::Global, None, true),
        (FlagTargetKind::User, None, false),
        (FlagTargetKind::Global, Some(true), true),
        (FlagTargetKind::Global, Some(false), false),
    ] {
        flags::mutate(
            &db,
            super::super::FLAG,
            Mutation::Override {
                kind,
                target_id: (kind == FlagTargetKind::User).then(|| id.clone()),
                enabled,
            },
            &actor,
        )
        .await
        .unwrap();
        assert_eq!(offered(&db), expected, "updated before admin write returns");
    }
}

#[tokio::test]
async fn crash_public_hint_old_background_read_cannot_overwrite_local_invalidation() {
    let db = connect_test_database("crash_public_hint_fence")
        .await
        .unwrap();
    let read = async {
        invalidate(&db).await;
        Ok(admin::Rollout::Global)
    };
    refresh_from(&db, read).await;
    assert!(
        !offered(&db),
        "invalidation fenced the older background result"
    );
}
