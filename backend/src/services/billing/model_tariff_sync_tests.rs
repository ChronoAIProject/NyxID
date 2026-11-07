use super::super::model_tariffs;
use super::*;
use crate::models::platform_model_tariff::{COLLECTION_NAME, ModelTariff, TariffStatus};

#[tokio::test]
async fn model_tariff_partial_lago_failure_keeps_active_revision_and_retries_whole_pending_set() {
    let db = crate::test_utils::connect_transaction_test_database("model_tariff_sync").await;
    model_tariffs::ensure_indexes(&db).await.unwrap();
    let (mut service, active, active_rates) = model_tariffs::tests::fixture();
    let mut pending = active.clone();
    pending.id = Uuid::new_v4().to_string();
    pending.revision = 3;
    pending.status = TariffStatus::Pending;
    for c in &mut pending.components {
        c.credits_per_unit = "0.000002000002".into();
    }
    model_tariffs::validate(&mut pending, &service.slug).unwrap();
    service
        .billing
        .as_mut()
        .unwrap()
        .platform_key_pricing
        .as_mut()
        .unwrap()
        .model_prices[0]
        .pending_tariff_id = Some(pending.id.clone());
    db.collection::<DownstreamService>(CATALOG)
        .insert_one(&service)
        .await
        .unwrap();
    db.collection::<ModelTariff>(COLLECTION_NAME)
        .insert_many([&active, &pending])
        .await
        .unwrap();
    db.collection::<BillingRateCache>(RATES)
        .insert_many(active_rates.values())
        .await
        .unwrap();
    let lago = RecordingLago::default();
    lago.fail_sync_at.store(2, Ordering::SeqCst);
    assert!(
        !model_tariffs::sync(&db, &lago, "standard", &pending)
            .await
            .unwrap()
    );
    let reloaded = reload(&db, &service).await;
    let before = model_tariffs::load_for(&db, std::slice::from_ref(&reloaded))
        .await
        .unwrap();
    let cached =
        crate::services::billing::config::load_rates_for(&db, std::slice::from_ref(&reloaded))
            .await
            .unwrap();
    let selected = model_tariffs::select(
        &reloaded,
        &active.model_id,
        CredentialClass::NyxidManagedMaster,
        &active.variant,
        &before,
        &cached,
        chrono::Utc::now(),
        900,
    )
    .unwrap();
    assert_eq!(selected.tariff().id, active.id);
    assert_eq!(selected.tariff().components, active.components);
    assert_eq!(
        reloaded
            .billing
            .unwrap()
            .platform_key_pricing
            .unwrap()
            .model_prices[0]
            .active_tariff_id
            .as_deref(),
        Some(active.id.as_str())
    );
    lago.fail_sync_at.store(0, Ordering::SeqCst);
    assert_eq!(
        model_tariffs::retry_pending(&db, &lago, "standard")
            .await
            .unwrap(),
        1
    );
    let reloaded = reload(&db, &service).await;
    let binding = &reloaded
        .billing
        .as_ref()
        .unwrap()
        .platform_key_pricing
        .as_ref()
        .unwrap()
        .model_prices[0];
    assert_eq!(
        binding.active_tariff_id.as_deref(),
        Some(pending.id.as_str())
    );
    assert!(binding.pending_tariff_id.is_none());
    assert_eq!(
        model_tariffs::retry_pending(&db, &lago, "standard")
            .await
            .unwrap(),
        0
    );
    assert!(lago.removed.lock().unwrap().is_empty());
    let tariffs = model_tariffs::load_for(&db, std::slice::from_ref(&reloaded))
        .await
        .unwrap();
    let rates =
        crate::services::billing::config::load_rates_for(&db, std::slice::from_ref(&reloaded))
            .await
            .unwrap();
    assert!(
        model_tariffs::select(
            &reloaded,
            &active.model_id,
            CredentialClass::NyxidManagedMaster,
            &active.variant,
            &tariffs,
            &rates,
            chrono::Utc::now(),
            900
        )
        .is_ok()
    );
    assert!(
        lago.synced
            .lock()
            .unwrap()
            .iter()
            .all(|c| c.metric_code.contains("_t3_") && !c.metric_code.contains("legacy"))
    );
    db.drop().await.unwrap();
}
#[tokio::test]
async fn model_tariff_stale_sync_cannot_publish_over_replacement() {
    let db = crate::test_utils::connect_transaction_test_database("model_tariff_stale").await;
    let (mut service, mut stale, _) = model_tariffs::tests::fixture();
    stale.status = TariffStatus::Pending;
    service
        .billing
        .as_mut()
        .unwrap()
        .platform_key_pricing
        .as_mut()
        .unwrap()
        .model_prices[0]
        .pending_tariff_id = Some("newer".into());
    db.collection::<DownstreamService>(CATALOG)
        .insert_one(&service)
        .await
        .unwrap();
    db.collection::<ModelTariff>(COLLECTION_NAME)
        .insert_one(&stale)
        .await
        .unwrap();
    let lago = RecordingLago::default();
    assert!(
        !model_tariffs::sync(&db, &lago, "standard", &stale)
            .await
            .unwrap()
    );
    assert!(lago.synced.lock().unwrap().is_empty());
    assert_eq!(
        db.collection::<ModelTariff>(COLLECTION_NAME)
            .find_one(doc! {"_id":&stale.id})
            .await
            .unwrap()
            .unwrap()
            .status,
        TariffStatus::Retired
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn model_tariff_deleted_offering_does_not_starve_bounded_sync() {
    let db = crate::test_utils::connect_transaction_test_database("model_tariff_orphan").await;
    let (_, mut t, _) = model_tariffs::tests::fixture();
    t.status = TariffStatus::Pending;
    db.collection::<ModelTariff>(COLLECTION_NAME)
        .insert_one(&t)
        .await
        .unwrap();
    let lago = RecordingLago::default();
    assert_eq!(
        model_tariffs::retry_pending(&db, &lago, "standard")
            .await
            .unwrap(),
        0
    );
    assert!(lago.synced.lock().unwrap().is_empty());
    assert_eq!(
        db.collection::<ModelTariff>(COLLECTION_NAME)
            .find_one(doc! {"_id":&t.id})
            .await
            .unwrap()
            .unwrap()
            .status,
        TariffStatus::Retired
    );
    db.drop().await.unwrap();
}
