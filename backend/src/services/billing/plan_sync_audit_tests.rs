use super::*;
use crate::services::billing::{
    lago_client::{ServicePriceSync, standard_charge_matches},
    plan_sync,
};

fn input(code: &str) -> ServicePriceSync {
    ServicePriceSync {
        metric_code: code.into(),
        metric_name: code.into(),
        metric_description: code.into(),
        credits_per_unit: "1".into(),
    }
}

// Lago 1.50, 32efd35931cca340050c2623d811b14e75d78de9:
// app/serializers/v1/charge_serializer.rb and db/structure.sql (charges).
// NyxID's production standard-charge create sends properties.amount and
// invoiceable=true, pay_in_advance=false, prorated=false. Other fields default.
fn serialized_charge(code: &str) -> Value {
    json!({"lago_id":"charge-id", "lago_billable_metric_id":code, "code":code,
        "invoice_display_name":null, "billable_metric_code":code,
        "created_at":"2026-01-01T00:00:00Z", "charge_model":"standard",
        "invoiceable":true, "regroup_paid_fees":null, "pay_in_advance":false,
        "prorated":false, "min_amount_cents":0, "accepts_target_wallet":false,
        "properties":{"amount":"1.000000"}, "applied_pricing_unit":null,
        "lago_parent_id":null, "filters":[]})
}

#[test]
fn lago_serializer_shape_is_compared_by_economics() {
    let charge = serialized_charge("unit");
    assert!(standard_charge_matches(
        std::slice::from_ref(&charge),
        "unit",
        Some(&input("unit"))
    ));
    let mut extra = charge.clone();
    extra["properties"]["presentation_group_keys"] = json!(["region"]);
    extra["properties"]["future_metadata"] = json!("not pricing");
    assert!(
        standard_charge_matches(&[extra], "unit", Some(&input("unit"))),
        "presentation and unknown fields must not cause perpetual repair"
    );
    for amount in ["1e0", "1.0000000000000000"] {
        let mut equivalent = charge.clone();
        equivalent["properties"]["amount"] = json!(amount);
        assert!(standard_charge_matches(
            &[equivalent],
            "unit",
            Some(&input("unit"))
        ));
    }
    let mut numeric = charge.clone();
    numeric["properties"]["amount"] = json!(1);
    assert!(standard_charge_matches(
        &[numeric],
        "unit",
        Some(&input("unit"))
    ));
    let mut sparse = charge;
    for field in [
        "invoiceable",
        "pay_in_advance",
        "prorated",
        "min_amount_cents",
        "filters",
    ] {
        sparse.as_object_mut().unwrap().remove(field);
    }
    assert!(standard_charge_matches(
        &[sparse],
        "unit",
        Some(&input("unit"))
    ));
    for field in ["pricing_group_keys", "grouped_by", "group_properties"] {
        let mut grouped = serialized_charge("unit");
        grouped["properties"][field] = json!(["region"]);
        assert!(
            !standard_charge_matches(&[grouped], "unit", Some(&input("unit"))),
            "{field}"
        );
    }
    assert!(!standard_charge_matches(
        &[serialized_charge("unit"), serialized_charge("unit")],
        "unit",
        Some(&input("unit"))
    ));
    for (field, value) in [
        ("invoiceable", json!(false)),
        ("pay_in_advance", json!(true)),
        ("prorated", json!(true)),
        ("min_amount_cents", json!(10)),
        ("filters", json!([{}])),
        ("group_properties", json!([{}])),
        ("charge_model", json!("graduated")),
        (
            "applied_pricing_unit",
            json!({"code":"token", "conversion_rate":"2"}),
        ),
    ] {
        let mut changed = serialized_charge("unit");
        changed[field] = value;
        assert!(
            !standard_charge_matches(&[changed], "unit", Some(&input("unit"))),
            "{field}"
        );
    }
}

async fn audit(f: &Fixture) {
    Box::pin(plan_sync::repair(
        &f.db,
        f.lago.as_ref(),
        "standard",
        "standard",
    ))
    .await
    .unwrap();
}
async fn published(f: &Fixture, code: &str) -> DownstreamService {
    let s = f.service(code).await;
    assert!(Box::pin(f.billing.sync_service_price(&s)).await.unwrap());
    s
}
fn reset_commands(f: &Fixture) {
    f.commands.store(0, Ordering::SeqCst);
    f.catalog_documents.store(0, Ordering::SeqCst);
    f.command_names.lock().unwrap().clear();
}
fn assert_read_only(f: &Fixture) {
    let names = f.command_names.lock().unwrap();
    assert!(
        names
            .iter()
            .all(|c| matches!(c.as_str(), "find" | "getMore" | "aggregate")),
        "audit issued writes: {names:?}"
    );
}
async fn intent(f: &Fixture, code: &str) -> Document {
    f.db.collection::<Document>("billing_plan_charges")
        .find_one(doc! {"plan_code":"standard","metric_code":code})
        .await
        .unwrap()
        .unwrap()
}
fn drift(f: &Fixture) {
    for charge in f.provider.charges.lock().unwrap().iter_mut() {
        charge["properties"]["amount"] = json!("99");
    }
}

#[tokio::test]
async fn clean_audit_reads_only_priced_catalog_rows_and_never_writes() {
    let f = Fixture::new("plan_clean_audit").await;
    for n in 0..20 {
        published(&f, &format!("priced{n}")).await;
    }
    for n in 0..280 {
        let mut s = dummy_service();
        s.id = Uuid::new_v4().to_string();
        s.slug = format!("free{n}");
        s.billing = None;
        f.db.collection::<DownstreamService>(SERVICES)
            .insert_one(s)
            .await
            .unwrap();
    }
    audit(&f).await;
    reset_commands(&f);
    audit(&f).await;
    println!(
        "clean audit 300 services/20 priced: commands={}, catalog_documents={}, names={:?}",
        f.commands.load(Ordering::SeqCst),
        f.catalog_documents.load(Ordering::SeqCst),
        f.command_names.lock().unwrap()
    );
    assert_read_only(&f);
    assert_eq!(f.catalog_documents.load(Ordering::SeqCst), 20);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn real_shape_audit_does_not_repair_presentation_fields() {
    let f = Fixture::new("plan_shape_audit").await;
    published(&f, "a").await;
    audit(&f).await;
    let mut charge = serialized_charge("platform_svc_a");
    charge["properties"]["presentation_group_keys"] = json!(["region"]);
    charge.as_object_mut().unwrap().remove("pay_in_advance");
    *f.provider.charges.lock().unwrap() = vec![charge];
    let before = f.provider.updates.lock().unwrap().len();
    reset_commands(&f);
    audit(&f).await;
    assert_eq!(f.provider.updates.lock().unwrap().len(), before);
    assert_read_only(&f);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn amount_drift_is_repaired_once() {
    let f = Fixture::new("plan_amount_drift").await;
    published(&f, "a").await;
    audit(&f).await;
    drift(&f);
    let before = f.provider.updates.lock().unwrap().len();
    audit(&f).await;
    assert_eq!(f.provider.updates.lock().unwrap().len(), before + 1);
    reset_commands(&f);
    audit(&f).await;
    assert_eq!(f.provider.updates.lock().unwrap().len(), before + 1);
    assert_read_only(&f);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn persistent_mismatch_backs_off_and_has_one_diagnostic() {
    let f = Fixture::new("plan_repair_backoff").await;
    let a = published(&f, "a").await;
    audit(&f).await;
    f.provider
        .stuck_codes
        .lock()
        .unwrap()
        .push("platform_svc_a".into());
    drift(&f);
    let before = f.provider.updates.lock().unwrap().len();
    let _ = Box::pin(plan_sync::repair(
        &f.db,
        f.lago.as_ref(),
        "standard",
        "standard",
    ))
    .await;
    let first = intent(&f, "platform_svc_a").await;
    assert_eq!(first.get_i64("repair_attempts"), Ok(1));
    let next = first
        .get_datetime("repair_next_at")
        .unwrap()
        .timestamp_millis();
    let failed = first
        .get_datetime("repair_failed_at")
        .unwrap()
        .timestamp_millis();
    assert_eq!(next - failed, 300_000);
    expire_plan_lease(&f.db, "standard").await;
    reset_commands(&f);
    audit(&f).await;
    assert_read_only(&f);
    assert_eq!(f.provider.updates.lock().unwrap().len(), before + 1);
    let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
        .await
        .unwrap();
    let diagnostics: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.code.starts_with("billing_plan_repair:"))
        .collect();
    assert_eq!(diagnostics.len(), 1);
    assert!(
        diagnostics[0].detail.contains("standard")
            && diagnostics[0].detail.contains("platform_svc_a")
    );
    for attempt in 2..=11 {
        f.db.collection::<Document>("billing_plan_charges")
            .update_one(
                doc! {"metric_code":"platform_svc_a"},
                doc! {"$set":{"repair_next_at":mongodb::bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        expire_plan_lease(&f.db, "standard").await;
        let now = chrono::Utc::now().timestamp_millis();
        audit(&f).await;
        let row = intent(&f, "platform_svc_a").await;
        assert_eq!(row.get_i64("repair_attempts"), Ok(attempt));
        let delay = row
            .get_datetime("repair_next_at")
            .unwrap()
            .timestamp_millis()
            - now;
        let expected = (300_000_i64 * 2_i64.pow((attempt - 1) as u32)).min(86_400_000);
        assert!(
            (expected..expected + 5000).contains(&delay),
            "{attempt}: {delay}"
        );
        assert_eq!(
            row.get_datetime("repair_failed_at"),
            first.get_datetime("repair_failed_at")
        );
    }
    f.provider.stuck_codes.lock().unwrap().clear();
    expire_plan_lease(&f.db, "standard").await;
    assert!(f.billing.sync_service_price(&a).await.unwrap());
    assert!(
        intent(&f, "platform_svc_a")
            .await
            .get("repair_next_at")
            .is_none()
    );
    audit(&f).await;
    assert!(
        crate::services::billing::startup_diagnostics(&f.db)
            .await
            .unwrap()
            .iter()
            .all(|d| !d.code.starts_with("billing_plan_repair:"))
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn one_rejected_code_does_not_starve_later_repairs() {
    let f = Fixture::new("plan_repair_starvation").await;
    for code in ["a", "b", "c"] {
        published(&f, code).await;
    }
    audit(&f).await;
    drift(&f);
    f.provider
        .stuck_codes
        .lock()
        .unwrap()
        .push("platform_svc_a".into());
    let _ = Box::pin(plan_sync::repair(
        &f.db,
        f.lago.as_ref(),
        "standard",
        "standard",
    ))
    .await;
    let charges = f.provider.charges.lock().unwrap().clone();
    for code in ["platform_svc_b", "platform_svc_c"] {
        assert!(
            standard_charge_matches(&charges, code, Some(&input(code))),
            "{code} starved"
        );
    }
    assert_eq!(
        intent(&f, "platform_svc_a")
            .await
            .get_i64("repair_attempts"),
        Ok(1)
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn cache_failure_does_not_starve_later_codes() {
    let f = Fixture::new("plan_cache_starvation").await;
    for code in ["a", "b", "c"] {
        published(&f, code).await;
    }
    // A published legacy code can enter the audit before any intent exists.
    f.db.collection::<Document>("billing_plan_charges")
        .delete_one(doc! {"metric_code":"platform_svc_a"})
        .await
        .unwrap();
    f.db.collection::<Document>("billing_rate_cache")
        .delete_many(doc! {})
        .await
        .unwrap();
    f.db.run_command(doc!{"collMod":"billing_rate_cache","validator":{"lago_metric_code":{"$ne":"platform_svc_a"}},"validationLevel":"strict"}).await.unwrap();
    let _ = Box::pin(plan_sync::repair(
        &f.db,
        f.lago.as_ref(),
        "standard",
        "standard",
    ))
    .await;
    for code in ["platform_svc_b", "platform_svc_c"] {
        assert_eq!(
            f.db.collection::<Document>("billing_rate_cache")
                .count_documents(doc! {"lago_metric_code":code})
                .await
                .unwrap(),
            1
        );
    }
    assert_eq!(
        intent(&f, "platform_svc_a").await.get_str("repair_reason"),
        Ok("cache_write_failed")
    );
    reset_commands(&f);
    audit(&f).await;
    assert_read_only(&f);
    f.db.run_command(doc! {"collMod":"billing_rate_cache","validator":{}})
        .await
        .unwrap();
    f.db.collection::<Document>("billing_plan_charges")
        .update_one(
            doc! {"metric_code":"platform_svc_a"},
            doc! {"$set":{"repair_next_at":mongodb::bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    audit(&f).await;
    assert_eq!(
        f.db.collection::<Document>("billing_rate_cache")
            .count_documents(doc! {})
            .await
            .unwrap(),
        3
    );
    assert!(
        intent(&f, "platform_svc_a")
            .await
            .get("repair_reason")
            .is_none()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn audit_does_not_publish_pending_or_failed_prices_from_old_intents() {
    let f = Fixture::new("plan_pending_audit").await;
    let a = published(&f, "a").await;
    for status in ["pending", "failed"] {
        f.db.collection::<Document>(SERVICES).update_one(doc!{"_id":&a.id},doc!{"$set":{"billing.platform_pricing.sync_status":status,"billing.platform_pricing.credits_per_unit":"2"}}).await.unwrap();
        f.db.collection::<Document>("billing_plan_charges")
            .update_one(
                doc! {"metric_code":"platform_svc_a"},
                doc! {"$set":{"credits_per_unit":"2"}},
            )
            .await
            .unwrap();
        let before = f.provider.updates.lock().unwrap().len();
        audit(&f).await;
        assert_eq!(
            f.provider.updates.lock().unwrap().len(),
            before,
            "{status} belongs to normal retry"
        );
        let cache =
            f.db.collection::<Document>("billing_rate_cache")
                .find_one(doc! {"lago_metric_code":"platform_svc_a"})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            cache.get_i64("credits_per_unit_pico"),
            Ok(1_000_000_000_000)
        );
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn audit_ignores_unpublished_immutable_intents() {
    let f = Fixture::new("plan_unpublished_immutable").await;
    published(&f, "a").await;
    audit(&f).await;
    for code in [
        "pending_tariff",
        "failed_card",
        "pending_machine",
        "deleted_source",
    ] {
        f.db.collection::<Document>("billing_plan_charges")
            .insert_one(doc! {
                "_id":Uuid::new_v4().to_string(),"plan_code":"standard","metric_code":code,
                "service_id":null,"writes_rate_cache":true,"metric_name":code,
                "metric_description":code,"credits_per_unit":"1"
            })
            .await
            .unwrap();
    }
    // Pending/failed immutable rows are owned by their publication retry. A
    // positive intent alone, even with a deleted source, is not publication.
    f.db.collection::<Document>("platform_model_tariffs")
        .insert_one(doc! {"_id":"pending_tariff","status":"pending"})
        .await
        .unwrap();
    f.db.collection::<Document>("sandbox_rate_cards")
        .insert_one(doc! {"_id":"failed_card","plan_code":"standard","sync_status":"failed"})
        .await
        .unwrap();
    f.db.collection::<Document>("machine_runtime_rates")
        .insert_one(doc! {"_id":"pending_machine","plan_code":"standard","sync_status":"pending"})
        .await
        .unwrap();
    f.db.collection::<Document>("billing_plan_sync")
        .update_one(
            doc! {"_id":"standard"},
            doc! {"$set":{"dirty":true},"$inc":{"generation":1_i64}},
        )
        .await
        .unwrap();
    let before = f.provider.updates.lock().unwrap().len();
    audit(&f).await;
    assert_eq!(f.provider.updates.lock().unwrap().len(), before);
    assert_eq!(
        f.db.collection::<Document>("billing_rate_cache")
            .count_documents(doc! {"lago_metric_code":{"$ne":"platform_svc_a"}})
            .await
            .unwrap(),
        0
    );
    reset_commands(&f);
    audit(&f).await;
    assert_read_only(&f);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn matching_late_result_completes_invalidation_and_clears_diagnostic() {
    let f = Fixture::new("plan_late_repair_recovery").await;
    published(&f, "a").await;
    audit(&f).await;
    f.provider
        .stuck_codes
        .lock()
        .unwrap()
        .push("platform_svc_a".into());
    drift(&f);
    let _ = Box::pin(plan_sync::repair(
        &f.db,
        f.lago.as_ref(),
        "standard",
        "standard",
    ))
    .await;
    expire_plan_lease(&f.db, "standard").await;
    // A tax read completed before the previously uncertain provider job landed.
    f.db.collection::<Document>("billing_tax_profiles")
        .insert_one(
            doc! {"_id":"tax", "plan_code":"standard","fetched_at":mongodb::bson::DateTime::now()},
        )
        .await
        .unwrap();
    f.provider.charges.lock().unwrap()[0]["properties"]["amount"] = json!("1");
    let before = f.provider.updates.lock().unwrap().len();
    audit(&f).await;
    assert_eq!(
        f.provider.updates.lock().unwrap().len(),
        before,
        "matching recovery needs no PUT"
    );
    assert!(
        intent(&f, "platform_svc_a")
            .await
            .get("repair_reason")
            .is_none()
    );
    let tax =
        f.db.collection::<Document>("billing_tax_profiles")
            .find_one(doc! {"_id":"tax"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        tax.get_datetime("fetched_at").unwrap().timestamp_millis(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn integrity_diagnostics_command_count() {
    let f = Fixture::new("plan_integrity_command_count").await;
    reset_commands(&f);
    let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
        .await
        .unwrap();
    println!(
        "healthy Integrity billing-diagnostics MongoDB commands: {}; names={:?}",
        f.commands.load(Ordering::SeqCst),
        f.command_names.lock().unwrap()
    );
    assert!(diagnostics.is_empty());
    assert_read_only(&f);
    f.db.drop().await.unwrap();
}
