//! Real replica-set acceptance tests for allocation accounting and admission.
use super::{
    admission, allowances, funding, grants, meter, reservation, service_limits, summaries,
};
use crate::models::{
    billing_ledger::{BillingLedgerEntry, COLLECTION_NAME as LEDGER},
    billing_target::{BillingServiceScope, BillingTargetKind},
    billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
    credit_grant::{
        COLLECTION_NAME as GRANTS, CreditGrant, CreditGrantSettlementLock, CreditGrantStatus,
    },
    credit_service_limit::{CreditServiceLimit, CreditServiceLimitInput},
    service_billing::{BillingMetric, ResaleSpec},
    usage_allowance::{AllowanceRecurrence, COLLECTION_NAME as ALLOWANCES, UsageAllowance},
    usage_allowance_period::{
        AllowanceSettlementLock, COLLECTION_NAME as PERIODS, UsageAllowancePeriod,
    },
    usage_meter::{BillingLayer, COLLECTION_NAME as USAGE, UsageFunding, UsageMeterRow},
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use mongodb::options::IndexOptions;
use reservation::tests::{EntitledLago, insert_platform_rate, route_context, wallet};
use uuid::Uuid;

pub(super) async fn database(name: &str) -> mongodb::Database {
    let db = crate::test_utils::connect_test_database(name)
        .await
        .expect("real MongoDB replica set required; set NYXID_TEST_DATABASE_URL");
    super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    db.collection::<UsageMeterRow>(USAGE)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "transaction_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .unwrap();
    db
}
fn rule(limit: i64) -> CreditServiceLimit {
    CreditServiceLimit {
        key: "catalog-cap".into(),
        service_ids: vec!["catalog-1".into()],
        service_slugs: vec!["service-one".into()],
        label: Some("LLM".into()),
        limit_micros: limit,
        consumed_micros: 0,
        reserved_micros: 0,
    }
}
pub(super) fn allocation(owner: &str, amount: i64, limit: Option<i64>) -> CreditGrant {
    let now = Utc::now();
    CreditGrant {
        id: Uuid::new_v4().to_string(),
        batch_id: "batch".into(),
        schedule_origin: None,
        recipient_user_id: owner.into(),
        target_kind: BillingTargetKind::SelectedUsers,
        amount_credits: amount / grants::CREDIT_MICROS,
        amount_micros: amount,
        remaining_micros: amount,
        reserved_micros: 0,
        scope: BillingServiceScope {
            all_services: true,
            service_ids: vec![],
            service_slugs: vec![],
        },
        service_limits: limit.map(rule).into_iter().collect(),
        expires_at: Some(now + Duration::days(1)),
        reason: None,
        granted_by: "admin".into(),
        status: CreditGrantStatus::Active,
        issued_ledgered_at: Some(now),
        terminal_ledgered_at: None,
        terminal_amount_micros: 0,
        active_settlement: None,
        created_at: now,
        updated_at: now,
        consumed_at: None,
        expired_at: None,
        revoked_at: None,
        revoke_requested_at: None,
        revoke_requested_by: None,
    }
}
pub(super) fn entitled() -> EntitledLago {
    EntitledLago {
        entitlements: vec![super::lago_client::Entitlement {
            code: "service-one".into(),
            raw: serde_json::json!({}),
        }],
    }
}
pub(super) async fn setup(db: &mongodb::Database, owner: &str, rate: i64) {
    db.collection::<BillingWallet>(WALLETS)
        .insert_one(wallet(owner, 10_000))
        .await
        .unwrap();
    insert_platform_rate(db, rate).await;
}
pub(super) async fn saved(db: &mongodb::Database, id: &str) -> CreditGrant {
    db.collection::<CreditGrant>(GRANTS)
        .find_one(doc! { "_id": id })
        .await
        .unwrap()
        .unwrap()
}
pub(super) async fn row(db: &mongodb::Database, request: &str) -> UsageMeterRow {
    db.collection::<UsageMeterRow>(USAGE)
        .find_one(doc! { "billing_request_id": request })
        .await
        .unwrap()
        .unwrap()
}
async fn mark_released(db: &mongodb::Database, row: &UsageMeterRow) -> UsageMeterRow {
    db.collection::<UsageMeterRow>(USAGE)
        .update_one(
            doc! { "_id": &row.id },
            doc! { "$set": { "released": true, "status": "failed" } },
        )
        .await
        .unwrap();
    db.collection::<UsageMeterRow>(USAGE)
        .find_one(doc! { "_id": &row.id })
        .await
        .unwrap()
        .unwrap()
}
async fn release(db: &mongodb::Database, row: &UsageMeterRow) {
    let released = mark_released(db, row).await;
    funding::release_usage_reservations(db, &released)
        .await
        .unwrap();
}
async fn finalize(
    db: &mongodb::Database,
    row: &UsageMeterRow,
    quantity: i64,
) -> funding::FundingSettlement {
    db.collection::<UsageMeterRow>(USAGE)
        .update_one(
            doc! { "_id": &row.id },
            doc! { "$set": { "quantity": quantity, "status": "finalized", "forwarded": true } },
        )
        .await
        .unwrap();
    let row = db
        .collection::<UsageMeterRow>(USAGE)
        .find_one(doc! { "_id": &row.id })
        .await
        .unwrap()
        .unwrap();
    funding::settle_usage_funding(db, &row).await.unwrap()
}

#[tokio::test]
async fn concurrent_admission_respects_shared_rule_and_records_every_hold() {
    let db = database("billing_caps_concurrent_admission").await;
    setup(&db, "owner", 1).await;
    let grant = allocation("owner", 100_000_000, Some(5_000_000));
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let mut tasks = vec![];
    for _ in 0..20 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            let ctx = route_context("owner");
            admission::open(&db, Some(&entitled()), &ctx, false, 900)
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let stored = saved(&db, &grant.id).await;
    assert_eq!(stored.reserved_micros, 5_000_000);
    assert_eq!(stored.service_limits[0].reserved_micros, 5_000_000);
    let rows: Vec<UsageMeterRow> = db
        .collection::<UsageMeterRow>(USAGE)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 20);
    assert_eq!(
        rows.iter()
            .map(|r| r
                .funding
                .as_ref()
                .unwrap()
                .grant_reservations
                .iter()
                .map(|v| v.amount_micros)
                .sum::<i64>())
            .sum::<i64>(),
        5_000_000
    );
    assert_eq!(
        db.collection::<BillingWallet>(WALLETS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
            .reserved_credits,
        15
    );
    for r in &rows {
        release(&db, r).await;
    }
    assert_eq!(
        saved(&db, &grant.id).await.service_limits[0].reserved_micros,
        0
    );
}

#[tokio::test]
async fn actual_true_up_obeys_cap_and_other_sources_cover_overage_once() {
    let db = database("billing_caps_actual_true_up").await;
    setup(&db, "owner", 1).await;
    let grant = allocation("owner", 100_000_000, Some(2_000_000));
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let ctx = route_context("owner");
    admission::open(&db, Some(&entitled()), &ctx, false, 900)
        .await
        .unwrap();
    let usage = row(&db, &ctx.billing_request_id).await;
    let result = finalize(&db, &usage, 3).await;
    assert_eq!(result.wallet_charge_credits, 1);
    assert_eq!(result.lago_billable_quantity_micros, 1_000_000);
    assert_eq!(
        saved(&db, &grant.id).await.service_limits[0].consumed_micros,
        2_000_000
    );
    let fresh = row(&db, &ctx.billing_request_id).await;
    assert_eq!(
        funding::settle_usage_funding(&db, &fresh).await.unwrap(),
        result
    );
    assert_eq!(saved(&db, &grant.id).await.remaining_micros, 98_000_000);
    assert_eq!(saved(&db, &grant.id).await.reserved_micros, 0);
    assert_eq!(
        db.collection::<BillingLedgerEntry>(LEDGER)
            .count_documents(doc! { "reference_id": &grant.id, "event_type": "grant_consumed" })
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn reclaim_during_hold_fences_extras_and_removes_only_final_remainder() {
    let db = database("billing_reclaim_during_hold").await;
    setup(&db, "owner", 40).await;
    let mut grant = allocation("owner", 1_000_000_000, Some(1_000_000_000));
    grant.remaining_micros = 100_000_000;
    grant.service_limits[0].consumed_micros = 900_000_000;
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let ctx = route_context("owner");
    admission::open(&db, Some(&entitled()), &ctx, false, 900)
        .await
        .unwrap();
    let pending = grants::request_revoke_grant(&db, &grant.id, Some("admin"))
        .await
        .unwrap();
    assert_eq!(pending.status, CreditGrantStatus::Active);
    assert_eq!(pending.reserved_micros, 40_000_000);
    assert_eq!(grants::eligible_available_micros(&pending, Utc::now()), 0);
    assert_eq!(
        grants::revoke_grant(&db, &grant.id)
            .await
            .unwrap()
            .revoke_requested_at,
        pending.revoke_requested_at
    );
    let summary = summaries::user_summary(&db, "owner", Utc::now())
        .await
        .unwrap();
    assert_eq!(summary.reserved_grant_micros, 40_000_000);
    assert_eq!(summary.available_grant_micros, 0);
    // Actual quantity 2 costs80. The admitted40 may settle; extra40 must use wallet.
    let result = finalize(&db, &row(&db, &ctx.billing_request_id).await, 2).await;
    assert_eq!(result.wallet_charge_credits, 40);
    let terminal = grants::revoke_grant(&db, &grant.id).await.unwrap();
    assert_eq!(terminal.status, CreditGrantStatus::Revoked);
    assert_eq!(terminal.terminal_amount_micros, 60_000_000);
    assert_eq!(grants::used_micros(&terminal), 940_000_000);
    assert_eq!(terminal.service_limits[0].consumed_micros, 940_000_000);
    grants::expire_due_grants(&db, Utc::now() + Duration::days(2))
        .await
        .unwrap();
    grants::recover_unledgered_events(&db, Utc::now())
        .await
        .unwrap();
    assert_eq!(
        db.collection::<BillingLedgerEntry>(LEDGER)
            .count_documents(doc! { "dedupe_key": format!("grant-revoked:{}", grant.id) })
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<BillingWallet>(WALLETS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        10_000
    );
}

#[tokio::test]
async fn expired_held_sources_remain_visible_and_release_completes_pending_reclaim() {
    let db = database("billing_expired_held_reclaim_release").await;
    setup(&db, "owner", 40).await;
    let grant = allocation("owner", 100_000_000, Some(50_000_000));
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let ctx = route_context("owner");
    admission::open(&db, Some(&entitled()), &ctx, false, 900)
        .await
        .unwrap();
    db.collection::<CreditGrant>(GRANTS).update_one(doc! { "_id": &grant.id }, doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1)) } }).await.unwrap();
    let summary = summaries::user_summary(&db, "owner", Utc::now())
        .await
        .unwrap();
    assert_eq!(summary.grants.len(), 1);
    assert_eq!(summary.available_grant_micros, 0);
    assert_eq!(summary.reserved_grant_micros, 40_000_000);
    grants::request_revoke_grant(&db, &grant.id, Some("admin"))
        .await
        .unwrap();
    let usage = mark_released(&db, &row(&db, &ctx.billing_request_id).await).await;
    funding::release_usage_reservations(&db, &usage)
        .await
        .unwrap();
    funding::release_usage_reservations(&db, &usage)
        .await
        .unwrap();
    let terminal = saved(&db, &grant.id).await;
    assert_eq!(terminal.terminal_amount_micros, 100_000_000);
    assert_eq!(terminal.service_limits[0].reserved_micros, 0);
    assert_eq!(terminal.status, CreditGrantStatus::Revoked);
}

pub(super) async fn add_allowance(db: &mongodb::Database, owner: &str) -> UsageAllowance {
    let now = Utc::now();
    let allowance = UsageAllowance {
        id: "units".into(),
        service_id: "catalog-1".into(),
        service_slug: "service-one".into(),
        metric: BillingMetric::Requests,
        quantity: 1,
        recurrence: AllowanceRecurrence::Monthly,
        target_kind: BillingTargetKind::SelectedUsers,
        target_user_ids: vec![owner.into()],
        is_active: true,
        created_by: "admin".into(),
        created_at: now,
        updated_at: now,
    };
    db.collection::<UsageAllowance>(ALLOWANCES)
        .insert_one(&allowance)
        .await
        .unwrap();
    allowance
}

#[tokio::test]
async fn transaction_abort_at_handoff_rolls_back_all_resources_and_rows() {
    let db = database("billing_admission_fault_boundaries").await;
    setup(&db, "owner", 1).await;
    add_allowance(&db, "owner").await;
    let grant = allocation("owner", 1_000_000, Some(1_000_000));
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    for after_rows in [false, true] {
        let ctx = route_context("owner");
        let mut prepared = reservation::prepare_admission(&db, Some(&entitled()), &ctx, false, 900)
            .await
            .unwrap()
            .unwrap();
        prepared.layers[0].estimated_quantity = 3; // allowance1, issued1, wallet1
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let holds =
            reservation::gate_and_reserve_in_session(&db, &ctx, Some(&prepared), &mut session)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(holds.layers[0].allowance_reservations[0].quantity, 1);
        assert_eq!(
            holds.layers[0].grant_reservations[0].amount_micros,
            1_000_000
        );
        assert_eq!(holds.total_reserved_credits, 1);
        if after_rows {
            meter::open_in_session(&db, &ctx, Some(&holds), &mut session)
                .await
                .unwrap();
        }
        // Simulates process loss at either side of the holds -> row handoff.
        // MongoDB abort/timeout does not expose any part of the transaction.
        session.abort_transaction().await.unwrap();
        assert_eq!(saved(&db, &grant.id).await.reserved_micros, 0);
        assert_eq!(
            saved(&db, &grant.id).await.service_limits[0].reserved_micros,
            0
        );
        assert_eq!(
            db.collection::<BillingWallet>(WALLETS)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
                .reserved_credits,
            0
        );
        assert_eq!(
            db.collection::<UsageAllowancePeriod>(PERIODS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<UsageMeterRow>(USAGE)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    // After commit but before forwarding, the existing abandon/release path
    // has complete ownership of every hold and can recover it idempotently.
    let ctx = route_context("owner");
    admission::open(&db, Some(&entitled()), &ctx, false, 900)
        .await
        .unwrap();
    assert_eq!(
        reservation::abandon_stale_unforwarded(&db, Utc::now() + Duration::seconds(1))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        reservation::abandon_stale_unforwarded(&db, Utc::now() + Duration::seconds(1))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<UsageAllowancePeriod>(PERIODS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
            .reserved_quantity,
        0
    );
}

#[tokio::test]
async fn second_layer_insert_error_aborts_holds_and_first_layer_without_compensation() {
    let db = database("billing_admission_partial_insert_failure").await;
    setup(&db, "owner", 1).await;
    let grant = allocation("owner", 2_000_000, Some(1_000_000));
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let mut ctx = route_context("owner");
    ctx.resale = Some(ResaleSpec {
        metric: BillingMetric::Requests,
        lago_metric_code: "platform_requests".into(),
    });
    meter::open(&db, &ctx, None).await.unwrap();
    db.collection::<UsageMeterRow>(USAGE)
        .delete_one(doc! { "layer": "platform" })
        .await
        .unwrap();
    let prepared = reservation::prepare_admission(&db, Some(&entitled()), &ctx, false, 900)
        .await
        .unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let holds =
        reservation::gate_and_reserve_in_session(&db, &ctx, prepared.as_ref(), &mut session)
            .await
            .unwrap();
    assert!(
        meter::open_in_session(&db, &ctx, holds.as_ref(), &mut session)
            .await
            .is_err()
    );
    let _ = session.abort_transaction().await;
    assert_eq!(
        db.collection::<UsageMeterRow>(USAGE)
            .count_documents(doc! { "layer": "platform" })
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<UsageMeterRow>(USAGE)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(saved(&db, &grant.id).await.reserved_micros, 0);
    assert_eq!(
        db.collection::<BillingWallet>(WALLETS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
            .reserved_credits,
        0
    );
}

#[tokio::test]
async fn duplicate_concurrent_open_never_commits_a_second_hold() {
    let db = database("billing_admission_same_request_race").await;
    setup(&db, "owner", 1).await;
    let grant = allocation("owner", 100_000_000, Some(5_000_000));
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let ctx = route_context("owner");
    let lago = entitled();
    let (a, b) = tokio::join!(
        admission::open(&db, Some(&lago), &ctx, false, 900),
        admission::open(&db, Some(&lago), &ctx, false, 900)
    );
    assert!(a.is_ok() || b.is_ok());
    admission::open(&db, Some(&entitled()), &ctx, false, 900)
        .await
        .unwrap();
    assert_eq!(
        db.collection::<UsageMeterRow>(USAGE)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(saved(&db, &grant.id).await.reserved_micros, 1_000_000);
}

#[tokio::test]
async fn unapplied_extra_lock_is_helped_before_admitting_new_holds() {
    let db = database("billing_grant_reservation_extra_lock_race").await;
    setup(&db, "owner", 1).await;
    let mut grant = allocation("owner", 100_000_000, Some(100_000_000));
    // Make an existing usage row without adding holds, then pause the extra
    // settlement exactly after its claim and before its resource mutation.
    let old_ctx = route_context("owner");
    meter::open(&db, &old_ctx, None).await.unwrap();
    let old = row(&db, &old_ctx.billing_request_id).await;
    db.collection::<UsageMeterRow>(USAGE)
        .update_one(
            doc! { "_id": &old.id },
            doc! { "$set": { "funding": bson::to_bson(&UsageFunding::default()).unwrap() } },
        )
        .await
        .unwrap();
    grant.active_settlement = Some(CreditGrantSettlementLock {
        operation_id: "extra-lock".into(),
        usage_row_id: old.id,
        reserved_micros: 0,
        consume_micros: 99_000_000,
        service_limit_key: Some("catalog-cap".into()),
        applied: false,
        updated_at: Utc::now(),
    });
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let ctx = route_context("owner");
    admission::open(&db, Some(&entitled()), &ctx, false, 900)
        .await
        .unwrap();
    let stored = saved(&db, &grant.id).await;
    assert_eq!(stored.remaining_micros, 1_000_000);
    assert_eq!(stored.reserved_micros, 1_000_000);
    assert_eq!(stored.service_limits[0].consumed_micros, 99_000_000);
    assert_eq!(stored.service_limits[0].reserved_micros, 1_000_000);
    assert_eq!(row(&db, &ctx.billing_request_id).await.reserved_credits, 0);
}

#[tokio::test]
async fn allowance_lock_is_helped_and_unrelated_locks_do_not_block_free_traffic() {
    let db = database("billing_allowance_reservation_extra_lock_race").await;
    setup(&db, "owner", 1).await;
    let allowance = add_allowance(&db, "owner").await;
    let period = allowances::ensure_current_period(&db, &allowance, "owner", Utc::now())
        .await
        .unwrap();
    let old_ctx = route_context("owner");
    meter::open(&db, &old_ctx, None).await.unwrap();
    let old = row(&db, &old_ctx.billing_request_id).await;
    db.collection::<UsageMeterRow>(USAGE)
        .update_one(
            doc! { "_id": &old.id },
            doc! { "$set": { "funding": bson::to_bson(&UsageFunding::default()).unwrap() } },
        )
        .await
        .unwrap();
    let lock = AllowanceSettlementLock {
        operation_id: "units-extra".into(),
        usage_row_id: old.id,
        reserved_quantity: 0,
        consume_quantity: 1,
        applied: false,
        updated_at: Utc::now(),
    };
    db.collection::<UsageAllowancePeriod>(PERIODS)
        .update_one(
            doc! { "_id": &period.id },
            doc! { "$set": { "active_settlement": bson::to_bson(&lock).unwrap() } },
        )
        .await
        .unwrap();
    let ctx = route_context("owner");
    admission::open(&db, Some(&entitled()), &ctx, false, 900)
        .await
        .unwrap();
    let stored = db
        .collection::<UsageAllowancePeriod>(PERIODS)
        .find_one(doc! { "_id": &period.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.consumed_quantity, 1);
    assert_eq!(stored.reserved_quantity, 0);
    assert_eq!(row(&db, &ctx.billing_request_id).await.reserved_credits, 1);
    let mut broken = allocation("owner", 100_000_000, None);
    broken.active_settlement = Some(CreditGrantSettlementLock {
        operation_id: "missing-usage".into(),
        usage_row_id: "missing".into(),
        reserved_micros: 0,
        consume_micros: 10,
        service_limit_key: None,
        applied: false,
        updated_at: Utc::now(),
    });
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&broken)
        .await
        .unwrap();
    let free = route_context("owner").with_platform_metering(false);
    admission::open(&db, Some(&entitled()), &free, false, 900)
        .await
        .unwrap();
    assert!(!row(&db, &free.billing_request_id).await.forwarded);
    assert_eq!(
        saved(&db, &broken.id).await.remaining_micros,
        broken.remaining_micros
    );
}

#[tokio::test]
async fn platform_and_resale_share_ceiling_allowance_units_do_not_use_it() {
    let db = database("billing_caps_platform_resale_shared").await;
    setup(&db, "owner", 1).await;
    add_allowance(&db, "owner").await;
    let grant = allocation("owner", 100_000_000, Some(2_000_000));
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let mut ctx = route_context("owner");
    ctx.resale = Some(ResaleSpec {
        metric: BillingMetric::Requests,
        lago_metric_code: "platform_requests".into(),
    });
    admission::open(&db, Some(&entitled()), &ctx, false, 900)
        .await
        .unwrap();
    let rows: Vec<UsageMeterRow> = db
        .collection::<UsageMeterRow>(USAGE)
        .find(doc! { "billing_request_id": &ctx.billing_request_id })
        .sort(doc! { "layer": 1 })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].layer, BillingLayer::Platform);
    assert!(
        rows[0]
            .funding
            .as_ref()
            .unwrap()
            .grant_reservations
            .is_empty()
    );
    assert_eq!(
        saved(&db, &grant.id).await.service_limits[0].reserved_micros,
        1_000_000
    );
    let platform = finalize(&db, &rows[0], 2).await;
    let resale = finalize(&db, &rows[1], 3).await;
    assert_eq!(platform.wallet_charge_credits, 0);
    assert_eq!(resale.wallet_charge_credits, 2);
    let stored = saved(&db, &grant.id).await;
    assert_eq!(stored.service_limits[0].consumed_micros, 2_000_000);
    assert_eq!(stored.service_limits[0].reserved_micros, 0);
}

async fn catalog(db: &mongodb::Database, id: &str, slug: &str) {
    db.collection::<mongodb::bson::Document>(crate::models::downstream_service::COLLECTION_NAME).insert_one(doc! {
        "_id": id, "name": slug, "slug": slug, "base_url": "https://example.invalid", "auth_method": "none", "auth_key_name": "",
        "credential_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![] },
        "is_active": true, "created_by": "admin", "created_at": bson::DateTime::from_chrono(Utc::now()), "updated_at": bson::DateTime::from_chrono(Utc::now()),
    }).await.unwrap();
}
fn limit_input(refs: &[&str], amount: i64) -> CreditServiceLimitInput {
    CreditServiceLimitInput {
        service_refs: refs.iter().map(|v| (*v).into()).collect(),
        amount_credits: amount,
        label: Some("LLM".into()),
    }
}

#[tokio::test]
async fn canonical_groups_reject_overlap_and_scope_and_bound_realizable_summary() {
    let db = database("billing_caps_canonical_groups").await;
    catalog(&db, "catalog-1", "service-one").await;
    catalog(&db, "catalog-2", "service-two").await;
    let scope = BillingServiceScope {
        all_services: false,
        service_ids: vec!["catalog-1".into(), "catalog-2".into()],
        service_slugs: vec!["service-one".into(), "service-two".into()],
    };
    let rules = service_limits::resolve(
        &db,
        &scope,
        10_000_000_000,
        &[
            limit_input(&["service-one"], 500),
            limit_input(&["catalog-2"], 300),
        ],
    )
    .await
    .unwrap();
    let mut grant = allocation("owner", 10_000_000_000, None);
    grant.scope = scope.clone();
    grant.service_limits = rules;
    assert_eq!(service_limits::realizable_micros(&grant), 800_000_000);
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let summary = summaries::user_summary(&db, "owner", Utc::now())
        .await
        .unwrap();
    assert_eq!(summary.available_grant_micros, 800_000_000);
    assert_eq!(summary.restricted_grant_count, 1);
    let admin = summaries::admin_summary(&db, Utc::now()).await.unwrap();
    assert_eq!(admin.available_micros, 800_000_000);
    assert_eq!(admin.unavailable_micros, 9_200_000_000);
    assert_eq!(
        admin.outstanding_micros,
        admin.available_micros
            + admin.reserved_micros
            + admin.pending_activation_micros
            + admin.unavailable_micros
    );
    grant.scope.all_services = true;
    assert_eq!(service_limits::realizable_micros(&grant), 10_000_000_000);
    assert!(grants::has_restrictions(&grant));
    for inputs in [
        vec![limit_input(&["catalog-1", "service-one"], 1)],
        vec![
            limit_input(&["catalog-1"], 1),
            limit_input(&["service-one"], 2),
        ],
        vec![limit_input(&["missing"], 1)],
        vec![limit_input(&["catalog-1"], 0)],
        vec![limit_input(&["catalog-1"], 10_001)],
    ] {
        assert!(
            service_limits::resolve(&db, &scope, 10_000_000_000, &inputs)
                .await
                .is_err()
        );
    }
    let narrow = BillingServiceScope {
        all_services: false,
        service_ids: vec!["catalog-1".into()],
        service_slugs: vec!["service-one".into()],
    };
    assert!(
        service_limits::resolve(
            &db,
            &narrow,
            10_000_000_000,
            &[limit_input(&["catalog-2"], 1)]
        )
        .await
        .is_err()
    );
    // A stale/reused slug never wins over the concrete execution identity.
    let selected =
        service_limits::matching_rule(&grant, Some("catalog-2"), Some("service-one")).unwrap();
    assert_eq!(selected.service_ids, vec!["catalog-2"]);
    assert!(
        service_limits::matching_rule(&grant, Some("custom-instance"), Some("service-one"))
            .is_none()
    );
    assert!(service_limits::matching_rule(&grant, None, Some("service-one")).is_some());
    assert!(!grants::service_scope_applies(
        &narrow,
        Some("catalog-2"),
        Some("service-one")
    ));
}

#[tokio::test]
async fn actor_scoped_retry_snapshot_recovers_partial_batch_and_concurrent_replay() {
    let db = database("billing_allocation_retry_snapshot").await;
    let users: Vec<_> = (0..60)
        .map(|i| {
            crate::test_utils::test_user(
                &format!("owner-{i:02}"),
                crate::models::user::UserType::Person,
            )
        })
        .collect();
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_many(users)
        .await
        .unwrap();
    let input = grants::IssueCreditGrantInput {
        amount_credits: 100,
        target_kind: BillingTargetKind::AllUsers,
        target_user_ids: vec![],
        all_services: true,
        service_refs: vec![],
        expires_at: None,
        reason: Some("batch".into()),
        granted_by: "admin".into(),
        service_limits: vec![],
        request_id: Some(Uuid::new_v4().to_string()),
    };
    let first = grants::issue_grants(&db, input.clone()).await.unwrap();
    assert_eq!(first.len(), 60);
    assert!(
        first
            .iter()
            .filter(|g| g.issued_ledgered_at.is_some())
            .count()
            <= 50
    );
    let preserved_id = &first[0].id;
    db.collection::<CreditGrant>(GRANTS)
        .update_one(
            doc! { "_id": preserved_id },
            doc! { "$set": { "remaining_micros": 1_234_567_i64 } },
        )
        .await
        .unwrap();
    let missing: Vec<&str> = first.iter().skip(50).map(|g| g.id.as_str()).collect();
    db.collection::<CreditGrant>(GRANTS)
        .delete_many(doc! { "_id": { "$in": missing } })
        .await
        .unwrap();
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            "later-owner",
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        grants::issue_grants(&db, input.clone()),
        grants::issue_grants(&db, input.clone())
    );
    assert_eq!(a.unwrap().len(), 60);
    assert_eq!(b.unwrap().len(), 60);
    assert_eq!(
        db.collection::<CreditGrant>(GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        60
    );
    assert_eq!(saved(&db, preserved_id).await.remaining_micros, 1_234_567);
    let mut changed = input.clone();
    changed.amount_credits = 101;
    assert!(matches!(
        grants::issue_grants(&db, changed).await,
        Err(crate::errors::AppError::Conflict(_))
    ));
    let mut other_actor = input;
    other_actor.granted_by = "other-admin".into();
    assert_eq!(
        grants::issue_grants(&db, other_actor).await.unwrap().len(),
        61
    );
    assert_eq!(
        db.collection::<CreditGrant>(GRANTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        121
    );
}

#[tokio::test]
async fn recurring_retry_and_period_snapshot_preserve_future_only_rules() {
    use crate::models::credit_schedule::{CreditExpiryPolicy, ScheduleRecurrence};
    let db = database("billing_recurring_caps_frozen").await;
    catalog(&db, "catalog-1", "service-one").await;
    for owner in ["owner-a", "owner-b"] {
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
    }
    for recurrence in [
        ScheduleRecurrence::Daily,
        ScheduleRecurrence::Weekly,
        ScheduleRecurrence::Monthly,
    ] {
        let input = super::schedules::CreateScheduleInput {
            amount_credits: 100,
            recurrence,
            expiry: CreditExpiryPolicy::Never,
            target_kind: BillingTargetKind::SelectedUsers,
            target_user_ids: vec!["owner-a".into(), "owner-b".into()],
            all_services: true,
            service_refs: vec![],
            reason: None,
            created_by: "admin".into(),
            service_limits: vec![limit_input(&["catalog-1"], 30)],
            request_id: Some(Uuid::new_v4().to_string()),
        };
        let (a, b) = tokio::join!(
            super::schedules::create_schedule(&db, input.clone()),
            super::schedules::create_schedule(&db, input.clone())
        );
        let schedule = a.unwrap();
        assert_eq!(b.unwrap().id, schedule.id);
        let now = Utc::now();
        super::schedules::disburse_due(&db, now, 1).await.unwrap();
        super::schedules::update_schedule(
            &db,
            &schedule.id,
            super::schedules::UpdateScheduleInput {
                service_limits: Some(vec![limit_input(&["service-one"], 20)]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        super::schedules::disburse_due(&db, now, 10).await.unwrap();
        let window = super::schedules::schedule_period(recurrence, now);
        let first: Vec<CreditGrant> = db.collection::<CreditGrant>(GRANTS).find(doc! { "schedule_origin.schedule_id": &schedule.id, "schedule_origin.period_start": bson::DateTime::from_chrono(window.start) }).await.unwrap().try_collect().await.unwrap();
        assert_eq!(first.len(), 2);
        assert!(
            first
                .iter()
                .all(|g| g.service_limits[0].limit_micros == 30_000_000)
        );
        let next = window.end + Duration::seconds(1);
        super::schedules::disburse_due(&db, next, 20).await.unwrap();
        let current = super::schedules::schedule_period(recurrence, next);
        let future: Vec<CreditGrant> = db.collection::<CreditGrant>(GRANTS).find(doc! { "schedule_origin.schedule_id": &schedule.id, "schedule_origin.period_start": bson::DateTime::from_chrono(current.start) }).await.unwrap().try_collect().await.unwrap();
        assert_eq!(future.len(), 2);
        assert!(
            future
                .iter()
                .all(|g| g.service_limits[0].limit_micros == 20_000_000
                    && g.service_limits[0].consumed_micros == 0
                    && g.service_limits[0].reserved_micros == 0)
        );
        let replay = super::schedules::create_schedule(&db, input).await.unwrap();
        assert_eq!(replay.service_limits[0].limit_micros, 20_000_000);
        super::schedules::update_schedule(
            &db,
            &schedule.id,
            super::schedules::UpdateScheduleInput {
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn concurrent_stale_release_cannot_free_another_requests_hold() {
    let db = database("billing_caps_release_operation_replay").await;
    setup(&db, "owner", 1).await;
    let grant = allocation("owner", 100_000_000, Some(10_000_000));
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let first = route_context("owner");
    let second = route_context("owner");
    admission::open(&db, Some(&entitled()), &first, false, 900)
        .await
        .unwrap();
    admission::open(&db, Some(&entitled()), &second, false, 900)
        .await
        .unwrap();
    let stale = mark_released(&db, &row(&db, &first.billing_request_id).await).await;
    let (a, b) = tokio::join!(
        funding::release_usage_reservations(&db, &stale),
        funding::release_usage_reservations(&db, &stale)
    );
    a.unwrap();
    b.unwrap();
    funding::release_usage_reservations(&db, &stale)
        .await
        .unwrap();
    let saved = saved(&db, &grant.id).await;
    assert_eq!(saved.reserved_micros, 1_000_000);
    assert_eq!(saved.service_limits[0].reserved_micros, 1_000_000);
}

#[tokio::test]
async fn canonical_scope_rejects_reused_slug_in_admission_and_actual_extra() {
    let db = database("billing_caps_reused_slug_execution").await;
    setup(&db, "owner", 1).await;
    let mut grant = allocation("owner", 100_000_000, Some(5_000_000));
    grant.scope = BillingServiceScope {
        all_services: false,
        service_ids: vec!["catalog-1".into()],
        service_slugs: vec!["service-one".into()],
    };
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let mut wrong = route_context("owner");
    wrong.catalog_service_id = Some("catalog-reusing-old-slug".into());
    // Same stored slug but a different catalog identity, including at true-up.
    admission::open(&db, Some(&entitled()), &wrong, false, 900)
        .await
        .unwrap();
    let usage = row(&db, &wrong.billing_request_id).await;
    assert!(
        usage
            .funding
            .as_ref()
            .unwrap()
            .grant_reservations
            .is_empty()
    );
    assert_eq!(finalize(&db, &usage, 2).await.wallet_charge_credits, 2);
    assert_eq!(
        saved(&db, &grant.id).await.remaining_micros,
        grant.amount_micros
    );
    // Separate user instances of the same concrete catalog service share cap.
    let mut first = route_context("owner");
    first.user_service_id = Some("instance-a".into());
    let mut second = route_context("owner");
    second.user_service_id = Some("instance-b".into());
    admission::open(&db, Some(&entitled()), &first, false, 900)
        .await
        .unwrap();
    admission::open(&db, Some(&entitled()), &second, false, 900)
        .await
        .unwrap();
    assert_eq!(
        saved(&db, &grant.id).await.service_limits[0].reserved_micros,
        2_000_000
    );
}

#[tokio::test]
async fn exhausted_cap_with_unrelated_lock_falls_through_to_independent_allocation() {
    let db = database("billing_caps_exhausted_unrelated_lock").await;
    setup(&db, "owner", 1).await;
    let mut exhausted = allocation("owner", 100_000_000, Some(1_000_000));
    exhausted.service_limits[0].consumed_micros = 1_000_000;
    exhausted.remaining_micros -= 1_000_000;
    exhausted.expires_at = Some(Utc::now() + Duration::hours(1));
    exhausted.active_settlement = Some(CreditGrantSettlementLock {
        operation_id: "unrelated-failed-journal".into(),
        usage_row_id: "not-present".into(),
        reserved_micros: 0,
        consume_micros: 1,
        service_limit_key: None,
        applied: true,
        updated_at: Utc::now(),
    });
    let independent = allocation("owner", 10_000_000, None);
    db.collection::<CreditGrant>(GRANTS)
        .insert_many([&exhausted, &independent])
        .await
        .unwrap();
    let ctx = route_context("owner");
    admission::open(&db, Some(&entitled()), &ctx, false, 900)
        .await
        .unwrap();
    let usage = row(&db, &ctx.billing_request_id).await;
    assert_eq!(
        usage.funding.unwrap().grant_reservations[0].grant_id,
        independent.id
    );
    assert_eq!(saved(&db, &exhausted.id).await.reserved_micros, 0);
}

#[tokio::test]
async fn concurrent_same_request_with_exactly_one_capacity_both_succeed() {
    for grant_funded in [false, true] {
        let db = database(if grant_funded {
            "billing_admission_one_grant_race"
        } else {
            "billing_admission_one_wallet_race"
        })
        .await;
        let owner = "owner";
        db.collection::<BillingWallet>(WALLETS)
            .insert_one(wallet(owner, if grant_funded { 0 } else { 1 }))
            .await
            .unwrap();
        insert_platform_rate(&db, 1).await;
        if grant_funded {
            db.collection::<CreditGrant>(GRANTS)
                .insert_one(allocation(owner, 1_000_000, Some(1_000_000)))
                .await
                .unwrap();
        }
        let ctx = route_context(owner);
        let lago = entitled();
        let (a, b) = tokio::join!(
            admission::open(&db, Some(&lago), &ctx, false, 900),
            admission::open(&db, Some(&lago), &ctx, false, 900)
        );
        assert!(a.is_ok(), "first: {a:?}");
        assert!(b.is_ok(), "second: {b:?}");
        assert_eq!(
            db.collection::<UsageMeterRow>(USAGE)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<BillingWallet>(WALLETS)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
                .reserved_credits,
            if grant_funded { 0 } else { 1 }
        );
        if grant_funded {
            assert_eq!(
                db.collection::<CreditGrant>(GRANTS)
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap()
                    .reserved_micros,
                1_000_000
            );
        }
    }
}

#[tokio::test]
async fn disabled_meter_does_not_access_an_unavailable_database() {
    let client =
        mongodb::Client::with_uri_str("mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=20")
            .await
            .unwrap();
    let db = client.database("billing_disabled_unreachable");
    let mut ctx = route_context("owner");
    ctx.platform_metered = false;
    ctx.platform_billable = false;
    ctx.resale = None;
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        meter::open(&db, &ctx, None),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!result.is_enabled());
    let result = admission::open(&db, None, &ctx, false, 900).await.unwrap();
    assert!(!result.is_enabled());
}

#[tokio::test]
async fn immediate_reclaim_is_isolated_from_another_allocation_and_wallet() {
    let db = database("billing_reclaim_isolated_remainder").await;
    setup(&db, "owner", 1).await;
    let mut grant = allocation("owner", 1_000_000_000, None);
    grant.remaining_micros = 100_000_000;
    let other = allocation("owner", 123_000_000, Some(10_000_000));
    db.collection::<CreditGrant>(GRANTS)
        .insert_many([&grant, &other])
        .await
        .unwrap();
    let result = grants::request_revoke_grant(&db, &grant.id, Some("admin"))
        .await
        .unwrap();
    assert_eq!(result.terminal_amount_micros, 100_000_000);
    assert_eq!(grants::used_micros(&result), 900_000_000);
    assert_eq!(
        saved(&db, &other.id).await,
        bson::from_document::<CreditGrant>(bson::to_document(&other).unwrap()).unwrap()
    );
    assert_eq!(
        db.collection::<BillingWallet>(WALLETS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        10_000
    );
}

#[tokio::test]
async fn summaries_include_unapplied_extra_commitments_without_double_counting_applied_locks() {
    let db = database("billing_summary_extra_commitment").await;
    setup(&db, "owner", 1).await;
    let ctx = route_context("owner");
    meter::open(&db, &ctx, None).await.unwrap();
    let usage = row(&db, &ctx.billing_request_id).await;
    db.collection::<UsageMeterRow>(USAGE)
        .update_one(
            doc! { "_id": &usage.id },
            doc! { "$set": { "funding": bson::to_bson(&UsageFunding::default()).unwrap() } },
        )
        .await
        .unwrap();
    let mut grant = allocation("owner", 100_000_000, Some(100_000_000));
    grant.scope = BillingServiceScope {
        all_services: false,
        service_ids: vec!["catalog-1".into()],
        service_slugs: vec!["service-one".into()],
    };
    grant.active_settlement = Some(CreditGrantSettlementLock {
        operation_id: "summary-extra".into(),
        usage_row_id: usage.id.clone(),
        reserved_micros: 0,
        consume_micros: 75_000_000,
        service_limit_key: Some("catalog-cap".into()),
        applied: false,
        updated_at: Utc::now(),
    });
    db.collection::<CreditGrant>(GRANTS)
        .insert_one(&grant)
        .await
        .unwrap();
    let before = saved(&db, &grant.id).await;
    let user = summaries::user_summary(&db, "owner", Utc::now())
        .await
        .unwrap();
    let admin = summaries::admin_summary(&db, Utc::now()).await.unwrap();
    assert_eq!(user.available_grant_micros, 25_000_000);
    assert_eq!(user.reserved_grant_micros, 75_000_000);
    assert_eq!(admin.available_micros, 25_000_000);
    assert_eq!(admin.reserved_micros, 75_000_000);
    assert_eq!(admin.outstanding_micros, 100_000_000);
    assert_eq!(
        service_limits::rules_with_commitments(&before)[0].reserved_micros,
        75_000_000
    );
    assert_eq!(
        before,
        saved(&db, &grant.id).await,
        "summaries never mutate counters"
    );
    let mut other_service = before.clone();
    other_service.service_limits.push(CreditServiceLimit {
        key: "other".into(),
        service_ids: vec!["catalog-2".into()],
        service_slugs: vec!["two".into()],
        label: None,
        limit_micros: 10_000_000,
        consumed_micros: 0,
        reserved_micros: 0,
    });
    assert_eq!(
        service_limits::available_for_service(&other_service, Some("catalog-2"), Some("two")),
        10_000_000
    );
    // Simulate the apply/record boundary. The remaining counter already includes
    // the charge, so the applied lock must not subtract it again.
    db.collection::<CreditGrant>(GRANTS).update_one(doc! { "_id": &grant.id }, doc! { "$set": {
        "remaining_micros": 25_000_000_i64, "service_limits.0.consumed_micros": 75_000_000_i64, "active_settlement.applied": true
    } }).await.unwrap();
    let user = summaries::user_summary(&db, "owner", Utc::now())
        .await
        .unwrap();
    let admin = summaries::admin_summary(&db, Utc::now()).await.unwrap();
    assert_eq!(user.available_grant_micros, 25_000_000);
    assert_eq!(user.reserved_grant_micros, 0);
    assert_eq!(admin.outstanding_micros, 25_000_000);
    assert_eq!(admin.available_micros, 25_000_000);
    grants::request_revoke_grant(&db, &grant.id, Some("admin"))
        .await
        .unwrap();
    assert_eq!(
        summaries::user_summary(&db, "owner", Utc::now())
            .await
            .unwrap()
            .available_grant_micros,
        0
    );
    funding::help_grant_settlement_lock(&db, &grant.id)
        .await
        .unwrap();
    let terminal = saved(&db, &grant.id).await;
    assert_eq!(terminal.terminal_amount_micros, 25_000_000);
    assert_eq!(terminal.service_limits[0].consumed_micros, 75_000_000);
}
