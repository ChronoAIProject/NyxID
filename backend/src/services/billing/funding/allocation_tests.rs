//! Current-main regression tests: actual transactions and exact Decimal128,
//! never standalone counter mocks or inferred orphan repairs.
use super::*;
use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS};
use crate::models::credit_service_limit::CreditServiceLimit;

fn pico(value: i128) -> Credits {
    Credits::from_pico(value).unwrap()
}

#[tokio::test]
async fn platform_and_resale_consume_one_shared_cap_after_platform_allowance() {
    let (db, mut ctx, wallet, mut layers, id) = fixture("allocation_platform_resale").await;
    let now = bson::DateTime::now();
    db.collection::<Document>("usage_allowances")
        .insert_one(doc! {
            "_id": "included", "service_id": "catalog", "service_slug": "pool-api",
            "metric": "tokens", "quantity": 1_i64, "recurrence": "one_time",
            "target_kind": "all_users", "is_active": true, "created_by": "admin",
            "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    ctx.resale = Some(crate::models::service_billing::ResaleSpec {
        metric: ctx.platform_metric,
        lago_metric_code: "resale_tokens".into(),
    });
    layers[0].estimated_quantity = 2;
    layers[0].reserved_credits = pico(4);
    let mut resale = layers[0].clone();
    resale.layer = BillingLayer::Resale;
    resale.lago_metric_code = "resale_tokens".into();
    layers.push(resale);
    let reservation = reserve_and_open_ordinary(&db, &ctx, &wallet, layers)
        .await
        .unwrap();
    assert_eq!(reservation.layers[0].allowance_reservations[0].quantity, 1);
    assert_eq!(reservation.layers[0].grant_reservations[0].amount, pico(2));
    assert_eq!(reservation.layers[1].grant_reservations[0].amount, pico(1));
    assert_eq!(grant(&db, &id).await.service_limits[0].reserved, pico(3));
    let rows: Vec<UsageMeterRow> = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find(doc! { "billing_request_id": &ctx.billing_request_id })
        .sort(doc! { "layer": 1 })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let mut wallet_total = Credits::ZERO;
    for row in rows {
        let updated = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one_and_update(
                doc! { "_id": &row.id },
                doc! { "$set": { "quantity": 3_i64, "status": "finalized" } },
            )
            .return_document(ReturnDocument::After)
            .await
            .unwrap()
            .unwrap();
        wallet_total = wallet_total
            .checked_add(
                settle_usage_funding(&db, &updated)
                    .await
                    .unwrap()
                    .wallet_charge_credits,
            )
            .unwrap();
    }
    // 6 units × 2 pico, minus 1 allowance unit × 2 pico, minus one 3-pico cap.
    assert_eq!(wallet_total, pico(7));
    let current = grant(&db, &id).await;
    assert_eq!(current.remaining, pico(7));
    assert_eq!(current.service_limits[0].consumed, pico(3));
    assert_eq!(current.service_limits[0].reserved, Credits::ZERO);
    db.drop().await.unwrap();
}
async fn fixture(
    label: &str,
) -> (
    mongodb::Database,
    BillingRouteContext,
    BillingWallet,
    Vec<LayerReservation>,
    String,
) {
    let (db, mut ctx, wallet, layers) = super::pool_tests::fixture(label).await;
    ctx.pool_attempt = None;
    db.collection::<Document>("usage_allowances")
        .delete_many(doc! {})
        .await
        .unwrap();
    let grant = db
        .collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    let rule = CreditServiceLimit {
        key: "group".into(),
        service_ids: vec!["catalog".into()],
        service_slugs: vec!["pool-api".into()],
        label: Some("LLM".into()),
        limit: pico(3),
        consumed: Credits::ZERO,
        reserved: Credits::ZERO,
    };
    db.collection::<Document>(CREDIT_GRANTS).update_one(doc! { "_id": &grant.id }, doc! { "$set": { "amount": pico(10), "remaining": pico(10), "service_limits": bson::to_bson(&vec![rule]).unwrap() } }).await.unwrap();
    super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    (db, ctx, wallet, layers, grant.id)
}
async fn grant(db: &mongodb::Database, id: &str) -> CreditGrant {
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": id })
        .await
        .unwrap()
        .unwrap()
}
async fn finalize(db: &mongodb::Database, request: &str, quantity: i64) -> UsageMeterRow {
    db.collection::<UsageMeterRow>(USAGE_METER)
        .find_one_and_update(
            doc! { "billing_request_id": request },
            doc! { "$set": { "quantity": quantity, "status": "finalized" } },
        )
        .return_document(ReturnDocument::After)
        .await
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn exact_cap_admission_actual_extras_and_reclaim_conserve_one_identity() {
    let (db, ctx, wallet, mut layers, id) = fixture("allocation_pico_extras").await;
    layers[0].estimated_quantity = 1;
    layers[0].reserved_credits = pico(2);
    let hold = reserve_and_open_ordinary(&db, &ctx, &wallet, layers)
        .await
        .unwrap();
    assert_eq!(
        hold.layers[0].grant_reservations[0]
            .service_limit_key
            .as_deref(),
        Some("group")
    );
    assert_eq!(grant(&db, &id).await.service_limits[0].reserved, pico(2));
    let row = finalize(&db, &ctx.billing_request_id, 5).await;
    let result = settle_usage_funding(&db, &row).await.unwrap();
    assert_eq!(result.wallet_charge_credits, pico(7));
    let current = grant(&db, &id).await;
    assert_eq!(current.remaining, pico(7));
    assert_eq!(current.service_limits[0].consumed, pico(3));
    assert_eq!(current.service_limits[0].reserved, Credits::ZERO);
    assert_eq!(settle_usage_funding(&db, &row).await.unwrap(), result);
    let revoked = super::super::grants::request_revoke_grant(&db, &id, Some("admin"))
        .await
        .unwrap();
    assert_eq!(revoked.terminal_amount, pico(7));
    assert_eq!(
        revoked.status,
        crate::models::credit_grant::CreditGrantStatus::Revoked
    );
    assert_eq!(
        super::super::grants::request_revoke_grant(&db, &id, Some("other"))
            .await
            .unwrap(),
        revoked
    );
    let wallet_now = db
        .collection::<BillingWallet>(WALLETS)
        .find_one(doc! { "_id": &wallet.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wallet_now.balance_credits, wallet.balance_credits);
    db.drop().await.unwrap();
}
#[tokio::test]
async fn held_reclaim_fences_new_admission_and_extra_but_honors_reserved_work() {
    let (db, ctx, wallet, layers, id) = fixture("allocation_pending_reclaim").await;
    reserve_and_open_ordinary(&db, &ctx, &wallet, layers.clone())
        .await
        .unwrap();
    let pending = super::super::grants::request_revoke_grant(&db, &id, Some("admin"))
        .await
        .unwrap();
    assert!(pending.revoke_requested_at.is_some());
    assert_eq!(pending.reserved, pico(3));
    let summary =
        super::super::summaries::allocations(&db, Some(&ctx.billing_owner_id), Utc::now())
            .await
            .unwrap();
    assert_eq!(summary.held, pico(3));
    assert_eq!(summary.unreserved, Credits::ZERO);
    let mut next = ctx.clone();
    next.billing_request_id = Uuid::new_v4().to_string();
    let next_hold = reserve_and_open_ordinary(&db, &next, &wallet, layers)
        .await
        .unwrap();
    assert!(next_hold.layers[0].grant_reservations.is_empty());
    let row = finalize(&db, &ctx.billing_request_id, 1).await;
    assert_eq!(
        settle_usage_funding(&db, &row)
            .await
            .unwrap()
            .wallet_charge_credits,
        Credits::ZERO
    );
    let current = grant(&db, &id).await;
    assert_eq!(current.terminal_amount, pico(8));
    assert_eq!(current.service_limits[0].consumed, pico(2));
    assert_eq!(current.reserved, Credits::ZERO);
    db.drop().await.unwrap();
}
#[tokio::test]
async fn duplicate_last_capacity_reuses_winner_without_double_reservation() {
    let (db, ctx, wallet, layers, id) = fixture("allocation_duplicate_atomic").await;
    db.collection::<Document>(CREDIT_GRANTS)
        .delete_many(doc! {})
        .await
        .unwrap();
    db.collection::<Document>(WALLETS)
        .update_one(
            doc! { "_id": &wallet.id },
            doc! { "$set": { "balance_credits": pico(10) } },
        )
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        reserve_and_open_ordinary(&db, &ctx, &wallet, layers.clone()),
        reserve_and_open_ordinary(&db, &ctx, &wallet, layers.clone())
    );
    assert!(a.is_ok(), "{a:?}");
    assert!(b.is_ok(), "{b:?}");
    assert_eq!(
        db.collection::<Document>(USAGE_METER)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let current = db
        .collection::<BillingWallet>(WALLETS)
        .find_one(doc! { "_id": &wallet.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.reserved_credits, pico(10));
    assert!(!current.is_suspended());
    assert!(grant_absent(&db, &id).await);
    db.drop().await.unwrap();
}
async fn grant_absent(db: &mongodb::Database, id: &str) -> bool {
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .find_one(doc! { "_id": id })
        .await
        .unwrap()
        .is_none()
}

#[tokio::test]
async fn unrelated_broken_lock_does_not_block_admission_and_expired_holds_remain_visible() {
    let (db, ctx, wallet, layers, id) = fixture("allocation_lock_isolation").await;
    let mut broken = grant(&db, &id).await;
    broken.id = "unrelated".into();
    broken.recipient_user_id = "other".into();
    broken.active_settlement = Some(CreditGrantSettlementLock {
        operation_id: "broken".into(),
        usage_row_id: "missing".into(),
        service_limit_key: None,
        reserved: Credits::ZERO,
        consume: pico(1),
        applied: false,
        updated_at: Utc::now(),
    });
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .insert_one(broken)
        .await
        .unwrap();
    reserve_and_open_ordinary(&db, &ctx, &wallet, layers)
        .await
        .unwrap();
    db.collection::<Document>(CREDIT_GRANTS).update_one(doc! { "_id": &id }, doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)) } }).await.unwrap();
    let summary =
        super::super::summaries::allocations(&db, Some(&ctx.billing_owner_id), Utc::now())
            .await
            .unwrap();
    assert_eq!(summary.remaining, pico(3));
    assert_eq!(summary.held, pico(3));
    assert_eq!(summary.expired_awaiting_cleanup, pico(7));
    assert_eq!(summary.unreserved, Credits::ZERO);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_admissions_share_one_exact_cap_and_abort_has_no_orphan_holds() {
    let (db, ctx, wallet, layers, id) = fixture("allocation_concurrent_cap").await;
    // Fail the handoff while the group still has capacity, proving its counter
    // rolls back too (rather than testing only an already-exhausted ceiling).
    let mut failed = ctx.clone();
    failed.billing_request_id = "failed-initial-handoff".into();
    let blocker = super::super::meter::reserved_row(
        &failed,
        BillingLayer::Platform,
        failed.platform_metric,
        failed.platform_lago_metric_code.clone(),
        None,
        None,
    );
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(blocker)
        .await
        .unwrap();
    assert!(
        reserve_and_open_ordinary(&db, &failed, &wallet, layers.clone())
            .await
            .is_err()
    );
    let untouched = grant(&db, &id).await;
    assert_eq!(untouched.reserved, Credits::ZERO);
    assert_eq!(untouched.service_limits[0].reserved, Credits::ZERO);
    assert_eq!(
        db.collection::<BillingWallet>(WALLETS)
            .find_one(doc! { "_id": &wallet.id })
            .await
            .unwrap()
            .unwrap()
            .reserved_credits,
        Credits::ZERO
    );
    db.collection::<Document>(USAGE_METER)
        .delete_many(doc! {})
        .await
        .unwrap();
    let mut calls = Vec::new();
    for _ in 0..8 {
        let (db, mut ctx, wallet, layers) =
            (db.clone(), ctx.clone(), wallet.clone(), layers.clone());
        ctx.billing_request_id = Uuid::new_v4().to_string();
        calls.push(tokio::spawn(async move {
            reserve_and_open_ordinary(&db, &ctx, &wallet, layers).await
        }));
    }
    for call in calls {
        call.await.unwrap().unwrap();
    }
    let current = grant(&db, &id).await;
    assert_eq!(current.reserved, pico(3));
    assert_eq!(current.service_limits[0].reserved, pico(3));
    let wallet_now = db
        .collection::<BillingWallet>(WALLETS)
        .find_one(doc! { "_id": &wallet.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wallet_now.reserved_credits, pico(77));
    assert_eq!(
        db.collection::<UsageMeterRow>(USAGE_METER)
            .count_documents(doc! {})
            .await
            .unwrap(),
        8
    );

    // A real insertion failure happens after the transaction has mutated holds.
    // Its rollback must release both total and group counters without guessing
    // which active usage rows belong to concurrent workers.
    let mut failed = ctx.clone();
    failed.billing_request_id = "failed-handoff".into();
    let blocker = super::super::meter::reserved_row(
        &failed,
        BillingLayer::Platform,
        failed.platform_metric,
        failed.platform_lago_metric_code.clone(),
        None,
        None,
    );
    db.collection::<UsageMeterRow>(USAGE_METER)
        .insert_one(blocker)
        .await
        .unwrap();
    assert!(
        reserve_and_open_ordinary(&db, &failed, &wallet, layers)
            .await
            .is_err()
    );
    assert_eq!(grant(&db, &id).await, current);
    assert_eq!(
        db.collection::<BillingWallet>(WALLETS)
            .find_one(doc! { "_id": &wallet.id })
            .await
            .unwrap()
            .unwrap(),
        wallet_now
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn unapplied_extra_lock_is_helped_for_available_candidate() {
    assert_unapplied_extra_lock_handoff(5, true).await;
}

#[tokio::test]
async fn capped_out_extra_lock_waits_for_own_usage_drain() {
    assert_unapplied_extra_lock_handoff(3, false).await;
}

async fn assert_unapplied_extra_lock_handoff(limit: i128, helped: bool) {
    let (db, ctx, wallet, mut layers, id) = fixture("allocation_extra_bridge").await;
    db.collection::<Document>(CREDIT_GRANTS)
        .update_one(
            doc! { "_id": &id },
            doc! { "$set": { "service_limits.0.limit": pico(limit) } },
        )
        .await
        .unwrap();
    layers[0].estimated_quantity = 1;
    layers[0].reserved_credits = pico(2);
    reserve_and_open_ordinary(&db, &ctx, &wallet, layers.clone())
        .await
        .unwrap();
    let row = finalize(&db, &ctx.billing_request_id, 5).await;
    let lock = CreditGrantSettlementLock {
        operation_id: format!("{}:grant:{}", row.id, id),
        usage_row_id: row.id.clone(),
        service_limit_key: Some("group".into()),
        reserved: pico(2),
        consume: pico(3),
        applied: false,
        updated_at: bson::DateTime::now().to_chrono(),
    };
    db.collection::<Document>(CREDIT_GRANTS)
        .update_one(
            doc! { "_id": &id },
            doc! { "$set": { "active_settlement": bson::to_bson(&lock).unwrap() } },
        )
        .await
        .unwrap();
    let summary =
        super::super::summaries::allocations(&db, Some(&ctx.billing_owner_id), Utc::now())
            .await
            .unwrap();
    assert_eq!(summary.held, pico(3));
    let mut next = ctx.clone();
    next.billing_request_id = Uuid::new_v4().to_string();
    let reserved = reserve_and_open_ordinary(&db, &next, &wallet, layers)
        .await
        .unwrap();
    let current = grant(&db, &id).await;
    if helped {
        assert_eq!(reserved.layers[0].grant_reservations[0].amount, pico(2));
        assert!(current.active_settlement.is_none());
        assert_eq!(current.remaining, pico(7));
        assert_eq!(current.reserved, pico(2));
        assert_eq!(current.service_limits[0].consumed, pico(3));
    } else {
        // An unrelated admission must not help an unusable source. Its own
        // settlement later drains the lock by indexed usage ID, even though
        // the source has no realizable cap capacity.
        assert!(reserved.layers[0].grant_reservations.is_empty());
        assert_eq!(current.active_settlement.as_ref(), Some(&lock));
        assert_eq!(current.remaining, pico(10));
        assert_eq!(current.reserved, pico(2));
        assert_eq!(current.service_limits[0].consumed, Credits::ZERO);
    }
    assert_eq!(
        settle_usage_funding(&db, &row)
            .await
            .unwrap()
            .wallet_charge_credits,
        pico(7)
    );
    let current = grant(&db, &id).await;
    assert!(current.active_settlement.is_none());
    assert_eq!(current.remaining, pico(7));
    assert_eq!(
        current.reserved,
        if helped { pico(2) } else { Credits::ZERO }
    );
    assert_eq!(current.service_limits[0].reserved, current.reserved);
    assert_eq!(current.service_limits[0].consumed, pico(3));
    let saved = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find_one(doc! { "_id": &row.id })
        .await
        .unwrap()
        .unwrap();
    let funding = saved.funding.unwrap();
    assert_eq!(funding.grant_consumptions.len(), 1);
    assert_eq!(funding.grant_consumptions[0].amount, pico(3));
    assert_eq!(
        funding.grant_consumptions[0].operation_id,
        lock.operation_id
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn admission_finishes_its_unapplied_wallet_settlement_before_reserving() {
    assert_wallet_settlement_handoff(false, 100).await;
}

#[tokio::test]
async fn admission_finishes_its_applied_wallet_settlement_before_reserving() {
    assert_wallet_settlement_handoff(true, 100).await;
}

#[tokio::test]
async fn admission_rejects_insufficient_balance_after_unapplied_wallet_true_up() {
    assert_wallet_settlement_handoff(false, 20).await;
}

#[tokio::test]
async fn admission_rejects_insufficient_balance_after_applied_wallet_true_up() {
    assert_wallet_settlement_handoff(true, 20).await;
}

async fn assert_wallet_settlement_handoff(applied: bool, balance: i128) {
    let (db, ctx, wallet, layers, id) = fixture("allocation_wallet_bridge").await;
    db.collection::<Document>(WALLETS)
        .update_one(
            doc! { "_id": &wallet.id },
            doc! { "$set": { "balance_credits": pico(balance) } },
        )
        .await
        .unwrap();
    reserve_and_open_ordinary(&db, &ctx, &wallet, layers.clone())
        .await
        .unwrap();
    let row = finalize(&db, &ctx.billing_request_id, 10).await;
    let settled = settle_usage_funding(&db, &row).await.unwrap();
    assert_eq!(settled.wallet_charge_credits, pico(17));
    // Persist the same crash boundary as the existing wallet settlement
    // bridge: funding is final, wallet application or journaling is pending.
    let mut fields = doc! { "active_settlement": {
        "row_id": &row.id, "reserved_credits": pico(7), "actual_credits": pico(17),
        "applied": applied, "updated_at": bson::DateTime::now(),
    } };
    if applied {
        fields.insert("reserved_credits", Credits::ZERO);
        fields.insert("pending_lago_debits", pico(17));
    }
    db.collection::<Document>(WALLETS)
        .update_one(doc! { "_id": &wallet.id }, doc! { "$set": fields })
        .await
        .unwrap();
    // An unrelated broken operation must never be scanned or helped.
    db.collection::<Document>(WALLETS)
        .insert_one(doc! { "_id": "other-wallet", "owner_id": "other",
        "active_settlement": { "row_id": "missing", "applied": false } })
        .await
        .unwrap();
    let mut next = ctx.clone();
    next.billing_request_id = Uuid::new_v4().to_string();
    let admitted = reserve_and_open_ordinary(&db, &next, &wallet, layers.clone()).await;
    if balance == 100 {
        assert_eq!(admitted.unwrap().total_reserved_credits, pico(10));
    } else {
        // The 7-pico estimate left 13 available, but the true 17-pico
        // debit leaves only 3. Never admit 10 against the old snapshot.
        assert!(matches!(admitted, Err(AppError::InsufficientCredits)));
        assert_eq!(
            db.collection::<UsageMeterRow>(USAGE_METER)
                .count_documents(doc! { "billing_request_id": &next.billing_request_id })
                .await
                .unwrap(),
            0
        );
    }
    let current = db
        .collection::<BillingWallet>(WALLETS)
        .find_one(doc! { "_id": &wallet.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        current.reserved_credits,
        if balance == 100 {
            pico(10)
        } else {
            Credits::ZERO
        }
    );
    assert_eq!(current.pending_lago_debits, pico(17));
    assert!(!current.is_suspended());
    assert!(
        !db.collection::<Document>(WALLETS)
            .find_one(doc! { "_id": &wallet.id })
            .await
            .unwrap()
            .unwrap()
            .contains_key("active_settlement")
    );
    assert!(
        db.collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! { "_id": &row.id })
            .await
            .unwrap()
            .unwrap()
            .released
    );
    super::super::reservation::finish_wallet_settlement(&db, &wallet.id, &ctx.billing_owner_id)
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! { "dedupe_key": format!("usage-settled:{}", row.id) })
            .await
            .unwrap(),
        1
    );
    assert_eq!(grant(&db, &id).await.service_limits[0].consumed, pico(3));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn canonical_identity_and_legacy_connection_slug_have_consistent_caps() {
    let (db, mut ctx, wallet, layers, id) = fixture("allocation_canonical_legacy").await;
    ctx.catalog_service_id = None;
    ctx.user_service_id = Some("connection".into());
    reserve_and_open_ordinary(&db, &ctx, &wallet, layers.clone())
        .await
        .unwrap();
    let row = finalize(&db, &ctx.billing_request_id, 8).await;
    let result = settle_usage_funding(&db, &row).await.unwrap();
    assert_eq!(result.wallet_charge_credits, pico(13));
    assert_eq!(grant(&db, &id).await.service_limits[0].consumed, pico(3));
    db.collection::<Document>(CREDIT_GRANTS).update_one(doc! { "_id": &id }, doc! { "$set": {
        "scope": { "all_services": false, "service_ids": ["catalog"], "service_slugs": ["pool-api"] }
    } }).await.unwrap();
    ctx.billing_request_id = Uuid::new_v4().to_string();
    ctx.catalog_service_id = Some("different-catalog".into());
    assert!(
        reserve_and_open_ordinary(&db, &ctx, &wallet, layers)
            .await
            .unwrap()
            .layers[0]
            .grant_reservations
            .is_empty()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn same_operation_replay_and_stale_claim_cannot_release_another_requests_hold() {
    let (db, ctx, wallet, mut layers, id) = fixture("allocation_operation_fence").await;
    db.collection::<Document>(CREDIT_GRANTS)
        .update_one(
            doc! { "_id": &id },
            doc! { "$set": { "service_limits": [] } },
        )
        .await
        .unwrap();
    layers[0].estimated_quantity = 1;
    layers[0].reserved_credits = pico(2);
    reserve_and_open_ordinary(&db, &ctx, &wallet, layers.clone())
        .await
        .unwrap();
    let row = finalize(&db, &ctx.billing_request_id, 1).await;
    settle_usage_funding(&db, &row).await.unwrap();
    let mut next = ctx.clone();
    next.billing_request_id = Uuid::new_v4().to_string();
    reserve_and_open_ordinary(&db, &next, &wallet, layers)
        .await
        .unwrap();
    let before = grant(&db, &id).await;
    assert!(
        settle_grant_resource(
            &db,
            &row,
            &id,
            &format!("{}:grant:{}", row.id, id),
            pico(2),
            pico(2),
            None,
            true
        )
        .await
        .unwrap()
    );
    assert_eq!(grant(&db, &id).await, before);
    let mut stale = finalize(&db, &next.billing_request_id, 1).await;
    stale.funding.as_mut().unwrap().settlement_claim_id = Some("stale".into());
    db.collection::<Document>(USAGE_METER)
        .update_one(
            doc! { "_id": &stale.id },
            doc! { "$set": { "funding.settlement_claim_id": "successor" } },
        )
        .await
        .unwrap();
    assert!(
        settle_grant_resource(
            &db,
            &stale,
            &id,
            "stale-operation",
            pico(2),
            pico(2),
            None,
            true
        )
        .await
        .is_err()
    );
    assert_eq!(grant(&db, &id).await, before);
    assert!(
        refresh_claimed_funding(&db, &mut stale, "stale")
            .await
            .is_err()
    );
    assert_eq!(
        stale.funding.unwrap().settlement_claim_id.as_deref(),
        Some("stale")
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn stale_allowance_acquisition_cannot_mutate_a_successors_resource() {
    let (db, mut ctx, wallet, mut layers) =
        super::pool_tests::fixture("allocation_allowance_claim").await;
    ctx.pool_attempt = None;
    layers[0].estimated_quantity = 1;
    layers[0].reserved_credits = pico(2);
    reserve_and_open_ordinary(&db, &ctx, &wallet, layers)
        .await
        .unwrap();
    let mut stale = finalize(&db, &ctx.billing_request_id, 1).await;
    let allocation = stale.funding.as_ref().unwrap().allowance_reservations[0].clone();
    let before = db
        .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
        .find_one(doc! { "_id": &allocation.period_id })
        .await
        .unwrap()
        .unwrap();
    stale.funding.as_mut().unwrap().settlement_claim_id = Some("stale".into());
    db.collection::<Document>(USAGE_METER)
        .update_one(
            doc! { "_id": &stale.id },
            doc! { "$set": { "funding.settlement_claim_id": "successor" } },
        )
        .await
        .unwrap();
    assert!(
        settle_allowance_resource(
            &db,
            &stale,
            &allocation.allowance_id,
            &allocation.period_id,
            "stale-allowance",
            1,
            1,
            true
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
            .find_one(doc! { "_id": &allocation.period_id })
            .await
            .unwrap()
            .unwrap(),
        before
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn capped_out_broken_lock_does_not_block_independent_usable_allocation() {
    let (db, ctx, wallet, layers, id) = fixture("allocation_exhausted_lock").await;
    let mut exhausted = grant(&db, &id).await;
    exhausted.id = "exhausted".into();
    exhausted.service_limits[0].consumed = pico(3);
    exhausted.remaining = pico(7);
    exhausted.expires_at = Some(Utc::now() + Duration::hours(1));
    exhausted.active_settlement = Some(CreditGrantSettlementLock {
        operation_id: "failed-journal".into(),
        usage_row_id: "unrelated-missing-row".into(),
        service_limit_key: Some("group".into()),
        reserved: Credits::ZERO,
        consume: pico(3),
        applied: true,
        updated_at: Utc::now(),
    });
    db.collection::<CreditGrant>(CREDIT_GRANTS)
        .insert_one(exhausted)
        .await
        .unwrap();
    let reservation = reserve_and_open_ordinary(&db, &ctx, &wallet, layers)
        .await
        .unwrap();
    assert_eq!(reservation.layers[0].grant_reservations[0].grant_id, id);
    assert_eq!(grant(&db, "exhausted").await.reserved, Credits::ZERO);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn journal_failure_retains_capped_lock_and_pending_reclaim_until_exact_retry() {
    let (db, ctx, wallet, mut layers, id) = fixture("allocation_reclaim_journal_retry").await;
    layers[0].estimated_quantity = 1;
    layers[0].reserved_credits = pico(2);
    reserve_and_open_ordinary(&db, &ctx, &wallet, layers)
        .await
        .unwrap();
    super::super::grants::request_revoke_grant(&db, &id, Some("admin"))
        .await
        .unwrap();
    let row = finalize(&db, &ctx.billing_request_id, 1).await;
    // An actual database validation failure prevents journal durability after
    // applying resource counters. Recovery must retain its durable operation.
    db.run_command(doc! { "collMod": "billing_ledger", "validator": { "$expr": { "$eq": [1, 0] } }, "validationLevel": "strict" }).await.unwrap();
    assert!(settle_usage_funding(&db, &row).await.is_err());
    let failed = grant(&db, &id).await;
    assert_eq!(failed.remaining, pico(8));
    assert_eq!(failed.reserved, Credits::ZERO);
    assert_eq!(failed.service_limits[0].consumed, pico(2));
    assert!(failed.active_settlement.as_ref().unwrap().applied);
    assert!(failed.revoke_requested_at.is_some());
    assert_eq!(
        failed.status,
        crate::models::credit_grant::CreditGrantStatus::Active
    );
    db.run_command(doc! { "collMod": "billing_ledger", "validator": {} })
        .await
        .unwrap();
    // Failed work retains its invocation claim. A fresh retry cannot take it
    // over until its lease expires, even after journal availability returns.
    assert!(matches!(
        settle_usage_funding(&db, &row).await,
        Err(AppError::Internal(message)) if message.contains("usage funding settlement is busy")
    ));
    assert_eq!(grant(&db, &id).await, failed);
    let stale_claim = db
        .collection::<Document>(USAGE_METER)
        .update_one(
            doc! { "_id": &row.id, "funding.settlement_claim_id": { "$type": "string" } },
            doc! { "$set": { "funding.settlement_claimed_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(FUNDING_CLAIM_LEASE_SECS + 1)) } },
        )
        .await
        .unwrap();
    assert_eq!(stale_claim.modified_count, 1);
    assert_eq!(
        settle_usage_funding(&db, &row)
            .await
            .unwrap()
            .wallet_charge_credits,
        Credits::ZERO
    );
    let recovered = grant(&db, &id).await;
    assert_eq!(recovered.terminal_amount, pico(8));
    assert_eq!(
        recovered.status,
        crate::models::credit_grant::CreditGrantStatus::Revoked
    );
    assert!(recovered.active_settlement.is_none());
    assert_eq!(recovered.service_limits[0].consumed, pico(2));
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {
            "dedupe_key": format!("grant-revoked:{id}") })
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn takeover_drains_applied_unrecorded_exhausted_cap_before_wallet_split() {
    let (db, ctx, wallet, mut layers, id) = fixture("allocation_takeover_history").await;
    layers[0].estimated_quantity = 1;
    layers[0].reserved_credits = pico(2);
    reserve_and_open_ordinary(&db, &ctx, &wallet, layers)
        .await
        .unwrap();
    let row = finalize(&db, &ctx.billing_request_id, 5).await;
    let lock = CreditGrantSettlementLock {
        operation_id: format!("{}:grant:{}", row.id, id),
        usage_row_id: row.id.clone(),
        service_limit_key: Some("group".into()),
        reserved: pico(2),
        consume: pico(3),
        applied: true,
        updated_at: Utc::now() - Duration::minutes(2),
    };
    db.collection::<Document>(CREDIT_GRANTS).update_one(doc! { "_id": &id }, doc! { "$set": { "remaining": pico(7), "reserved": Credits::ZERO, "service_limits.0.consumed": pico(3), "service_limits.0.reserved": Credits::ZERO, "active_settlement": bson::to_bson(&lock).unwrap() } }).await.unwrap();
    let result = settle_usage_funding(&db, &row).await.unwrap();
    assert_eq!(result.wallet_charge_credits, pico(7));
    let saved = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .find_one(doc! { "_id": &row.id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.funding.unwrap().grant_consumptions.len(), 1);
    assert!(grant(&db, &id).await.active_settlement.is_none());
    db.drop().await.unwrap();
}

type ExtraBarrier = (
    std::sync::Arc<tokio::sync::Notify>,
    std::sync::Arc<tokio::sync::Notify>,
);
static EXTRA_BARRIERS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, ExtraBarrier>>,
> = std::sync::LazyLock::new(Default::default);
pub(super) async fn pause_allowance_extra(period: &str) {
    let hook = EXTRA_BARRIERS.lock().unwrap().remove(period);
    if let Some((arrived, resume)) = hook {
        arrived.notify_one();
        resume.notified().await;
    }
}
#[tokio::test]
async fn competing_extra_allowance_consumption_uses_recorded_quantity_for_exact_wallet_remainder() {
    let (db, mut ctx, wallet, mut layers) =
        super::pool_tests::fixture("allowance_extra_conservation").await;
    ctx.pool_attempt = None;
    db.collection::<Document>(CREDIT_GRANTS)
        .delete_many(doc! {})
        .await
        .unwrap();
    db.collection::<Document>("usage_allowances")
        .update_one(doc! {}, doc! { "$set": { "quantity": 3_i64 } })
        .await
        .unwrap();
    layers[0].estimated_quantity = 1;
    layers[0].reserved_credits = pico(2);
    reserve_and_open_ordinary(&db, &ctx, &wallet, layers.clone())
        .await
        .unwrap();
    let a = finalize(&db, &ctx.billing_request_id, 4).await;
    let mut competitor = ctx.clone();
    competitor.billing_request_id = Uuid::new_v4().to_string();
    layers[0].estimated_quantity = 0;
    layers[0].reserved_credits = Credits::ZERO;
    reserve_and_open_ordinary(&db, &competitor, &wallet, layers)
        .await
        .unwrap();
    let b = finalize(&db, &competitor.billing_request_id, 1).await;
    let period = a.funding.as_ref().unwrap().allowance_reservations[0]
        .period_id
        .clone();
    let (arrived, resume) = (
        std::sync::Arc::new(tokio::sync::Notify::new()),
        std::sync::Arc::new(tokio::sync::Notify::new()),
    );
    EXTRA_BARRIERS
        .lock()
        .unwrap()
        .insert(period.clone(), (arrived.clone(), resume.clone()));
    let task_db = db.clone();
    let task = tokio::spawn(async move { settle_usage_funding(&task_db, &a).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), arrived.notified())
        .await
        .unwrap();
    assert_eq!(
        settle_usage_funding(&db, &b)
            .await
            .unwrap()
            .wallet_charge_credits,
        Credits::ZERO
    );
    resume.notify_one();
    assert_eq!(task.await.unwrap().unwrap().wallet_charge_credits, pico(4));
    let period = db
        .collection::<UsageAllowancePeriod>(USAGE_ALLOWANCE_PERIODS)
        .find_one(doc! { "_id": period })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(period.consumed_quantity, 3);
    assert_eq!(period.reserved_quantity, 0);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn capped_pool_recovery_respects_renewed_lease_and_releases_pending_reclaim_once() {
    let (db, mut ctx, wallet, layers, id) = fixture("allocation_pool_cleanup").await;
    let now = Utc::now();
    ctx.pool_attempt = Some(crate::models::usage_meter::PoolAttemptAccounting {
        pool_id: Uuid::new_v4().to_string(),
        member_id: "member".into(),
        attempt: 1,
        lease_until: now + Duration::seconds(30),
        outcome: None,
        completion_cause: None,
    });
    reserve_and_open_pool(&db, &ctx, &wallet, layers)
        .await
        .unwrap();
    super::super::grants::request_revoke_grant(&db, &id, Some("admin"))
        .await
        .unwrap();
    super::super::pool_attempt::renew(&db, &ctx.billing_request_id, now + Duration::minutes(5), 1)
        .await
        .unwrap();
    assert_eq!(
        super::super::pool_attempt::recover_expired(&db, now + Duration::minutes(1))
            .await
            .unwrap(),
        0
    );
    assert_eq!(grant(&db, &id).await.reserved, pico(3));
    assert_eq!(
        super::super::pool_attempt::recover_expired(&db, now + Duration::minutes(6))
            .await
            .unwrap(),
        1
    );
    // Lease cleanup releases the wallet and marks the meter abandoned. The
    // following reconciler stage durably releases its benefits and reclaim.
    let released = grant(&db, &id).await;
    assert_eq!(released.terminal_amount, Credits::ZERO);
    assert_eq!(released.reserved, pico(3));
    assert_eq!(released.service_limits[0].reserved, pico(3));
    assert!(released.revoke_requested_at.is_some());
    assert_eq!(recover_terminal_releases(&db).await.unwrap(), 1);
    let current = grant(&db, &id).await;
    assert_eq!(current.terminal_amount, pico(10));
    assert_eq!(current.reserved, Credits::ZERO);
    assert_eq!(current.service_limits[0].reserved, Credits::ZERO);
    assert_eq!(
        super::super::pool_attempt::recover_expired(&db, now + Duration::minutes(7))
            .await
            .unwrap(),
        0
    );
    assert_eq!(recover_terminal_releases(&db).await.unwrap(), 0);
    assert_eq!(grant(&db, &id).await, current);
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! { "dedupe_key": format!("grant-revoked:{id}") })
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn public_cap_funding_replay_requires_same_live_authority() {
    use crate::models::platform_public_budget::{CALLS, PublicCallReservation, PublicCallState};
    let (db, mut ctx, wallet, layers, id) = fixture("allocation_public_authority").await;
    ctx.api_key_id = Some("public-key".into());
    ctx.user_service_id = None;
    let call_id = Uuid::new_v4().to_string();
    ctx.public_admission = Some(super::super::route_context::PublicBillingAdmission {
        call_id: call_id.clone(),
        model_id: "model".into(),
        expires_at: Utc::now() + Duration::minutes(1),
    });
    db.collection::<PublicCallReservation>(CALLS)
        .insert_one(PublicCallReservation {
            continuation: false,
            id: call_id.clone(),
            loop_id: Uuid::new_v4().to_string(),
            sponsor_owner_user_id: ctx.billing_owner_id.clone(),
            credential_api_key_id: "public-key".into(),
            billing_request_id: ctx.billing_request_id.clone(),
            route_digest: crate::services::platform_public_budget_service::route_digest(&ctx)
                .unwrap(),
            model_id: "model".into(),
            state: PublicCallState::Prepared,
            maximum: pico(10),
            cap_charge: Credits::ZERO,
            meter_row_ids: vec![],
            next_reconcile_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(1),
        })
        .await
        .unwrap();
    reserve_and_open_public(&db, &ctx, &wallet, layers.clone())
        .await
        .unwrap();
    assert_eq!(grant(&db, &id).await.service_limits[0].reserved, pico(3));
    assert!(
        reserve_and_open_public(&db, &ctx, &wallet, layers.clone())
            .await
            .is_ok()
    );
    let mut other = ctx.clone();
    other.public_admission.as_mut().unwrap().call_id = "different".into();
    assert!(
        reserve_and_open_public(&db, &other, &wallet, layers.clone())
            .await
            .is_err()
    );
    other = ctx.clone();
    other.public_admission.as_mut().unwrap().expires_at = Utc::now() - Duration::seconds(1);
    assert!(
        reserve_and_open_public(&db, &other, &wallet, layers)
            .await
            .is_err()
    );
    assert_eq!(
        db.collection::<UsageMeterRow>(USAGE_METER)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(grant(&db, &id).await.reserved, pico(3));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn managed_cap_funding_commits_authority_and_preserves_duplicate_boundaries() {
    let (db, mut ctx, wallet, layers, id) = fixture("allocation_managed_authority").await;
    let execution = Uuid::new_v4().to_string();
    let expires = Utc::now() + Duration::minutes(1);
    ctx.managed_admission = Some(super::super::route_context::ManagedBillingAdmission {
        execution_id: execution.clone(),
        route_digest: "digest".into(),
        expires_at: expires,
    });
    // Only the bridge's conditional update is used here; the full managed
    // lifecycle/dispatch integration suite validates its immutable contract.
    db.collection::<Document>(crate::models::managed_execution::COLLECTION_NAME).insert_one(doc! { "_id": &execution, "state": "prepared", "billing_request_id": &ctx.billing_request_id, "billing_owner_id": &ctx.billing_owner_id, "actor_user_id": &ctx.actor_user_id, "route_digest": "digest", "expires_at": bson::DateTime::from_chrono(expires) }).await.unwrap();
    reserve_and_open_managed(&db, &ctx, &wallet, layers.clone())
        .await
        .unwrap();
    assert_eq!(grant(&db, &id).await.service_limits[0].reserved, pico(3));
    let authority = db
        .collection::<Document>(crate::models::managed_execution::COLLECTION_NAME)
        .find_one(doc! { "_id": &execution })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(authority.get_str("state").unwrap(), "funded");
    assert_eq!(authority.get_array("meter_row_ids").unwrap().len(), 1);
    assert!(
        reserve_and_open_managed(&db, &ctx, &wallet, layers.clone())
            .await
            .is_ok()
    );
    ctx.managed_admission.as_mut().unwrap().route_digest = "other".into();
    assert!(
        reserve_and_open_managed(&db, &ctx, &wallet, layers)
            .await
            .is_err()
    );
    assert_eq!(grant(&db, &id).await.reserved, pico(3));
    db.drop().await.unwrap();
}
