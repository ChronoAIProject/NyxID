use super::super::lago_client::{
    Entitlement, LagoAck, LagoError, LagoEvent, LagoTaxConfig, LagoUsage, OwnerProvisionInput,
};
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;

#[path = "retry_tests.rs"]
mod retry_tests;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Captured,
    GuardsWritten,
    InvalidationAdvanced,
    RepairBeforeCommit,
    SweepSelected,
    ProviderReturned,
    BeforePublication,
}
#[derive(Clone)]
struct Gate {
    phase: Phase,
    entered: Arc<Notify>,
    resume: Arc<Notify>,
    used: Arc<AtomicBool>,
}
impl Gate {
    fn new(phase: Phase) -> Self {
        Self {
            phase,
            entered: Arc::new(Notify::new()),
            resume: Arc::new(Notify::new()),
            used: Arc::new(AtomicBool::new(false)),
        }
    }
    async fn entered(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(10), self.entered.notified())
            .await
            .unwrap();
    }
}
tokio::task_local! { static GATE: Gate; }
pub(super) async fn point(phase: Phase) {
    if let Ok(gate) = GATE.try_with(Clone::clone)
        && gate.phase == phase
        && !gate.used.swap(true, Ordering::SeqCst)
    {
        gate.entered.notify_one();
        gate.resume.notified().await;
    }
}
struct Provider {
    rate: &'static str,
    plan: &'static str,
    calls: AtomicUsize,
}
impl Provider {
    fn new(rate: &'static str) -> Self {
        Self {
            rate,
            plan: "standard",
            calls: AtomicUsize::new(0),
        }
    }
    fn config(&self) -> LagoTaxConfig {
        let mut config = zero_config();
        config.plan["code"] = serde_json::json!(self.plan);
        config.plan["taxes"] = serde_json::json!([{"code":"tax","name":"Tax","rate":self.rate}]);
        config
    }
}
#[async_trait::async_trait]
impl LagoApi for Provider {
    async fn tax_config(&self, _: &str, _: &str) -> AppResult<LagoTaxConfig> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.config())
    }
    async fn ensure_customer(&self, _: &OwnerProvisionInput) -> AppResult<String> {
        unreachable!()
    }
    async fn ensure_subscription(&self, _: &str, _: &str) -> AppResult<String> {
        Ok("repaired".into())
    }
    async fn record_event(&self, _: &LagoEvent) -> Result<LagoAck, LagoError> {
        unreachable!()
    }
    async fn record_events_batch(&self, _: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        unreachable!()
    }
    async fn current_usage(&self, _: &str, _: &str) -> AppResult<LagoUsage> {
        unreachable!()
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        unreachable!()
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        Ok(vec![])
    }
}
const WALLETS: &str = crate::models::billing_wallet::COLLECTION_NAME;
async fn fixture(existing: bool) -> (Database, BillingWallet) {
    let db = crate::test_utils::connect_transaction_test_database("tax_ordering").await;
    fixture_on_db(db, existing).await
}
async fn fixture_on_db(db: Database, existing: bool) -> (Database, BillingWallet) {
    db.collection::<Document>(COLLECTION_NAME)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"owner_id":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let now = bson::DateTime::now();
    db.collection::<Document>(WALLETS).insert_one(doc! {
        "_id":"wallet", "owner_id":"owner", "lago_customer_id":"owner", "lago_subscription_id":"sub",
        "plan_kind":"prepaid", "collection_state":"good", "balance_synced_at":now, "created_at":now, "updated_at":now,
    }).await.unwrap();
    let wallet = db
        .collection::<BillingWallet>(WALLETS)
        .find_one(doc! {"_id":"wallet"})
        .await
        .unwrap()
        .unwrap();
    if existing {
        let mut profile = resolve_config("owner", "sub", &Provider::new("1").config()).unwrap();
        profile.fetched_at = chrono::DateTime::UNIX_EPOCH;
        let mut legacy = bson::to_document(&profile).unwrap();
        legacy.remove("revision");
        db.collection::<Document>(COLLECTION_NAME)
            .insert_one(legacy)
            .await
            .unwrap();
    }
    (db, wallet)
}
async fn profile(db: &Database) -> Option<BillingTaxProfile> {
    db.collection::<BillingTaxProfile>(COLLECTION_NAME)
        .find_one(doc! {"owner_id":"owner"})
        .await
        .unwrap()
}
fn rate(profile: &BillingTaxProfile) -> Credits {
    snapshot(profile, "requests").unwrap().rate_percent
}
async fn invalidate(db: &Database, owner: bool, plan: &str) {
    if owner {
        super::super::webhook::handle_lago_webhook_event(
            db,
            None,
            "customer.updated",
            &serde_json::json!({"customer":{"external_id":"owner"}}),
            900,
        )
        .await
        .unwrap();
    } else {
        invalidate_plan(db, plan).await.unwrap();
    }
}
async fn two_publishers(existing: bool) {
    for phase in [
        Phase::Captured,
        Phase::ProviderReturned,
        Phase::BeforePublication,
    ] {
        let (db, wallet) = fixture(existing).await;
        let old = Provider::new("5");
        let new = Provider::new("9");
        let gate = Gate::new(phase);
        let (old_result, ()) = tokio::join!(
            GATE.scope(gate.clone(), refresh(&db, &old, &wallet, 900)),
            async {
                gate.entered().await;
                refresh(&db, &new, &wallet, 900).await.unwrap();
                gate.resume.notify_one();
            }
        );
        let saved = profile(&db).await.unwrap();
        db.drop().await.unwrap();
        assert_eq!(
            rate(&saved),
            "9".parse().unwrap(),
            "late publication replaced the winner"
        );
        assert_eq!(rate(&old_result.unwrap()), rate(&saved));
        assert_eq!(old.calls.load(Ordering::SeqCst), 1);
        assert_eq!(new.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn tax_ordering_red_two_existing_publishers() {
    two_publishers(true).await;
}
#[tokio::test]
async fn tax_ordering_red_two_absent_publishers() {
    two_publishers(false).await;
}
async fn invalidation_wins(existing: bool, owner: bool) {
    for phase in [
        Phase::Captured,
        Phase::ProviderReturned,
        Phase::BeforePublication,
    ] {
        let (db, wallet) = fixture(existing).await;
        let provider = Provider::new("5");
        let gate = Gate::new(phase);
        let (result, ()) = tokio::join!(
            GATE.scope(gate.clone(), refresh(&db, &provider, &wallet, 900)),
            async {
                gate.entered().await;
                invalidate(&db, owner, "standard").await;
                gate.resume.notify_one();
            }
        );
        let saved = profile(&db).await;
        db.drop().await.unwrap();
        assert!(
            result.is_err(),
            "completed invalidation must reject the captured provider result"
        );
        assert!(saved.is_none_or(|p| p.fetched_at == chrono::DateTime::UNIX_EPOCH));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn tax_ordering_red_existing_plan_invalidation() {
    invalidation_wins(true, false).await;
}
#[tokio::test]
async fn tax_ordering_red_existing_owner_invalidation() {
    invalidation_wins(true, true).await;
}
#[tokio::test]
async fn tax_ordering_red_cold_plan_invalidation() {
    invalidation_wins(false, false).await;
}
#[tokio::test]
async fn tax_ordering_red_cold_owner_invalidation() {
    invalidation_wins(false, true).await;
}
#[tokio::test]
async fn tax_ordering_red_old_style_profile_write() {
    let (db, wallet) = fixture(true).await;
    let old = Provider::new("5");
    let gate = Gate::new(Phase::ProviderReturned);
    let (result, ()) = tokio::join!(
        GATE.scope(gate.clone(), refresh(&db, &old, &wallet, 900)),
        async {
            gate.entered().await;
            let winner = resolve_config("owner", "sub", &Provider::new("9").config()).unwrap();
            let mut fields = bson::to_document(&winner).unwrap();
            fields.remove("_id");
            fields.remove("revision");
            db.collection::<Document>(COLLECTION_NAME)
                .update_one(doc! {"owner_id":"owner"}, doc! {"$set":fields})
                .await
                .unwrap();
            gate.resume.notify_one();
        }
    );
    let saved = profile(&db).await.unwrap();
    db.drop().await.unwrap();
    assert_eq!(rate(&saved), "9".parse().unwrap());
    assert_eq!(rate(&result.unwrap()), rate(&saved));
}
#[tokio::test]
async fn tax_ordering_red_forced_refresh_reuses_fresh_profile() {
    let (db, wallet) = fixture(false).await;
    let winner = resolve_config("owner", "sub", &Provider::new("9").config()).unwrap();
    db.collection::<BillingTaxProfile>(COLLECTION_NAME)
        .insert_one(winner)
        .await
        .unwrap();
    let provider = Provider::new("5");
    let result = refresh(&db, &provider, &wallet, 900).await;
    db.drop().await.unwrap();
    assert_eq!(rate(&result.unwrap()), "9".parse().unwrap());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn tax_ordering_red_changed_plan_uses_prefetch_epoch() {
    let (db, wallet) = fixture(true).await;
    let provider = Provider {
        plan: "other",
        ..Provider::new("5")
    };
    let gate = Gate::new(Phase::ProviderReturned);
    let (result, ()) = tokio::join!(
        GATE.scope(gate.clone(), refresh(&db, &provider, &wallet, 900)),
        async {
            gate.entered().await;
            invalidate_plan(&db, "other").await.unwrap();
            gate.resume.notify_one();
        }
    );
    let saved = profile(&db).await.unwrap();
    db.drop().await.unwrap();
    assert!(result.is_err());
    assert_eq!(saved.plan_code, "standard");
    assert_eq!(saved.fetched_at, chrono::DateTime::UNIX_EPOCH);
}

async fn current_wallet(db: &Database) -> BillingWallet {
    db.collection::<BillingWallet>(WALLETS)
        .find_one(doc! {"_id":"wallet"})
        .await
        .unwrap()
        .unwrap()
}
async fn invalidating(db: &Database, owner: bool) -> AppResult<()> {
    if owner {
        invalidate_owner(db, "owner").await?;
    } else {
        invalidate_plan(db, "standard").await?;
    }
    Ok(())
}
async fn wait_at<F: std::future::Future>(future: &mut std::pin::Pin<Box<F>>, gate: &Gate) {
    tokio::select! {
        _ = future => panic!("operation completed before its barrier"),
        () = gate.entered() => {},
    }
}
async fn settlement_rate(db: &Database) -> AppResult<Credits> {
    let now = bson::DateTime::now();
    let row: UsageMeterRow = bson::from_document(doc! {
        "_id":"usage", "transaction_id":"usage", "billing_request_id":"request", "layer":"platform",
        "billing_owner_id":"owner", "actor_user_id":"owner", "metric":"requests", "lago_metric_code":"requests",
        "credential_class":"user_owned", "status":"finalized", "forwarded":true, "released":false,
        "lago_acked":false, "attempt":0, "created_at":now, "updated_at":now, "funding":{"tax_accounting":true},
    }).unwrap();
    Ok(settlement_tax(db, &row, Credits::from_whole(1))
        .await?
        .unwrap()
        .rate_percent)
}

#[tokio::test]
async fn tax_ordering_counter_then_common_publication_is_invalidated_or_retried_after_crash() {
    for owner in [false, true] {
        for crash in [false, true] {
            let (db, wallet) = fixture(true).await;
            let provider = Provider::new("5");
            let publication_gate = Gate::new(Phase::BeforePublication);
            let mut publication = Box::pin(GATE.scope(
                publication_gate.clone(),
                refresh(&db, &provider, &wallet, 900),
            ));
            wait_at(&mut publication, &publication_gate).await;
            let invalidation_gate = Gate::new(Phase::InvalidationAdvanced);
            let mut invalidation =
                Box::pin(GATE.scope(invalidation_gate.clone(), invalidating(&db, owner)));
            wait_at(&mut invalidation, &invalidation_gate).await;
            publication_gate.resume.notify_one();
            let published = publication.await.unwrap();
            assert!(
                fresh(&published, &wallet, 900),
                "step one alone does not fence common publication"
            );
            if crash {
                drop(invalidation);
                assert!(fresh(&profile(&db).await.unwrap(), &wallet, 900));
                invalidating(&db, owner).await.unwrap();
            } else {
                invalidation_gate.resume.notify_one();
                invalidation.await.unwrap();
            }
            let saved = profile(&db).await.unwrap();
            db.drop().await.unwrap();
            assert_eq!(saved.fetched_at, chrono::DateTime::UNIX_EPOCH);
            assert_eq!(saved.revision, published.revision + 1);
        }
    }
}

#[tokio::test]
async fn tax_ordering_cold_guard_commit_precedes_owner_and_plan_invalidation() {
    for owner in [false, true] {
        let (db, wallet) = fixture(false).await;
        let provider = Provider::new("5");
        let gate = Gate::new(Phase::GuardsWritten);
        let mut publisher =
            Box::pin(GATE.scope(gate.clone(), refresh(&db, &provider, &wallet, 900)));
        wait_at(&mut publisher, &gate).await;
        assert!(profile(&db).await.is_none());
        assert!(
            settlement_rate(&db).await.is_err(),
            "no provisional zero-tax value"
        );
        let mut invalidation = Box::pin(invalidating(&db, owner));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut invalidation)
                .await
                .is_err()
        );
        gate.resume.notify_one();
        let (published, invalidated) = tokio::join!(publisher, invalidation);
        published.unwrap();
        invalidated.unwrap();
        let saved = profile(&db).await.unwrap();
        let settled = settlement_rate(&db).await.unwrap();
        db.drop().await.unwrap();
        assert_eq!(saved.fetched_at, chrono::DateTime::UNIX_EPOCH);
        assert_eq!(
            settled,
            "5".parse().unwrap(),
            "historical settlement still uses verified stale data"
        );
    }
}

#[tokio::test]
async fn tax_ordering_capture_after_counter_cold_insert_has_no_phantom_fence() {
    for owner in [false, true] {
        for insert_first in [false, true] {
            let (db, _) = fixture(false).await;
            let invalidation_gate = Gate::new(Phase::InvalidationAdvanced);
            let mut invalidation =
                Box::pin(GATE.scope(invalidation_gate.clone(), invalidating(&db, owner)));
            wait_at(&mut invalidation, &invalidation_gate).await;
            let wallet = current_wallet(&db).await;
            let provider = Provider::new("5");
            let gate = Gate::new(Phase::BeforePublication);
            let mut publisher =
                Box::pin(GATE.scope(gate.clone(), refresh(&db, &provider, &wallet, 900)));
            wait_at(&mut publisher, &gate).await;
            if insert_first {
                gate.resume.notify_one();
                publisher.await.unwrap();
                invalidation_gate.resume.notify_one();
                invalidation.await.unwrap();
            } else {
                invalidation_gate.resume.notify_one();
                invalidation.await.unwrap();
                gate.resume.notify_one();
                publisher.await.unwrap();
            }
            let saved = profile(&db).await.unwrap();
            db.drop().await.unwrap();
            assert_eq!(
                saved.fetched_at == chrono::DateTime::UNIX_EPOCH,
                insert_first
            );
            assert_eq!(rate(&saved), "5".parse().unwrap());
        }
    }
}

#[tokio::test]
async fn tax_ordering_two_cold_transactions_serialize_without_refetch() {
    let (db, wallet) = fixture(false).await;
    let first = Provider::new("5");
    let second = Provider::new("9");
    let first_gate = Gate::new(Phase::GuardsWritten);
    let mut a = Box::pin(GATE.scope(first_gate.clone(), refresh(&db, &first, &wallet, 900)));
    wait_at(&mut a, &first_gate).await;
    let second_gate = Gate::new(Phase::BeforePublication);
    let mut b = Box::pin(GATE.scope(second_gate.clone(), refresh(&db, &second, &wallet, 900)));
    wait_at(&mut b, &second_gate).await;
    second_gate.resume.notify_one();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut b)
            .await
            .is_err()
    );
    first_gate.resume.notify_one();
    let (a, b) = tokio::join!(a, b);
    let count = db
        .collection::<Document>(COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    let saved = profile(&db).await.unwrap();
    db.drop().await.unwrap();
    assert_eq!(rate(&a.unwrap()), "5".parse().unwrap());
    assert_eq!(rate(&b.unwrap()), "5".parse().unwrap());
    assert_eq!(rate(&saved), "5".parse().unwrap());
    assert_eq!(count, 1);
    assert_eq!(first.calls.load(Ordering::SeqCst), 1);
    assert_eq!(second.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tax_ordering_negative_cache_and_failure_preserve_verified_winner() {
    let (db, wallet) = fixture(false).await;
    let failure = Provider::new("invalid");
    assert!(refresh(&db, &failure, &wallet, 900).await.is_err());
    let negative = profile(&db).await.unwrap();
    assert!(negative.fingerprint.is_empty());
    assert_eq!(negative.revision, 0);
    assert!(settlement_rate(&db).await.is_err());
    assert!(
        ensure_fresh(&db, &Provider::new("9"), &wallet, 900)
            .await
            .is_err()
    );
    let old = Provider::new("5");
    let gate = Gate::new(Phase::BeforePublication);
    let (old_result, ()) = tokio::join!(
        GATE.scope(gate.clone(), refresh(&db, &old, &wallet, 900)),
        async {
            gate.entered().await;
            assert!(settlement_rate(&db).await.is_err());
            refresh(&db, &Provider::new("9"), &wallet, 900)
                .await
                .unwrap();
            assert_eq!(settlement_rate(&db).await.unwrap(), "9".parse().unwrap());
            gate.resume.notify_one();
        }
    );
    assert_eq!(rate(&old_result.unwrap()), "9".parse().unwrap());
    // A late provider failure retains the accepted value/revision and only
    // records the existing failed-attempt backoff.
    invalidate_plan(&db, "standard").await.unwrap();
    let gate = Gate::new(Phase::Captured);
    let (failure_result, ()) = tokio::join!(
        GATE.scope(gate.clone(), refresh(&db, &failure, &wallet, 900)),
        async {
            gate.entered().await;
            refresh(&db, &Provider::new("12"), &wallet, 900)
                .await
                .unwrap();
            gate.resume.notify_one();
        }
    );
    assert!(failure_result.is_err());
    let saved = profile(&db).await.unwrap();
    let unused = Provider::new("5");
    let cached = ensure_fresh(&db, &unused, &wallet, 900).await.unwrap();
    db.drop().await.unwrap();
    assert_eq!(rate(&saved), "12".parse().unwrap());
    assert_eq!(saved.revision, 3);
    assert!(saved.refresh_failed_at.is_some());
    assert_eq!(rate(&cached), rate(&saved));
    assert_eq!(unused.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn tax_ordering_forced_sweep_reuses_or_loses_to_inline() {
    for phase in [Phase::SweepSelected, Phase::BeforePublication] {
        let (db, wallet) = fixture(true).await;
        let old = Provider::new("5");
        let new = Provider::new("9");
        let gate = Gate::new(phase);
        let (sweep, ()) =
            tokio::join!(GATE.scope(gate.clone(), reconcile(&db, &old, 900)), async {
                gate.entered().await;
                assert_eq!(settlement_rate(&db).await.unwrap(), "1".parse().unwrap());
                ensure_fresh(&db, &new, &wallet, 900).await.unwrap();
                gate.resume.notify_one();
            });
        sweep.unwrap();
        let saved = profile(&db).await.unwrap();
        db.drop().await.unwrap();
        assert_eq!(rate(&saved), "9".parse().unwrap());
        assert_eq!(
            old.calls.load(Ordering::SeqCst),
            usize::from(phase != Phase::SweepSelected)
        );
        assert_eq!(new.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn tax_ordering_binding_repair_fences_existing_and_cold_publishers() {
    for existing in [true, false] {
        let (db, _) = fixture(existing).await;
        db.collection::<Document>(WALLETS)
            .update_one(
                doc! {"_id":"wallet"},
                doc! {"$set":{"lago_subscription_id":""}},
            )
            .await
            .unwrap();
        let wallet = current_wallet(&db).await;
        let old = Provider::new("5");
        let service = super::super::BillingService::new_with_lago(
            db.clone(),
            Arc::new(crate::test_utils::test_app_config()),
            Arc::new(Provider::new("9")),
        );
        let publication_gate = Gate::new(Phase::BeforePublication);
        let repair_gate = Gate::new(Phase::RepairBeforeCommit);
        let (result, repaired, ()) = tokio::join!(
            GATE.scope(publication_gate.clone(), refresh(&db, &old, &wallet, 900)),
            async {
                publication_gate.entered().await;
                GATE.scope(
                    repair_gate.clone(),
                    service.ensure_org_wallet_subscription(&wallet),
                )
                .await
            },
            async {
                repair_gate.entered().await;
                publication_gate.resume.notify_one();
                repair_gate.resume.notify_one();
            }
        );
        assert!(result.is_err());
        let repaired = repaired.unwrap();
        assert_eq!(repaired.lago_subscription_id.as_deref(), Some("repaired"));
        assert_eq!(repaired.tax_generation, 1);
        assert_eq!(repaired.balance_credits, wallet.balance_credits);
        let new = ensure_fresh(&db, &Provider::new("9"), &repaired, 900)
            .await
            .unwrap();
        db.drop().await.unwrap();
        assert_eq!(new.subscription_id, "repaired");
        assert_eq!(rate(&new), "9".parse().unwrap());
    }
}

#[tokio::test]
async fn tax_ordering_plan_partial_invalidation_error_requires_retry() {
    let (db, wallet) = fixture(false).await;
    let a = resolve_config("owner", "sub", &Provider::new("5").config()).unwrap();
    let mut b = bson::to_document(
        &resolve_config("other", "other-sub", &Provider::new("9").config()).unwrap(),
    )
    .unwrap();
    b.insert("reject_invalidation", true);
    db.collection::<BillingTaxProfile>(COLLECTION_NAME)
        .insert_one(a)
        .await
        .unwrap();
    db.collection::<Document>(COLLECTION_NAME)
        .insert_one(b)
        .await
        .unwrap();
    db.run_command(doc! {"collMod":COLLECTION_NAME,"validator":{"$expr":{"$or":[
        {"$ne":["$reject_invalidation",true]}, {"$gt":["$fetched_at",bson::DateTime::from_millis(0)]}
    ]}}}).await.unwrap();
    assert!(
        invalidate_plan(&db, "standard").await.is_err(),
        "row update failure must not report invalidation success"
    );
    let partial = profile(&db).await.unwrap();
    assert!(!fresh(&partial, &wallet, 900));
    db.run_command(doc! {"collMod":COLLECTION_NAME,"validator":{}})
        .await
        .unwrap();
    invalidate_plan(&db, "standard").await.unwrap();
    let fresh_rows = db
        .collection::<Document>(COLLECTION_NAME)
        .count_documents(doc! {"fetched_at":{"$gt":bson::DateTime::from_millis(0)}})
        .await
        .unwrap();
    let epoch = ordering::capture_plan_epoch(&db).await.unwrap();
    db.drop().await.unwrap();
    assert_eq!(fresh_rows, 0);
    assert_eq!(epoch, 2);
}

#[tokio::test]
async fn tax_ordering_owner_preserves_failure_backoff_plan_clears_it() {
    let (db, _) = fixture(true).await;
    let now = bson::DateTime::now();
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"owner_id":"owner"},
            doc! {"$set":{"refresh_failed_at":now}},
        )
        .await
        .unwrap();
    let wallet = invalidate_owner(&db, "owner").await.unwrap().unwrap();
    let owned = profile(&db).await.unwrap();
    invalidate_plan(&db, "standard").await.unwrap();
    let planned = profile(&db).await.unwrap();
    db.drop().await.unwrap();
    assert_eq!(wallet.tax_generation, 1);
    assert_eq!(owned.revision, 1);
    assert_eq!(owned.refresh_failed_at, Some(now.to_chrono()));
    assert_eq!(planned.revision, 2);
    assert!(planned.refresh_failed_at.is_none());
}

#[tokio::test]
async fn tax_ordering_old_post_commit_and_invisible_aba_are_explicit_limits() {
    let (db, wallet) = fixture(true).await;
    let original = profile(&db).await.unwrap();
    let provider = Provider::new("5");
    let gate = Gate::new(Phase::BeforePublication);
    let (published, ()) = tokio::join!(
        GATE.scope(gate.clone(), refresh(&db, &provider, &wallet, 900)),
        async {
            gate.entered().await;
            for value in [
                resolve_config("owner", "sub", &Provider::new("9").config()).unwrap(),
                original,
            ] {
                let mut fields = bson::to_document(&value).unwrap();
                fields.remove("_id");
                fields.remove("revision");
                db.collection::<Document>(COLLECTION_NAME)
                    .update_one(doc! {"owner_id":"owner"}, doc! {"$set":fields})
                    .await
                    .unwrap();
            }
            gate.resume.notify_one();
        }
    );
    assert_eq!(
        rate(&published.unwrap()),
        "5".parse().unwrap(),
        "an invisible legacy ABA cannot be detected"
    );
    let old = resolve_config("owner", "sub", &Provider::new("1").config()).unwrap();
    let mut fields = bson::to_document(&old).unwrap();
    fields.remove("_id");
    fields.remove("revision");
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(doc! {"owner_id":"owner"}, doc! {"$set":fields})
        .await
        .unwrap();
    let saved = profile(&db).await.unwrap();
    db.drop().await.unwrap();
    assert_eq!(rate(&saved), "1".parse().unwrap());
    assert_eq!(saved.revision, 1);
}

#[tokio::test]
async fn tax_ordering_same_plan_changed_binding_uses_wallet_guard() {
    // A caller may hold the old empty binding while repair has already
    // invalidated the live profile, before this refresh captures that profile.
    let (db, _) = fixture(true).await;
    db.collection::<Document>(WALLETS)
        .update_one(
            doc! {"_id":"wallet"},
            doc! {"$set":{"lago_subscription_id":""}},
        )
        .await
        .unwrap();
    let old_wallet = current_wallet(&db).await;
    repair_subscription(&db, &old_wallet, "repaired")
        .await
        .unwrap();
    let provider = Provider::new("5");
    let result = refresh(&db, &provider, &old_wallet, 900).await;
    let saved = profile(&db).await.unwrap();
    db.drop().await.unwrap();
    assert!(result.is_err());
    assert_eq!(saved.fetched_at, chrono::DateTime::UNIX_EPOCH);
    assert_eq!(saved.revision, 1);
}

#[tokio::test]
async fn tax_ordering_changed_plan_commits_accepted_data_once() {
    let (db, wallet) = fixture(true).await;
    let provider = Provider {
        plan: "other",
        ..Provider::new("9")
    };
    let published = refresh(&db, &provider, &wallet, 900).await.unwrap();
    let saved = profile(&db).await.unwrap();
    let guarded = current_wallet(&db).await;
    db.drop().await.unwrap();
    assert_eq!(saved.plan_code, "other");
    assert_eq!(saved.revision, 1);
    assert_eq!(rate(&published), "9".parse().unwrap());
    assert!(guarded.tax_guard_token.is_some());
    assert_eq!(guarded.tax_generation, wallet.tax_generation);
}

#[tokio::test]
async fn tax_ordering_webhook_refresh_loses_to_inline_without_second_fetch() {
    let (db, wallet) = fixture(true).await;
    let old = Provider::new("5");
    let new = Provider::new("9");
    let gate = Gate::new(Phase::ProviderReturned);
    let payload = serde_json::json!({"customer":{"external_id":"owner"}});
    let (webhook, ()) = tokio::join!(
        GATE.scope(
            gate.clone(),
            super::super::webhook::handle_lago_webhook_event(
                &db,
                Some(&old),
                "customer.updated",
                &payload,
                900
            )
        ),
        async {
            gate.entered().await;
            ensure_fresh(&db, &new, &wallet, 900).await.unwrap();
            gate.resume.notify_one();
        }
    );
    webhook.unwrap();
    let saved = profile(&db).await.unwrap();
    db.drop().await.unwrap();
    assert_eq!(rate(&saved), "9".parse().unwrap());
    assert_eq!(old.calls.load(Ordering::SeqCst), 1);
    assert_eq!(new.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tax_ordering_publication_and_invalidation_command_counts() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let recorded = commands.clone();
    let handler = EventHandler::<CommandEvent>::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            let collection = event.command.get_str(&event.command_name).unwrap_or("");
            recorded
                .lock()
                .unwrap()
                .push(format!("{}:{}", event.command_name, collection));
        }
    });
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "tax_publication_counts",
        handler,
    )
    .await;
    let (db, wallet) = fixture_on_db(db, true).await;
    ordering::capture_plan_epoch(&db).await.unwrap();
    let take = |label: &str, expected: &[&str]| {
        let actual = std::mem::take(&mut *commands.lock().unwrap());
        println!("TAX_PROTOCOL_COMMANDS {label}: {actual:?}");
        assert_eq!(actual, expected, "{label}");
    };
    let provider = Provider::new("9");
    commands.lock().unwrap().clear();
    refresh(&db, &provider, &wallet, 900).await.unwrap();
    take(
        "common_refresh",
        &[
            "find:billing_tax_profiles",
            "find:billing_tax_plan_epoch",
            "update:billing_tax_profiles",
        ],
    );
    refresh(&db, &provider, &wallet, 900).await.unwrap();
    take("forced_fresh", &["find:billing_tax_profiles"]);
    invalidate_plan(&db, "standard").await.unwrap();
    take(
        "plan_invalidation",
        &[
            "update:billing_tax_plan_epoch",
            "update:billing_tax_profiles",
        ],
    );
    let wallet = invalidate_owner(&db, "owner").await.unwrap().unwrap();
    take(
        "owner_invalidation",
        &[
            "findAndModify:billing_wallet",
            "update:billing_tax_profiles",
        ],
    );
    db.collection::<Document>(COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    refresh(&db, &provider, &wallet, 900).await.unwrap();
    take(
        "cold_refresh",
        &[
            "find:billing_tax_profiles",
            "find:billing_tax_plan_epoch",
            "update:billing_wallet",
            "update:billing_tax_plan_epoch",
            "insert:billing_tax_profiles",
            "commitTransaction:",
        ],
    );
    db.collection::<Document>(WALLETS)
        .update_one(
            doc! {"_id":"wallet"},
            doc! {"$set":{"lago_subscription_id":""}},
        )
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    repair_subscription(&db, &wallet, "repaired").await.unwrap();
    take(
        "binding_repair",
        &[
            "update:billing_wallet",
            "update:billing_tax_profiles",
            "commitTransaction:",
        ],
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn tax_ordering_binding_repair_revision_rejects_existing_subscription_capture() {
    let (db, wallet) = fixture(true).await;
    // Retain a caller's previously loaded binding while repairing the missing
    // binding now stored on a legacy wallet. This publisher uses the common CAS.
    db.collection::<Document>(WALLETS)
        .update_one(
            doc! {"_id":"wallet"},
            doc! {"$unset":{"lago_subscription_id":""}},
        )
        .await
        .unwrap();
    let missing = current_wallet(&db).await;
    let service = super::super::BillingService::new_with_lago(
        db.clone(),
        Arc::new(crate::test_utils::test_app_config()),
        Arc::new(Provider::new("9")),
    );
    let provider = Provider::new("5");
    let gate = Gate::new(Phase::BeforePublication);
    let (publication, repaired) = tokio::join!(
        GATE.scope(gate.clone(), refresh(&db, &provider, &wallet, 900)),
        async {
            gate.entered().await;
            let repaired = service
                .ensure_org_wallet_subscription(&missing)
                .await
                .unwrap();
            gate.resume.notify_one();
            repaired
        }
    );
    let saved = profile(&db).await.unwrap();
    db.drop().await.unwrap();
    assert!(publication.is_err());
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.fetched_at, chrono::DateTime::UNIX_EPOCH);
    assert_eq!(rate(&saved), "1".parse().unwrap());
    assert_eq!(repaired.lago_subscription_id.as_deref(), Some("repaired"));
}
