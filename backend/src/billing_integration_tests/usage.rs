use super::*;
use crate::handlers::billing::{self, BillingUsageResponse, BillingUsageRow, UsageQuery};
use crate::models::credits::Credits;
use crate::models::usage_meter::{
    AllowanceConsumptionAllocation, BillingLayer, GrantConsumptionAllocation, UsageFunding,
    UsageStatus,
};
use crate::models::usage_rollup_hourly::UsageRollupMeasures;
use crate::models::usage_rollup_state::STATE_ID;
use crate::test_utils::test_auth_user;
use axum::extract::{Query, State};
use futures::TryStreamExt;

pub(super) fn meter(owner: &str, quantity: i64) -> UsageMeterRow {
    let now = Utc::now();
    let id = Uuid::new_v4().to_string();
    UsageMeterRow {
        model_tariff: None,
        org_channel_guest: None,
        stuck_request_exit: None,
        rollup_pending: true,
        id: id.clone(),
        transaction_id: id.clone(),
        billing_request_id: id,
        layer: BillingLayer::Platform,
        flush_seq: None,
        billing_owner_id: owner.to_string(),
        wallet_id: Some("wallet".into()),
        actor_user_id: owner.to_string(),
        api_key_id: None,
        assistant_source: None,
        user_service_id: None,
        service_id: Some("service".into()),
        service_slug: Some("llm-test".into()),
        metric: BillingMetric::Tokens,
        lago_metric_code: "platform_tokens".into(),
        operation: None,
        credential_class: CredentialClass::NyxidManagedMaster,
        model: Some("test-model".into()),
        reported_model: None,
        token_breakdown: None,
        audio_tokens: None,
        image_tokens: None,
        reserved_credits: crate::models::credits::Credits::from_whole(0),
        funding: None,
        quantity: Some(quantity),
        pending_resale_quantity: None,
        pending_platform_usage: None,
        usage_checkpoint: None,
        realtime_session: None,
        pool_attempt: None,
        machine_usage: None,
        status: UsageStatus::Finalized,
        forwarded: true,
        released: false,
        lago_acked: false,
        attempt: 0,
        settlement_attempts: 0,
        settlement_next_retry_at: None,
        created_at: now,
        updated_at: now,
        finalized_at: Some(now),
        expires_at: None,
        last_error: None,
    }
}

async fn read_usage(state: &crate::AppState, actor: &str) -> BillingUsageResponse {
    billing::get_usage(
        State(state.clone()),
        test_auth_user(actor),
        Query(UsageQuery {
            owner_id: None,
            machine_id: None,
            job_id: None,
            period: None,
            bucket: None,
        }),
    )
    .await
    .expect("usage read")
    .0
}

async fn rate(db: &mongodb::Database, model: Option<&str>, micros: i64) {
    db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
        .insert_one(BillingRateCache {
            id: BillingRateCache::cache_id("platform_tokens", model),
            lago_metric_code: "platform_tokens".into(),
            model: model.map(str::to_string),
            credits_per_unit_micros: micros,
            credits_per_unit_pico: None,
            synced_at: Utc::now(),
            retired_at: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn free_usage_is_visible_with_zero_cost_and_status_filters() {
    let Some(db) = connect_test_database("billing_usage_free").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    rate(&db, None, 50).await;
    for (status, forwarded, quantity) in [
        (UsageStatus::Finalized, true, Some(7)),
        (UsageStatus::DeadLetter, true, Some(3)),
        (UsageStatus::DeadLetter, false, Some(100)),
        (UsageStatus::Forwarded, true, Some(100)),
        (UsageStatus::Finalized, true, None),
    ] {
        let mut row = meter(&owner, 1);
        row.wallet_id = None;
        row.status = status;
        row.forwarded = forwarded;
        row.quantity = quantity;
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(row)
            .await
            .unwrap();
    }
    let result = read_usage(&state, &owner).await;
    assert_eq!(result.rows.len(), 1);
    let row = &result.rows[0];
    assert!(!row.billable);
    assert_eq!(row.quantity, 10);
    assert_eq!(row.events, 2);
    assert_eq!(row.estimated_credits_micros, Some(0));
    assert_eq!(row.wallet_credits_micros, Some(0));
    assert_eq!(row.grant_credits_micros, Some(0));
    assert_eq!(row.allowance_credits_micros, Some(0));
    assert_eq!(row.allowance_quantity, 0);
    assert_eq!(result.totals.estimated_credits_micros, Some(0));
    assert_eq!(
        db.collection::<UsageMeterRow>(USAGE_METER)
            .count_documents(doc! {"lago_acked": true})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn exact_funding_costs_survive_retries_repricing_and_missing_rates() {
    use crate::models::billing_target::{BillingServiceScope, BillingTargetKind};
    use crate::models::credit_grant::{COLLECTION_NAME as GRANTS, CreditGrant, CreditGrantStatus};
    use crate::models::usage_allowance::{
        AllowanceRecurrence, COLLECTION_NAME as ALLOWANCES, UsageAllowance,
    };
    use crate::services::billing::funding::settle_usage_funding;
    let Some(db) = connect_test_database("billing_usage_exact_funding").await else {
        return;
    };
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    crate::services::billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    rate(&db, Some("test-model"), 1).await;
    // Fractional wallet cost must never be replaced by the whole-credit debit.
    for (name, allowance_units, grant_micros, wallet_micros) in [
        ("grant", 0, 2440, 0),
        ("allowance", 2440, 0, 0),
        ("mixed", 1200, 1000, 240),
        ("wallet", 0, 0, 2440),
    ] {
        let owner = insert_owner(&db).await;
        let now = Utc::now();
        if allowance_units > 0 {
            db.collection::<UsageAllowance>(ALLOWANCES)
                .insert_one(UsageAllowance {
                    bundle_id: None,
                    id: Uuid::new_v4().to_string(),
                    service_id: "service".into(),
                    service_slug: "llm-test".into(),
                    metric: BillingMetric::Tokens,
                    quantity: allowance_units,
                    recurrence: AllowanceRecurrence::OneTime,
                    target_kind: BillingTargetKind::SelectedUsers,
                    target_user_ids: vec![owner.clone()],
                    target_org_ids: Vec::new(),
                    target_group_ids: Vec::new(),
                    is_active: true,
                    created_by: owner.clone(),
                    created_at: now,
                    updated_at: now,
                })
                .await
                .unwrap();
        }
        let grant_id = Uuid::new_v4().to_string();
        if grant_micros > 0 {
            db.collection::<CreditGrant>(GRANTS)
                .insert_one(CreditGrant {
                    service_limits: Vec::new(),
                    revoke_requested_at: None,
                    revoke_requested_by: None,
                    id: grant_id.clone(),
                    batch_id: Uuid::new_v4().to_string(),
                    schedule_origin: None,
                    recipient_user_id: owner.clone(),
                    target_kind: BillingTargetKind::SelectedUsers,
                    target_org_ids: Vec::new(),
                    target_group_ids: Vec::new(),
                    amount_credits: 1,
                    amount: crate::models::credits::Credits::from_micros(grant_micros),
                    remaining: crate::models::credits::Credits::from_micros(grant_micros),
                    reserved: crate::models::credits::Credits::from_micros(0),
                    scope: BillingServiceScope {
                        all_services: true,
                        service_ids: vec![],
                        service_slugs: vec![],
                    },
                    expires_at: None,
                    reason: None,
                    granted_by: owner.clone(),
                    status: CreditGrantStatus::Active,
                    issued_ledgered_at: Some(now),
                    terminal_ledgered_at: None,
                    terminal_amount: crate::models::credits::Credits::from_micros(0),
                    active_settlement: None,
                    created_at: now,
                    updated_at: now,
                    consumed_at: None,
                    expired_at: None,
                    revoked_at: None,
                })
                .await
                .unwrap();
        }
        let mut row = meter(&owner, 2440);
        row.funding = Some(UsageFunding {
            credits_per_unit_micros: 90,
            credits_per_unit_pico: None,
            ..Default::default()
        });
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(&row)
            .await
            .unwrap();
        let settlement = settle_usage_funding(&db, &row).await.unwrap();
        assert_eq!(
            settlement.wallet_charge_credits,
            // Exact wallet shares replace the former whole-credit ceiling (issue #1672).
            crate::models::credits::Credits::from_micros(wallet_micros),
            "{name}"
        );
        assert_eq!(
            settlement.lago_billable_quantity_micros,
            wallet_micros * 1_000_000,
            "{name}"
        );
        let saved = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        let funding = saved.funding.as_ref().unwrap();
        assert_eq!(
            funding.total_charge,
            Some(crate::models::credits::Credits::from_micros(2440)),
            "{name}"
        );
        assert_eq!(funding.allowance_funded_quantity, Some(allowance_units));
        assert_eq!(
            funding.allowance_funded,
            Some(crate::models::credits::Credits::from_micros(
                allowance_units
            ))
        );
        assert_eq!(
            funding.grant_funded,
            Some(crate::models::credits::Credits::from_micros(grant_micros))
        );
        assert_eq!(
            funding.wallet_funded,
            Some(crate::models::credits::Credits::from_micros(wallet_micros))
        );
        db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
            .update_many(
                doc! {},
                doc! { "$set": { "credits_per_unit_micros": 7_i64 } },
            )
            .await
            .unwrap();
        // Both the already-settled path and the stale-caller crash-recovery path
        // return the original debit/quantity without consuming benefits twice.
        assert_eq!(settle_usage_funding(&db, &saved).await.unwrap(), settlement);
        assert_eq!(settle_usage_funding(&db, &row).await.unwrap(), settlement);
        let after_retry = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! {"_id": &row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_retry.funding, saved.funding);
        let result = read_usage(&state, &owner).await;
        assert_eq!(result.rows[0].estimated_credits_micros, Some(2440));
        assert_eq!(result.rows[0].wallet_credits_micros, Some(wallet_micros));
        assert_eq!(result.rows[0].grant_credits_micros, Some(grant_micros));
        assert_eq!(
            result.rows[0].allowance_credits_micros,
            Some(allowance_units)
        );
        assert_eq!(result.rows[0].allowance_quantity, allowance_units);
        assert_eq!(result.totals.estimated_credits_micros, Some(2440));
        db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
            .update_many(
                doc! {},
                doc! { "$set": { "credits_per_unit_micros": 1_i64 } },
            )
            .await
            .unwrap();
    }
    db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
        .delete_many(doc! {})
        .await
        .unwrap();
    let owners: Vec<User> = db
        .collection::<User>(USERS)
        .find(doc! {})
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    for owner in owners {
        let result = read_usage(&state, &owner.id).await;
        assert_eq!(result.rows[0].estimated_credits_micros, Some(2440));
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn historical_funding_uses_model_rate_and_sums_with_exact_and_free_rows() {
    use crate::models::usage_meter::{AllowanceConsumptionAllocation, GrantConsumptionAllocation};
    let Some(db) = connect_test_database("billing_usage_historical").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    rate(&db, None, 90).await;
    rate(&db, Some("test-model"), 2).await;
    let mut old = meter(&owner, 100);
    old.funding = Some(UsageFunding {
        settled: true,
        wallet_charge_credits: Some(crate::models::credits::Credits::from_whole(1)),
        allowance_consumptions: vec![AllowanceConsumptionAllocation {
            operation_id: "a".into(),
            allowance_id: "a".into(),
            period_id: "a".into(),
            quantity: 20,
        }],
        grant_consumptions: vec![GrantConsumptionAllocation {
            service_limit_key: None,
            operation_id: "g".into(),
            grant_id: "g".into(),
            amount: crate::models::credits::Credits::from_micros(50),
        }],
        ..Default::default()
    });
    let legacy = meter(&owner, 10);
    // Exact carry assignment is durable, so the recovery fixture must be
    // persisted before settlement, just as the production meter path does.
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&legacy)
        .await
        .unwrap();
    let legacy_settlement = crate::services::billing::funding::settle_usage_funding(&db, &legacy)
        .await
        .unwrap();
    assert_eq!(
        legacy_settlement.wallet_charge_credits,
        // #1672: the previous one-credit expectation encoded the ceiling bug.
        crate::models::credits::Credits::from_micros(20)
    );
    assert_eq!(legacy_settlement.lago_billable_quantity_micros, 10_000_000);
    let mut exact = meter(&owner, 30);
    exact.funding = Some(UsageFunding {
        settled: true,
        total_charge: Some(crate::models::credits::Credits::from_micros(30)),
        wallet_funded: Some(crate::models::credits::Credits::from_micros(20)),
        grant_funded: Some(crate::models::credits::Credits::from_micros(10)),
        allowance_funded: Some(crate::models::credits::Credits::from_micros(0)),
        allowance_funded_quantity: Some(0),
        ..Default::default()
    });
    let mut free = meter(&owner, 1000);
    free.wallet_id = None;
    let mut unknown = meter(&owner, 99);
    unknown.lago_metric_code = "missing".into();
    let mut mixed_exact = meter(&owner, 30);
    mixed_exact.lago_metric_code = "missing_mixed".into();
    mixed_exact.funding = exact.funding.clone();
    let mut mixed_historical = meter(&owner, 100);
    mixed_historical.lago_metric_code = mixed_exact.lago_metric_code.clone();
    mixed_historical.funding = old.funding.clone();
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_many([old, exact, free, unknown, mixed_exact, mixed_historical])
        .await
        .unwrap();
    let result = read_usage(&state, &owner).await;
    let charged = result
        .rows
        .iter()
        .find(|row| row.lago_metric_code == "platform_tokens" && row.billable)
        .unwrap();
    assert_eq!(charged.estimated_credits_micros, Some(250));
    assert_eq!(charged.wallet_credits_micros, Some(150));
    assert_eq!(charged.grant_credits_micros, Some(60));
    assert_eq!(charged.allowance_credits_micros, Some(40));
    assert_eq!(charged.allowance_quantity, 20);
    let missing = result
        .rows
        .iter()
        .find(|row| row.lago_metric_code == "missing")
        .unwrap();
    assert_eq!(missing.estimated_credits_micros, None);
    assert_eq!(missing.wallet_credits_micros, None);
    let mixed = result
        .rows
        .iter()
        .find(|row| row.lago_metric_code == "missing_mixed")
        .unwrap();
    assert_eq!(mixed.events, 2);
    assert_eq!(mixed.estimated_credits_micros, None);
    assert_eq!(mixed.wallet_credits_micros, None);
    assert_eq!(mixed.allowance_credits_micros, None);
    assert_eq!(mixed.grant_credits_micros, Some(60));
    assert_eq!(mixed.allowance_quantity, 20);
    assert_eq!(result.totals.estimated_credits_micros, Some(250));
    assert_eq!(
        result.totals.estimated_credits_micros,
        Some(
            result
                .rows
                .iter()
                .filter_map(|r| r.estimated_credits_micros)
                .sum()
        )
    );
    assert_eq!(result.totals.wallet_credits_micros, Some(150));
    assert_eq!(result.totals.grant_credits_micros, Some(120));
    assert_eq!(result.totals.allowance_credits_micros, Some(40));
    assert_eq!(result.totals.allowance_quantity, 40);
    db.drop().await.unwrap();
}

fn reservation_priced(quantity: i64, pico: Option<i64>, micros: i64, owner: &str) -> UsageMeterRow {
    let mut row = meter(owner, quantity);
    row.funding = Some(UsageFunding {
        credits_per_unit_pico: pico,
        credits_per_unit_micros: micros,
        ..Default::default()
    });
    row
}

fn grant_consumption(amount: Credits) -> crate::models::usage_meter::GrantConsumptionAllocation {
    crate::models::usage_meter::GrantConsumptionAllocation {
        service_limit_key: None,
        operation_id: Uuid::new_v4().to_string(),
        grant_id: Uuid::new_v4().to_string(),
        amount,
    }
}

/// Exact (estimated, wallet, grant, allowance) costs of a usage row, after
/// checking that each bounded micro display is the truncation of its exact value.
fn costs(row: &BillingUsageRow) -> [Option<Credits>; 4] {
    let exact = [
        row.estimated_credits,
        row.wallet_credits,
        row.grant_credits,
        row.allowance_credits,
    ];
    assert_eq!(
        [
            row.estimated_credits_micros,
            row.wallet_credits_micros,
            row.grant_credits_micros,
            row.allowance_credits_micros,
        ],
        exact.map(|value| value.map(Credits::display_micros)),
    );
    exact
}

#[tokio::test]
async fn historical_usage_is_priced_per_row_from_reservation_rates() {
    use crate::models::usage_meter::AllowanceConsumptionAllocation;
    let Some(db) = connect_test_database("billing_usage_reservation_rate").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    // No cache row: each meter is valued at its own reservation rate.
    // 2_000_000 x 0.000000250001 + 2_000_000 x 0.0000005 + 10 x 0.00009
    // = 0.500002 + 1 + 0.0009 credits.
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_many([
            reservation_priced(2_000_000, Some(250_001), 0, &owner),
            reservation_priced(2_000_000, Some(500_000), 0, &owner),
            reservation_priced(10, None, 90, &owner),
        ])
        .await
        .unwrap();

    let result = read_usage(&state, &owner).await;
    assert_eq!(result.rows.len(), 1);
    let row = &result.rows[0];
    assert_eq!(row.events, 3);
    let gross = Credits::from_micros(1_500_902);
    assert_eq!(
        costs(row),
        [
            Some(gross),
            Some(gross),
            Some(Credits::ZERO),
            Some(Credits::ZERO)
        ]
    );
    assert_eq!(row.estimated_credits_micros, Some(1_500_902));
    assert_eq!(result.totals.estimated_credits, Some(gross));
    assert_eq!(result.totals.estimated_credits_micros, Some(1_500_902));
    assert_eq!(result.totals.wallet_credits, Some(gross));
    assert_eq!(result.totals.wallet_credits_micros, Some(1_500_902));

    // One meter without any recorded rate leaves the group's cost unknown.
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(meter(&owner, 5))
        .await
        .unwrap();
    let result = read_usage(&state, &owner).await;
    assert_eq!(result.rows.len(), 1);
    let row = &result.rows[0];
    assert_eq!(row.events, 4);
    assert_eq!(costs(row), [None, None, Some(Credits::ZERO), None]);
    assert_eq!(result.totals.estimated_credits, None);
    assert_eq!(result.totals.estimated_credits_micros, None);

    // Each row is priced exactly, rate x quantity, as exact settlement does:
    // 1001 x 0.0000015 = 0.0015015 and two 1-unit rows at 0.0000015, so the
    // gross is 0.0015045 credits (1504.5 micros, displayed as 1504). Allowance
    // units cost rate x units: 400 x 0.0000015 = 0.0006. The wallet share is
    // 0.0015045 - 0.0006 - 0.0005 = 0.0004045.
    let funded_owner = insert_owner(&db).await;
    let mut funded = reservation_priced(1001, Some(1_500_000), 0, &funded_owner);
    let funding = funded.funding.as_mut().unwrap();
    funding.settled = true;
    funding.wallet_charge_credits = Some(Credits::from_whole(1));
    funding.allowance_consumptions = vec![AllowanceConsumptionAllocation {
        operation_id: "a".into(),
        allowance_id: "a".into(),
        period_id: "a".into(),
        quantity: 400,
    }];
    funding.grant_consumptions = vec![grant_consumption(Credits::from_micros(500))];
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_many([
            funded,
            reservation_priced(1, Some(1_500_000), 0, &funded_owner),
            reservation_priced(1, Some(1_500_000), 0, &funded_owner),
        ])
        .await
        .unwrap();
    let result = read_usage(&state, &funded_owner).await;
    assert_eq!(result.rows.len(), 1);
    let row = &result.rows[0];
    assert_eq!(row.events, 3);
    assert_eq!(
        costs(row),
        [
            Some(Credits::from_pico(1_504_500_000).unwrap()),
            Some(Credits::from_pico(404_500_000).unwrap()),
            Some(Credits::from_micros(500)),
            Some(Credits::from_micros(600)),
        ]
    );
    assert_eq!(row.estimated_credits_micros, Some(1504));
    assert_eq!(row.wallet_credits_micros, Some(404));
    assert_eq!(row.grant_credits_micros, Some(500));
    assert_eq!(row.allowance_credits_micros, Some(600));
    assert_eq!(row.allowance_quantity, 400);

    // A settled row's allowance units are valued at the same exact rate:
    // 2 x 0.0000015 gross, 1 x 0.0000015 allowance, the rest from the wallet.
    let allowance_owner = insert_owner(&db).await;
    let mut covered = reservation_priced(2, Some(1_500_000), 0, &allowance_owner);
    let funding = covered.funding.as_mut().unwrap();
    funding.settled = true;
    funding.wallet_charge_credits = Some(Credits::from_whole(1));
    funding.allowance_consumptions = vec![AllowanceConsumptionAllocation {
        operation_id: "b".into(),
        allowance_id: "b".into(),
        period_id: "b".into(),
        quantity: 1,
    }];
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(covered)
        .await
        .unwrap();
    let result = read_usage(&state, &allowance_owner).await;
    assert_eq!(result.rows.len(), 1);
    let row = &result.rows[0];
    assert_eq!(
        costs(row),
        [
            Some(Credits::from_pico(3_000_000).unwrap()),
            Some(Credits::from_pico(1_500_000).unwrap()),
            Some(Credits::ZERO),
            Some(Credits::from_pico(1_500_000).unwrap()),
        ]
    );
    assert_eq!(row.estimated_credits_micros, Some(3));
    assert_eq!(row.wallet_credits_micros, Some(1));
    assert_eq!(row.allowance_credits_micros, Some(1));
    assert_eq!(row.allowance_quantity, 1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn grant_settled_history_derives_gross_from_grant_consumption() {
    let Some(db) = connect_test_database("billing_usage_grant_settled").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    let grant_settled = |owner: &str, wallet_charge_credits| {
        let mut row = meter(owner, 100);
        row.funding = Some(UsageFunding {
            settled: true,
            wallet_charge_credits: Some(wallet_charge_credits),
            grant_consumptions: vec![grant_consumption(Credits::from_micros(1234))],
            ..Default::default()
        });
        row
    };
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(grant_settled(&owner, Credits::ZERO))
        .await
        .unwrap();

    let grant = Credits::from_micros(1234);
    let assert_grant_settled = |result: &BillingUsageResponse| {
        assert_eq!(result.rows.len(), 1);
        assert_eq!(
            costs(&result.rows[0]),
            [
                Some(grant),
                Some(Credits::ZERO),
                Some(grant),
                Some(Credits::ZERO)
            ]
        );
        assert_eq!(result.rows[0].estimated_credits_micros, Some(1234));
        assert_eq!(result.totals.estimated_credits, Some(grant));
        assert_eq!(result.totals.estimated_credits_micros, Some(1234));
        assert_eq!(result.totals.wallet_credits, Some(Credits::ZERO));
        assert_eq!(result.totals.grant_credits, Some(grant));
        assert_eq!(result.totals.allowance_credits, Some(Credits::ZERO));
    };
    assert_grant_settled(&read_usage(&state, &owner).await);
    // The exact derivation wins over any cached estimate (90 x 100 = 9000).
    rate(&db, None, 90).await;
    assert_grant_settled(&read_usage(&state, &owner).await);

    // A pre-cutover row stores an Int64 whole-credit wallet debit and legacy
    // `amount_micros` grant keys; the derivation reads it identically.
    let legacy_owner = insert_owner(&db).await;
    let mut legacy = bson::to_document(&meter(&legacy_owner, 100)).unwrap();
    legacy.insert(
        "funding",
        doc! {
            "settled": true,
            "wallet_charge_credits": 0_i64,
            "grant_consumptions": [
                { "operation_id": "x", "grant_id": "g", "amount_micros": 1234_i64 },
            ],
        },
    );
    db.collection::<bson::Document>(USAGE_METER)
        .insert_one(legacy)
        .await
        .unwrap();
    assert_grant_settled(&read_usage(&state, &legacy_owner).await);

    // A wallet debit means the grant did not cover the gross; without a rate
    // the cost stays unknown while the grant-funded part remains visible.
    let charged_owner = insert_owner(&db).await;
    let mut charged = grant_settled(&charged_owner, Credits::from_whole(1));
    charged.lago_metric_code = "platform_svc_removed".into();
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(charged)
        .await
        .unwrap();
    let result = read_usage(&state, &charged_owner).await;
    assert_eq!(costs(&result.rows[0]), [None, None, Some(grant), None]);
    assert_eq!(result.rows[0].grant_credits_micros, Some(1234));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn retired_rate_still_prices_historical_usage() {
    let Some(db) = connect_test_database("billing_usage_retired_rate").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
        .insert_one(BillingRateCache {
            id: BillingRateCache::cache_id("platform_tokens", None),
            lago_metric_code: "platform_tokens".into(),
            model: None,
            credits_per_unit_micros: 3,
            credits_per_unit_pico: None,
            synced_at: Utc::now(),
            retired_at: Some(Utc::now()),
        })
        .await
        .unwrap();
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(meter(&owner, 100))
        .await
        .unwrap();

    let result = read_usage(&state, &owner).await;
    let gross = Credits::from_micros(300);
    assert_eq!(
        costs(&result.rows[0]),
        [
            Some(gross),
            Some(gross),
            Some(Credits::ZERO),
            Some(Credits::ZERO)
        ]
    );
    assert_eq!(result.rows[0].estimated_credits_micros, Some(300));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn org_platform_key_request_charges_personal_wallet_and_personal_rollout() {
    Box::pin(org_credential_request(true)).await;
}

#[tokio::test]
async fn org_byok_request_keeps_org_wallet_and_org_rollout() {
    Box::pin(org_credential_request(false)).await;
}

async fn org_credential_request(platform_key: bool) {
    use crate::models::downstream_service::{PlatformKeyAudience, PlatformKeyConfig};
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
    use crate::services::feature_flag_service as flags;
    use crate::services::user_api_key_service::{CreateApiKeyParams, create_api_key};
    let Some(db) = connect_test_database("billing_org_credential_payer").await else {
        return;
    };
    create_usage_index(&db).await;
    insert_fresh_rates(&db).await;
    let actor = insert_owner(&db).await;
    // Keep payer/rollout assertions independent of the actor approval preference.
    crate::test_utils::set_test_approval_preference(&db, &actor, false).await;

    let org = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(crate::test_utils::test_membership(
            &org,
            &actor,
            OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    flags::set_platform_override(
        &db,
        flags::BILLING_FLAG_KEY,
        &flags::FlagTarget::Global,
        false,
        &actor,
    )
    .await
    .unwrap();
    if platform_key {
        flags::set_platform_override(
            &db,
            flags::BILLING_FLAG_KEY,
            &flags::FlagTarget::User(actor.clone()),
            true,
            &actor,
        )
        .await
        .unwrap();
    } else {
        flags::set_platform_org_override(&db, &org, flags::BILLING_FLAG_KEY, true, &actor)
            .await
            .unwrap();
    }
    // An explicit person override also wins when acting for an org.
    assert!(
        flags::billing_rollout_enabled(&db, &org, &actor)
            .await
            .unwrap()
    );
    // The org-wide recipient baseline has no acting person's override.
    assert_eq!(
        flags::billing_recipient_rollout_enabled(&db, &test_user(&org, UserType::Org))
            .await
            .unwrap(),
        !platform_key
    );
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    let (downstream_url, downstream) = start_billing_downstream().await;
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = "llm-org-payer".into();
    catalog.base_url = downstream_url.clone();
    catalog.auth_method = "bearer".into();
    catalog.credential_encrypted = state
        .encryption_keys
        .encrypt(b"platform-test-key")
        .await
        .unwrap();
    catalog.platform_key = Some(PlatformKeyConfig {
        purpose: Default::default(),
        enabled: true,
        audience: PlatformKeyAudience::Restricted,
        allowed_owner_ids: vec![org.clone()],
    });
    catalog.billing = Some(ServiceBilling {
        platform_billable: true,
        platform_metric: Some(BillingMetric::Tokens),
        ..Default::default()
    });
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    let connection = insert_route_service(
        &db,
        &org,
        &catalog.slug,
        &downstream_url,
        Some(&catalog.id),
        None,
    )
    .await;
    let mut update = doc! { "credential_binding": if platform_key { "platform" } else { "user" }, "auth_method": "bearer" };
    if !platform_key {
        let key = create_api_key(
            &db,
            &state.encryption_keys,
            &org,
            CreateApiKeyParams {
                label: "Org key",
                credential_type: "bearer",
                credential: "org-test-key",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();
        update.insert("api_key_id", key.id);
    }
    db.collection::<UserService>(USER_SERVICES)
        .update_one(doc! {"_id": &connection.id}, doc! {"$set": update})
        .await
        .unwrap();
    // There is no personal connection: the request must resolve this org row.
    let token = route_access_token(&state, &actor);
    let (_, private) = crate::routes::build_router();
    let app = private.with_state(state.clone());
    let request = route_request(
        Method::POST,
        &format!("/api/v1/proxy/s/{}/chat/completions", catalog.slug),
        &token,
        Body::from(r#"{"model":"test-model","messages":[]}"#),
    );
    call_mounted_route(&app, request).await;
    assert_route_settled(&db, &catalog.slug, BillingMetric::Tokens).await;
    let row = usage_row_for_service(&db, &catalog.slug).await;
    let (payer, other) = if platform_key {
        (&actor, &org)
    } else {
        (&org, &actor)
    };
    assert_eq!(&row.billing_owner_id, payer);
    assert_eq!(row.actor_user_id, actor);
    assert_eq!(
        row.credential_class,
        if platform_key {
            CredentialClass::NyxidManagedMaster
        } else {
            CredentialClass::UserOwned
        }
    );
    assert_eq!(row.quantity, Some(5));
    assert!(row.wallet_id.is_some());
    assert_eq!(
        wallet(&db, payer).await.pending_lago_debits,
        crate::models::credits::Credits::from_whole(5)
    );
    assert!(state.billing.get_wallet(other).await.unwrap().is_none());
    let personal_usage = read_usage(&state, &actor).await;
    assert_eq!(personal_usage.rows.len(), usize::from(platform_key));
    if platform_key {
        assert_eq!(
            personal_usage.totals.estimated_credits_micros,
            Some(5_000_000)
        );
    }
    downstream.abort();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn execution_owner_preserves_org_acl_for_non_master_credentials() {
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
    use crate::services::billing::owner_resolver::{BillingOwnerResolver, PaysFrom};
    let Some(db) = connect_test_database("billing_execution_owner_acl").await else {
        return;
    };
    let actor = insert_owner(&db).await;
    let outsider = insert_owner(&db).await;
    let org = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(crate::test_utils::test_membership(
            &org,
            &actor,
            OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    let resolver = BillingOwnerResolver::new(db.clone());
    let personal = resolver
        .resolve_for_ordinary_execution(&actor, &org, CredentialClass::NyxidManagedMaster)
        .await
        .unwrap();
    assert_eq!(personal.owner_id, actor);
    assert_eq!(personal.pays, PaysFrom::Personal);
    for class in [
        CredentialClass::UserOwned,
        CredentialClass::AgentOverrideUserOwned,
        CredentialClass::NodeManaged,
        CredentialClass::NyxidPlatformOauthApp,
        CredentialClass::NoAuth,
    ] {
        let resolved = resolver
            .resolve_for_ordinary_execution(&actor, &org, class)
            .await
            .unwrap();
        assert_eq!(resolved.owner_id, org);
        assert_eq!(
            resolved.pays,
            PaysFrom::OrgWallet {
                org_id: org.clone()
            }
        );
        assert!(matches!(
            resolver
                .resolve_for_ordinary_execution(&outsider, &org, class)
                .await,
            Err(AppError::Forbidden(_))
        ));
        assert_eq!(
            resolver
                .resolve_for_ordinary_execution(&actor, &actor, class)
                .await
                .unwrap(),
            personal
        );
    }
    assert!(matches!(
        resolver.resolve_for_resource(&outsider, &org).await,
        Err(AppError::Forbidden(_))
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn day_buckets_split_usage_by_utc_day_without_changing_totals() {
    let Some(db) = connect_test_database("billing_usage_day_buckets").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    let today = Utc::now();
    let yesterday = today - chrono::Duration::days(1);
    for (at, quantity) in [(yesterday, 4), (today, 5), (today, 6)] {
        let mut row = meter(&owner, quantity);
        row.wallet_id = None;
        row.created_at = at;
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(row)
            .await
            .unwrap();
    }
    let flat = read_usage(&state, &owner).await;
    assert_eq!(flat.rows.len(), 1);
    assert!(flat.rows[0].day.is_none());
    let daily = billing::get_usage(
        State(state.clone()),
        test_auth_user(&owner),
        Query(UsageQuery {
            owner_id: None,
            machine_id: None,
            job_id: None,
            period: Some("7d".into()),
            bucket: Some("day".into()),
        }),
    )
    .await
    .expect("daily usage read")
    .0;
    let days: Vec<_> = daily
        .rows
        .iter()
        .map(|row| {
            (
                row.day.expect("bucketed day").date_naive(),
                row.quantity,
                row.events,
            )
        })
        .collect();
    assert_eq!(
        days,
        vec![(yesterday.date_naive(), 4, 1), (today.date_naive(), 11, 2),]
    );
    assert_eq!(daily.totals.quantity, flat.totals.quantity);
    let invalid = billing::get_usage(
        State(state),
        test_auth_user(&owner),
        Query(UsageQuery {
            owner_id: None,
            machine_id: None,
            job_id: None,
            period: None,
            bucket: Some("hour".into()),
        }),
    )
    .await;
    assert!(matches!(invalid, Err(AppError::ValidationError(_))));
}

#[tokio::test]
async fn operation_rows_report_the_operation_and_keep_pricing_from_retired_rates() {
    let Some(db) = connect_test_database("billing_usage_operations").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    let code = "platform_svc_x_byok_op_get_me";
    let now = Utc::now();
    // Removing the operation price retired its rate; history must still price.
    for (rate_code, micros, retired_at) in [
        (code, 3_000_000, Some(now)),
        ("platform_svc_x_byok", 1_000_000, None),
    ] {
        db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
            .insert_one(BillingRateCache {
                id: BillingRateCache::cache_id(rate_code, None),
                lago_metric_code: rate_code.into(),
                model: None,
                credits_per_unit_micros: micros,
                credits_per_unit_pico: None,
                synced_at: now,
                retired_at,
            })
            .await
            .unwrap();
    }
    for (lago_metric_code, operation, quantity) in
        [(code, Some("get_me"), 2), ("platform_svc_x_byok", None, 1)]
    {
        let mut row = meter(&owner, quantity);
        row.service_slug = Some("x".into());
        row.metric = BillingMetric::Requests;
        row.lago_metric_code = lago_metric_code.into();
        row.operation = operation.map(str::to_string);
        row.model = None;
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(row)
            .await
            .unwrap();
    }
    let result = read_usage(&state, &owner).await;
    assert_eq!(result.rows.len(), 2);
    let priced = result
        .rows
        .iter()
        .find(|row| row.operation.as_deref() == Some("get_me"))
        .unwrap();
    assert_eq!(priced.lago_metric_code, code);
    assert_eq!(priced.quantity, 2);
    assert_eq!(priced.estimated_credits_micros, Some(6_000_000));
    let base = result
        .rows
        .iter()
        .find(|row| row.operation.is_none())
        .unwrap();
    assert_eq!(base.estimated_credits_micros, Some(1_000_000));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_job_usage_filters_are_applied_before_totals_and_keep_owner_scope() {
    let Some(db) = connect_test_database("billing_machine_filters").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let foreign_owner = Uuid::new_v4().to_string();
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    let machine = Uuid::new_v4().to_string();
    let other_machine = Uuid::new_v4().to_string();
    let job = Uuid::new_v4().to_string();
    let other_job = Uuid::new_v4().to_string();
    for (actor, node, job_id, quantity) in [
        (&owner, &machine, &job, 7),
        (&owner, &machine, &other_job, 11),
        (&owner, &other_machine, &job, 100),
        (&foreign_owner, &machine, &job, 1000),
    ] {
        let mut row = meter(actor, quantity);
        row.wallet_id = None;
        row.machine_usage = Some(crate::models::usage_meter::MachineUsageAttribution {
            node_id: node.clone(),
            job_id: job_id.clone(),
            runtime_id: Uuid::new_v4().to_string(),
            runtime_generation: Some(1),
            grant_id: None,
            principal_kind: "api_key".into(),
        });
        db.collection::<UsageMeterRow>(USAGE_METER)
            .insert_one(row)
            .await
            .unwrap();
    }
    for (job_filter, expected) in [(None, 18), (Some(job.clone()), 7)] {
        let result = billing::get_usage(
            State(state.clone()),
            test_auth_user(&owner),
            Query(UsageQuery {
                owner_id: None,
                period: None,
                bucket: None,
                machine_id: Some(machine.clone()),
                job_id: job_filter.clone(),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(result.machine_id.as_deref(), Some(machine.as_str()));
        assert_eq!(result.job_id, job_filter);
        assert!(result.covers_from <= Utc::now());
        assert_eq!(result.totals.quantity, expected);
        assert!(
            result
                .rows
                .iter()
                .all(|row| row.machine_usage.as_ref().unwrap().node_id == machine)
        );
    }
    assert!(
        billing::get_usage(
            State(state),
            test_auth_user(&owner),
            Query(UsageQuery {
                owner_id: None,
                period: None,
                bucket: None,
                machine_id: Some("not-a-uuid".into()),
                job_id: None,
            })
        )
        .await
        .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn long_usage_matches_raw_and_daily_rollup_and_survives_raw_expiry() {
    let Some(db) = connect_test_database("billing_usage_rollup_parity").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    let mut raw = meter(&owner, 17);
    raw.wallet_id = None;
    raw.created_at = Utc::now() - chrono::Duration::days(60);
    raw.updated_at = raw.created_at;
    raw.finalized_at = Some(raw.created_at);
    raw.token_breakdown = Some(crate::models::service_billing::TokenBreakdown {
        prompt_tokens: 11,
        completion_tokens: 6,
        cached_tokens: 2,
        cache_creation_tokens: 1,
    });
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&raw)
        .await
        .unwrap();

    db.collection::<bson::Document>(crate::models::usage_rollup_state::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": STATE_ID, "sequence": 0_i64, "batch": bson::Bson::Null,
            "daily_ready": false,
            "rolled_up_through": bson::DateTime::from_millis(0),
        })
        .await
        .unwrap();
    let read_started = Utc::now();
    let raw_answer = billing::get_usage(
        State(state.clone()),
        test_auth_user(&owner),
        Query(UsageQuery {
            owner_id: None,
            machine_id: None,
            job_id: None,
            period: Some("90d".into()),
            bucket: None,
        }),
    )
    .await
    .unwrap()
    .0;

    let all_answer = billing::get_usage(
        State(state.clone()),
        test_auth_user(&owner),
        Query(UsageQuery {
            owner_id: None,
            machine_id: None,
            job_id: None,
            period: Some("all".into()),
            bucket: None,
        }),
    )
    .await
    .unwrap()
    .0;
    for answer in [&raw_answer, &all_answer] {
        assert!(
            answer.covers_from >= read_started - chrono::Duration::days(29),
            "raw-only long history must advertise the retention boundary"
        );
        assert!(answer.covers_from <= Utc::now() - chrono::Duration::days(28));
        assert_eq!(answer.covers_from.time(), chrono::NaiveTime::MIN);
    }

    let day = raw
        .created_at
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    let measures = UsageRollupMeasures {
        quantity: 17,
        events: 1,
        requests: 0,
        exact_cost_events: 0,
        legacy_cost_events: 1,
        legacy_quantity: 17,
        legacy_allowance_quantity: 0,
        legacy_grant_cost: Credits::ZERO,
        gross_cost: Credits::ZERO,
        wallet_cost: Credits::ZERO,
        grant_cost: Credits::ZERO,
        allowance_cost: Credits::ZERO,
        prompt_tokens: 11,
        completion_tokens: 6,
        cached_tokens: 2,
        cache_creation_tokens: 1,
        audio_input_tokens: 0,
        audio_output_tokens: 0,
        image_input_tokens: 0,
        image_output_tokens: 0,
        rows_folded: 1,
    };
    let daily = crate::models::usage_rollup_daily::UsageRollupDaily {
        id: Uuid::new_v4().to_string(),
        day,
        actor: owner.clone(),
        owner: owner.clone(),
        service_id: raw.service_id.clone(),
        service_slug: raw.service_slug.clone(),
        class: "nyxid_managed_master".into(),
        metric: raw.metric.as_str().into(),
        code: raw.lago_metric_code.clone(),
        layer: "platform".into(),
        model: raw.model.clone(),
        billable: false,
        exact: false,
        measures,
        cost_partitions: std::collections::BTreeMap::new(),
        single_display_key: None,
        query_costs: None,
        last_batch: 1,
    };
    db.collection::<crate::models::usage_rollup_daily::UsageRollupDaily>(
        crate::models::usage_rollup_daily::COLLECTION_NAME,
    )
    .insert_one(daily)
    .await
    .unwrap();
    db.collection::<bson::Document>(crate::models::usage_rollup_state::COLLECTION_NAME)
        .replace_one(
            doc! { "_id": STATE_ID },
            doc! {
                "_id": STATE_ID, "sequence": 1_i64, "batch": bson::Bson::Null,
                "daily_ready": true, "folded_before": bson::DateTime::from_chrono(Utc::now()),
                "rolled_up_through": bson::DateTime::from_chrono(Utc::now()),
            },
        )
        .await
        .unwrap();
    crate::services::billing::usage_rollup::ensure_indexes(&db)
        .await
        .unwrap();
    assert_eq!(
        db.collection::<bson::Document>(crate::models::usage_rollup_daily::COLLECTION_NAME)
            .count_documents(doc! { "owner": &owner })
            .await
            .unwrap(),
        1
    );
    db.collection::<UsageMeterRow>(USAGE_METER)
        .delete_one(doc! { "_id": &raw.id })
        .await
        .unwrap();
    let rollup_result = billing::get_usage(
        State(state.clone()),
        test_auth_user(&owner),
        Query(UsageQuery {
            owner_id: None,
            machine_id: None,
            job_id: None,
            period: Some("90d".into()),
            bucket: None,
        }),
    )
    .await;
    if let Err(error) = &rollup_result {
        eprintln!("long rollup read error: {error}");
    }
    let rollup_answer = rollup_result.unwrap().0;
    assert_eq!(rollup_answer.totals.quantity, raw_answer.totals.quantity);
    assert_eq!(rollup_answer.totals.events, raw_answer.totals.events);
    assert_eq!(
        rollup_answer.totals.estimated_credits,
        raw_answer.totals.estimated_credits
    );
    assert_eq!(rollup_answer.rows.len(), 1);

    assert_eq!(
        rollup_answer.rows[0].token_breakdown,
        raw_answer.rows[0].token_breakdown
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn long_usage_rollup_parity_preserves_funding_tokens_and_unknowns() {
    let Some(db) = connect_test_database("billing_usage_rollup_full_parity").await else {
        return;
    };
    let owner = insert_owner(&db).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    rate(&db, None, 2).await;
    let old = Utc::now() - chrono::Duration::days(60);

    // The exact and unknown legacy rows deliberately share one display group.
    // The raw path keeps their known funding while preserving a null total for
    // the unknown legacy portion; the daily path must produce the same result.
    let mut exact = meter(&owner, 30);
    exact.created_at = old;
    exact.updated_at = old;
    exact.finalized_at = Some(old);
    exact.service_id = Some("service-exact".into());
    exact.service_slug = Some("exact-service".into());
    exact.lago_metric_code = "mixed-no-rate".into();
    exact.transaction_id = format!("{}:platform", exact.billing_request_id);
    exact.released = true;
    exact.lago_acked = true;
    exact.funding = Some(UsageFunding {
        settled: true,
        total_charge: Some(Credits::from_micros(500)),
        wallet_funded: Some(Credits::from_micros(300)),
        grant_funded: Some(Credits::from_micros(100)),
        allowance_funded: Some(Credits::from_micros(100)),
        allowance_funded_quantity: Some(10),
        ..Default::default()
    });
    exact.token_breakdown = Some(crate::models::service_billing::TokenBreakdown {
        prompt_tokens: 20,
        completion_tokens: 10,
        cached_tokens: 3,
        cache_creation_tokens: 2,
    });

    let mut unknown = meter(&owner, 7);
    unknown.created_at = old;
    unknown.updated_at = old;
    unknown.finalized_at = Some(old);
    unknown.service_id = exact.service_id.clone();
    unknown.service_slug = exact.service_slug.clone();
    unknown.lago_metric_code = exact.lago_metric_code.clone();
    unknown.model = exact.model.clone();
    unknown.transaction_id = format!("{}:platform", unknown.billing_request_id);
    unknown.status = UsageStatus::DeadLetter;
    unknown.lago_acked = true;
    unknown.released = true;
    unknown.funding = None;

    // A second service and layer exercise the independent rollup dimensions,
    // while its legacy grant and allowance fields remain rateable.
    let mut legacy = meter(&owner, 100);
    legacy.created_at = old;
    legacy.updated_at = old;
    legacy.finalized_at = Some(old);
    legacy.service_id = Some("service-legacy".into());
    legacy.service_slug = Some("legacy-service".into());
    legacy.layer = BillingLayer::Resale;
    legacy.transaction_id = format!("{}:resale", legacy.billing_request_id);
    legacy.lago_acked = true;
    legacy.released = true;
    legacy.funding = Some(UsageFunding {
        settled: true,
        credits_per_unit_micros: 2,
        allowance_consumptions: vec![AllowanceConsumptionAllocation {
            operation_id: "allowance-operation".into(),
            allowance_id: "allowance".into(),
            period_id: "period".into(),
            quantity: 10,
        }],
        grant_consumptions: vec![GrantConsumptionAllocation {
            operation_id: "grant-operation".into(),
            grant_id: "grant".into(),
            service_limit_key: None,
            amount: Credits::from_micros(50),
        }],
        wallet_charge_credits: Some(Credits::from_micros(130)),
        ..Default::default()
    });

    let mut free = meter(&owner, 8);
    free.created_at = old;
    free.updated_at = old;
    free.finalized_at = Some(old);
    free.wallet_id = None;
    free.service_id = Some("service-free".into());
    free.service_slug = Some("free-service".into());
    free.metric = BillingMetric::Requests;
    free.lago_metric_code = "free_requests".into();
    free.transaction_id = format!("{}:platform", free.billing_request_id);
    free.released = false;
    free.funding = None;

    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_many([exact, unknown, legacy, free])
        .await
        .unwrap();

    let raw_answer = billing::get_usage(
        State(state.clone()),
        test_auth_user(&owner),
        Query(UsageQuery {
            owner_id: None,
            machine_id: None,
            job_id: None,
            period: Some("90d".into()),
            bucket: None,
        }),
    )
    .await
    .unwrap()
    .0;

    crate::services::billing::usage_rollup::ensure_indexes(&db)
        .await
        .unwrap();
    while crate::services::billing::usage_rollup::fold_once(&db, Utc::now())
        .await
        .unwrap()
        > 0
    {}
    assert!(
        crate::services::billing::usage_rollup::state(&db)
            .await
            .unwrap()
            .unwrap()
            .daily_ready
    );

    let rows = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|row| !row.rollup_pending));
    db.collection::<UsageMeterRow>(USAGE_METER)
        .delete_many(doc! {})
        .await
        .unwrap();

    let rollup_answer = billing::get_usage(
        State(state.clone()),
        test_auth_user(&owner),
        Query(UsageQuery {
            owner_id: None,
            machine_id: None,
            job_id: None,
            period: Some("90d".into()),
            bucket: None,
        }),
    )
    .await
    .unwrap()
    .0;

    let numeric = |answer: &BillingUsageResponse| {
        let mut rows = answer
            .rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "service_slug": row.service_slug,
                    "layer": row.layer,
                    "metric": row.metric.as_str(),
                    "quantity": row.quantity,
                    "requests": row.requests,
                    "bytes": row.bytes,
                    "events": row.events,
                    "estimated_credits": row.estimated_credits,
                    "wallet_credits": row.wallet_credits,
                    "grant_credits": row.grant_credits,
                    "allowance_credits": row.allowance_credits,
                    "token_breakdown": row.token_breakdown,
                })
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row.to_string());
        let totals = serde_json::json!({
            "quantity": answer.totals.quantity,
            "requests": answer.totals.requests,
            "bytes": answer.totals.bytes,
            "events": answer.totals.events,
            "estimated_credits": answer.totals.estimated_credits,
            "wallet_credits": answer.totals.wallet_credits,
            "grant_credits": answer.totals.grant_credits,
            "allowance_credits": answer.totals.allowance_credits,
        });
        (rows, totals)
    };
    assert_eq!(numeric(&raw_answer), numeric(&rollup_answer));
    assert_eq!(raw_answer.totals.allowance_quantity, 20);
    assert_eq!(rollup_answer.totals.allowance_quantity, 0);
    assert!(rollup_answer.rows.iter().all(|row| !row.detail_retained));
    assert!(
        rollup_answer
            .rows
            .iter()
            .any(|row| row.service_slug.as_deref() == Some("exact-service")
                && row.estimated_credits.is_none()
                && row.grant_credits == Some(Credits::from_micros(100)))
    );
    let daily = db.collection::<bson::Document>(crate::models::usage_rollup_daily::COLLECTION_NAME);
    let original = daily
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    for row in &original {
        let mut legacy = row.clone();
        let mut old_costs = bson::Document::new();
        for field in [
            "gross_cost",
            "wallet_cost",
            "grant_cost",
            "allowance_cost",
            "legacy_grant_cost",
        ] {
            let amount = Credits::from_bson(row.get(field).unwrap().clone(), 1_000_000).unwrap();
            let old = if field == "legacy_grant_cost" {
                "legacy_grant".into()
            } else {
                format!("{field}_micros")
            };
            old_costs.insert(&old, amount.display_micros());
            legacy.insert(&old, amount.display_micros());
            legacy.remove(field);
            legacy.remove(format!("query_{field}"));
        }
        legacy.insert("query_costs", old_costs);
        daily
            .replace_one(doc! {"_id": row.get("_id").unwrap()}, legacy)
            .await
            .unwrap();
    }
    let read_daily = || {
        billing::get_usage(
            State(state.clone()),
            test_auth_user(&owner),
            Query(UsageQuery {
                owner_id: None,
                machine_id: None,
                job_id: None,
                period: Some("90d".into()),
                bucket: None,
            }),
        )
    };
    let legacy_answer = read_daily().await.unwrap().0;
    assert_eq!(
        numeric(&raw_answer),
        numeric(&legacy_answer),
        "before the marker, legacy micros mirrors match raw history"
    );
    for row in original {
        daily
            .replace_one(doc! {"_id": row.get("_id").unwrap()}, &row)
            .await
            .unwrap();
    }
    db.collection::<bson::Document>("billing_migrations")
        .insert_one(doc! {
            "_id": "exact-v2-rollups", "completed_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    let normalized = read_daily().await.unwrap().0;
    assert_eq!(
        numeric(&raw_answer),
        numeric(&normalized),
        "after the marker, flat exact mirrors match raw history"
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn long_usage_active_fold_and_handover_preserve_rows() {
    use crate::services::billing::usage_rollup;
    use tokio::sync::Notify;

    let db = connect_test_database("usage_history_active_fold")
        .await
        .unwrap();
    let owner = insert_owner(&db).await;
    let fold_at = Utc::now();
    let app = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    usage_rollup::ensure_indexes(&db).await.unwrap();
    let mut old = meter(&owner, 17);
    old.wallet_id = None;
    old.created_at = Utc::now() - chrono::Duration::days(60);
    let mut first = meter(&owner, 2);
    first.wallet_id = None;
    first.operation = Some("first".into());
    first.api_key_id = Some(Uuid::new_v4().to_string());
    let mut second = first.clone();
    second.id = Uuid::new_v4().to_string();
    second.transaction_id = second.id.clone();
    second.billing_request_id = second.id.clone();
    second.operation = Some("second".into());
    second.api_key_id = Some(Uuid::new_v4().to_string());
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_many([old.clone(), first, second])
        .await
        .unwrap();
    while usage_rollup::fold_once(&db, fold_at).await.unwrap() > 0 {}
    db.collection::<UsageMeterRow>(USAGE_METER)
        .delete_one(doc! {"_id": &old.id})
        .await
        .unwrap();
    let mut pending = old;
    pending.id = Uuid::new_v4().to_string();
    pending.transaction_id = pending.id.clone();
    pending.billing_request_id = pending.id.clone();
    pending.quantity = Some(13);
    pending.rollup_pending = true;
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(pending)
        .await
        .unwrap();

    let read = || {
        billing::get_usage(
            State(app.clone()),
            test_auth_user(&owner),
            Query(UsageQuery {
                owner_id: None,
                machine_id: None,
                job_id: None,
                period: Some("90d".into()),
                bucket: None,
            }),
        )
    };
    let baseline = read().await.unwrap().0;
    assert_eq!(baseline.totals.quantity, 34);
    assert_eq!(
        baseline
            .rows
            .iter()
            .filter(|row| row.detail_retained)
            .count(),
        3
    );
    assert_eq!(
        baseline
            .rows
            .iter()
            .filter(|row| row.operation.is_some())
            .count(),
        2
    );
    assert!(
        baseline
            .rows
            .iter()
            .filter(|row| !row.detail_retained)
            .all(|row| row.operation.is_none()
                && row.api_key_id.is_none()
                && row.api_key_name.is_none()
                && row.machine_usage.is_none()
                && row.org_channel_guest.is_none()
                && !row.lago_acked)
    );

    let claimed = Arc::new(Notify::new());
    let finish_fold = Arc::new(Notify::new());
    let raw_read = Arc::new(Notify::new());
    let finish_read = Arc::new(Notify::new());
    let worker =
        usage_rollup::TEST_FOLD_PAUSE.scope((claimed.clone(), finish_fold.clone()), async {
            let result = usage_rollup::fold_once(&db, fold_at).await.unwrap();
            finish_read.notify_one();
            result
        });
    let reader = async {
        claimed.notified().await;
        // The claim stays open for the entire endpoint call; no timing assumptions.
        let busy = read().await.unwrap().0;
        let sorted_rows = |rows: &[billing::BillingUsageRow]| {
            let mut values = rows
                .iter()
                .map(|row| serde_json::to_value(row).unwrap())
                .collect::<Vec<_>>();
            values.sort_by_key(|row| row.to_string());
            values
        };
        assert_eq!(sorted_rows(&busy.rows), sorted_rows(&baseline.rows));
        assert_eq!(
            serde_json::to_value(&busy.totals).unwrap(),
            serde_json::to_value(&baseline.totals).unwrap()
        );
        let exhausted = billing::TEST_USAGE_SNAPSHOT_EXPIRED
            .scope(true, read())
            .await
            .unwrap()
            .0;
        assert_eq!(
            exhausted.totals.quantity, baseline.totals.quantity,
            "snapshot exhaustion must retain permanent history"
        );
        assert_eq!(sorted_rows(&exhausted.rows), sorted_rows(&baseline.rows));
        billing::TEST_USAGE_READ_PAUSE
            .scope(
                std::cell::RefCell::new(Some((raw_read.clone(), finish_read.clone()))),
                read(),
            )
            .await
            .unwrap()
            .0
    };
    let complete_between_tiers = async {
        raw_read.notified().await;
        finish_fold.notify_one();
    };
    let (folded, during, ()) = tokio::join!(worker, reader, complete_between_tiers);
    assert_eq!(folded, 1);
    assert_eq!(
        during.totals.quantity, baseline.totals.quantity,
        "one row counted between tier reads must appear exactly once"
    );
    assert_eq!(during.totals.events, baseline.totals.events);
    let after = read().await.unwrap().0;
    assert_eq!(after.totals.quantity, baseline.totals.quantity);
    assert_eq!(
        after.rows.iter().filter(|row| row.detail_retained).count(),
        2
    );
    db.drop().await.unwrap();
}
