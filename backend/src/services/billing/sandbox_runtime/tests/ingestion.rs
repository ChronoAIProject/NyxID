use super::*;
use crate::services::feature_flag_service::{self as flags, FlagTarget};

async fn set_flag(f: &Fixture, target: FlagTarget, value: Option<bool>) {
    match value {
        Some(value) => flags::set_platform_override(
            &f.db,
            flags::CMA_SANDBOX_BILLING_FLAG_KEY,
            &target,
            value,
            &f.owner,
        )
        .await
        .map(|_| ())
        .unwrap(),
        None => flags::clear_platform_override(&f.db, flags::CMA_SANDBOX_BILLING_FLAG_KEY, &target)
            .await
            .map(|_| ())
            .unwrap(),
    }
}

#[tokio::test]
async fn cma_flag_defaults_off_and_global_user_overrides_control_new_coverage() {
    let mut f = fixture("sandbox_flags").await;
    let input = request(&f, 2).await;
    let user = FlagTarget::User(f.owner.clone());
    set_flag(&f, user.clone(), None).await;
    assert!(
        !flags::find_flag(flags::CMA_SANDBOX_BILLING_FLAG_KEY)
            .unwrap()
            .default_enabled
    );
    for _ in 0..2 {
        assert!(matches!(
            f.billing
                .reserve_sandbox_window(&f.producer, input.clone())
                .await,
            Err(AppError::BillingProviderUnavailable(_))
        ));
        assert!(
            f.billing
                .quote_sandbox(&f.producer, &f.owner, &input.sandbox_id, allocation())
                .await
                .is_err()
        );
    }
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    assert_eq!(
        f.db.collection::<Document>(USAGE)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    set_flag(&f, FlagTarget::Global, Some(true)).await;
    set_flag(&f, user.clone(), Some(false)).await;
    assert!(
        f.billing
            .reserve_sandbox_window(&f.producer, input.clone())
            .await
            .is_err()
    );
    set_flag(&f, user.clone(), None).await;
    let admitted = f
        .billing
        .reserve_sandbox_window(&f.producer, input.clone())
        .await
        .unwrap();
    let held = wallet(&f).await.reserved_credits;
    set_flag(&f, FlagTarget::Global, Some(false)).await;
    assert_eq!(
        f.billing
            .reserve_sandbox_window(&f.producer, input.clone())
            .await
            .unwrap()
            .id,
        admitted.id
    );
    assert_eq!(
        f.billing
            .sandbox_window_receipt(&f.producer, &input.request_id)
            .await
            .unwrap()
            .unwrap()
            .id,
        admitted.id
    );
    assert_eq!(wallet(&f).await.reserved_credits, held);
    let mut another = input.clone();
    another.request_id = Uuid::new_v4().to_string();
    another.previous_window_id = Some(admitted.id.clone());
    assert!(
        f.billing
            .reserve_sandbox_window(&f.producer, another.clone())
            .await
            .is_err()
    );
    set_flag(&f, user, Some(true)).await;
    Arc::make_mut(&mut f.billing.config).cma_billing_enabled = false;
    assert_eq!(
        f.billing
            .sandbox_window_receipt(&f.producer, &input.request_id)
            .await
            .unwrap()
            .unwrap()
            .id,
        admitted.id,
        "fleet disable must preserve receipt recovery"
    );
    assert!(
        f.billing
            .reserve_sandbox_window(&f.producer, another)
            .await
            .is_err()
    );
    let window = elapsed(&f, admitted).await;
    assert!(
        f.billing
            .finalize_sandbox_window(&f.producer, evidence(&window))
            .await
            .unwrap()
            .evidence_digest
            .is_some()
    );
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    for component in &window.components {
        let row =
            f.db.collection::<UsageMeterRow>(USAGE)
                .find_one(doc! {"_id": &component.usage_row_id})
                .await
                .unwrap()
                .unwrap();
        assert!(
            row.funding.unwrap().settled,
            "fleet disable must preserve exact settlement"
        );
    }
}

#[tokio::test]
async fn ordered_cursor_survives_restart_and_shared_boundary_revision_replay() {
    let f = fixture("sandbox_cursor").await;
    let first_input = request(&f, 10).await;
    let first = f
        .billing
        .reserve_sandbox_window(&f.producer, first_input.clone())
        .await
        .unwrap();
    let mut renewal = first_input;
    renewal.request_id = Uuid::new_v4().to_string();
    renewal.previous_window_id = Some(first.id.clone());
    let second = f
        .billing
        .reserve_sandbox_window(&f.producer, renewal.clone())
        .await
        .unwrap();
    let second = elapsed(&f, second).await;
    let mut first = first;
    first.covered_until = second.covered_from;
    first.covered_from = first.covered_until - 2;
    f.db.collection::<SandboxBillingWindow>(WINDOW_COLLECTION)
        .replace_one(doc! {"_id": &first.id}, &first)
        .await
        .unwrap();
    let retained = vec![interval(
        40,
        first.covered_from,
        second.covered_until,
        allocation(),
    )];
    let mut proof = evidence(&second);
    proof.intervals = retained.clone();
    assert!(matches!(
        f.billing
            .finalize_sandbox_window(&f.producer, proof.clone())
            .await,
        Err(AppError::Conflict(_))
    ));
    let cursor = f
        .billing
        .sandbox_evidence_cursor(&f.producer, &second.sandbox_id)
        .await
        .unwrap()
        .unwrap();
    assert!(cursor.last_evidence_revision.is_none());
    assert_eq!(
        f.db.collection::<Document>(EVIDENCE_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let mut first_proof = evidence(&first);
    first_proof.intervals = retained;
    f.billing
        .finalize_sandbox_window(&f.producer, first_proof.clone())
        .await
        .unwrap();
    let restarted =
        BillingService::new_with_lago(f.db.clone(), f.billing.config.clone(), f.provider.clone());
    let cursor = restarted
        .sandbox_evidence_cursor(&f.producer, &second.sandbox_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cursor.last_evidence_revision, Some(40));
    assert_eq!(cursor.evidence_covered_until, Some(first.covered_until));
    for revision in [39, 42] {
        let mut unordered = proof.clone();
        unordered.intervals[0].revision = revision;
        assert!(matches!(
            restarted
                .finalize_sandbox_window(&f.producer, unordered)
                .await,
            Err(AppError::Conflict(_))
        ));
    }
    // A boundary revision may not be rewritten even if both arithmetic and
    // the window-local quantity would otherwise pass validation.
    let mut changed = proof.clone();
    changed.intervals[0] = interval(
        40,
        first.covered_from,
        second.covered_until,
        SandboxAllocation {
            cpu_millis: 0,
            memory_mib: 0,
            disk_gib: 10,
        },
    );
    assert!(matches!(
        restarted
            .finalize_sandbox_window(&f.producer, changed)
            .await,
        Err(AppError::Conflict(_))
    ));
    restarted
        .finalize_sandbox_window(&f.producer, proof.clone())
        .await
        .unwrap();
    let debit = wallet(&f).await.pending_lago_debits;
    let ledger_count =
        f.db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap();
    restarted
        .finalize_sandbox_window(&f.producer, first_proof)
        .await
        .unwrap();
    restarted
        .finalize_sandbox_window(&f.producer, proof)
        .await
        .unwrap();
    assert_eq!(wallet(&f).await.pending_lago_debits, debit);
    assert_eq!(
        f.db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        ledger_count
    );
    let cursor = restarted
        .sandbox_evidence_cursor(&f.producer, &second.sandbox_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cursor.evidence_covered_until, Some(second.covered_until));
    assert_eq!(cursor.last_evidence_window_id, Some(second.id.clone()));
    assert_eq!(
        f.db.collection::<Document>(EVIDENCE_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        restarted
            .sandbox_window_receipt(&f.producer, &renewal.request_id)
            .await
            .unwrap()
            .unwrap()
            .last_usage_revision,
        Some(40)
    );
}

#[tokio::test]
async fn late_component_failure_rolls_back_evidence_cursor_and_all_quantities() {
    let f = fixture("sandbox_ingest_rollback").await;
    let input = request(&f, 2).await;
    let window = elapsed(
        &f,
        f.billing
            .reserve_sandbox_window(&f.producer, input)
            .await
            .unwrap(),
    )
    .await;
    let held = wallet(&f).await.reserved_credits;
    let last_id = &window.components[2].usage_row_id;
    // Fail after the first two quantity updates, then repair the persisted row
    // to prove the exact retained submission remains retryable after rollback.
    f.db.collection::<Document>(USAGE)
        .update_one(doc! {"_id":last_id}, doc! {"$set":{"released":true}})
        .await
        .unwrap();
    assert!(matches!(
        f.billing
            .finalize_sandbox_window(&f.producer, evidence(&window))
            .await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(
        f.db.collection::<Document>(EVIDENCE_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.db.collection::<Document>(USAGE)
            .count_documents(doc! {"quantity":{"$ne":bson::Bson::Null}})
            .await
            .unwrap(),
        0
    );
    assert!(
        f.billing
            .sandbox_evidence_cursor(&f.producer, &window.sandbox_id)
            .await
            .unwrap()
            .unwrap()
            .last_evidence_revision
            .is_none()
    );
    assert_eq!(wallet(&f).await.reserved_credits, held);
    f.db.collection::<Document>(USAGE)
        .update_one(doc! {"_id":last_id}, doc! {"$set":{"released":false}})
        .await
        .unwrap();
    f.billing
        .finalize_sandbox_window(&f.producer, evidence(&window))
        .await
        .unwrap();
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
}

#[tokio::test]
async fn personal_owner_and_live_account_authority_are_required() {
    let f = fixture("sandbox_owner_authority").await;
    let input = request(&f, 2).await;
    f.db.collection::<Document>("users")
        .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"user_type":"org"}})
        .await
        .unwrap();
    assert!(
        f.billing
            .reserve_sandbox_window(&f.producer, input.clone())
            .await
            .is_err()
    );
    f.db.collection::<Document>("users")
        .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"user_type":"person"}})
        .await
        .unwrap();
    f.db.collection::<Document>("service_accounts")
        .update_one(
            doc! {"_id":&f.producer.id},
            doc! {"$inc":{"credential_generation":1_i64}},
        )
        .await
        .unwrap();
    assert!(matches!(
        f.billing.reserve_sandbox_window(&f.producer, input).await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
}

#[tokio::test]
async fn retained_evidence_recovers_settlement_and_ambiguous_lago_delivery_once() {
    let f = fixture("sandbox_reconcile_recovery").await;
    let input = request(&f, 2).await;
    let window = elapsed(
        &f,
        f.billing
            .reserve_sandbox_window(&f.producer, input)
            .await
            .unwrap(),
    )
    .await;
    let wallets = f.db.collection::<Document>(WALLETS);
    let saved_wallet = wallets
        .find_one_and_delete(doc! {"owner_id":&f.owner})
        .await
        .unwrap()
        .unwrap();
    let accepted = f
        .billing
        .finalize_sandbox_window(&f.producer, evidence(&window))
        .await
        .unwrap();
    assert!(accepted.evidence_digest.is_some());
    assert_eq!(
        f.db.collection::<Document>(USAGE)
            .count_documents(doc! {"quantity":{"$ne":bson::Bson::Null}})
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        f.db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    wallets.insert_one(saved_wallet).await.unwrap();
    let recovered = super::super::super::reservation::recover_retryable_settlements_at(
        &f.db,
        Utc::now() + Duration::hours(1),
    )
    .await
    .unwrap();
    assert_eq!(recovered.recovered, 3);
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    let pending = wallet(&f).await.pending_lago_debits;
    assert_eq!(pending, "0.00002044".parse().unwrap());
    let rows = f.db.collection::<Document>(USAGE);
    let age =
        doc! {"$set":{"updated_at":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(1))}};
    rows.update_many(doc! {}, age.clone()).await.unwrap();
    f.provider.available.store(false, Ordering::SeqCst);
    let retry = f.billing.reconciler().run_once().await.unwrap();
    assert_eq!(retry.retried, 3);
    assert!(f.provider.events.lock().unwrap().is_empty());
    assert_eq!(wallet(&f).await.pending_lago_debits, pending);
    f.provider.available.store(true, Ordering::SeqCst);
    f.provider.lost_ack.store(true, Ordering::SeqCst);
    rows.update_many(doc! {}, age.clone()).await.unwrap();
    let ambiguous = f.billing.reconciler().run_once().await.unwrap();
    assert_eq!(ambiguous.pushed, 2);
    assert_eq!(ambiguous.retried, 1);
    assert_eq!(f.provider.events.lock().unwrap().len(), 3);
    rows.update_many(doc! {}, age).await.unwrap();
    f.billing.reconciler().run_once().await.unwrap();
    assert_eq!(
        rows.count_documents(doc! {"lago_acked":true})
            .await
            .unwrap(),
        3
    );
    assert_eq!(f.provider.events.lock().unwrap().len(), 3);
    assert_eq!(f.provider.attempts.load(Ordering::SeqCst), 4);
    assert_eq!(wallet(&f).await.pending_lago_debits, pending);
    f.billing
        .finalize_sandbox_window(&f.producer, evidence(&window))
        .await
        .unwrap();
    f.billing.reconciler().run_once().await.unwrap();
    assert_eq!(f.provider.attempts.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn concurrent_identical_admission_and_evidence_retries_converge() {
    let f = fixture("sandbox_duplicate_race").await;
    let input = request(&f, 2).await;
    let (a, b) = tokio::join!(
        f.billing.reserve_sandbox_window(&f.producer, input.clone()),
        f.billing.reserve_sandbox_window(&f.producer, input.clone())
    );
    assert!(a.is_ok() || b.is_ok());
    let receipt = f
        .billing
        .reserve_sandbox_window(&f.producer, input)
        .await
        .unwrap();
    for winner in [a, b].into_iter().filter_map(Result::ok) {
        assert_eq!(winner.id, receipt.id);
    }
    assert_eq!(
        f.db.collection::<Document>(WINDOW_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.db.collection::<Document>(USAGE)
            .count_documents(doc! {})
            .await
            .unwrap(),
        3
    );
    let window = elapsed(&f, receipt).await;
    let proof = evidence(&window);
    let (a, b) = tokio::join!(
        f.billing
            .finalize_sandbox_window(&f.producer, proof.clone()),
        f.billing
            .finalize_sandbox_window(&f.producer, proof.clone())
    );
    assert!(a.is_ok() || b.is_ok());
    f.billing
        .finalize_sandbox_window(&f.producer, proof)
        .await
        .unwrap();
    assert_eq!(
        wallet(&f).await.pending_lago_debits,
        "0.00002044".parse().unwrap()
    );
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
}
