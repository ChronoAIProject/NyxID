use super::*;
use crate::models::credits::Credits;
use crate::services::billing::ledger;
const PROFILES: &str = crate::models::billing_tax_profile::COLLECTION_NAME;
fn request(owner: &str, id: &str) -> BillingRouteContext {
    route_context(
        &CoverageCase {
            ingress: BillingIngress::Proxy,
            scenario: "tax",
            node_intent: NodeIntent::Direct,
            metric: BillingMetric::Requests,
        },
        id,
        owner,
    )
}
async fn fixture(
    label: &str,
    tax_unavailable: bool,
) -> (mongodb::Database, String, Arc<FakeLago>, BillingService) {
    let db = crate::test_utils::connect_transaction_test_database(label).await;
    create_usage_index(&db).await;
    insert_fresh_rates(&db).await;
    let owner = insert_owner(&db).await;
    let lago = Arc::new(FakeLago {
        tax_rate: Some("9".into()),
        tax_unavailable,
        wallet_balance: Some(Credits::from_whole(100)),
        ..Default::default()
    });
    let service = billing_service(&db, lago.clone(), 1);
    service.ensure_wallet(&owner).await.unwrap();
    (db, owner, lago, service)
}
#[tokio::test]
async fn gross_hold_debit_tax_postings_chain_and_refresh_rounding() {
    let (db, owner, _, service) = fixture("tax_gross_settlement", false).await;
    let metered = service.open(&request(&owner, "tax-gross")).await.unwrap();
    assert_eq!(
        wallet(&db, &owner).await.reserved_credits,
        "1.09".parse().unwrap()
    );
    service.mark_forwarded(&metered).await.unwrap();
    service
        .settle(&metered, PlatformUsage::single_request(0), None, None)
        .await
        .unwrap();
    let row = usage_row(&db, "tax-gross").await;
    let funding = row.funding.as_ref().unwrap();
    assert_eq!(funding.wallet_funded, Some(Credits::from_whole(1)));
    assert_eq!(funding.wallet_tax_credits, Some("0.09".parse().unwrap()));
    assert_eq!(funding.wallet_charge_credits, Some("1.09".parse().unwrap()));
    assert_eq!(funding.lago_billable_quantity_micros, Some(1_000_000));
    assert_eq!(
        wallet(&db, &owner).await.pending_lago_debits,
        "1.09".parse().unwrap()
    );
    // A retry must reuse the gross split and the original usage dedupe key.
    service
        .settle(&metered, PlatformUsage::single_request(0), None, None)
        .await
        .unwrap();
    assert_eq!(
        wallet(&db, &owner).await.pending_lago_debits,
        "1.09".parse().unwrap()
    );
    assert_eq!(
        db.collection::<bson::Document>("billing_ledger")
            .count_documents(doc! {"dedupe_key":format!("usage-settled:{}",row.id)})
            .await
            .unwrap(),
        1
    );
    let entry = db
        .collection::<crate::models::billing_ledger::BillingLedgerEntry>("billing_ledger")
        .find_one(doc! {"dedupe_key":format!("usage-settled:{}",row.id)})
        .await
        .unwrap()
        .unwrap();
    ledger::validate_postings(&entry).unwrap();
    assert!(
        entry
            .postings
            .iter()
            .any(|p| p.account == format!("tax:usage:{owner}")
                && p.amount == "0.09".parse().unwrap())
    );
    let report = ledger::verify_chain(&db, &ledger::TEST_BILLING_LEDGER_HMAC_KEY, None, None, None)
        .await
        .unwrap();
    assert_eq!(report.status, ledger::BillingLedgerStatus::Ok);
    // Existing provider-refresh adjustment absorbs a bounded cent residue.
    db.collection::<bson::Document>(USAGE_METER)
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"lago_acked":true}})
        .await
        .unwrap();
    crate::services::billing::webhook::refresh_wallet_balance(&db, &owner, "98.9".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(
        wallet(&db, &owner).await.balance_credits,
        "98.9".parse().unwrap()
    );
    assert_eq!(wallet(&db, &owner).await.pending_lago_debits, Credits::ZERO);
    let adjustment = db
        .collection::<crate::models::billing_ledger::BillingLedgerEntry>("billing_ledger")
        .find_one(doc! {"movement":"wallet_adjusted"})
        .await
        .unwrap()
        .unwrap();
    assert!(
        adjustment
            .postings
            .iter()
            .any(|p| p.account == "external:lago" && p.amount == "0.01".parse().unwrap())
    );
    let report = ledger::verify_chain(&db, &ledger::TEST_BILLING_LEDGER_HMAC_KEY, None, None, None)
        .await
        .unwrap();
    assert_eq!(report.status, ledger::BillingLedgerStatus::Ok);
    db.drop().await.unwrap();
}
#[tokio::test]
async fn mixed_allowance_grant_and_wallet_settlement_taxes_only_the_wallet_share() {
    let (db, owner, _, service) = fixture("tax_mixed_funding", false).await;
    let now = bson::DateTime::now();
    db.collection::<bson::Document>("usage_allowances").insert_one(doc! {
        "_id":Uuid::new_v4().to_string(), "service_id":"proxy-catalog", "service_slug":"proxy-tax-mixed",
        "metric":"tokens", "quantity":1_i64, "recurrence":"one_time", "target_kind":"all_users",
        "target_user_ids":[], "target_org_ids":[], "target_group_ids":[], "is_active":true,
        "created_by":"admin", "created_at":now, "updated_at":now,
    }).await.unwrap();
    let grant: Credits = "0.5".parse().unwrap();
    db.collection::<bson::Document>("credit_grants")
        .insert_one(doc! {
            "_id":Uuid::new_v4().to_string(), "batch_id":"batch", "recipient_user_id":&owner,
            "target_kind":"selected_users", "amount_credits":0_i64, "amount":grant,
            "remaining":grant, "reserved":Credits::ZERO,
            "scope":{"all_services":true,"service_ids":[],"service_slugs":[]}, "status":"active",
            "issued_ledgered_at":now, "granted_by":"admin", "terminal_amount":Credits::ZERO,
            "created_at":now, "updated_at":now,
        })
        .await
        .unwrap();
    let ctx = route_context(
        &CoverageCase {
            ingress: BillingIngress::Proxy,
            scenario: "tax-mixed",
            node_intent: NodeIntent::Direct,
            metric: BillingMetric::Tokens,
        },
        "tax-mixed",
        &owner,
    )
    .with_native_text_bound(1, 2, 1);
    let metered = service.open(&ctx).await.unwrap();
    assert_eq!(
        wallet(&db, &owner).await.reserved_credits,
        "1.635".parse().unwrap()
    );
    service.mark_forwarded(&metered).await.unwrap();
    service
        .settle(&metered, PlatformUsage::llm_completion(0, 3), None, None)
        .await
        .unwrap();
    let funding = usage_row(&db, "tax-mixed").await.funding.unwrap();
    assert_eq!(funding.allowance_funded, Some(Credits::from_whole(1)));
    assert_eq!(funding.grant_funded, Some(grant));
    assert_eq!(funding.wallet_funded, Some("1.5".parse().unwrap()));
    assert_eq!(funding.wallet_tax_credits, Some("0.135".parse().unwrap()));
    assert_eq!(funding.lago_billable_quantity_micros, Some(1_500_000));
    assert_eq!(
        wallet(&db, &owner).await.pending_lago_debits,
        "1.635".parse().unwrap()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn missing_and_stale_profiles_refresh_once_then_prepaid_fails_closed() {
    let (db, owner, lago, service) = fixture("tax_inline_refresh", false).await;
    for stale in [false, true] {
        if stale {
            db.collection::<bson::Document>(PROFILES)
                .update_many(
                    doc! {},
                    doc! {"$set":{"fetched_at":bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
        } else {
            db.collection::<bson::Document>(PROFILES)
                .delete_many(doc! {})
                .await
                .unwrap();
        }
        lago.tax_fetches.store(0, Ordering::SeqCst);
        let meter = service
            .open(&request(&owner, &Uuid::new_v4().to_string()))
            .await
            .unwrap();
        assert_eq!(lago.tax_fetches.load(Ordering::SeqCst), 1);
        service
            .fail(&meter, "test cancelled before forwarding")
            .await
            .unwrap();
    }
    db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id":&owner},
            doc! {"$set":{"balance_credits":Credits::from_whole(1)}},
        )
        .await
        .unwrap();
    assert!(matches!(
        service.open(&request(&owner, "tax-prepaid-short")).await,
        Err(AppError::InsufficientCredits)
    ));
    db.collection::<bson::Document>(BILLING_WALLET).update_one(doc!{"owner_id":&owner},doc!{"$set":{"plan_kind":"hybrid","has_payment_instrument":true,"balance_credits":Credits::ZERO,"overdraft_cap_credits":Credits::from_whole(1)}}).await.unwrap();
    assert!(
        service
            .open(&request(&owner, "tax-overdraft-short"))
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}
#[tokio::test]
async fn tax_provider_failure_is_closed_but_benefit_only_needs_no_profile() {
    let (db, owner, lago, service) = fixture("tax_profile_failure", true).await;
    db.collection::<bson::Document>(PROFILES)
        .delete_many(doc! {})
        .await
        .unwrap();
    lago.tax_fetches.store(0, Ordering::SeqCst);
    assert!(service.open(&request(&owner, "tax-outage")).await.is_err());
    assert_eq!(lago.tax_fetches.load(Ordering::SeqCst), 1);
    assert_eq!(wallet(&db, &owner).await.reserved_credits, Credits::ZERO);
    assert!(
        service
            .open(&request(&owner, "tax-outage-second"))
            .await
            .is_err()
    );
    assert_eq!(lago.tax_fetches.load(Ordering::SeqCst), 1);
    let marker = db
        .collection::<crate::models::billing_tax_profile::BillingTaxProfile>(PROFILES)
        .find_one(doc! {"owner_id": &owner})
        .await
        .unwrap()
        .unwrap();
    assert!(marker.refresh_failed_at.is_some());
    assert!(crate::services::billing::taxes::snapshot(&marker, "platform_requests").is_err());
    // A new benefit-only admission still succeeds when its own refresh fails.
    db.collection::<bson::Document>(PROFILES)
        .delete_many(doc! {})
        .await
        .unwrap();
    let now = bson::DateTime::now();
    db.collection::<bson::Document>("credit_grants").insert_one(doc! {
        "_id":"tax-benefit", "batch_id":"batch", "recipient_user_id":&owner,"target_kind":"selected_users",
        "amount_credits":2_i64,"amount":Credits::from_whole(2),"remaining":Credits::from_whole(2),"reserved":Credits::ZERO,
        "scope":{"all_services":true,"service_ids":[],"service_slugs":[]},"status":"active","issued_ledgered_at":now,
        "granted_by":"admin","terminal_amount":Credits::ZERO,"created_at":now,"updated_at":now,
    }).await.unwrap();
    lago.tax_fetches.store(0, Ordering::SeqCst);
    let metered = service
        .open(&request(&owner, "benefit-no-tax"))
        .await
        .unwrap();
    assert_eq!(lago.tax_fetches.load(Ordering::SeqCst), 1);
    service.mark_forwarded(&metered).await.unwrap();
    service
        .settle(&metered, PlatformUsage::single_request(0), None, None)
        .await
        .unwrap();
    assert_eq!(wallet(&db, &owner).await.pending_lago_debits, Credits::ZERO);
    let row = usage_row(&db, "benefit-no-tax").await;
    assert_eq!(row.funding.unwrap().lago_billable_quantity_micros, Some(0));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn reconciliation_refreshes_before_recovering_benefit_estimate_wallet_overage() {
    let (db, owner, lago, service) = fixture("tax_recovery_refresh_order", false).await;
    let now = bson::DateTime::now();
    db.collection::<bson::Document>("credit_grants")
        .insert_one(doc! {
            "_id":Uuid::new_v4().to_string(), "batch_id":"batch", "recipient_user_id":&owner,
            "target_kind":"selected_users", "amount_credits":1_i64, "amount":Credits::from_whole(1),
            "remaining":Credits::from_whole(1), "reserved":Credits::ZERO,
            "scope":{"all_services":true,"service_ids":[],"service_slugs":[]}, "status":"active",
            "issued_ledgered_at":now, "granted_by":"admin", "terminal_amount":Credits::ZERO,
            "created_at":now, "updated_at":now,
        })
        .await
        .unwrap();
    db.collection::<bson::Document>(PROFILES)
        .delete_many(doc! {})
        .await
        .unwrap();
    lago.tax_fetches.store(0, Ordering::SeqCst);
    let metered = service
        .open(&request(&owner, "benefit-estimate-wallet-overage"))
        .await
        .unwrap();
    assert_eq!(lago.tax_fetches.load(Ordering::SeqCst), 1);
    db.collection::<bson::Document>(PROFILES)
        .delete_many(doc! {})
        .await
        .unwrap();
    lago.tax_fetches.store(0, Ordering::SeqCst);
    service.mark_forwarded(&metered).await.unwrap();
    // Crash after quantities are persisted but before funding is settled. The
    // actual quantity exceeds the fully grant-funded admission estimate.
    let usage = PlatformUsage {
        requests: 2,
        ..Default::default()
    };
    db.collection::<bson::Document>(USAGE_METER)
        .update_one(
            doc! {"billing_request_id":"benefit-estimate-wallet-overage"},
            doc! {"$set":{"pending_platform_usage":bson::to_bson(&usage).unwrap()}},
        )
        .await
        .unwrap();
    service.reconciler().run_once().await.unwrap();
    assert_eq!(lago.tax_fetches.load(Ordering::SeqCst), 1);
    let row = usage_row(&db, "benefit-estimate-wallet-overage").await;
    assert!(row.released);
    let funding = row.funding.unwrap();
    assert!(funding.settled);
    assert_eq!(funding.grant_funded, Some(Credits::from_whole(1)));
    assert_eq!(funding.wallet_funded, Some(Credits::from_whole(1)));
    assert_eq!(funding.wallet_tax_credits, Some("0.09".parse().unwrap()));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_funding_without_tax_fields_keeps_pre_tax_settlement() {
    let (db, owner, _, service) = fixture("tax_legacy_funding", false).await;
    let metered = service
        .open(&request(&owner, "legacy-no-tax"))
        .await
        .unwrap();
    let row = usage_row(&db, "legacy-no-tax").await;
    db.collection::<bson::Document>(USAGE_METER)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$unset":{
                "funding.tax_accounting":"",
                "funding.applied_tax":"", "funding.wallet_tax_credits":""
            }},
        )
        .await
        .unwrap();
    service.mark_forwarded(&metered).await.unwrap();
    service
        .settle(&metered, PlatformUsage::single_request(0), None, None)
        .await
        .unwrap();
    let funding = usage_row(&db, "legacy-no-tax").await.funding.unwrap();
    assert!(!funding.tax_accounting);
    assert_eq!(funding.wallet_tax_credits, Some(Credits::ZERO));
    assert_eq!(funding.wallet_charge_credits, Some(Credits::from_whole(1)));
    assert_eq!(
        wallet(&db, &owner).await.pending_lago_debits,
        Credits::from_whole(1)
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn stale_wallet_profile_fails_closed_and_forced_refresh_bypasses_backoff() {
    let (db, owner, _, _) = fixture("tax_stale_failure", false).await;
    db.collection::<bson::Document>(PROFILES)
        .update_many(
            doc! {},
            doc! {
                "$set": {"fetched_at": bson::DateTime::from_millis(0)}
            },
        )
        .await
        .unwrap();
    let lago = Arc::new(FakeLago {
        tax_unavailable: true,
        ..Default::default()
    });
    let service = billing_service(&db, lago.clone(), 1);
    for id in ["stale-failure", "stale-backoff"] {
        assert!(matches!(
            service.open(&request(&owner, id)).await,
            Err(AppError::BillingNotConfigured(_))
        ));
    }
    assert_eq!(lago.tax_fetches.load(Ordering::SeqCst), 1);
    crate::services::billing::taxes::reconcile(&db, lago.as_ref(), 900)
        .await
        .unwrap();
    assert_eq!(lago.tax_fetches.load(Ordering::SeqCst), 2);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn inline_refresh_times_out_and_bill_observes_negative_cache() {
    let (db, owner, _, _) = fixture("tax_inline_timeout", false).await;
    db.collection::<bson::Document>(PROFILES)
        .delete_many(doc! {})
        .await
        .unwrap();
    let lago = FakeLago {
        tax_delay: Some(std::time::Duration::from_secs(60)),
        ..Default::default()
    };
    let wallet = wallet(&db, &owner).await;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        crate::services::billing::taxes::ensure_fresh(&db, &lago, &wallet, 900),
    )
    .await
    .unwrap();
    assert!(matches!(
        result,
        Err(AppError::BillingProviderUnavailable(_))
    ));
    assert!(
        crate::services::billing::bill::current(&db, &lago, &wallet, 900)
            .await
            .is_err()
    );
    assert_eq!(lago.tax_fetches.load(Ordering::SeqCst), 1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn spill_settlement_uses_old_verified_profile() {
    let (db, owner, _, service) = fixture("tax_old_spill", false).await;
    let now = bson::DateTime::now();
    db.collection::<bson::Document>("credit_grants")
        .insert_one(doc! {
            "_id":Uuid::new_v4().to_string(), "batch_id":"batch", "recipient_user_id":&owner,
            "target_kind":"selected_users", "amount_credits":1_i64, "amount":Credits::from_whole(1),
            "remaining":Credits::from_whole(1), "reserved":Credits::ZERO,
            "scope":{"all_services":true,"service_ids":[],"service_slugs":[]}, "status":"active",
            "issued_ledgered_at":now, "granted_by":"admin", "terminal_amount":Credits::ZERO,
            "created_at":now, "updated_at":now,
        })
        .await
        .unwrap();
    let metered = service.open(&request(&owner, "old-spill")).await.unwrap();
    assert!(
        usage_row(&db, "old-spill")
            .await
            .funding
            .unwrap()
            .applied_tax
            .is_none()
    );
    db.collection::<bson::Document>(PROFILES)
        .update_many(
            doc! {},
            doc! {
                "$set": {"fetched_at": bson::DateTime::from_millis(0)}
            },
        )
        .await
        .unwrap();
    service.mark_forwarded(&metered).await.unwrap();
    service
        .settle(
            &metered,
            PlatformUsage {
                requests: 2,
                ..Default::default()
            },
            None,
            None,
        )
        .await
        .unwrap();
    let funding = usage_row(&db, "old-spill").await.funding.unwrap();
    assert_eq!(funding.wallet_funded, Some(Credits::from_whole(1)));
    assert_eq!(funding.wallet_tax_credits, Some("0.09".parse().unwrap()));
    assert_eq!(funding.wallet_charge_credits, Some("1.09".parse().unwrap()));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn customer_webhook_resolves_tax_owner_by_wallet_customer_identity() {
    let (db, owner, _, _) = fixture("tax_webhook_identity", false).await;
    let customer = format!("lago-customer-{}", Uuid::new_v4());
    db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id": &owner},
            doc! {
                "$set": {"lago_customer_id": &customer}
            },
        )
        .await
        .unwrap();
    let lago = FakeLago {
        tax_rate: Some("12".into()),
        ..Default::default()
    };
    crate::services::billing::webhook::handle_lago_webhook_event(
        &db,
        Some(&lago),
        "customer.updated",
        &serde_json::json!({"customer":{"external_id":customer}}),
    )
    .await
    .unwrap();
    let profile = db
        .collection::<crate::models::billing_tax_profile::BillingTaxProfile>(PROFILES)
        .find_one(doc! {"owner_id": &owner})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::services::billing::taxes::snapshot(&profile, "platform_requests")
            .unwrap()
            .rate_percent,
        "12".parse().unwrap()
    );
    assert_eq!(lago.tax_fetches.load(Ordering::SeqCst), 1);
    db.drop().await.unwrap();
}
