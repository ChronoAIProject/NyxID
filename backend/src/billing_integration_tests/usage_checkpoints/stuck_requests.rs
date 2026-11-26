mod ownership;
use super::*;

#[tokio::test]
async fn stuck_request_dead_letter_final_quantity_retries_without_writing_it_off() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    let old = bson::DateTime::from_chrono(Utc::now() - Duration::hours(2));
    f.db.collection::<bson::Document>(USAGE_METER)
        .update_many(
            doc! {},
            doc! {"$set": {
                "status": "dead_letter", "quantity": 3_i64,
                "last_error": "billing settlement retries exhausted",
                "finalized_at": old, "updated_at": old,
                "settlement_attempts": reservation::MAX_SETTLEMENT_ATTEMPTS,
                "settlement_next_retry_at": old,
            }},
        )
        .await
        .unwrap();
    f.count();
    reservation::recover_retryable_settlements(&f.db)
        .await
        .unwrap();
    let recovered_commands = f.count();
    reservation::recover_retryable_settlements(&f.db)
        .await
        .unwrap();
    let replay_commands = f.count();
    println!(
        "STUCK_REQUEST_KNOWN_RETRY_COMMANDS recovered={recovered_commands} idle_replay={replay_commands}"
    );
    let row = f.row().await;
    let held = wallet(&f.db, &f.owner).await.reserved_credits;
    f.charged(3).await;
    f.db.drop().await.unwrap();
    assert!(
        row.released,
        "known final usage must retry after dead-lettering"
    );
    assert_eq!(
        row.quantity,
        Some(3),
        "unknown-consumption policy must not change final evidence"
    );
    assert_eq!(held, Credits::ZERO);
}

#[tokio::test]
async fn stuck_request_zero_legacy_final_quantity_needs_no_missing_rate() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    f.db.collection::<bson::Document>(USAGE_METER)
        .update_many(
            doc! {},
            doc! {
                "$set": {"status": "finalized", "quantity": 0_i64,
                    "finalized_at": bson::DateTime::from_millis(0)},
                "$unset": {"funding": ""},
            },
        )
        .await
        .unwrap();
    f.db.collection::<bson::Document>("billing_rate_cache")
        .delete_many(doc! {})
        .await
        .unwrap();
    reservation::recover_retryable_settlements(&f.db)
        .await
        .unwrap();
    let row = f.row().await;
    let held = wallet(&f.db, &f.owner).await.reserved_credits;
    f.clean().await;
    f.db.drop().await.unwrap();
    assert!(
        row.released,
        "a deliberate zero final quantity needs no rate to return its legacy hold"
    );
    assert_eq!(held, Credits::ZERO);
}

#[tokio::test]
async fn stuck_request_dead_letter_waits_between_failures_and_keeps_known_quantity() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    let old = bson::DateTime::from_millis(0);
    f.db.collection::<bson::Document>(USAGE_METER)
        .update_many(
            doc! {},
            doc! {
                "$set": {
                    "status": "dead_letter", "quantity": 3_i64,
                    "last_error": "billing settlement retries exhausted",
                    "finalized_at": old, "updated_at": old,
                    "settlement_attempts": reservation::MAX_SETTLEMENT_ATTEMPTS,
                },
                "$unset": {"funding": ""},
            },
        )
        .await
        .unwrap();
    f.db.collection::<bson::Document>(BILLING_RATE_CACHE)
        .delete_many(doc! {})
        .await
        .unwrap();
    reservation::recover_retryable_settlements(&f.db)
        .await
        .unwrap();
    let failed = f.row().await;
    assert_eq!(failed.status, UsageStatus::DeadLetter);
    assert_eq!(failed.quantity, Some(3));
    assert_eq!(
        failed.settlement_attempts,
        reservation::MAX_SETTLEMENT_ATTEMPTS + 1
    );
    assert!(
        !failed.released,
        "nonzero consumption still needs its historical rate"
    );
    reservation::recover_retryable_settlements(&f.db)
        .await
        .unwrap();
    assert_eq!(
        f.row().await.settlement_attempts,
        failed.settlement_attempts
    );
    f.clean().await;

    insert_fresh_rates(&f.db).await;
    f.db.collection::<bson::Document>(USAGE_METER)
        .update_many(doc! {}, doc! {"$set": {"updated_at": old}})
        .await
        .unwrap();
    reservation::recover_retryable_settlements(&f.db)
        .await
        .unwrap();
    reservation::recover_retryable_settlements(&f.db)
        .await
        .unwrap();
    f.charged(3).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn stuck_request_known_dead_letter_recovers_after_each_wallet_handoff() {
    use crate::billing_integration_tests::hold_release::{Phase as MoneyPhase, crash_money_at};
    for phase in [
        MoneyPhase::WalletLocked,
        MoneyPhase::WalletApplied,
        MoneyPhase::WalletReleased,
    ] {
        let f = Fixture::new().await;
        let _metered = f.open(true).await;
        let old = bson::DateTime::from_millis(0);
        f.db.collection::<bson::Document>(USAGE_METER)
            .update_many(
                doc! {},
                doc! {"$set": {
                    "status": "dead_letter", "quantity": 3_i64,
                    "last_error": "billing settlement retries exhausted",
                    "finalized_at": old, "updated_at": old,
                    "settlement_attempts": reservation::MAX_SETTLEMENT_ATTEMPTS,
                }},
            )
            .await
            .unwrap();
        crash_money_at(phase, reservation::recover_retryable_settlements(&f.db)).await;
        f.clean().await;
        // Model the next eligible pass without sleeping. The survivor uses
        // the ordinary applied-lock completion before another retry can run.
        f.db.collection::<bson::Document>(USAGE_METER)
            .update_many(doc! {}, doc! {"$set": {"settlement_next_retry_at": old}})
            .await
            .unwrap();
        reservation::recover_retryable_settlements(&f.db)
            .await
            .unwrap();
        // An unapplied lock can first return to dead letter; its next slow
        // attempt is due one hour later. Preserve the actual quantity.
        f.db.collection::<bson::Document>(USAGE_METER)
            .update_many(
                doc! {"status": "dead_letter"},
                doc! {"$set": {"updated_at": old}},
            )
            .await
            .unwrap();
        reservation::recover_retryable_settlements(&f.db)
            .await
            .unwrap();
        reservation::recover_retryable_settlements(&f.db)
            .await
            .unwrap();
        f.charged(3).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn stuck_request_age_alone_does_not_end_unknown_or_provider_delivery_dead_letters() {
    for (status, quantity, released, error) in [
        ("forwarded", None, false, ""),
        (
            "dead_letter",
            None,
            false,
            "billing settlement retries exhausted",
        ),
        (
            "dead_letter",
            Some(3_i64),
            true,
            "Lago rejected usage event",
        ),
    ] {
        let f = Fixture::new().await;
        let _metered = f.open(true).await;
        f.db.collection::<bson::Document>(USAGE_METER)
            .update_many(
                doc! {},
                doc! {"$set": {
                    "status": status, "quantity": quantity, "released": released,
                    "last_error": error,
                    "updated_at": bson::DateTime::from_millis(0),
                    "settlement_attempts": reservation::MAX_SETTLEMENT_ATTEMPTS,
                }},
            )
            .await
            .unwrap();
        let before =
            f.db.collection::<bson::Document>(USAGE_METER)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
        reservation::recover_retryable_settlements(&f.db)
            .await
            .unwrap();
        let after =
            f.db.collection::<bson::Document>(USAGE_METER)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
        f.db.drop().await.unwrap();
        assert_eq!(before, after, "{status}, {quantity:?}, {released}");
    }
}

mod exit;
