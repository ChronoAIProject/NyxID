use super::*;
use crate::test_utils::connect_transaction_test_database;
use mongodb::{Database, options::ReturnDocument};

async fn seed(db: &Database, id: &str, model: bool) {
    db.collection::<Document>(execution::COLLECTION)
        .insert_one(
            doc! {"_id":id,"kind":KIND,"user_id":"owner","agent_id":"agent","profile":"profile",
            "claim":"generation","status":"setting_up","model_dispatched":model,
            "until":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1))},
        )
        .await
        .unwrap();
}

async fn settle(db: &Database, id: &str, success: bool) -> Document {
    db.collection::<Document>(execution::COLLECTION)
        .find_one_and_update(
            doc! {"_id":id,"status":"setting_up"},
            terminal(
                doc! {"status":if success {"ready"} else {"failed"},"reason":"$literal-reason"},
                doc! {},
                success,
            ),
        )
        .return_document(ReturnDocument::After)
        .await
        .unwrap()
        .unwrap()
}

fn delay(row: &Document, seconds: i64) {
    let remaining = row.get_datetime("until").unwrap().to_chrono() - Utc::now();
    assert!(
        (remaining.num_milliseconds() - seconds * 1000).abs() < 2000,
        "{remaining:?}, expected {seconds}s"
    );
}

#[tokio::test]
async fn guest_setup_backoff_grows_durably_and_caps_without_double_settlement() {
    let db = connect_transaction_test_database("guest_setup_backoff_growth").await;
    seed(&db, "retry", true).await;
    for (index, seconds) in [60, 120, 240, 480, 960, 1920, 3600, 3600]
        .into_iter()
        .enumerate()
    {
        let rows = db.collection::<Document>(execution::COLLECTION);
        rows.update_one(doc! {"_id":"retry"}, doc! {"$set":{"status":"setting_up"}})
            .await
            .unwrap();
        let row = settle(&db, "retry", false).await;
        delay(&row, seconds);
        assert_eq!(
            row.get_i64("model_failures").unwrap(),
            (index as i64 + 1).min(7)
        );
        assert_eq!(row.get_str("reason"), Ok("$literal-reason"));
        assert_eq!(
            rows.update_one(
                doc! {"_id":"retry","status":"setting_up"},
                terminal(doc! {"status":"failed"}, doc! {}, false)
            )
            .await
            .unwrap()
            .matched_count,
            0
        );
        // Each iteration re-reads Mongo state; no in-memory retry counter.
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_setup_backoff_success_resets_and_non_model_failures_keep_short_retry() {
    let db = connect_transaction_test_database("guest_setup_backoff_unpaid").await;
    seed(&db, "retry", true).await;
    let rows = db.collection::<Document>(execution::COLLECTION);
    for reason in ["credits_unavailable", "setup_capacity", "setup_readiness"] {
        rows.update_one(
            doc! {"_id":"retry"},
            doc! {"$set":{"status":"setting_up","model_failures":6_i64,"reason":reason},
            "$unset":{"model_dispatched":""}},
        )
        .await
        .unwrap();
        let row = settle(&db, "retry", false).await;
        delay(&row, 60);
        assert_eq!(
            row.get_i64("model_failures"),
            Ok(6),
            "unpaid failures do not erase paid failures"
        );
    }
    rows.update_one(
        doc! {"_id":"retry"},
        doc! {"$set":{"status":"setting_up","model_dispatched":true}},
    )
    .await
    .unwrap();
    assert_eq!(
        settle(&db, "retry", true).await.get_i64("model_failures"),
        Ok(0)
    );
    rows.update_one(doc! {"_id":"retry"}, doc! {"$set":{"status":"setting_up"}})
        .await
        .unwrap();
    delay(&settle(&db, "retry", false).await, 60);
    seed(&db, "legacy", false).await;
    rows.update_one(
        doc! {"_id":"legacy"},
        doc! {"$unset":{"model_dispatched":""}},
    )
    .await
    .unwrap();
    let row = settle(&db, "legacy", false).await;
    delay(&row, 60);
    assert_eq!(row.get_i64("model_failures"), Ok(0));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_setup_backoff_owner_reset_is_scoped_and_preserves_active_lease() {
    let db = connect_transaction_test_database("guest_setup_backoff_reset").await;
    let rows = db.collection::<Document>(execution::COLLECTION);
    for id in [
        "active",
        "failed",
        "other-profile",
        "other-owner",
        "other-agent",
    ] {
        seed(&db, id, true).await;
        rows.update_one(doc! {"_id":id}, doc! {"$set":{"model_failures":7_i64}})
            .await
            .unwrap();
    }
    for (id, field, value) in [
        ("other-profile", "profile", "other"),
        ("other-owner", "user_id", "other"),
        ("other-agent", "agent_id", "other"),
    ] {
        rows.update_one(doc! {"_id":id}, doc! {"$set":{field:value}})
            .await
            .unwrap();
    }
    rows.update_one(doc! {"_id":"failed"}, doc! {"$set":{"status":"failed"}})
        .await
        .unwrap();
    let before = rows.find_one(doc! {"_id":"active"}).await.unwrap().unwrap();
    reset(&db, "owner", "agent", Some("profile")).await.unwrap();
    let active = rows.find_one(doc! {"_id":"active"}).await.unwrap().unwrap();
    assert_eq!(active.get_datetime("until"), before.get_datetime("until"));
    assert_eq!(active.get_str("status"), Ok("setting_up"));
    assert_eq!(active.get_i64("model_failures"), Ok(0));
    let failed = rows.find_one(doc! {"_id":"failed"}).await.unwrap().unwrap();
    delay(&failed, 0);
    assert_eq!(failed.get_i64("model_failures"), Ok(0));
    // An old running attempt cannot put the previous long cooldown back.
    let settled = settle(&db, "active", false).await;
    assert_eq!(settled.get_i64("model_failures"), Ok(0));
    delay(&settled, 0);
    for id in ["other-profile", "other-owner", "other-agent"] {
        assert_eq!(
            rows.find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap()
                .get_i64("model_failures"),
            Ok(7)
        );
    }
    reset(&db, "owner", "agent", None).await.unwrap();
    assert_eq!(
        rows.find_one(doc! {"_id":"other-profile"})
            .await
            .unwrap()
            .unwrap()
            .get_i64("model_failures"),
        Ok(0)
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_setup_backoff_configuration_changes_reset_but_token_refresh_does_not() {
    let f = Box::pin(crate::services::assistant_authority_tests::direct_fixture(
        "guest_setup_backoff_config",
    ))
    .await;
    let db = &f.state.db;
    let mut agent = team::agent(db, &f.owner, &f.chat.agent_id).await.unwrap();
    db.collection::<Document>("user_services")
        .insert_one(doc! {"_id":"connection","api_key_id":"key","endpoint_id":"endpoint"})
        .await
        .unwrap();
    db.collection::<Document>("user_api_keys")
        .insert_one(doc! {"_id":"key","credential_epoch":1_i64,"status":"active"})
        .await
        .unwrap();
    db.collection::<Document>("user_endpoints")
        .insert_one(doc! {"_id":"endpoint","url":"https://model.example/v1"})
        .await
        .unwrap();
    let target = doc! {"catalog":"catalog","connection":"connection"};
    let original = fingerprint(db, &agent, Some(&target)).await.unwrap();
    for field in [
        "updated_at",
        "last_used_at",
        "expires_at",
        "access_token_encrypted",
        "refresh_token_encrypted",
        "status",
    ] {
        db.collection::<Document>("user_api_keys")
            .update_one(
                doc! {"_id":"key"},
                doc! {"$set":{field:bson::DateTime::now()}},
            )
            .await
            .unwrap();
    }
    assert_eq!(
        original,
        fingerprint(db, &agent, Some(&target)).await.unwrap()
    );
    for (collection, field, value) in [
        ("user_api_keys", "credential_epoch", bson::Bson::Int64(2)),
        (
            "user_endpoints",
            "url",
            bson::Bson::String("https://changed.example/v1".into()),
        ),
        (
            "user_services",
            "auth_key_name",
            bson::Bson::String("x-api-key".into()),
        ),
    ] {
        let old = fingerprint(db, &agent, Some(&target)).await.unwrap();
        let rows = db.collection::<Document>(execution::COLLECTION);
        rows.replace_one(
            doc! {"_id":"retry"},
            doc! {"_id":"retry","claim":"generation","status":"failed",
            "model_failures":7_i64,"retry_target":&target,"retry_config":&old,
            "until":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1))},
        )
        .upsert(true)
        .await
        .unwrap();
        let record = rows.find_one(doc! {"_id":"retry"}).await.unwrap().unwrap();
        db.collection::<Document>(collection)
            .update_one(doc! {}, doc! {"$set":{field:value}})
            .await
            .unwrap();
        refresh(db, &agent, &record).await.unwrap();
        let after = rows.find_one(doc! {"_id":"retry"}).await.unwrap().unwrap();
        assert_eq!(after.get_i64("model_failures"), Ok(0));
        delay(&after, 0);
        assert_ne!(after.get_str("retry_config"), Ok(old.as_str()));
        // A second guest holding the stale snapshot cannot reset a new claim.
        rows.update_one(
            doc! {"_id":"retry"},
            doc! {"$set":{"claim":"new","status":"setting_up","model_failures":3_i64}},
        )
        .await
        .unwrap();
        refresh(db, &agent, &record).await.unwrap();
        assert_eq!(
            rows.find_one(doc! {"_id":"retry"})
                .await
                .unwrap()
                .unwrap()
                .get_i64("model_failures"),
            Ok(3)
        );
    }
    let before = fingerprint(db, &agent, Some(&target)).await.unwrap();
    agent.model = "nyxagent/channel".into();
    assert_ne!(
        before,
        fingerprint(db, &agent, Some(&target)).await.unwrap()
    );
    db.collection::<Document>("user_services")
        .update_one(
            doc! {"_id":"connection"},
            doc! {"$set":{"credential_binding":"platform"}},
        )
        .await
        .unwrap();
    let before = fingerprint(db, &agent, Some(&target)).await.unwrap();
    db.collection::<Document>("user_api_keys")
        .update_one(doc! {"_id":"key"}, doc! {"$inc":{"credential_epoch":1_i64}})
        .await
        .unwrap();
    assert_eq!(
        before,
        fingerprint(db, &agent, Some(&target)).await.unwrap(),
        "unused retained BYOK keys cannot defeat platform backoff"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_setup_backoff_crash_and_guest_force_cannot_bypass_cooldown() {
    let f = Box::pin(crate::services::assistant_authority_tests::direct_fixture(
        "guest_setup_backoff_crash",
    ))
    .await;
    let agent = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    let (_, row) = claim(&f.state, &f.owner, &agent, &agent.model, false, WAIT)
        .await
        .unwrap()
        .unwrap();
    let rows = f.state.db.collection::<Document>(execution::COLLECTION);
    rows.update_one(
        doc! {"conversation_id":&row.id},
        doc! {"$set":{"model_dispatched":true,"model_failures":2_i64,
        "until":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}},
    )
    .await
    .unwrap();
    assert!(
        !kick_bounded(&f.state, &f.owner, &agent.id, &agent.model, true, WAIT)
            .await
            .unwrap()
    );
    let failed = record(&f.state.db, &f.owner, &agent.id, &agent.model)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.get_str("status"), Ok("failed"));
    assert_eq!(failed.get_i64("model_failures"), Ok(3));
    delay(&failed, 240);
    sweep(&f.state).await.unwrap();
    assert!(
        claim(&f.state, &f.owner, &agent, &agent.model, true, WAIT)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        record(&f.state.db, &f.owner, &agent.id, &agent.model)
            .await
            .unwrap()
            .unwrap()
            .get_i64("model_failures"),
        Ok(3)
    );
    // A new worker after the cooldown inherits the count, but not dispatch.
    rows.update_one(doc! {"_id":failed.get_str("_id").unwrap()},
        doc! {"$set":{"until":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
    let (_, retry) = claim(&f.state, &f.owner, &agent, &agent.model, true, WAIT)
        .await
        .unwrap()
        .unwrap();
    let retry_record = record(&f.state.db, &f.owner, &agent.id, &agent.model)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retry_record.get_i64("model_failures"), Ok(3));
    assert!(retry_record.get_bool("model_dispatched").is_err());
    rows.update_one(
        doc! {"conversation_id":&retry.id},
        doc! {"$set":{"model_dispatched":true}},
    )
    .await
    .unwrap();
    let failed = settle(&f.state.db, retry_record.get_str("_id").unwrap(), false).await;
    assert_eq!(failed.get_i64("model_failures"), Ok(4));
    delay(&failed, 480);
    // Explicit settings save resets the failed generation, including all profiles.
    team::update_agent(
        &f.state.db,
        &f.owner,
        &agent.id,
        None,
        None,
        team::AgentStyle::default(),
    )
    .await
    .unwrap();
    let reset = record(&f.state.db, &f.owner, &agent.id, &agent.model)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reset.get_i64("model_failures"), Ok(0));
    delay(&reset, 0);
    let (_, next) = claim(&f.state, &f.owner, &agent, &agent.model, true, WAIT)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(next.id, row.id);
    let next = record(&f.state.db, &f.owner, &agent.id, &agent.model)
        .await
        .unwrap()
        .unwrap();
    assert!(next.get_bool("model_dispatched").is_err());
    assert!(next.get_bool("retry_reset").is_err());
    f.state.db.drop().await.unwrap();
}
