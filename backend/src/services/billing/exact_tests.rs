//! Issue #1672 regressions through the real MongoDB funding and journal paths.
use super::{lago_client::*, *};
use crate::models::{credit_grant::CreditGrant, credits::Credits, usage_meter::UsageMeterRow};
use async_trait::async_trait;
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, Document, doc};
struct Entitled;
#[async_trait]
impl LagoApi for Entitled {
    async fn ensure_customer(
        &self,
        input: &OwnerProvisionInput,
    ) -> crate::errors::AppResult<String> {
        Ok(input.external_customer_id.clone())
    }
    async fn ensure_subscription(
        &self,
        customer: &str,
        _: &str,
    ) -> crate::errors::AppResult<String> {
        Ok(customer.into())
    }
    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        Ok(LagoAck {
            transaction_id: event.transaction_id.clone(),
        })
    }
    async fn record_events_batch(&self, events: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        Ok(events
            .iter()
            .map(|e| LagoAck {
                transaction_id: e.transaction_id.clone(),
            })
            .collect())
    }
    async fn current_usage(
        &self,
        customer: &str,
        subscription: &str,
    ) -> crate::errors::AppResult<LagoUsage> {
        Ok(LagoUsage {
            customer_id: customer.into(),
            subscription_id: subscription.into(),
            raw: serde_json::json!({}),
        })
    }
    async fn wallet_balance(&self, _: &str) -> crate::errors::AppResult<i64> {
        Ok(0)
    }
    async fn entitlements(&self, _: &str) -> crate::errors::AppResult<Vec<Entitlement>> {
        Ok(vec![Entitlement {
            code: "*".into(),
            raw: serde_json::json!({}),
        }])
    }
}

async fn database(name: &str) -> mongodb::Database {
    ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    let db = crate::test_utils::connect_test_database(name)
        .await
        .unwrap();
    db.collection::<Document>("billing_ledger")
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "seq": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    db.collection::<Document>("billing_ledger")
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "postings.account": 1, "seq": 1 })
                .build(),
        )
        .await
        .unwrap();
    db.collection::<Document>("billing_migrations")
        .delete_many(doc! {})
        .await
        .unwrap();
    db
}

fn old_wallet(balance: i64) -> Document {
    let now = bson::DateTime::from_chrono(Utc::now());
    doc! {
        "_id": "wallet",
        "owner_id": "owner",
        "lago_customer_id": "owner",
        "lago_wallet_id": "lago-wallet",
        "lago_subscription_id": "subscription",
        "plan_kind": "prepaid",
        "balance_credits": balance,
        "reserved_credits": 0_i64,
        "pending_lago_debits": 0_i64,
        "pending_topup_expiry_credits": 0_i64,
        "overdraft_cap_credits": 0_i64,
        "has_payment_instrument": false,
        "suspended": false,
        "collection_state": "good",
        "balance_synced_at": now,
        "created_at": now,
        "updated_at": now,
    }
}

fn old_grant(micros: i64) -> Document {
    let now = bson::DateTime::from_chrono(Utc::now());
    doc! {
        "_id": "grant",
        "batch_id": "batch",
        "recipient_user_id": "owner",
        "target_kind": "selected_users",
        "amount_credits": 1_i64,
        "amount_micros": micros,
        "remaining_micros": micros,
        "reserved_micros": 0_i64,
        "scope": { "all_services": true, "service_ids": [], "service_slugs": [] },
        "granted_by": "admin",
        "status": "active",
        "created_at": now,
        "updated_at": now,
        "issued_ledgered_at": now,
    }
}

fn usage(id: &str) -> Document {
    let now = bson::DateTime::from_chrono(Utc::now());
    doc! {
        "_id": id,
        "transaction_id": id,
        "billing_request_id": id,
        "layer": "platform",
        "billing_owner_id": "owner",
        "wallet_id": "wallet",
        "actor_user_id": "owner",
        "metric": "cache_read_tokens",
        "lago_metric_code": "exact-cache",
        "credential_class": "user_owned",
        "reserved_credits": Credits::ZERO,
        "funding": { "credits_per_unit_micros": 0_i64, "credits_per_unit_pico": 800_000_i64 },
        "quantity": 1_i64,
        "status": "finalized",
        "forwarded": true,
        "released": false,
        "lago_acked": false,
        "attempt": 0_i32,
        "created_at": now,
        "updated_at": now,
    }
}

async fn assert_accounts(db: &mongodb::Database) {
    account_reconciliation::run_once(db).await.unwrap();
    let status = db
        .collection::<crate::models::chain_verify_status::ChainVerifyStatus>("chain_verify_status")
        .find_one(doc! { "_id": account_reconciliation::CHECK_ID })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        status.outcome,
        crate::models::chain_verify_status::ChainVerifyOutcome::Ok,
        "{:?}",
        status.break_detail
    );
}

#[tokio::test]
async fn account_scan_finishes_unequal_batches_without_certifying_deferred_accounts() {
    use crate::models::chain_verify_status::ChainVerifyStatus;
    let db = database("exact_account_scan_cycles").await;
    // Zero balances isolate cursor bookkeeping from journal movement checks.
    let wallets = (0..account_reconciliation::BATCH + 1)
        .map(|i| {
            let mut row = old_wallet(0);
            row.insert("exact_accounting_version", 2);
            row.insert("_id", format!("wallet-{i:05}"));
            row.insert("owner_id", format!("owner-{i:05}"));
            if i == 0 {
                row.insert("active_settlement", doc! {});
            }
            row
        })
        .collect::<Vec<_>>();
    let grants = (0..2 * account_reconciliation::BATCH + 1)
        .map(|i| {
            let mut row = old_grant(0);
            row.insert("_id", format!("grant-{i:05}"));
            row
        })
        .collect::<Vec<_>>();
    db.collection::<Document>("billing_wallet")
        .insert_many(wallets)
        .await
        .unwrap();
    db.collection::<Document>("credit_grants")
        .insert_many(grants)
        .await
        .unwrap();
    for _ in 0..3 {
        account_reconciliation::run_once(&db).await.unwrap();
    }
    let statuses = db.collection::<ChainVerifyStatus>("chain_verify_status");
    let status = statuses
        .find_one(doc! { "_id": account_reconciliation::CHECK_ID })
        .await
        .unwrap()
        .unwrap();
    assert!(status.last_full_pass_at.is_none());
    db.collection::<Document>("billing_wallet")
        .update_one(
            doc! { "_id": "wallet-00000" },
            doc! { "$unset": { "active_settlement": "" } },
        )
        .await
        .unwrap();
    for i in 0..3 {
        account_reconciliation::run_once(&db).await.unwrap();
        let status = statuses
            .find_one(doc! { "_id": account_reconciliation::CHECK_ID })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.last_full_pass_at.is_some(), i == 2);
    }
    // Provisioning publishes a wallet before its opening transaction. A sweep
    // in that window must defer it instead of reporting a broken account.
    let db = database("exact_account_pending_opening").await;
    db.collection::<Document>("billing_wallet")
        .insert_one(old_wallet(1))
        .await
        .unwrap();
    account_reconciliation::run_once(&db).await.unwrap();
    let status = db
        .collection::<ChainVerifyStatus>("chain_verify_status")
        .find_one(doc! { "_id": account_reconciliation::CHECK_ID })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        status.outcome,
        crate::models::chain_verify_status::ChainVerifyOutcome::Ok
    );
    assert!(status.last_full_pass_at.is_none());
    exact_migration::run(&db).await.unwrap();
    assert_accounts(&db).await;
}

#[tokio::test]
async fn sub_micro_grant_funds_zero_wallet_end_to_end_and_44608_tokens() {
    use crate::models::service_billing::{BillingMetric, PlatformUsage, ServiceBilling};
    for quantity in [1_i64, 44_608] {
        let db = database("exact_grant_regression").await;
        db.collection::<Document>("billing_wallet")
            .insert_one(old_wallet(0))
            .await
            .unwrap();
        db.collection::<Document>("credit_grants")
            .insert_one(old_grant(1_000_000))
            .await
            .unwrap();
        exact_migration::run(&db).await.unwrap();
        db.collection::<Document>("billing_rate_cache")
            .insert_one(doc! {
                "_id": "exact-cache:*",
                "lago_metric_code": "exact-cache",
                "model": null,
                "credits_per_unit_micros": 0_i64,
                "credits_per_unit_pico": 800_000_i64,
                "synced_at": bson::DateTime::from_chrono(Utc::now()),
            })
            .await
            .unwrap();
        let mut ctx = route_context::BillingRouteContext::new(
            route_inventory::BillingIngress::Proxy,
            uuid::Uuid::new_v4().to_string(),
            "owner".into(),
            "owner".into(),
            None,
            Some("service".into()),
            Some("service".into()),
            Some("service".into()),
            route_context::NodeIntent::Direct,
            "bearer".into(),
            crate::models::usage_meter::CredentialClass::UserOwned,
            BillingMetric::CacheReadTokens,
            Some(&ServiceBilling {
                platform_billable: true,
                ..Default::default()
            }),
            false,
        )
        .with_platform_metering(true);
        ctx.platform_lago_metric_code = "exact-cache".into();
        // The allowance exists but has no units left, as in the production report.
        let now = Utc::now();
        let allowance = crate::models::usage_allowance::UsageAllowance {
            id: "allowance".into(),
            bundle_id: None,
            service_id: "service".into(),
            service_slug: "service".into(),
            metric: BillingMetric::CacheReadTokens,
            quantity: 1,
            recurrence: crate::models::usage_allowance::AllowanceRecurrence::Daily,
            target_kind: crate::models::billing_target::BillingTargetKind::AllUsers,
            target_user_ids: vec![],
            target_org_ids: vec![],
            target_group_ids: vec![],
            is_active: true,
            created_by: "admin".into(),
            created_at: now,
            updated_at: now,
        };
        db.collection::<crate::models::usage_allowance::UsageAllowance>("usage_allowances")
            .insert_one(&allowance)
            .await
            .unwrap();
        let period = allowances::ensure_current_period(&db, &allowance, "owner", now)
            .await
            .unwrap();
        db.collection::<Document>("usage_allowance_periods")
            .update_one(
                doc! { "_id": period.id },
                doc! { "$set": { "consumed_quantity": 1_i64 } },
            )
            .await
            .unwrap();
        let reservation = reservation::gate_and_reserve(&db, Some(&Entitled), &ctx, false, 900)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reservation.total_reserved_credits, Credits::ZERO);
        let metered = meter::open(&db, &ctx, Some(&reservation)).await.unwrap();
        meter::mark_forwarded(&db, &metered).await.unwrap();
        let result = PlatformUsage {
            cache_read_tokens: quantity,
            ..Default::default()
        };
        meter::settle(&db, &metered, result.clone(), None, None)
            .await
            .unwrap();
        meter::settle(&db, &metered, result, None, None)
            .await
            .unwrap();
        let row = db
            .collection::<UsageMeterRow>("usage_meter")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        let funding = row.funding.unwrap();
        let cost = Credits::from_pico(800_000)
            .unwrap()
            .checked_mul(quantity)
            .unwrap();
        assert_eq!(funding.total_charge, Some(cost));
        assert_eq!(funding.grant_funded, Some(cost));
        assert_eq!(funding.wallet_funded, Some(Credits::ZERO));
        assert_eq!(funding.wallet_charge_credits, Some(Credits::ZERO));
        assert_eq!(funding.lago_billable_quantity_micros, Some(0));
        assert_eq!(
            Credits::checked_sum([
                funding.allowance_funded.unwrap(),
                funding.grant_funded.unwrap(),
                funding.wallet_funded.unwrap()
            ])
            .unwrap(),
            cost
        );
        let grant = db
            .collection::<CreditGrant>("credit_grants")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            grant.remaining,
            Credits::from_whole(1).checked_sub(cost).unwrap()
        );
        assert_eq!(
            db.collection::<Document>("billing_ledger")
                .count_documents(doc! { "movement": "usage_settled" })
                .await
                .unwrap(),
            0
        );
        assert_accounts(&db).await;
    }
}

#[tokio::test]
async fn migration_concurrent_idempotent_and_absorbs_late_micro_deltas() {
    let db = database("exact_migration").await;
    db.collection::<Document>("billing_wallet")
        .insert_one(old_wallet(7))
        .await
        .unwrap();
    db.collection::<Document>("credit_grants")
        .insert_one(old_grant(123_456))
        .await
        .unwrap();
    let (a, b) = tokio::join!(exact_migration::run(&db), exact_migration::run(&db));
    a.unwrap();
    b.unwrap();
    exact_migration::run(&db).await.unwrap();
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! { "movement": "opening_balance" })
            .await
            .unwrap(),
        2
    );
    let grant = db
        .collection::<Document>("credit_grants")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(!grant.contains_key("amount_micros"));
    assert_eq!(
        Credits::from_bson(grant["remaining"].clone(), 1_000_000)
            .unwrap()
            .to_string(),
        "0.123456"
    );
    db.collection::<Document>("credit_grants")
        .update_one(
            doc! { "_id": "grant" },
            doc! { "$inc": { "remaining_micros": -3_i64, "reserved_micros": 2_i64 } },
        )
        .await
        .unwrap();
    exact_migration::absorb_grant_deltas(&db, None)
        .await
        .unwrap();
    exact_migration::absorb_grant_deltas(&db, None)
        .await
        .unwrap();
    let grant = db
        .collection::<CreditGrant>("credit_grants")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(grant.remaining.to_string(), "0.123453");
    assert_eq!(grant.reserved.to_string(), "0.000002");
    assert_accounts(&db).await;
    // Legacy i64 wallet readers and required-micro grant readers fail closed.
    #[derive(serde::Deserialize)]
    struct OldWallet {
        #[serde(rename = "balance_credits")]
        _balance: i64,
    }
    #[derive(serde::Deserialize)]
    struct OldGrant {
        #[serde(rename = "amount_micros")]
        _amount: i64,
    }
    let wallet = db
        .collection::<Document>("billing_wallet")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(bson::from_document::<OldWallet>(wallet).is_err());
    assert!(bson::from_document::<OldGrant>(bson::to_document(&grant).unwrap()).is_err());
}

#[tokio::test]
async fn billing_readiness_does_not_wait_for_rollup_normalization() {
    let db = database("billing_before_rollups").await;
    db.collection::<Document>("billing_wallet")
        .insert_one(old_wallet(10))
        .await
        .unwrap();
    // A derived document that cannot normalize must never hold money cutover.
    db.collection::<Document>(crate::models::usage_rollup_hourly::COLLECTION_NAME)
        .insert_one(doc! { "_id": "bad-analytics", "cost_partitions": "malformed" })
        .await
        .unwrap();
    exact_migration::start(&db, true).await.unwrap();
    exact_migration::require_ready(&db).await.unwrap();
    assert!(!exact_migration::rollup_ready(&db).await.unwrap());
    assert!(exact_migration::run(&db).await.is_err());
    exact_migration::require_ready(&db).await.unwrap();
    let diagnostics = startup_diagnostics(&db).await.unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "billing_rollup_normalization");
    db.collection::<Document>(crate::models::usage_rollup_hourly::COLLECTION_NAME)
        .update_one(
            doc! { "_id": "bad-analytics" },
            doc! { "$set": { "cost_partitions": {} } },
        )
        .await
        .unwrap();
    exact_migration::run(&db).await.unwrap();
    assert!(exact_migration::rollup_ready(&db).await.unwrap());
    assert!(startup_diagnostics(&db).await.unwrap().is_empty());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn rollup_normalization_interleaves_with_fold_without_losing_increment() {
    for normalize_first in [false, true] {
        let db = database("exact_rollup_interleaving").await;
        usage_rollup::ensure_indexes(&db).await.unwrap();
        let end = usage_rollup::hour(Utc::now());
        let make = |id: &str| {
            let mut row = usage(id);
            row.insert("transaction_id", format!("{id}:platform"));
            row.insert(
                "created_at",
                bson::DateTime::from_chrono(end - chrono::Duration::hours(2)),
            );
            row.insert("released", true);
            row.insert("lago_acked", true);
            row.insert(
                "token_breakdown",
                doc! { "prompt_tokens": 3_i64, "completion_tokens": 2_i64 },
            );
            row.insert(
                "funding",
                doc! {
                    "settled": true, "total_charge": Credits::from_micros(1),
                    "wallet_funded": Credits::from_micros(1), "grant_funded": Credits::ZERO,
                    "allowance_funded": Credits::ZERO,
                },
            );
            row
        };
        let meters = db.collection::<Document>("usage_meter");
        meters.insert_one(make("first")).await.unwrap();
        while usage_rollup::fold_once(&db, end).await.unwrap() > 0 {}
        let mut selected = Vec::new();
        for collection in [
            crate::models::usage_rollup_hourly::COLLECTION_NAME,
            crate::models::usage_rollup_daily::COLLECTION_NAME,
        ] {
            let rollups = db.collection::<Document>(collection);
            rollups.update_many(doc! {}, doc! {
                "$unset": { "gross_cost": "", "wallet_cost": "" },
                "$set": {
                    "gross_cost_micros": 1_i64, "wallet_cost_micros": 1_i64,
                    "query_costs": { "gross_cost_micros": 1_i64, "wallet_cost_micros": 1_i64 },
                },
            }).await.unwrap();
            let row = rollups.find_one(doc! {}).await.unwrap().unwrap();
            selected.push((collection, vec![row.get_str("_id").unwrap().to_owned()]));
        }
        // Deterministically place a real fold after the normalizer's ID read,
        // testing both orders of the atomic write and the concurrent increment.
        if normalize_first {
            for (collection, ids) in &selected {
                exact_migration::normalize_rollup_batch(&db, collection, ids)
                    .await
                    .unwrap();
            }
        }
        meters.insert_one(make("second")).await.unwrap();
        while usage_rollup::fold_once(&db, end).await.unwrap() > 0 {}
        for (collection, ids) in &selected {
            exact_migration::normalize_rollup_batch(&db, collection, ids)
                .await
                .unwrap();
            // Replay is idempotent and must not rescale an exact root fallback.
            exact_migration::normalize_rollup_batch(&db, collection, ids)
                .await
                .unwrap();
            let row = db
                .collection::<Document>(collection)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            for value in [
                row["gross_cost"].clone(),
                row["query_gross_cost"].clone(),
                row.get_document("query_costs").unwrap()["gross_cost"].clone(),
            ] {
                assert_eq!(
                    Credits::from_bson(value, crate::models::credits::SCALE).unwrap(),
                    Credits::from_micros(2)
                );
            }
            assert_eq!(row.get_i64("quantity").unwrap(), 2);
            assert_eq!(row.get_i64("prompt_tokens").unwrap(), 6);
            assert_eq!(row.get_i64("completion_tokens").unwrap(), 4);
            assert!(!row.contains_key("gross_cost_micros"));
        }
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn concurrent_carry_assignments_and_retries_conserve_quantity() {
    let db = database("exact_carry_concurrent").await;
    let mut tasks = Vec::new();
    for index in 0..32 {
        let doc = usage(&format!("row-{index}"));
        db.collection::<Document>("usage_meter")
            .insert_one(&doc)
            .await
            .unwrap();
        let row: UsageMeterRow = bson::from_document(doc).unwrap();
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            let q = lago_carry::assign(
                &db,
                &row,
                Credits::from_pico(1).unwrap(),
                Credits::from_pico(3).unwrap(),
            )
            .await
            .unwrap();
            let retry = lago_carry::assign(
                &db,
                &row,
                Credits::from_pico(1).unwrap(),
                Credits::from_pico(3).unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(q, retry);
            q
        }));
    }
    let mut total = 0_i64;
    for task in tasks {
        total += task.await.unwrap();
    }
    assert_eq!(total, 32_000_000 / 3);
    let carry = db
        .collection::<crate::models::billing_lago_carry::BillingLagoCarry>("billing_lago_carry")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(carry.wallet_total.pico(), 32);
    assert_eq!(carry.remainder_numerator, (32_000_000 % 3).to_string());
}

#[tokio::test]
async fn v2_postings_balance_commit_every_field_and_reconciliation_detects_corruption() {
    use crate::models::billing_ledger::{BillingLedgerEntry, PostingSide};
    let db = database("exact_v2_integrity").await;
    db.collection::<Document>("billing_wallet")
        .insert_one(old_wallet(7))
        .await
        .unwrap();
    exact_migration::run(&db).await.unwrap();
    webhook::refresh_wallet_balance(&db, "owner", "7.12345".parse().unwrap())
        .await
        .unwrap();
    assert_accounts(&db).await;
    let entries: Vec<BillingLedgerEntry> = db
        .collection("billing_ledger")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    for entry in entries {
        ledger::validate_postings(&entry).unwrap();
        for field in 0..3 {
            let mut changed = entry.clone();
            match field {
                0 => changed.postings[0].account.push('x'),
                1 => changed.postings[0].side = PostingSide::Credit,
                _ => changed.postings[0].amount = Credits::from_pico(1).unwrap(),
            }
            assert_ne!(
                ledger::compute_entry_hash(&changed, &ledger::TEST_BILLING_LEDGER_HMAC_KEY),
                entry.entry_hash
            );
        }
    }
    let report = ledger::verify_chain(&db, &ledger::TEST_BILLING_LEDGER_HMAC_KEY, None, None, None)
        .await
        .unwrap();
    assert!(report.break_info.is_none());
    db.collection::<Document>("billing_wallet")
        .update_one(
            doc! {},
            doc! { "$inc": { "balance_credits": Credits::from_pico(1).unwrap() } },
        )
        .await
        .unwrap();
    account_reconciliation::run_once(&db).await.unwrap();
    let status = db
        .collection::<Document>("chain_verify_status")
        .find_one(doc! { "_id": account_reconciliation::CHECK_ID })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.get_str("outcome").unwrap(), "broken");
    db.collection::<Document>("billing_ledger")
        .update_one(
            doc! { "seq": 1_i64 },
            doc! { "$set": { "postings.0.amount": Credits::from_pico(1).unwrap() } },
        )
        .await
        .unwrap();
    let report = ledger::verify_chain(&db, &ledger::TEST_BILLING_LEDGER_HMAC_KEY, None, None, None)
        .await
        .unwrap();
    assert!(report.break_info.is_some());
}

#[tokio::test]
async fn mixed_grant_wallet_fraction_and_legacy_unfunded_row_recovery() {
    let db = database("exact_mixed_and_legacy").await;
    db.collection::<Document>("billing_wallet")
        .insert_one(old_wallet(1))
        .await
        .unwrap();
    db.collection::<Document>("credit_grants")
        .insert_one(old_grant(1))
        .await
        .unwrap();
    exact_migration::run(&db).await.unwrap();
    let mut document = usage("mixed");
    document.insert("quantity", 2_i64);
    db.collection::<Document>("usage_meter")
        .insert_one(&document)
        .await
        .unwrap();
    let row: UsageMeterRow = bson::from_document(document).unwrap();
    reservation::claim_released_and_settle(&db, &row)
        .await
        .unwrap();
    reservation::claim_released_and_settle(&db, &row)
        .await
        .unwrap();
    let saved = db
        .collection::<UsageMeterRow>("usage_meter")
        .find_one(doc! { "_id": "mixed" })
        .await
        .unwrap()
        .unwrap();
    let funded = saved.funding.unwrap();
    assert_eq!(funded.total_charge.unwrap().to_string(), "0.0000016");
    assert_eq!(funded.grant_funded.unwrap().to_string(), "0.000001");
    assert_eq!(funded.wallet_funded.unwrap().to_string(), "0.0000006");
    let wallet = db
        .collection::<crate::models::billing_wallet::BillingWallet>("billing_wallet")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wallet.pending_lago_debits.to_string(), "0.0000006");
    assert_accounts(&db).await;
    // Two openings, one grant consumption and one wallet debit: the append
    // count per charged row is unchanged from v1, including replay.
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        4
    );
    let mut legacy = usage("legacy");
    legacy.remove("funding");
    legacy.insert("reserved_credits", 0_i64);
    db.collection::<Document>("usage_meter")
        .insert_one(&legacy)
        .await
        .unwrap();
    let rate = crate::models::billing_rate_cache::BillingRateCache {
        id: crate::models::billing_rate_cache::BillingRateCache::cache_id("exact-cache", None),
        lago_metric_code: "exact-cache".into(),
        model: None,
        credits_per_unit_micros: 0,
        credits_per_unit_pico: Some(800_000),
        synced_at: Utc::now(),
        retired_at: None,
    };
    db.collection::<crate::models::billing_rate_cache::BillingRateCache>("billing_rate_cache")
        .insert_one(rate)
        .await
        .unwrap();
    let row: UsageMeterRow = bson::from_document(legacy).unwrap();
    reservation::claim_released_and_settle(&db, &row)
        .await
        .unwrap();
    let saved = db
        .collection::<UsageMeterRow>("usage_meter")
        .find_one(doc! { "_id": "legacy" })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved
            .funding
            .unwrap()
            .wallet_charge_credits
            .unwrap()
            .to_string(),
        "0.0000008"
    );
    assert_accounts(&db).await;
}

#[tokio::test]
async fn migration_applies_legacy_grant_lock_once_and_recovers_allocation() {
    for applied in [false, true] {
        let db = database("exact_legacy_grant_lock").await;
        let operation = "locked:grant:grant";
        let mut grant = old_grant(if applied { 7 } else { 10 });
        grant.insert("reserved_micros", if applied { 0_i64 } else { 3_i64 });
        grant.insert(
            "active_settlement",
            doc! {
                "operation_id": operation,
                "usage_row_id": "locked",
                "reserved_micros": 3_i64,
                "consume_micros": 3_i64,
                "applied": applied,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            },
        );
        db.collection::<Document>("credit_grants")
            .insert_one(grant)
            .await
            .unwrap();
        let mut document = usage("locked");
        document.get_document_mut("funding").unwrap().insert(
            "grant_reservations",
            vec![doc! { "grant_id": "grant", "amount_micros": 3_i64 }],
        );
        document
            .get_document_mut("funding")
            .unwrap()
            .insert("credits_per_unit_pico", 3_000_000_i64);
        db.collection::<Document>("usage_meter")
            .insert_one(&document)
            .await
            .unwrap();
        exact_migration::run(&db).await.unwrap();
        let migrated = db
            .collection::<CreditGrant>("credit_grants")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(migrated.remaining, Credits::from_micros(7));
        assert_eq!(migrated.reserved, Credits::ZERO);
        assert!(migrated.active_settlement.unwrap().applied);
        let row: UsageMeterRow = bson::from_document(document).unwrap();
        funding::settle_usage_funding(&db, &row).await.unwrap();
        let saved = db
            .collection::<UsageMeterRow>("usage_meter")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.funding.unwrap().grant_funded,
            Some(Credits::from_micros(3))
        );
        assert_accounts(&db).await;
    }
}

#[tokio::test]
async fn old_actual_append_verify_and_sweep_fail_closed_on_v2() {
    let db = database("exact_old_verifier").await;
    db.collection::<Document>("billing_wallet")
        .insert_one(old_wallet(1))
        .await
        .unwrap();
    // Startup refuses existing-data cutover until old writers have drained.
    // Previously startup errored. It now serves nonbilling traffic with a
    // durable pending status; billed admission alone returns typed 503.
    exact_migration::start(&db, false).await.unwrap();
    assert!(!exact_migration::ready(&db).await.unwrap());
    assert!(matches!(
        exact_migration::require_ready(&db).await,
        Err(crate::errors::AppError::BillingProviderUnavailable(_))
    ));
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    exact_migration::start(&db, true).await.unwrap();
    exact_migration::start(&db, false).await.unwrap();
    let mut old = db
        .collection::<Document>("billing_ledger")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    old.insert("event_type", "usage_settled");
    old.insert("amount_credits", 1_i64);
    old.insert("dedupe_key", "old-inflight");
    let old: legacy_v1_fixture::model::BillingLedgerEntry = bson::from_document(old).unwrap();
    // append_chained_entry executes the frozen old read_tail, which must reject
    // the v2 event variant before attempting a new append.
    assert!(
        legacy_v1_fixture::ledger::append_chained_entry(
            &db,
            old,
            &ledger::TEST_BILLING_LEDGER_HMAC_KEY
        )
        .await
        .is_err()
    );
    assert!(
        legacy_v1_fixture::ledger::verify_chain(
            &db,
            &ledger::TEST_BILLING_LEDGER_HMAC_KEY,
            None,
            None,
            None
        )
        .await
        .is_err()
    );
    assert!(
        legacy_v1_fixture::verification::run_once(
            &db,
            &[3; 32],
            &ledger::TEST_BILLING_LEDGER_HMAC_KEY
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.collection::<Document>("chain_verify_status")
            .count_documents(doc! { "_id": "billing_ledger", "outcome": "broken" })
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn concurrent_journal_retries_do_not_add_entries_with_nonunique_dedupe_index() {
    use crate::models::billing_ledger::BillingLedgerEntry;
    let db = database("exact_journal_concurrency").await;
    // Older installations can retain a nonunique dedupe lookup after historic
    // duplicates. Sequence serialization must still make new postings unique.
    let started = std::time::Instant::now();
    let mut tasks = Vec::new();
    for id in 0..16 {
        for _ in 0..2 {
            let db = db.clone();
            tasks.push(tokio::spawn(async move {
                let entry = ledger::exact_entry(
                    "owner",
                    &id.to_string(),
                    "usage_settled",
                    format!("usage-settled:{id}"),
                    ledger::transfer(
                        format!("usage:{id}"),
                        "wallet:owner".into(),
                        Credits::from_pico(1).unwrap(),
                    ),
                    None,
                );
                ledger::append_chained_entry(&db, entry, &ledger::TEST_BILLING_LEDGER_HMAC_KEY)
                    .await
                    .unwrap();
            }));
        }
    }
    for task in tasks {
        task.await.unwrap();
    }
    let entries: Vec<BillingLedgerEntry> = db
        .collection("billing_ledger")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(entries.len(), 16);
    for entry in entries {
        ledger::validate_postings(&entry).unwrap();
    }
    assert!(
        ledger::verify_chain(&db, &ledger::TEST_BILLING_LEDGER_HMAC_KEY, None, None, None)
            .await
            .unwrap()
            .break_info
            .is_none()
    );
    eprintln!(
        "v2 append measurement: 16 charged rows, 32 attempts, 16 durable entries in {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn mixed_account_lifecycle_balances_after_issue_hold_release_expire_revoke_and_topup() {
    use crate::models::billing_target::BillingTargetKind;
    let db = database("exact_account_lifecycle").await;
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(crate::test_utils::test_user(
            &owner,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let mut wallet = old_wallet(10);
    wallet.insert("owner_id", &owner);
    wallet.insert("lago_customer_id", &owner);
    db.collection::<Document>("billing_wallet")
        .insert_one(wallet)
        .await
        .unwrap();
    exact_migration::run(&db).await.unwrap();
    let input = grants::IssueCreditGrantInput {
        amount_credits: 1,
        target_kind: BillingTargetKind::SelectedUsers,
        target_user_ids: vec![owner.clone()],
        target_org_ids: vec![],
        target_group_ids: vec![],
        all_services: true,
        service_refs: vec![],
        expires_at: Some(Utc::now() + chrono::Duration::days(1)),
        reason: None,
        granted_by: "admin".into(),
    };
    let grant = grants::issue_grants(&db, input.clone())
        .await
        .unwrap()
        .remove(0);
    assert_accounts(&db).await;
    let mut row = usage("lifecycle");
    row.insert("billing_owner_id", &owner);
    row.insert("actor_user_id", &owner);
    db.collection::<Document>("usage_meter")
        .insert_one(&row)
        .await
        .unwrap();
    reservation::claim_released_and_settle(&db, &bson::from_document(row).unwrap())
        .await
        .unwrap();
    assert_accounts(&db).await;
    // A wallet hold is availability, not a balance movement; exercise the real
    // reservation/release primitives around an already-journaled partial grant.
    let hold: Credits = "0.0000007".parse().unwrap();
    assert!(
        reservation::try_reserve_prepaid(&db, &owner, hold)
            .await
            .unwrap()
            .is_some()
    );
    assert_accounts(&db).await;
    reservation::release_wallet_hold(&db, &owner, hold)
        .await
        .unwrap();
    assert_accounts(&db).await;
    grants::expire_due_grants(&db, Utc::now() + chrono::Duration::days(2))
        .await
        .unwrap();
    assert_accounts(&db).await;
    let expired = db
        .collection::<CreditGrant>("credit_grants")
        .find_one(doc! { "_id": grant.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(expired.terminal_amount.to_string(), "0.9999992");
    let grant = grants::issue_grants(&db, input).await.unwrap().remove(0);
    grants::revoke_grant(&db, &grant.id).await.unwrap();
    assert_accounts(&db).await;
    webhook::refresh_wallet_balance(&db, &owner, "10.12345".parse().unwrap())
        .await
        .unwrap();
    assert_accounts(&db).await;
    let now = Utc::now();
    let operation = crate::models::billing_wallet::PurchasedCreditExpiryOperation {
        operation_id: "expiry".into(),
        processing_token: "worker".into(),
        lease_until: now + chrono::Duration::minutes(1),
        amount: "0.12345".parse().unwrap(),
        items: vec![crate::models::billing_wallet::PurchasedCreditExpiryItem {
            lago_purchase_transaction_id: "purchase".into(),
            reference_id: "purchase".into(),
            amount: "0.12345".parse().unwrap(),
            settled_at: now - chrono::Duration::days(366),
        }],
        lago_void_transaction_id: Some("void".into()),
        wallet_balance_applied: false,
        history_applied: false,
        created_at: now,
        updated_at: now,
    };
    db.collection::<Document>("billing_wallet")
        .update_one(
            doc! {},
            doc! { "$set": { "active_topup_expiry": bson::to_bson(&operation).unwrap() } },
        )
        .await
        .unwrap();
    webhook::apply_balance(&db, "wallet", Credits::from_whole(10), Some(&operation))
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>("billing_wallet")
        .update_one(doc! {}, doc! { "$unset": { "active_topup_expiry": "" } })
        .await
        .unwrap();
    assert_accounts(&db).await;
}

#[tokio::test]
async fn randomized_real_settlements_conserve_funding_and_account_balances() {
    use rand::{Rng, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(1672);
    for iteration in 0..24 {
        let db = database("exact_randomized_settlement").await;
        let quantity = rng.gen_range(1_i64..=1_000);
        let rate = [1_i64, 800_000, 1_000_001, 1_000_000_000_000][iteration % 4];
        let total = Credits::from_pico(i128::from(rate))
            .unwrap()
            .checked_mul(quantity)
            .unwrap();
        let grant_amount = Credits::from_pico(rng.gen_range(0..=total.pico())).unwrap();
        let allowed = rng.gen_range(0..=quantity);
        db.collection::<Document>("billing_wallet")
            .insert_one(old_wallet(if iteration % 2 == 0 { 0 } else { 1000 }))
            .await
            .unwrap();
        let mut grant = old_grant(0);
        grant.insert("amount_micros", grant_amount);
        grant.insert("remaining_micros", grant_amount);
        db.collection::<Document>("credit_grants")
            .insert_one(grant)
            .await
            .unwrap();
        exact_migration::run(&db).await.unwrap();
        let now = bson::DateTime::from_chrono(Utc::now());
        if allowed > 0 {
            db.collection::<Document>("usage_allowances")
                .insert_one(doc! {
                    "_id": "allowance",
                    "service_id": "service",
                    "service_slug": "service",
                    "metric": "cache_read_tokens",
                    "quantity": allowed,
                    "recurrence": "daily",
                    "target_kind": "all_users",
                    "target_user_ids": [],
                    "is_active": true,
                    "created_by": "admin",
                    "created_at": now,
                    "updated_at": now,
                })
                .await
                .unwrap();
        }
        let mut document = usage("randomized");
        document.insert("quantity", quantity);
        document.insert("service_id", "service");
        document.insert("service_slug", "service");
        document
            .get_document_mut("funding")
            .unwrap()
            .insert("credits_per_unit_pico", rate);
        db.collection::<Document>("usage_meter")
            .insert_one(&document)
            .await
            .unwrap();
        reservation::claim_released_and_settle(&db, &bson::from_document(document).unwrap())
            .await
            .unwrap();
        let row = db
            .collection::<UsageMeterRow>("usage_meter")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        let funding = row.funding.unwrap();
        let allowance = Credits::from_pico(i128::from(rate))
            .unwrap()
            .checked_mul(allowed)
            .unwrap();
        let grant = grant_amount.min(total.checked_sub(allowance).unwrap());
        let wallet = total
            .checked_sub(allowance)
            .unwrap()
            .checked_sub(grant)
            .unwrap();
        assert_eq!(funding.total_charge, Some(total));
        assert_eq!(funding.allowance_funded, Some(allowance));
        assert_eq!(funding.grant_funded, Some(grant));
        assert_eq!(funding.wallet_charge_credits, Some(wallet));
        assert_eq!(
            Credits::checked_sum([allowance, grant, wallet]).unwrap(),
            total
        );
        assert_accounts(&db).await;
    }
}

#[tokio::test]
async fn metering_only_and_legacy_wallet_rows_never_consume_benefits() {
    let db = database("exact_unfunded_semantics").await;
    db.collection::<Document>("billing_wallet")
        .insert_one(old_wallet(10))
        .await
        .unwrap();
    let mut grant = old_grant(1_000_000);
    grant.insert("target_kind", "all_users");
    db.collection::<Document>("credit_grants")
        .insert_one(grant)
        .await
        .unwrap();
    exact_migration::run(&db).await.unwrap();
    let now = bson::DateTime::now();
    db.collection::<Document>("usage_allowances")
        .insert_one(doc! {
            "_id": "free-benefit",
            "service_id": "service",
            "service_slug": "service",
            "metric": "cache_read_tokens",
            "quantity": 100_i64,
            "recurrence": "daily",
            "target_kind": "all_users",
            "is_active": true,
            "created_by": "admin",
            "created_at": now,
            "updated_at": now,
        })
        .await
        .unwrap();
    db.collection::<Document>("billing_rate_cache")
        .insert_one(doc! {
            "_id": "exact-cache:*",
            "lago_metric_code": "exact-cache",
            "model": null,
            "credits_per_unit_micros": 0_i64,
            "credits_per_unit_pico": 800_000_i64,
            "synced_at": now,
        })
        .await
        .unwrap();
    let before = db
        .collection::<Document>("credit_grants")
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    for (id, wallet) in [("meter-only", false), ("legacy-wallet", true)] {
        let mut row = usage(id);
        row.remove("funding");
        row.insert("service_id", "service");
        row.insert("service_slug", "service");
        if !wallet {
            row.remove("wallet_id");
        }
        db.collection::<Document>("usage_meter")
            .insert_one(&row)
            .await
            .unwrap();
        let row = bson::from_document(row).unwrap();
        reservation::claim_released_and_settle(&db, &row)
            .await
            .unwrap();
        let saved = db
            .collection::<UsageMeterRow>("usage_meter")
            .find_one(doc! { "_id": id })
            .await
            .unwrap()
            .unwrap();
        assert!(saved.released);
        if wallet {
            assert_eq!(
                saved.funding.unwrap().wallet_funded.unwrap().to_string(),
                "0.0000008"
            );
        } else {
            assert!(saved.funding.is_none());
        }
    }
    assert_eq!(
        db.collection::<Document>("credit_grants")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(
        db.collection::<Document>("usage_allowance_periods")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_accounts(&db).await;
}

#[tokio::test]
async fn aborted_carry_and_split_retry_with_new_grant_conserve_rational_residue() {
    let db = database("exact_atomic_carry").await;
    db.collection::<Document>("billing_wallet")
        .insert_one(old_wallet(1))
        .await
        .unwrap();
    exact_migration::run(&db).await.unwrap();
    let mut row = usage("atomic");
    row.get_document_mut("funding")
        .unwrap()
        .insert("credits_per_unit_pico", 3_i64);
    db.collection::<Document>("usage_meter")
        .insert_one(&row)
        .await
        .unwrap();
    // Inject failure at the final row write, after the carry update in the same
    // transaction. The provider quantity must not survive this aborted commit.
    db.run_command(
        doc! { "collMod": "usage_meter", "validator": { "funding.settled": { "$ne": true } } },
    )
    .await
    .unwrap();
    let row: UsageMeterRow = bson::from_document(row).unwrap();
    assert!(funding::settle_usage_funding(&db, &row).await.is_err());
    assert_eq!(
        db.collection::<Document>("billing_lago_carry")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let mut grant = old_grant(0);
    grant.insert("amount_micros", Credits::from_pico(1).unwrap());
    grant.insert("remaining_micros", Credits::from_pico(1).unwrap());
    db.collection::<Document>("credit_grants")
        .insert_one(grant)
        .await
        .unwrap();
    exact_migration::migrate_one(&db, "credit_grants", "grant")
        .await
        .unwrap();
    db.run_command(doc! { "collMod": "usage_meter", "validator": { } })
        .await
        .unwrap();
    db.collection::<Document>("usage_meter")
        .update_one(
            doc! { "_id": "atomic" },
            doc! { "$unset": { "funding.settlement_claim_id": "", "funding.settlement_claimed_at": "" } , },
        )
        .await
        .unwrap();
    reservation::claim_released_and_settle(&db, &row)
        .await
        .unwrap();
    let saved = db
        .collection::<UsageMeterRow>("usage_meter")
        .find_one(doc! { "_id": "atomic" })
        .await
        .unwrap()
        .unwrap();
    let funding = saved.funding.unwrap();
    let wallet = funding.wallet_funded.unwrap();
    assert_eq!(wallet.pico(), 2);
    assert_eq!(funding.grant_funded.unwrap().pico(), 1);
    let carry = db
        .collection::<crate::models::billing_lago_carry::BillingLagoCarry>("billing_lago_carry")
        .find_one(doc! { "_id": funding.lago_carry_id.unwrap() })
        .await
        .unwrap()
        .unwrap();
    let q = i128::from(funding.lago_billable_quantity_micros.unwrap());
    let remainder: i128 = carry.remainder_numerator.parse().unwrap();
    assert_eq!(q * 3 + remainder, wallet.pico() * 1_000_000);
    assert_eq!(carry.wallet_total, wallet);
    // A stale worker's attempted three-pico share cannot replace the committed
    // two-pico share, even when its retry observes the newer worker's commit.
    let replay = lago_carry::commit_settlement(
        &db,
        &row,
        Credits::from_pico(3).unwrap(),
        Credits::from_pico(3).unwrap(),
        doc! {},
    )
    .await
    .unwrap();
    assert_eq!(replay.wallet_charge_credits, wallet);
    assert_eq!(i128::from(replay.lago_billable_quantity_micros), q);
    assert_accounts(&db).await;
}

#[tokio::test]
async fn migration_isolates_bad_documents_and_skips_zero_openings() {
    let db = database("exact_isolated_cutover").await;
    let mut bad = old_wallet(1);
    bad.insert("_id", "bad");
    bad.insert("owner_id", "bad-owner");
    bad.insert("balance_credits", "invalid");
    db.collection::<Document>("billing_wallet")
        .insert_many([bad, old_wallet(0)])
        .await
        .unwrap();
    let grants = (0..150)
        .map(|i| {
            let mut grant = old_grant(0);
            grant.insert("_id", format!("empty-{i}"));
            grant.insert("terminal_amount_micros", 0_i64);
            grant
        })
        .collect::<Vec<_>>();
    db.collection::<Document>("credit_grants")
        .insert_many(grants)
        .await
        .unwrap();
    exact_migration::start(&db, false).await.unwrap();
    assert!(!exact_migration::ready(&db).await.unwrap());
    // UI reads still decode legacy amounts while cutover is pending.
    assert_eq!(
        provisioning::get_wallet(&db, "owner")
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        Credits::ZERO
    );
    exact_migration::start(&db, true).await.unwrap();
    assert!(!exact_migration::ready(&db).await.unwrap());
    assert_eq!(
        db.collection::<Document>("billing_migration_errors")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.collection::<Document>("credit_grants")
            .count_documents(doc! { "amount": { "$type": "decimal" } })
            .await
            .unwrap(),
        150
    );
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.collection::<Document>("billing_wallet")
        .update_one(
            doc! { "_id": "bad" },
            doc! { "$set": { "balance_credits": 1_i64 } },
        )
        .await
        .unwrap();
    exact_migration::start(&db, false).await.unwrap();
    assert!(exact_migration::ready(&db).await.unwrap());
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn cutover_isolates_malformed_deltas_and_retries_without_reposting() {
    let db = database("exact_isolated_deltas").await;
    let grants = db.collection::<Document>("credit_grants");
    for id in ["a-bad", "z-good"] {
        let mut grant = old_grant(0);
        grant.insert("_id", id);
        grants.insert_one(grant).await.unwrap();
        exact_migration::migrate_one(&db, "credit_grants", id)
            .await
            .unwrap();
    }
    grants
        .update_one(
            doc! { "_id": "a-bad" },
            doc! { "$set": { "remaining_micros": "invalid" } },
        )
        .await
        .unwrap();
    grants
        .update_one(
            doc! { "_id": "z-good" },
            doc! { "$set": { "remaining_micros": 1_i64 } },
        )
        .await
        .unwrap();
    exact_migration::start(&db, true).await.unwrap();
    assert!(!exact_migration::ready(&db).await.unwrap());
    let errors = db.collection::<Document>("billing_migration_errors");
    assert_eq!(errors.count_documents(doc! {}).await.unwrap(), 1);
    let good = db
        .collection::<CreditGrant>("credit_grants")
        .find_one(doc! { "_id": "z-good" })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(good.remaining, Credits::from_micros(1));
    let journal = db.collection::<Document>("billing_ledger");
    assert_eq!(journal.count_documents(doc! {}).await.unwrap(), 1);
    grants
        .update_one(
            doc! { "_id": "a-bad" },
            doc! { "$set": { "remaining_micros": 2_i64 } },
        )
        .await
        .unwrap();
    exact_migration::start(&db, false).await.unwrap();
    assert!(exact_migration::ready(&db).await.unwrap());
    assert_eq!(errors.count_documents(doc! {}).await.unwrap(), 0);
    assert_eq!(journal.count_documents(doc! {}).await.unwrap(), 2);
    exact_migration::start(&db, false).await.unwrap();
    assert_eq!(journal.count_documents(doc! {}).await.unwrap(), 2);
    assert_accounts(&db).await;
}

#[tokio::test]
async fn two_hundred_concurrent_wallet_settlements_leave_no_locks() {
    let db = database("exact_concurrent_wallets").await;
    let mut wallets = Vec::new();
    let mut rows = Vec::new();
    for i in 0..200 {
        let id = format!("concurrent-{i}");
        let mut wallet = old_wallet(0);
        wallet.insert("_id", &id);
        wallet.insert("owner_id", &id);
        wallets.push(wallet);
        let mut row = usage(&id);
        row.insert("wallet_id", &id);
        row.insert("billing_owner_id", &id);
        rows.push(row);
    }
    db.collection::<Document>("billing_wallet")
        .insert_many(wallets)
        .await
        .unwrap();
    exact_migration::run(&db).await.unwrap();
    db.collection::<Document>("usage_meter")
        .insert_many(&rows)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let mut tasks = Vec::new();
    for row in rows {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            let row = bson::from_document(row).unwrap();
            reservation::claim_released_and_settle(&db, &row)
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    println!(
        "200-wallet settlement burst: {:.2} appends/s ({:.3}s)",
        200.0 / started.elapsed().as_secs_f64(),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(
        db.collection::<Document>("billing_wallet")
            .count_documents(doc! { "active_settlement": { "$type": "object" } })
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>("usage_meter")
            .count_documents(doc! { "released": true, "status": "finalized" })
            .await
            .unwrap(),
        200
    );
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! { "movement": "usage_settled" })
            .await
            .unwrap(),
        200
    );
    assert_eq!(
        db.collection::<Document>("billing_account_balances")
            .count_documents(doc! {})
            .await
            .unwrap(),
        200,
        "journal-only usage and external accounts must never get checkpoints"
    );
    assert_accounts(&db).await;
}

#[tokio::test]
async fn account_check_uses_constant_work_after_many_postings() {
    let db = database("exact_bounded_account_check").await;
    db.collection::<Document>("billing_wallet")
        .insert_one(old_wallet(0))
        .await
        .unwrap();
    exact_migration::run(&db).await.unwrap();
    let mut first_checkpoint = None;
    for i in 0..1_000 {
        let entry = ledger::exact_entry(
            "owner",
            "wallet",
            "wallet_adjusted",
            format!("many:{i}"),
            ledger::transfer(
                "wallet:owner".into(),
                "external:lago".into(),
                Credits::from_pico(1).unwrap(),
            ),
            None,
        );
        ledger::append_chained_entry(&db, entry, &ledger::TEST_BILLING_LEDGER_HMAC_KEY)
            .await
            .unwrap();
        if i == 0 {
            first_checkpoint = db
                .collection::<Document>("billing_account_balances")
                .find_one(doc! { "_id": "wallet:owner" })
                .await
                .unwrap();
        }
    }
    db.collection::<Document>("billing_wallet")
        .update_one(
            doc! { "_id": "wallet" },
            doc! { "$set": { "balance_credits": Credits::from_pico(1_000).unwrap() } },
        )
        .await
        .unwrap();
    assert_accounts(&db).await;
    let evidence = db
        .run_command(doc! {
            "explain": {
                "find": "billing_account_balances",
                "filter": { "_id": { "$in": ["wallet:owner"] } },
            },
            "verbosity": "executionStats",
        })
        .await
        .unwrap();
    let stats = evidence.get_document("executionStats").unwrap();
    assert_eq!(stats.get_i32("totalDocsExamined").unwrap(), 1);
    assert_eq!(stats.get_i32("totalKeysExamined").unwrap(), 1);
    // The append path updates only reconciled asset checkpoints. Historical
    // posting seeks are intentionally owned by the rolling verifier, so usage
    // and provider accounts do not create checkpoint documents.
    assert_eq!(
        db.collection::<Document>("billing_account_balances")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let checkpoint = db
        .collection::<Document>("billing_account_balances")
        .find_one(doc! { "_id": "wallet:owner" })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.get_i64("through_seq").unwrap(), 1_000);
    // Editing both mutable balances cannot bypass the authenticated checkpoint.
    db.collection::<Document>("billing_account_balances")
        .update_one(
            doc! { "_id": "wallet:owner" },
            doc! { "$set": { "balance": Credits::from_whole(1) } },
        )
        .await
        .unwrap();
    db.collection::<Document>("billing_wallet")
        .update_one(
            doc! {},
            doc! { "$set": { "balance_credits": Credits::from_whole(1) } },
        )
        .await
        .unwrap();
    account_reconciliation::run_once(&db).await.unwrap();
    let status = db
        .collection::<Document>("chain_verify_status")
        .find_one(doc! { "_id": account_reconciliation::CHECK_ID })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.get_str("outcome").unwrap(), "broken");
    // Replaying an authentic old checkpoint also fails against the newest seq.
    db.collection::<Document>("billing_account_balances")
        .replace_one(doc! { "_id": "wallet:owner" }, first_checkpoint.unwrap())
        .await
        .unwrap();
    db.collection::<Document>("billing_wallet")
        .update_one(
            doc! {},
            doc! { "$set": { "balance_credits": Credits::from_pico(1).unwrap() } },
        )
        .await
        .unwrap();
    account_reconciliation::run_once(&db).await.unwrap();
    let status = db
        .collection::<Document>("chain_verify_status")
        .find_one(doc! { "_id": account_reconciliation::CHECK_ID })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.get_str("outcome").unwrap(), "broken");
}

#[test]
fn exact_keys_override_legacy_micros_without_changing_units() {
    let old: CreditGrant = bson::from_document(old_grant(123_456)).unwrap();
    assert_eq!(old.amount.to_string(), "0.123456");
    let mut both = old_grant(9_999_999);
    both.insert("amount", "0.000000000001".parse::<Credits>().unwrap());
    both.insert("remaining", Credits::from_whole(2));
    let new: CreditGrant = bson::from_document(both).unwrap();
    assert_eq!(new.amount.pico(), 1);
    assert_eq!(new.remaining, Credits::from_whole(2));
    let stored = bson::to_document(&new).unwrap();
    assert!(!stored.contains_key("amount_micros"));
    assert!(matches!(
        stored.get("amount"),
        Some(bson::Bson::Decimal128(_))
    ));
    let funding: crate::models::usage_meter::UsageFunding = bson::from_document(doc! {
        "total_charge_micros": 123_i64,
        "total_charge": Credits::from_pico(1).unwrap(),
        "grant_funded_micros": 2_i64,
    })
    .unwrap();
    assert_eq!(funding.total_charge.unwrap().pico(), 1);
    assert_eq!(funding.grant_funded.unwrap(), Credits::from_micros(2));
    let stored = bson::to_document(&funding).unwrap();
    assert!(!stored.contains_key("total_charge_micros"));
    assert!(!stored.contains_key("grant_funded_micros"));
}

#[tokio::test]
async fn pending_cutover_gates_money_but_allows_metering_and_legacy_reads() {
    use crate::models::service_billing::{BillingMetric, ServiceBilling};
    let db = database("exact_pending_readiness").await;
    db.collection::<Document>("billing_wallet")
        .insert_one(old_wallet(10))
        .await
        .unwrap();
    db.collection::<Document>("credit_grants")
        .insert_one(old_grant(1_000_000))
        .await
        .unwrap();
    exact_migration::start(&db, false).await.unwrap();
    let ctx = route_context::BillingRouteContext::new(
        route_inventory::BillingIngress::Proxy,
        uuid::Uuid::new_v4().to_string(),
        "owner".into(),
        "owner".into(),
        None,
        Some("service".into()),
        Some("service".into()),
        Some("service".into()),
        route_context::NodeIntent::Direct,
        "bearer".into(),
        crate::models::usage_meter::CredentialClass::UserOwned,
        BillingMetric::CacheReadTokens,
        Some(&ServiceBilling {
            platform_billable: true,
            ..Default::default()
        }),
        false,
    )
    .with_platform_metering(true);
    assert!(matches!(
        reservation::gate_and_reserve(&db, Some(&Entitled), &ctx, false, 900).await,
        Err(crate::errors::AppError::BillingProviderUnavailable(_))
    ));
    assert!(
        reservation::gate_and_reserve(
            &db,
            Some(&Entitled),
            &ctx.with_platform_metering(false),
            false,
            900
        )
        .await
        .unwrap()
        .is_none()
    );
    assert_eq!(
        provisioning::get_wallet(&db, "owner")
            .await
            .unwrap()
            .unwrap()
            .balance_credits,
        Credits::from_whole(10)
    );
    assert_eq!(
        grants::list_grants(&db, Some("owner"), None, 50, 0)
            .await
            .unwrap()
            .0
            .len(),
        1
    );
    let config = std::sync::Arc::new(crate::test_utils::test_app_config());
    let reconciler =
        reconcile::BillingReconciler::new(db.clone(), Some(std::sync::Arc::new(Entitled)), config);
    let stats = reconciler.run_once().await.unwrap();
    assert_eq!(stats.recovered_settlements, 0);
    assert_eq!(stats.grants_expired, 0);
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    exact_migration::start(&db, true).await.unwrap();
    assert!(exact_migration::ready(&db).await.unwrap());
    // A previously-created reconciler sees the durable completion without restart.
    reconciler.run_once().await.unwrap();
}

#[tokio::test]
async fn aggregation_exact_keys_take_precedence_including_null() {
    let db = database("exact_query_precedence").await;
    let pico = Credits::from_pico(1).unwrap();
    db.collection::<Document>("usage_meter")
        .insert_many([
            doc! { "_id": "both", "funding": { "total_charge": pico, "total_charge_micros": 1_000_000_i64 } },
            doc! { "_id": "null", "funding": { "total_charge": null, "total_charge_micros": 1_000_000_i64 } },
            doc! { "_id": "old", "funding": { "total_charge_micros": 1_000_000_i64 } },
            doc! { "_id": "none" },
        ])
        .await.unwrap();
    let rows: Vec<Document> = db
        .collection::<Document>("usage_meter")
        .aggregate(vec![
            doc! { "$project": {
                "amount": amounts::credit_expr("$funding.total_charge"),
                "exact": amounts::funding_cost_present(),
            } },
            doc! { "$sort": { "_id": 1 } },
        ])
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows[0]["amount"], bson::Bson::from(pico));
    assert!(rows[0].get_bool("exact").unwrap());
    assert_eq!(
        Credits::from_bson(rows[1]["amount"].clone(), 1_000_000).unwrap(),
        Credits::ZERO
    );
    assert!(!rows[1].get_bool("exact").unwrap());
    assert_eq!(rows[2]["amount"], bson::Bson::Null);
    assert!(!rows[2].get_bool("exact").unwrap());
    assert_eq!(
        Credits::from_bson(rows[3]["amount"].clone(), 1_000_000).unwrap(),
        Credits::from_whole(1)
    );
    assert!(rows[3].get_bool("exact").unwrap());
}

#[tokio::test]
async fn migration_ready_latches_only_a_durable_completion_and_diagnostics_are_readable() {
    let db = database("exact_readiness_diagnostics").await;
    let migrations = db.collection::<Document>("billing_migrations");
    migrations.delete_many(doc! {}).await.unwrap();
    assert!(!exact_migration::ready(&db).await.unwrap());
    migrations
        .insert_one(
            doc! { "_id": "exact-v2", "detail": "waiting", "updated_at": bson::DateTime::now() },
        )
        .await
        .unwrap();
    migrations
        .insert_one(doc! {
            "_id": "exact-v2-rollups",
            "completed_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    db.collection::<Document>("billing_migration_errors")
        .insert_many((0..7).map(|i| doc! { "_id": format!("credit_grants:bad-{i}") }))
        .await
        .unwrap();
    db.collection::<Document>("billing_rate_diagnostics")
        .insert_one(doc! { "_id": "plan", "rejected_metrics": ["tokens", "images"] })
        .await
        .unwrap();
    let diagnostics = startup_diagnostics(&db).await.unwrap();
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(
        diagnostics[0].detail,
        "7 failed documents; first keys: credit_grants:bad-0, credit_grants:bad-1, \
         credit_grants:bad-2, credit_grants:bad-3, credit_grants:bad-4"
    );
    assert_eq!(diagnostics[1].detail, "Rejected metrics: tokens, images");
    assert!(!exact_migration::ready(&db).await.unwrap());
    migrations
        .update_one(
            doc! { "_id": "exact-v2" },
            doc! { "$set": { "completed_at": bson::DateTime::now() } },
        )
        .await
        .unwrap();
    assert!(exact_migration::ready(&db).await.unwrap());
    // The production completion marker is monotonic. Removing it in this
    // disposable fixture proves a latched read no longer queries MongoDB.
    migrations.delete_many(doc! {}).await.unwrap();
    assert!(exact_migration::ready(&db).await.unwrap());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn voice_seconds_reconcile_uses_allowance_then_grant_then_wallet_once() {
    use crate::models::{
        assistant_voice::{VoiceWindow, WINDOWS},
        service_billing::{BillingMetric, ServiceBilling},
    };
    let db = database("voice_funding").await;
    crate::services::assistant_voice::ensure_indexes(&db)
        .await
        .unwrap();
    db.collection::<Document>("billing_wallet")
        .insert_one(old_wallet(10_000_000))
        .await
        .unwrap();
    db.collection::<Document>("credit_grants")
        .insert_one(old_grant(100_000))
        .await
        .unwrap();
    exact_migration::run(&db).await.unwrap();
    db.collection::<Document>("billing_rate_cache")
        .insert_one(doc! {
            "_id":"voice-rate:*","lago_metric_code":"voice-rate","model":null,
            "credits_per_unit_micros":10_000_i64,"synced_at":bson::DateTime::from_chrono(Utc::now())
        })
        .await
        .unwrap();
    let now = Utc::now();
    let allowance = crate::models::usage_allowance::UsageAllowance {
        id: "allowance".into(),
        bundle_id: None,
        service_id: "service".into(),
        service_slug: "service".into(),
        metric: BillingMetric::VoiceSeconds,
        quantity: 10,
        recurrence: crate::models::usage_allowance::AllowanceRecurrence::Daily,
        target_kind: crate::models::billing_target::BillingTargetKind::AllUsers,
        target_user_ids: vec![],
        target_org_ids: vec![],
        target_group_ids: vec![],
        is_active: true,
        created_by: "admin".into(),
        created_at: now,
        updated_at: now,
    };
    db.collection::<crate::models::usage_allowance::UsageAllowance>("usage_allowances")
        .insert_one(&allowance)
        .await
        .unwrap();
    let sid = uuid::Uuid::new_v4().to_string();
    let id = voice::window_id(&sid, 0).unwrap();
    let mut ctx = route_context::BillingRouteContext::new(
        route_inventory::BillingIngress::Proxy,
        id.clone(),
        "owner".into(),
        "owner".into(),
        None,
        Some("service".into()),
        Some("service".into()),
        Some("service".into()),
        route_context::NodeIntent::Direct,
        "bearer".into(),
        crate::models::usage_meter::CredentialClass::UserOwned,
        BillingMetric::VoiceSeconds,
        Some(&ServiceBilling {
            platform_billable: true,
            ..Default::default()
        }),
        false,
    )
    .with_platform_metering(true);
    ctx.platform_lago_metric_code = "voice-rate".into();
    ctx.requested_voice_seconds = 30;
    // Keep the service's text primary. Silence must still settle the duration
    // component without estimating tokens or losing its funding split.
    ctx.platform_metric = BillingMetric::Tokens;
    ctx.platform_lago_metric_code = "voice-text-rate".into();
    ctx.platform_components = vec![crate::models::service_billing::ResaleSpec {
        metric: BillingMetric::VoiceSeconds,
        lago_metric_code: "voice-rate".into(),
    }];
    db.collection::<Document>("billing_rate_cache")
        .insert_one(doc! {
            "_id":"voice-text-rate:*","lago_metric_code":"voice-text-rate","model":null,
            "credits_per_unit_micros":1_i64,"synced_at":bson::DateTime::from_chrono(now)
        })
        .await
        .unwrap();
    let reservation = reservation::gate_and_reserve(&db, Some(&Entitled), &ctx, false, 900)
        .await
        .unwrap()
        .unwrap();
    let metered = meter::open(&db, &ctx, Some(&reservation)).await.unwrap();
    meter::mark_forwarded(&db, &metered).await.unwrap();
    db.collection(WINDOWS)
        .insert_one(VoiceWindow {
            id: uuid::Uuid::new_v4().to_string(),
            billing_request_id: id.clone(),
            session_id: sid,
            user_id: "owner".into(),
            start_second: 0,
            context_identity: "test".into(),
            reserved_seconds: 30,
            observed_seconds: 0,
            sealed: false,
            settled: false,
            uncertain: true,
            deadline: now - chrono::Duration::seconds(1),
        })
        .await
        .unwrap();
    voice::observe(&db, &id, 29).await.unwrap();
    voice::observe(&db, &id, 27).await.unwrap(); // Replayed lower cumulative event cannot reduce usage.
    assert_eq!(voice::reconcile(&db).await.unwrap(), 1);
    assert_eq!(voice::reconcile(&db).await.unwrap(), 0);
    voice::observe(&db, &id, 30).await.unwrap(); // Late tail after the deadline is waived.
    voice::finalize(&db, &id, false).await.unwrap();
    let row = db
        .collection::<UsageMeterRow>("usage_meter")
        .find_one(doc! {"billing_request_id":&id,"metric":"voice_seconds"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.quantity, Some(29));
    let funding = row.funding.unwrap();
    assert_eq!(
        funding.allowance_funded,
        Some(Credits::from_micros(100_000))
    );
    assert_eq!(funding.grant_funded, Some(Credits::from_micros(100_000)));
    assert_eq!(funding.wallet_funded, Some(Credits::from_micros(90_000)));
    assert!(funding.settled);
    let text = db
        .collection::<UsageMeterRow>("usage_meter")
        .find_one(doc! {"billing_request_id":&id,"metric":"tokens"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(text.quantity, Some(0));
    assert_accounts(&db).await;
}
