use chrono::Utc;
use futures::future::join_all;
use mongodb::bson::{Document, doc};
use uuid::Uuid;

use crate::models::service_pool::CooldownPolicy;
use crate::models::service_pool_member_health::COLLECTION_NAME as HEALTH;
use crate::services::service_pool_health_service::{
    self as health, HealthScope, ObservationTicket,
};

async fn fixture(label: &str) -> (mongodb::Database, HealthScope, HealthScope) {
    let db = crate::test_utils::connect_transaction_test_database(label).await;
    crate::db::ensure_indexes(&db)
        .await
        .expect("production indexes");
    let pool_id = Uuid::new_v4().to_string();
    let owner_id = Uuid::new_v4().to_string();
    let first_id = Uuid::new_v4().to_string();
    let second_id = Uuid::new_v4().to_string();
    db.collection::<Document>("service_pools")
        .insert_one(doc! {
            "_id": &pool_id,
            "user_id": &owner_id,
            "slug": "review-pool",
            "name": "Review pool",
            "strategy": "priority",
            "config_revision": 0_i64,
            "is_active": true,
            "members": [
                { "user_service_id": &first_id, "enabled": true },
                { "user_service_id": &second_id, "enabled": true },
            ],
            "created_at": mongodb::bson::DateTime::now(),
            "updated_at": mongodb::bson::DateTime::now(),
        })
        .await
        .expect("pool fixture");
    let first = HealthScope {
        pool_id,
        user_service_id: first_id,
        owner_id,
        pool_config_revision: 0,
        credential_identity: Uuid::new_v4().to_string(),
        credential_epoch: 1,
        destination_fingerprint: "destination-a".into(),
        config_fingerprint: "configuration-a".into(),
        model: Some("review-model".into()),
    };
    let second = HealthScope {
        user_service_id: second_id,
        credential_identity: Uuid::new_v4().to_string(),
        ..first.clone()
    };
    (db, first, second)
}

async fn fail(db: &mongodb::Database, ticket: &ObservationTicket, policy: &CooldownPolicy) {
    health::record_failure(db, ticket, policy, "upstream_429", Some(429), None)
        .await
        .expect("persist failure");
}

async fn assert_cooling(db: &mongodb::Database, scope: &HealthScope, expected: bool) {
    let row = health::load_for_scope(db, scope)
        .await
        .expect("load health");
    assert_eq!(
        row.as_ref()
            .is_some_and(|row| health::is_cooling(row, Utc::now())),
        expected,
        "unexpected cooldown for member {}",
        scope.user_service_id,
    );
}

#[tokio::test]
async fn service_pool_health_legacy_member_enabled_default_is_respected() {
    let (db, scope, _) = fixture("pool_health_legacy_member").await;
    db.collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &scope.pool_id },
            doc! { "$unset": { "members.$[].enabled": "" } },
        )
        .await
        .unwrap();
    let ticket = health::issue_observation_ticket(&db, scope.clone())
        .await
        .expect("legacy members default to enabled");
    fail(&db, &ticket, &CooldownPolicy::default()).await;
    assert_cooling(&db, &scope, true).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_health_threshold_and_success_use_real_database_state() {
    let (db, scope, _) = fixture("pool_health_threshold").await;
    let policy = CooldownPolicy {
        failures_to_open: 2,
        ..Default::default()
    };
    let first = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    fail(&db, &first, &policy).await;
    assert_cooling(&db, &scope, false).await;
    let second = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    fail(&db, &second, &policy).await;
    assert_cooling(&db, &scope, true).await;
    let row = health::load_for_scope(&db, &scope).await.unwrap().unwrap();
    assert_eq!(row.consecutive_failures, 2);
    assert!(row.expires_at.is_some_and(|expiry| expiry > Utc::now()));
    let recovered = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    health::record_success(&db, &recovered).await.unwrap();
    assert_cooling(&db, &scope, false).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_health_concurrent_initialization_and_failures_keep_every_observation() {
    let (db, scope, _) = fixture("pool_health_concurrent").await;
    let tickets = join_all((0..8).map(|_| health::issue_observation_ticket(&db, scope.clone())))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("concurrent ticket initialization");
    let policy = CooldownPolicy::default();
    join_all(tickets.iter().map(|ticket| fail(&db, ticket, &policy))).await;
    let row = health::load_for_scope(&db, &scope).await.unwrap().unwrap();
    assert_eq!(row.consecutive_failures, 8);
    assert_cooling(&db, &scope, true).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_health_duplicate_older_failure_is_counted_once() {
    let (db, scope, _) = fixture("pool_health_duplicate_older").await;
    let older = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    let newer = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    let policy = CooldownPolicy::default();
    fail(&db, &newer, &policy).await;
    fail(&db, &older, &policy).await;
    fail(&db, &older, &policy).await;
    let row = health::load_for_scope(&db, &scope).await.unwrap().unwrap();
    assert_eq!(row.consecutive_failures, 2);
    assert_eq!(row.observation_sequence, newer.observation_sequence);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_health_concurrent_failure_cannot_shorten_retry_after() {
    let (db, scope, _) = fixture("pool_health_retry_after").await;
    let first = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    let second = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    let policy = CooldownPolicy::default();
    health::record_failure(
        &db,
        &first,
        &policy,
        "upstream_429",
        Some(429),
        Some(std::time::Duration::from_secs(120)),
    )
    .await
    .unwrap();
    let retry_at = health::load_for_scope(&db, &scope)
        .await
        .unwrap()
        .unwrap()
        .cooldown_until
        .expect("Retry-After opens cooldown");
    fail(&db, &second, &policy).await;
    let actual = health::load_for_scope(&db, &scope)
        .await
        .unwrap()
        .unwrap()
        .cooldown_until
        .expect("member remains cooling");
    assert!(
        actual >= retry_at,
        "an in-flight failure must preserve Retry-After"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_health_member_reset_preserves_other_members() {
    let (db, first, second) = fixture("pool_health_member_reset").await;
    let policy = CooldownPolicy::default();
    for scope in [&first, &second] {
        let ticket = health::issue_observation_ticket(&db, scope.clone())
            .await
            .unwrap();
        fail(&db, &ticket, &policy).await;
    }
    health::reset_pool(
        &db,
        &first.pool_id,
        &first.owner_id,
        Some(&first.user_service_id),
    )
    .await
    .unwrap();
    assert_cooling(&db, &first, false).await;
    assert_cooling(&db, &second, true).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_health_pool_reset_fences_old_outcomes_and_accepts_new_ones() {
    let (db, first, second) = fixture("pool_health_pool_reset").await;
    let old_first = health::issue_observation_ticket(&db, first.clone())
        .await
        .unwrap();
    let old_second = health::issue_observation_ticket(&db, second.clone())
        .await
        .unwrap();
    let policy = CooldownPolicy::default();
    fail(&db, &old_first, &policy).await;
    fail(&db, &old_second, &policy).await;
    health::reset_pool(&db, &first.pool_id, &first.owner_id, None)
        .await
        .unwrap();
    for scope in [&first, &second] {
        assert_cooling(&db, scope, false).await;
    }
    fail(&db, &old_first, &policy).await;
    assert_cooling(&db, &first, false).await;
    let fresh = health::issue_observation_ticket(&db, first.clone())
        .await
        .unwrap();
    fail(&db, &fresh, &policy).await;
    health::record_success(&db, &old_first).await.unwrap();
    assert_cooling(&db, &first, true).await;
    assert_cooling(&db, &second, false).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_health_reset_before_first_observation_does_not_break_future_attempts() {
    let (db, scope, _) = fixture("pool_health_empty_reset").await;
    health::reset_pool(&db, &scope.pool_id, &scope.owner_id, None)
        .await
        .unwrap();
    let ticket = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    fail(&db, &ticket, &CooldownPolicy::default()).await;
    assert_cooling(&db, &scope, true).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_health_older_success_cannot_clear_newer_failure() {
    let (db, scope, _) = fixture("pool_health_old_success").await;
    let old = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    let newer = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    fail(&db, &newer, &CooldownPolicy::default()).await;
    health::record_success(&db, &old).await.unwrap();
    assert_cooling(&db, &scope, true).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_health_expired_row_cannot_be_changed_by_its_old_ticket() {
    let (db, scope, _) = fixture("pool_health_replacement").await;
    let old = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    // Delete the row exactly as Mongo's TTL monitor does, then initialize a
    // fresh attempt while the older provider response is still outstanding.
    db.collection::<Document>(HEALTH)
        .delete_many(doc! { "pool_id": &scope.pool_id })
        .await
        .unwrap();
    let fresh = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    fail(&db, &fresh, &CooldownPolicy::default()).await;
    health::record_success(&db, &old).await.unwrap();
    fail(&db, &old, &CooldownPolicy::default()).await;
    let row = health::load_for_scope(&db, &scope).await.unwrap().unwrap();
    assert_eq!(row.consecutive_failures, 1);
    assert_cooling(&db, &scope, true).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_health_configuration_and_credential_changes_do_not_inherit_cooldown() {
    let (db, original, _) = fixture("pool_health_scope_change").await;
    let old = health::issue_observation_ticket(&db, original.clone())
        .await
        .unwrap();
    fail(&db, &old, &CooldownPolicy::default()).await;
    let replaced_key = HealthScope {
        credential_epoch: 2,
        ..original.clone()
    };
    assert_cooling(&db, &replaced_key, false).await;
    let override_key = HealthScope {
        credential_identity: Uuid::new_v4().to_string(),
        ..original.clone()
    };
    assert_cooling(&db, &override_key, false).await;
    db.collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &original.pool_id },
            doc! { "$inc": { "config_revision": 1_i64 } },
        )
        .await
        .unwrap();
    let reconfigured = HealthScope {
        pool_config_revision: 1,
        ..original.clone()
    };
    assert_cooling(&db, &reconfigured, false).await;
    let fresh = health::issue_observation_ticket(&db, reconfigured.clone())
        .await
        .unwrap();
    fail(&db, &fresh, &CooldownPolicy::default()).await;
    fail(&db, &old, &CooldownPolicy::default()).await;
    let row = health::load_for_scope(&db, &reconfigured)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.consecutive_failures, 1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_health_reset_racing_failure_cannot_restore_cooldown() {
    let (db, scope, _) = fixture("pool_health_reset_race").await;
    let ticket = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    let policy = CooldownPolicy::default();
    let (reset, _) = tokio::join!(
        health::reset_pool(&db, &scope.pool_id, &scope.owner_id, None),
        fail(&db, &ticket, &policy),
    );
    reset.unwrap();
    assert_cooling(&db, &scope, false).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_runtime_observation_preserves_configuration_timestamp() {
    let (db, scope, _) = fixture("pool_config_timestamp").await;
    let pools = db.collection::<Document>("service_pools");
    let before = pools
        .find_one(doc! {"_id":&scope.pool_id})
        .await
        .unwrap()
        .unwrap();
    let ticket = health::issue_observation_ticket(&db, scope.clone())
        .await
        .unwrap();
    fail(&db, &ticket, &CooldownPolicy::default()).await;
    let after = pools
        .find_one(doc! {"_id":&scope.pool_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.get("updated_at"), after.get("updated_at"));
    assert_eq!(after.get_i64("health_observation_sequence").unwrap(), 1);
    db.drop().await.unwrap();
}
