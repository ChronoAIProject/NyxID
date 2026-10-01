use super::*;
use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS};
use crate::models::service_billing::BillingMetric;
use crate::models::usage_meter::{CredentialClass, PoolAttemptAccounting};
use crate::services::billing::{BillingIngress, NodeIntent};

async fn fixture(
    label: &str,
) -> (
    mongodb::Database,
    BillingRouteContext,
    BillingWallet,
    Vec<LayerReservation>,
) {
    let db = crate::test_utils::connect_transaction_test_database(label).await;
    crate::db::ensure_indexes(&db).await.unwrap();
    let now = bson::DateTime::now();
    let wallet: BillingWallet = bson::from_document(doc! {
        "_id": Uuid::new_v4().to_string(), "owner_id": "pool-owner", "lago_customer_id": "customer",
        "plan_kind": "prepaid", "balance_credits": Credits::from_pico(100).unwrap(),
        "reserved_credits": Credits::ZERO, "collection_state": "good",
        "balance_synced_at": now, "created_at": now, "updated_at": now,
    })
    .unwrap();
    db.collection::<BillingWallet>(WALLETS)
        .insert_one(&wallet)
        .await
        .unwrap();
    db.collection::<Document>("users")
        .insert_one(doc! {
            "_id": "pool-owner", "user_type": "person", "is_active": true,
        })
        .await
        .unwrap();
    db.collection::<Document>("usage_allowances").insert_one(doc! {
        "_id": Uuid::new_v4().to_string(), "service_id": "catalog", "service_slug": "pool-api",
        "metric": "tokens", "quantity": 2_i64, "recurrence": "one_time", "target_kind": "all_users",
        "target_user_ids": [], "target_org_ids": [], "target_group_ids": [], "is_active": true,
        "created_by": "admin", "created_at": now, "updated_at": now,
    }).await.unwrap();
    db.collection::<Document>(CREDIT_GRANTS).insert_one(doc! {
        "_id": Uuid::new_v4().to_string(), "batch_id": "batch", "recipient_user_id": "pool-owner",
        "target_kind": "selected_users", "amount_credits": 0_i64,
        "amount": Credits::from_pico(3).unwrap(), "remaining": Credits::from_pico(3).unwrap(),
        "reserved": Credits::ZERO,
        "scope": { "all_services": true, "service_ids": [], "service_slugs": [] },
        "status": "active", "issued_ledgered_at": now, "granted_by": "admin",
        "terminal_amount": Credits::ZERO, "created_at": now, "updated_at": now,
    }).await.unwrap();
    let mut ctx = BillingRouteContext::new(
        BillingIngress::Proxy,
        Uuid::new_v4().to_string(),
        "pool-owner".into(),
        "pool-owner".into(),
        None,
        Some("member".into()),
        Some("catalog".into()),
        Some("pool-api".into()),
        NodeIntent::Direct,
        "bearer".into(),
        CredentialClass::UserOwned,
        BillingMetric::Tokens,
        None,
        false,
    )
    .with_platform_metering(true);
    ctx.pool_attempt = Some(PoolAttemptAccounting {
        pool_id: Uuid::new_v4().to_string(),
        member_id: "member".into(),
        attempt: 1,
        lease_until: Utc::now() + Duration::minutes(1),
        outcome: None,
        completion_cause: None,
    });
    let layers = vec![LayerReservation {
        layer: BillingLayer::Platform,
        metric: BillingMetric::Tokens,
        lago_metric_code: "platform_tokens".into(),
        estimated_quantity: 5,
        credits_per_unit_micros: 0,
        credits_per_unit_pico: Some(2),
        reserved_credits: Credits::from_pico(10).unwrap(),
        allowance_reservations: vec![],
        grant_reservations: vec![],
    }];
    (db, ctx, wallet, layers)
}

async fn holds(db: &mongodb::Database) -> (i64, Credits, Credits) {
    let period = db
        .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    let grant = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    let wallet = db
        .collection::<BillingWallet>(WALLETS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    (
        period.reserved_quantity,
        grant.reserved,
        wallet.reserved_credits,
    )
}

#[tokio::test]
async fn pool_admission_atomicity_preserves_exact_allowance_grant_wallet_precedence() {
    let (db, ctx, wallet, layers) = fixture("pool_atomic_admission").await;
    // Force a real failure AFTER all funding mutations, at the meter insert.
    let blocker = super::super::meter::reserved_row(
        &ctx,
        BillingLayer::Platform,
        BillingMetric::Tokens,
        "platform_tokens".into(),
        None,
        None,
    );
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(&blocker)
        .await
        .unwrap();
    assert!(
        reserve_and_open_pool(&db, &ctx, &wallet, layers.clone())
            .await
            .is_err()
    );
    assert_eq!(holds(&db).await, (0, Credits::ZERO, Credits::ZERO));
    db.collection::<UsageMeterRow>(USAGE_METER)
        .delete_one(doc! { "_id": blocker.id })
        .await
        .unwrap();
    let reserved = reserve_and_open_pool(&db, &ctx, &wallet, layers)
        .await
        .unwrap();
    assert_eq!(
        reserved.total_reserved_credits,
        Credits::from_pico(3).unwrap()
    );
    assert_eq!(
        holds(&db).await,
        (
            2,
            Credits::from_pico(3).unwrap(),
            Credits::from_pico(3).unwrap()
        )
    );
    let row = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    let funding = row.funding.unwrap();
    assert_eq!(funding.allowance_reservations[0].quantity, 2);
    assert_eq!(
        funding.grant_reservations[0].amount,
        Credits::from_pico(3).unwrap()
    );
    assert_eq!(row.reserved_credits, Credits::from_pico(3).unwrap());
    assert!(row.quantity.is_none());
    assert!(row.pool_attempt.is_some());
    super::super::pool_attempt::finish(
        &db,
        &ctx.billing_request_id,
        crate::models::usage_meter::PoolAttemptOutcome::Unsent,
    )
    .await
    .unwrap();
    assert_eq!(holds(&db).await, (0, Credits::ZERO, Credits::ZERO));
    db.drop().await.unwrap();
}
