use super::super::lago_client::{
    Entitlement, LagoAck, LagoError, LagoEvent, LagoUsage, OwnerProvisionInput,
};
use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;

#[derive(Default)]
struct Provider {
    pause_history: AtomicBool,
    entered: Notify,
    resume: Notify,
    history: Mutex<Vec<LagoWalletTransaction>>,
    voids: AtomicUsize,
    fail_void: AtomicBool,
    refuse_void: AtomicBool,
    dispatch_http: Option<super::super::lago_client::dispatch_test_support::DispatchHttp>,
    fail_history: AtomicBool,
}
#[async_trait::async_trait]
impl LagoApi for Provider {
    async fn ensure_customer(&self, _: &OwnerProvisionInput) -> AppResult<String> {
        unreachable!()
    }
    async fn ensure_subscription(&self, _: &str, _: &str) -> AppResult<String> {
        unreachable!()
    }
    async fn record_event(&self, _: &LagoEvent) -> Result<LagoAck, LagoError> {
        unreachable!()
    }
    async fn record_events_batch(&self, _: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        unreachable!()
    }
    async fn current_usage(&self, customer: &str, subscription: &str) -> AppResult<LagoUsage> {
        Ok(LagoUsage {
            customer_id: customer.into(),
            subscription_id: subscription.into(),
            raw: serde_json::json!({"customer_usage":{"amount_cents":0}}),
        })
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        Ok(7)
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        Ok(vec![])
    }
    async fn wallet_transactions(&self, _: &str) -> AppResult<Vec<LagoWalletTransaction>> {
        if self.fail_history.load(Ordering::SeqCst) {
            return Err(crate::errors::AppError::BillingProviderUnavailable(
                "history unavailable".into(),
            ));
        }
        let captured = self.history.lock().unwrap().clone();
        if self.pause_history.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.notified().await;
        }
        Ok(captured)
    }
    async fn void_wallet_credits(
        &self,
        _: &str,
        amount: Credits,
        operation_id: &str,
    ) -> AppResult<String> {
        assert_eq!(amount, Credits::from_whole(3));
        let number = self.voids.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_void.load(Ordering::SeqCst) || self.refuse_void.load(Ordering::SeqCst) {
            return Err(crate::errors::AppError::BillingProviderUnavailable(
                "ambiguous timeout".into(),
            ));
        }
        let receipt = receipt(operation_id, number);
        let id = receipt.id.clone();
        self.history.lock().unwrap().push(receipt);
        Ok(id)
    }
    async fn dispatch_wallet_void(
        &self,
        wallet: &str,
        amount: Credits,
        operation: &str,
        window: &DispatchWindow,
    ) -> super::super::lago_client::ProviderDispatch<String> {
        use super::super::lago_client::ProviderDispatch;
        if let Some(http) = &self.dispatch_http {
            return http.void(wallet, amount, operation, window).await;
        }
        match self.void_wallet_credits(wallet, amount, operation).await {
            Ok(receipt) => ProviderDispatch::Applied(receipt),
            Err(error) if self.refuse_void.load(Ordering::SeqCst) => {
                ProviderDispatch::Refused(error)
            }
            Err(error) => ProviderDispatch::Ambiguous(error),
        }
    }
}
fn receipt(operation_id: &str, number: usize) -> LagoWalletTransaction {
    LagoWalletTransaction {
        id: format!("void-{number}"),
        transaction_type: "outbound".into(),
        transaction_status: "voided".into(),
        status: "settled".into(),
        credit_amount: Credits::from_whole(3),
        remaining_credit: None,
        name: Some(purchased_credit_expiry_transaction_name(operation_id)),
        settled_at: Some(Utc::now()),
        created_at: Utc::now(),
    }
}
async fn fixture(
    marked: bool,
) -> (
    mongodb::Database,
    BillingWallet,
    PurchasedCreditExpiryOperation,
) {
    let db = crate::test_utils::connect_test_database("expiry_dispatch_2468")
        .await
        .unwrap();
    let now = bson::DateTime::now();
    let mut op = doc! {
        "operation_id": "operation",
        "processing_token": "worker-a",
        "dispatch_started_at": bson::Bson::Null,
        "lease_until": bson::DateTime::from_chrono(Utc::now() + Duration::seconds(600)),
        "amount": Credits::from_whole(3),
        "items": [{
            "lago_purchase_transaction_id": "purchase",
            "reference_id": "purchase",
            "amount": Credits::from_whole(3),
            "settled_at": now,
        }],
        "created_at": now,
        "updated_at": now,
    };
    if marked {
        op.insert("dispatch_started_at", now);
    }
    let wallet: BillingWallet = bson::from_document(doc! {
        "_id": "wallet",
        "owner_id": "owner",
        "lago_customer_id": "owner",
        "lago_wallet_id": "provider-wallet",
        "lago_subscription_id": "owner:starter",
        "plan_kind": "prepaid",
        "balance_credits": Credits::from_whole(10),
        "pending_topup_expiry_credits": Credits::from_whole(3),
        "collection_state": "good",
        "balance_synced_at": now,
        "created_at": now,
        "updated_at": now,
        "active_topup_expiry": op.clone(),
    })
    .unwrap();
    // Insert raw BSON so the red test also exercises a marker unknown to the base model.
    let mut raw = bson::to_document(&wallet).unwrap();
    raw.insert("active_topup_expiry", op.clone());
    db.collection::<bson::Document>(BILLING_WALLETS)
        .insert_one(raw)
        .await
        .unwrap();
    super::super::ledger::append_chained_entry(
        &db,
        super::super::ledger::exact_entry(
            "owner",
            "wallet",
            "wallet_funded",
            "fixture".into(),
            super::super::ledger::transfer(
                "wallet:owner".into(),
                "external:lago".into(),
                Credits::from_whole(10),
            ),
            None,
        ),
        &super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    )
    .await
    .unwrap();
    (db, wallet, bson::from_document(op).unwrap())
}
async fn expire_claim(db: &mongodb::Database) {
    db.collection::<bson::Document>(BILLING_WALLETS)
        .update_one(
            doc! { "_id": "wallet" },
            doc! { "$set": {
                "active_topup_expiry.lease_until": bson::DateTime::from_millis(0),
                "active_topup_expiry.processing_lease_until": bson::DateTime::from_millis(0),
            } },
        )
        .await
        .unwrap();
}
async fn claim(db: &mongodb::Database) -> (BillingWallet, PurchasedCreditExpiryOperation) {
    let wallet = db
        .collection::<BillingWallet>(BILLING_WALLETS)
        .find_one(doc! { "_id": "wallet" })
        .await
        .unwrap()
        .unwrap();
    let op = acquire_expiry_operation(
        db,
        &wallet,
        wallet.active_topup_expiry.clone().unwrap(),
        Utc::now(),
    )
    .await
    .unwrap()
    .unwrap();
    (wallet, op)
}
#[tokio::test]
async fn expiry_dispatch_stale_history_reader_never_posts_after_successor() {
    let (db, wallet, operation) = fixture(false).await;
    let provider = Provider {
        pause_history: AtomicBool::new(true),
        ..Default::default()
    };
    let (first, ()) = tokio::join!(
        complete_expiry_operation(&db, &provider, &wallet, operation, Utc::now()),
        async {
            provider.entered.notified().await;
            expire_claim(&db).await;
            let (wallet, op) = claim(&db).await;
            assert_eq!(
                complete_expiry_operation(&db, &provider, &wallet, op, Utc::now())
                    .await
                    .unwrap(),
                1
            );
            provider.resume.notify_one();
        }
    );
    first.unwrap();
    super::super::account_reconciliation::run_once(&db)
        .await
        .unwrap();
    let integrity = db
        .collection::<crate::models::chain_verify_status::ChainVerifyStatus>("chain_verify_status")
        .find_one(doc! {"_id":"billing_accounts"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        integrity.outcome,
        crate::models::chain_verify_status::ChainVerifyOutcome::Ok,
        "{:?}",
        integrity.break_detail
    );
    let calls = provider.voids.load(Ordering::SeqCst);
    db.drop().await.unwrap();
    assert_eq!(calls, 1, "stale history reader dispatched a second void");
}
#[tokio::test]
async fn expiry_dispatch_waits_for_history_and_recovers_even_after_quiet_period() {
    let (db, wallet, operation) = fixture(true).await;
    let provider = Provider::default();
    complete_expiry_operation(&db, &provider, &wallet, operation, Utc::now())
        .await
        .unwrap();
    let calls = provider.voids.load(Ordering::SeqCst);
    if calls != 0 {
        db.drop().await.unwrap();
    }
    assert_eq!(calls, 0, "dispatch marker must forbid a blind POST");
    let diagnostics = super::super::startup_diagnostics(&db).await.unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "billing_expiry_unresolved")
    );
    provider
        .history
        .lock()
        .unwrap()
        .push(receipt("operation", 1));
    db.collection::<bson::Document>(BILLING_WALLETS)
        .update_one(
            doc! { "_id": "wallet" },
            doc! { "$set": {
                "active_topup_expiry.dispatch_started_at": bson::DateTime::from_chrono(Utc::now() - Duration::hours(25)),
            }},
        )
        .await
        .unwrap();
    expire_claim(&db).await;
    let (wallet, op) = claim(&db).await;
    assert_eq!(
        complete_expiry_operation(&db, &provider, &wallet, op, Utc::now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(provider.voids.load(Ordering::SeqCst), 0);
    assert!(
        !super::super::startup_diagnostics(&db)
            .await
            .unwrap()
            .iter()
            .any(|d| d.code == "billing_expiry_unresolved")
    );
    db.drop().await.unwrap();
}
#[tokio::test]
async fn expiry_dispatch_ambiguous_post_does_not_repeat_on_next_pass() {
    let (db, wallet, operation) = fixture(false).await;
    let provider = Provider {
        fail_void: AtomicBool::new(true),
        ..Default::default()
    };
    assert!(
        complete_expiry_operation(&db, &provider, &wallet, operation, Utc::now())
            .await
            .is_err()
    );
    expire_claim(&db).await;
    let (wallet, op) = claim(&db).await;
    let _ = complete_expiry_operation(&db, &provider, &wallet, op, Utc::now()).await;
    let calls = provider.voids.load(Ordering::SeqCst);
    db.drop().await.unwrap();
    assert_eq!(
        calls, 1,
        "an ambiguous POST is not permission to send again"
    );
}

#[tokio::test]
async fn expiry_dispatch_claim_uses_database_time_not_sweep_start() {
    let (db, _, _) = fixture(false).await;
    expire_claim(&db).await;
    let wallet = db
        .collection::<BillingWallet>(BILLING_WALLETS)
        .find_one(doc! { "_id": "wallet" })
        .await
        .unwrap()
        .unwrap();
    let operation = acquire_expiry_operation(
        &db,
        &wallet,
        wallet.active_topup_expiry.clone().unwrap(),
        Utc::now() - Duration::hours(1),
    )
    .await
    .unwrap()
    .unwrap();
    let remaining = operation.lease_until - Utc::now();
    db.drop().await.unwrap();
    assert!(
        remaining > Duration::seconds(590),
        "claim reused a stale sweep deadline: {remaining}"
    );
}

#[tokio::test]
async fn expiry_dispatch_expired_token_cannot_post_without_a_successor() {
    let (db, wallet, operation) = fixture(false).await;
    expire_claim(&db).await;
    let provider = Provider::default();
    complete_expiry_operation(&db, &provider, &wallet, operation, Utc::now())
        .await
        .unwrap();
    let calls = provider.voids.load(Ordering::SeqCst);
    db.drop().await.unwrap();
    assert_eq!(calls, 0, "an expired token is not dispatch authority");
}

#[tokio::test]
async fn expiry_dispatch_renews_while_provider_history_is_blocked() {
    let (db, wallet, operation) = fixture(false).await;
    let before = operation.lease_until;
    let provider = Provider {
        pause_history: AtomicBool::new(true),
        ..Default::default()
    };
    let (result, ()) = tokio::join!(
        complete_with_renewal(
            &db,
            &provider,
            &wallet,
            operation,
            Utc::now(),
            std::time::Duration::from_millis(50)
        ),
        async {
            provider.entered.notified().await;
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let saved = db
                        .collection::<BillingWallet>(BILLING_WALLETS)
                        .find_one(doc! { "_id": "wallet" })
                        .await
                        .unwrap()
                        .unwrap();
                    if saved.active_topup_expiry.unwrap().lease_until > before {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            })
            .await
            .expect("operation claim was not renewed");
            provider.resume.notify_one();
        }
    );
    assert_eq!(result.unwrap(), 1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn expiry_dispatch_stale_release_preserves_live_successor() {
    let (db, wallet, operation) = fixture(false).await;
    expire_claim(&db).await;
    let (_, successor) = claim(&db).await;
    let provider = Provider::default();
    complete_expiry_operation(&db, &provider, &wallet, operation, Utc::now())
        .await
        .unwrap();
    let saved = db
        .collection::<BillingWallet>(BILLING_WALLETS)
        .find_one(doc! { "_id": "wallet" })
        .await
        .unwrap()
        .unwrap();
    let active = saved.active_topup_expiry.unwrap();
    assert_eq!(active.processing_token, successor.processing_token);
    assert_eq!(active.lease_until, successor.lease_until);
    assert_eq!(provider.voids.load(Ordering::SeqCst), 0);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn expiry_dispatch_unresolved_logs_once_per_operation() {
    use tracing::instrument::WithSubscriber;
    // With only one registered dispatcher, tracing may cache `never` when a
    // parallel test first visits this callsite without a scoped subscriber.
    // Keep two dispatchers alive so registration consults both subscribers.
    let _peer_dispatch = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
    for history_unavailable in [false, true] {
        let (db, wallet, operation) = fixture(true).await;
        let provider = Provider {
            fail_history: AtomicBool::new(history_unavailable),
            ..Default::default()
        };
        let capture = tempfile::NamedTempFile::new().unwrap();
        let writer = capture.reopen().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::ERROR)
            .with_writer(move || writer.try_clone().unwrap())
            .finish();
        async {
            let result =
                complete_expiry_operation(&db, &provider, &wallet, operation, Utc::now()).await;
            assert_eq!(result.is_err(), history_unavailable);
            for _ in 0..3 {
                expire_claim(&db).await;
                let (wallet, op) = claim(&db).await;
                let result =
                    complete_expiry_operation(&db, &provider, &wallet, op, Utc::now()).await;
                assert_eq!(result.is_err(), history_unavailable);
            }
        }
        .with_subscriber(subscriber)
        .await;
        let logs = std::fs::read_to_string(capture.path()).unwrap();
        let visible = super::super::startup_diagnostics(&db)
            .await
            .unwrap()
            .iter()
            .any(|d| d.code == "billing_expiry_unresolved");
        let reported = db
            .collection::<BillingWallet>(BILLING_WALLETS)
            .find_one(doc! { "_id": &wallet.id })
            .await
            .unwrap()
            .unwrap()
            .active_topup_expiry
            .unwrap()
            .unresolved_reported_at;
        db.drop().await.unwrap();
        assert_eq!(
            logs.matches("Purchased-credit expiry dispatch is unresolved")
                .count(),
            1,
            "history_unavailable={history_unavailable}: {logs}"
        );
        assert!(visible);
        assert!(reported.is_some());
        assert_eq!(provider.voids.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn expiry_dispatch_renewal_keeps_polling_the_wallet_transaction() {
    let (db, wallet, operation) = fixture(false).await;
    let work = async {
        let mut session = db.client().start_session().await?;
        session.start_transaction().await?;
        db.collection::<bson::Document>(BILLING_WALLETS)
            .update_one(
                doc! { "_id": &wallet.id },
                doc! { "$set": { "transaction_probe": true } },
            )
            .session(&mut session)
            .await?;
        // Renewal must wait for this transaction's lock while the runner keeps
        // polling the work that will commit and release it.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        session.commit_transaction().await?;
        Ok(1)
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        run_with_expiry_renewal(
            &db,
            &wallet,
            &operation,
            work,
            std::time::Duration::from_millis(50),
        ),
    )
    .await;
    db.drop().await.unwrap();
    assert_eq!(
        result
            .expect("renewal blocked the transaction that must release the wallet lock")
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn expiry_dispatch_stale_purchase_read_cannot_start_a_second_operation() {
    let (db, _, _) = fixture(false).await;
    db.collection::<bson::Document>(BILLING_WALLETS)
        .update_one(
            doc! { "_id": "wallet" },
            doc! {
                "$unset": { "active_topup_expiry": "" },
                "$set": { "pending_topup_expiry_credits": Credits::ZERO },
            },
        )
        .await
        .unwrap();
    let provider = Provider {
        pause_history: AtomicBool::new(true),
        history: Mutex::new(vec![LagoWalletTransaction {
            id: "purchase".into(),
            transaction_type: "inbound".into(),
            transaction_status: "purchased".into(),
            status: "settled".into(),
            credit_amount: Credits::from_whole(3),
            remaining_credit: Some(Credits::from_whole(3)),
            name: None,
            settled_at: Some(Utc::now() - Duration::days(366)),
            created_at: Utc::now() - Duration::days(366),
        }]),
        ..Default::default()
    };
    let (first, ()) = tokio::join!(
        expire_purchased_credits(&db, &provider, Utc::now()),
        async {
            provider.entered.notified().await;
            assert_eq!(
                expire_purchased_credits(&db, &provider, Utc::now())
                    .await
                    .unwrap(),
                1
            );
            provider.resume.notify_one();
        }
    );
    first.unwrap();
    let calls = provider.voids.load(Ordering::SeqCst);
    db.drop().await.unwrap();
    assert_eq!(
        calls, 1,
        "stale purchase read started another expiry operation"
    );
}

#[tokio::test]
async fn expiry_dispatch_uncertain_return_keeps_deadline_for_legacy_runners() {
    for scenario in [
        "marked",
        "post_error",
        "legacy_history_error",
        "new_shape_legacy_history_error",
    ] {
        let marked = scenario == "marked";
        let (db, wallet, operation) = fixture(marked).await;
        if scenario == "legacy_history_error" {
            db.collection::<bson::Document>(BILLING_WALLETS)
                .update_one(
                    doc! { "_id": &wallet.id },
                    doc! { "$unset": { "active_topup_expiry.dispatch_started_at": "" } },
                )
                .await
                .unwrap();
        }
        let deadline = operation.lease_until;
        let provider = Provider {
            fail_void: AtomicBool::new(true),
            fail_history: AtomicBool::new(scenario.ends_with("history_error")),
            ..Default::default()
        };
        let result =
            complete_expiry_operation(&db, &provider, &wallet, operation, Utc::now()).await;
        assert_eq!(result.is_ok(), marked);
        let saved = db
            .collection::<BillingWallet>(BILLING_WALLETS)
            .find_one(doc! { "_id": &wallet.id })
            .await
            .unwrap()
            .unwrap()
            .active_topup_expiry
            .unwrap();
        db.drop().await.unwrap();
        assert!(
            saved.lease_until >= deadline,
            "an uncertain dispatch must not become eligible sooner for an old runner"
        );
        assert_eq!(
            saved.processing_lease_until,
            Some(bson::DateTime::from_millis(0).to_chrono())
        );
        assert_eq!(
            saved.dispatch_started_at.is_some(),
            matches!(scenario, "marked" | "post_error")
        );
        assert_eq!(
            provider.voids.load(Ordering::SeqCst),
            if scenario == "post_error" { 1 } else { 0 }
        );
    }
}

#[tokio::test]
async fn expiry_dispatch_r2_refusal_releases_marker_and_retries() {
    let (db, wallet, op) = fixture(false).await;
    let provider = Provider {
        refuse_void: AtomicBool::new(true),
        ..Default::default()
    };
    assert!(
        complete_expiry_operation(&db, &provider, &wallet, op, Utc::now())
            .await
            .is_err()
    );
    let (wallet, op) = claim(&db).await;
    let cleared = op.dispatch_started_at.is_none();
    assert_eq!(wallet.pending_topup_expiry_credits, Credits::from_whole(3));
    provider.refuse_void.store(false, Ordering::SeqCst);
    let result = complete_expiry_operation(&db, &provider, &wallet, op, Utc::now()).await;
    db.drop().await.unwrap();
    assert!(cleared, "definite refusal left dispatch marker");
    assert_eq!(result.unwrap(), 1);
    assert_eq!(provider.voids.load(Ordering::SeqCst), 2);
    assert_eq!(provider.history.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn expiry_dispatch_r2_after_quiet_period_racing_passes_post_once_and_audit() {
    let (db, _, _) = fixture(true).await;
    expire_claim(&db).await;
    db.collection::<bson::Document>(BILLING_WALLETS).update_one(doc! {"_id":"wallet"},
        doc! {"$set": {"active_topup_expiry.dispatch_started_at": bson::DateTime::from_chrono(Utc::now()-Duration::hours(25))}}).await.unwrap();
    let (wallet, op) = claim(&db).await;
    let provider = Provider::default();
    let (a, b) = tokio::join!(
        complete_expiry_operation(&db, &provider, &wallet, op.clone(), Utc::now()),
        complete_expiry_operation(&db, &provider, &wallet, op, Utc::now())
    );
    let audits = db
        .collection::<bson::Document>("audit_log")
        .find(doc! {"event_type":"billing_expiry_redispatched"})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    db.drop().await.unwrap();
    a.unwrap();
    b.unwrap();
    assert_eq!(
        provider.voids.load(Ordering::SeqCst),
        1,
        "exactly one re-dispatch after quiet period"
    );
    assert_eq!(audits.len(), 1, "re-dispatch needs one durable audit row");
    assert_eq!(
        audits[0].get_document("event_data").unwrap(),
        &doc! {"wallet_id":"wallet", "operation_id":"operation", "attempt":2_i64}
    );
    assert!(audits[0].contains_key("seq"));
}
#[tokio::test]
async fn expiry_dispatch_r2_release_is_immediate_only_for_upgraded_readers() {
    let (db, wallet, op) = fixture(true).await;
    let provider = Provider::default();
    complete_expiry_operation(&db, &provider, &wallet, op, Utc::now())
        .await
        .unwrap();
    let raw = db
        .collection::<bson::Document>(BILLING_WALLETS)
        .find_one(doc! {"_id":"wallet"})
        .await
        .unwrap()
        .unwrap();
    let active = raw.get_document("active_topup_expiry").unwrap();
    let old_deadline = *active.get_datetime("lease_until").unwrap();
    let processing = active.get_datetime("processing_lease_until").ok().copied();
    let (_, successor) = claim(&db).await;
    db.drop().await.unwrap();
    assert!(
        old_deadline > bson::DateTime::now(),
        "old binary could immediately resend"
    );
    assert_eq!(processing, Some(bson::DateTime::from_millis(0)));
    assert_ne!(successor.processing_token, "worker-a");
}

#[tokio::test]
async fn expiry_dispatch_r2_each_ambiguous_attempt_waits_a_full_day() {
    let (db, wallet, op) = fixture(false).await;
    let provider = Provider {
        fail_void: AtomicBool::new(true),
        ..Default::default()
    };
    assert!(
        complete_expiry_operation(&db, &provider, &wallet, op, Utc::now())
            .await
            .is_err()
    );
    for attempt in 1..=3 {
        for _ in 0..3 {
            let (wallet, op) = claim(&db).await;
            assert_eq!(
                complete_expiry_operation(&db, &provider, &wallet, op, Utc::now())
                    .await
                    .unwrap(),
                0
            );
        }
        assert_eq!(provider.voids.load(Ordering::SeqCst), attempt);
        db.collection::<bson::Document>(BILLING_WALLETS).update_one(doc! {"_id":"wallet"},
            doc! {"$set": {"active_topup_expiry.dispatch_started_at": bson::DateTime::from_chrono(Utc::now()-Duration::hours(25))}}).await.unwrap();
        let (wallet, op) = claim(&db).await;
        assert!(
            complete_expiry_operation(&db, &provider, &wallet, op, Utc::now())
                .await
                .is_err(),
            "elapsed quiet period must admit a fresh attempt"
        );
        assert_eq!(provider.voids.load(Ordering::SeqCst), attempt + 1);
    }
    assert_eq!(
        db.collection::<bson::Document>("audit_log")
            .count_documents(doc! {"event_type":"billing_expiry_redispatched"})
            .await
            .unwrap(),
        3
    );
    assert!(
        super::super::startup_diagnostics(&db)
            .await
            .unwrap()
            .iter()
            .any(|d| d.code == "billing_expiry_unresolved")
    );
    provider
        .history
        .lock()
        .unwrap()
        .push(receipt("operation", 99));
    let (wallet, op) = claim(&db).await;
    assert_eq!(
        complete_expiry_operation(&db, &provider, &wallet, op, Utc::now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(provider.voids.load(Ordering::SeqCst), 4);
    assert!(
        !super::super::startup_diagnostics(&db)
            .await
            .unwrap()
            .iter()
            .any(|d| d.code == "billing_expiry_unresolved")
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn expiry_dispatch_old_takeover_restores_legacy_lease_authority() {
    let (db, wallet, op) = fixture(true).await;
    complete_expiry_operation(&db, &Provider::default(), &wallet, op, Utc::now())
        .await
        .unwrap();
    db.collection::<bson::Document>(BILLING_WALLETS).update_one(doc! {"_id":"wallet"},
        doc! {"$set": {"active_topup_expiry.processing_token":"legacy-worker",
            "active_topup_expiry.lease_until": bson::DateTime::from_chrono(Utc::now()+Duration::seconds(600))}}).await.unwrap();
    let wallet = db
        .collection::<BillingWallet>(BILLING_WALLETS)
        .find_one(doc! {"_id":"wallet"})
        .await
        .unwrap()
        .unwrap();
    assert!(
        acquire_expiry_operation(
            &db,
            &wallet,
            wallet.active_topup_expiry.clone().unwrap(),
            Utc::now()
        )
        .await
        .unwrap()
        .is_none()
    );
    expire_claim(&db).await;
    let (_, successor) = claim(&db).await;
    assert_ne!(successor.processing_token, "legacy-worker");
    db.drop().await.unwrap();
}

async fn assert_expiry_sending_window(monotonic: u64, wall: u64, sends: bool) {
    let (db, wallet, op) = fixture(false).await;
    let http =
        super::super::lago_client::dispatch_test_support::DispatchHttp::new(&db, monotonic, wall)
            .await;
    let provider = Provider {
        dispatch_http: Some(http),
        ..Default::default()
    };
    let result = complete_expiry_operation(&db, &provider, &wallet, op, Utc::now()).await;
    let saved = db
        .collection::<BillingWallet>(BILLING_WALLETS)
        .find_one(doc! {"_id": &wallet.id})
        .await
        .unwrap()
        .unwrap();
    let diagnostic = unresolved_diagnostics(&db).await.unwrap();
    let count = provider.dispatch_http.as_ref().unwrap().posts();
    db.drop().await.unwrap();
    assert_eq!(
        count,
        usize::from(sends),
        "expired sender must make no HTTP request"
    );
    assert_eq!(result.is_ok(), sends);
    if !sends {
        let op = saved.active_topup_expiry.unwrap();
        assert!(op.dispatch_started_at.is_some());
        assert!(op.lago_void_transaction_id.is_none());
        assert_eq!(
            op.processing_lease_until,
            Some(bson::DateTime::from_millis(0).to_chrono())
        );
        assert_eq!(saved.pending_topup_expiry_credits, Credits::from_whole(3));
        assert_eq!(diagnostic.len(), 1);
    } else {
        assert!(saved.active_topup_expiry.is_none());
        assert!(diagnostic.is_empty());
    }
}

#[tokio::test]
async fn expiry_sending_window_rejects_monotonic_delay() {
    assert_expiry_sending_window(31, 0, false).await;
}
#[tokio::test]
async fn expiry_sending_window_rejects_wall_clock_delay() {
    assert_expiry_sending_window(0, 31, false).await;
}
#[tokio::test]
async fn expiry_sending_window_allows_prompt_send() {
    assert_expiry_sending_window(1, 1, true).await;
}
