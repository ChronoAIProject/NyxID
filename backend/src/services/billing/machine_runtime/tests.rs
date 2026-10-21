//! Local integration tests: real MongoDB transactions and shared exact billing,
//! with only Lago's external API replaced by an in-memory provider.
use super::*;
use crate::config::AppConfig;
use crate::services::billing::lago_client::{
    Entitlement, LagoAck, LagoApi, LagoError, LagoEvent, LagoUsage, OwnerProvisionInput, PlanRate,
    PlanRates, RuntimeSubscription, ServicePriceSync,
};
use async_trait::async_trait;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct LocalLago {
    available: AtomicBool,
    entitled: AtomicBool,
    rates: Mutex<Vec<PlanRate>>,
    owner: String,
    plan: String,
    subscription_failure: AtomicUsize,
}

impl LocalLago {
    fn new(owner: String, plan: String) -> Self {
        Self {
            available: AtomicBool::new(true),
            entitled: AtomicBool::new(true),
            rates: Mutex::new(vec![]),
            owner,
            plan,
            subscription_failure: AtomicUsize::new(0),
        }
    }
    fn check(&self) -> AppResult<()> {
        if self.available.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(unavailable("local provider unavailable"))
        }
    }
}

#[async_trait]
impl LagoApi for LocalLago {
    async fn tax_config(
        &self,
        _: &str,
        _: &str,
    ) -> crate::errors::AppResult<crate::services::billing::lago_client::LagoTaxConfig> {
        self.check()?;
        Ok(crate::services::billing::taxes::zero_config())
    }
    async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String> {
        Ok(owner.external_customer_id.clone())
    }
    async fn ensure_subscription(&self, customer: &str, _: &str) -> AppResult<String> {
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
            .map(|event| LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
            .collect())
    }
    async fn current_usage(&self, customer: &str, subscription: &str) -> AppResult<LagoUsage> {
        Ok(LagoUsage {
            customer_id: customer.into(),
            subscription_id: subscription.into(),
            raw: serde_json::json!({}),
        })
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        self.check()?;
        Ok(10)
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        self.check()?;
        Ok(if self.entitled.load(Ordering::SeqCst) {
            vec![Entitlement {
                code: SKU.into(),
                raw: serde_json::json!({}),
            }]
        } else {
            vec![]
        })
    }
    async fn runtime_subscription(&self, id: &str) -> AppResult<RuntimeSubscription> {
        self.check()?;
        let failure = self.subscription_failure.load(Ordering::SeqCst);
        if failure == 1 {
            return Err(unavailable("local subscription unavailable"));
        }
        Ok(RuntimeSubscription {
            external_id: id.into(),
            external_customer_id: if failure == 3 {
                "wrong-owner".into()
            } else {
                self.owner.clone()
            },
            plan_code: if failure == 4 {
                "wrong-plan".into()
            } else {
                self.plan.clone()
            },
            status: if failure == 2 {
                "terminated".into()
            } else {
                "active".into()
            },
        })
    }
    async fn plan_rates(&self, _: &str) -> AppResult<PlanRates> {
        self.check()?;
        Ok(PlanRates {
            rates: self.rates.lock().unwrap().clone(),
            rejected: vec![],
        })
    }
    async fn sync_standard_charge(&self, _: &str, input: &ServicePriceSync) -> AppResult<()> {
        self.check()?;
        let exact: Credits = input.credits_per_unit.parse()?;
        let pico = i64::try_from(exact.pico()).unwrap();
        let mut rates = self.rates.lock().unwrap();
        rates.retain(|rate| rate.lago_metric_code != input.metric_code);
        rates.push(PlanRate {
            lago_metric_code: input.metric_code.clone(),
            credits_per_unit_micros: pico / 1_000_000,
            credits_per_unit_pico: Some(pico),
        });
        Ok(())
    }
}

struct Fixture {
    db: Database,
    billing: BillingService,
    config: Arc<AppConfig>,
    lago: Arc<LocalLago>,
    owner: String,
    rate: MachineRuntimeRate,
    terms: MachineRuntimeTerms,
}

macro_rules! tx {
    ($f:expr, $session:ident, $body:expr) => {{
        let mut $session = $f.db.client().start_session().await.unwrap();
        $session.start_transaction().await.unwrap();
        let result: AppResult<_> = $body.await;
        if result.is_ok() {
            $session.commit_transaction().await.unwrap();
        } else {
            $session.abort_transaction().await.unwrap();
        }
        result
    }};
}

async fn fixture(label: &str) -> Fixture {
    let db = crate::test_utils::connect_transaction_test_database(label).await;
    crate::services::billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    crate::db::ensure_indexes(&db).await.unwrap();
    ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    db.collection::<Document>("users")
        .insert_one(doc! {"_id":&owner,"user_type":"person","is_active":true})
        .await
        .unwrap();
    db.collection::<Document>(WALLETS).insert_one(doc!{
        "_id":Uuid::new_v4().to_string(),"owner_id":&owner,"lago_customer_id":&owner,
        "lago_wallet_id":"local-wallet","lago_subscription_id":"local-subscription",
        "plan_kind":"prepaid","balance_credits":Credits::from_whole(10),"reserved_credits":Credits::ZERO,
        "pending_lago_debits":Credits::ZERO,"pending_topup_expiry_credits":Credits::ZERO,
        "overdraft_cap_credits":Credits::ZERO,"has_payment_instrument":false,"suspended":false,
        "collection_state":"good","created_at":now,"updated_at":now,"balance_synced_at":now,
    }).await.unwrap();
    let config = Arc::new(AppConfig {
        billing_enabled: true,
        billing_allocations_enabled: false,
        lago_api_url: None,
        lago_api_key: None,
        ..crate::test_utils::test_app_config()
    });
    let lago = Arc::new(LocalLago::new(owner.clone(), config.lago_plan_code.clone()));
    let billing = BillingService::new_with_lago(db.clone(), config.clone(), lago.clone());
    let draft = billing.stage_initial_machine_runtime_rate().await.unwrap();
    assert_eq!(draft.sync_status, PricingSyncStatus::Pending);
    billing.sync_machine_runtime_rate(&draft.id).await.unwrap();
    let rate = billing.runtime_rate(&draft.id).await.unwrap();
    Fixture {
        db,
        billing,
        config,
        lago,
        owner,
        rate,
        terms: MachineRuntimeTerms {
            profile_version: "remote-machine".into(),
            image_digest: format!("sha256:{}", "a".repeat(64)),
            policy_version: "account-balance-v1".into(),
        },
    }
}

async fn quote(f: &Fixture) -> MachineRuntimeQuote {
    f.billing
        .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
        .await
        .unwrap()
}
async fn wallet(f: &Fixture) -> BillingWallet {
    f.db.collection::<BillingWallet>(WALLETS)
        .find_one(doc! {"owner_id":&f.owner})
        .await
        .unwrap()
        .unwrap()
}
async fn run(f: &Fixture, id: &str) -> MachineRuntimeRun {
    f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}
async fn accept(
    f: &Fixture,
    p: &PreparedRuntimeWindow,
    node: &str,
    operation: &str,
    generation: i64,
) -> AppResult<FundedRuntimeWindow> {
    tx!(
        f,
        s,
        f.billing.accept_machine_quote_in_session(
            &mut s,
            p,
            AcceptRuntimeQuote {
                owner_id: &f.owner,
                node_id: node,
                operation_id: operation,
                runtime_generation: generation,
                terms: &f.terms,
            }
        )
    )
}
async fn start(
    f: &Fixture,
    node: &str,
    from: DateTime<Utc>,
    until: DateTime<Utc>,
) -> (FundedRuntimeWindow, PreparedRuntimeWindow, String) {
    let q = quote(f).await;
    let p = f
        .billing
        .prepare_machine_start(&f.owner, &q.id, &Uuid::new_v4().to_string(), from, until)
        .await
        .unwrap();
    let op = Uuid::new_v4().to_string();
    let w = accept(f, &p, node, &op, 1).await.unwrap();
    (w, p, op)
}
async fn ready(f: &Fixture, id: &str, generation: i64, time: DateTime<Utc>) {
    tx!(
        f,
        s,
        f.billing
            .prove_machine_ready_in_session(&mut s, id, generation, time, "ready-proof")
    )
    .unwrap();
}
async fn stopped(f: &Fixture, id: &str, generation: i64, time: DateTime<Utc>) {
    tx!(
        f,
        s,
        f.billing
            .prove_machine_stopped_in_session(&mut s, id, generation, time, "stop-proof")
    )
    .unwrap();
}
async fn finalize(f: &Fixture, id: &str) -> Vec<String> {
    let mut result = vec![];
    while let Some(row) = tx!(
        f,
        s,
        f.billing.finalize_machine_interval_in_session(&mut s, id)
    )
    .unwrap()
    {
        result.push(row);
    }
    result
}
async fn settle(f: &Fixture, id: &str) {
    f.billing.settle_machine_runtime(id, 100).await.unwrap();
    assert_eq!(f.billing.settle_machine_runtime(id, 100).await.unwrap(), 0);
}
async fn wait_for_evidence(time: DateTime<Utc>) {
    // Production accepts no more than one second of observation clock skew.
    // Leave half that allowance as test scheduling headroom, never alter data clocks.
    if let Ok(delay) = (time - Utc::now() - Duration::milliseconds(500)).to_std() {
        assert!(delay.as_secs() < 60);
        tokio::time::sleep(delay).await;
    }
}

#[tokio::test]
async fn quote_fifteen_minute_expiry_terms_and_accepted_replay_are_durable() {
    let f = fixture("runtime_quote_boundary").await;
    let now = Utc::now();
    let expired = f
        .billing
        .quote_machine_runtime_at(
            &f.owner,
            f.terms.clone(),
            &f.rate.id,
            now - Duration::minutes(15),
        )
        .await
        .unwrap();
    assert_eq!(
        (expired.accept_before - expired.created_at).num_seconds(),
        900
    );
    let p = f
        .billing
        .prepare_machine_start(
            &f.owner,
            &expired.id,
            &Uuid::new_v4().to_string(),
            now,
            now + Duration::seconds(30),
        )
        .await
        .unwrap();
    let node = Uuid::new_v4().to_string();
    let op = Uuid::new_v4().to_string();
    assert!(matches!(
        accept(&f, &p, &node, &op, 1).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    let q = f
        .billing
        .quote_machine_runtime_at(
            &f.owner,
            f.terms.clone(),
            &f.rate.id,
            Utc::now() - Duration::minutes(14),
        )
        .await
        .unwrap();
    let p = f
        .billing
        .prepare_machine_start(
            &f.owner,
            &q.id,
            &Uuid::new_v4().to_string(),
            now,
            now + Duration::seconds(30),
        )
        .await
        .unwrap();
    let mut wrong = f.terms.clone();
    wrong.policy_version = "unaccepted-policy".into();
    assert!(
        tx!(
            &f,
            s,
            f.billing.accept_machine_quote_in_session(
                &mut s,
                &p,
                AcceptRuntimeQuote {
                    owner_id: &f.owner,
                    node_id: &node,
                    operation_id: &op,
                    runtime_generation: 1,
                    terms: &wrong,
                }
            )
        )
        .is_err()
    );
    let accepted = accept(&f, &p, &node, &op, 1).await.unwrap();
    let held = wallet(&f).await.reserved_credits;
    f.db.collection::<Document>(QUOTE_COLLECTION)
        .update_one(
            doc! {"_id":&q.id},
            doc! {"$set":{
                "accept_before":bson::DateTime::from_chrono(now-Duration::seconds(1)),
            }},
        )
        .await
        .unwrap();
    let replay = accept(&f, &p, &node, &op, 1).await.unwrap();
    assert_eq!(replay.run_id, accepted.run_id);
    assert_eq!(replay.usage_row_id, accepted.usage_row_id);
    assert_eq!(wallet(&f).await.reserved_credits, held);
    let mut offline_config = (*f.config).clone();
    offline_config.billing_enabled = false;
    let offline = BillingService::new(f.db.clone(), Arc::new(offline_config));
    assert!(!offline.lago_configured());
    let revision = run(&f, &accepted.run_id).await.revision;
    let recovered = offline
        .replay_machine_start(
            AcceptRuntimeQuote {
                owner_id: &f.owner,
                node_id: &node,
                operation_id: &op,
                runtime_generation: 1,
                terms: &f.terms,
            },
            &q.id,
            &p.interval_id,
            DateTime::from_timestamp_nanos(p.from_ns),
            DateTime::from_timestamp_nanos(p.until_ns),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.run_id, accepted.run_id);
    assert_eq!(recovered.usage_row_id, accepted.usage_row_id);
    assert_eq!(recovered.covered_until, accepted.covered_until);
    assert_eq!(recovered.rate_version, accepted.rate_version);
    assert_eq!(run(&f, &accepted.run_id).await.revision, revision);
    assert_eq!(wallet(&f).await.reserved_credits, held);
    assert!(
        offline
            .replay_machine_start(
                AcceptRuntimeQuote {
                    owner_id: &f.owner,
                    node_id: &node,
                    operation_id: &op,
                    runtime_generation: 2,
                    terms: &f.terms,
                },
                &q.id,
                &p.interval_id,
                DateTime::from_timestamp_nanos(p.from_ns),
                DateTime::from_timestamp_nanos(p.until_ns),
            )
            .await
            .is_err()
    );
    assert!(
        accept(&f, &p, &node, &Uuid::new_v4().to_string(), 1)
            .await
            .is_err()
    );
    assert_eq!(
        f.db.collection::<Document>(RUN_COLLECTION)
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
        1
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn paid_runtime_admission_rejects_missing_and_disabled_prerequisites() {
    let f = fixture("runtime_strict_admission").await;
    let mut disabled = (*f.config).clone();
    disabled.billing_enabled = false;
    let disabled = BillingService::new_with_lago(f.db.clone(), Arc::new(disabled), f.lago.clone());
    assert!(
        disabled
            .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
            .await
            .is_err()
    );
    let mut fail_closed = (*f.config).clone();
    fail_closed.billing_fail_closed = true;
    let fail_closed =
        BillingService::new_with_lago(f.db.clone(), Arc::new(fail_closed), f.lago.clone());
    assert!(
        fail_closed
            .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
            .await
            .is_err()
    );
    f.db.collection::<Document>("billing_migrations")
        .delete_one(doc! {"_id":"exact-v2"})
        .await
        .unwrap();
    assert!(
        f.billing
            .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
            .await
            .is_err()
    );
    f.db.collection::<Document>("billing_migrations")
        .insert_one(doc! {"_id":"exact-v2","completed_at":bson::DateTime::now()})
        .await
        .unwrap();
    let no_provider = BillingService::new(f.db.clone(), f.config.clone());
    assert!(
        no_provider
            .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
            .await
            .is_err()
    );
    let saved = wallet(&f).await;
    f.db.collection::<BillingWallet>(WALLETS)
        .delete_one(doc! {"_id":&saved.id})
        .await
        .unwrap();
    assert!(
        f.billing
            .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
            .await
            .is_err()
    );
    f.db.collection::<BillingWallet>(WALLETS)
        .insert_one(&saved)
        .await
        .unwrap();
    for field in ["lago_wallet_id", "lago_subscription_id"] {
        f.db.collection::<Document>(WALLETS)
            .update_one(
                doc! {"_id":&saved.id},
                doc! {"$set":{field:bson::Bson::Null}},
            )
            .await
            .unwrap();
        assert!(
            f.billing
                .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
                .await
                .is_err(),
            "missing {field}"
        );
        f.db.collection::<BillingWallet>(WALLETS)
            .replace_one(doc! {"_id":&saved.id}, &saved)
            .await
            .unwrap();
    }
    f.lago.available.store(false, Ordering::SeqCst);
    assert!(
        f.billing
            .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
            .await
            .is_err()
    );
    f.lago.available.store(true, Ordering::SeqCst);
    f.lago.entitled.store(false, Ordering::SeqCst);
    assert!(matches!(
        f.billing
            .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
            .await,
        Err(AppError::PlanEntitlementRequired(_))
    ));
    f.lago.entitled.store(true, Ordering::SeqCst);
    for failure in 1..=4 {
        f.lago.subscription_failure.store(failure, Ordering::SeqCst);
        assert!(
            f.billing
                .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
                .await
                .is_err(),
            "subscription failure {failure}"
        );
    }
    f.lago.subscription_failure.store(0, Ordering::SeqCst);
    let provider_rates = std::mem::take(&mut *f.lago.rates.lock().unwrap());
    assert!(
        f.billing
            .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
            .await
            .is_err()
    );
    *f.lago.rates.lock().unwrap() = provider_rates;
    f.db.collection::<Document>(RATE_COLLECTION)
        .update_one(
            doc! {"_id":&f.rate.id},
            doc! {"$set":{"sync_status":"pending"}},
        )
        .await
        .unwrap();
    assert!(
        f.billing
            .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
            .await
            .is_err()
    );
    f.db.collection::<Document>(RATE_COLLECTION)
        .update_one(
            doc! {"_id":&f.rate.id},
            doc! {"$set":{"sync_status":"synced"}},
        )
        .await
        .unwrap();
    assert!(
        f.billing
            .quote_machine_runtime(&f.owner, f.terms.clone(), &Uuid::new_v4().to_string())
            .await
            .is_err()
    );
    for denied in [
        doc! {"is_active":false},
        doc! {"machine_restricted":true},
        doc! {"user_type":"organization"},
    ] {
        f.db.collection::<Document>("users")
            .update_one(doc! {"_id":&f.owner}, doc! {"$set":denied})
            .await
            .unwrap();
        assert!(
            f.billing
                .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
                .await
                .is_err()
        );
        f.db.collection::<Document>("users")
            .update_one(
                doc! {"_id":&f.owner},
                doc! {"$set":{"is_active":true,"machine_restricted":false,"user_type":"person"}},
            )
            .await
            .unwrap();
    }
    assert_eq!(
        f.db.collection::<Document>(QUOTE_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.db.collection::<Document>(USAGE)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    assert_eq!(quote(&f).await.funding_mode, "account_balance");
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn acceptance_abort_and_interval_replay_conserve_all_shared_holds() {
    let f = fixture("runtime_accept_rollback").await;
    let q = quote(&f).await;
    let now = Utc::now();
    let p = f
        .billing
        .prepare_machine_start(
            &f.owner,
            &q.id,
            &Uuid::new_v4().to_string(),
            now - Duration::seconds(1),
            now + Duration::seconds(20),
        )
        .await
        .unwrap();
    let node = Uuid::new_v4().to_string();
    let op = Uuid::new_v4().to_string();
    let mut s = f.db.client().start_session().await.unwrap();
    s.start_transaction().await.unwrap();
    f.billing
        .accept_machine_quote_in_session(
            &mut s,
            &p,
            AcceptRuntimeQuote {
                owner_id: &f.owner,
                node_id: &node,
                operation_id: &op,
                runtime_generation: 1,
                terms: &f.terms,
            },
        )
        .await
        .unwrap();
    s.abort_transaction().await.unwrap();
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    for collection in [RUN_COLLECTION, INTERVAL_COLLECTION, HEAD_COLLECTION, USAGE] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "{collection}"
        );
    }
    let w = accept(&f, &p, &node, &op, 1).await.unwrap();
    ready(&f, &w.run_id, 1, Utc::now()).await;
    let next = f
        .billing
        .prepare_machine_next_window(
            &f.owner,
            &w.run_id,
            &Uuid::new_v4().to_string(),
            now + Duration::seconds(20),
            now + Duration::seconds(25),
        )
        .await
        .unwrap();
    let renewed = tx!(
        &f,
        s,
        f.billing
            .reserve_machine_next_window_in_session(&mut s, &next)
    )
    .unwrap();
    let held = wallet(&f).await.reserved_credits;
    let replay = tx!(
        &f,
        s,
        f.billing
            .reserve_machine_next_window_in_session(&mut s, &next)
    )
    .unwrap();
    assert_eq!(renewed.usage_row_id, replay.usage_row_id);
    assert_eq!(wallet(&f).await.reserved_credits, held);
    assert_eq!(
        f.db.collection::<Document>(INTERVAL_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn fully_allowance_funded_acceptance_conflicts_with_concurrent_wallet_suspension() {
    use crate::models::usage_allowance_period::{COLLECTION_NAME as PERIODS, UsageAllowancePeriod};

    let f = fixture("runtime_allowance_wallet_fence").await;
    let allowance_id = Uuid::new_v4().to_string();
    let timestamp = bson::DateTime::now();
    f.db.collection::<Document>(crate::models::usage_allowance::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &allowance_id, "service_id": SKU, "service_slug": SKU,
            "metric": "machine_running_seconds", "quantity": 300_i64,
            "recurrence": "one_time", "target_kind": "all_users",
            "target_user_ids": [], "target_org_ids": [], "target_group_ids": [],
            "is_active": true, "created_by": "local-test",
            "created_at": timestamp, "updated_at": timestamp,
        })
        .await
        .unwrap();
    f.db.collection::<Document>(WALLETS)
        .update_one(
            doc! {"owner_id": &f.owner},
            doc! {"$set": {"balance_credits": Credits::ZERO}},
        )
        .await
        .unwrap();
    let q = quote(&f).await;
    let from = Utc::now();
    let p = f
        .billing
        .prepare_machine_start(
            &f.owner,
            &q.id,
            &Uuid::new_v4().to_string(),
            from,
            from + Duration::seconds(30),
        )
        .await
        .unwrap();
    let node = Uuid::new_v4().to_string();
    let operation = Uuid::new_v4().to_string();

    // Prove this candidate uses only the allowance, then roll that transaction
    // back so the actual race begins with no run or shared benefit hold.
    let mut control = f.db.client().start_session().await.unwrap();
    control.start_transaction().await.unwrap();
    let funded = f
        .billing
        .accept_machine_quote_in_session(
            &mut control,
            &p,
            AcceptRuntimeQuote {
                owner_id: &f.owner,
                node_id: &node,
                operation_id: &operation,
                runtime_generation: 1,
                terms: &f.terms,
            },
        )
        .await
        .unwrap();
    let row =
        f.db.collection::<UsageMeterRow>(USAGE)
            .find_one(doc! {"_id": &funded.usage_row_id})
            .session(&mut control)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(row.reserved_credits, Credits::ZERO);
    let allocations = &row.funding.as_ref().unwrap().allowance_reservations;
    assert_eq!(allocations.len(), 1);
    assert_eq!(allocations[0].allowance_id, allowance_id);
    assert_eq!(allocations[0].quantity, 30);
    control.abort_transaction().await.unwrap();

    let mut raced = f.db.client().start_session().await.unwrap();
    raced.start_transaction().await.unwrap();
    let before =
        f.db.collection::<BillingWallet>(WALLETS)
            .find_one(doc! {"owner_id": &f.owner})
            .session(&mut raced)
            .await
            .unwrap()
            .unwrap();
    assert!(!before.suspended);
    f.db.collection::<Document>(WALLETS)
        .update_one(doc! {"_id": &before.id}, doc! {"$set": {"suspended": true}})
        .await
        .unwrap();
    let result = f
        .billing
        .accept_machine_quote_in_session(
            &mut raced,
            &p,
            AcceptRuntimeQuote {
                owner_id: &f.owner,
                node_id: &node,
                operation_id: &operation,
                runtime_generation: 1,
                terms: &f.terms,
            },
        )
        .await;
    assert!(
        result.is_err(),
        "stale wallet snapshot admitted paid runtime"
    );
    raced.abort_transaction().await.unwrap();
    for collection in [RUN_COLLECTION, INTERVAL_COLLECTION, HEAD_COLLECTION, USAGE] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "{collection}"
        );
    }
    let period =
        f.db.collection::<UsageAllowancePeriod>(PERIODS)
            .find_one(doc! {"allowance_id": &allowance_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(period.reserved_quantity, 0);
    assert_eq!(period.consumed_quantity, 0);
    let after = wallet(&f).await;
    assert!(after.suspended);
    assert_eq!(after.reserved_credits, Credits::ZERO);
    let quote =
        f.db.collection::<MachineRuntimeQuote>(QUOTE_COLLECTION)
            .find_one(doc! {"_id": &q.id})
            .await
            .unwrap()
            .unwrap();
    assert!(quote.accepted_run_id.is_none());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn issued_live_quote_remains_acceptable_after_its_rate_is_retired() {
    let f = fixture("runtime_retired_live_quote").await;
    let q = quote(&f).await;
    f.billing
        .retire_machine_runtime_rate(&f.rate.id)
        .await
        .unwrap();
    assert!(
        f.billing
            .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
            .await
            .is_err()
    );
    let from = Utc::now();
    assert!(from < q.accept_before);
    let prepared = f
        .billing
        .prepare_machine_start(
            &f.owner,
            &q.id,
            &Uuid::new_v4().to_string(),
            from,
            from + Duration::seconds(20),
        )
        .await
        .unwrap();
    let window = accept(
        &f,
        &prepared,
        &Uuid::new_v4().to_string(),
        &Uuid::new_v4().to_string(),
        1,
    )
    .await
    .unwrap();
    let accepted = run(&f, &window.run_id).await;
    assert_eq!(accepted.quote_id, q.id);
    assert_eq!(accepted.rate.id, q.rate_version);
    assert_eq!(accepted.rate.credits_per_second, q.credits_per_second);
    assert_eq!(window.rate_version, f.rate.id);
    assert_eq!(
        wallet(&f).await.reserved_credits,
        q.credits_per_second.checked_mul(20).unwrap()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn accepted_old_and_new_versions_coexist_through_renewal_settlement_and_lago() {
    let f = fixture("runtime_frozen_versions").await;
    let now = Utc::now();
    let (old, _, _) = start(
        &f,
        &Uuid::new_v4().to_string(),
        now - Duration::seconds(3),
        now + Duration::seconds(20),
    )
    .await;
    let old_ready = Utc::now();
    ready(&f, &old.run_id, 1, old_ready).await;
    let new = f
        .billing
        .stage_initial_machine_runtime_rate()
        .await
        .unwrap();
    let new_price: Credits = "0.000080".parse().unwrap();
    f.db.collection::<Document>(RATE_COLLECTION)
        .update_one(
            doc! {"_id":&new.id},
            doc! {"$set":{"credits_per_second":new_price}},
        )
        .await
        .unwrap();
    f.billing.sync_machine_runtime_rate(&new.id).await.unwrap();
    f.billing
        .retire_machine_runtime_rate(&f.rate.id)
        .await
        .unwrap();
    // A mutable generic cache may drift or retire independently. Accepted
    // runtime settlement must use its frozen funding rate, never this cache.
    f.db.collection::<crate::models::billing_rate_cache::BillingRateCache>(
        crate::models::billing_rate_cache::COLLECTION_NAME,
    )
    .insert_one(crate::models::billing_rate_cache::BillingRateCache {
        id: crate::models::billing_rate_cache::BillingRateCache::cache_id(
            &f.rate.lago_metric_code,
            None,
        ),
        lago_metric_code: f.rate.lago_metric_code.clone(),
        model: None,
        credits_per_unit_micros: 999,
        credits_per_unit_pico: Some(999_000_000),
        synced_at: Utc::now(),
        retired_at: Some(Utc::now()),
    })
    .await
    .unwrap();
    assert!(
        f.billing
            .quote_machine_runtime(&f.owner, f.terms.clone(), &f.rate.id)
            .await
            .is_err()
    );
    let p = f
        .billing
        .prepare_machine_next_window(
            &f.owner,
            &old.run_id,
            &Uuid::new_v4().to_string(),
            now + Duration::seconds(20),
            now + Duration::seconds(30),
        )
        .await
        .unwrap();
    tx!(
        &f,
        s,
        f.billing.reserve_machine_next_window_in_session(&mut s, &p)
    )
    .unwrap();
    let q = f
        .billing
        .quote_machine_runtime(&f.owner, f.terms.clone(), &new.id)
        .await
        .unwrap();
    let p = f
        .billing
        .prepare_machine_start(
            &f.owner,
            &q.id,
            &Uuid::new_v4().to_string(),
            now - Duration::seconds(3),
            now + Duration::seconds(20),
        )
        .await
        .unwrap();
    let newer = accept(
        &f,
        &p,
        &Uuid::new_v4().to_string(),
        &Uuid::new_v4().to_string(),
        1,
    )
    .await
    .unwrap();
    let new_ready = Utc::now();
    ready(&f, &newer.run_id, 1, new_ready).await;
    for (id, ready_at, price, code) in [
        (
            &old.run_id,
            old_ready,
            f.rate.credits_per_second,
            f.rate.lago_metric_code.clone(),
        ),
        (&newer.run_id, new_ready, new_price, new.lago_metric_code),
    ] {
        stopped(&f, id, 1, ready_at + Duration::seconds(1)).await;
        finalize(&f, id).await;
        settle(&f, id).await;
        let intervals: Vec<MachineRuntimeInterval> =
            f.db.collection::<MachineRuntimeInterval>(INTERVAL_COLLECTION)
                .find(doc! {"run_id":id})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
        let mut total = Credits::ZERO;
        for interval in intervals {
            let row =
                f.db.collection::<UsageMeterRow>(USAGE)
                    .find_one(doc! {"_id":interval.usage_row_id})
                    .await
                    .unwrap()
                    .unwrap();
            let funding = row.funding.as_ref().unwrap();
            assert_eq!(funding.settlement_rate, Some(price));
            assert_eq!(row.lago_metric_code, code);
            total = total
                .checked_add(funding.wallet_charge_credits.unwrap())
                .unwrap();
            verify_usage_provider_rate(&f.db, f.lago.as_ref(), &row)
                .await
                .unwrap();
            assert_eq!(
                LagoEvent::from_usage_row(&row, Some("local-subscription".into()))
                    .unwrap()
                    .code,
                code
            );
        }
        assert_eq!(total, price);
    }
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    assert_eq!(
        wallet(&f).await.pending_lago_debits,
        f.rate.credits_per_second.checked_add(new_price).unwrap()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn fractional_windows_finalize_atomically_once_and_drop_the_final_fraction() {
    let f = fixture("runtime_fractional_windows").await;
    let base = Utc::now();
    // Keep the fractional carry while leaving time for admission on loaded runners.
    let boundary = base + Duration::seconds(30) + Duration::milliseconds(600);
    let end = boundary + Duration::seconds(30);
    let stop = boundary + Duration::milliseconds(600) - Duration::nanoseconds(1);
    let (w, _, _) = start(
        &f,
        &Uuid::new_v4().to_string(),
        base - Duration::seconds(1),
        boundary,
    )
    .await;
    let whole_seconds = (boundary - Utc::now() - Duration::milliseconds(600)).num_seconds();
    assert!(
        whole_seconds > 0,
        "initial admission exhausted the test window"
    );
    let ready_at = boundary - Duration::seconds(whole_seconds) - Duration::milliseconds(600);
    wait_for_evidence(ready_at).await;
    ready(&f, &w.run_id, 1, ready_at).await;
    let next = f
        .billing
        .prepare_machine_next_window(
            &f.owner,
            &w.run_id,
            &Uuid::new_v4().to_string(),
            boundary,
            end,
        )
        .await
        .unwrap();
    tx!(
        &f,
        s,
        f.billing
            .reserve_machine_next_window_in_session(&mut s, &next)
    )
    .unwrap();
    wait_for_evidence(boundary).await;
    tx!(
        &f,
        s,
        f.billing.prove_machine_running_in_session(
            &mut s,
            &w.run_id,
            1,
            boundary,
            "first-window-proof"
        )
    )
    .unwrap();
    let mut s = f.db.client().start_session().await.unwrap();
    s.start_transaction().await.unwrap();
    assert!(
        f.billing
            .finalize_machine_interval_in_session(&mut s, &w.run_id)
            .await
            .unwrap()
            .is_some()
    );
    s.abort_transaction().await.unwrap();
    assert_eq!(run(&f, &w.run_id).await.cumulative_covered_ns, 0);
    assert!(
        f.db.collection::<UsageMeterRow>(USAGE)
            .find_one(doc! {"_id":&w.usage_row_id})
            .await
            .unwrap()
            .unwrap()
            .quantity
            .is_none()
    );
    assert_eq!(finalize(&f, &w.run_id).await.len(), 1);
    let first = run(&f, &w.run_id).await;
    assert_eq!(
        first.cumulative_covered_ns,
        whole_seconds * SECOND_NS + 600_000_000
    );
    assert_eq!(first.cumulative_seconds, whole_seconds);
    assert!(finalize(&f, &w.run_id).await.is_empty());
    settle(&f, &w.run_id).await;
    wait_for_evidence(stop).await;
    stopped(&f, &w.run_id, 1, stop).await;
    assert_eq!(finalize(&f, &w.run_id).await.len(), 1);
    settle(&f, &w.run_id).await;
    let saved = run(&f, &w.run_id).await;
    assert_eq!(
        saved.cumulative_covered_ns,
        whole_seconds * SECOND_NS + 1_199_999_999
    );
    assert_eq!(saved.cumulative_seconds, whole_seconds + 1);
    assert_eq!(
        wallet(&f).await.pending_lago_debits,
        f.rate
            .credits_per_second
            .checked_mul(whole_seconds + 1)
            .unwrap()
    );
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    tx!(
        &f,
        s,
        f.billing
            .close_machine_runtime_in_session(&mut s, &w.run_id)
    )
    .unwrap();
    assert_eq!(
        run(&f, &w.run_id).await.state,
        MachineRuntimeRunState::Closed
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn unknown_stop_holds_liability_and_topup_never_resumes_a_closed_run() {
    let f = fixture("runtime_pending_stop").await;
    let now = Utc::now();
    let node = Uuid::new_v4().to_string();
    let (w, _, _) = start(
        &f,
        &node,
        now - Duration::seconds(5),
        now + Duration::seconds(20),
    )
    .await;
    let ready_at = Utc::now();
    ready(&f, &w.run_id, 1, ready_at).await;
    tx!(
        &f,
        s,
        f.billing
            .request_machine_runtime_stop_in_session(&mut s, &w.run_id, 1, false)
    )
    .unwrap();
    let held = wallet(&f).await.reserved_credits;
    assert!(finalize(&f, &w.run_id).await.is_empty());
    assert_eq!(
        f.billing
            .settle_machine_runtime(&w.run_id, 100)
            .await
            .unwrap(),
        0
    );
    assert!(
        tx!(
            &f,
            s,
            f.billing
                .close_machine_runtime_in_session(&mut s, &w.run_id)
        )
        .is_err()
    );
    assert_eq!(
        super::super::reservation::abandon_stale_unforwarded(&f.db, Utc::now() + Duration::days(1))
            .await
            .unwrap(),
        0
    );
    assert_eq!(wallet(&f).await.reserved_credits, held);
    let detail = f
        .billing
        .machine_runtime_detail(&f.owner, &w.run_id, 0)
        .await
        .unwrap();
    assert_eq!(detail.pending_credits, held);
    assert_eq!(detail.attribution, "machine_runtime");
    assert!(!detail.service_costs_included);
    let q = quote(&f).await;
    let p = f
        .billing
        .prepare_machine_start(
            &f.owner,
            &q.id,
            &Uuid::new_v4().to_string(),
            now,
            now + Duration::seconds(20),
        )
        .await
        .unwrap();
    assert!(
        accept(&f, &p, &node, &Uuid::new_v4().to_string(), 2)
            .await
            .is_err()
    );
    let stop = ready_at + Duration::seconds(1);
    stopped(&f, &w.run_id, 1, stop).await;
    let stopped_detail = f
        .billing
        .machine_runtime_detail(&f.owner, &w.run_id, 0)
        .await
        .unwrap();
    assert_eq!(stopped_detail.pending_credits, held);
    assert_eq!(stopped_detail.reserved_credits, held);
    finalize(&f, &w.run_id).await;
    settle(&f, &w.run_id).await;
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    assert_eq!(
        wallet(&f).await.pending_lago_debits,
        f.rate.credits_per_second
    );
    tx!(
        &f,
        s,
        f.billing
            .close_machine_runtime_in_session(&mut s, &w.run_id)
    )
    .unwrap();
    super::super::webhook::refresh_wallet_balance(&f.db, &f.owner, Credits::from_whole(20))
        .await
        .unwrap();
    assert_eq!(
        run(&f, &w.run_id).await.state,
        MachineRuntimeRunState::Closed
    );
    assert!(
        f.billing
            .prepare_machine_next_window(
                &f.owner,
                &w.run_id,
                &Uuid::new_v4().to_string(),
                now,
                now + Duration::seconds(25)
            )
            .await
            .is_err()
    );
    let fresh = quote(&f).await;
    let p = f
        .billing
        .prepare_machine_start(
            &f.owner,
            &fresh.id,
            &Uuid::new_v4().to_string(),
            stop + Duration::milliseconds(1),
            stop + Duration::seconds(25),
        )
        .await
        .unwrap();
    let resumed = accept(&f, &p, &node, &Uuid::new_v4().to_string(), 2)
        .await
        .unwrap();
    assert_ne!(resumed.run_id, w.run_id);
    assert_eq!(
        run(&f, &resumed.run_id).await.state,
        MachineRuntimeRunState::Accepted
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn unfunded_tail_is_platform_residual_and_unused_finite_funds_are_released() {
    let f = fixture("runtime_unfunded_tail").await;
    let base = Utc::now();
    let end = base + Duration::seconds(3);
    let (w, _, _) = start(
        &f,
        &Uuid::new_v4().to_string(),
        base - Duration::seconds(2),
        end,
    )
    .await;
    let mut ready_at = Utc::now();
    if (ns(end).unwrap() - ns(ready_at).unwrap()) % SECOND_NS == 0 {
        ready_at += Duration::nanoseconds(1);
    }
    assert!(
        ready_at < end,
        "initial admission exhausted its three-second test window"
    );
    let covered_ns = ns(end).unwrap() - ns(ready_at).unwrap();
    let charged_seconds = covered_ns / SECOND_NS;
    assert_ne!(covered_ns % SECOND_NS, 0);
    let expected_charge = f
        .rate
        .credits_per_second
        .checked_mul(charged_seconds)
        .unwrap();
    ready(&f, &w.run_id, 1, ready_at).await;
    let stop = end + Duration::seconds(1);
    wait_for_evidence(stop).await;
    stopped(&f, &w.run_id, 1, stop).await;
    finalize(&f, &w.run_id).await;
    settle(&f, &w.run_id).await;
    assert_eq!(run(&f, &w.run_id).await.cumulative_covered_ns, covered_ns);
    assert_eq!(run(&f, &w.run_id).await.cumulative_seconds, charged_seconds);
    assert_eq!(wallet(&f).await.pending_lago_debits, expected_charge);
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    tx!(
        &f,
        s,
        f.billing
            .close_machine_runtime_in_session(&mut s, &w.run_id)
    )
    .unwrap();
    assert_eq!(run(&f, &w.run_id).await.platform_residual_ns, 1_000_000_000);
    // Closing again cannot turn platform residual or a final fractional
    // covered second into customer debt or count residual a second time.
    tx!(
        &f,
        s,
        f.billing
            .close_machine_runtime_in_session(&mut s, &w.run_id)
    )
    .unwrap();
    assert_eq!(run(&f, &w.run_id).await.platform_residual_ns, 1_000_000_000);
    assert_eq!(wallet(&f).await.pending_lago_debits, expected_charge);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn controlled_replacement_requires_settled_fence_and_never_overlaps_generations() {
    let f = fixture("runtime_replacement").await;
    let now = Utc::now();
    let (w, _, _) = start(
        &f,
        &Uuid::new_v4().to_string(),
        now - Duration::seconds(5),
        now + Duration::seconds(20),
    )
    .await;
    let first_ready = Utc::now();
    let old_stop = first_ready + Duration::milliseconds(600);
    ready(&f, &w.run_id, 1, first_ready).await;
    tx!(
        &f,
        s,
        f.billing
            .request_machine_runtime_stop_in_session(&mut s, &w.run_id, 1, true)
    )
    .unwrap();
    assert!(
        f.billing
            .prepare_machine_replacement(
                &f.owner,
                &w.run_id,
                &Uuid::new_v4().to_string(),
                old_stop,
                now + Duration::seconds(25)
            )
            .await
            .is_err()
    );
    stopped(&f, &w.run_id, 1, old_stop).await;
    let p = f
        .billing
        .prepare_machine_replacement(
            &f.owner,
            &w.run_id,
            &Uuid::new_v4().to_string(),
            old_stop,
            now + Duration::seconds(25),
        )
        .await
        .unwrap();
    assert!(
        tx!(
            &f,
            s,
            f.billing
                .reserve_machine_replacement_in_session(&mut s, &p, 2)
        )
        .is_err()
    );
    finalize(&f, &w.run_id).await;
    settle(&f, &w.run_id).await;
    let overlap = f
        .billing
        .prepare_machine_replacement(
            &f.owner,
            &w.run_id,
            &Uuid::new_v4().to_string(),
            old_stop - Duration::nanoseconds(1),
            now + Duration::seconds(25),
        )
        .await
        .unwrap();
    assert!(
        tx!(
            &f,
            s,
            f.billing
                .reserve_machine_replacement_in_session(&mut s, &overlap, 2)
        )
        .is_err()
    );
    let replacement = tx!(
        &f,
        s,
        f.billing
            .reserve_machine_replacement_in_session(&mut s, &p, 2)
    )
    .unwrap();
    assert_eq!(replacement.run_id, w.run_id);
    assert_eq!(replacement.rate_version, f.rate.id);
    assert!(
        tx!(
            &f,
            s,
            f.billing.prove_machine_ready_in_session(
                &mut s,
                &w.run_id,
                1,
                old_stop,
                "stale-generation"
            )
        )
        .is_err()
    );
    let replacement_ready = Utc::now().max(old_stop + Duration::milliseconds(100));
    let replacement_stop = replacement_ready + Duration::milliseconds(600);
    ready(&f, &w.run_id, 2, replacement_ready).await;
    wait_for_evidence(replacement_stop).await;
    stopped(&f, &w.run_id, 2, replacement_stop).await;
    finalize(&f, &w.run_id).await;
    settle(&f, &w.run_id).await;
    assert_eq!(
        run(&f, &w.run_id).await.cumulative_covered_ns,
        1_200_000_000
    );
    assert_eq!(run(&f, &w.run_id).await.cumulative_seconds, 1);
    assert_eq!(
        wallet(&f).await.pending_lago_debits,
        f.rate.credits_per_second
    );
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn later_topup_cannot_retroactively_fund_an_expired_running_cutoff() {
    let f = fixture("runtime_expired_cutoff").await;
    let from = Utc::now();
    let until = from + Duration::seconds(3);
    let (w, _, _) = start(&f, &Uuid::new_v4().to_string(), from, until).await;
    ready(&f, &w.run_id, 1, Utc::now()).await;
    let held = wallet(&f).await.reserved_credits;
    // UTC is the production authority. A single monotonic sleep does not prove
    // that wall time crossed the persisted cutoff (host clock correction can
    // move it backwards). Establish and retain the exact precondition.
    let persisted_cutoff = run(&f, &w.run_id).await.covered_until_ns;
    assert_eq!(persisted_cutoff, ns(until).unwrap());
    let wait_started = std::time::Instant::now();
    while ns(Utc::now()).unwrap() <= persisted_cutoff {
        assert!(
            wait_started.elapsed() < std::time::Duration::from_secs(15),
            "UTC did not reach original cutoff: now_ns={}, cutoff_ns={}",
            ns(Utc::now()).unwrap(),
            persisted_cutoff
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let before_count =
        f.db.collection::<Document>(INTERVAL_COLLECTION)
            .count_documents(doc! {"run_id":&w.run_id})
            .await
            .unwrap();
    assert_eq!(before_count, 1);
    // Isolated fixture simulates newly available funds after the old cutoff.
    // The reservation must still refuse to convert the unfunded gap into debt.
    f.db.collection::<Document>(WALLETS)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$inc":{"balance_credits":Credits::from_whole(10)}},
        )
        .await
        .unwrap();
    let next = f
        .billing
        .prepare_machine_next_window(
            &f.owner,
            &w.run_id,
            &Uuid::new_v4().to_string(),
            until,
            until + Duration::seconds(20),
        )
        .await;
    if let Ok(next) = next {
        let before_ns = ns(Utc::now()).unwrap();
        assert!(
            before_ns >= persisted_cutoff,
            "UTC cutoff precondition changed before reserve: now_ns={before_ns}, cutoff_ns={persisted_cutoff}"
        );
        let result = tx!(
            &f,
            s,
            f.billing
                .reserve_machine_next_window_in_session(&mut s, &next)
        );
        let after_count =
            f.db.collection::<Document>(INTERVAL_COLLECTION)
                .count_documents(doc! {"run_id":&w.run_id})
                .await
                .unwrap();
        assert!(result.is_err(), "expired UTC reservation must be rejected");
        assert_eq!(after_count, before_count);
    }
    assert_eq!(
        run(&f, &w.run_id).await.covered_until_ns,
        ns(until).unwrap()
    );
    assert_eq!(wallet(&f).await.reserved_credits, held);
    assert_eq!(
        f.db.collection::<Document>(INTERVAL_COLLECTION)
            .count_documents(doc! {"run_id":&w.run_id})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn settlement_marker_repair_advances_beyond_one_hundred_completed_usage_rows() {
    let f = fixture("runtime_marker_paging").await;
    let now = Utc::now();
    let (w, _, _) = start(
        &f,
        &Uuid::new_v4().to_string(),
        now,
        now + Duration::seconds(20),
    )
    .await;
    // A stop before readiness finalizes zero billable seconds and releases
    // the initial reservation through the real funding/ledger path.
    stopped(&f, &w.run_id, 1, Utc::now()).await;
    finalize(&f, &w.run_id).await;
    settle(&f, &w.run_id).await;
    let intervals =
        f.db.collection::<MachineRuntimeInterval>(INTERVAL_COLLECTION);
    let usage = f.db.collection::<UsageMeterRow>(USAGE);
    let template = intervals
        .find_one(doc! {"run_id":&w.run_id})
        .await
        .unwrap()
        .unwrap();
    let row = usage
        .find_one(doc! {"_id":&template.usage_row_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.quantity, Some(0));
    assert!(row.released && row.funding.as_ref().unwrap().settled);
    let before = wallet(&f).await;
    let ledger_before =
        f.db.collection::<Document>(crate::models::billing_ledger::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap();
    intervals
        .update_one(
            doc! {"_id":&template.id},
            doc! {"$set":{"settlement_complete":false}},
        )
        .await
        .unwrap();
    let mut extra_intervals = Vec::new();
    let mut extra_usage = Vec::new();
    // Fault fixture: settlement committed but the derived marker did not.
    // These zero-liability rows exercise the paging repair without moving funds.
    for offset in 0..100_i64 {
        let mut interval = template.clone();
        interval.id = Uuid::new_v4().to_string();
        interval.interval_id = Uuid::new_v4().to_string();
        interval.usage_row_id = Uuid::new_v4().to_string();
        interval.covered_from_ns = template.covered_until_ns + offset;
        interval.covered_until_ns = interval.covered_from_ns + 1;
        interval.reserved_seconds = 0;
        interval.reserved_total = Credits::ZERO;
        interval.evidenced_ns = Some(0);
        interval.settlement_complete = false;
        let mut usage_row = row.clone();
        usage_row.id = interval.usage_row_id.clone();
        usage_row.transaction_id = format!(
            "machine:{}:{}:{}:{}",
            interval.node_id,
            interval.runtime_generation,
            interval.rate_version,
            interval.interval_id
        );
        usage_row.billing_request_id = interval.id.clone();
        usage_row.reserved_credits = Credits::ZERO;
        extra_intervals.push(interval);
        extra_usage.push(usage_row);
    }
    intervals.insert_many(extra_intervals).await.unwrap();
    usage.insert_many(extra_usage).await.unwrap();
    assert_eq!(
        f.billing
            .settle_machine_runtime(&w.run_id, 100)
            .await
            .unwrap(),
        100
    );
    assert_eq!(
        intervals
            .count_documents(doc! {"run_id":&w.run_id,"settlement_complete":{"$ne":true}})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.billing
            .settle_machine_runtime(&w.run_id, 100)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.billing
            .settle_machine_runtime(&w.run_id, 100)
            .await
            .unwrap(),
        0
    );
    let after = wallet(&f).await;
    assert_eq!(after.reserved_credits, before.reserved_credits);
    assert_eq!(after.pending_lago_debits, before.pending_lago_debits);
    assert_eq!(
        f.db.collection::<Document>(crate::models::billing_ledger::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        ledger_before
    );
    tx!(
        &f,
        s,
        f.billing
            .close_machine_runtime_in_session(&mut s, &w.run_id)
    )
    .unwrap();
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn runtime_price_retry_is_leased_bounded_and_retains_historical_versions() {
    let f = fixture("runtime_price_retry").await;
    let pending = f
        .billing
        .stage_initial_machine_runtime_rate()
        .await
        .unwrap();
    f.lago.available.store(false, Ordering::SeqCst);
    assert_eq!(f.billing.retry_machine_runtime_rates(1).await.unwrap(), 0);
    let first =
        f.db.collection::<Document>(RATE_COLLECTION)
            .find_one(doc! {"_id":&pending.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(first.get_str("sync_status").unwrap(), "failed");
    assert!(first.get_datetime("sync_retry_at").unwrap().to_chrono() > Utc::now());
    f.lago.available.store(true, Ordering::SeqCst);
    assert_eq!(
        f.billing.retry_machine_runtime_rates(1).await.unwrap(),
        0,
        "backoff remains authoritative"
    );
    f.db.collection::<Document>(RATE_COLLECTION).update_one(doc!{"_id":&pending.id},doc!{"$set":{"sync_retry_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    let (a, b) = tokio::join!(
        f.billing.retry_machine_runtime_rates(1),
        f.billing.retry_machine_runtime_rates(1)
    );
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    f.billing
        .retire_machine_runtime_rate(&pending.id)
        .await
        .unwrap();
    let listed = f.billing.list_machine_runtime_rates().await.unwrap();
    assert!(
        listed
            .iter()
            .any(|r| r.id == pending.id && r.retired && r.sync_status == PricingSyncStatus::Synced)
    );
    assert!(listed.iter().any(|r| r.id == f.rate.id));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn quote_preflight_reports_live_prerequisites_without_funding_or_account_mutations() {
    let f = fixture("runtime_quote_preflight").await;
    // Exact cutover is monotonic: once observed complete, production latches
    // readiness for this database. Exercise pending -> complete before the
    // first successful preflight rather than simulating an unsupported rollback.
    let marker =
        f.db.collection::<Document>("billing_migrations")
            .find_one_and_delete(doc! {"_id":"exact-v2"})
            .await
            .unwrap()
            .unwrap();
    let exact = f
        .billing
        .machine_runtime_preflight(&f.owner, "remote-machine", None)
        .await
        .unwrap();
    assert!(!exact.ready);
    assert!(!exact.exact_ready);
    assert_eq!(exact.reason, Some("exact_accounting_not_ready"));
    f.db.collection::<Document>("billing_migrations")
        .insert_one(marker)
        .await
        .unwrap();
    let before_wallet = bson::to_document(&wallet(&f).await).unwrap();
    let view = f
        .billing
        .machine_runtime_preflight(&f.owner, "remote-machine", None)
        .await
        .unwrap();
    assert!(view.ready);
    assert!(view.exact_ready);
    assert_eq!(view.wallet, "ready");
    assert_eq!(view.subscription, "active");
    assert_eq!(view.provider, "ready");
    assert_eq!(view.rate_version.as_deref(), Some(f.rate.id.as_str()));
    assert_eq!(view.credits_per_second, Some(f.rate.credits_per_second));
    assert!(!view.reservation_guarantee);
    assert_eq!(view.funds.unwrap().estimated_seconds, "250000");
    assert_eq!(bson::to_document(&wallet(&f).await).unwrap(), before_wallet);
    for collection in [
        QUOTE_COLLECTION,
        RUN_COLLECTION,
        INTERVAL_COLLECTION,
        "usage_allowance_periods",
    ] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    f.lago.available.store(false, Ordering::SeqCst);
    let unavailable = f
        .billing
        .machine_runtime_preflight(&f.owner, "remote-machine", None)
        .await
        .unwrap();
    assert!(!unavailable.ready);
    assert_eq!(unavailable.provider, "unavailable");
    f.db.collection::<Document>(WALLETS)
        .delete_many(doc! {})
        .await
        .unwrap();
    let missing = f
        .billing
        .machine_runtime_preflight(&f.owner, "remote-machine", None)
        .await
        .unwrap();
    assert_eq!(missing.wallet, "missing");
    assert!(missing.funds.is_none());
    assert_eq!(
        f.db.collection::<Document>(WALLETS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn runtime_profile_rate_binding_rejects_foreign_quote_before_writes() {
    let f = fixture("runtime_profile_quote_mismatch").await;
    let before = bson::to_document(&wallet(&f).await).unwrap();
    let mut foreign = f.terms.clone();
    foreign.profile_version = "test-only-foreign-profile".into();
    assert!(
        f.billing
            .quote_machine_runtime(&f.owner, foreign, &f.rate.id)
            .await
            .is_err()
    );
    assert_eq!(bson::to_document(&wallet(&f).await).unwrap(), before);
    for collection in [QUOTE_COLLECTION, RUN_COLLECTION, INTERVAL_COLLECTION] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    f.db.drop().await.unwrap();
}

#[path = "profile_binding_tests.rs"]
mod profile_binding;

#[tokio::test]
async fn runtime_preflight_wallet_seconds_use_gross_per_second_price() {
    let f = fixture("machine_preflight_gst").await;
    let mut config = super::super::taxes::zero_config();
    config.customer["taxes"] = serde_json::json!([{"code":"gst","name":"GST","rate":"9"}]);
    let profile =
        super::super::taxes::resolve_config(&f.owner, "local-subscription", &config).unwrap();
    f.db.collection::<crate::models::billing_tax_profile::BillingTaxProfile>(
        crate::models::billing_tax_profile::COLLECTION_NAME,
    )
    .insert_one(profile)
    .await
    .unwrap();
    let funds = f
        .billing
        .runtime_funds_estimate(&f.owner, &f.rate)
        .await
        .unwrap();
    let gross = super::super::taxes::gross_ceiling(f.rate.credits_per_second, "9".parse().unwrap())
        .unwrap();
    assert_eq!(
        funds.estimated_seconds,
        (Credits::from_whole(10).pico() / gross.pico()).to_string()
    );
    assert_eq!(funds.estimated_seconds, "229357");
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    let now = bson::DateTime::now();
    f.db.collection::<Document>("credit_grants").insert_one(doc! {
        "_id":Uuid::new_v4().to_string(), "batch_id":"batch", "recipient_user_id":&f.owner,
        "target_kind":"selected_users", "amount_credits":1_i64,
        "amount":Credits::from_whole(1), "remaining":Credits::from_whole(1), "reserved":Credits::ZERO,
        "scope":{"all_services":true,"service_ids":[],"service_slugs":[]}, "status":"active",
        "issued_ledgered_at":now, "granted_by":"admin", "terminal_amount":Credits::ZERO,
        "created_at":now,"updated_at":now,
    }).await.unwrap();
    let funds = f
        .billing
        .runtime_funds_estimate(&f.owner, &f.rate)
        .await
        .unwrap();
    // The extra grant buys 25,000 untaxed seconds, independently of the wallet.
    assert_eq!(funds.estimated_seconds, "254357");
    f.db.collection::<Document>(WALLETS)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$set":{"balance_credits":Credits::ZERO}},
        )
        .await
        .unwrap();
    f.db.collection::<Document>(crate::models::billing_tax_profile::COLLECTION_NAME)
        .delete_many(doc! {})
        .await
        .unwrap();
    f.lago.available.store(false, Ordering::SeqCst);
    let funds = f
        .billing
        .runtime_funds_estimate(&f.owner, &f.rate)
        .await
        .unwrap();
    assert_eq!(funds.estimated_seconds, "25000");
    f.db.drop().await.unwrap();
}
