//! Real replica-set transactions; only the external provider is replaced.
use super::super::lago_client::*;
use super::*;
use crate::{
    config::AppConfig,
    models::{downstream_service::DownstreamService, user::UserType},
};
use async_trait::async_trait;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Provider {
    owner: String,
    plan: String,
    rates: Mutex<Vec<PlanRate>>,
    available: AtomicBool,
    lost_ack: AtomicBool,
    attempts: AtomicUsize,
    events: Mutex<std::collections::BTreeMap<String, LagoEvent>>,
}
#[async_trait]
impl LagoApi for Provider {
    async fn ensure_customer(&self, _: &OwnerProvisionInput) -> AppResult<String> {
        panic!("must not provision")
    }
    async fn ensure_subscription(&self, _: &str, _: &str) -> AppResult<String> {
        panic!("must not provision")
    }
    async fn sync_standard_charge(&self, _: &str, input: &ServicePriceSync) -> AppResult<()> {
        if !self.available.load(Ordering::SeqCst) {
            return Err(unavailable("Test provider unavailable"));
        }
        let pico =
            i64::try_from(input.credits_per_unit.parse::<Credits>().unwrap().pico()).unwrap();
        let mut rates = self.rates.lock().unwrap();
        rates.retain(|r| r.lago_metric_code != input.metric_code);
        rates.push(PlanRate {
            lago_metric_code: input.metric_code.clone(),
            credits_per_unit_pico: Some(pico),
            credits_per_unit_micros: pico / 1_000_000,
        });
        Ok(())
    }
    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let mut events = self.events.lock().unwrap();
        if events.contains_key(&event.transaction_id) {
            return Err(LagoError::duplicate(
                "Retained transaction already accepted",
            ));
        }
        events.insert(event.transaction_id.clone(), event.clone());
        if self.lost_ack.swap(false, Ordering::SeqCst) {
            return Err(LagoError::retry("Accepted event response was lost"));
        }
        Ok(LagoAck {
            transaction_id: event.transaction_id.clone(),
        })
    }
    async fn record_events_batch(&self, _: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        panic!("unused")
    }
    async fn current_usage(&self, _: &str, _: &str) -> AppResult<LagoUsage> {
        Err(unavailable("Usage read unavailable in this fixture"))
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        Err(unavailable("Balance refresh unavailable in this fixture"))
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        Ok(vec![Entitlement {
            code: "all_services".into(),
            raw: serde_json::json!({}),
        }])
    }
    async fn tax_config(&self, _: &str, _: &str) -> AppResult<LagoTaxConfig> {
        Ok(super::super::taxes::zero_config())
    }
    async fn sandbox_subscription(
        &self,
        subscription: &str,
        _: &[String],
    ) -> AppResult<SandboxProviderSnapshot> {
        if !self.available.load(Ordering::SeqCst) {
            return Err(unavailable("Test provider unavailable"));
        }
        Ok(SandboxProviderSnapshot {
            subscription: RuntimeSubscription {
                external_id: subscription.into(),
                external_customer_id: self.owner.clone(),
                plan_code: self.plan.clone(),
                status: "active".into(),
            },
            rates: self.rates.lock().unwrap().clone(),
        })
    }
}

struct Fixture {
    db: Database,
    billing: BillingService,
    producer: ProducerAuthority,
    provider: Arc<Provider>,
    owner: String,
    service: DownstreamService,
}

fn allocation() -> SandboxAllocation {
    SandboxAllocation {
        cpu_millis: 1000,
        memory_mib: 1024,
        disk_gib: 10,
    }
}

async fn fixture(name: &str) -> Fixture {
    let db = crate::test_utils::connect_transaction_test_database(name).await;
    super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    crate::db::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(crate::test_utils::test_user(&owner, UserType::Person))
        .await
        .unwrap();
    crate::services::feature_flag_service::set_platform_override(
        &db,
        "experimental:billing",
        &crate::services::feature_flag_service::FlagTarget::Global,
        true,
        &owner,
    )
    .await
    .unwrap();
    crate::services::feature_flag_service::set_platform_override(
        &db,
        crate::services::feature_flag_service::CMA_SANDBOX_BILLING_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::User(owner.clone()),
        true,
        &owner,
    )
    .await
    .unwrap();
    let now = bson::DateTime::now();
    db.collection::<Document>(WALLETS).insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"owner_id":&owner,"lago_customer_id":&owner,
        "lago_wallet_id":"test-wallet","lago_subscription_id":"test-subscription","plan_kind":"prepaid",
        "balance_credits":Credits::from_whole(10),"reserved_credits":Credits::ZERO,"pending_lago_debits":Credits::ZERO,
        "pending_topup_expiry_credits":Credits::ZERO,"overdraft_cap_credits":Credits::ZERO,"has_payment_instrument":false,
        "suspended":false,"collection_state":"good","created_at":now,"updated_at":now,"balance_synced_at":now,
    }).await.unwrap();
    super::super::taxes::seed_test_profiles(&db).await;
    let producer_id = Uuid::new_v4().to_string();
    let jti = Uuid::new_v4().to_string();
    db.collection::<Document>("service_accounts").insert_one(doc! {
        "_id":&producer_id,"name":"CMA test","client_id":format!("sa_{}",Uuid::new_v4().simple()),
        "client_secret_hash":"test-only","secret_prefix":"test","allowed_scopes":WRITE_SCOPE,
        "is_active":true,"credential_generation":0_i64,"purpose":"general","created_by":&owner,"created_at":now,"updated_at":now,
    }).await.unwrap();
    db.collection::<Document>("service_account_tokens").insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"jti":&jti,"service_account_id":&producer_id,"scope":WRITE_SCOPE,
        "revoked":false,"credential_generation":0_i64,"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::hours(1)),"created_at":now,
    }).await.unwrap();
    let mut service = crate::test_utils::test_auto_connected_catalog_service();
    service.slug = "cma-test".into();
    let prices = METRICS
        .into_iter()
        .map(|metric| SandboxPriceSnapshot {
            metric,
            lago_metric_code: format!("cma_{}_v1", metric.as_str()),
            credits_per_unit: "0.00000001".parse().unwrap(),
        })
        .collect::<Vec<_>>();
    let mut rates = vec![];
    for price in &prices {
        let pico = i64::try_from(price.credits_per_unit.pico()).unwrap();
        db.collection::<Document>("billing_rate_cache").insert_one(doc! {
            "_id":format!("{}:*",price.lago_metric_code),"lago_metric_code":&price.lago_metric_code,
            "credits_per_unit_pico":pico,"credits_per_unit_micros":pico/1_000_000,"synced_at":now,
        }).await.unwrap();
        rates.push(PlanRate {
            lago_metric_code: price.lago_metric_code.clone(),
            credits_per_unit_pico: Some(pico),
            credits_per_unit_micros: pico / 1_000_000,
        });
    }
    db.collection::<DownstreamService>("downstream_services")
        .insert_one(&service)
        .await
        .unwrap();
    let config = Arc::new(AppConfig {
        billing_enabled: true,
        cma_billing_enabled: true,
        cma_billing_service_id: Some(service.id.clone()),
        cma_billing_service_account_id: Some(producer_id.clone()),
        ..crate::test_utils::test_app_config()
    });
    let provider = Arc::new(Provider {
        owner: owner.clone(),
        plan: config.lago_plan_code.clone(),
        rates: Mutex::new(rates),
        available: AtomicBool::new(true),
        lost_ack: AtomicBool::new(false),
        attempts: AtomicUsize::new(0),
        events: Mutex::new(Default::default()),
    });
    super::super::sandbox_rates::ensure_indexes(&db)
        .await
        .unwrap();
    db.collection::<crate::models::sandbox_rate_card::SandboxRateCard>(
        crate::models::sandbox_rate_card::COLLECTION_NAME,
    )
    .insert_one(crate::models::sandbox_rate_card::SandboxRateCard {
        id: Uuid::new_v4().to_string(),
        version: 1,
        effective_from: Utc::now() - Duration::days(1),
        plan_code: config.lago_plan_code.clone(),
        prices,
        sync_status: PricingSyncStatus::Synced,
        created_by: None,
        created_at: Utc::now(),
    })
    .await
    .unwrap();
    let billing = BillingService::new_with_lago(db.clone(), config, provider.clone());
    let producer = authorize_producer(&db, Some(&producer_id), &producer_id, WRITE_SCOPE, &jti)
        .await
        .unwrap();
    Fixture {
        db,
        billing,
        producer,
        provider,
        owner,
        service,
    }
}

async fn request(f: &Fixture, seconds: i64) -> ReserveSandboxWindow {
    let sandbox = format!("sbx_{}", Uuid::new_v4().simple());
    let quote = f
        .billing
        .quote_sandbox(&f.producer, &f.owner, &sandbox, allocation())
        .await
        .unwrap();
    ReserveSandboxWindow {
        request_id: Uuid::new_v4().to_string(),
        quote_id: quote.id,
        owner_id: f.owner.clone(),
        sandbox_id: sandbox,
        runtime_generation: 1,
        operation_id: Uuid::new_v4().to_string(),
        previous_window_id: None,
        seconds,
        allocation: allocation(),
    }
}

async fn wallet(f: &Fixture) -> BillingWallet {
    f.db.collection::<BillingWallet>(WALLETS)
        .find_one(doc! {"owner_id":&f.owner})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn committed_receipt_replays_during_provider_outage_and_disabled_admission() {
    let mut f = fixture("sandbox_receipt").await;
    let input = request(&f, 60).await;
    let first = f
        .billing
        .reserve_sandbox_window(&f.producer, input.clone())
        .await
        .unwrap();
    let held = wallet(&f).await.reserved_credits;
    assert!(held > Credits::ZERO);
    assert_eq!(first.components.len(), 3);
    f.provider.available.store(false, Ordering::SeqCst);
    Arc::make_mut(&mut f.billing.config).cma_billing_enabled = false;
    let replay = f
        .billing
        .reserve_sandbox_window(&f.producer, input.clone())
        .await
        .unwrap();
    assert_eq!(replay.id, first.id);
    assert_eq!(wallet(&f).await.reserved_credits, held);
    let mut changed = input;
    changed.seconds += 1;
    assert!(matches!(
        f.billing.reserve_sandbox_window(&f.producer, changed).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(
        f.db.collection::<Document>(USAGE)
            .count_documents(doc! {})
            .await
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn three_component_insufficient_funds_rolls_back_every_hold_and_acceptance() {
    let f = fixture("sandbox_atomic").await;
    let input = request(&f, 60).await;
    f.db.collection::<Document>(WALLETS)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$set":{"balance_credits":"0.001218".parse::<Credits>().unwrap()}},
        )
        .await
        .unwrap();
    assert!(
        f.billing
            .reserve_sandbox_window(&f.producer, input.clone())
            .await
            .is_err()
    );
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    for collection in [WINDOW_COLLECTION, CURSOR_COLLECTION, USAGE] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    let quote =
        f.db.collection::<SandboxBillingQuote>(QUOTE_COLLECTION)
            .find_one(doc! {"_id":&input.quote_id})
            .await
            .unwrap()
            .unwrap();
    assert!(quote.accepted_at.is_none());
}

#[tokio::test]
async fn concurrent_sandboxes_share_the_same_strict_wallet_capacity() {
    let f = fixture("sandbox_competing").await;
    let one = request(&f, 60).await;
    let two = request(&f, 60).await;
    f.db.collection::<Document>(WALLETS)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$set":{"balance_credits":"0.0013".parse::<Credits>().unwrap()}},
        )
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        f.billing.reserve_sandbox_window(&f.producer, one),
        f.billing.reserve_sandbox_window(&f.producer, two)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        f.db.collection::<Document>(WINDOW_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let wallet = wallet(&f).await;
    assert!(wallet.reserved_credits <= wallet.balance_credits);
}

#[tokio::test]
async fn producer_revocation_after_extraction_cannot_commit_money() {
    let f = fixture("sandbox_revoke").await;
    let input = request(&f, 60).await;
    f.db.collection::<Document>("service_account_tokens")
        .update_one(
            doc! {"jti":&f.producer.token_jti},
            doc! {"$set":{"revoked":true}},
        )
        .await
        .unwrap();
    assert!(matches!(
        f.billing.reserve_sandbox_window(&f.producer, input).await,
        Err(AppError::Forbidden(_))
    ));
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    assert!(
        authorize_producer(
            &f.db,
            Some(&f.producer.id),
            &f.producer.id,
            "proxy",
            &f.producer.token_jti
        )
        .await
        .is_err()
    );
    assert!(
        authorize_producer(
            &f.db,
            Some("another-account"),
            &f.producer.id,
            WRITE_SCOPE,
            &f.producer.token_jti
        )
        .await
        .is_err()
    );
}

// Move a synthetic funded test window into the past without sleeping. The
// duration, funded quantities and every identity stay exactly as admitted.
async fn elapsed(f: &Fixture, mut window: SandboxBillingWindow) -> SandboxBillingWindow {
    let duration = window.covered_until - window.covered_from;
    window.covered_until = Utc::now().timestamp() - 10;
    window.covered_from = window.covered_until - duration;
    f.db.collection::<SandboxBillingWindow>(WINDOW_COLLECTION)
        .replace_one(doc! {"_id":&window.id}, &window)
        .await
        .unwrap();
    window
}
fn interval(
    revision: i64,
    start: i64,
    end: i64,
    allocation: SandboxAllocation,
) -> SandboxIntervalEvidence {
    SandboxIntervalEvidence {
        revision,
        started_at: start,
        ended_at: end,
        quantity: SandboxUsageQuantity {
            cpu_millis_seconds: allocation.cpu_millis * (end - start),
            memory_mib_seconds: allocation.memory_mib * (end - start),
            disk_gib_seconds: allocation.disk_gib * (end - start),
        },
        allocation,
    }
}
fn evidence(window: &SandboxBillingWindow) -> SandboxEvidence {
    SandboxEvidence {
        window_id: window.id.clone(),
        owner_id: window.owner_id.clone(),
        sandbox_id: window.sandbox_id.clone(),
        runtime_generation: window.runtime_generation,
        intervals: vec![
            interval(
                1,
                window.covered_from,
                window.covered_from + 1,
                allocation(),
            ),
            interval(
                2,
                window.covered_from + 1,
                window.covered_until,
                SandboxAllocation {
                    cpu_millis: 0,
                    memory_mib: 0,
                    disk_gib: 10,
                },
            ),
        ],
    }
}

#[tokio::test]
async fn paused_compute_and_retained_disk_settle_exactly_once_through_shared_ledger() {
    let f = fixture("sandbox_settle").await;
    let input = request(&f, 2).await;
    let window = elapsed(
        &f,
        f.billing
            .reserve_sandbox_window(&f.producer, input)
            .await
            .unwrap(),
    )
    .await;
    let proof = evidence(&window);
    let completed = f
        .billing
        .finalize_sandbox_window(&f.producer, proof.clone())
        .await
        .unwrap();
    assert_eq!(
        completed
            .components
            .iter()
            .map(|c| c.finalized_quantity.unwrap())
            .collect::<Vec<_>>(),
        vec![1000, 1024, 20]
    );
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    let debits = wallet(&f).await.pending_lago_debits;
    assert_eq!(debits, "0.00002044".parse().unwrap());
    let postings =
        f.db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap();
    f.billing
        .finalize_sandbox_window(&f.producer, proof.clone())
        .await
        .unwrap();
    assert_eq!(wallet(&f).await.pending_lago_debits, debits);
    assert_eq!(
        f.db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        postings
    );
    let mut changed = proof;
    changed.intervals[1] = interval(
        2,
        window.covered_from + 1,
        window.covered_until,
        allocation(),
    );
    assert!(matches!(
        f.billing
            .finalize_sandbox_window(&f.producer, changed)
            .await,
        Err(AppError::Conflict(_))
    ));
    let row =
        f.db.collection::<UsageMeterRow>(USAGE)
            .find_one(doc! {"_id":&completed.components[0].usage_row_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        verify_usage_provider_rate(&f.db, f.provider.as_ref(), &row)
            .await
            .unwrap()
            .as_deref(),
        Some("test-subscription")
    );
    f.provider.available.store(false, Ordering::SeqCst);
    assert!(
        verify_usage_provider_rate(&f.db, f.provider.as_ref(), &row)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn incomplete_future_or_wrong_identity_evidence_cannot_release_holds() {
    let f = fixture("sandbox_bad_evidence").await;
    let input = request(&f, 2).await;
    let funded = f
        .billing
        .reserve_sandbox_window(&f.producer, input)
        .await
        .unwrap();
    let held = wallet(&f).await.reserved_credits;
    assert!(
        f.billing
            .finalize_sandbox_window(&f.producer, evidence(&funded))
            .await
            .is_err()
    );
    let window = elapsed(&f, funded).await;
    let mut incomplete = evidence(&window);
    incomplete.intervals.pop();
    assert!(
        f.billing
            .finalize_sandbox_window(&f.producer, incomplete)
            .await
            .is_err()
    );
    let mut wrong = evidence(&window);
    wrong.runtime_generation += 1;
    assert!(
        f.billing
            .finalize_sandbox_window(&f.producer, wrong)
            .await
            .is_err()
    );
    assert_eq!(wallet(&f).await.reserved_credits, held);
    assert_eq!(
        f.db.collection::<Document>(USAGE)
            .count_documents(doc! {"quantity":{"$ne":bson::Bson::Null}})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn catalog_identity_drift_and_expired_quote_do_not_admit_usage() {
    let f = fixture("sandbox_price_drift").await;
    let input = request(&f, 60).await;
    f.db.collection::<Document>(QUOTE_COLLECTION).update_one(doc! {"_id":&input.quote_id},doc! {"$set":{"accept_before":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    assert!(
        f.billing
            .reserve_sandbox_window(&f.producer, input)
            .await
            .is_err()
    );
    let input = request(&f, 60).await;
    f.db.collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&f.service.id},
            doc! {"$set":{"slug":"changed-catalog-identity"}},
        )
        .await
        .unwrap();
    assert!(matches!(
        f.billing.reserve_sandbox_window(&f.producer, input).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
}

mod ingestion;
mod routes;

mod rates;
