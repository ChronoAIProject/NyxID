use super::*;
use futures::TryStreamExt;

pub(super) async fn clean_holds(f: &Fixture) {
    f.clean().await;
    assert_eq!(
        f.db.collection::<bson::Document>("billing_hold_observations")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(f.db.collection::<bson::Document>("audit_log").count_documents(doc!{"event_type":{"$in":["billing_wallet_hold_excess_repaired","billing_grant_hold_excess_repaired","billing_allowance_hold_excess_repaired"]}}).await.unwrap(),0,
        "normal recovery returns holds; conservation repair must not hide a leak");
}

#[tokio::test]
async fn realtime_ws_crash_at_each_money_handoff_recovers_exactly_once() {
    use crate::billing_integration_tests::hold_release::{Phase as MoneyPhase, crash_money_at};
    for phase in [
        MoneyPhase::WalletLocked,
        MoneyPhase::WalletApplied,
        MoneyPhase::WalletReleased,
    ] {
        let f = Fixture::new().await;
        let mut session = start(&f, BillingMetric::Tokens).await;
        let mut collector = RealtimeLlmUsageCollector::new(true);
        let mut bytes = 0;
        observe(&mut session, &mut collector, &done("one", 3), &mut bytes).await;
        expire(&f).await;
        crash_money_at(phase, recover(&f)).await;
        clean_holds(&f).await;
        recover(&f).await;
        recover(&f).await;
        f.charged(3).await;
        clean_holds(&f).await;
        f.db.drop().await.unwrap();
    }
}

async fn mixed(f: &Fixture) -> crate::services::billing::MeteredProxyContext {
    use crate::models::billing_target::BillingTargetKind;
    use crate::services::billing::grants::{IssueCreditGrantInput, issue_grants};
    let mut route = route_context(
        &CoverageCase {
            ingress: BillingIngress::Proxy,
            scenario: "realtime_mixed",
            node_intent: NodeIntent::Direct,
            metric: BillingMetric::Tokens,
        },
        &Uuid::new_v4().to_string(),
        &f.owner,
    )
    .with_native_text_bound(2, 1, 32);
    route.platform_billable = true;
    route.service_platform_billable = true;
    let now = bson::DateTime::now();
    f.db.collection::<bson::Document>("usage_allowances").insert_one(doc!{
        "_id":"included","service_id":route.catalog_service_id.as_deref().unwrap(),
        "service_slug":route.service_slug.as_deref().unwrap(),"metric":"tokens","quantity":1_i64,
        "recurrence":"one_time","target_kind":"all_users","is_active":true,"created_by":"admin",
        "created_at":now,"updated_at":now,
    }).await.unwrap();
    issue_grants(
        &f.db,
        IssueCreditGrantInput {
            amount_credits: 1,
            target_kind: BillingTargetKind::SelectedUsers,
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
    f.service.open(&route).await.unwrap()
}

#[tokio::test]
async fn realtime_ws_benefit_handoffs_return_all_holds_and_keep_accounts_clean() {
    use crate::billing_integration_tests::hold_release::{Phase as MoneyPhase, crash_money_at};
    for phase in [
        MoneyPhase::AllowanceLocked,
        MoneyPhase::AllowanceApplied,
        MoneyPhase::AllowanceRecorded,
        MoneyPhase::GrantLocked,
        MoneyPhase::GrantApplied,
        MoneyPhase::GrantRecorded,
    ] {
        let f = Fixture::new().await;
        let metered = mixed(&f).await;
        let mut session = f
            .service
            .mark_realtime_forwarded(&metered, true, true)
            .await
            .unwrap()
            .unwrap();
        let mut collector = RealtimeLlmUsageCollector::new(true);
        let mut bytes = 0;
        observe(&mut session, &mut collector, &done("one", 3), &mut bytes).await;
        expire(&f).await;
        crash_money_at(phase, recover(&f)).await;
        clean_holds(&f).await;
        let interrupted = f.row().await;
        assert!(
            interrupted
                .funding
                .as_ref()
                .unwrap()
                .settlement_claim_id
                .is_some()
        );
        assert!(
            reservation::claim_released_and_settle(&f.db, &interrupted)
                .await
                .is_err(),
            "a survivor must not steal the still-live funding claim at {phase:?}"
        );
        // The retry cutoff passed to reservation recovery does not advance
        // funding's independent real-clock claim lease. Expire only that
        // timestamp to model the next eligible pass without sleeping.
        f.db.collection::<UsageMeterRow>(USAGE_METER)
            .update_one(
                doc! {"_id": &interrupted.id},
                doc! {"$set": {
                    "funding.settlement_claimed_at": bson::DateTime::from_millis(0),
                }},
            )
            .await
            .unwrap();
        recover(&f).await;
        recover(&f).await;
        let row = f.row().await;
        assert_eq!(row.quantity, Some(3));
        assert!(row.released, "{phase:?}: {:?}", row.status);
        let funding = row.funding.unwrap();
        assert!(funding.settled);
        let wallet = wallet(&f.db, &f.owner).await;
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(wallet.pending_lago_debits, Credits::from_whole(1));
        let grant =
            f.db.collection::<crate::models::credit_grant::CreditGrant>("credit_grants")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(grant.reserved, Credits::ZERO);
        assert_eq!(grant.remaining, Credits::ZERO);
        let period =
            f.db.collection::<crate::models::usage_allowance_period::UsageAllowancePeriod>(
                "usage_allowance_periods",
            )
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!((period.reserved_quantity, period.consumed_quantity), (0, 1));
        clean_holds(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn realtime_ws_prefix_prices_components_resale_and_gross_tax_once() {
    use crate::models::service_billing::ResaleSpec;
    let f = Fixture::new().await;
    let mut route = route_context(
        &CoverageCase {
            ingress: BillingIngress::Proxy,
            scenario: "realtime_components",
            node_intent: NodeIntent::Direct,
            metric: BillingMetric::Requests,
        },
        &Uuid::new_v4().to_string(),
        &f.owner,
    );
    route.platform_billable = true;
    route.service_platform_billable = true;
    route.platform_components = vec![
        ResaleSpec {
            metric: BillingMetric::InputTokens,
            lago_metric_code: "rt_input".into(),
        },
        ResaleSpec {
            metric: BillingMetric::OutputTokens,
            lago_metric_code: "rt_output".into(),
        },
    ];
    route.resale = Some(ResaleSpec {
        metric: BillingMetric::Tokens,
        lago_metric_code: "rt_resale".into(),
    });
    f.db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
        .insert_many([
            rate("rt_input", Utc::now()),
            rate("rt_output", Utc::now()),
            rate("rt_resale", Utc::now()),
        ])
        .await
        .unwrap();
    let tax = doc! {"code":"test","name":"test","rate_percent":Credits::from_whole(10)};
    f.db.collection::<bson::Document>("billing_tax_profiles").update_one(doc!{},doc!{"$set":{
        "default_taxes":[tax.clone()],"metric_taxes.platform_requests":[tax.clone()],"metric_taxes.rt_input":[tax.clone()],"metric_taxes.rt_output":[tax.clone()],"metric_taxes.rt_resale":[tax],
        "fingerprint":"realtime-tax-test"
    }}).await.unwrap();
    let metered = f.service.open(&route).await.unwrap();
    let mut session = f
        .service
        .mark_realtime_forwarded(&metered, true, true)
        .await
        .unwrap()
        .unwrap();
    let mut collector = RealtimeLlmUsageCollector::new(true);
    let mut bytes = 0;
    observe(&mut session,&mut collector,r#"{"type":"response.done","response":{"id":"one","usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}}"#,&mut bytes).await;
    expire(&f).await;
    recover(&f).await;
    recover(&f).await;
    let rows: Vec<UsageMeterRow> =
        f.db.collection::<UsageMeterRow>(USAGE_METER)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(rows.len(), 4);
    for row in rows {
        let expected = match row.lago_metric_code.as_str() {
            "platform_requests" => 1,
            "rt_input" => 2,
            "rt_output" => 3,
            "rt_resale" => 5,
            other => panic!("{other}"),
        };
        assert_eq!(row.quantity, Some(expected));
        assert!(row.released);
        assert_eq!(
            f.db.collection::<bson::Document>("billing_ledger")
                .count_documents(doc! {"dedupe_key":format!("usage-settled:{}",row.id)})
                .await
                .unwrap(),
            1
        );
    }
    let wallet = wallet(&f.db, &f.owner).await;
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    assert_eq!(
        wallet.pending_lago_debits,
        "12.1".parse::<Credits>().unwrap()
    );
    clean_holds(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn realtime_ws_bytes_and_request_metrics_use_the_same_durable_prefix() {
    for metric in [BillingMetric::Bytes, BillingMetric::Requests] {
        let f = Fixture::new().await;
        let mut session = start(&f, metric).await;
        let mut collector = RealtimeLlmUsageCollector::new(true);
        let mut bytes = 0;
        observe(&mut session, &mut collector, &done("one", 3), &mut bytes).await;
        observe(&mut session, &mut collector, &done("two", 5), &mut bytes).await;
        expire(&f).await;
        recover(&f).await;
        recover(&f).await;
        settled(
            &f,
            if metric == BillingMetric::Bytes {
                bytes
            } else {
                1
            },
        )
        .await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn realtime_ws_byte_pricing_and_legacy_resale_estimate_keep_the_original_rule() {
    for recovery in [false, true] {
        let f = Fixture::new().await;
        let mut route = route_context(
            &CoverageCase {
                ingress: BillingIngress::Proxy,
                scenario: "realtime_byte_fallback",
                node_intent: NodeIntent::Direct,
                metric: BillingMetric::Bytes,
            },
            &Uuid::new_v4().to_string(),
            &f.owner,
        );
        route.platform_billable = true;
        route.service_platform_billable = true;
        route.service_slug = Some("custom-realtime".into());
        route.resale = Some(crate::models::service_billing::ResaleSpec {
            metric: BillingMetric::Tokens,
            lago_metric_code: "rt_fallback".into(),
        });
        f.db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
            .insert_one(rate("rt_fallback", Utc::now()))
            .await
            .unwrap();
        let metered = f.service.open(&route).await.unwrap();
        let mut session = f
            .service
            .mark_realtime_forwarded(&metered, true, false)
            .await
            .unwrap()
            .unwrap();
        let mut collector = RealtimeLlmUsageCollector::new(true);
        let mut bytes = 0;
        observe(&mut session, &mut collector, &done("one", 3), &mut bytes).await;
        let expected = crate::services::llm_usage_service::estimate_tokens_from_bytes(bytes);
        assert_ne!(expected, 3);
        if recovery {
            expire(&f).await;
            recover(&f).await;
        } else {
            let usage = crate::services::llm_usage_service::platform_usage(None, bytes, true);
            let rows = session.persist_close(usage).await.unwrap();
            settle_rows(&f.db, rows).await;
        }
        recover(&f).await;
        let rows: Vec<UsageMeterRow> =
            f.db.collection::<UsageMeterRow>(USAGE_METER)
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
        assert_eq!(rows.len(), 2);
        for row in rows {
            assert!(row.released);
            assert_eq!(
                row.quantity,
                Some(
                    if row.layer == crate::models::usage_meter::BillingLayer::Platform {
                        bytes
                    } else {
                        expected
                    }
                )
            );
        }
        let wallet = wallet(&f.db, &f.owner).await;
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(
            wallet.pending_lago_debits,
            Credits::from_whole(bytes + expected)
        );
        clean_holds(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn realtime_ws_unused_component_hold_returns_before_nonzero_component_funding() {
    use crate::models::billing_target::BillingTargetKind;
    use crate::models::service_billing::ResaleSpec;
    use crate::services::billing::grants::{IssueCreditGrantInput, issue_grants};
    for recovery in [false, true] {
        let f = Fixture::new().await;
        issue_grants(
            &f.db,
            IssueCreditGrantInput {
                amount_credits: 5,
                target_kind: BillingTargetKind::SelectedUsers,
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
        let mut route = route_context(
            &CoverageCase {
                ingress: BillingIngress::Proxy,
                scenario: "realtime_unused_component",
                node_intent: NodeIntent::Direct,
                metric: BillingMetric::Tokens,
            },
            &Uuid::new_v4().to_string(),
            &f.owner,
        );
        route.platform_billable = true;
        route.service_platform_billable = true;
        route.request_bytes = 0;
        route.requested_images = 1;
        route.platform_components = vec![ResaleSpec {
            metric: BillingMetric::Images,
            lago_metric_code: "rt_unused_image".into(),
        }];
        f.db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
            .insert_one(rate("rt_unused_image", Utc::now()))
            .await
            .unwrap();
        let metered = f.service.open(&route).await.unwrap();
        let mut session = f
            .service
            .mark_realtime_forwarded(&metered, true, true)
            .await
            .unwrap()
            .unwrap();
        let mut collector = RealtimeLlmUsageCollector::new(true);
        let mut bytes = 0;
        observe(&mut session, &mut collector, &done("one", 5), &mut bytes).await;
        if recovery {
            expire(&f).await;
            recover(&f).await;
        } else {
            close(&f, &session, collector, bytes).await;
        }
        let wallet = wallet(&f.db, &f.owner).await;
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(
            wallet.pending_lago_debits,
            Credits::ZERO,
            "unused image hold must fund the actual tokens"
        );
        let grant =
            f.db.collection::<crate::models::credit_grant::CreditGrant>("credit_grants")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(grant.reserved, Credits::ZERO);
        assert_eq!(grant.remaining, Credits::ZERO);
        clean_holds(&f).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn realtime_ws_zero_prefix_returns_allowance_grant_and_wallet_holds() {
    let f = Fixture::new().await;
    let metered = mixed(&f).await;
    let _session = f
        .service
        .mark_realtime_forwarded(&metered, true, true)
        .await
        .unwrap()
        .unwrap();
    expire(&f).await;
    recover(&f).await;
    recover(&f).await;
    let row = f.row().await;
    assert_eq!(row.quantity, Some(0));
    assert!(row.released);
    let wallet = wallet(&f.db, &f.owner).await;
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
    let grant =
        f.db.collection::<crate::models::credit_grant::CreditGrant>("credit_grants")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(grant.reserved, Credits::ZERO);
    assert_eq!(grant.remaining, Credits::from_whole(1));
    let period =
        f.db.collection::<crate::models::usage_allowance_period::UsageAllowancePeriod>(
            "usage_allowance_periods",
        )
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!((period.reserved_quantity, period.consumed_quantity), (0, 0));
    clean_holds(&f).await;
    f.db.drop().await.unwrap();
}
