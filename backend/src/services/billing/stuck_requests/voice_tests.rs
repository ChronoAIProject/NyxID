use super::*;
use crate::billing_integration_tests::usage_checkpoints::Fixture;
use crate::models::{
    service_billing::{BillingMetric, PlatformUsage, ResaleSpec},
    usage_meter::UsageMeterRow,
};
use chrono::Utc;

async fn fixture(suffix: &str, legacy: bool) -> (Fixture, String, String) {
    use crate::services::billing::grants::{IssueCreditGrantInput, issue_grants};
    let f = Fixture::new().await;
    crate::services::assistant_voice::ensure_indexes(&f.db)
        .await
        .unwrap();
    let seed = f.open(false).await;
    f.db.collection::<Document>(METERS)
        .delete_many(doc! {})
        .await
        .unwrap();
    issue_grants(
        &f.db,
        IssueCreditGrantInput {
            amount_credits: 1,
            target_kind: crate::models::billing_target::BillingTargetKind::SelectedUsers,
            target_user_ids: vec![f.owner.clone()],
            target_org_ids: vec![],
            target_group_ids: vec![],
            all_services: true,
            service_refs: vec![],
            expires_at: None,
            reason: None,
            granted_by: f.owner.clone(),
            amount: None,
            service_limits: vec![],
            request_id: None,
        },
    )
    .await
    .unwrap();
    let sid = uuid::Uuid::new_v4().to_string();
    let id = format!("voice:{sid}:{suffix}");
    let mut route = seed.route.unwrap();
    route.platform_billable = true;
    route.service_platform_billable = true;
    let metrics = [
        BillingMetric::Tokens,
        BillingMetric::InputTokens,
        BillingMetric::OutputTokens,
        BillingMetric::CacheReadTokens,
        BillingMetric::CacheWriteTokens,
    ];
    let specs: Vec<_> = metrics
        .iter()
        .enumerate()
        .map(|(i, m)| ResaleSpec {
            metric: *m,
            lago_metric_code: format!("voice-test-{i}"),
        })
        .collect();
    for spec in &specs {
        let mut rate = crate::billing_integration_tests::rate(&spec.lago_metric_code, Utc::now());
        rate.credits_per_unit_micros = 0;
        rate.credits_per_unit_pico = Some(800_000);
        f.db.collection("billing_rate_cache")
            .insert_one(rate)
            .await
            .unwrap();
    }
    route.platform_metric = specs[0].metric;
    route.platform_lago_metric_code = specs[0].lago_metric_code.clone();
    route.platform_components = specs[1..].to_vec();
    let route = crate::services::voice::token_billing_context_for_test(route, id.clone());
    let m = f.service.open(&route).await.unwrap();
    f.service.mark_forwarded(&m).await.unwrap();
    f.db.collection::<Document>(METERS).update_many(doc!{},doc!{"$set":{"created_at":bson::DateTime::from_millis(0),"forwarded_owner":{"generation_id":"gone","ingress":"llm_provider","marked_at":bson::DateTime::from_millis(0)}}}).await.unwrap();
    if legacy {
        f.db.collection::<Document>(METERS)
            .update_many(doc! {}, doc! {"$unset":{"forwarded_owner":""}})
            .await
            .unwrap();
    }
    f.db.collection::<Document>("security_replicas").insert_one(doc!{"_id":"survivor","generation_id":"survivor","started_at":bson::DateTime::now(),"observed_at":bson::DateTime::now()}).await.unwrap();
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        5
    );
    (f, sid, id)
}
async fn recover_all(f: &Fixture) {
    super::super::voice::reconcile(&f.db).await.unwrap();
    for _ in 0..4 {
        recover(&f.db).await.unwrap();
    }
    super::super::reservation::recover_retryable_settlements_at(
        &f.db,
        Utc::now() + chrono::Duration::hours(2),
    )
    .await
    .unwrap();
}
async fn check(f: &Fixture, id: &str, quantities: [i64; 5]) {
    let rows: Vec<UsageMeterRow> =
        f.db.collection(METERS)
            .find(doc! {"billing_request_id":id})
            .sort(doc! {"lago_metric_code":1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(rows.len(), 5);
    for (row, q) in rows.iter().zip(quantities) {
        assert!(row.released, "{} {:?}", row.metric.as_str(), row.quantity);
        assert_eq!(row.quantity, Some(q));
    }
    let grant =
        f.db.collection::<crate::models::credit_grant::CreditGrant>("credit_grants")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(grant.reserved, Credits::ZERO);
    f.clean().await;
    assert_eq!(
        f.db.collection::<Document>("billing_hold_observations")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn voice_stuck_five_token_rows_without_window_recover_once() {
    for suffix in ["response:1", "responses", "0"] {
        for legacy in [false, true] {
            let (f, _, id) = fixture(suffix, legacy).await;
            // A successful bounded sweep can stop before it visits this request.
            // Run the worker until its five durable rows have reached settlement.
            let meters = f.db.collection::<Document>(METERS);
            loop {
                recover_all(&f).await;
                if meters
                    .count_documents(doc! {"billing_request_id": &id, "released": true})
                    .await
                    .unwrap()
                    == 5
                {
                    break;
                }
            }
            check(&f, &id, [0; 5]).await;
            recover_all(&f).await;
            assert_eq!(
                f.db.collection::<Document>("audit_log")
                    .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
                    .await
                    .unwrap(),
                1
            );
            f.db.drop().await.unwrap();
        }
    }
}
#[tokio::test]
async fn voice_stuck_native_checkpoint_survives_closed_or_unselected_session() {
    let (f, sid, id) = fixture("responses", true).await;
    f.db.collection::<Document>("assistant_voice_sessions").insert_one(doc!{
        "_id":sid,"user_id":&f.owner,"generation":1_i64,"lease_owner":"gone","live_slot":false,
        "responses_model":"task-model","responses_reserved":true,"responses_finalized":true,
        "responses_usage":{"response-one":bson::to_bson(&PlatformUsage{tokens:10,input_tokens:7,output_tokens:3,..Default::default()}).unwrap()},
        "lease_until":bson::DateTime::from_millis(0),"observed_seconds":0_i64,"billing_finalized":true,"final_usage_confirmed":true,
        "reconcile_deadline":bson::DateTime::from_millis(0),"provider_session_id":Bson::Null,
    }).await.unwrap();
    recover_all(&f).await;
    check(&f, &id, [10, 7, 3, 0, 0]).await;
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn voice_stuck_diagnostic_distinguishes_dead_and_live_voice() {
    let (f, _, _) = fixture("response:1", false).await;
    let d = diagnostics(&f.db).await.unwrap();
    assert!(
        d[0].technical_details
            .as_ref()
            .unwrap()
            .contains("class=eligible_voice eligible=true"),
        "{}",
        d[0].technical_details.as_ref().unwrap()
    );
    f.db.collection::<Document>(METERS)
        .update_many(
            doc! {},
            doc! {"$set":{"forwarded_owner.generation_id":"survivor"}},
        )
        .await
        .unwrap();
    let d = diagnostics(&f.db).await.unwrap();
    assert!(
        d[0].technical_details
            .as_ref()
            .unwrap()
            .contains("class=voice_live_session eligible=false"),
        "{}",
        d[0].technical_details.as_ref().unwrap()
    );
    recover_all(&f).await;
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {"released":false})
            .await
            .unwrap(),
        5
    );
    f.db.drop().await.unwrap();
}

async fn session(f: &Fixture, sid: &str, reported: bool) {
    let usage = if reported {
        doc! {"report":bson::to_bson(&PlatformUsage{tokens:10,input_tokens:7,output_tokens:3,..Default::default()}).unwrap()}
    } else {
        doc! {}
    };
    f.db.collection::<Document>("assistant_voice_sessions").insert_one(doc!{
        "_id":sid,"user_id":&f.owner,"generation":1_i64,"lease_owner":"gone","live_slot":false,
        "lease_until":bson::DateTime::from_millis(0),
        "responses_model":"task-model","responses_reserved":true,"responses_finalized":false,
        "responses_usage":usage,"observed_seconds":12_i64,"billing_finalized":false,"final_usage_confirmed":true,
        "reconcile_deadline":bson::DateTime::from_millis(0),"provider_session_id":Bson::Null,
    }).await.unwrap();
}
#[tokio::test]
async fn voice_stuck_live_takeover_registry_failure_and_evidence_wait_do_not_end() {
    for kind in [
        "live_takeover",
        "missing_start",
        "missing_registry",
        "evidence_wait",
        "bad_usage",
        "unknown_shape",
    ] {
        let (f, sid, id) = fixture("responses", true).await;
        session(&f, &sid, true).await;
        match kind {
            "live_takeover" => {
                f.db.collection::<Document>("assistant_voice_sessions")
                    .update_one(
                        doc! {},
                        doc! {"$set":{"lease_owner":"survivor","live_slot":true}},
                    )
                    .await
                    .unwrap();
            }
            "missing_start" => {
                f.db.collection::<Document>("security_replicas")
                    .update_many(doc! {}, doc! {"$unset":{"started_at":""}})
                    .await
                    .unwrap();
            }
            "missing_registry" => {
                f.db.collection::<Document>("security_replicas")
                    .delete_many(doc! {})
                    .await
                    .unwrap();
            }
            "evidence_wait" => {
                f.db.collection::<Document>("assistant_voice_sessions").update_one(doc!{},doc!{"$set":{"provider_session_id":"provider","final_usage_confirmed":false,"reconcile_deadline":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1))}}).await.unwrap();
            }
            "bad_usage" => {
                f.db.collection::<Document>("assistant_voice_sessions")
                    .update_one(doc! {}, doc! {"$set":{"responses_usage":"bad"}})
                    .await
                    .unwrap();
            }
            _ => {
                f.db.collection::<Document>(METERS)
                    .update_many(
                        doc! {},
                        doc! {"$set":{"billing_request_id":"voice:unsupported"}},
                    )
                    .await
                    .unwrap();
            }
        }
        recover_all(&f).await;
        assert_eq!(
            f.db.collection::<Document>(METERS)
                .count_documents(doc! {"released":false,"quantity":Bson::Null})
                .await
                .unwrap(),
            5,
            "{kind}"
        );
        let d = diagnostics(&f.db).await.unwrap();
        if kind == "live_takeover" {
            assert!(
                d[0].technical_details
                    .as_ref()
                    .unwrap()
                    .contains("class=voice_live_session eligible=false"),
                "{}",
                d[0].technical_details.as_ref().unwrap()
            );
        }
        if kind == "evidence_wait" {
            assert!(
                d[0].technical_details
                    .as_ref()
                    .unwrap()
                    .contains("class=voice_evidence_pending eligible=false"),
                "{}",
                d[0].technical_details.as_ref().unwrap()
            );
        }
        assert!(
            f.db.collection::<Document>("audit_log")
                .find_one(doc! {"event_type":"billing_unknown_request_ended"})
                .await
                .unwrap()
                .is_none(),
            "{id}"
        );
        f.clean().await;
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn voice_stuck_native_evidence_and_terminal_crash_matrix() {
    for phase in [
        Phase::BeforeTerminal,
        Phase::AfterRows,
        Phase::AfterAudit,
        Phase::BeforeCommit,
        Phase::AfterCommit,
    ] {
        let (f, sid, id) = fixture("responses", false).await;
        session(&f, &sid, true).await;
        super::recovery_tests::at(phase, terminal::publish(&f.db, &id)).await;
        f.clean().await;
        recover_all(&f).await;
        check(&f, &id, [10, 7, 3, 0, 0]).await;
        // An old voice finalizer already holding its own stale report cannot replace the decision.
        super::super::meter::settle_voice_checkpoint(
            &f.db,
            &id,
            PlatformUsage {
                tokens: 90,
                input_tokens: 60,
                output_tokens: 30,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
        check(&f, &id, [10, 7, 3, 0, 0]).await;
        assert_eq!(
            f.db.collection::<Document>("audit_log")
                .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
                .await
                .unwrap(),
            1
        );
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn voice_stuck_finalizer_and_checkpoint_write_win_transaction_race() {
    for checkpoint in [true, false] {
        let (f, sid, id) = fixture("responses", false).await;
        session(&f, &sid, false).await;
        let gate = super::recovery_tests::Gate::new(Phase::BeforeTerminal);
        let mut recovery = Box::pin(
            super::recovery_tests::GATE.scope(gate.clone(), terminal::publish(&f.db, &id)),
        );
        tokio::select! {_=&mut recovery=>panic!("missed voice barrier"),()=gate.entered.notified()=>{}}
        let usage = PlatformUsage {
            tokens: 10,
            input_tokens: 7,
            output_tokens: 3,
            ..Default::default()
        };
        if checkpoint {
            f.db.collection::<Document>("assistant_voice_sessions")
                .update_one(
                    doc! {"_id":&sid,"generation":1_i64},
                    doc! {"$set":{"responses_usage":{"report":bson::to_bson(&usage).unwrap()}}},
                )
                .await
                .unwrap();
        } else {
            super::super::meter::settle_voice_checkpoint(&f.db, &id, usage, None)
                .await
                .unwrap();
        }
        gate.resume.notify_one();
        let rows = recovery.await.unwrap();
        super::super::meter::settle_persisted(&f.db, rows)
            .await
            .unwrap();
        check(&f, &id, [10, 7, 3, 0, 0]).await;
        assert_eq!(
            f.db.collection::<Document>("audit_log")
                .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
                .await
                .unwrap(),
            u64::from(checkpoint)
        );
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn voice_stuck_command_counts_audit_and_expired_grant_sweep() {
    for (suffix, reported) in [("response:1", false), ("responses", true)] {
        let (f, sid, id) = fixture(suffix, false).await;
        if reported {
            session(&f, &sid, true).await;
        }
        f.db.collection::<Document>("credit_grants")
            .update_many(
                doc! {},
                doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        assert_eq!(
            super::super::grants::expire_due_grants(&f.db, Utc::now())
                .await
                .unwrap(),
            0
        );
        f.count();
        let rows = terminal::publish(&f.db, &id).await.unwrap();
        let publication = f.count();
        super::super::meter::settle_persisted(&f.db, rows)
            .await
            .unwrap();
        let settlement = f.count();
        println!(
            "VOICE_STUCK_COMMANDS suffix={suffix} rows=5 publication={publication} settlement={settlement}"
        );
        check(&f, &id, if reported { [10, 7, 3, 0, 0] } else { [0; 5] }).await;
        let audit =
            f.db.collection::<crate::models::audit_log::AuditLog>("audit_log")
                .find_one(doc! {"event_type":"billing_unknown_request_ended"})
                .await
                .unwrap()
                .unwrap();
        println!(
            "VOICE_STUCK_AUDIT {}",
            serde_json::to_string(&audit.event_data).unwrap()
        );
        assert!(audit.seq.is_some() && audit.entry_hash.is_some());
        assert_eq!(
            super::super::grants::expire_due_grants(&f.db, Utc::now())
                .await
                .unwrap(),
            1
        );
        f.clean().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn voice_stuck_duration_writer_recovers_recorded_seconds_and_initial_request() {
    use crate::services::billing::voice as windows;
    for (start, expected_request) in [(0, 1), (30, 0)] {
        let f = Fixture::new().await;
        crate::services::assistant_voice::ensure_indexes(&f.db)
            .await
            .unwrap();
        let seed = f.open(false).await;
        f.db.collection::<Document>(METERS)
            .delete_many(doc! {})
            .await
            .unwrap();
        let mut route = seed.route.unwrap();
        route.platform_billable = true;
        route.service_platform_billable = true;
        route.platform_metric = BillingMetric::VoiceSeconds;
        route.platform_lago_metric_code = "seconds".into();
        route.platform_components = vec![ResaleSpec {
            metric: BillingMetric::Requests,
            lago_metric_code: "requests".into(),
        }];
        for code in ["seconds", "requests"] {
            let mut rate = crate::billing_integration_tests::rate(code, Utc::now());
            rate.credits_per_unit_micros = 10_000;
            rate.credits_per_unit_pico = None;
            f.db.collection("billing_rate_cache")
                .insert_one(rate)
                .await
                .unwrap();
        }
        let sid = uuid::Uuid::new_v4().to_string();
        let id = windows::window_id(&sid, start).unwrap();
        let meter = windows::reserve(&f.db, &f.service, route, &sid, start)
            .await
            .unwrap();
        f.service.mark_forwarded(&meter).await.unwrap();
        windows::observe(&f.db, &id, start + 29).await.unwrap();
        windows::observe(&f.db, &id, start + 27).await.unwrap();
        f.db.collection::<Document>(METERS).update_many(doc!{},doc!{"$set":{"created_at":bson::DateTime::from_millis(0)},"$unset":{"forwarded_owner":""}}).await.unwrap();
        f.db.collection::<Document>("security_replicas").insert_one(doc!{"_id":"survivor","generation_id":"survivor","started_at":bson::DateTime::now(),"observed_at":bson::DateTime::now()}).await.unwrap();
        // Keep the evidence deadline even though the process is known dead.
        recover_all(&f).await;
        assert_eq!(
            f.db.collection::<Document>(METERS)
                .count_documents(doc! {"quantity":Bson::Null,"released":false})
                .await
                .unwrap(),
            2
        );
        f.db.collection::<Document>(crate::models::assistant_voice::WINDOWS)
            .update_many(
                doc! {},
                doc! {"$set":{"deadline":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        recover_all(&f).await;
        windows::observe(&f.db, &id, start + 30).await.unwrap();
        windows::finalize(&f.db, &id, false).await.unwrap();
        for (metric, quantity) in [("voice_seconds", 29), ("requests", expected_request)] {
            let row =
                f.db.collection::<UsageMeterRow>(METERS)
                    .find_one(doc! {"metric":metric})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(row.quantity, Some(quantity));
            assert!(row.released);
        }
        f.clean().await;
        f.db.drop().await.unwrap();
    }
}

pub(crate) mod round2;

mod round3;
