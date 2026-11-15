use super::*;
use crate::billing_integration_tests::usage_checkpoints::Fixture;
use chrono::{Duration, Utc};

async fn roster(
    f: &Fixture,
    generation: &str,
    start: Option<bson::DateTime>,
    observed: bson::DateTime,
) {
    f.db.collection::<Document>("security_replicas").insert_one(doc!{"_id":generation,"generation_id":generation,"started_at":start,"observed_at":observed}).await.unwrap();
}
async fn read_registry(f: &Fixture) -> AppResult<Registry> {
    super::super::hold_conservation::backlog::retry_snapshot_read(|| async {
        let mut tx = f.db.client().start_session().snapshot(true).await?;
        registry(&f.db, &mut tx).await
    })
    .await
}
#[tokio::test]
async fn stuck_request_classification_ownership_and_protocol_matrix() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    roster(
        &f,
        "live",
        Some(bson::DateTime::now()),
        bson::DateTime::now(),
    )
    .await;
    let r = read_registry(&f).await.unwrap();
    let mut raw =
        f.db.collection::<Document>(METERS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    raw.insert(
        "created_at",
        bson::DateTime::from_chrono(Utc::now() - Duration::days(8)),
    );
    raw.remove("forwarded_owner");
    assert!(classify(&raw, Some(&r)).unwrap().eligible);
    let mut recent = raw.clone();
    recent.insert(
        "created_at",
        bson::DateTime::from_millis(r.now.timestamp_millis() - 59 * 60_000),
    );
    assert_eq!(
        classify(&recent, Some(&r)).unwrap().label,
        "legacy_may_be_live"
    );
    recent.insert(
        "created_at",
        bson::DateTime::from_millis(r.now.timestamp_millis() - 61 * 60_000),
    );
    assert!(classify(&recent, Some(&r)).unwrap().eligible);
    for (change, label) in [
        (doc! {"pool_attempt":{}}, "excluded_pool"),
        (doc! {"realtime_session":{}}, "excluded_realtime"),
        (doc! {"managed_execution_id":"managed"}, "excluded_managed"),
        (doc! {"model_tariff":{}}, "excluded_certified_model"),
        (
            doc! {"metric":"machine_running_seconds"},
            "excluded_runtime",
        ),
        (
            doc! {"metric":"sandbox_cpu_millis_seconds"},
            "excluded_runtime",
        ),
        (doc! {"metric":"voice_seconds"}, "excluded_voice"),
        (
            doc! {"billing_request_id":"voice:legacy:1"},
            "excluded_voice",
        ),
        (doc! {"quantity":0_i64}, "final_quantity_awaiting_retry"),
        (
            doc! {"pending_platform_usage":{}},
            "durable_finalization_intent",
        ),
        (
            doc! {"pending_resale_quantity":0_i64},
            "durable_finalization_intent",
        ),
        (
            doc! {"usage_checkpoint":{"final_report":true}},
            "has_final_checkpoint",
        ),
    ] {
        let mut row = raw.clone();
        row.extend(change);
        let c = classify(&row, Some(&r)).unwrap();
        assert_eq!(c.label, label);
        assert!(!c.eligible, "{label}");
    }
    let mut row = raw.clone();
    row.insert(
        "forwarded_owner",
        doc! {"generation_id":"live","ingress":"proxy","marked_at":bson::DateTime::from_millis(0)},
    );
    assert_eq!(
        classify(&row, Some(&r)).unwrap().label,
        "owned_by_live_generation"
    );
    row.insert(
        "forwarded_owner",
        doc! {"generation_id":"gone","ingress":"proxy","marked_at":bson::DateTime::from_millis(0)},
    );
    assert!(classify(&row, Some(&r)).unwrap().eligible);
    row.get_document_mut("forwarded_owner")
        .unwrap()
        .insert("marked_at", r.now);
    assert_eq!(
        classify(&row, Some(&r)).unwrap().label,
        "owner_expiry_margin"
    );
    row.get_document_mut("forwarded_owner")
        .unwrap()
        .insert("ingress", "ssh_tunnel");
    assert_eq!(
        classify(&row, Some(&r)).unwrap().label,
        "excluded_ssh_tunnel"
    );
    assert_eq!(classify(&raw, None).unwrap().label, "registry_unavailable");
    roster(&f, "old-live", None, bson::DateTime::now()).await;
    assert_eq!(
        classify(&raw, Some(&read_registry(&f).await.unwrap()))
            .unwrap()
            .label,
        "legacy_registry_start_unknown"
    );
    f.db.collection::<Document>("security_replicas").update_one(doc!{"_id":"old-live"},doc!{"$set":{"observed_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(164))}}).await.unwrap();
    assert!(
        !classify(&raw, Some(&read_registry(&f).await.unwrap()))
            .unwrap()
            .eligible
    );
    f.db.collection::<Document>("security_replicas").update_one(doc!{"_id":"old-live"},doc!{"$set":{"observed_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(166))}}).await.unwrap();
    assert!(
        classify(&raw, Some(&read_registry(&f).await.unwrap()))
            .unwrap()
            .eligible
    );
    raw.insert("created_at", bson::DateTime::now());
    assert!(
        !classify(&raw, Some(&read_registry(&f).await.unwrap()))
            .unwrap()
            .eligible
    );
    f.db.collection::<Document>("security_replicas")
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(read_registry(&f).await.is_err());
    roster(&f, "bad", None, bson::DateTime::now()).await;
    f.db.collection::<Document>("security_replicas")
        .update_one(doc! {}, doc! {"$set":{"observed_at":"bad"}})
        .await
        .unwrap();
    assert!(read_registry(&f).await.is_err());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn stuck_request_inventory_is_indexed_bounded_and_read_only() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    roster(
        &f,
        "live",
        Some(bson::DateTime::now()),
        bson::DateTime::now(),
    )
    .await;
    let mut raw =
        f.db.collection::<Document>(METERS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    raw.remove("forwarded_owner");
    raw.insert("created_at", bson::DateTime::from_millis(0));
    f.db.collection::<Document>(METERS)
        .replace_one(doc! {}, &raw)
        .await
        .unwrap();
    let plan=f.db.run_command(doc!{"explain":{"aggregate":METERS,"pipeline":inventory::pipeline(),"cursor":{}},"verbosity":"executionStats"}).await.unwrap();
    let plan = serde_json::to_string(&plan).unwrap();
    assert!(plan.contains("IXSCAN"), "{plan}");
    assert!(!plan.contains("COLLSCAN"), "{plan}");
    println!("STUCK_REQUEST_INVENTORY_EXPLAIN {plan}");
    f.count();
    let result = diagnostics(&f.db).await.unwrap();
    let commands = std::mem::take(&mut *f.commands.lock().unwrap());
    println!("STUCK_REQUEST_INVENTORY_COMMANDS {commands:?}");
    assert!(commands.iter().all(|c| matches!(
        c.as_str(),
        "aggregate" | "getMore" | "killCursors" | "abortTransaction"
    )));
    assert!(
        result[0].detail.contains("distinct_requests=1"),
        "{}",
        result[0].detail
    );
    assert_eq!(
        raw,
        f.db.collection::<Document>(METERS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
    );
    f.db.collection::<Document>("security_replicas")
        .drop()
        .await
        .unwrap();
    let result = diagnostics(&f.db).await.unwrap();
    assert!(
        result[0]
            .detail
            .contains("class=registry_unavailable eligible=false"),
        "{}",
        result[0].detail
    );
    // One bad row remains visible as incompleteness, not a page error.
    f.db.collection::<Document>(METERS)
        .update_one(doc! {}, doc! {"$set":{"reserved_credits":"malformed"}})
        .await
        .unwrap();
    let result = diagnostics(&f.db).await.unwrap();
    assert!(
        result[0].detail.contains("invalid_rows=1"),
        "{}",
        result[0].detail
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn stuck_request_inventory_deduplicates_requests_and_separates_benefit_units() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    roster(
        &f,
        "survivor",
        Some(bson::DateTime::now()),
        bson::DateTime::now(),
    )
    .await;
    f.db.collection::<Document>("credit_grants")
        .insert_one(doc! {"_id":"g","recipient_user_id":&f.owner})
        .await
        .unwrap();
    f.db.collection::<Document>("usage_allowance_periods")
        .insert_one(doc! {"_id":"p","owner_user_id":&f.owner})
        .await
        .unwrap();
    f.db.collection::<Document>(METERS).update_many(doc!{},doc!{"$set":{
        "created_at":bson::DateTime::from_millis(0),
        "funding.grant_reservations":[{"grant_id":"g","amount":Credits::from_whole(2)}],
        "funding.allowance_reservations":[{"allowance_id":"a","period_id":"p","quantity":7_i64}]
    },"$unset":{"forwarded_owner":""}}).await.unwrap();
    let mut raw =
        f.db.collection::<Document>(METERS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    raw.insert("_id", "sibling");
    raw.insert("transaction_id", "sibling");
    f.db.collection::<Document>(METERS)
        .insert_one(raw)
        .await
        .unwrap();
    let d = diagnostics(&f.db).await.unwrap();
    assert!(
        d[0].detail.contains("rows=2 distinct_requests=1"),
        "{}",
        d[0].detail
    );
    assert!(
        d[0].detail
            .contains("kind=grant unit=credits rows=2 requests=1 held=4 resources=1"),
        "{}",
        d[0].detail
    );
    assert!(
        d[0].detail
            .contains("kind=allowance unit=tokens rows=2 requests=1 held=14 resources=1"),
        "{}",
        d[0].detail
    );
    // A coordinator marker protects both rows, including the unmarked sibling.
    f.db.collection::<Document>(METERS)
        .update_one(
            doc! {"_id":"sibling"},
            doc! {"$set":{"realtime_session":{"closed":false}}},
        )
        .await
        .unwrap();
    let d = diagnostics(&f.db).await.unwrap();
    assert!(
        d[0].detail
            .contains("class=excluded_realtime eligible=false age=over_7d rows=2 requests=1"),
        "{}",
        d[0].detail
    );
    assert!(!d[0].detail.contains("eligible=true"));
    f.db.drop().await.unwrap();
}

#[derive(Clone, Default)]
struct ReadGate {
    entered: std::sync::Arc<tokio::sync::Notify>,
    resume: std::sync::Arc<tokio::sync::Notify>,
    used: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
tokio::task_local! { static READ_GATE: ReadGate; }
pub(super) async fn after_first_row() {
    if let Ok(gate) = READ_GATE.try_with(Clone::clone)
        && !gate.used.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        gate.entered.notify_one();
        gate.resume.notified().await;
    }
}

#[tokio::test]
async fn stuck_request_inventory_pins_history_across_cursor_batches() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    roster(
        &f,
        "live",
        Some(bson::DateTime::now()),
        bson::DateTime::now(),
    )
    .await;
    let raw =
        f.db.collection::<Document>(METERS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    let mut rows = Vec::new();
    for i in 0..600 {
        let mut row = raw.clone();
        row.insert("_id", format!("cursor-{i}"));
        row.insert("transaction_id", format!("cursor-{i}"));
        row.insert("billing_request_id", format!("cursor-{i}"));
        rows.push(row);
    }
    f.db.collection::<Document>(METERS)
        .insert_many(rows)
        .await
        .unwrap();
    let gate = ReadGate::default();
    let reading = READ_GATE.scope(gate.clone(), diagnostics(&f.db));
    let writing = async {
        gate.entered.notified().await;
        // These writes advance history while the reader is between cursor batches.
        // A restart would silently switch the inventory to the new, empty state.
        f.db.collection::<Document>(METERS)
            .update_many(doc! {}, doc! {"$set":{"released":true}})
            .await
            .unwrap();
        gate.resume.notify_one();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(reading, writing)
    })
    .await
    .unwrap();
    let result = result.unwrap();
    assert_eq!(result.len(), 1, "{result:?}");
    assert!(
        result[0].detail.contains("rows=601 distinct_requests=601"),
        "{}",
        result[0].detail
    );
    assert!(
        result[0].detail.starts_with("complete=true"),
        "{}",
        result[0].detail
    );
    assert!(diagnostics(&f.db).await.unwrap().is_empty());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn stuck_request_inventory_failure_reasons_are_metadata_only() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    roster(
        &f,
        "live",
        Some(bson::DateTime::now()),
        bson::DateTime::now(),
    )
    .await;
    f.db.collection::<Document>(METERS)
        .drop_index(inventory::INDEX)
        .await
        .unwrap();
    let d = diagnostics(&f.db).await.unwrap();
    assert!(
        d[0].detail
            .contains("reason=index_unavailable stage=inventory database_code=2 elapsed_ms="),
        "{}",
        d[0].detail
    );
    assert!(!d[0].detail.contains(&f.owner));
    f.db.collection::<Document>(METERS)
        .create_index(index())
        .await
        .unwrap();
    f.db.collection::<Document>("security_replicas")
        .drop()
        .await
        .unwrap();
    let d = diagnostics(&f.db).await.unwrap();
    assert!(
        d[0].detail
            .contains("reason=registry_evidence_unavailable stage=registry"),
        "{}",
        d[0].detail
    );
    assert!(!d[0].detail.contains("eligible=true"));
    roster(
        &f,
        "live",
        Some(bson::DateTime::now()),
        bson::DateTime::now(),
    )
    .await;
    f.db.collection::<Document>(METERS)
        .update_many(doc! {}, doc! {"$set":{"reserved_credits":"bad"}})
        .await
        .unwrap();
    let d = diagnostics(&f.db).await.unwrap();
    assert!(
        d[0].detail.contains("reason=row_decode_error stage=decode"),
        "{}",
        d[0].detail
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn stuck_request_inventory_timeout_is_explicit_and_bounded() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    roster(
        &f,
        "live",
        Some(bson::DateTime::now()),
        bson::DateTime::now(),
    )
    .await;
    let gate = ReadGate::default();
    let start = std::time::Instant::now();
    // Stop consuming an open cursor; cancellation must also release its transaction.
    let result = READ_GATE.scope(gate, diagnostics(&f.db)).await.unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(6));
    assert!(
        result[0]
            .detail
            .contains("reason=deadline_exceeded stage=inventory elapsed_ms="),
        "{}",
        result[0].detail
    );
    assert!(
        f.db.collection::<Document>(METERS)
            .find_one(doc! {"released":false})
            .await
            .unwrap()
            .is_some()
    );
    f.db.drop().await.unwrap();
}

#[path = "scale_tests.rs"]
mod scale;
