use super::*;
use crate::models::usage_meter::{UsageFunding, UsageMeterRow};
use mongodb::bson::Document;
use uuid::Uuid;

const OWNER: &str = "machine-owner";
const WALLET: &str = "machine-wallet";
const SKU: &str = "remote-machine-runtime";
const RATE_PICO: i64 = 40_000_000;

fn credits(seconds: i64) -> Credits {
    Credits::from_pico(i128::from(RATE_PICO) * i128::from(seconds)).unwrap()
}

fn layer(seconds: i64) -> LayerReservation {
    LayerReservation {
        layer: BillingLayer::Platform,
        metric: BillingMetric::MachineRunningSeconds,
        lago_metric_code: "remote_machine_running_seconds_version_one".into(),
        estimated_quantity: seconds,
        credits_per_unit_micros: 40,
        credits_per_unit_pico: Some(RATE_PICO),
        reserved_credits: Credits::ZERO,
        allowance_reservations: vec![],
        grant_reservations: vec![],
        tax_cache_ttl_secs: 900,
        applied_tax: None,
    }
}

#[test]
fn machine_funding_requires_a_finite_distinct_positive_reservation() {
    let candidates = MachineFundingCandidates {
        owner_id: OWNER.into(),
        service_id: None,
        service_slug: SKU.into(),
        periods: vec![],
    };
    assert!(validate_layer(OWNER, &candidates, &layer(300)).is_ok());
    assert!(validate_layer(OWNER, &candidates, &layer(301)).is_err());
    assert!(validate_layer(OWNER, &candidates, &layer(0)).is_err());
    assert!(validate_layer("other-owner", &candidates, &layer(1)).is_err());
    let mut invalid = layer(1);
    invalid.metric = BillingMetric::VoiceSeconds;
    assert!(validate_layer(OWNER, &candidates, &invalid).is_err());
    invalid = layer(1);
    invalid.credits_per_unit_micros = 0;
    invalid.credits_per_unit_pico = Some(0);
    assert!(validate_layer(OWNER, &candidates, &invalid).is_err());
}

async fn fixture(name: &str, balance_seconds: i64) -> mongodb::Database {
    let db = crate::test_utils::connect_transaction_test_database(name).await;
    super::super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        super::super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    crate::db::ensure_indexes(&db).await.unwrap();
    let now = bson::DateTime::now();
    db.collection::<Document>("billing_migrations")
        .update_one(
            doc! { "_id": "exact-v2" },
            doc! { "$set": { "completed_at": now } },
        )
        .upsert(true)
        .await
        .unwrap();
    db.collection::<Document>("users")
        .insert_one(doc! {
            "_id": OWNER, "user_type": "person", "is_active": true,
        })
        .await
        .unwrap();
    db.collection::<Document>(WALLETS)
        .insert_one(doc! {
            "_id": WALLET, "owner_id": OWNER, "lago_customer_id": "customer",
            "lago_wallet_id": "provider-wallet", "lago_subscription_id": "subscription",
            "plan_kind": "prepaid", "balance_credits": credits(balance_seconds),
            "reserved_credits": Credits::ZERO, "pending_lago_debits": Credits::ZERO,
            "pending_topup_expiry_credits": Credits::ZERO,
            "overdraft_cap_credits": Credits::ZERO,
            "has_payment_instrument": false, "suspended": false, "collection_state": "good",
            "created_at": now, "updated_at": now, "balance_synced_at": now,
        })
        .await
        .unwrap();
    crate::services::billing::taxes::seed_test_profiles(&db).await;
    db
}

async fn benefits(db: &mongodb::Database) {
    let now = bson::DateTime::now();
    for (id, metric, quantity) in [
        ("matching", "machine_running_seconds", 2_i64),
        ("voice", "voice_seconds", 100_i64),
    ] {
        db.collection::<Document>(ALLOWANCES)
            .insert_one(doc! {
                "_id": id, "service_id": SKU, "service_slug": SKU, "metric": metric,
                "quantity": quantity, "recurrence": "one_time", "target_kind": "all_users",
                "target_user_ids": [], "target_org_ids": [], "target_group_ids": [],
                "is_active": true, "created_by": "admin", "created_at": now, "updated_at": now,
            })
            .await
            .unwrap();
    }
    db.collection::<Document>(GRANTS)
        .insert_one(doc! {
            "_id": "grant", "batch_id": "batch", "recipient_user_id": OWNER,
            "target_kind": "selected_users", "amount_credits": 0_i64,
            "amount": credits(3), "remaining": credits(3), "reserved": Credits::ZERO,
            "scope": { "all_services": true, "service_ids": [], "service_slugs": [] },
            "status": "active", "issued_ledgered_at": now, "granted_by": "admin",
            "terminal_amount": Credits::ZERO, "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
}

fn row(id: &str, reservation: &BillingReservation) -> UsageMeterRow {
    let layer = &reservation.layers[0];
    let now = bson::DateTime::now();
    bson::from_document(doc! {
        "_id": id, "transaction_id": id, "billing_request_id": id,
        "layer": "platform", "billing_owner_id": OWNER, "wallet_id": WALLET,
        "actor_user_id": OWNER, "service_slug": SKU,
        "metric": "machine_running_seconds", "lago_metric_code": &layer.lago_metric_code,
        "credential_class": "no_auth", "reserved_credits": reservation.total_reserved_credits,
        "funding": bson::to_bson(&UsageFunding {
            credits_per_unit_micros: 40, credits_per_unit_pico: Some(RATE_PICO),
            settlement_rate: Some(credits(1)),
            tax_accounting: true,
            applied_tax: layer.applied_tax.clone(),
            allowance_reservations: layer.allowance_reservations.clone(),
            grant_reservations: layer.grant_reservations.clone(),
            ..UsageFunding::default()
        }).unwrap(),
        "quantity": Bson::Null, "status": "forwarded", "forwarded": true,
        "released": false, "lago_acked": false, "attempt": 0_i32,
        "created_at": now, "updated_at": now,
    })
    .unwrap()
}

async fn reserve(db: &mongodb::Database, seconds: i64, id: &str) -> AppResult<BillingReservation> {
    let candidates = prepare_machine_window_funding(db, OWNER, None, SKU, Utc::now()).await?;
    reserve_prepared(db, seconds, id, candidates).await
}

async fn reserve_prepared(
    db: &mongodb::Database,
    seconds: i64,
    id: &str,
    candidates: MachineFundingCandidates,
) -> AppResult<BillingReservation> {
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let id = id.to_owned();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<BillingReservation> = async {
                let reservation = reserve_machine_window_in_session(
                    &db,
                    session,
                    OWNER,
                    WALLET,
                    &candidates,
                    layer(seconds),
                    Utc::now(),
                )
                .await?;
                db.collection::<UsageMeterRow>("usage_meter")
                    .insert_one(row(&id, &reservation))
                    .session(&mut *session)
                    .await?;
                Ok(reservation)
            }
            .await;
            crate::services::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

async fn wallet(db: &mongodb::Database) -> BillingWallet {
    db.collection::<BillingWallet>(WALLETS)
        .find_one(doc! { "_id": WALLET })
        .await
        .unwrap()
        .unwrap()
}

async fn usage(db: &mongodb::Database, id: &str) -> UsageMeterRow {
    db.collection::<UsageMeterRow>("usage_meter")
        .find_one(doc! { "_id": id })
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn machine_window_atomicity_matching_allowance_and_exact_precedence() {
    let db = fixture("machine_p3_atomic", 100).await;
    benefits(&db).await;
    let reservation = reserve(&db, 10, "first").await.unwrap();
    assert_eq!(reservation.layers[0].allowance_reservations[0].quantity, 2);
    assert_eq!(reservation.layers[0].allowance_reservations.len(), 1);
    assert_eq!(
        reservation.layers[0].grant_reservations[0].amount,
        credits(3)
    );
    assert_eq!(reservation.total_reserved_credits, credits(5));
    // A real duplicate insert fails after the funding mutations, proving that
    // the caller-owned transaction rolls every hold back with its usage row.
    assert!(reserve(&db, 10, "first").await.is_err());
    assert_eq!(wallet(&db).await.reserved_credits, credits(5));
    let grant = db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! { "_id": "grant" })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(grant.reserved, credits(3));
    assert_eq!(
        db.collection::<Document>(PERIODS)
            .count_documents(doc! { "allowance_id": "voice" })
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_window_never_uses_overdraft_or_pending_provider_funds() {
    let db = fixture("machine_p3_balance", 10).await;
    benefits(&db).await;
    db.collection::<Document>(WALLETS)
        .update_one(
            doc! { "_id": WALLET },
            doc! { "$set": {
                "plan_kind": "hybrid", "has_payment_instrument": true,
                "overdraft_cap_credits": credits(1_000),
                "pending_lago_debits": credits(6), "pending_topup_expiry_credits": credits(4),
            } },
        )
        .await
        .unwrap();
    assert!(matches!(
        reserve(&db, 6, "unfunded").await,
        Err(AppError::InsufficientCredits)
    ));
    assert_eq!(wallet(&db).await.reserved_credits, Credits::ZERO);
    let grant = db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(grant.reserved, Credits::ZERO);
    let period = db
        .collection::<UsageAllowancePeriod>(PERIODS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(period.reserved_quantity, 0);
    // Fully held benefits still fund their finite window with no wallet spend.
    assert_eq!(
        reserve(&db, 5, "benefits_only")
            .await
            .unwrap()
            .total_reserved_credits,
        Credits::ZERO
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_machine_and_service_reservations_conserve_shared_balance() {
    // Nineteen funded slots for twenty contenders guarantees both consumer
    // classes commit, regardless of which class reaches MongoDB first.
    let db = fixture("machine_p3_race", 190).await;
    let candidates = prepare_machine_window_funding(&db, OWNER, None, SKU, Utc::now())
        .await
        .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(20));
    let tasks = (0..20).map(|index| {
        let db = db.clone();
        let candidates = candidates.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            let is_machine = index % 2 == 0;
            let admitted = if is_machine {
                match reserve_prepared(&db, 10, &Uuid::new_v4().to_string(), candidates).await {
                    Ok(_) => true,
                    Err(AppError::InsufficientCredits) => false,
                    Err(error) => panic!("Machine reservation failed unexpectedly: {error}"),
                }
            } else {
                super::super::super::reservation::try_reserve_prepaid(&db, OWNER, credits(10))
                    .await
                    .unwrap()
                    .is_some()
            };
            (is_machine, admitted)
        })
    });
    let results = futures::future::join_all(tasks).await;
    let admitted: Vec<_> = results
        .into_iter()
        .map(Result::unwrap)
        .filter(|(_, accepted)| *accepted)
        .collect();
    assert_eq!(admitted.len(), 19);
    assert!(admitted.iter().any(|(is_machine, _)| *is_machine));
    assert!(admitted.iter().any(|(is_machine, _)| !is_machine));
    let wallet = wallet(&db).await;
    assert_eq!(wallet.reserved_credits, credits(190));
    assert_eq!(wallet.available_credits().unwrap(), Credits::ZERO);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_machine_and_service_reservations_conserve_shared_grant() {
    let db = fixture("machine_p3_grant_race", 0).await;
    benefits(&db).await;
    db.collection::<Document>(ALLOWANCES)
        .delete_many(doc! {})
        .await
        .unwrap();
    db.collection::<Document>(GRANTS)
        .update_one(
            doc! {"_id":"grant"},
            doc! {"$set":{"amount":credits(30),"remaining":credits(30)}},
        )
        .await
        .unwrap();
    let candidates = prepare_machine_window_funding(&db, OWNER, None, SKU, Utc::now())
        .await
        .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(4));
    let tasks = (0..4).map(|index| {
        let db = db.clone();
        let candidates = candidates.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            let is_machine = index % 2 == 0;
            let reserved = if is_machine {
                match reserve_prepared(&db, 10, &Uuid::new_v4().to_string(), candidates).await {
                    Ok(hold) => hold.layers[0]
                        .grant_reservations
                        .iter()
                        .map(|allocation| allocation.amount)
                        .try_fold(Credits::ZERO, Credits::checked_add)
                        .unwrap(),
                    Err(AppError::InsufficientCredits) => Credits::ZERO,
                    Err(error) => panic!("Machine grant reservation failed: {error}"),
                }
            } else {
                match super::super::tests::ordinary(
                    &db,
                    OWNER,
                    None,
                    Some("ordinary-service"),
                    BillingMetric::Requests,
                    10,
                    credits(1),
                )
                .await
                {
                    Ok(hold) => Credits::checked_sum(
                        hold.layers[0].grant_reservations.iter().map(|a| a.amount),
                    )
                    .unwrap(),
                    Err(AppError::InsufficientCredits) => Credits::ZERO,
                    Err(error) => panic!("Ordinary admission failed: {error}"),
                }
            };
            (is_machine, reserved)
        })
    });
    let holds: Vec<_> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        holds
            .iter()
            .map(|(_, amount)| *amount)
            .try_fold(Credits::ZERO, Credits::checked_add)
            .unwrap(),
        credits(30)
    );
    assert!(
        holds
            .iter()
            .any(|(machine, amount)| *machine && *amount > Credits::ZERO)
    );
    assert!(
        holds
            .iter()
            .any(|(machine, amount)| !machine && *amount > Credits::ZERO)
    );
    let grant = db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! {"_id":"grant"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(grant.reserved, credits(30));
    assert_eq!(
        super::super::super::grants::available_grant(&grant).unwrap(),
        Credits::ZERO
    );
    assert_eq!(wallet(&db).await.reserved_credits, Credits::ZERO);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn pending_machine_liability_is_held_then_finite_settlement_releases_unused_funds() {
    let db = fixture("machine_p3_pending", 100).await;
    reserve(&db, 10, "runtime").await.unwrap();
    let pending = usage(&db, "runtime").await;
    assert!(
        super::super::settle_usage_funding(&db, &pending)
            .await
            .is_err()
    );
    assert!(
        super::super::release_usage_reservations(&db, &pending)
            .await
            .is_err()
    );
    assert_eq!(wallet(&db).await.reserved_credits, credits(10));
    db.collection::<Document>("usage_meter")
        .update_one(
            doc! { "_id": "runtime" },
            doc! {
                "$set": { "status": "finalized", "quantity": 6_i64 },
            },
        )
        .await
        .unwrap();
    let finalized = usage(&db, "runtime").await;
    assert!(
        super::super::super::reservation::claim_released_and_settle(&db, &finalized)
            .await
            .unwrap()
    );
    let after = wallet(&db).await;
    assert_eq!(after.reserved_credits, Credits::ZERO);
    assert_eq!(after.pending_lago_debits, credits(6));
    assert_eq!(after.available_credits().unwrap(), credits(94));
    // Replay is idempotent through the actual bounded wallet/ledger crash path.
    super::super::super::reservation::claim_released_and_settle(&db, &finalized)
        .await
        .unwrap();
    assert_eq!(wallet(&db).await.pending_lago_debits, credits(6));
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {
                "reference_id": "runtime",
            })
            .await
            .unwrap(),
        1
    );
    // Explicitly failed rows use the same recovery path without requiring a
    // fresh quote, top-up, or new reservation to settle their old liability.
    reserve(&db, 10, "failed-runtime").await.unwrap();
    let past = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(2));
    db.collection::<Document>("usage_meter")
        .update_one(
            doc! { "_id": "failed-runtime" },
            doc! {
                "$set": { "status": "failed", "quantity": 4_i64, "settlement_next_retry_at": past },
            },
        )
        .await
        .unwrap();
    assert_eq!(
        super::super::super::reservation::recover_retryable_settlements(&db)
            .await
            .unwrap()
            .recovered,
        1
    );
    assert_eq!(wallet(&db).await.pending_lago_debits, credits(10));
    assert!(usage(&db, "failed-runtime").await.released);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_settlement_honors_expired_holds_and_never_spends_new_benefits() {
    let db = fixture("machine_p3_expiry", 100).await;
    benefits(&db).await;
    reserve(&db, 10, "runtime").await.unwrap();
    let expired = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(1));
    db.collection::<Document>(GRANTS)
        .update_one(
            doc! { "_id": "grant" },
            doc! {
                "$set": { "expires_at": expired },
            },
        )
        .await
        .unwrap();
    db.collection::<Document>(PERIODS)
        .update_many(
            doc! {},
            doc! {
                "$set": { "period_end": expired },
            },
        )
        .await
        .unwrap();
    db.collection::<Document>(ALLOWANCES)
        .update_one(
            doc! { "_id": "voice" },
            doc! {
                "$set": { "metric": "machine_running_seconds" },
            },
        )
        .await
        .unwrap();
    db.collection::<Document>("usage_meter")
        .update_one(
            doc! { "_id": "runtime" },
            doc! {
                "$set": { "status": "finalized", "quantity": 6_i64 },
            },
        )
        .await
        .unwrap();
    let finalized = usage(&db, "runtime").await;
    super::super::super::reservation::claim_released_and_settle(&db, &finalized)
        .await
        .unwrap();
    assert_eq!(wallet(&db).await.pending_lago_debits, credits(1));
    assert_eq!(wallet(&db).await.reserved_credits, Credits::ZERO);
    let funding = usage(&db, "runtime").await.funding.unwrap();
    assert_eq!(funding.allowance_funded_quantity, Some(2));
    assert_eq!(funding.grant_funded, Some(credits(3)));
    assert_eq!(
        db.collection::<Document>(PERIODS)
            .count_documents(doc! { "allowance_id": "voice" })
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_funding_rejects_missing_wallet_subscription_and_generic_free_rows() {
    let db = fixture("machine_p3_strict", 100).await;
    db.collection::<Document>(WALLETS)
        .update_one(
            doc! { "_id": WALLET },
            doc! {
                "$unset": { "lago_subscription_id": "" },
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        reserve(&db, 10, "missing_subscription").await,
        Err(AppError::BillingNotConfigured(_))
    ));
    db.collection::<Document>(WALLETS)
        .delete_one(doc! { "_id": WALLET })
        .await
        .unwrap();
    assert!(matches!(
        reserve(&db, 10, "missing_wallet").await,
        Err(AppError::BillingNotConfigured(_))
    ));
    let reservation = BillingReservation {
        owner_id: OWNER.into(),
        wallet_id: WALLET.into(),
        total_reserved_credits: credits(10),
        layers: vec![layer(10)],
    };
    let mut invalid = row("metering_only", &reservation);
    invalid.wallet_id = None;
    invalid.quantity = Some(1);
    invalid.status = crate::models::usage_meter::UsageStatus::Finalized;
    assert!(
        super::super::settle_usage_funding(&db, &invalid)
            .await
            .is_err()
    );
    invalid.wallet_id = Some(WALLET.into());
    invalid.funding = None;
    assert!(
        super::super::settle_usage_funding(&db, &invalid)
            .await
            .is_err()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_window_waits_for_service_wallet_crash_bridge() {
    let db = fixture("machine_p3_wallet_lock", 100).await;
    db.collection::<Document>(WALLETS)
        .update_one(
            doc! { "_id": WALLET },
            doc! { "$set": {
                "active_settlement": {
                    "row_id": "service-usage", "reserved_credits": credits(1),
                    "actual_credits": credits(99), "applied": false,
                    "updated_at": bson::DateTime::now(),
                },
            } },
        )
        .await
        .unwrap();
    assert!(matches!(
        reserve(&db, 10, "blocked").await,
        Err(AppError::BillingProviderUnavailable(_))
    ));
    assert_eq!(wallet(&db).await.reserved_credits, Credits::ZERO);
    assert_eq!(
        db.collection::<Document>("usage_meter")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_background_settlement_recovers_failed_claim_and_benefit_crash_bridge() {
    let db = fixture("machine_p3_recover", 100).await;
    benefits(&db).await;
    reserve(&db, 10, "runtime").await.unwrap();
    let past = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(2));
    // A worker died after persisting the shared grant lock, before completing
    // its consumption and the final funding split. Recovery must consume once.
    db.collection::<Document>(GRANTS)
        .update_one(
            doc! { "_id": "grant" },
            doc! { "$set": {
                "active_settlement": {
                    "operation_id": "runtime:grant:grant", "usage_row_id": "runtime",
                    "reserved": credits(3), "consume": credits(3), "applied": false,
                    "updated_at": past,
                },
            } },
        )
        .await
        .unwrap();
    db.collection::<Document>("usage_meter")
        .update_one(
            doc! { "_id": "runtime" },
            doc! {
                "$set": { "status": "finalized", "quantity": 6_i64, "finalized_at": past },
            },
        )
        .await
        .unwrap();
    let recovered = super::super::super::reservation::recover_retryable_settlements(&db)
        .await
        .unwrap();
    assert_eq!(recovered.recovered, 1);
    assert_eq!(recovered.retried, 0);
    assert_eq!(wallet(&db).await.pending_lago_debits, credits(1));
    assert_eq!(wallet(&db).await.reserved_credits, Credits::ZERO);
    let grant = db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! { "_id": "grant" })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(grant.remaining, Credits::ZERO);
    assert_eq!(grant.reserved, Credits::ZERO);
    assert!(grant.active_settlement.is_none());
    let funding = usage(&db, "runtime").await.funding.unwrap();
    assert_eq!(funding.grant_consumptions.len(), 1);
    assert_eq!(funding.grant_funded, Some(credits(3)));
    assert_eq!(
        super::super::super::reservation::recover_retryable_settlements(&db)
            .await
            .unwrap()
            .recovered,
        0
    );
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {
                "dedupe_key": "grant-consumed:grant:runtime:grant:grant",
            })
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn capped_machine_window_and_pending_reclaim_retain_evidence_boundary() {
    let db = fixture("machine_allocation_caps", 100).await;
    benefits(&db).await;
    db.collection::<Document>(ALLOWANCES)
        .delete_many(doc! {})
        .await
        .unwrap();
    let rule = crate::models::credit_service_limit::CreditServiceLimit {
        key: "runtime".into(),
        service_ids: vec!["runtime-catalog".into()],
        service_slugs: vec![SKU.into()],
        label: Some("Runtime".into()),
        limit: credits(3),
        consumed: Credits::ZERO,
        reserved: Credits::ZERO,
    };
    db.collection::<Document>(GRANTS)
        .update_one(
            doc! { "_id": "grant" },
            doc! { "$set": { "service_limits": bson::to_bson(&vec![rule]).unwrap() } },
        )
        .await
        .unwrap();
    let reservation = reserve(&db, 10, "cap-window").await.unwrap();
    assert_eq!(
        reservation.layers[0].grant_reservations[0].amount,
        credits(3)
    );
    assert_eq!(reservation.total_reserved_credits, credits(7));
    let pending = super::super::super::grants::request_revoke_grant(&db, "grant", Some("admin"))
        .await
        .unwrap();
    assert!(pending.revoke_requested_at.is_some());
    let usage = usage(&db, "cap-window").await;
    assert!(
        super::super::release_usage_reservations(&db, &usage)
            .await
            .is_err()
    );
    assert!(
        super::super::settle_usage_funding(&db, &usage)
            .await
            .is_err()
    );
    let next = reserve(&db, 2, "next-window").await.unwrap();
    assert!(next.layers[0].grant_reservations.is_empty());
    let grant = db
        .collection::<CreditGrant>(GRANTS)
        .find_one(doc! { "_id": "grant" })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(grant.reserved, credits(3));
    assert_eq!(grant.service_limits[0].reserved, credits(3));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_wallet_windows_hold_gross_tax_and_benefits_skip_profiles() {
    let db = fixture("machine_gross_tax", 1).await;
    let mut config = crate::services::billing::taxes::zero_config();
    config.customer["taxes"] = serde_json::json!([{"code":"gst","name":"GST","rate":"9"}]);
    let profile =
        crate::services::billing::taxes::resolve_config(OWNER, "subscription", &config).unwrap();
    db.collection::<Document>(crate::models::billing_tax_profile::COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    db.collection::<crate::models::billing_tax_profile::BillingTaxProfile>(
        crate::models::billing_tax_profile::COLLECTION_NAME,
    )
    .insert_one(profile)
    .await
    .unwrap();
    assert!(matches!(
        reserve(&db, 1, "short-tax").await,
        Err(AppError::InsufficientCredits)
    ));
    let gross =
        crate::services::billing::taxes::gross_ceiling(credits(1), "9".parse().unwrap()).unwrap();
    db.collection::<Document>(WALLETS)
        .update_one(doc! {"_id":WALLET}, doc! {"$set":{"balance_credits":gross}})
        .await
        .unwrap();
    let reservation = reserve(&db, 1, "gross-window").await.unwrap();
    assert_eq!(reservation.total_reserved_credits, gross);
    assert_eq!(wallet(&db).await.reserved_credits, gross);
    assert_eq!(
        reservation.layers[0]
            .applied_tax
            .as_ref()
            .unwrap()
            .rate_percent,
        "9".parse().unwrap()
    );
    db.drop().await.unwrap();

    let db = fixture("machine_benefits_no_tax_profile", 0).await;
    benefits(&db).await;
    db.collection::<Document>(crate::models::billing_tax_profile::COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    let reservation = reserve(&db, 5, "benefit-window").await.unwrap();
    assert_eq!(reservation.total_reserved_credits, Credits::ZERO);
    assert_eq!(reservation.layers[0].allowance_reservations[0].quantity, 2);
    assert_eq!(
        reservation.layers[0].grant_reservations[0].amount,
        credits(3)
    );
    db.drop().await.unwrap();
}
