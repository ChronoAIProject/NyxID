use super::{policy, price_pins, pricing_schema, pricing_tests as fixtures, tariff_store::*};
use crate::{
    models::sandbox_usage_rates::*,
    services::{
        audit_service::AuditActor,
        feature_flag_service::{self as flags, FlagTarget},
    },
};
use chrono::{Duration, Utc};
use mongodb::{
    Database,
    bson::{Document, doc},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

fn actor() -> AuditActor {
    AuditActor {
        user_id: uuid::Uuid::new_v4().to_string(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    }
}
fn input(version: i64) -> PublishTariff {
    PublishTariff {
        producer_namespace: fixtures::scope().producer_namespace,
        expected_version: version,
        effective_from: Utc::now() + Duration::minutes(10 + version),
        rates: BTreeMap::from([(
            "sandbox_cpu_core_nanoseconds_v1".into(),
            "0.000000000001".into(),
        )]),
    }
}
pub(super) async fn enable(db: &Database, enabled: bool) {
    flags::set_platform_override(
        db,
        flags::CMA_SANDBOX_USAGE_CHARGE_FLAG_KEY,
        &FlagTarget::Global,
        enabled,
        &uuid::Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
}
async fn fixture(name: &str) -> Database {
    let db = crate::test_utils::connect_transaction_test_database(name).await;
    enable(&db, true).await;
    pricing_schema::prepare(&db).await.unwrap();
    db
}
async fn count(db: &Database, collection: &str) -> u64 {
    db.collection::<Document>(collection)
        .count_documents(doc! {})
        .await
        .unwrap()
}
fn historical_policy(tariff: &UsageTariff) -> UsagePolicy {
    let pin = fixtures::pin(tariff);
    UsagePolicy {
        id: pin.policy_id.unwrap(),
        schema_version: 1,
        scope_key: pin.scope_key,
        scope: pin.scope,
        version: 1,
        effective_from: fixtures::time(),
        dimensions: pin.dimensions,
        legacy_coverage: pin.legacy_coverage,
        created_by: "synthetic-admin".into(),
        created_at: tariff.created_at,
    }
}

#[tokio::test]
async fn absent_and_disabled_flag_touch_no_new_namespaces_even_on_startup() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(Mutex::new(Vec::new()));
    let capture = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "n3_off",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                capture.lock().unwrap().push(event.command.to_string());
            }
        }),
    )
    .await;
    crate::db::ensure_indexes(&db).await.unwrap();
    crate::services::personal_erasure_service::ensure_startup_indexes(&db)
        .await
        .unwrap();
    for disabled in [false, true] {
        if disabled {
            enable(&db, false).await;
        }
        assert!(pricing_schema::prepare(&db).await.is_err());
        assert!(
            publish_tariff(&db, input(0), &actor(), &[42; 32])
                .await
                .is_err()
        );
        assert!(
            price_pins::pin(&db, &fixtures::scope(), fixtures::time())
                .await
                .is_err()
        );
        assert!(list_tariffs(&db, "cma-example").await.is_err());
        assert!(list_policies(&db, &fixtures::scope()).await.is_err());
    }
    let observed = commands.lock().unwrap().clone();
    for collection in [COLLECTION_NAME, POLICIES, PINS, SCHEMA] {
        assert!(
            !observed.iter().any(|c| c.contains(collection)),
            "disabled namespace IO: {collection}"
        );
        assert!(
            !db.list_collection_names()
                .await
                .unwrap()
                .contains(&collection.to_owned())
        );
    }
}

#[tokio::test]
async fn explicit_preparation_is_required_and_idempotent_with_owner_indexes() {
    let db = crate::test_utils::connect_transaction_test_database("n3_prepare").await;
    enable(&db, true).await;
    assert!(
        publish_tariff(&db, input(0), &actor(), &[42; 32])
            .await
            .is_err()
    );
    pricing_schema::prepare(&db).await.unwrap();
    pricing_schema::prepare(&db).await.unwrap();
    assert_eq!(count(&db, SCHEMA).await, 1);
    use futures::TryStreamExt;
    for collection in [POLICIES, PINS] {
        let indexes: Vec<_> = db
            .collection::<Document>(collection)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(
            indexes
                .iter()
                .any(|i| i.keys.get_i32("scope.owner_id") == Ok(1))
        );
    }
    publish_tariff(&db, input(0), &actor(), &[42; 32])
        .await
        .unwrap();
}

#[tokio::test]
async fn publication_is_future_append_only_and_concurrent_editors_share_one_revision() {
    let db = fixture("n3_publish").await;
    let actor = actor();
    let (one, two) = tokio::join!(
        publish_tariff(&db, input(0), &actor, &[42; 32]),
        publish_tariff(&db, input(0), &actor, &[42; 32])
    );
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    let card = one.or(two).unwrap();
    assert_eq!(card.effective_from.timestamp() % 60, 0);
    assert!(card.effective_from >= card.created_at + Duration::seconds(301));
    assert_eq!(card.rates[0].credits_per_unit.pico(), 1);
    assert!(card.rates[0].provider_proof.is_none());
    for invalid in [
        "-1",
        "NaN",
        "1e-8",
        "0.0000000000001",
        "1000000.000000000001",
    ] {
        let mut request = input(1);
        request.rates.values_mut().for_each(|v| *v = invalid.into());
        assert!(
            publish_tariff(&db, request, &actor, &[42; 32])
                .await
                .is_err()
        );
    }
    let mut retro = input(1);
    retro.effective_from = Utc::now();
    assert!(publish_tariff(&db, retro, &actor, &[42; 32]).await.is_err());
    assert!(
        publish_tariff(&db, input(0), &actor, &[42; 32])
            .await
            .is_err()
    );
    assert_eq!(count(&db, COLLECTION_NAME).await, 1);
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"sandbox_usage_tariff_published"})
            .await
            .unwrap(),
        1
    );
    let next = publish_tariff(&db, input(1), &actor, &[42; 32])
        .await
        .unwrap();
    assert_eq!(next.version, 2);
    assert_eq!(list_tariffs(&db, "cma-example").await.unwrap()[1], card);
}

#[tokio::test]
async fn failed_audit_rolls_back_publication() {
    let db = fixture("n3_audit").await;
    crate::services::audit_service::log_system_event(db.clone(), "n3_test_ready", None)
        .await
        .unwrap();
    db.run_command(doc!{"collMod":crate::models::audit_log::COLLECTION_NAME,"validator":{"event_type":{"$ne":"sandbox_usage_tariff_published"}},"validationAction":"error"}).await.unwrap();
    assert!(
        publish_tariff(&db, input(0), &actor(), &[42; 32])
            .await
            .is_err()
    );
    assert_eq!(count(&db, COLLECTION_NAME).await, 0);
}

#[tokio::test]
async fn shadow_pins_survive_reenable_policy_publication_and_owner_balance_changes() {
    let db = fixture("n3_shadow_pin").await;
    let scope = fixtures::scope();
    let at = fixtures::time();
    let (a, b) = tokio::join!(
        price_pins::pin(&db, &scope, at),
        price_pins::pin(&db, &scope, at)
    );
    let original = a.unwrap();
    assert_eq!(original, b.unwrap());
    assert_eq!(count(&db, PINS).await, 1);
    assert_eq!(original.dimensions, policy::shadow());
    assert_eq!(original.charge_mode, ChargeMode::Shadow);
    let tariff = publish_tariff(&db, input(0), &actor(), &[42; 32])
        .await
        .unwrap();
    publish_policy(
        &db,
        PublishPolicy {
            scope: scope.clone(),
            expected_version: 0,
            effective_from: tariff.effective_from,
            dimensions: fixtures::pin(&tariff).dimensions,
            legacy_coverage: None,
        },
        &actor(),
        &[42; 32],
    )
    .await
    .unwrap();
    enable(&db, false).await;
    assert!(
        price_pins::pin(&db, &scope, at + Duration::minutes(1))
            .await
            .is_err()
    );
    assert!(
        publish_tariff(&db, input(1), &actor(), &[42; 32])
            .await
            .is_err()
    );
    assert_eq!(
        price_pins::recover(&db, &scope, at).await.unwrap(),
        Some(original.clone())
    );
    // A synthetic funding change cannot rewrite the historical decision.
    db.collection::<Document>("billing_wallet")
        .insert_one(
            doc! {"_id":uuid::Uuid::new_v4().to_string(),"owner_id":&scope.owner_id,"balance":100},
        )
        .await
        .unwrap();
    enable(&db, true).await;
    assert_eq!(price_pins::pin(&db, &scope, at).await.unwrap(), original);
    for owner in ["other-owner", "another-owner"] {
        let mut other = scope.clone();
        other.owner_id = owner.into();
        let separate = price_pins::pin(&db, &other, at).await.unwrap();
        assert_ne!(separate.id, original.id);
    }
    let mut epoch = scope;
    epoch.source_epoch = uuid::Uuid::new_v4().to_string();
    assert_ne!(
        price_pins::pin(&db, &epoch, at).await.unwrap().id,
        original.id
    );
}

#[tokio::test]
async fn late_evidence_uses_usage_time_policy_and_tariff_without_money_effects() {
    let db = fixture("n3_boundaries").await;
    // Synthetic historical rows simulate elapsed publication notice. The API rejects backdating.
    let first = fixtures::tariff();
    let first_policy = historical_policy(&first);
    let boundary = fixtures::time() + Duration::minutes(1);
    let mut second = first.clone();
    second.id = uuid::Uuid::new_v4().to_string();
    second.version = 2;
    second.effective_from = boundary;
    second.rates[0].credits_per_unit = crate::models::credits::Credits::from_whole(2);
    let mut second_policy = first_policy.clone();
    second_policy.id = uuid::Uuid::new_v4().to_string();
    second_policy.version = 2;
    second_policy.effective_from = boundary;
    second_policy.dimensions.cpu = Selection::Free;
    db.collection::<UsageTariff>(COLLECTION_NAME)
        .insert_many([&first, &second])
        .await
        .unwrap();
    db.collection::<UsagePolicy>(POLICIES)
        .insert_many([&first_policy, &second_policy])
        .await
        .unwrap();
    let before = price_pins::pin(&db, &fixtures::scope(), fixtures::time())
        .await
        .unwrap();
    let at = price_pins::pin(&db, &fixtures::scope(), boundary)
        .await
        .unwrap();
    assert_eq!(before.tariff_id, Some(first.id));
    assert_eq!(before.policy_id, Some(first_policy.id));
    assert_eq!(at.tariff_id, Some(second.id));
    assert_eq!(at.dimensions.cpu, Selection::Free);
    assert_eq!(before.charge_mode, ChargeMode::Shadow);
    assert_eq!(at.charge_mode, ChargeMode::Shadow);
    assert_eq!(
        price_pins::pin(&db, &fixtures::scope(), fixtures::time())
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        price_pins::pin(&db, &fixtures::scope(), boundary)
            .await
            .unwrap(),
        at
    );
    for namespace in [
        "billing_wallet",
        "billing_ledger",
        "usage_meter",
        "sandbox_rate_cards",
        "sandbox_billing_quotes",
        "sandbox_billing_windows",
    ] {
        assert_eq!(
            count(&db, namespace).await,
            0,
            "unexpected old billing effect in {namespace}"
        );
    }
}

#[tokio::test]
async fn policies_cannot_rewrite_history_or_shrink_legacy_coverage() {
    let db = fixture("n3_policy").await;
    let actor = actor();
    let make = || PublishPolicy {
        scope: fixtures::scope(),
        expected_version: 0,
        effective_from: Utc::now() + Duration::minutes(10),
        dimensions: policy::shadow(),
        legacy_coverage: Some(LegacyCoverage {
            covered_until: fixtures::time(),
            manifest_digest: "a".repeat(64),
        }),
    };
    let first = publish_policy(&db, make(), &actor, &[42; 32])
        .await
        .unwrap();
    assert!(
        publish_policy(&db, make(), &actor, &[42; 32])
            .await
            .is_err()
    );
    let mut retro = make();
    retro.expected_version = 1;
    retro.effective_from = first.effective_from - Duration::minutes(1);
    assert!(publish_policy(&db, retro, &actor, &[42; 32]).await.is_err());
    let mut shrink = make();
    shrink.expected_version = 1;
    shrink.effective_from = first.effective_from + Duration::minutes(1);
    shrink.legacy_coverage = None;
    assert!(
        publish_policy(&db, shrink, &actor, &[42; 32])
            .await
            .is_err()
    );
    assert_eq!(
        list_policies(&db, &fixtures::scope()).await.unwrap(),
        vec![first]
    );
}
