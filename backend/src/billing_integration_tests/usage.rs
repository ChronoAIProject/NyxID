use super::*;
use crate::handlers::billing::{self, BillingUsageResponse, BillingUsageRow, UsageQuery};
use crate::models::credits::Credits;
use crate::models::usage_meter::{BillingLayer, UsageFunding};
use crate::test_utils::test_auth_user;
use axum::extract::{Query, State};

fn meter(owner: &str, quantity: i64) -> UsageMeterRow {
    let now = Utc::now();
    let id = Uuid::new_v4().to_string();
    UsageMeterRow {
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
        user_service_id: None,
        service_id: Some("service".into()),
        service_slug: Some("llm-test".into()),
        metric: BillingMetric::Tokens,
        lago_metric_code: "platform_tokens".into(),
        credential_class: CredentialClass::NyxidManagedMaster,
        model: Some("test-model".into()),
        token_breakdown: None,
        reserved_credits: crate::models::credits::Credits::from_whole(0),
        funding: None,
        quantity: Some(quantity),
        pending_resale_quantity: None,
        pending_platform_usage: None,
        pool_attempt: None,
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
        Query(UsageQuery { period: None }),
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
        .resolve_for_execution(&actor, &org, CredentialClass::NyxidManagedMaster)
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
            .resolve_for_execution(&actor, &org, class)
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
            resolver.resolve_for_execution(&outsider, &org, class).await,
            Err(AppError::Forbidden(_))
        ));
        assert_eq!(
            resolver
                .resolve_for_execution(&actor, &actor, class)
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
