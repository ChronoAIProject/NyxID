//! Mounted feature admission: real auth, personal flag resolution, lifecycle and
//! exact Billing transactions. Only platform readiness/retained-volume metadata
//! and Lago's external protocol are local fixtures; no cluster claim is made.
use super::build_router_with_state;
use crate::{
    AppState,
    crypto::jwt,
    errors::AppResult,
    models::{
        credits::Credits,
        machine_runtime_billing::{
            INTERVAL_COLLECTION, QUOTE_COLLECTION, RATE_COLLECTION, RUN_COLLECTION,
        },
        remote_machine::{COLLECTION_NAME as MACHINES, ManagedMachine},
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    services::{
        billing::{BillingService, lago_client::*},
        feature_flag_service::{self, FlagTarget},
        remote_machine_manager::PlatformReadiness,
        remote_machine_runtime as runtime, remote_machine_service as lifecycle, role_service,
    },
    test_utils,
};
use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;
use uuid::Uuid;

const FLAG: &str = "experimental:remote-machines";
const DENIAL: &str = "Forbidden: Remote Machines are not enabled for this account";

pub(super) async fn set_enabled(db: &Database, user: &str, enabled: bool) {
    feature_flag_service::set_platform_override(
        db,
        FLAG,
        &FlagTarget::User(user.into()),
        enabled,
        user,
    )
    .await
    .unwrap();
}

/// The same LagoApi seam used by runtime integration suites, with every
/// implemented provider operation counted so denied resync proves zero I/O.
struct LocalLago {
    calls: AtomicUsize,
    rates: Mutex<Vec<PlanRate>>,
    plan: String,
}
impl LocalLago {
    fn called(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}
#[async_trait]
impl LagoApi for LocalLago {
    async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String> {
        self.called();
        Ok(owner.external_customer_id.clone())
    }
    async fn ensure_subscription(&self, customer: &str, _: &str) -> AppResult<String> {
        self.called();
        Ok(customer.into())
    }
    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        self.called();
        Ok(LagoAck {
            transaction_id: event.transaction_id.clone(),
        })
    }
    async fn record_events_batch(&self, events: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        self.called();
        Ok(events
            .iter()
            .map(|event| LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
            .collect())
    }
    async fn current_usage(&self, customer: &str, subscription: &str) -> AppResult<LagoUsage> {
        self.called();
        Ok(LagoUsage {
            customer_id: customer.into(),
            subscription_id: subscription.into(),
            raw: json!({}),
        })
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        self.called();
        Ok(10)
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        self.called();
        Ok(vec![Entitlement {
            code: "remote-machine-runtime".into(),
            raw: json!({}),
        }])
    }
    async fn runtime_subscription(&self, id: &str) -> AppResult<RuntimeSubscription> {
        self.called();
        Ok(RuntimeSubscription {
            external_id: id.into(),
            external_customer_id: id.into(),
            plan_code: self.plan.clone(),
            status: "active".into(),
        })
    }
    async fn plan_rates(&self, _: &str) -> AppResult<PlanRates> {
        self.called();
        Ok(PlanRates {
            rates: self.rates.lock().unwrap().clone(),
            rejected: vec![],
        })
    }
    async fn sync_standard_charge(&self, _: &str, input: &ServicePriceSync) -> AppResult<()> {
        self.called();
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
    state: AppState,
    app: Router,
    lago: Arc<LocalLago>,
    rate: String,
}
async fn fixture(label: &str) -> Fixture {
    let db = test_utils::connect_transaction_test_database(label).await;
    crate::db::ensure_indexes(&db).await.unwrap();
    role_service::seed_system_roles(&db).await.unwrap();
    let config = crate::config::AppConfig {
        billing_enabled: true,
        billing_allocations_enabled: false,
        ..test_utils::test_app_config()
    };
    let lago = Arc::new(LocalLago {
        calls: AtomicUsize::new(0),
        rates: Mutex::new(vec![]),
        plan: config.lago_plan_code.clone(),
    });
    let mut state = test_utils::test_app_state_with_config(db.clone(), config);
    state.billing = Arc::new(BillingService::new_with_lago(
        db.clone(),
        Arc::new(state.config.clone()),
        lago.clone(),
    ));
    let rate = state
        .billing
        .stage_initial_machine_runtime_rate()
        .await
        .unwrap();
    state
        .billing
        .sync_machine_runtime_rate(&rate.id)
        .await
        .unwrap();
    state.local_machine_platform = Some(Arc::new(test_utils::LocalMachinePlatform {
        profile: lifecycle::fixed_profile(format!("registry.test/full@sha256:{}", "a".repeat(64))),
        readiness: PlatformReadiness {
            configuration_ready: true,
            manager_started: true,
            namespace: "nyxid-machine",
            blockers: vec![],
            physical_acceptance: "native_state_only_no_physical_guarantee",
            lifecycle_basis: "agent_sandbox_native_observation",
        },
    }));
    let (_, private) = build_router_with_state(state.clone());
    Fixture {
        db,
        app: private.with_state(state.clone()),
        state,
        lago,
        rate: rate.id,
    }
}
async fn actor(f: &Fixture, role: &str) -> (String, String) {
    let id = Uuid::new_v4();
    let roles = role_service::get_platform_role_ids(&f.db).await.unwrap();
    let mut user = test_utils::test_user(&id.to_string(), UserType::Person);
    user.role_ids = match role {
        "admin" => vec![roles.admin],
        "operator" => vec![roles.operator],
        _ => vec![],
    };
    f.db.collection::<User>(USERS)
        .insert_one(user)
        .await
        .unwrap();
    let now = bson::DateTime::now();
    f.db.collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME).insert_one(doc! {
        "_id": Uuid::new_v4().to_string(), "owner_id":id.to_string(), "lago_customer_id":id.to_string(), "lago_wallet_id":format!("wallet-{id}"), "lago_subscription_id":id.to_string(),
        "plan_kind":"prepaid", "balance_credits":Credits::from_whole(10), "reserved_credits":Credits::ZERO, "pending_lago_debits":Credits::ZERO, "pending_topup_expiry_credits":Credits::ZERO,
        "overdraft_cap_credits":Credits::ZERO,"has_payment_instrument":false,"suspended":false,"collection_state":"good","created_at":now,"updated_at":now,"balance_synced_at":now
    }).await.unwrap();
    let token = jwt::generate_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &id,
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    (id.to_string(), token)
}
async fn request(
    f: &Fixture,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
    key: &str,
) -> (StatusCode, Value) {
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("idempotency-key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn assert_flag_denied(response: (StatusCode, Value)) {
    assert_eq!(response.0, StatusCode::FORBIDDEN, "{}", response.1);
    assert_eq!(response.1["error"], "forbidden");
    assert_eq!(response.1["error_code"], 1002);
    assert_eq!(response.1["message"], DENIAL);
}
async fn durable(f: &Fixture) -> Vec<(String, Vec<Document>)> {
    let mut rows = vec![];
    for name in [
        MACHINES,
        crate::models::node::COLLECTION_NAME,
        crate::models::machine_free_slot::COLLECTION_NAME,
        crate::models::remote_machine_operation::COLLECTION_NAME,
        crate::models::remote_machine_outbox::COLLECTION_NAME,
        QUOTE_COLLECTION,
        RUN_COLLECTION,
        INTERVAL_COLLECTION,
        RATE_COLLECTION,
        crate::models::billing_wallet::COLLECTION_NAME,
        crate::models::usage_meter::COLLECTION_NAME,
        crate::models::credit_grant::COLLECTION_NAME,
        crate::models::usage_allowance_period::COLLECTION_NAME,
    ] {
        let values =
            f.db.collection::<Document>(name)
                .find(doc! {})
                .sort(doc! {"_id":1})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
        rows.push((name.into(), values));
    }
    rows
}
fn create_body(free: bool, quote: Option<&str>) -> Value {
    json!({"free_slot":free,"policy_version":"remote-machine-mvp","data_policy_version":"remote-machine-data","acknowledge_shared_environment":true,"quote_id":quote,"accept_account_balance":!free})
}
fn action_body(kind: &str, revision: i64, quote: Option<&str>) -> Value {
    json!({"action":kind,"expected_revision":revision,"policy_version":"remote-machine-mvp","data_policy_version":"remote-machine-data","acknowledge_shared_environment":true,"quote_id":quote,"accept_account_balance":quote.is_some()})
}
async fn current(f: &Fixture, id: &str) -> ManagedMachine {
    lifecycle::managed_machine_for_node(&f.db, id)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn flag_absent_and_explicit_off_block_all_mounted_admission_before_effects_for_users_and_admins()
 {
    let f = fixture("flag_mounted_denial").await;
    for role in ["user", "admin"] {
        let (owner, token) = actor(&f, role).await;
        for enabled in [None, Some(false)] {
            if let Some(enabled) = enabled {
                set_enabled(&f.db, &owner, enabled).await;
            }
            let before = durable(&f).await;
            let calls = f.lago.calls.load(Ordering::SeqCst);
            for free in [true, false] {
                assert_flag_denied(
                    request(
                        &f,
                        "POST",
                        "/api/v1/remote-machine-quotes",
                        &token,
                        json!({"free_slot":free}),
                        "quote-off",
                    )
                    .await,
                );
                assert_flag_denied(
                    request(
                        &f,
                        "POST",
                        "/api/v1/remote-machines",
                        &token,
                        create_body(free, Some(&Uuid::new_v4().to_string())),
                        "create-off",
                    )
                    .await,
                );
                let node = Uuid::new_v4();
                for (path, action) in [
                    (format!("/api/v1/remote-machines/{node}/actions"), "resume"),
                    (format!("/api/v1/remote-machines/{node}/actions"), "rekey"),
                    (format!("/api/v1/remote-machines/{node}/rekey"), "rekey"),
                ] {
                    assert_flag_denied(
                        request(
                            &f,
                            "POST",
                            &path,
                            &token,
                            action_body(action, 1, None),
                            "action-off",
                        )
                        .await,
                    );
                }
            }
            assert_eq!(
                durable(&f).await,
                before,
                "{role}/{enabled:?} denial mutated admission/funding state"
            );
            assert_eq!(
                f.lago.calls.load(Ordering::SeqCst),
                calls,
                "flag denial reached provider"
            );
        }
    }
    f.db.drop().await.unwrap();
}

/// No runtime is claimed here: real never-submitted safety settlement closes
/// the accepted run; retained PVC identity is the explicit external fixture.
async fn paused_fixture(f: &Fixture, id: &str) -> ManagedMachine {
    let machine = current(f, id).await;
    runtime::safety_pause(
        &f.db,
        Some(f.state.billing.clone()),
        &machine,
        "runtime_funding_unavailable",
    )
    .await
    .unwrap();
    let stopped = current(f, id).await;
    assert!(
        runtime::finalize_never_submitted(&f.db, Some(f.state.billing.clone()), &stopped)
            .await
            .unwrap()
    );
    f.db.collection::<Document>(MACHINES).update_one(doc! {"_id":id}, doc! {"$set":{"pvc_name":"fixture-retained-workspace","pvc_uid":Uuid::new_v4().to_string(),"runtime_generation":1_i64,"credential_epoch":1_i64}}).await.unwrap();
    current(f, id).await
}

#[tokio::test]
async fn flag_on_personal_users_and_admins_accept_free_paid_create_resume_and_current_replay_gate()
{
    let f = fixture("flag_mounted_success").await;
    for role in ["user", "admin"] {
        for free in [true, false] {
            let (owner, token) = actor(&f, role).await;
            let (_, other_token) = actor(&f, "user").await;
            set_enabled(&f.db, &owner, true).await;
            // An enabled person does not opt another authenticated account in.
            assert_flag_denied(
                request(
                    &f,
                    "POST",
                    "/api/v1/remote-machine-quotes",
                    &other_token,
                    json!({"free_slot":free}),
                    "other",
                )
                .await,
            );
            let (status, quote) = request(
                &f,
                "POST",
                "/api/v1/remote-machine-quotes",
                &token,
                json!({"free_slot":free}),
                "quote",
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{role}/{free}: {quote}");
            assert_eq!(quote["free_slot_requested"], free);
            assert_eq!(quote["quote_id"].is_string(), !free);
            let body = create_body(free, quote["quote_id"].as_str());
            set_enabled(&f.db, &owner, false).await;
            let before = durable(&f).await;
            let calls = f.lago.calls.load(Ordering::SeqCst);
            assert_flag_denied(
                request(
                    &f,
                    "POST",
                    "/api/v1/remote-machines",
                    &token,
                    body.clone(),
                    "create-enabled",
                )
                .await,
            );
            assert_eq!(
                durable(&f).await,
                before,
                "valid quote denied before first create must remain unconsumed"
            );
            assert_eq!(f.lago.calls.load(Ordering::SeqCst), calls);
            set_enabled(&f.db, &owner, true).await;
            let (status, created) = request(
                &f,
                "POST",
                "/api/v1/remote-machines",
                &token,
                body.clone(),
                "create-enabled",
            )
            .await;
            assert_eq!(status, StatusCode::ACCEPTED, "{role}/{free}: {created}");
            let id = created["machine"]["node_id"].as_str().unwrap();
            if !free {
                assert!(current(&f, id).await.runtime_run_id.is_some());
            }
            set_enabled(&f.db, &owner, false).await;
            let before = durable(&f).await;
            assert_flag_denied(
                request(
                    &f,
                    "POST",
                    "/api/v1/remote-machines",
                    &token,
                    body.clone(),
                    "create-enabled",
                )
                .await,
            );
            assert_eq!(durable(&f).await, before);
            set_enabled(&f.db, &owner, true).await;
            let (status, replay) = request(
                &f,
                "POST",
                "/api/v1/remote-machines",
                &token,
                body,
                "create-enabled",
            )
            .await;
            assert_eq!(status, StatusCode::ACCEPTED, "{replay}");
            assert_eq!(replay["replayed"], true);
            assert_eq!(replay["machine"]["node_id"], id);
            let paused = paused_fixture(&f, id).await;
            let (status, quote) = request(
                &f,
                "POST",
                "/api/v1/remote-machine-quotes",
                &token,
                json!({"free_slot":free,"machine_id":id}),
                "resume-quote",
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{role}/{free}: {quote}");
            let body = action_body("resume", paused.revision, quote["quote_id"].as_str());
            let path = format!("/api/v1/remote-machines/{id}/actions");
            set_enabled(&f.db, &owner, false).await;
            let before = durable(&f).await;
            assert_flag_denied(
                request(
                    &f,
                    "POST",
                    "/api/v1/remote-machine-quotes",
                    &token,
                    json!({"free_slot":free,"machine_id":id}),
                    "resume-quote-off",
                )
                .await,
            );
            assert_flag_denied(
                request(&f, "POST", &path, &token, body.clone(), "resume-enabled").await,
            );
            for (suffix, kind) in [("actions", "rekey"), ("rekey", "rekey")] {
                assert_flag_denied(
                    request(
                        &f,
                        "POST",
                        &format!("/api/v1/remote-machines/{id}/{suffix}"),
                        &token,
                        action_body(kind, paused.revision, None),
                        "rekey-off",
                    )
                    .await,
                );
            }
            assert_eq!(durable(&f).await, before);
            set_enabled(&f.db, &owner, true).await;
            let (status, resumed) =
                request(&f, "POST", &path, &token, body.clone(), "resume-enabled").await;
            assert_eq!(status, StatusCode::ACCEPTED, "{role}/{free}: {resumed}");
            assert_eq!(resumed["operation"]["action"], "resume");
            assert_eq!(current(&f, id).await.pvc_uid, paused.pvc_uid);
            if !free {
                assert_ne!(current(&f, id).await.runtime_run_id, paused.runtime_run_id);
            }
            set_enabled(&f.db, &owner, false).await;
            let before = durable(&f).await;
            assert_flag_denied(
                request(&f, "POST", &path, &token, body.clone(), "resume-enabled").await,
            );
            assert_eq!(durable(&f).await, before);
            set_enabled(&f.db, &owner, true).await;
            let (status, replay) = request(&f, "POST", &path, &token, body, "resume-enabled").await;
            assert_eq!(status, StatusCode::ACCEPTED, "{replay}");
            assert_eq!(replay["operation"]["id"], resumed["operation"]["id"]);
        }
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn flag_on_rekey_dedicated_and_generic_paths_accept_and_replay_only_while_enabled() {
    let f = fixture("flag_mounted_rekey").await;
    for role in ["user", "admin"] {
        for free in [true, false] {
            for suffix in ["rekey", "actions"] {
                let (owner, token) = actor(&f, role).await;
                set_enabled(&f.db, &owner, true).await;
                let (status, quote) = request(
                    &f,
                    "POST",
                    "/api/v1/remote-machine-quotes",
                    &token,
                    json!({"free_slot":free}),
                    "quote",
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{quote}");
                let (status, created) = request(
                    &f,
                    "POST",
                    "/api/v1/remote-machines",
                    &token,
                    create_body(free, quote["quote_id"].as_str()),
                    "create",
                )
                .await;
                assert_eq!(status, StatusCode::ACCEPTED, "{created}");
                let id = created["machine"]["node_id"].as_str().unwrap();
                let paused = paused_fixture(&f, id).await;
                let body = action_body("rekey", paused.revision, None);
                let path = format!("/api/v1/remote-machines/{id}/{suffix}");
                let (status, accepted) =
                    request(&f, "POST", &path, &token, body.clone(), "rekey").await;
                assert_eq!(
                    status,
                    StatusCode::ACCEPTED,
                    "{role}/{free}/{suffix}: {accepted}"
                );
                assert_eq!(accepted["operation"]["action"], "rekey");
                set_enabled(&f.db, &owner, false).await;
                let before = durable(&f).await;
                assert_flag_denied(request(&f, "POST", &path, &token, body.clone(), "rekey").await);
                assert_eq!(durable(&f).await, before);
                set_enabled(&f.db, &owner, true).await;
                let (status, replay) = request(&f, "POST", &path, &token, body, "rekey").await;
                assert_eq!(status, StatusCode::ACCEPTED, "{replay}");
                assert_eq!(replay["operation"]["id"], accepted["operation"]["id"]);
            }
        }
    }
    f.db.drop().await.unwrap();
}

fn rate_body() -> Value {
    json!({"profile_version":"remote-machine","credits_per_second":"0.000040"})
}

#[tokio::test]
async fn flag_runtime_rate_writes_require_human_admin_and_personal_enable_while_reads_remain_available()
 {
    let f = fixture("flag_rate_authority").await;
    let (admin, admin_token) = actor(&f, "admin").await;
    let (operator, operator_token) = actor(&f, "operator").await;
    let (user, user_token) = actor(&f, "user").await;
    let paths = [
        "/api/v1/admin/billing/runtime/rates".into(),
        format!("/api/v1/admin/billing/runtime/rates/{}/resync", f.rate),
        format!("/api/v1/admin/billing/runtime/rates/{}/retire", f.rate),
    ];
    for enabled in [None, Some(false)] {
        if let Some(enabled) = enabled {
            set_enabled(&f.db, &admin, enabled).await;
        }
        let before = durable(&f).await;
        let calls = f.lago.calls.load(Ordering::SeqCst);
        for path in &paths {
            assert_flag_denied(request(&f, "POST", path, &admin_token, rate_body(), "rate").await);
        }
        assert_eq!(durable(&f).await, before);
        assert_eq!(
            f.lago.calls.load(Ordering::SeqCst),
            calls,
            "denied resync contacted Lago"
        );
        for token in [&admin_token, &operator_token] {
            let (status, view) = request(
                &f,
                "GET",
                "/api/v1/admin/billing/runtime",
                token,
                Value::Null,
                "read",
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{view}");
            assert_eq!(view["rates"].as_array().unwrap().len(), 1);
        }
    }
    for person in [&admin, &operator, &user] {
        set_enabled(&f.db, person, true).await;
    }
    let id = Uuid::parse_str(&admin).unwrap();
    let external_client = "external-fixture";
    let now = chrono::Utc::now();
    f.db.collection::<crate::models::oauth_client::OauthClient>(
        crate::models::oauth_client::COLLECTION_NAME,
    )
    .insert_one(crate::models::oauth_client::OauthClient {
        id: external_client.into(),
        client_name: "Runtime rate human-only fixture".into(),
        client_secret_hash: String::new(),
        redirect_uris: vec![],
        allowed_scopes: "read write".into(),
        scope_provenance: Default::default(),
        grant_types: "authorization_code".into(),
        client_type: "public".into(),
        is_active: true,
        delegation_scopes: String::new(),
        default_service_catalog_slugs: vec![],
        broker_capability_enabled: false,
        revocation_webhook_url: None,
        revocation_webhook_secret_encrypted: None,
        connection_webhook_url: None,
        connection_webhook_secret_encrypted: None,
        connection_webhook_key_id: None,
        connection_webhook_enabled: false,
        created_by: None,
        created_at: now,
        updated_at: now,
    })
    .await
    .unwrap();
    let consent = crate::services::consent_service::grant_consent(
        &f.db,
        &admin,
        external_client,
        "read write",
    )
    .await
    .unwrap();
    let reference = crate::services::consent_service::protected_reference(&consent).unwrap();
    let external = jwt::generate_oauth_access_token_with_consent(
        &f.state.jwt_keys,
        &f.state.config,
        &id,
        "read write",
        None,
        None,
        None,
        None,
        None,
        external_client,
        Some(&reference),
    )
    .unwrap();
    let claims = jwt::verify_token(&f.state.jwt_keys, &f.state.config, &external).unwrap();
    crate::services::oauth_token_authority_service::validate_claims(&f.db, &claims)
        .await
        .expect(
            "external fixture must have live OAuth authority before testing the human-only gate",
        );
    let delegated = jwt::generate_delegated_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &id,
        "account:read",
        "delegated-fixture",
        60,
        None,
    )
    .unwrap();
    let key = crate::services::key_service::create_api_key(
        &f.db,
        &admin,
        "Rate fixture",
        "proxy",
        None,
        None,
        Some(&[]),
        Some(&[]),
        Some(true),
        Some(true),
        Some(false),
        None,
        None,
        Some("codex"),
        None,
    )
    .await
    .unwrap();
    let before = durable(&f).await;
    let calls = f.lago.calls.load(Ordering::SeqCst);
    for (kind, token) in [
        ("user", user_token.as_str()),
        ("operator", operator_token.as_str()),
        ("external", external.as_str()),
        ("delegated", delegated.as_str()),
        ("api-key", key.full_key.as_str()),
    ] {
        for path in &paths {
            let (status, error) =
                request(&f, "POST", path, token, rate_body(), "denied-rate").await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{kind}/{path}: {error}");
            assert_ne!(
                error["message"], DENIAL,
                "flag must not replace {kind} authority checks"
            );
            if matches!(kind, "user" | "operator") {
                assert_eq!(error["message"], "Forbidden: Admin access required");
            }
        }
    }
    assert_eq!(durable(&f).await, before);
    assert_eq!(f.lago.calls.load(Ordering::SeqCst), calls);
    let (status, denied) = request(
        &f,
        "GET",
        "/api/v1/admin/billing/runtime",
        &user_token,
        Value::Null,
        "read",
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    for token in [&admin_token, &operator_token] {
        let (status, view) = request(
            &f,
            "GET",
            "/api/v1/admin/billing/runtime",
            token,
            Value::Null,
            "read",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
    }
    let (status, staged) = request(&f, "POST", &paths[0], &admin_token, rate_body(), "stage").await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    let rate = staged["rate_version"].as_str().unwrap();
    let calls = f.lago.calls.load(Ordering::SeqCst);
    let (status, synced) = request(
        &f,
        "POST",
        &format!("/api/v1/admin/billing/runtime/rates/{rate}/resync"),
        &admin_token,
        json!({}),
        "sync",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    assert_eq!(synced["synced"], true);
    assert!(f.lago.calls.load(Ordering::SeqCst) > calls);
    let (status, retired) = request(
        &f,
        "POST",
        &format!("/api/v1/admin/billing/runtime/rates/{rate}/retire"),
        &admin_token,
        json!({}),
        "retire",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{retired}");
    assert_eq!(retired["retired"], true);
    let row =
        f.db.collection::<Document>(RATE_COLLECTION)
            .find_one(doc! {"_id":rate})
            .await
            .unwrap()
            .unwrap();
    assert!(row.get_bool("retired").unwrap());
    assert_eq!(row.get_str("sync_status").unwrap(), "synced");
    f.db.drop().await.unwrap();
}

/// Exercise production readiness consumers with explicit native observation
/// and live Node lease fixtures (no running container or Kubernetes is implied).
async fn ready_fixture(f: &Fixture, id: &str) -> ManagedMachine {
    let created = current(f, id).await;
    let advanced =
        lifecycle::advance_runtime_generation(&f.db, id, created.revision, 0, None, None)
            .await
            .unwrap();
    let bound = lifecycle::apply_runtime_observation(
        &f.db,
        lifecycle::RuntimeObservationInput {
            machine_id: advanced.node_id.clone(),
            sandbox_uid: Uuid::new_v4().to_string(),
            pod_uid: Some(Uuid::new_v4().to_string()),
            pvc_name: Some("fixture-workspace".into()),
            pvc_uid: Some(Uuid::new_v4().to_string()),
            ready: true,
            physically_stopped: false,
            source: "local-native-api-fixture".into(),
            now: None,
        },
    )
    .await
    .unwrap();
    let deadline = bound
        .funded_until
        .unwrap_or(chrono::Utc::now() + chrono::Duration::minutes(5));
    let bound = runtime::install_test_coverage(&f.db, &bound, deadline)
        .await
        .unwrap();
    crate::services::remote_machine_native::activate(&f.db, &bound)
        .await
        .unwrap();
    let delivery = Uuid::new_v4().to_string();
    lifecycle::accept_bootstrap_delivery(
        &f.db,
        id,
        bound.runtime_generation,
        bound.credential_epoch,
        &delivery,
        "sealed-local-fixture",
    )
    .await
    .unwrap();
    let binding = nyxid_machine::managed::Binding {
        node_id: id.into(),
        sandbox_uid: bound.sandbox_uid.clone().unwrap(),
        pod_uid: bound.pod_uid.clone().unwrap(),
        runtime_generation: bound.runtime_generation as u64,
        credential_epoch: bound.credential_epoch as u64,
        writer_fence: bound.writer_fence.clone(),
        operation_id: bound.active_operation_id.clone().unwrap(),
        delivery_id: delivery,
    };
    let connection = Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
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
    let lease = doc! {"instance_name":"local-flag-test","generation_id":Uuid::new_v4().to_string(),"connection_id":&connection,"internal_base_url":"http://127.0.0.1","claimed_at":bson::DateTime::from_chrono(now),"renewed_at":bson::DateTime::from_chrono(now),"expires_at":bson::DateTime::from_chrono(now+chrono::Duration::minutes(5)),"capabilities_resolved":true};
    f.db.collection::<Document>(crate::models::node::COLLECTION_NAME).update_one(doc! {"_id":id}, doc! {"$set":{"is_active":true,"status":"online","connection_owner":lease,"machine":bson::to_bson(&profile).unwrap()}}).await.unwrap();
    crate::services::remote_machine_manager::mark_runtime_online_with_billing(
        &f.db,
        id,
        &binding,
        &connection,
        f.state.billing.clone(),
    )
    .await
    .unwrap();
    let ready = current(f, id).await;
    assert_eq!(
        ready.resource_state,
        crate::models::remote_machine::ResourceState::Ready
    );
    ready
}

#[tokio::test]
async fn flag_off_after_running_preserves_mounted_owner_reads_pause_delete_and_owner_authority() {
    let f = fixture("flag_off_management").await;
    for free in [true, false] {
        let (owner, token) = actor(&f, "user").await;
        let (_, foreign_token) = actor(&f, "user").await;
        set_enabled(&f.db, &owner, true).await;
        let (status, quote) = request(
            &f,
            "POST",
            "/api/v1/remote-machine-quotes",
            &token,
            json!({"free_slot":free}),
            "quote",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{quote}");
        let (status, created) = request(
            &f,
            "POST",
            "/api/v1/remote-machines",
            &token,
            create_body(free, quote["quote_id"].as_str()),
            "create",
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{created}");
        let id = created["machine"]["node_id"].as_str().unwrap();
        let ready = ready_fixture(&f, id).await;
        set_enabled(&f.db, &owner, false).await;
        let off_machine = current(&f, id).await;
        assert_eq!(
            off_machine.resource_state,
            crate::models::remote_machine::ResourceState::Ready,
            "flag change must not auto-pause"
        );
        for path in [
            "/api/v1/remote-machines".into(),
            "/api/v1/remote-machine-entitlement".into(),
            format!("/api/v1/remote-machines/{id}"),
            format!("/api/v1/remote-machines/{id}/operations"),
            format!("/api/v1/remote-machines/{id}/events"),
            format!("/api/v1/remote-machines/{id}/billing"),
            format!("/api/v1/remote-machines/{id}/billing/runs"),
            format!("/api/v1/remote-machines/{id}/usage"),
        ] {
            let (status, value) = request(&f, "GET", &path, &token, Value::Null, "read").await;
            let expected = if free && path == format!("/api/v1/remote-machines/{id}/billing") {
                StatusCode::NOT_FOUND // Free Machines have no runtime billing contract.
            } else {
                StatusCode::OK
            };
            assert_eq!(status, expected, "{free}/{path}: {value}");
            if expected == StatusCode::NOT_FOUND {
                assert_eq!(
                    value["message"],
                    "Not found: runtime billing contract not found"
                );
            }
        }
        let path = format!("/api/v1/remote-machines/{id}");
        let (status, denied) =
            request(&f, "GET", &path, &foreign_token, Value::Null, "foreign").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{denied}");
        let before = durable(&f).await;
        for (method, endpoint, body) in [
            (
                "POST",
                format!("{path}/actions"),
                action_body("pause", ready.revision, None),
            ),
            (
                "DELETE",
                path.clone(),
                json!({"expected_revision":ready.revision,"data_policy_version":"remote-machine-data","acknowledge_data_loss":true}),
            ),
        ] {
            let (status, denied) =
                request(&f, method, &endpoint, &foreign_token, body, "foreign").await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{denied}");
        }
        assert_eq!(durable(&f).await, before);
        let (status, pause) = request(
            &f,
            "POST",
            &format!("{path}/actions"),
            &token,
            action_body("pause", ready.revision, None),
            "pause-off",
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{pause}");
        assert_eq!(pause["operation"]["action"], "pause");
        let stopping = current(&f, id).await;
        assert_eq!(
            stopping.resource_state,
            crate::models::remote_machine::ResourceState::Stopping
        );
        // Trusted external stop observation passes through production evidence,
        // exact settlement and native finality, while the flag remains OFF.
        let observation = crate::models::remote_machine::NativeRuntimeObservation {
            sandbox_uid: stopping.sandbox_uid.clone().unwrap(),
            pod_uid: stopping.pod_uid.clone(),
            runtime_generation: stopping.runtime_generation,
            credential_epoch: stopping.credential_epoch,
            writer_fence: stopping.writer_fence.clone(),
            operation_id: stopping.active_operation_id.clone().unwrap(),
            sandbox_generation: stopping
                .native_observation
                .as_ref()
                .map_or(1, |o| o.sandbox_generation + 1),
            sandbox_resource_version: Uuid::new_v4().to_string(),
            observed_at: chrono::DateTime::from_timestamp_millis(
                chrono::Utc::now().timestamp_millis(),
            )
            .unwrap(),
            running: false,
            suspended: true,
            stop_confirmed: true,
            run_not_after: stopping.funded_until.or(stopping.native_run_not_after),
            evidence_ref: format!("local-native-stop-fixture:{}", Uuid::new_v4()),
        };
        let stopped = crate::services::remote_machine_native::record(
            &f.db,
            &stopping,
            &observation,
            Document::new(),
        )
        .await
        .unwrap();
        runtime::native_observe_and_settle(&f.db, f.state.billing.clone(), &stopped, &observation)
            .await
            .unwrap();
        let paused = crate::services::remote_machine_native::finalize_stop(&f.db, &stopped, None)
            .await
            .unwrap();
        let (status, deleted) = request(&f, "DELETE", &path, &token, json!({"expected_revision":paused.revision,"data_policy_version":"remote-machine-data","acknowledge_data_loss":true}), "delete-off").await;
        assert_eq!(status, StatusCode::ACCEPTED, "{deleted}");
        assert_eq!(deleted["operation"]["action"], "delete");
        assert_eq!(
            current(&f, id).await.resource_state,
            crate::models::remote_machine::ResourceState::Deleting
        );
    }
    f.db.drop().await.unwrap();
}
