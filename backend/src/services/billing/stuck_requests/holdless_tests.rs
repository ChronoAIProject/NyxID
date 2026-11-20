use super::*;
use crate::billing_integration_tests::usage_checkpoints::Fixture;
use crate::models::usage_meter::{UsageMeterRow, UsageStatus};
use crate::services::billing::{meter, reconcile::BillingReconciler, usage_rollup};
use chrono::Utc;
use std::{collections::BTreeMap, sync::Arc};

tokio::task_local! { pub(super) static UNTIMED: (); }
async fn run(db: &mongodb::Database) -> AppResult<()> {
    UNTIMED.scope((), super::recover(db)).await
}
fn only_metadata_writes(f: &Fixture) {
    for (command, collection) in std::mem::take(&mut *f.writes.lock().unwrap()) {
        assert!(
            matches!(collection.as_str(), METERS | STATE | "audit_log"),
            "unexpected {command} on {collection}"
        );
    }
}

async fn dead(f: &Fixture) {
    f.db.collection::<Document>("security_replicas")
        .insert_one(doc! {
            "_id":"survivor", "generation_id":"survivor", "started_at":bson::DateTime::now(),
            "observed_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    f.db.collection::<Document>(METERS).update_many(doc! {}, doc! {"$set": {
        "created_at":bson::DateTime::from_millis(1_700_000_000_000),
        "forwarded_owner":{"generation_id":"dead", "ingress":"proxy", "marked_at":bson::DateTime::from_millis(0)},
    }}).await.unwrap();
}

async fn open(f: &Fixture, wallet: bool) -> meter::MeteredProxyContext {
    let metered = f.open(false).await;
    if !wallet {
        return metered;
    }
    f.db.collection::<Document>(METERS)
        .delete_many(doc! {})
        .await
        .unwrap();
    let mut route = metered.route.unwrap().with_native_text_bound(0, 0, 0);
    route.platform_metric = BillingMetric::Images;
    route.platform_billable = true;
    route.service_platform_billable = true;
    let metered = f.service.open(&route).await.unwrap();
    f.service.mark_forwarded(&metered).await.unwrap();
    assert_eq!(f.row().await.reserved_credits, Credits::ZERO);
    metered
}

async fn money(f: &Fixture) -> BTreeMap<String, Vec<Vec<u8>>> {
    let mut result = BTreeMap::new();
    for name in f.db.list_collection_names().await.unwrap() {
        if name == METERS
            || name == STATE
            || name == "audit_log"
            || name == "security_replicas"
            || name.starts_with("usage_rollup")
        {
            continue;
        }
        let docs: Vec<Document> =
            f.db.collection::<Document>(&name)
                .find(doc! {})
                .sort(doc! {"_id":1})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
        result.insert(
            name,
            docs.iter().map(|d| bson::to_vec(d).unwrap()).collect(),
        );
    }
    result
}

async fn raw(f: &Fixture) -> Document {
    f.db.collection::<Document>(METERS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap()
}

async fn seed_terminal(f: &Fixture) {
    f.db.collection::<Document>(METERS).update_many(doc! {}, doc! {"$set": {
        "status":"finalized", "quantity":0_i64, "finalized_at":bson::DateTime::now(),
        "stuck_request_exit":{"policy":"waive_unknown_consumption", "lane":"holdless", "ended_at":bson::DateTime::now()},
        "funding.settled":true, "funding.wallet_charge_credits":Credits::ZERO,
        "funding.wallet_funded":Credits::ZERO, "funding.grant_funded":Credits::ZERO,
        "funding.allowance_funded":Credits::ZERO, "funding.total_charge":Credits::ZERO,
        "funding.lago_billable_quantity_micros":0_i64,
    }}).await.unwrap();
}

#[tokio::test]
async fn holdless_dead_request_ends_once_without_money_movement() {
    for wallet in [false, true] {
        let f = Fixture::new().await;
        let metered = open(&f, wallet).await;
        dead(&f).await;
        let before = money(&f).await;
        f.writes.lock().unwrap().clear();
        run(&f.db).await.unwrap();
        let row = f.row().await;
        assert_eq!(row.quantity, Some(0), "dead holdless request must finish");
        assert_eq!(row.status, UsageStatus::Finalized);
        let saved = raw(&f).await;
        let marker = saved.get_document("stuck_request_exit").unwrap();
        assert_eq!(
            marker.get_str("policy").unwrap(),
            "waive_unknown_consumption"
        );
        assert_eq!(marker.get_str("lane").unwrap(), "holdless");
        meter::settle_persisted(&f.db, vec![row]).await.unwrap();
        assert!(f.row().await.released);
        only_metadata_writes(&f);
        f.service
            .settle(
                &metered,
                crate::models::service_billing::PlatformUsage::llm_completion(0, 7),
                None,
                None,
            )
            .await
            .unwrap();
        run(&f.db).await.unwrap();
        assert_eq!(f.row().await.quantity, Some(0));
        assert!(money(&f).await == before, "resource BSON changed");
        only_metadata_writes(&f);
        let state =
            f.db.collection::<Document>(STATE)
                .find_one(doc! {"_id":STATE_ID})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(state.get_i64("holdless_ended_requests").unwrap(), 1);
        assert_eq!(state.get_i64("holdless_ended_rows").unwrap(), 1);
        assert_eq!(state.get_i64("ended_requests").unwrap_or(0), 0);
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn holdless_metering_ack_retention_and_original_day_rollup() {
    afterlife(false).await;
}
#[tokio::test]
async fn holdless_wallet_ack_retention_and_original_day_rollup() {
    afterlife(true).await;
}
async fn afterlife(wallet: bool) {
    let f = Fixture::new().await;
    open(&f, wallet).await;
    dead(&f).await;
    seed_terminal(&f).await;
    let before = money(&f).await;
    f.writes.lock().unwrap().clear();
    let row = f.row().await;
    meter::settle_persisted(&f.db, vec![row]).await.unwrap();
    let released = f.row().await;
    assert!(released.released);
    assert!(released.lago_acked, "release must acknowledge its own zero");
    assert_eq!(
        released.expires_at,
        Some(released.updated_at + chrono::Duration::days(30)),
        "release applies the same 30-day horizon as ordinary acknowledgement"
    );
    assert!(
        money(&f).await == before,
        "ordinary settlement and release change no resources"
    );
    only_metadata_writes(&f);
    f.db.collection::<Document>(METERS)
        .update_many(
            doc! {},
            doc! {"$set":{"updated_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let lago = Arc::new(crate::billing_integration_tests::FakeLago::default());
    let reconciler = BillingReconciler::new(
        f.db.clone(),
        Some(lago.clone()),
        Arc::new(crate::test_utils::test_app_config()),
    );
    let before_push = raw(&f).await;
    let stats = reconciler.push_usage_for_test().await.unwrap();
    assert_eq!(
        raw(&f).await,
        before_push,
        "push never selects the finished row"
    );
    assert_eq!(stats.pushed, 0);
    assert_eq!(
        lago.recorded_events
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    let row = f.row().await;
    assert!(
        row.lago_acked,
        "holdless metering rows need acknowledgement too"
    );
    assert!(row.expires_at.is_some_and(|at| at > Utc::now()));
    assert!(
        money(&f).await == before,
        "acknowledgement changes no wallet, even its generation"
    );
    only_metadata_writes(&f);
    assert_eq!(usage_rollup::fold_once(&f.db, Utc::now()).await.unwrap(), 1);
    let daily = f
        .db
        .collection::<crate::models::usage_rollup_daily::UsageRollupDaily>("usage_rollup_daily")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(daily.day, usage_rollup::day(row.created_at));
    assert_eq!(daily.measures.quantity, 0);
    assert_eq!(daily.measures.requests, 1);
    assert_eq!(usage_rollup::fold_once(&f.db, Utc::now()).await.unwrap(), 0);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_ordinary_command_baseline() {
    for funded in [false, true] {
        let f = Fixture::new().await;
        f.count();
        let metered = f.open(funded).await;
        let open_and_forward = f.count();
        f.service
            .settle(
                &metered,
                crate::models::service_billing::PlatformUsage::llm_completion(0, 3),
                None,
                None,
            )
            .await
            .unwrap();
        let settle_and_release = f.count();
        let row = f.row().await;
        f.count();
        super::super::reconcile::mark_lago_acked(&f.db, &row.id)
            .await
            .unwrap();
        let ack = f.count();
        assert_eq!(
            (open_and_forward, settle_and_release, ack),
            if funded { (19, 29, 3) } else { (6, 3, 2) }
        );
        println!(
            "ordinary funded={funded} open+forward={open_and_forward} settle+release={settle_and_release} ack={ack}"
        );
        f.db.drop().await.unwrap();
    }
    let f = Fixture::new().await;
    f.count();
    run(&f.db).await.unwrap();
    println!("empty recovery commands={}", f.count());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_summary_is_one_chained_record_and_integrity_counts_fall() {
    let f = Fixture::new().await;
    open(&f, false).await;
    let base = raw(&f).await;
    for (id, request, layer, metric) in [
        (
            "sibling",
            base.get_str("billing_request_id").unwrap(),
            "platform",
            "input_tokens",
        ),
        ("other", "other-request", "resale", "requests"),
    ] {
        let mut row = base.clone();
        row.insert("_id", id);
        row.insert("transaction_id", id);
        row.insert("billing_request_id", request);
        row.insert("layer", layer);
        row.insert("metric", metric);
        f.db.collection::<Document>(METERS)
            .insert_one(row)
            .await
            .unwrap();
    }
    dead(&f).await;
    let before = diagnostics(&f.db).await.unwrap();
    assert!(before.iter().any(|d| {
        d.technical_details
            .as_ref()
            .unwrap()
            .contains("holdless_rows_at_least=3")
    }));
    run(&f.db).await.unwrap();
    let rows: Vec<UsageMeterRow> =
        f.db.collection::<UsageMeterRow>(METERS)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert!(rows.iter().all(|row| row.quantity == Some(0)));
    meter::settle_persisted(&f.db, rows).await.unwrap();
    let audits: Vec<Document> =
        f.db.collection::<Document>("audit_log")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(
        audits.len(),
        1,
        "one summary per pass, never one audit per request"
    );
    let audit = &audits[0];
    assert_eq!(
        audit.get_str("event_type").unwrap(),
        "billing_holdless_requests_ended"
    );
    assert!(audit.get("seq").is_some());
    assert!(audit.get_str("entry_hash").is_ok());
    let report = crate::services::audit_chain_service::verify_chain(
        &f.db,
        crate::services::audit_service::audit_chain_hmac_key().unwrap(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(report.checked_count, 1);
    assert!(report.break_info.is_none());
    let json = serde_json::to_string(audit).unwrap();
    assert!(!json.contains(&f.owner));
    assert!(!json.contains(base.get_str("billing_request_id").unwrap()));
    let after = diagnostics(&f.db).await.unwrap();
    let detail = &after
        .iter()
        .find(|d| d.code == "billing_stuck_requests")
        .unwrap()
        .technical_details
        .as_ref()
        .unwrap();
    assert!(detail.contains("holdless_rows_at_least=0"), "{detail}");
    assert!(
        detail.contains("holdless_ended_requests=2 holdless_ended_rows=3"),
        "{detail}"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_terminal_crashes_rollback_each_write_or_resume_committed_rows() {
    use recovery_tests::{Phase, at};
    for phase in [
        Phase::BeforeTerminal,
        Phase::BeforeHoldlessRow(0),
        Phase::AfterHoldlessRow(0),
        Phase::BeforeHoldlessRow(1),
        Phase::AfterHoldlessRow(1),
        Phase::AfterRows,
        Phase::BeforeCommit,
        Phase::AfterCommit,
    ] {
        let f = Fixture::new().await;
        open(&f, false).await;
        let mut sibling = raw(&f).await;
        sibling.insert("_id", "sibling");
        sibling.insert("transaction_id", "sibling");
        f.db.collection::<Document>(METERS)
            .insert_one(sibling)
            .await
            .unwrap();
        dead(&f).await;
        let before = money(&f).await;
        f.writes.lock().unwrap().clear();
        at(phase, run(&f.db)).await;
        let rows: Vec<UsageMeterRow> =
            f.db.collection::<UsageMeterRow>(METERS)
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
        assert!(rows.iter().all(|r| r.quantity
            == if phase == Phase::AfterCommit {
                Some(0)
            } else {
                None
            }));
        run(&f.db).await.unwrap();
        let rows: Vec<UsageMeterRow> =
            f.db.collection::<UsageMeterRow>(METERS)
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
        assert!(rows.iter().all(|r| r.quantity == Some(0)));
        meter::settle_persisted(&f.db, rows).await.unwrap();
        assert!(money(&f).await == before, "resource BSON changed");
        only_metadata_writes(&f);
        assert_eq!(
            f.db.collection::<Document>("audit_log")
                .count_documents(doc! {"event_type":"billing_holdless_requests_ended"})
                .await
                .unwrap(),
            1
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn holdless_real_finalizer_wins_the_transaction_race() {
    use recovery_tests::{GATE, Gate, Phase};
    let f = Fixture::new().await;
    let metered = open(&f, false).await;
    dead(&f).await;
    let gate = Gate::new(Phase::BeforeTerminal);
    let mut running = Box::pin(GATE.scope(gate.clone(), run(&f.db)));
    tokio::select! { _=&mut running=>panic!("missed terminal barrier"), ()=gate.entered.notified()=>{} }
    f.service
        .settle(
            &metered,
            crate::models::service_billing::PlatformUsage::llm_completion(0, 3),
            None,
            None,
        )
        .await
        .unwrap();
    gate.resume.notify_one();
    running.await.unwrap();
    assert_eq!(f.row().await.quantity, Some(3));
    assert!(!raw(&f).await.contains_key("stuck_request_exit"));
    assert_eq!(
        f.db.collection::<Document>("audit_log")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_live_unknown_owners_and_all_protocol_siblings_stay_untouched() {
    for change in [
        doc! {"forwarded_owner.generation_id":"survivor"},
        doc! {"forwarded_owner.marked_at":bson::DateTime::now()},
        doc! {"pool_attempt":{}},
        doc! {"realtime_session":{}},
        doc! {"managed_execution_id":"managed"},
        doc! {"model_tariff":{}},
        doc! {"metric":"machine_running_seconds"},
        doc! {"metric":"sandbox_cpu_millis_seconds"},
        doc! {"metric":"sandbox_memory_mib_seconds"},
        doc! {"metric":"sandbox_disk_gib_seconds"},
        doc! {"billing_request_id":"voice:session:0"},
        doc! {"billing_request_id":"adapter-call:call"},
        doc! {"forwarded_owner.ingress":"mcp"},
        doc! {"forwarded_owner.ingress":"ssh_exec"},
        doc! {"forwarded_owner.ingress":"ssh_tunnel"},
        doc! {"forwarded_owner.ingress":"ssh_web_terminal"},
        doc! {"forwarded_owner.ingress":"channel_inbound"},
        doc! {"forwarded_owner.ingress":"channel_outbound"},
        doc! {"forwarded_owner.ingress":"future"},
        doc! {"quantity":0_i64},
        doc! {"quantity":3_i64},
        doc! {"pending_platform_usage":{}},
        doc! {"pending_resale_quantity":0_i64},
        doc! {"usage_checkpoint":{"final_report":true,"recovered":true}},
    ] {
        let f = Fixture::new().await;
        open(&f, false).await;
        dead(&f).await;
        let mut sibling = raw(&f).await;
        sibling.insert("_id", "sibling");
        sibling.insert("transaction_id", "sibling");
        f.db.collection::<Document>(METERS)
            .insert_one(sibling)
            .await
            .unwrap();
        f.db.collection::<Document>(METERS)
            .update_one(doc! {"_id":"sibling"}, doc! {"$set":&change})
            .await
            .unwrap();
        // Identities protect their entire actual request, including its coordinator.
        if let Ok(request) = change.get_str("billing_request_id") {
            f.db.collection::<Document>(METERS)
                .update_many(doc! {}, doc! {"$set":{"billing_request_id":request}})
                .await
                .unwrap();
        }
        let before: Vec<Document> =
            f.db.collection::<Document>(METERS)
                .find(doc! {})
                .sort(doc! {"_id":1})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
        run(&f.db).await.unwrap();
        let after: Vec<Document> =
            f.db.collection::<Document>(METERS)
                .find(doc! {})
                .sort(doc! {"_id":1})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
        assert_eq!(before, after, "{change:?}");
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn holdless_legacy_requires_a_complete_registry_and_live_owners_have_no_age_limit() {
    for mode in ["absent", "unknown_start", "recent", "legacy_dead", "live"] {
        let f = Fixture::new().await;
        open(&f, false).await;
        dead(&f).await;
        f.db.collection::<Document>(METERS)
            .update_many(doc! {}, doc! {"$unset":{"forwarded_owner":""}})
            .await
            .unwrap();
        match mode {
            "absent" => {
                f.db.collection::<Document>("security_replicas")
                    .delete_many(doc! {})
                    .await
                    .unwrap();
            }
            "unknown_start" => {
                f.db.collection::<Document>("security_replicas")
                    .update_many(doc! {}, doc! {"$unset":{"started_at":""}})
                    .await
                    .unwrap();
            }
            "recent" => {
                f.db.collection::<Document>(METERS)
                    .update_many(doc! {}, doc! {"$set":{"created_at":bson::DateTime::now()}})
                    .await
                    .unwrap();
            }
            "live" => {
                f.db.collection::<Document>(METERS).update_many(doc! {}, doc! {"$set":{"forwarded_owner":{"generation_id":"survivor","ingress":"proxy","marked_at":bson::DateTime::from_millis(0)}}}).await.unwrap();
            }
            _ => {}
        }
        run(&f.db).await.unwrap();
        assert_eq!(
            f.row().await.quantity,
            if mode == "legacy_dead" { Some(0) } else { None },
            "{mode}"
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn holdless_request_with_a_held_sibling_stays_in_money_lane() {
    let f = Fixture::new().await;
    f.open(true).await;
    let mut sibling = raw(&f).await;
    sibling.insert("_id", "sibling");
    sibling.insert("transaction_id", "sibling");
    sibling.insert("reserved_credits", Credits::ZERO);
    sibling.insert("funding", Bson::Null);
    f.db.collection::<Document>(METERS)
        .insert_one(sibling)
        .await
        .unwrap();
    dead(&f).await;
    run(&f.db).await.unwrap();
    let state =
        f.db.collection::<Document>(STATE)
            .find_one(doc! {"_id":STATE_ID})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(state.get_i64("ended_requests").unwrap(), 1);
    assert_eq!(state.get_i64("holdless_ended_requests").unwrap_or(0), 0);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_final_checkpoint_and_intent_use_existing_recovery() {
    for evidence in ["checkpoint", "intent", "quantity"] {
        let f = Fixture::new().await;
        let metered = if evidence == "checkpoint" {
            f.open(true).await
        } else {
            open(&f, false).await
        };
        dead(&f).await;
        match evidence {
            "checkpoint" => {
                let mut recorder = f.service.stream_checkpoints(&metered, true);
                recorder.observe_value(None, &serde_json::json!({"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}})).await;
                f.db.collection::<Document>(METERS).update_many(doc! {}, doc! {"$set":{"usage_checkpoint.recover_after":bson::DateTime::from_millis(0)}}).await.unwrap();
            }
            "intent" => {
                f.db.collection::<Document>(METERS).update_many(doc! {}, doc! {"$set":{"pending_platform_usage":bson::to_bson(&crate::models::service_billing::PlatformUsage::llm_completion(0, 3)).unwrap()}}).await.unwrap();
            }
            _ => {
                f.db.collection::<Document>(METERS).update_many(doc! {}, doc! {"$set":{"quantity":3_i64,"status":"finalized","finalized_at":bson::DateTime::from_millis(0)}}).await.unwrap();
            }
        }
        run(&f.db).await.unwrap();
        assert!(!raw(&f).await.contains_key("stuck_request_exit"));
        crate::services::billing::reservation::recover_retryable_settlements_at(
            &f.db,
            Utc::now() + chrono::Duration::hours(2),
        )
        .await
        .unwrap();
        assert_eq!(f.row().await.quantity, Some(3), "{evidence}");
        assert!(f.row().await.released, "{evidence}");
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn holdless_candidate_cap_and_durable_cursor_make_progress_without_timing_a_read() {
    let f = Fixture::new().await;
    open(&f, false).await;
    let base = raw(&f).await;
    f.db.collection::<Document>(METERS)
        .delete_many(doc! {})
        .await
        .unwrap();
    for n in 0..3 {
        let mut row = base.clone();
        row.insert("_id", format!("{n}"));
        row.insert("transaction_id", format!("{n}"));
        row.insert("billing_request_id", format!("{n}"));
        f.db.collection::<Document>(METERS)
            .insert_one(row)
            .await
            .unwrap();
    }
    dead(&f).await;
    for n in 1..=3 {
        crate::services::billing::recovery_batch::TEST_BATCH_LIMIT
            .scope(1, run(&f.db))
            .await
            .unwrap();
        let rows =
            f.db.collection::<Document>(METERS)
                .count_documents(doc! {"quantity":0_i64})
                .await
                .unwrap();
        assert_eq!(rows, n);
        let state =
            f.db.collection::<Document>(STATE)
                .find_one(doc! {"_id":STATE_ID})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            state.get_str("holdless_cursor").unwrap(),
            format!("{}", n - 1)
        );
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_returned_benefits_are_not_moved_again() {
    let f = Fixture::new().await;
    open(&f, true).await;
    let row = f.row().await;
    let funding = doc! {
        "settled": false,
        "grant_reservations":[{"grant_id":"grant", "amount":Credits::from_whole(5)}],
        "allowance_reservations":[{"allowance_id":"allowance", "period_id":"period", "quantity":2_i64}],
        "grant_consumptions":[{"operation_id":format!("{}:grant-release:grant",row.id),"grant_id":"grant","amount":Credits::ZERO}],
        "allowance_consumptions":[{"operation_id":format!("{}:allowance-release:period",row.id),"allowance_id":"allowance","period_id":"period","quantity":0_i64}],
    };
    f.db.collection::<Document>(METERS)
        .update_many(doc! {}, doc! {"$set":{"funding":funding}})
        .await
        .unwrap();
    f.db.collection::<Document>("credit_grants").insert_one(doc! {"_id":"grant","recipient_user_id":&f.owner,"remaining":Credits::from_whole(5),"reserved":Credits::ZERO}).await.unwrap();
    f.db.collection::<Document>("usage_allowance_periods").insert_one(doc! {"_id":"period","owner_user_id":&f.owner,"reserved_quantity":0_i64,"consumed_quantity":0_i64}).await.unwrap();
    dead(&f).await;
    let before = money(&f).await;
    f.writes.lock().unwrap().clear();
    run(&f.db).await.unwrap();
    assert_eq!(f.row().await.quantity, Some(0));
    meter::settle_persisted(&f.db, vec![f.row().await])
        .await
        .unwrap();
    assert!(money(&f).await == before, "resource BSON changed");
    only_metadata_writes(&f);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_malformed_unit_is_reported_and_healthy_request_still_finishes() {
    let f = Fixture::new().await;
    open(&f, false).await;
    let mut poison = raw(&f).await;
    poison.insert("_id", "0");
    poison.insert("transaction_id", "0");
    poison.insert("billing_request_id", "poison");
    poison.insert("layer", "unreadable");
    f.db.collection::<Document>(METERS)
        .insert_one(poison.clone())
        .await
        .unwrap();
    poison.insert("_id", -1_i32);
    poison.insert("transaction_id", "malformed-id");
    poison.insert("billing_request_id", "malformed-id");
    f.db.collection::<Document>(METERS)
        .insert_one(poison)
        .await
        .unwrap();
    dead(&f).await;
    let before =
        f.db.collection::<Document>(METERS)
            .find_one(doc! {"_id":"0"})
            .await
            .unwrap()
            .unwrap();
    let error = run(&f.db)
        .await
        .expect_err("malformed request is a failed unit");
    assert!(
        matches!(
            error,
            AppError::BillingRecoveryFailed {
                failed_units: 2,
                ..
            }
        ),
        "{error}"
    );
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .find_one(doc! {"_id":"0"})
            .await
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {"quantity":0_i64})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_summary_failure_retains_durable_counts_for_one_retry() {
    let f = Fixture::new().await;
    open(&f, false).await;
    dead(&f).await;
    f.db.run_command(doc! {"create":"audit_log","validator":{"event_type":{"$ne":"billing_holdless_requests_ended"}}}).await.unwrap();
    assert!(run(&f.db).await.is_err());
    assert_eq!(f.row().await.quantity, Some(0));
    f.db.run_command(doc! {"collMod":"audit_log","validator":{}})
        .await
        .unwrap();
    run(&f.db).await.unwrap();
    run(&f.db).await.unwrap();
    assert_eq!(
        f.db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"billing_holdless_requests_ended"})
            .await
            .unwrap(),
        1
    );
    let state =
        f.db.collection::<Document>(STATE)
            .find_one(doc! {"_id":STATE_ID})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(state.get_i64("holdless_ended_requests").unwrap(), 1);
    assert!(!state.contains_key("holdless_summary"));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_budget_cancels_uncommitted_transaction_with_virtual_time() {
    use recovery_tests::{GATE, Gate, Phase};
    let f = Fixture::new().await;
    open(&f, false).await;
    dead(&f).await;
    let before = raw(&f).await;
    let gate = Gate::new(Phase::AfterRows);
    let mut running = Box::pin(GATE.scope(gate.clone(), super::recover(&f.db)));
    tokio::select! {_=&mut running=>panic!("missed barrier"),()=gate.entered.notified()=>{}}
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    running.await.unwrap();
    tokio::time::resume();
    assert_eq!(raw(&f).await, before);
    run(&f.db).await.unwrap();
    assert_eq!(f.row().await.quantity, Some(0));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_drain_finishes_current_request_and_starts_no_next_unit() {
    use recovery_tests::{GATE, Gate, Phase};
    let f = Fixture::new().await;
    open(&f, false).await;
    let mut second = raw(&f).await;
    second.insert("_id", "second");
    second.insert("transaction_id", "second");
    second.insert("billing_request_id", "second");
    f.db.collection::<Document>(METERS)
        .insert_one(second)
        .await
        .unwrap();
    dead(&f).await;
    crate::services::billing::reconcile::with_drain_for_test(async {
        let gate = Gate::new(Phase::AfterRows);
        let mut running = Box::pin(GATE.scope(gate.clone(), run(&f.db)));
        tokio::select! {_=&mut running=>panic!("missed barrier"),()=gate.entered.notified()=>{}}
        crate::services::billing::reconcile::drain_for_test();
        gate.resume.notify_one();
        running.await.unwrap();
    })
    .await;
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {"quantity":0_i64})
            .await
            .unwrap(),
        1
    );
    run(&f.db).await.unwrap();
    run(&f.db).await.unwrap();
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {"quantity":0_i64})
            .await
            .unwrap(),
        2
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_selection_seeks_existing_index_including_later_pages() {
    let f = Fixture::new().await;
    f.db.collection::<Document>(METERS).insert_many((0..201).map(|n|doc! {"_id":format!("row-{n:03}"),"transaction_id":format!("row-{n:03}"),"billing_request_id":"probe","forwarded":true,"released":false})).await.unwrap();
    for after in ["", "row-099"] {
        let plan=f.db.run_command(doc! {"explain": {
            "find":METERS,"filter":holdless::page_filter(after),"projection":{"_id":1,"billing_request_id":1},
            "sort":{"_id":1},"limit":100_i64,"hint":inventory::INDEX,
        }, "verbosity":"executionStats"}).await.unwrap();
        let json = serde_json::to_string(&plan).unwrap();
        assert!(json.contains(inventory::INDEX), "{json}");
        assert!(!json.contains("COLLSCAN"), "{json}");
        assert!(!json.contains("\"stage\":\"SORT\""), "{json}");
        assert!(
            plan.get_document("executionStats")
                .unwrap()
                .get_i32("totalKeysExamined")
                .unwrap()
                <= 100
        );
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_summary_crashes_cannot_duplicate_or_lose_the_chained_record() {
    use recovery_tests::{Phase, at};
    for phase in [Phase::AfterAudit, Phase::AfterHoldlessSummary] {
        let f = Fixture::new().await;
        open(&f, false).await;
        dead(&f).await;
        at(phase, run(&f.db)).await;
        assert_eq!(f.row().await.quantity, Some(0));
        assert_eq!(
            f.db.collection::<Document>("audit_log")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        for _ in 0..2 {
            run(&f.db).await.unwrap();
        }
        assert_eq!(
            f.db.collection::<Document>("audit_log")
                .count_documents(doc! {"event_type":"billing_holdless_requests_ended"})
                .await
                .unwrap(),
            1
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn holdless_oversized_request_is_counted_without_partial_publication() {
    let f = Fixture::new().await;
    open(&f, false).await;
    let base = raw(&f).await;
    for n in 0..128 {
        let mut sibling = base.clone();
        sibling.insert("_id", format!("sibling-{n:03}"));
        sibling.insert("transaction_id", format!("sibling-{n:03}"));
        f.db.collection::<Document>(METERS)
            .insert_one(sibling)
            .await
            .unwrap();
    }
    dead(&f).await;
    let error = run(&f.db).await.unwrap_err();
    assert!(matches!(
        error,
        AppError::BillingRecoveryFailed {
            failed_units: 1,
            ..
        }
    ));
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {"quantity":{"$ne":Bson::Null}})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.db.collection::<Document>("audit_log")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_row_write_rejection_aborts_all_siblings_and_summary_counts() {
    let f = Fixture::new().await;
    open(&f, false).await;
    let mut sibling = raw(&f).await;
    sibling.insert("_id", "sibling");
    sibling.insert("transaction_id", "sibling");
    f.db.collection::<Document>(METERS)
        .insert_one(sibling)
        .await
        .unwrap();
    dead(&f).await;
    f.db.run_command(
        doc! {"collMod":METERS,"validator":{"$nor":[{"_id":"sibling","status":"finalized"}]}},
    )
    .await
    .unwrap();
    assert!(run(&f.db).await.is_err());
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {"quantity":0_i64})
            .await
            .unwrap(),
        0
    );
    let state =
        f.db.collection::<Document>(STATE)
            .find_one(doc! {"_id":STATE_ID})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(state.get_i64("holdless_ended_requests").unwrap_or(0), 0);
    assert!(!state.contains_key("holdless_summary"));
    f.db.run_command(doc! {"collMod":METERS,"validator":{}})
        .await
        .unwrap();
    run(&f.db).await.unwrap();
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {"quantity":0_i64})
            .await
            .unwrap(),
        2
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn holdless_active_claim_or_resource_lock_remains_with_normal_recovery() {
    for mode in ["claim", "wallet_lock", "consumption"] {
        let f = Fixture::new().await;
        open(&f, true).await;
        dead(&f).await;
        let row = f.row().await;
        match mode {
            "claim" => {
                f.db.collection::<Document>(METERS)
                    .update_many(
                        doc! {},
                        doc! {"$set":{"funding.settlement_claim_id":"claim"}},
                    )
                    .await
                    .unwrap();
            }
            "wallet_lock" => {
                f.db.collection::<Document>("billing_wallet").update_many(doc! {},doc! {"$set":{"active_settlement":{"row_id":&row.id,"actual_credits":Credits::from_whole(1),"reserved_credits":Credits::ZERO,"applied":true}}}).await.unwrap();
            }
            _ => {
                f.db.collection::<Document>(METERS)
                    .update_many(
                        doc! {},
                        doc! {"$set":{"funding.wallet_funded":Credits::from_whole(1)}},
                    )
                    .await
                    .unwrap();
            }
        }
        let before = raw(&f).await;
        let resources = money(&f).await;
        f.writes.lock().unwrap().clear();
        run(&f.db).await.unwrap();
        assert_eq!(raw(&f).await, before, "{mode}");
        assert!(money(&f).await == resources, "{mode}");
        only_metadata_writes(&f);
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn holdless_finalized_zero_resumes_ordinary_release_after_crash_and_retry_failure() {
    for failed in [false, true] {
        let f = Fixture::new().await;
        open(&f, true).await;
        dead(&f).await;
        recovery_tests::at(recovery_tests::Phase::AfterCommit, run(&f.db)).await;
        assert!(!f.row().await.released);
        if failed {
            crate::services::billing::reservation::record_settlement_failure(
                &f.db,
                &f.row().await,
                Utc::now(),
            )
            .await
            .unwrap();
        }
        let before = money(&f).await;
        f.writes.lock().unwrap().clear();
        crate::services::billing::reservation::recover_retryable_settlements_at(
            &f.db,
            Utc::now() + chrono::Duration::hours(2),
        )
        .await
        .unwrap();
        assert!(f.row().await.released);
        assert_eq!(f.row().await.quantity, Some(0));
        assert!(money(&f).await == before);
        only_metadata_writes(&f);
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn holdless_guest_zero_exposure_finishes_but_unreturned_exposure_waits() {
    for held in [Credits::ZERO, Credits::from_whole(1)] {
        let f = Fixture::new().await;
        open(&f, true).await;
        dead(&f).await;
        let row = f.row().await;
        f.db.collection::<Document>(METERS).update_many(doc! {},doc! {"$set":{"org_channel_guest":{"channel_id":"fixture","sender_hash":"fixture"}}}).await.unwrap();
        f.db.collection::<Document>(crate::models::org_channel::HOLDS)
            .insert_one(doc! {"_id":&row.id,"state":"reserved","reserved_credits":held})
            .await
            .unwrap();
        let before = money(&f).await;
        f.writes.lock().unwrap().clear();
        run(&f.db).await.unwrap();
        assert_eq!(
            f.row().await.quantity,
            if held == Credits::ZERO { Some(0) } else { None }
        );
        assert!(money(&f).await == before);
        only_metadata_writes(&f);
        f.db.drop().await.unwrap();
    }
}

#[test]
fn holdless_acknowledged_meter_retention_stays_thirty_days() {
    let source = include_str!("../reconcile.rs");
    assert!(source.contains("const ACKED_RETENTION_DAYS: i64 = 30;"));
}

#[tokio::test]
async fn holdless_ordinary_metering_still_has_no_retention_date() {
    let f = Fixture::new().await;
    let metered = f.open(false).await;
    f.service
        .settle(
            &metered,
            crate::models::service_billing::PlatformUsage::llm_completion(0, 3),
            None,
            None,
        )
        .await
        .unwrap();
    f.db.collection::<Document>(METERS)
        .update_many(
            doc! {},
            doc! {"$set":{"updated_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let before = raw(&f).await;
    let lago = Arc::new(crate::billing_integration_tests::FakeLago::default());
    let reconciler = BillingReconciler::new(
        f.db.clone(),
        Some(lago.clone()),
        Arc::new(crate::test_utils::test_app_config()),
    );
    reconciler.push_usage_for_test().await.unwrap();
    assert_eq!(before, raw(&f).await);
    let row = f.row().await;
    assert_eq!(row.quantity, Some(3));
    assert!(row.wallet_id.is_none() && row.released);
    assert!(!row.lago_acked && row.expires_at.is_none());
    assert_eq!(
        lago.recorded_events
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    f.db.drop().await.unwrap();
}
