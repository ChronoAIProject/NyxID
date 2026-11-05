use super::*;
use crate::{
    models::sandbox_rate_card::{COLLECTION_NAME as CARDS, SandboxRateCard},
    services::{
        audit_service::AuditActor,
        billing::sandbox_rates::{self, PublishRateCard},
    },
};
use std::collections::BTreeMap;

fn approved() -> BTreeMap<String, String> {
    serde_json::from_str(include_str!("../../../../../assets/cma-sandbox-rates.json")).unwrap()
}
fn actor(f: &Fixture) -> AuditActor {
    AuditActor {
        user_id: f.owner.clone(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    }
}
fn future() -> chrono::DateTime<Utc> {
    chrono::DateTime::from_timestamp(Utc::now().timestamp() + 600, 0).unwrap()
}

#[tokio::test]
async fn deployment_seed_is_exact_idempotent_and_never_overwrites_runtime_edits() {
    let db = crate::test_utils::connect_transaction_test_database("cma_rate_seed").await;
    let key = [42; 32];
    sandbox_rates::seed(&db, "test-plan", &key).await.unwrap();
    sandbox_rates::seed(&db, "test-plan", &key).await.unwrap();
    let first = sandbox_rates::effective(&db, Utc::now()).await.unwrap();
    assert_eq!(first.version, 1);
    assert_eq!(first.sync_status, PricingSyncStatus::Pending);
    assert_eq!(
        first
            .prices
            .iter()
            .map(|p| p.credits_per_unit.pico())
            .collect::<Vec<_>>(),
        [10_000, 1_000, 40_000]
    );
    let actor = AuditActor {
        user_id: Uuid::new_v4().to_string(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    let mut changed = approved();
    changed.insert(METRICS[0].as_str().into(), "0.000000020000".into());
    let effective_from = future();
    let second = sandbox_rates::publish(
        &db,
        "test-plan",
        PublishRateCard {
            expected_version: 1,
            effective_from,
            rates: changed.clone(),
        },
        &actor,
        &key,
    )
    .await
    .unwrap();
    sandbox_rates::seed(&db, "different-deploy-plan", &key)
        .await
        .unwrap();
    assert_eq!(sandbox_rates::list(&db).await.unwrap().len(), 2);
    assert_eq!(
        sandbox_rates::effective(&db, effective_from - Duration::seconds(1))
            .await
            .unwrap()
            .id,
        first.id
    );
    assert_eq!(
        sandbox_rates::effective(&db, effective_from)
            .await
            .unwrap()
            .id,
        second.id
    );
    assert!(
        sandbox_rates::publish(
            &db,
            "test-plan",
            PublishRateCard {
                expected_version: 1,
                effective_from: effective_from + Duration::seconds(1),
                rates: changed
            },
            &actor,
            &key
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(
                doc! {"event_type":{"$in":["cma_rate_card_seeded","cma_rate_card_published"]}}
            )
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn malformed_rates_and_backdated_versions_have_no_persisted_effect() {
    let f = fixture("cma_invalid_rates").await;
    let initial = sandbox_rates::list(&f.db).await.unwrap();
    let mut inputs = Vec::new();
    for invalid in [
        "0",
        "-1",
        "NaN",
        "1e-8",
        "0.0000000000001",
        "1000000.000000000001",
    ] {
        let mut rates = approved();
        rates.insert(METRICS[0].as_str().into(), invalid.into());
        inputs.push(rates);
    }
    let mut missing = approved();
    missing.remove(METRICS[2].as_str());
    inputs.push(missing);
    let mut extra = approved();
    extra.insert("requests".into(), "1".into());
    inputs.push(extra);
    for rates in inputs {
        assert!(matches!(
            sandbox_rates::publish(
                &f.db,
                &f.billing.config.lago_plan_code,
                PublishRateCard {
                    expected_version: 1,
                    effective_from: future(),
                    rates
                },
                &actor(&f),
                &[42; 32]
            )
            .await,
            Err(AppError::ValidationError(_))
        ));
    }
    assert!(
        sandbox_rates::publish(
            &f.db,
            &f.billing.config.lago_plan_code,
            PublishRateCard {
                expected_version: 1,
                effective_from: Utc::now(),
                rates: approved()
            },
            &actor(&f),
            &[42; 32]
        )
        .await
        .is_err()
    );
    assert_eq!(
        sandbox_rates::list(&f.db).await.unwrap()[0].id,
        initial[0].id
    );
}

#[tokio::test]
async fn rate_change_caps_coverage_and_preserves_delayed_usage_at_accepted_prices() {
    let f = fixture("cma_rate_cutoff").await;
    let input = request(&f, 300).await;
    let original = sandbox_rates::effective(&f.db, Utc::now()).await.unwrap();
    let published = sandbox_rates::publish(
        &f.db,
        &f.billing.config.lago_plan_code,
        PublishRateCard {
            expected_version: 1,
            effective_from: future(),
            rates: approved(),
        },
        &actor(&f),
        &[42; 32],
    )
    .await
    .unwrap();
    f.db.collection::<Document>(CARDS).update_one(doc! {"version":1},
        doc! {"$set":{"sync_claim":"another-worker","sync_claim_until":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(1))}}).await.unwrap();
    assert_eq!(f.billing.retry_sandbox_rate_cards().await.unwrap(), 0);
    f.db.collection::<Document>(CARDS).update_one(doc! {"version":1},
        doc! {"$set":{"sync_claim_until":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    f.provider.available.store(false, Ordering::SeqCst);
    assert_eq!(f.billing.retry_sandbox_rate_cards().await.unwrap(), 0);
    f.provider.available.store(true, Ordering::SeqCst);
    assert_eq!(f.billing.retry_sandbox_rate_cards().await.unwrap(), 1);
    assert_eq!(
        sandbox_rates::list(&f.db).await.unwrap()[0].sync_status,
        PricingSyncStatus::Synced
    );
    // Move only the synthetic future policy clock close enough to test a cutoff
    // without waiting. Production publication enforces the full notice period.
    let cutoff = Utc::now().timestamp() + 120;
    f.db.collection::<Document>(CARDS)
        .update_one(
            doc! {"_id":&published.id},
            doc! {"$set":{"effective_from":bson::DateTime::from_millis(cutoff*1000)}},
        )
        .await
        .unwrap();
    let window = f
        .billing
        .reserve_sandbox_window(&f.producer, input.clone())
        .await
        .unwrap();
    assert_eq!(window.covered_until, cutoff);
    // Exercise the exact boundary independently of the wall-clock renewal
    // lead time: the old card cannot fund even one second at the new price.
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(matches!(
        sandbox_rates::coverage_limit(
            &f.db,
            &mut session,
            &original.id,
            cutoff,
            cutoff + 10
        )
        .await,
        Err(AppError::Conflict(message)) if message.starts_with("CMA rate changed;")
    ));
    session.abort_transaction().await.unwrap();
    f.db.collection::<Document>(CARDS).update_one(doc!{"_id":&published.id},doc!{"$set":{"effective_from":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    let mut old = input.clone();
    old.request_id = Uuid::new_v4().to_string();
    old.previous_window_id = Some(window.id.clone());
    assert!(matches!(
        f.billing.reserve_sandbox_window(&f.producer, old).await,
        Err(AppError::Conflict(_))
    ));
    let fresh = f
        .billing
        .quote_sandbox(
            &f.producer,
            &f.owner,
            &format!("sbx_{}", Uuid::new_v4().simple()),
            allocation(),
        )
        .await
        .unwrap();
    assert_eq!(fresh.rate_card_id, published.id);
    assert_eq!(fresh.prices, published.prices);
    // A provider drift observed during admission is durable repair work, even
    // after the version previously synchronized successfully.
    f.provider.available.store(false, Ordering::SeqCst);
    assert!(
        f.billing
            .quote_sandbox(&f.producer, &f.owner, &fresh.sandbox_id, allocation())
            .await
            .is_err()
    );
    assert_eq!(
        sandbox_rates::list(&f.db).await.unwrap()[0].sync_status,
        PricingSyncStatus::Failed
    );
    f.provider.available.store(true, Ordering::SeqCst);
    assert_eq!(f.billing.retry_sandbox_rate_cards().await.unwrap(), 1);
    let elapsed = elapsed(&f, window).await;
    f.billing
        .finalize_sandbox_window(&f.producer, evidence(&elapsed))
        .await
        .unwrap();
    for component in &elapsed.components {
        let row =
            f.db.collection::<UsageMeterRow>(USAGE)
                .find_one(doc! {"_id":&component.usage_row_id})
                .await
                .unwrap()
                .unwrap();
        let price = original
            .prices
            .iter()
            .find(|p| p.metric == component.metric)
            .unwrap();
        assert_eq!(
            row.funding.as_ref().unwrap().settlement_rate,
            Some(price.credits_per_unit)
        );
        assert_eq!(row.lago_metric_code, price.lago_metric_code);
        assert_eq!(
            super::super::evidence::verify_usage_provider_rate(&f.db, f.provider.as_ref(), &row)
                .await
                .unwrap()
                .as_deref(),
            Some("test-subscription")
        );
    }
    let saved =
        f.db.collection::<SandboxRateCard>(CARDS)
            .find_one(doc! {"_id":&original.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(saved.prices, original.prices);
}

#[tokio::test]
async fn rate_publication_rolls_back_when_its_audit_cannot_commit() {
    let f = fixture("cma_rate_audit").await;
    // Ensure the shared audit collection exists before applying a validator.
    crate::services::audit_service::log_system_event(f.db.clone(), "cma_rate_test_ready", None)
        .await
        .unwrap();
    f.db.run_command(doc! {
        "collMod": crate::models::audit_log::COLLECTION_NAME,
        "validator": {"event_type": {"$ne": "cma_rate_card_published"}},
        "validationAction": "error",
    })
    .await
    .unwrap();
    let result = sandbox_rates::publish(
        &f.db,
        &f.billing.config.lago_plan_code,
        PublishRateCard {
            expected_version: 1,
            effective_from: future(),
            rates: approved(),
        },
        &actor(&f),
        &[42; 32],
    )
    .await;
    assert!(result.is_err());
    assert_eq!(sandbox_rates::list(&f.db).await.unwrap().len(), 1);
    assert_eq!(
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"cma_rate_card_published"})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn concurrent_rate_editors_publish_only_one_next_version() {
    let f = fixture("cma_rate_editors").await;
    let at = future();
    let actor = actor(&f);
    let one = PublishRateCard {
        expected_version: 1,
        effective_from: at,
        rates: approved(),
    };
    let mut changed = approved();
    changed.insert(METRICS[0].as_str().into(), "0.00000003".into());
    let two = PublishRateCard {
        expected_version: 1,
        effective_from: at,
        rates: changed,
    };
    let (a, b) = tokio::join!(
        sandbox_rates::publish(
            &f.db,
            &f.billing.config.lago_plan_code,
            one,
            &actor,
            &[42; 32]
        ),
        sandbox_rates::publish(
            &f.db,
            &f.billing.config.lago_plan_code,
            two,
            &actor,
            &[42; 32]
        )
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(sandbox_rates::list(&f.db).await.unwrap().len(), 2);
    assert_eq!(
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"cma_rate_card_published"})
            .await
            .unwrap(),
        1
    );
}
