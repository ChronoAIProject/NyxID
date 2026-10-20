//! Local lifecycle/Billing integration: real MongoDB majority transactions;
//! only the external Lago protocol and Kubernetes observations are fixtures.
use super::*;
use crate::config::AppConfig;
use crate::models::remote_machine_operation::{COLLECTION_NAME as OPERATIONS, LifecycleAction};
use crate::models::{
    billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
    credits::Credits,
    machine_runtime_billing::*,
};
use crate::services::billing::lago_client::{
    Entitlement, LagoAck, LagoApi, LagoError, LagoEvent, LagoUsage, OwnerProvisionInput, PlanRate,
    PlanRates, RuntimeSubscription, ServicePriceSync,
};
use crate::services::billing::machine_runtime::SKU;
use crate::services::remote_machine_native;
use crate::services::remote_machine_service::{CreateMachineInput, LifecycleRequest};
use async_trait::async_trait;
use serde_json::json;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use uuid::Uuid;
fn unavailable(reason: &str) -> AppError {
    AppError::BillingProviderUnavailable(reason.into())
}
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
    billing: Arc<BillingService>,
    lago: Arc<LocalLago>,
    owner: String,
    rate: MachineRuntimeRate,
}
async fn fixture(label: &str) -> Fixture {
    let db = crate::test_utils::connect_transaction_test_database(label).await;
    crate::services::billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        crate::services::billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    crate::db::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    db.collection("users")
        .insert_one(crate::test_utils::test_user(
            &owner,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    db.collection::<Document>(WALLETS).insert_one(doc!{
      "_id":Uuid::new_v4().to_string(),"owner_id":&owner,"lago_customer_id":&owner,"lago_wallet_id":"local-wallet","lago_subscription_id":"local-subscription",
      "plan_kind":"prepaid","balance_credits":Credits::from_whole(10),"reserved_credits":Credits::ZERO,"pending_lago_debits":Credits::ZERO,"pending_topup_expiry_credits":Credits::ZERO,
      "overdraft_cap_credits":Credits::ZERO,"has_payment_instrument":false,"suspended":false,"collection_state":"good","created_at":now,"updated_at":now,"balance_synced_at":now
    }).await.unwrap();
    let config = Arc::new(AppConfig {
        billing_enabled: true,
        billing_allocations_enabled: false,
        lago_api_url: None,
        lago_api_key: None,
        ..crate::test_utils::test_app_config()
    });
    let lago = Arc::new(LocalLago::new(owner.clone(), config.lago_plan_code.clone()));
    let billing = Arc::new(BillingService::new_with_lago(
        db.clone(),
        config,
        lago.clone(),
    ));
    let draft = billing.stage_initial_machine_runtime_rate().await.unwrap();
    billing.sync_machine_runtime_rate(&draft.id).await.unwrap();
    let rate = db
        .collection::<MachineRuntimeRate>(RATE_COLLECTION)
        .find_one(doc! {"_id":draft.id})
        .await
        .unwrap()
        .unwrap();
    Fixture {
        db,
        billing,
        lago,
        owner,
        rate,
    }
}
async fn personal_remote_flag(f: &Fixture, enabled: bool) {
    use crate::services::feature_flag_service::{self, FlagTarget, REMOTE_MACHINES_FLAG_KEY};
    feature_flag_service::set_platform_override(
        &f.db,
        REMOTE_MACHINES_FLAG_KEY,
        &FlagTarget::User(f.owner.clone()),
        enabled,
        &f.owner,
    )
    .await
    .unwrap();
    assert_eq!(
        feature_flag_service::personal_flag_enabled(&f.db, &f.owner, REMOTE_MACHINES_FLAG_KEY)
            .await
            .unwrap(),
        enabled
    );
}

async fn input(f: &Fixture) -> CreateMachineInput {
    let mut input = CreateMachineInput::fixed(
        &f.owner,
        false,
        &format!("registry.test/node@sha256:{}", "a".repeat(64)),
    );
    input.idempotency_key = Uuid::new_v4().to_string();
    let quote = f
        .billing
        .quote_machine_runtime(
            &f.owner,
            terms(&input.profile, "remote-machine-mvp").unwrap(),
            &f.rate.id,
        )
        .await
        .unwrap();
    input.request_body = json!({"free_slot":false,"quote_id":quote.id,"accept_account_balance":true,"policy_version":crate::models::remote_machine::POLICY_VERSION,"data_policy_version":crate::models::remote_machine::DATA_POLICY_VERSION,"acknowledge_shared_environment":true,"acknowledge_data_loss":true});
    input
}
async fn current(f: &Fixture, id: &str) -> ManagedMachine {
    lifecycle::managed_machine_for_node(&f.db, id)
        .await
        .unwrap()
        .unwrap()
}
async fn wallet(f: &Fixture) -> BillingWallet {
    f.db.collection(WALLETS)
        .find_one(doc! {"owner_id":&f.owner})
        .await
        .unwrap()
        .unwrap()
}
async fn native_fixture(
    f: &Fixture,
    machine: &ManagedMachine,
    running: bool,
    stopped: bool,
) -> (ManagedMachine, NativeRuntimeObservation) {
    // API fixture facts are deliberately separate from provider signatures.
    // Millisecond canonicalization matches Kubernetes/ManagedMachine storage.
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let observation = NativeRuntimeObservation {
        sandbox_uid: machine.sandbox_uid.clone().unwrap(),
        pod_uid: machine.pod_uid.clone(),
        runtime_generation: machine.runtime_generation,
        credential_epoch: machine.credential_epoch,
        writer_fence: machine.writer_fence.clone(),
        operation_id: machine.active_operation_id.clone().unwrap(),
        sandbox_generation: machine
            .native_observation
            .as_ref()
            .map_or(1, |o| o.sandbox_generation + 1),
        sandbox_resource_version: Uuid::new_v4().to_string(),
        observed_at: DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap(),
        running,
        suspended: stopped,
        stop_confirmed: stopped,
        run_not_after: machine.funded_until.or(machine.native_run_not_after),
        evidence_ref: format!("kubernetes-fixture:{}", Uuid::new_v4()),
    };
    let machine = remote_machine_native::record(&f.db, machine, &observation, Document::new())
        .await
        .unwrap();
    (machine, observation)
}

async fn start_ready(f: &Fixture) -> ManagedMachine {
    start_ready_with_age(f, 0).await
}
async fn start_ready_with_age(f: &Fixture, age_seconds: i64) -> ManagedMachine {
    let mut input = input(f).await;
    input.now = Some(Utc::now() - Duration::seconds(age_seconds));
    let created = lifecycle::create_machine_with_billing(&f.db, input, f.billing.clone())
        .await
        .unwrap()
        .machine;
    let advanced = lifecycle::advance_runtime_generation(
        &f.db,
        &created.node_id,
        created.revision,
        0,
        None,
        None,
    )
    .await
    .unwrap();
    let bound = lifecycle::apply_runtime_observation(
        &f.db,
        lifecycle::RuntimeObservationInput {
            machine_id: advanced.node_id.clone(),
            sandbox_uid: Uuid::new_v4().to_string(),
            pod_uid: Some(Uuid::new_v4().to_string()),
            pvc_name: Some("workspace".into()),
            pvc_uid: Some(Uuid::new_v4().to_string()),
            ready: true,
            physically_stopped: false,
            source: "test-api".into(),
            now: None,
        },
    )
    .await
    .unwrap();
    let bound = install_test_coverage(
        &f.db,
        &bound,
        bound
            .funded_until
            .unwrap_or(Utc::now() + Duration::minutes(5)),
    )
    .await
    .unwrap();
    remote_machine_native::activate(&f.db, &bound)
        .await
        .unwrap();
    let delivery_id = Uuid::new_v4().to_string();
    lifecycle::accept_bootstrap_delivery(
        &f.db,
        &bound.node_id,
        bound.runtime_generation,
        bound.credential_epoch,
        &delivery_id,
        "sealed-fixture",
    )
    .await
    .unwrap();
    let managed_binding = nyxid_machine::managed::Binding {
        node_id: bound.node_id.clone(),
        sandbox_uid: bound.sandbox_uid.clone().unwrap(),
        pod_uid: bound.pod_uid.clone().unwrap(),
        runtime_generation: bound.runtime_generation as u64,
        credential_epoch: bound.credential_epoch as u64,
        writer_fence: bound.writer_fence.clone(),
        operation_id: bound.active_operation_id.clone().unwrap(),
        delivery_id,
    };
    let connection = Uuid::new_v4().to_string();
    let now = Utc::now();
    let profile = nyxid_machine::MachineProfile {
        version: nyxid_machine::PROTOCOL_VERSION,
        runtime_id: Uuid::new_v4().to_string(),
        shell: true,
        files: true,
        browser: Some(false),
        roots: vec!["/workspace".into()],
        authority_versions: vec![nyxid_machine::authority::VERSION],
        commands_isolated: Some(true),
        os: "linux".into(),
        arch: "aarch64".into(),
        ..Default::default()
    };
    let lease = doc! {"instance_name":"local-runtime-test","generation_id":Uuid::new_v4().to_string(),"connection_id":&connection,"internal_base_url":"http://127.0.0.1","claimed_at":bson::DateTime::from_chrono(now),"renewed_at":bson::DateTime::from_chrono(now),"expires_at":bson::DateTime::from_chrono(now+Duration::minutes(5)),"capabilities_resolved":true};
    f.db.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(doc!{"_id":&bound.node_id},doc!{"$set":{"is_active":true,"status":"online","connection_owner":lease,"machine":bson::to_bson(&profile).unwrap()}}).await.unwrap();
    super::super::remote_machine_manager::mark_runtime_online_with_billing(
        &f.db,
        &bound.node_id,
        &managed_binding,
        &connection,
        f.billing.clone(),
    )
    .await
    .unwrap();
    current(f, &bound.node_id).await
}
async fn action(
    f: &Fixture,
    m: &ManagedMachine,
    action: LifecycleAction,
    mut body: serde_json::Value,
) -> AppResult<lifecycle::LifecycleResult> {
    body["policy_version"] = json!(crate::models::remote_machine::POLICY_VERSION);
    body["data_policy_version"] = json!(crate::models::remote_machine::DATA_POLICY_VERSION);
    body["acknowledge_shared_environment"] = json!(true);
    body["acknowledge_data_loss"] = json!(true);
    lifecycle::request_action_with_billing(
        &f.db,
        LifecycleRequest {
            owner_id: f.owner.clone(),
            machine_id: m.node_id.clone(),
            action,
            expected_revision: Some(m.revision),
            idempotency_key: Uuid::new_v4().to_string(),
            request_body: body,
            now: None,
        },
        f.billing.clone(),
    )
    .await
}

#[tokio::test]
async fn paid_create_replay_keeps_one_funding_operation_during_provider_outage() {
    let f = fixture("runtime_lifecycle_replay").await;
    let mut input = input(&f).await;
    input.request_body["name"] = json!("paid-browser-machine");
    input.request_body["capabilities"] = json!(["files", "browser", "computer"]);
    let created = lifecycle::create_machine_with_billing(&f.db, input.clone(), f.billing.clone())
        .await
        .unwrap();
    assert!(!created.machine.free_slot);
    assert_eq!(created.machine.name, "paid-browser-machine");
    assert_eq!(
        created.machine.capabilities,
        ["browser", "computer", "files"]
    );
    assert!(created.machine.funded_until.is_some());
    let held = wallet(&f).await.reserved_credits;
    assert!(held > Credits::ZERO);
    f.lago.available.store(false, Ordering::SeqCst);
    let replay = lifecycle::create_machine_with_billing(&f.db, input, f.billing.clone())
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.machine.node_id, created.machine.node_id);
    assert_eq!(replay.machine.name, created.machine.name);
    assert_eq!(replay.machine.capabilities, created.machine.capabilities);
    assert_eq!(wallet(&f).await.reserved_credits, held);
    assert_eq!(
        f.db.collection::<Document>(RUN_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.db.collection::<Document>(OPERATIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn failed_funding_leaves_no_machine_node_operation_or_quote_acceptance() {
    let f = fixture("runtime_lifecycle_atomic_failure").await;
    let input = input(&f).await;
    f.db.collection::<Document>(WALLETS)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$set":{"balance_credits":Credits::ZERO}},
        )
        .await
        .unwrap();
    assert!(
        lifecycle::create_machine_with_billing(&f.db, input, f.billing.clone())
            .await
            .is_err()
    );
    for collection in [
        MACHINES,
        OPERATIONS,
        RUN_COLLECTION,
        crate::models::node::COLLECTION_NAME,
        crate::models::remote_machine_outbox::COLLECTION_NAME,
    ] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "{collection}"
        );
    }
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn concurrent_quote_acceptance_has_one_lifecycle_and_funding_winner() {
    let f = fixture("runtime_lifecycle_quote_race").await;
    let a = input(&f).await;
    let mut b = a.clone();
    b.idempotency_key = Uuid::new_v4().to_string();
    let (a, b) = tokio::join!(
        lifecycle::create_machine_with_billing(&f.db, a, f.billing.clone()),
        lifecycle::create_machine_with_billing(&f.db, b, f.billing.clone())
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(
        f.db.collection::<Document>(MACHINES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.db.collection::<Document>(RUN_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn deleted_owner_retains_paid_liability_until_native_stop_and_local_settlement() {
    let f = fixture("runtime_deleted_owner").await;
    let machine = start_ready(&f).await;
    let held = wallet(&f).await.reserved_credits;
    assert!(held > Credits::ZERO);
    crate::test_utils::accept_and_process_owner_erasure(
        &f.db,
        &f.owner,
        crate::services::admin_user_service::ACCOUNT_DELETION_POLICY_VERSION,
    )
    .await
    .unwrap();
    assert!(
        f.db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":&f.owner})
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        crate::services::erasure_admission::begin_write(&f.db, &f.owner).await,
        Err(AppError::Conflict(_))
    ));
    maintain_local_safety(&f.db, f.billing.clone())
        .await
        .unwrap();
    // Source absence is not Machine completion. The durable job must continue
    // to require native evidence and keep the already reserved paid liability.
    let mut state = crate::test_utils::test_app_state(f.db.clone());
    state.billing = f.billing.clone();
    f.db.collection::<Document>(crate::models::personal_erasure::COLLECTION_NAME)
        .update_one(
            doc! {"user_id":&f.owner},
            vec![doc! {"$set":{"retry_at":"$$NOW"}}],
        )
        .await
        .unwrap();
    crate::services::personal_erasure_service::process_one(&state)
        .await
        .unwrap();
    let pending_erasure = crate::services::personal_erasure_service::status(&f.db, &f.owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        pending_erasure.status,
        crate::models::personal_erasure::ErasureStatus::Pending
    );
    assert!(pending_erasure.completed_at.is_none());
    let machine_checkpoint = pending_erasure
        .domains
        .iter()
        .find(|domain| domain.domain == "machine")
        .unwrap();
    assert_eq!(
        machine_checkpoint.status,
        crate::models::personal_erasure::DomainStatus::Pending
    );
    assert_eq!(
        machine_checkpoint.reason.as_deref(),
        Some("machine_lifecycle_pending")
    );
    let deleting = current(&f, &machine.node_id).await;
    assert_eq!(deleting.resource_state, ResourceState::Deleting);
    assert_eq!(wallet(&f).await.reserved_credits, held);
    assert!(require_accounting_final(&f.db, &deleting).await.is_err());
    let pending =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&deleting.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(pending.state, MachineRuntimeRunState::StopPending);
    let (deleting, stopped) = native_fixture(&f, &deleting, false, true).await;
    assert!(
        remote_machine_native::finalize_stop(&f.db, &deleting, Some("kubernetes-cleanup-fixture"))
            .await
            .is_err(),
        "native cleanup does not replace unsettled local responsibility"
    );
    native_observe_and_settle(&f.db, f.billing.clone(), &deleting, &stopped)
        .await
        .unwrap();
    require_accounting_final(&f.db, &deleting).await.unwrap();
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    assert!(
        remote_machine_native::finalize_stop(&f.db, &deleting, None)
            .await
            .is_err()
    );
    assert_eq!(
        current(&f, &machine.node_id).await.resource_state,
        ResourceState::Deleting
    );
    let deleted =
        remote_machine_native::finalize_stop(&f.db, &deleting, Some("kubernetes-cleanup-fixture"))
            .await
            .unwrap();
    assert_eq!(deleted.resource_state, ResourceState::Deleted);
    f.db.collection::<Document>(crate::models::personal_erasure::COLLECTION_NAME)
        .update_one(
            doc! {"user_id":&f.owner},
            vec![doc! {"$set":{"retry_at":"$$NOW"}}],
        )
        .await
        .unwrap();
    crate::services::personal_erasure_service::process_one(&state)
        .await
        .unwrap();
    let settled_erasure = crate::services::personal_erasure_service::status(&f.db, &f.owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        settled_erasure
            .domains
            .iter()
            .find(|domain| domain.domain == "machine")
            .unwrap()
            .status,
        crate::models::personal_erasure::DomainStatus::Complete
    );
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn pause_fences_execution_retains_liability_until_stop_and_manual_resume() {
    let f = fixture("runtime_lifecycle_pause").await;
    let machine = start_ready(&f).await;
    let node =
        f.db.collection::<Node>(crate::models::node::COLLECTION_NAME)
            .find_one(doc! {"_id":&machine.node_id})
            .await
            .unwrap()
            .unwrap();
    assert!(
        require_execution_cutoff(&f.db, &node)
            .await
            .unwrap()
            .is_some()
    );
    let paused = action(
        &f,
        &machine,
        LifecycleAction::Pause,
        json!({"action":"pause"}),
    )
    .await
    .unwrap()
    .machine;
    assert!(require_execution_cutoff(&f.db, &node).await.is_err());
    assert!(require_accounting_final(&f.db, &paused).await.is_err());
    let run =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&paused.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(run.state, MachineRuntimeRunState::StopPending);
    let (paused, stopped) = native_fixture(&f, &paused, false, true).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &paused, &stopped)
        .await
        .unwrap();
    let final_paused = remote_machine_native::finalize_stop(&f.db, &paused, None)
        .await
        .unwrap();
    assert_eq!(final_paused.resource_state, ResourceState::Paused);
    require_accounting_final(&f.db, &final_paused)
        .await
        .unwrap();
    f.db.collection::<Document>(WALLETS)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$set":{"balance_credits":Credits::from_whole(100)}},
        )
        .await
        .unwrap();
    renew(&f.db, f.billing.clone(), &final_paused)
        .await
        .unwrap();
    assert_eq!(
        current(&f, &machine.node_id).await.resource_state,
        ResourceState::Paused
    );
    assert!(
        action(
            &f,
            &final_paused,
            LifecycleAction::Resume,
            json!({"action":"resume"})
        )
        .await
        .is_err()
    );
    let q = f
        .billing
        .quote_machine_runtime(
            &f.owner,
            terms(&machine.profile, &machine.policy_version).unwrap(),
            &f.rate.id,
        )
        .await
        .unwrap();
    let resumed = action(
        &f,
        &final_paused,
        LifecycleAction::Resume,
        json!({"quote_id":q.id,"accept_account_balance":true}),
    )
    .await
    .unwrap()
    .machine;
    assert_ne!(resumed.runtime_run_id, machine.runtime_run_id);
    assert_eq!(resumed.node_id, machine.node_id);
    assert_eq!(resumed.profile, machine.profile);
    assert_eq!(resumed.pvc_uid, machine.pvc_uid);
    assert_eq!(resumed.pvc_name, machine.pvc_name);
    assert!(resumed.pvc_uid.is_some());
    assert!(resumed.funded_until.unwrap() > Utc::now());
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn paid_generation_cannot_advance_without_replacement_settlement() {
    let f = fixture("runtime_lifecycle_replace").await;
    let machine = start_ready(&f).await;
    let (machine, stopped) = native_fixture(&f, &machine, false, true).await;
    assert!(
        lifecycle::advance_runtime_generation(
            &f.db,
            &machine.node_id,
            machine.revision,
            machine.runtime_generation,
            Some("new-pod"),
            None
        )
        .await
        .is_err()
    );
    assert!(
        native_replace_stopped(
            &f.db,
            f.billing.clone(),
            &machine,
            &stopped,
            Some("new-pod")
        )
        .await
        .is_err(),
        "unexpected controller replacement has no native owner authorization"
    );
    let rekey = action(
        &f,
        &machine,
        LifecycleAction::Rekey,
        json!({"action":"rekey"}),
    )
    .await
    .unwrap()
    .machine;
    let (machine, stopped) = native_fixture(&f, &rekey, false, true).await;
    let new = native_replace_stopped(
        &f.db,
        f.billing.clone(),
        &machine,
        &stopped,
        Some("new-pod"),
    )
    .await
    .unwrap();
    assert_eq!(new.runtime_run_id, machine.runtime_run_id);
    assert_eq!(new.runtime_generation, machine.runtime_generation + 1);
    assert_eq!(new.credential_epoch, machine.credential_epoch + 1);
    let run =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&new.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(run.state, MachineRuntimeRunState::Accepted);
    assert_eq!(run.rate.id, f.rate.id);
    assert!(!new.writer_admitted);
    assert!(new.funded_until.unwrap() > Utc::now());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_stop_without_terminal_time_never_bills_the_observation_tail() {
    let f = fixture("runtime_stop_observation_bound").await;
    let machine = start_ready(&f).await;
    let paused = action(
        &f,
        &machine,
        LifecycleAction::Pause,
        json!({"action":"pause"}),
    )
    .await
    .unwrap()
    .machine;
    let before =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&paused.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    let (paused, proof) = native_fixture(&f, &paused, false, true).await;
    let observed = proof.observed_at;
    native_observe_and_settle(&f.db, f.billing.clone(), &paused, &proof)
        .await
        .unwrap();
    let after =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&paused.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(after.state, MachineRuntimeRunState::Closed);
    assert_eq!(after.proven_until_ns, before.proven_until_ns);
    assert_eq!(after.cumulative_seconds, 0);
    assert!(after.stop_is_upper_bound);
    assert_eq!(after.platform_residual_ns, 0);
    assert_eq!(
        after.platform_residual_upper_bound_ns,
        observed.timestamp_nanos_opt().unwrap() - before.ready_ns.unwrap()
    );
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn writer_admission_alone_does_not_advance_billable_running_evidence() {
    let f = fixture("runtime_admission_not_running").await;
    let machine = start_ready(&f).await;
    let runs = f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION);
    let before = runs
        .find_one(doc! {"_id":&machine.runtime_run_id})
        .await
        .unwrap()
        .unwrap();
    let (machine, unknown) = native_fixture(&f, &machine, false, false).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &machine, &unknown)
        .await
        .unwrap();
    assert_eq!(
        runs.find_one(doc! {"_id":&machine.runtime_run_id})
            .await
            .unwrap()
            .unwrap()
            .proven_until_ns,
        before.proven_until_ns
    );
    let (machine, running) = native_fixture(&f, &machine, true, false).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &machine, &running)
        .await
        .unwrap();
    assert_eq!(
        runs.find_one(doc! {"_id":&machine.runtime_run_id})
            .await
            .unwrap()
            .unwrap()
            .proven_until_ns,
        running.observed_at.timestamp_nanos_opt()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn operation_display_expiry_keeps_idempotency_and_free_slot() {
    let f = fixture("runtime_operation_display").await;
    let mut input = CreateMachineInput::fixed(&f.owner, true, "test-image");
    input.idempotency_key = Uuid::new_v4().to_string();
    let created = lifecycle::create_machine(&f.db, input.clone())
        .await
        .unwrap();
    f.db.collection::<Document>(OPERATIONS).update_one(doc!{"_id":&created.operation.id},doc!{"$set":{"state":"succeeded","terminal_at":bson::DateTime::from_chrono(Utc::now()-Duration::days(91))}}).await.unwrap();
    assert!(
        super::super::remote_machine_operation_service::get_display_operation(
            &f.db,
            &f.owner,
            &created.operation.id
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        super::super::remote_machine_operation_service::get_operation(
            &f.db,
            &f.owner,
            &created.operation.id
        )
        .await
        .unwrap()
        .is_some()
    );
    assert!(
        lifecycle::create_machine(&f.db, input)
            .await
            .unwrap()
            .replayed
    );
    f.db.drop().await.unwrap();
}
#[test]
fn retry_backoff_is_bounded_without_abandoning_cleanup() {
    let retry = super::super::remote_machine_operation_service::retry_backoff_seconds;
    assert_eq!(retry(1), 5);
    assert_eq!(retry(2), 10);
    assert_eq!(retry(4), 40);
    assert_eq!(retry(1000), 300);
}

#[tokio::test]
async fn funded_never_submitted_delete_closes_zero_usage_before_cleanup() {
    let f = fixture("runtime_unsubmitted_delete").await;
    let created = lifecycle::create_machine_with_billing(&f.db, input(&f).await, f.billing.clone())
        .await
        .unwrap()
        .machine;
    assert_eq!(created.runtime_generation, 0);
    let deleted = action(
        &f,
        &created,
        LifecycleAction::Delete,
        json!({"action":"delete"}),
    )
    .await
    .unwrap()
    .machine;
    assert!(
        lifecycle::finalize_unallocated_delete(&f.db, &created.node_id)
            .await
            .is_err()
    );
    finalize_never_submitted(&f.db, Some(f.billing.clone()), &deleted)
        .await
        .unwrap();
    let final_state = lifecycle::finalize_unallocated_delete(&f.db, &created.node_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(final_state.resource_state, ResourceState::Deleted);
    let run =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&created.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(run.state, MachineRuntimeRunState::Closed);
    assert_eq!(run.cumulative_seconds, 0);
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    f.db.drop().await.unwrap();
}

async fn unresolved_legacy_submission_cannot_settle_as_never_submitted(allocate: bool) {
    let f = fixture(if allocate {
        "runtime_unknown_submission_allocated"
    } else {
        "runtime_unknown_submission_unallocated"
    })
    .await;
    let created = lifecycle::create_machine_with_billing(&f.db, input(&f).await, f.billing.clone())
        .await
        .unwrap()
        .machine;
    let machine = if allocate {
        lifecycle::advance_runtime_generation(
            &f.db,
            &created.node_id,
            created.revision,
            0,
            None,
            None,
        )
        .await
        .unwrap()
    } else {
        created
    };
    // Legacy workers may have dispatched before recording a submission
    // marker. Lease expiry does not close that original request.
    let changed =
        f.db.collection::<Document>(crate::models::remote_machine_outbox::COLLECTION_NAME)
            .update_one(
                doc! {"machine_id":&machine.node_id,"kind":"create_sandbox"},
                doc! {"$set":{"state":"claimed","attempts":1_i64,
                "claimed_until":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(5)),
                "submission":bson::Bson::Null}},
            )
            .await
            .unwrap();
    assert_eq!(changed.matched_count, 1);
    let deleting = action(
        &f,
        &machine,
        LifecycleAction::Delete,
        json!({"action":"delete"}),
    )
    .await
    .unwrap()
    .machine;
    assert!(deleting.runtime_submission_started_at.is_none());
    assert!(deleting.pod_uid.is_none());
    let runs = f.db.collection::<Document>(RUN_COLLECTION);
    let intervals = f.db.collection::<Document>(INTERVAL_COLLECTION);
    let before_run = runs
        .find_one(doc! {"_id":&deleting.runtime_run_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        before_run.get_i64("runtime_generation").unwrap(),
        deleting.runtime_generation + if allocate { 0 } else { 1 }
    );
    let before_interval = intervals
        .find_one(doc! {"run_id":&deleting.runtime_run_id})
        .await
        .unwrap()
        .unwrap();
    let before_wallet = bson::to_document(&wallet(&f).await).unwrap();
    let ledger = f.db.collection::<Document>("billing_ledger");
    let before_ledger = ledger.count_documents(doc! {}).await.unwrap();
    let outcome = finalize_never_submitted(&f.db, Some(f.billing.clone()), &deleting).await;
    assert_eq!(
        runs.find_one(doc! {"_id":&deleting.runtime_run_id})
            .await
            .unwrap()
            .unwrap(),
        before_run,
        "an unresolved original create must not gain a fabricated zero stop"
    );
    assert_eq!(
        intervals
            .find_one(doc! {"run_id":&deleting.runtime_run_id})
            .await
            .unwrap()
            .unwrap(),
        before_interval
    );
    assert_eq!(bson::to_document(&wallet(&f).await).unwrap(), before_wallet);
    assert_eq!(
        ledger.count_documents(doc! {}).await.unwrap(),
        before_ledger
    );
    assert!(outcome.is_err() || !outcome.unwrap());
    let after = current(&f, &deleting.node_id).await;
    assert_eq!(after.resource_state, ResourceState::Deleting);
    assert!(after.last_runtime_evidence.is_none());
    let op =
        f.db.collection::<Document>(OPERATIONS)
            .find_one(doc! {"_id":&after.active_operation_id})
            .await
            .unwrap()
            .unwrap();
    assert!(op.get("terminal_at").is_none_or(|v| v == &bson::Bson::Null));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn paid_unallocated_delete_rejects_unknown_legacy_submission() {
    unresolved_legacy_submission_cannot_settle_as_never_submitted(false).await;
}

#[tokio::test]
async fn paid_allocated_delete_rejects_unknown_legacy_submission() {
    unresolved_legacy_submission_cannot_settle_as_never_submitted(true).await;
}

#[tokio::test]
async fn submission_marker_prevents_false_local_stop_and_deadline_keeps_workspace() {
    let f = fixture("runtime_submission_fence").await;
    let created = lifecycle::create_machine_with_billing(&f.db, input(&f).await, f.billing.clone())
        .await
        .unwrap()
        .machine;
    let advanced = lifecycle::advance_runtime_generation(
        &f.db,
        &created.node_id,
        created.revision,
        0,
        None,
        None,
    )
    .await
    .unwrap();
    mark_submission(&f.db, &advanced).await.unwrap();
    safety_pause(
        &f.db,
        Some(f.billing.clone()),
        &advanced,
        "expansion_deadline_exceeded",
    )
    .await
    .unwrap();
    let stopped = current(&f, &advanced.node_id).await;
    assert!(
        !finalize_never_submitted(&f.db, Some(f.billing.clone()), &stopped)
            .await
            .unwrap()
    );
    assert!(wallet(&f).await.reserved_credits > Credits::ZERO);
    assert_eq!(stopped.resource_state, ResourceState::Stopping);
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn unused_paid_generation_can_pause_then_require_a_fresh_manual_resume_quote() {
    let f = fixture("runtime_unsubmitted_pause").await;
    let created = lifecycle::create_machine_with_billing(&f.db, input(&f).await, f.billing.clone())
        .await
        .unwrap()
        .machine;
    safety_pause(
        &f.db,
        Some(f.billing.clone()),
        &created,
        "runtime_funding_unavailable",
    )
    .await
    .unwrap();
    let stopped = current(&f, &created.node_id).await;
    assert!(
        finalize_never_submitted(&f.db, Some(f.billing.clone()), &stopped)
            .await
            .unwrap()
    );
    let paused = current(&f, &created.node_id).await;
    assert_eq!(paused.runtime_generation, 1);
    assert!(has_never_submitted_stop(&paused));
    let q = f
        .billing
        .quote_machine_runtime(
            &f.owner,
            terms(&paused.profile, &paused.policy_version).unwrap(),
            &f.rate.id,
        )
        .await
        .unwrap();
    let resume = action(
        &f,
        &paused,
        LifecycleAction::Resume,
        json!({"quote_id":q.id,"accept_account_balance":true}),
    )
    .await
    .unwrap()
    .machine;
    let advanced = lifecycle::advance_runtime_generation(
        &f.db,
        &resume.node_id,
        resume.revision,
        resume.runtime_generation,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(advanced.runtime_generation, 2);
    assert_eq!(advanced.runtime_run_id, resume.runtime_run_id);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn persisted_millisecond_cutoff_renews_exactly_once_under_contention() {
    let f = fixture("runtime_lifecycle_renewal").await;
    let machine = start_ready_with_age(&f, window_seconds() - renewal_margin_seconds() + 5).await;
    let cutoff = machine.funded_until.unwrap();
    let before = wallet(&f).await.reserved_credits;
    let (a, b) = tokio::join!(
        renew(&f.db, f.billing.clone(), &machine),
        renew(&f.db, f.billing.clone(), &machine)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let current = current(&f, &machine.node_id).await;
    assert_eq!(
        current.funded_until,
        Some(cutoff + Duration::seconds(window_seconds()))
    );
    let run =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&current.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        run.covered_until_ns,
        current.funded_until.unwrap().timestamp_nanos_opt().unwrap()
    );
    assert!(wallet(&f).await.reserved_credits > before);
    assert_eq!(
        f.db.collection::<Document>(INTERVAL_COLLECTION)
            .count_documents(doc! {"run_id":&current.runtime_run_id})
            .await
            .unwrap(),
        2
    );
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn stopped_replacement_can_be_narrowed_by_owner_delete_and_closed() {
    let f = fixture("runtime_replacement_stop_narrows").await;
    let machine = start_ready(&f).await;
    let machine = action(
        &f,
        &machine,
        LifecycleAction::Rekey,
        json!({"action":"rekey"}),
    )
    .await
    .unwrap()
    .machine;
    let (machine, stopped) = native_fixture(&f, &machine, false, true).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &machine, &stopped)
        .await
        .unwrap();
    let after = current(&f, &machine.node_id).await;
    assert!(!after.writer_admitted);
    let run =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&after.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(run.state, MachineRuntimeRunState::Stopped);
    assert!(run.replacement_requested);
    let deleted = action(
        &f,
        &after,
        LifecycleAction::Delete,
        json!({"action":"delete"}),
    )
    .await
    .unwrap()
    .machine;
    let (deleted, stopped) = native_fixture(&f, &deleted, false, true).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &deleted, &stopped)
        .await
        .unwrap();
    let run =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&deleted.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(run.state, MachineRuntimeRunState::Closed);
    assert!(!run.replacement_requested);
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn transactional_execution_fence_rejects_after_owner_pause() {
    let f = fixture("runtime_exec_pause_fence").await;
    let machine = start_ready(&f).await;
    let node =
        f.db.collection::<Node>(crate::models::node::COLLECTION_NAME)
            .find_one(doc! {"_id":&machine.node_id})
            .await
            .unwrap()
            .unwrap();
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let fence = fence_execution_in_session(&f.db, &node, &mut session)
        .await
        .unwrap();
    assert_eq!(
        fence.runtime_generation,
        Some(machine.runtime_generation as u64)
    );
    assert_eq!(fence.funding_cutoff, machine.funded_until);
    session.commit_transaction().await.unwrap();
    let stopped = action(
        &f,
        &machine,
        LifecycleAction::Pause,
        json!({"action":"pause"}),
    )
    .await
    .unwrap()
    .machine;
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        fence_execution_in_session(&f.db, &node, &mut session)
            .await
            .is_err()
    );
    session.abort_transaction().await.unwrap();
    assert_eq!(stopped.stop_reason.as_deref(), Some("owner_pause"));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn owner_pause_conflicts_with_an_admission_using_an_earlier_snapshot() {
    let f = fixture("runtime_exec_pause_race").await;
    let machine = start_ready(&f).await;
    let node =
        f.db.collection::<Node>(crate::models::node::COLLECTION_NAME)
            .find_one(doc! {"_id":&machine.node_id})
            .await
            .unwrap()
            .unwrap();
    let mut admission = f.db.client().start_session().await.unwrap();
    admission.start_transaction().await.unwrap();
    f.db.collection::<ManagedMachine>(MACHINES)
        .find_one(doc! {"_id":&machine.node_id})
        .session(&mut admission)
        .await
        .unwrap()
        .unwrap();
    action(
        &f,
        &machine,
        LifecycleAction::Pause,
        json!({"action":"pause"}),
    )
    .await
    .unwrap();
    // The old snapshot still reads Ready, but the admission's mandatory write
    // conflicts with the committed owner stop. It cannot mint a live lease.
    assert!(
        fence_execution_in_session(&f.db, &node, &mut admission)
            .await
            .is_err()
    );
    let _ = admission.abort_transaction().await;
    assert_eq!(
        current(&f, &machine.node_id).await.resource_state,
        ResourceState::Stopping
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn independent_funding_pass_stops_expired_runtime_without_releasing_liability() {
    let f = fixture("runtime_funding_maintenance_expiry").await;
    let machine = start_ready(&f).await;
    let reserved = wallet(&f).await.reserved_credits;
    f.db.collection::<Document>(MACHINES).update_one(doc!{"_id":&machine.node_id},
        doc!{"$set":{"funded_until":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}})
        .await.unwrap();
    let server = wiremock::MockServer::start().await;
    let manager = super::super::remote_machine_manager::Manager {
        db: f.db.clone(),
        billing: Some(f.billing.clone()),
        api: nyxid_machine_updater::sandbox::SandboxApi::new(server.uri().parse().unwrap(), None)
            .unwrap(),
        storage_class: Some("local-fixture-csi".into()),
        namespace: "nyxid-machine".into(),
        bootstrap_base_url: "https://nyxid.test".into(),
    };
    manager.funding_tick().await.unwrap();
    let stopped = current(&f, &machine.node_id).await;
    assert_eq!(stopped.resource_state, ResourceState::Stopping);
    assert_eq!(
        stopped.stop_reason.as_deref(),
        Some("runtime_funding_expired")
    );
    f.db.collection::<Document>(OPERATIONS)
        .update_one(
            doc! {"_id":&stopped.active_operation_id},
            doc! {"$set":{"error_code":"runtime_evidence_pending"}},
        )
        .await
        .unwrap();
    super::super::remote_machine_events::sweep(&f.db, Utc::now())
        .await
        .unwrap();
    let notices =
        super::super::remote_machine_events::list(&f.db, &f.owner, &stopped.node_id, None)
            .await
            .unwrap();
    assert!(notices.iter().any(|event| event.operation_id
        == *stopped.active_operation_id.as_ref().unwrap()
        && event.message.contains("resume manually")));
    assert!(!stopped.writer_admitted);
    assert_eq!(wallet(&f).await.reserved_credits, reserved);
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&stopped.runtime_run_id})
            .await
            .unwrap()
            .unwrap()
            .state,
        MachineRuntimeRunState::StopPending
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn missing_platform_does_not_strand_an_expired_never_submitted_contract() {
    let f = fixture("runtime_no_platform_deadline").await;
    let created = lifecycle::create_machine_with_billing(&f.db, input(&f).await, f.billing.clone())
        .await
        .unwrap()
        .machine;
    f.db.collection::<Document>(OPERATIONS).update_one(doc!{"_id":&created.active_operation_id},
        doc!{"$set":{"deadline_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}})
        .await.unwrap();
    maintain_local_safety(&f.db, f.billing.clone())
        .await
        .unwrap();
    let paused = current(&f, &created.node_id).await;
    assert_eq!(paused.resource_state, ResourceState::Paused);
    assert_eq!(
        paused.stop_reason.as_deref(),
        Some("expansion_deadline_exceeded")
    );
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    assert_eq!(
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&paused.runtime_run_id})
            .await
            .unwrap()
            .unwrap()
            .state,
        MachineRuntimeRunState::Closed
    );
    let pause =
        f.db.collection::<crate::models::remote_machine_operation::MachineOperation>(OPERATIONS)
            .find_one(doc! {"_id":&paused.active_operation_id})
            .await
            .unwrap()
            .unwrap();
    assert!(pause.terminal_at.is_some());
    assert_eq!(pause.action, LifecycleAction::Pause);
    f.db.drop().await.unwrap();
}

#[path = "remote_machine_runtime_tests/journeys.rs"]
mod journeys;

#[path = "remote_machine_runtime_tests/native.rs"]
mod native;

#[tokio::test]
async fn profile_binding_paid_tampering_has_no_hold_machine_or_outbox() {
    let f = fixture("profile_binding_paid_tampering").await;
    let accepted = input(&f).await;
    let before = wallet(&f).await.reserved_credits;
    for (field, value) in [
        ("profile_version", json!("test-foreign-profile")),
        ("profile_version", serde_json::Value::Null),
        ("cpu_millis", json!(4000)),
        ("memory_mib", json!(8192)),
        ("workspace_gib", json!(50)),
    ] {
        let mut altered = accepted.clone();
        altered.request_body[field] = value;
        assert!(
            lifecycle::create_machine_with_billing(&f.db, altered, f.billing.clone())
                .await
                .is_err()
        );
    }
    let mut wrong_spec = accepted.clone();
    wrong_spec.profile.workspace_gib = 50;
    assert!(
        lifecycle::create_machine_with_billing(&f.db, wrong_spec, f.billing.clone())
            .await
            .is_err()
    );
    let mut wrong_image = accepted.clone();
    wrong_image.profile.image_digest = format!("registry.test/node@sha256:{}", "b".repeat(64));
    assert!(
        lifecycle::create_machine_with_billing(&f.db, wrong_image, f.billing.clone())
            .await
            .is_err()
    );
    let quote_id = accepted.request_body["quote_id"].as_str().unwrap();
    f.db.collection::<Document>(crate::models::machine_runtime_billing::QUOTE_COLLECTION)
        .update_one(
            doc! {"_id":quote_id},
            doc! {"$set":{"terms.profile_version":"test-foreign-profile"}},
        )
        .await
        .unwrap();
    assert!(
        lifecycle::create_machine_with_billing(&f.db, accepted.clone(), f.billing.clone())
            .await
            .is_err()
    );
    assert_eq!(wallet(&f).await.reserved_credits, before);
    for collection in [
        MACHINES,
        crate::models::remote_machine_outbox::COLLECTION_NAME,
        RUN_COLLECTION,
        INTERVAL_COLLECTION,
    ] {
        assert_eq!(
            f.db.collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "{collection}"
        );
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn profile_binding_free_and_paid_creation_allocate_identical_controlled_resources() {
    let f = fixture("profile_binding_paid_free").await;
    let paid_input = input(&f).await;
    let paid = lifecycle::create_machine_with_billing(&f.db, paid_input.clone(), f.billing.clone())
        .await
        .unwrap()
        .machine;
    let mut free_input = CreateMachineInput::fixed(&f.owner, true, &paid.profile.image_digest);
    free_input.idempotency_key = Uuid::new_v4().to_string();
    free_input.request_body["profile_version"] = json!("remote-machine");
    let free = lifecycle::create_machine_with_billing(&f.db, free_input, f.billing.clone())
        .await
        .unwrap()
        .machine;
    assert_eq!(paid.profile, free.profile);
    assert_eq!(
        (
            paid.profile.cpu_millis,
            paid.profile.memory_mib,
            paid.profile.workspace_gib,
            paid.profile.ephemeral_gib
        ),
        (2000, 2048, 20, None)
    );
    let run =
        f.db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":&paid.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(run.terms.profile_version, paid.profile.profile_version);
    assert_eq!(run.rate.profile_version, paid.profile.profile_version);
    let before = wallet(&f).await.reserved_credits;
    let mut replay = paid_input;
    replay.request_body["profile_version"] = json!("remote-machine");
    replay.profile.image_digest = format!("changed-default@sha256:{}", "b".repeat(64));
    assert!(
        lifecycle::create_machine_with_billing(&f.db, replay.clone(), f.billing.clone())
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(wallet(&f).await.reserved_credits, before);
    f.db.collection::<Document>(RUN_COLLECTION)
        .update_one(
            doc! {"_id":&paid.runtime_run_id},
            doc! {"$set":{"rate.profile_version":"test-foreign-profile"}},
        )
        .await
        .unwrap();
    assert!(
        lifecycle::create_machine_with_billing(&f.db, replay, f.billing.clone())
            .await
            .is_err()
    );
    assert_eq!(wallet(&f).await.reserved_credits, before);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn mounted_profile_quote_and_preflight_exclude_foreign_rates_and_bind_resources() {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    async fn send(
        app: &Router,
        token: &str,
        method: &str,
        path: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .header("idempotency-key", "profile-http-intent")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }
    let f = fixture("profile_binding_mounted_quote").await;
    personal_remote_flag(&f, true).await;
    let mut foreign = f.rate.clone();
    foreign.id = Uuid::new_v4().to_string();
    foreign.profile_version = "test-only-foreign-profile".into();
    foreign.created_at = Utc::now() + Duration::seconds(5);
    foreign.lago_metric_code = format!(
        "remote_machine_running_seconds_{}",
        foreign.id.replace('-', "")
    );
    f.db.collection::<MachineRuntimeRate>(RATE_COLLECTION)
        .insert_one(&foreign)
        .await
        .unwrap();
    let mut state = crate::test_utils::test_app_state(f.db.clone());
    state.billing = f.billing.clone();
    state.local_machine_platform = Some(Arc::new(crate::test_utils::LocalMachinePlatform {
        profile: lifecycle::fixed_profile(format!("registry.test/node@sha256:{}", "a".repeat(64))),
        readiness: crate::services::remote_machine_manager::PlatformReadiness {
            configuration_ready: true,
            manager_started: true,
            namespace: "nyxid-machine",
            blockers: vec![],
            physical_acceptance: "native_state_only_no_physical_guarantee",
            lifecycle_basis: "agent_sandbox_native_observation",
        },
    }));
    let token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&f.owner).unwrap(),
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let app = private.with_state(state);
    let (status, quoted) = send(
        &app,
        &token,
        "POST",
        "/api/v1/remote-machine-quotes",
        json!({"free_slot":false,"profile_version":"remote-machine"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{quoted}");
    assert_eq!(quoted["rate_version"], f.rate.id);
    assert_eq!(quoted["profile"]["profile_version"], "remote-machine");
    assert_eq!(quoted["profile"]["cpu_millis"], 2000);
    assert_eq!(quoted["profile"]["memory_mib"], 2048);
    assert_eq!(quoted["profile"]["workspace_gib"], 20);
    assert_eq!(
        quoted["billing_preflight"]["profile_version"],
        quoted["profile"]["profile_version"]
    );
    assert_eq!(
        quoted["billing_preflight"]["rate_version"],
        quoted["rate_version"]
    );
    let (status, entitlement) = send(
        &app,
        &token,
        "GET",
        "/api/v1/remote-machine-entitlement?profile_version=remote-machine",
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{entitlement}");
    assert_eq!(
        entitlement["billing_preflight"]["rate_version"],
        quoted["rate_version"]
    );
    assert!(
        quoted["billing_preflight"]["funds"]["estimated_seconds"].is_string(),
        "{quoted}"
    );
    assert_eq!(
        entitlement["billing_preflight"]["funds"]["estimated_seconds"],
        quoted["billing_preflight"]["funds"]["estimated_seconds"]
    );
    let create = json!({"free_slot":false,"profile_version":"test-only-foreign-profile","quote_id":quoted["quote_id"],"accept_account_balance":true,
        "policy_version":"remote-machine-mvp","data_policy_version":"remote-machine-data","acknowledge_shared_environment":true});
    let (status, _) = send(&app, &token, "POST", "/api/v1/remote-machines", create).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    assert_eq!(
        f.db.collection::<Document>(MACHINES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.db.collection::<Document>(RATE_COLLECTION)
        .update_one(doc! {"_id":&f.rate.id}, doc! {"$set":{"retired":true}})
        .await
        .unwrap();
    let (status, error) = send(
        &app,
        &token,
        "POST",
        "/api/v1/remote-machine-quotes",
        json!({"free_slot":false,"profile_version":"remote-machine"}),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{error}");
    assert_eq!(error["error"], "billing_provider_unavailable");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("requested profile")
    );
    let (status, entitlement) = send(
        &app,
        &token,
        "GET",
        "/api/v1/remote-machine-entitlement?profile_version=remote-machine",
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{entitlement}");
    assert_eq!(
        entitlement["billing_preflight"]["rate_version"],
        serde_json::Value::Null
    );
    assert_eq!(entitlement["billing_preflight"]["ready"], false);
    assert_eq!(
        entitlement["billing_preflight"]["reason"],
        "profile_pricing_unavailable"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn profile_binding_pre_additive_full_intent_replays_after_expiry_and_outage() {
    use sha2::{Digest, Sha256};
    let f = fixture("profile_binding_historical_intent").await;
    let mut intent = input(&f).await;
    crate::services::remote_machine_configuration::normalize_request(&mut intent.request_body)
        .unwrap();
    let created = lifecycle::create_machine_with_billing(&f.db, intent.clone(), f.billing.clone())
        .await
        .unwrap();
    let prior_full_body: std::collections::BTreeMap<_, _> =
        intent.request_body.as_object().unwrap().iter().collect();
    assert!(intent.request_body.get("profile_version").is_none());
    let digest = hex::encode(Sha256::digest(
        serde_json::to_vec(&prior_full_body).unwrap(),
    ));
    f.db.collection::<Document>(OPERATIONS)
        .update_one(
            doc! {"_id":&created.operation.id},
            doc! {"$set":{"request_digest":digest}},
        )
        .await
        .unwrap();
    f.db.collection::<Document>(QUOTE_COLLECTION).update_one(doc!{"_id":intent.request_body["quote_id"].as_str().unwrap()},doc!{"$set":{"accept_before":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(1))}}).await.unwrap();
    let before = wallet(&f).await.reserved_credits;
    f.lago.available.store(false, Ordering::SeqCst);
    intent.request_body["profile_version"] = json!("remote-machine");
    intent.profile.image_digest = format!("changed-default@sha256:{}", "b".repeat(64));
    assert!(
        lifecycle::has_recorded_request(
            &f.db,
            &f.owner,
            None,
            LifecycleAction::Create,
            &intent.idempotency_key,
            &intent.request_body
        )
        .await
        .unwrap()
    );
    let replay = lifecycle::create_machine_with_billing(&f.db, intent.clone(), f.billing.clone())
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.machine.profile, created.machine.profile);
    assert_eq!(replay.machine.node_id, created.machine.node_id);
    for (key, value) in [
        ("profile_version", json!("foreign-profile")),
        ("name", json!("changed-machine")),
        ("quote_id", json!(Uuid::new_v4().to_string())),
        ("capabilities", json!(["files"])),
    ] {
        let mut changed = intent.clone();
        changed.request_body[key] = value;
        assert!(
            lifecycle::create_machine_with_billing(&f.db, changed, f.billing.clone())
                .await
                .is_err()
        );
    }
    assert_eq!(wallet(&f).await.reserved_credits, before);
    assert_eq!(
        f.db.collection::<Document>(MACHINES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.db.collection::<Document>(RUN_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}

// Real lifecycle, exact billing, Mongo transactions and manager funding tick;
// only external Lago and native Kubernetes observations are local doubles.
#[tokio::test]
async fn remote_flag_off_keeps_established_renewal_metering_settlement_and_delete() {
    let f = fixture("remote_flag_off_paid_safety").await;
    personal_remote_flag(&f, true).await;
    let machine = start_ready_with_age(&f, window_seconds() - renewal_margin_seconds() + 5).await;
    let before = bson::to_document(&current(&f, &machine.node_id).await).unwrap();
    personal_remote_flag(&f, false).await;
    assert_eq!(
        bson::to_document(&current(&f, &machine.node_id).await).unwrap(),
        before,
        "changing only the personal rollout override cannot stop or rewrite a runtime"
    );
    let held = wallet(&f).await.reserved_credits;
    renew(&f.db, f.billing.clone(), &machine).await.unwrap();
    let renewed = current(&f, &machine.node_id).await;
    assert_eq!(renewed.resource_state, ResourceState::Ready);
    assert!(renewed.funded_until > machine.funded_until);
    assert!(wallet(&f).await.reserved_credits > held);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (observed, running) = native_fixture(&f, &renewed, true, false).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &observed, &running)
        .await
        .unwrap();
    let run: MachineRuntimeRun =
        f.db.collection(RUN_COLLECTION)
            .find_one(doc! {"_id":&observed.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        run.proven_until_ns,
        running.observed_at.timestamp_nanos_opt()
    );
    let paused = action(
        &f,
        &current(&f, &machine.node_id).await,
        LifecycleAction::Pause,
        json!({"action":"pause"}),
    )
    .await
    .unwrap()
    .machine;
    let (paused, stopped) = native_fixture(&f, &paused, false, true).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &paused, &stopped)
        .await
        .unwrap();
    let paused = remote_machine_native::finalize_stop(&f.db, &paused, None)
        .await
        .unwrap();
    assert_eq!(paused.resource_state, ResourceState::Paused);
    require_accounting_final(&f.db, &paused).await.unwrap();
    let run: MachineRuntimeRun =
        f.db.collection(RUN_COLLECTION)
            .find_one(doc! {"_id":&paused.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(run.state, MachineRuntimeRunState::Closed);
    assert!(run.cumulative_seconds >= 1);
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    assert_eq!(
        wallet(&f).await.pending_lago_debits,
        f.rate
            .credits_per_second
            .checked_mul(run.cumulative_seconds)
            .unwrap(),
        "settled usage accrues an exact debit before provider balance refresh"
    );
    let deleted = action(
        &f,
        &paused,
        LifecycleAction::Delete,
        json!({"action":"delete"}),
    )
    .await
    .unwrap()
    .machine;
    let (deleted, _) = native_fixture(&f, &deleted, false, true).await;
    let deleted =
        remote_machine_native::finalize_stop(&f.db, &deleted, Some("task-owned-cleanup-double"))
            .await
            .unwrap();
    assert_eq!(deleted.resource_state, ResourceState::Deleted);
    assert!(
        lifecycle::get_machine(&f.db, &Uuid::new_v4().to_string(), &deleted.node_id)
            .await
            .unwrap()
            .is_none()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn remote_flag_off_manager_still_protects_insufficient_funds() {
    let f = fixture("remote_flag_off_funding_protection").await;
    personal_remote_flag(&f, true).await;
    let machine = start_ready_with_age(&f, window_seconds() - renewal_margin_seconds() + 5).await;
    personal_remote_flag(&f, false).await;
    let held = wallet(&f).await.reserved_credits;
    // Disposable fixture exhausts spendable funds without releasing existing holds.
    f.db.collection::<Document>(WALLETS)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$set":{"balance_credits":held}},
        )
        .await
        .unwrap();
    let api = wiremock::MockServer::start().await;
    let manager = crate::services::remote_machine_manager::Manager {
        db: f.db.clone(),
        billing: Some(f.billing.clone()),
        api: nyxid_machine_updater::sandbox::SandboxApi::new(api.uri().parse().unwrap(), None)
            .unwrap(),
        storage_class: Some("local-fixture-csi".into()),
        namespace: "nyxid-machine".into(),
        bootstrap_base_url: "https://nyxid.test".into(),
    };
    manager.funding_tick().await.unwrap();
    let stopped = current(&f, &machine.node_id).await;
    assert_eq!(stopped.resource_state, ResourceState::Stopping);
    assert_eq!(
        stopped.stop_reason.as_deref(),
        Some("runtime_funding_unavailable")
    );
    assert_eq!(stopped.funded_until, machine.funded_until);
    assert_eq!(wallet(&f).await.reserved_credits, held);
    assert!(!stopped.writer_admitted);
    let (stopped, observation) = native_fixture(&f, &stopped, false, true).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &stopped, &observation)
        .await
        .unwrap();
    let paused = remote_machine_native::finalize_stop(&f.db, &stopped, None)
        .await
        .unwrap();
    assert_eq!(paused.resource_state, ResourceState::Paused);
    require_accounting_final(&f.db, &paused).await.unwrap();
    assert_eq!(wallet(&f).await.reserved_credits, Credits::ZERO);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn persisted_control_plane_delete_stop_settles_charged_quarantine_once_and_preserves_upper_bound()
 {
    use crate::models::remote_machine::{
        DeleteRecoveryProgress, DeleteStopEvidence, DeleteStopKind, DeleteStorageDisposition,
    };
    let f = fixture("delete_recovery_charged_quarantine").await;
    let ready = start_ready(&f).await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (observed, running) = native_fixture(&f, &ready, true, false).await;
    native_observe_and_settle(&f.db, f.billing.clone(), &observed, &running)
        .await
        .unwrap();
    let deleting = action(
        &f,
        &current(&f, &ready.node_id).await,
        LifecycleAction::Delete,
        json!({"action":"delete"}),
    )
    .await
    .unwrap()
    .machine;
    f.db.collection::<Document>(MACHINES).update_one(doc!{"_id":&deleting.node_id,"revision":deleting.revision},
        doc!{"$set":{"native_uncertainty":"predecessor_node_unhealthy_or_stale","native_node_uid":"original-node-uid"},"$inc":{"revision":1_i64}}).await.unwrap();
    let deleting = current(&f, &ready.node_id).await;
    let stop_at = DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap();
    let progress = DeleteRecoveryProgress {
        evidence_version: super::super::remote_machine_delete_recovery::EVIDENCE_VERSION,
        evidence_id: Uuid::new_v4().to_string(),
        operation_id: deleting.active_operation_id.clone().unwrap(),
        owner_id: deleting.owner_id.clone(),
        runtime_generation: deleting.runtime_generation,
        credential_epoch: deleting.credential_epoch,
        writer_fence: deleting.writer_fence.clone(),
        deletion_basis_digest: super::super::remote_machine_delete_recovery::basis_digest(
            &deleting,
        )
        .unwrap(),
        evaluated_revision: deleting.revision,
        original_uncertainty: deleting.native_uncertainty.clone(),
        sandbox_uid: deleting.sandbox_uid.clone(),
        pod_uid: deleting.pod_uid.clone(),
        node_uid: deleting.native_node_uid.clone(),
        pvc_uid: deleting.pvc_uid.clone(),
        provisioning_request_id: deleting.pvc_uid.as_ref().map(|uid| format!("pvc-{uid}")),
        provisioning_terminal: None,
        volume: None,
        stop: Some(DeleteStopEvidence {
            kind: DeleteStopKind::ControlPlaneSuspended,
            source_operation_id: deleting.active_operation_id.clone(),
            source: "sandbox-kubernetes-control-plane".into(),
            evidence_ref: Uuid::new_v4().to_string(),
            observed_at: stop_at,
            sandbox_generation: 3,
            sandbox_resource_version: "original-rv".into(),
        }),
        storage: DeleteStorageDisposition::ProvisioningUnknown,
        creation_terminal: false,
        resources_complete: false,
        blocker: None,
        observed_at: stop_at,
    };
    f.db.collection::<Document>(MACHINES).update_one(doc!{"_id":&deleting.node_id,"revision":deleting.revision},
        doc!{"$set":{"delete_recovery":bson::to_bson(&progress).unwrap()},"$inc":{"revision":1_i64}}).await.unwrap();
    let deleting = current(&f, &ready.node_id).await;
    assert!(require_accounting_final(&f.db, &deleting).await.is_err());
    assert!(!remote_machine_native::predecessor_stopped(&deleting));
    assert!(!remote_machine_native::current_running(
        &deleting,
        &running,
        Utc::now()
    ));
    assert!(
        native_observe_and_settle(&f.db, f.billing.clone(), &deleting, &running)
            .await
            .is_err()
    );
    // Legacy external-fact rows deserialize, but cannot be consumed without
    // the production control-plane producer refreshing their authority.
    let mut legacy = deleting.clone();
    legacy.delete_recovery.as_mut().unwrap().evidence_version = 0;
    legacy
        .delete_recovery
        .as_mut()
        .unwrap()
        .stop
        .as_mut()
        .unwrap()
        .kind = DeleteStopKind::TrustedRuntime;
    assert!(
        settle_persisted_delete(&f.db, f.billing.clone(), &legacy)
            .await
            .is_err()
    );
    assert!(require_accounting_final(&f.db, &deleting).await.is_err());
    settle_persisted_delete(&f.db, f.billing.clone(), &deleting)
        .await
        .unwrap();
    require_accounting_final(&f.db, &deleting).await.unwrap();
    let first: MachineRuntimeRun =
        f.db.collection(RUN_COLLECTION)
            .find_one(doc! {"_id":&deleting.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(first.state, MachineRuntimeRunState::Closed);
    assert_eq!(first.stop_ns, stop_at.timestamp_nanos_opt());
    assert!(first.stop_is_upper_bound);
    assert!(first.cumulative_seconds >= 1);
    let wallet_first = bson::to_document(&wallet(&f).await).unwrap();
    let ledger_first =
        f.db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap();
    let usage_first =
        f.db.collection::<Document>("usage_records")
            .count_documents(doc! {})
            .await
            .unwrap();
    settle_persisted_delete(&f.db, f.billing.clone(), &deleting)
        .await
        .unwrap();
    let second: MachineRuntimeRun =
        f.db.collection(RUN_COLLECTION)
            .find_one(doc! {"_id":&deleting.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(second.stop_ns, first.stop_ns);
    assert_eq!(second.stop_evidence_id, first.stop_evidence_id);
    assert_eq!(second.cumulative_seconds, first.cumulative_seconds);
    assert_eq!(bson::to_document(&wallet(&f).await).unwrap(), wallet_first);
    assert_eq!(
        f.db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        ledger_first
    );
    assert_eq!(
        f.db.collection::<Document>("usage_records")
            .count_documents(doc! {})
            .await
            .unwrap(),
        usage_first
    );
    assert_eq!(
        current(&f, &ready.node_id).await.native_uncertainty,
        deleting.native_uncertainty
    );
    f.db.drop().await.unwrap();
}
