//! Local cross-service plumbing only: real NyxID HTTP callback, transactions,
//! funding and settlement; the external runtime, model and Lago are local mocks.
use super::*;
use crate::models::{
    billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
    platform_bot_config::{PlatformBotConfig, PlatformExecutionProfile},
    platform_bot_runtime_controls::PlatformBotRuntimeControls,
    platform_manager_ingress::{IngressAudience, IngressPhase, PlatformManagerIngress},
    platform_public_session::{COLLECTION_NAME as SESSIONS, PlatformPublicSession},
    usage_meter::{COLLECTION_NAME as METERS, UsageMeterRow},
};
use crate::services::{
    audit_service::AuditActor, platform_bot_runtime_control_service as controls,
    platform_public_credential_service as credentials,
};
use axum::{
    body::{Body, to_bytes},
    extract::State,
    http::Request,
    response::IntoResponse,
    routing::post,
};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use tokio::sync::Mutex;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const NORMAL: u8 = 0;
const REPLAY: u8 = 1;
const NO_CALLBACK: u8 = 2;
const FUND_ONLY: u8 = 3;
const EXPIRE_CONSENT: u8 = 4;
const CATALOG_DRIFT: u8 = 5;
const SPONSOR_REVOKED: u8 = 6;
const WRONG_BODY: u8 = 7;
const CAP_DAY_ENDED: u8 = 8;
const SLOW_CALLBACK: u8 = 9;

#[derive(Clone)]
struct ExternalRuntime {
    state: AppState,
    manifest: Arc<Mutex<RuntimeManifest>>,
    callback_url: reqwest::Url,
    mode: Arc<AtomicU8>,
    callback_statuses: Arc<Mutex<Vec<u16>>>,
    cleanup_authorities: Arc<Mutex<Vec<Value>>>,
}

#[derive(Deserialize)]
struct ProbeRequest {
    contract: String,
    nonce: String,
    binding: ManagerBinding,
    manifest: RuntimeManifest,
    selected_profile: String,
    probe_id: String,
    probe_callback: String,
    probe_authorization: String,
    probe_body_utf8: String,
}
impl Drop for ProbeRequest {
    fn drop(&mut self) {
        self.probe_authorization.zeroize();
    }
}

async fn discovery(
    State(mock): State<ExternalRuntime>,
    request: Request<Body>,
) -> axum::response::Response {
    let raw = Zeroizing::new(
        request.headers()["x-nyxid-public-authority"]
            .to_str()
            .unwrap()
            .to_owned(),
    );
    assert!(!request.headers().contains_key("authorization"));
    let body = Zeroizing::new(
        to_bytes(request.into_body(), MAX_BYTES)
            .await
            .unwrap()
            .to_vec(),
    );
    let authority = verify_assertion(&mock.state, &raw, "discovery", &body).unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(authority["binding"], value["binding"]);
    let manifest = mock.manifest.lock().await.clone();
    axum::Json(json!({"nonce":value["nonce"],"binding":value["binding"],"manifest":manifest}))
        .into_response()
}

async fn readiness(mock: ExternalRuntime, request: Request<Body>) -> axum::response::Response {
    let raw = Zeroizing::new(
        request.headers()["x-nyxid-public-authority"]
            .to_str()
            .unwrap()
            .to_owned(),
    );
    assert!(!request.headers().contains_key("authorization"));
    let bytes = Zeroizing::new(
        to_bytes(request.into_body(), MAX_BYTES)
            .await
            .unwrap()
            .to_vec(),
    );
    let authority = verify_assertion(&mock.state, &raw, "readiness", &bytes).unwrap();
    let request: ProbeRequest = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(request.contract, EXECUTION_CONTRACT);
    assert_eq!(request.probe_callback, mock.callback_url.path());
    assert_eq!(
        authority["binding"],
        serde_json::to_value(&request.binding).unwrap()
    );
    let probe = mock
        .state
        .db
        .collection::<PublicProbe>(PROBES)
        .find_one(doc! {"_id": &request.probe_id})
        .await
        .unwrap()
        .unwrap();
    let body: Value = serde_json::from_str(&request.probe_body_utf8).unwrap();
    assert_eq!(
        body["max_output_tokens"],
        request.manifest.profiles[0].model_operations[0].max_output_tokens
    );
    assert_eq!(
        probe.operation.max_output_tokens,
        body["max_output_tokens"].as_u64().unwrap() as u32
    );
    // JWT deadline, stored route deadline and callback body all use one bound.
    let claims = verify_assertion(
        &mock.state,
        &request.probe_authorization,
        "probe_model",
        request.probe_body_utf8.as_bytes(),
    )
    .unwrap();
    assert_eq!(claims["probe_id"], request.probe_id);
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&[AUDIENCE]);
    validation.set_issuer(&[&mock.state.config.jwt_issuer]);
    validation.leeway = 0;
    let signed = jsonwebtoken::decode::<Assertion>(
        &request.probe_authorization,
        &mock.state.jwt_keys.decoding,
        &validation,
    )
    .unwrap();
    assert_eq!(signed.claims.exp, probe.expires_at.timestamp());
    let (_, mut route, maximum, _) = super::super::route(
        &mock.state,
        &request.binding.sponsor_owner_user_id,
        &probe.id,
        &probe.operation,
    )
    .await
    .unwrap();
    route.public_probe = true;
    route.public_admission.as_mut().unwrap().expires_at = probe.expires_at;
    assert_eq!(
        probe.maximum, maximum,
        "serialized advertised bound must be reserved truthfully"
    );
    assert_eq!(
        probe.route_digest,
        super::super::super::platform_public_budget_service::route_digest(&route).unwrap()
    );
    let mode = mock.mode.load(Ordering::SeqCst);
    if matches!(mode, SLOW_CALLBACK | NO_CALLBACK) {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    if mode == EXPIRE_CONSENT {
        mock.state
            .db
            .collection::<Document>(PROBE_CONSENTS)
            .update_many(
                doc! {},
                doc! {"$set": {"expires_at": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
    }
    if matches!(
        mode,
        FUND_ONLY | CATALOG_DRIFT | SPONSOR_REVOKED | CAP_DAY_ENDED
    ) {
        if mode == CATALOG_DRIFT {
            mock.state
                .db
                .collection::<Document>(SERVICES)
                .update_one(
                    doc! {"_id": &probe.operation.service_id},
                    doc! {"$set": {"base_url": "https://drift.invalid/v1"}},
                )
                .await
                .unwrap();
        }
        if mode == SPONSOR_REVOKED {
            mock.state
                .db
                .collection::<Document>(crate::models::user::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &request.binding.sponsor_owner_user_id},
                    doc! {"$set": {"is_active": false}},
                )
                .await
                .unwrap();
        }
        if mode == CAP_DAY_ENDED {
            // Keep a live signed token/quoted route, but end the stored cap day.
            // Actual funding must use Mongo's clock rather than the caller's token TTL.
            mock.state.db.collection::<Document>(PROBES).update_one(doc! {"_id": &probe.id},
                doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now().date_naive().and_hms_opt(0,0,0).unwrap().and_utc())}}).await.unwrap();
        }
        let funding = mock.state.billing.open_required(&route).await;
        if mode == FUND_ONLY {
            funding.unwrap();
        } else {
            assert!(
                funding.is_err(),
                "drift must deny actual funding, before any reservation"
            );
        }
    } else if mode != NO_CALLBACK {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let callback = || {
            client
                .post(mock.callback_url.clone())
                .bearer_auth(&request.probe_authorization)
                .header("content-type", "application/json")
                .body(if mode == WRONG_BODY {
                    request
                        .probe_body_utf8
                        .replace("Reply OK.", "Changed request")
                } else {
                    request.probe_body_utf8.clone()
                })
                .send()
        };
        let responses = if mode == REPLAY {
            let (first, second) = tokio::join!(callback(), callback());
            vec![first.unwrap(), second.unwrap(), callback().await.unwrap()]
        } else {
            vec![callback().await.unwrap()]
        };
        for response in responses {
            mock.callback_statuses
                .lock()
                .await
                .push(response.status().as_u16());
            // Consume the real handler response to finish the HTTP exchange.
            let _ = response.bytes().await.unwrap();
        }
    }
    axum::Json(
        json!({"contract": EXECUTION_CONTRACT, "nonce": request.nonce,
        "binding": request.binding, "manifest": request.manifest,
        "selected_profile": request.selected_profile, "probe_id": request.probe_id}),
    )
    .into_response()
}

async fn cleanup_request(
    State(mock): State<ExternalRuntime>,
    request: Request<Body>,
) -> axum::response::Response {
    let raw = Zeroizing::new(
        request.headers()["x-nyxid-public-authority"]
            .to_str()
            .unwrap()
            .to_owned(),
    );
    assert!(!request.headers().contains_key("authorization"));
    let bytes = Zeroizing::new(
        to_bytes(request.into_body(), MAX_BYTES)
            .await
            .unwrap()
            .to_vec(),
    );
    let authority = verify_assertion(&mock.state, &raw, "cleanup", &bytes).unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(authority["public_session_id"], body["public_session_id"]);
    assert_eq!(authority["policy_digest"], body["policy_digest"]);
    mock.cleanup_authorities.lock().await.push(authority);
    axum::Json(json!({"contract": CLEANUP_CONTRACT,
        "public_session_id": body["public_session_id"], "policy_digest": body["policy_digest"], "erased": true}))
    .into_response()
}

struct UnusedOwnerHook;
#[async_trait::async_trait]
impl crate::services::assistant_nyxagent_runtime::OwnerTurnSettledHook for UnusedOwnerHook {
    async fn settled(
        &self,
        _: crate::models::assistant_conversation::AssistantConversation,
        _: ReplyRef,
    ) {
        panic!("public handshake must never use the personal flow");
    }
}

struct Handshake {
    state: AppState,
    provider: MockServer,
    _lago: MockServer,
    runtime: ExternalRuntime,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    reviewed: ManagerRuntimePolicy,
    actor: AuditActor,
    profile: PlatformBotConfig,
    producers: Arc<PlatformManagerProducers>,
}
impl Drop for Handshake {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Handshake {
    async fn new(label: &str, mode: u8, provider_status: u16, provider_model: &str) -> Self {
        let db = crate::test_utils::connect_transaction_test_database(label).await;
        // Actual indexes provide the unique meter, bucket, ledger and session fences.
        crate::db::ensure_indexes(&db).await.unwrap();
        let provider = MockServer::start().await;
        let lago = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/api/v1/subscriptions/public-subscription/entitlements",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"entitlements":[{"code":"*"}]})),
            )
            .mount(&lago)
            .await;
        let mut config = crate::test_utils::test_app_config();
        config.billing_enabled = true;
        config.lago_api_url = Some(lago.uri());
        config.lago_api_key = Some("local-test-lago-key".into());
        let state = crate::test_utils::test_app_state_with_config(db.clone(), config);
        let callback_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let callback_base = format!("http://{}", callback_listener.local_addr().unwrap());
        let callback_app = axum::Router::new()
            .route(
                "/api/v1/public-nyxbot/models/{service_id}/{*path}",
                post(crate::handlers::platform_public_model::dispatch),
            )
            .with_state(state.clone());
        let callback_task = tokio::spawn(async move {
            axum::serve(callback_listener, callback_app).await.unwrap();
        });
        let runtime_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let runtime_base = format!("http://{}", runtime_listener.local_addr().unwrap());
        let mut outer = outer();
        outer.base_url = runtime_base;
        let mut service = model();
        service.base_url = format!("{}/v1", provider.uri());
        service.credential_encrypted = state
            .encryption_keys
            .encrypt(b"local-test-provider-key")
            .await
            .unwrap();
        service.billing = Some(serde_json::from_value(json!({"platform_billable":true,"platform_key_pricing":{
            "metric":"input_tokens","credits_per_unit":"0.000005","lago_metric_code":"public_input","sync_status":"synced","components":[
            {"metric":"output_tokens","credits_per_unit":"0.00002","lago_metric_code":"public_output","sync_status":"synced"},
            {"metric":"cache_read_tokens","credits_per_unit":"0.0000008","lago_metric_code":"public_cache_read","sync_status":"synced"},
            {"metric":"cache_write_tokens","credits_per_unit":"0.000006","lago_metric_code":"public_cache_write","sync_status":"synced"},
            {"metric":"images","credits_per_unit":"0.2","lago_metric_code":"public_images","sync_status":"synced"}]}})).unwrap());
        db.collection::<DownstreamService>(SERVICES)
            .insert_many([&outer, &service])
            .await
            .unwrap();
        cache_rates(
            &db,
            &[
                ("public_input", "0.000005"),
                ("public_output", "0.00002"),
                ("public_cache_read", "0.0000008"),
                ("public_cache_write", "0.000006"),
                ("public_images", "0.2"),
            ],
        )
        .await;
        let e = evidence(&outer, &service, &[]);
        validate_manifest(&e.manifest)
            .expect("real catalog authority must satisfy the runtime manifest contract");
        let reviewed = reviewed(&e);
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let roles = crate::services::role_service::get_platform_role_ids(&db)
            .await
            .unwrap();
        let actor_id = uuid::Uuid::new_v4().to_string();
        let mut human =
            crate::test_utils::test_user(&actor_id, crate::models::user::UserType::Person);
        human.role_ids = vec![roles.admin];
        let sponsor = crate::test_utils::test_user(
            &e.binding.sponsor_owner_user_id,
            crate::models::user::UserType::Org,
        );
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_many([human, sponsor])
            .await
            .unwrap();
        db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME).insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "org_user_id": &e.binding.sponsor_owner_user_id,
            "member_user_id": &actor_id, "role": "admin", "revoked_at": bson::Bson::Null, "created_at": bson::DateTime::now(),
        }).await.unwrap();
        crate::services::feature_flag_service::set_platform_override(
            &db,
            crate::services::feature_flag_service::BILLING_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            &actor_id,
        )
        .await
        .unwrap();
        let now = Utc::now();
        let profile = PlatformBotConfig {
            id: e.binding.profile_id.clone(),
            channel_bot_id: uuid::Uuid::new_v4().to_string(),
            manager_bot_id: e.binding.manager_bot_id.clone(),
            observation_id: "public-handshake-observation".into(),
            execution_profile: PlatformExecutionProfile::PlatformOnboarding,
            generation: 1,
            enabled: false,
            guide: None,
            funding_owner_user_id: e.binding.sponsor_owner_user_id.clone(),
            configured_by: actor_id.clone(),
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        db.collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
            .insert_one(&profile)
            .await
            .unwrap();
        db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME).insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "provider":"telegram-new", "fields": {
                "manager_bot_id": &profile.manager_bot_id, "webhook_ready":"true", "observation_id": &profile.observation_id,
            }, "updated_by": &actor_id, "updated_at": bson::DateTime::now(),
        }).await.unwrap();
        db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).insert_one(doc! {
            "_id": &profile.channel_bot_id, "platform_profile_id": &profile.id, "platform_bot_id": &profile.manager_bot_id, "is_active":true,
        }).await.unwrap();
        db.collection::<Document>(WALLETS).insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "owner_id": &e.binding.sponsor_owner_user_id,
            "lago_customer_id":"public-customer", "lago_subscription_id":"public-subscription", "lago_wallet_id":"public-wallet",
            "plan_kind":"prepaid", "balance_credits": Credits::from_whole(20).decimal(), "reserved_credits": Credits::ZERO.decimal(),
            "collection_state":"good", "balance_synced_at":bson::DateTime::now(), "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
        }).await.unwrap();
        fence(&db, &e)
            .await
            .expect("live catalog and sponsor must authorize before any fault is injected");
        let runtime = ExternalRuntime {
            state: state.clone(),
            manifest: Arc::new(Mutex::new(e.manifest)),
            callback_url: reqwest::Url::parse(&format!(
                "{callback_base}/api/v1/public-nyxbot/models/{}/responses",
                service.id
            ))
            .unwrap(),
            mode: Arc::new(AtomicU8::new(mode)),
            callback_statuses: Arc::new(Mutex::new(vec![])),
            cleanup_authorities: Arc::new(Mutex::new(vec![])),
        };
        let readiness_mock = runtime.clone();
        let runtime_app = axum::Router::new()
            .route("/v1/public/profiles", post(discovery))
            .route(
                "/v1/public/readiness",
                post(move |request: Request<Body>| readiness(readiness_mock.clone(), request)),
            )
            .route("/v1/public/cleanup", post(cleanup_request))
            .with_state(runtime.clone());
        let runtime_task = tokio::spawn(async move {
            axum::serve(runtime_listener, runtime_app).await.unwrap();
        });
        Mock::given(method("POST")).and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(provider_status).set_body_json(json!({"model":provider_model,
                "output":[{"type":"message","content":[{"type":"output_text","text":"OK"}]}],
                "usage":{"input_tokens":20,"output_tokens":3,"input_tokens_details":{"cached_tokens":4}}})))
            .mount(&provider).await;
        let producers = crate::services::platform_manager_transport::compose(
            state.clone(),
            Arc::new(UnusedOwnerHook),
        );
        let actor = AuditActor {
            user_id: actor_id,
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        let fixture = Self {
            state,
            provider,
            _lago: lago,
            runtime,
            tasks: vec![callback_task, runtime_task],
            reviewed,
            actor,
            profile,
            producers,
        };
        fixture.consent().await;
        let (discovery, deployed) = discover(&fixture.state, &fixture.reviewed)
            .await
            .expect("signed discovery must succeed before readiness fault injection");
        assert!(deployed.is_some());
        assert_eq!(
            discovery.default_profile.as_deref(),
            Some(
                fixture
                    .reviewed
                    .public_policy
                    .as_ref()
                    .unwrap()
                    .nyxagent_profile
                    .as_str()
            )
        );
        assert_eq!(discovery.profiles.len(), 1);
        fixture
    }

    async fn consent(&self) {
        let mut session = self.state.db.client().start_session().await.unwrap();
        session
            .start_transaction()
            .and_run2(async |session| {
                tx::transaction_result(
                    record_probe_consent(
                        &self.state.db,
                        session,
                        &self.actor.user_id,
                        &self.reviewed,
                    )
                    .await,
                )
            })
            .await
            .unwrap();
    }
    async fn probe(&self) -> AppResult<PublicReadiness> {
        self.consent().await;
        self.producers
            .public
            .probe_public_readiness(&self.state.db, &self.reviewed)
            .await
    }
    async fn reset_probe_rate(&self) {
        // Advance only the test's retry window, without waiting 30 seconds.
        self.state
            .db
            .collection::<Document>(crate::models::coordination::RATE_WINDOW_COLLECTION_NAME)
            .delete_many(doc! {})
            .await
            .unwrap();
    }
    async fn wallet(&self) -> BillingWallet {
        self.state
            .db
            .collection::<BillingWallet>(WALLETS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
    }
    async fn meters(&self) -> Vec<UsageMeterRow> {
        self.state
            .db
            .collection::<UsageMeterRow>(METERS)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap()
    }
    async fn assert_probe_prepared(&self) {
        assert_eq!(
            self.state
                .db
                .collection::<PublicProbe>(PROBES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1,
            "negative readiness case must reach real probe preparation"
        );
    }
    async fn expire_and_recover(&self) {
        self.state
            .db
            .collection::<Document>(PROBES)
            .update_many(
                doc! {"state":{"$in":["prepared","funded"]}},
                doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        sweep_probes(&self.state, 100).await.unwrap();
    }
    async fn activate(&self) -> PlatformBotRuntimeControls {
        let validated = match policy::assess(&self.state.db, &self.reviewed)
            .await
            .unwrap()
        {
            PublicReadiness::Ready(proof) => Some(proof.policy_digest),
            PublicReadiness::Unavailable(_) => None,
        };
        self.apply(validated).await.unwrap()
    }
    async fn apply(&self, validated: Option<String>) -> AppResult<PlatformBotRuntimeControls> {
        let current = controls::get(&self.state.db, &self.profile.id)
            .await
            .unwrap();
        let selection = self.reviewed.public_policy.as_ref().unwrap();
        controls::update(
            &self.state.db,
            &self.actor,
            b"local-public-audit-key",
            &self.profile.id,
            &controls::RuntimeChange {
                public_deployment_digest: validated,
                initialize: current.is_none(),
                expected_profile_generation: current.is_none().then_some(1),
                expected_manager_authority_epoch: current
                    .as_ref()
                    .map_or(0, |c| c.manager_authority_epoch),
                expected_personal_control_revision: current
                    .as_ref()
                    .map_or(0, |c| c.personal_control_revision),
                expected_public_policy_revision: current
                    .as_ref()
                    .map_or(0, |c| c.public_policy_revision),
                personal_enabled: Some(false),
                public_enabled: Some(true),
                emergency_stopped: None,
                public_policy: Some(Some(
                    crate::models::platform_bot_runtime_controls::PublicRuntimePolicy {
                        sponsor_owner_user_id: selection.sponsor_owner_user_id.clone(),
                        nyxagent_profile: selection.nyxagent_profile.clone(),
                        total_daily_credits: selection.total_daily_credits,
                        sender_daily_credits: selection.sender_daily_credits,
                    },
                )),
            },
            &controls::ReadinessAdapter(self.producers.public.as_ref()),
        )
        .await
    }
    async fn ingress(&self, sender: i64) -> PublicIngress {
        let ingress = PublicIngress {
            manager: self.reviewed.manager.clone(),
            public_policy_revision: self.reviewed.public_policy_revision,
            sender_id: sender,
            chat_id: sender,
            ingress: IngressClaim {
                receipt_id: uuid::Uuid::new_v4().to_string(),
                claim_id: uuid::Uuid::new_v4().to_string(),
            },
        };
        let now = Utc::now();
        self.state
            .db
            .collection::<PlatformManagerIngress>(
                crate::models::platform_manager_ingress::COLLECTION_NAME,
            )
            .insert_one(PlatformManagerIngress {
                id: ingress.ingress.receipt_id.clone(),
                claim_id: ingress.ingress.claim_id.clone(),
                profile_id: self.profile.id.clone(),
                manager_bot_id: self.profile.manager_bot_id.clone(),
                telegram_bot_id: ingress.manager.telegram_bot_id,
                manager_authority_epoch: ingress.manager.manager_authority_epoch,
                observation_id: self.profile.observation_id.clone(),
                sender_id: sender,
                chat_id: sender,
                update_id: now.timestamp_millis(),
                message_id: 1,
                original_private_command: false,
                audience: IngressAudience::Public {
                    public_policy_revision: ingress.public_policy_revision,
                },
                phase: IngressPhase::Admitted,
                execution: None,
                decision: None,
                reply_id: None,
                delivery_recovery_queued: false,
                interrupted_reason: None,
                effect_revision: 0,
                admitted_at: now,
                start_before: now + Duration::minutes(2),
                settled_at: None,
            })
            .await
            .unwrap();
        ingress
    }
}

#[tokio::test]
async fn signed_http_probe_concurrency_funds_once_settles_and_persists_ready() {
    let fixture = Handshake::new("public_handshake", REPLAY, 200, "review-fixture-model").await;
    let PublicReadiness::Ready(proof) = fixture.probe().await.unwrap() else {
        panic!("local handshake must produce persisted Ready");
    };
    assert_eq!(
        fixture.provider.received_requests().await.unwrap().len(),
        1,
        "replay/concurrency must have one external provider effect"
    );
    let statuses = fixture.runtime.callback_statuses.lock().await;
    assert_eq!(statuses.iter().filter(|s| **s == 200).count(), 1);
    assert_eq!(statuses.len(), 3);
    drop(statuses);
    let meters = fixture.meters().await;
    assert_eq!(
        meters.len(),
        5,
        "live pricing shape has five component meters"
    );
    let actual = meters
        .iter()
        .try_fold(Credits::ZERO, |total, row| {
            assert!(row.forwarded && row.released);
            assert!(row.api_key_id.is_none());
            let funding = row.funding.as_ref().unwrap();
            assert!(funding.settled);
            total.checked_add(funding.total_charge.unwrap())
        })
        .unwrap();
    assert_eq!(actual, "0.0001432".parse().unwrap()); // 16 uncached input + 4 cached + 3 output
    let wallet = fixture.wallet().await;
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    assert_eq!(wallet.pending_lago_debits, actual);
    let bucket = fixture
        .state
        .db
        .collection::<PublicBudgetBucket>(BUCKETS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bucket.held, Credits::ZERO);
    assert_eq!(bucket.spent, actual);
    assert!(
        fixture
            .state
            .db
            .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
            .find_one(doc! {"_id":&proof.proof_record_id})
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        policy::activation(&fixture.state.db, &fixture.reviewed)
            .await
            .is_err(),
        "validation alone must not activate public execution"
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn rejected_or_mismatched_model_never_persists_ready() {
    for (status, model) in [(400, "review-fixture-model"), (200, "different-model")] {
        let fixture =
            Handshake::new("public_handshake_model_rejected", NORMAL, status, model).await;
        assert!(fixture.probe().await.is_err());
        assert_eq!(fixture.provider.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            fixture
                .state
                .db
                .collection::<Document>(DEPLOYMENT_RECORDS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(fixture.wallet().await.reserved_credits, Credits::ZERO);
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn expired_consent_and_changed_signed_body_prevent_funding_and_provider_effects() {
    for mode in [
        EXPIRE_CONSENT,
        WRONG_BODY,
        CATALOG_DRIFT,
        SPONSOR_REVOKED,
        CAP_DAY_ENDED,
    ] {
        let fixture = Handshake::new(
            "public_handshake_authority",
            mode,
            200,
            "review-fixture-model",
        )
        .await;
        let grant_id = uuid::Uuid::new_v4().to_string();
        fixture.state.db.collection::<Document>(crate::models::credit_grant::COLLECTION_NAME).insert_one(doc! {
            "_id": &grant_id, "batch_id": "local-authority-regression", "recipient_user_id": &fixture.reviewed.public_policy.as_ref().unwrap().sponsor_owner_user_id,
            "target_kind":"selected_users", "amount_credits": 1_i64, "amount": Credits::from_whole(1).decimal(),
            "remaining": Credits::from_whole(1).decimal(), "reserved": Credits::ZERO.decimal(),
            "scope":{"all_services":true,"service_ids":[],"service_slugs":[]}, "status":"active", "issued_ledgered_at":bson::DateTime::now(),
            "terminal_amount":Credits::ZERO.decimal(), "granted_by":&fixture.actor.user_id, "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
        }).await.unwrap();
        assert!(fixture.probe().await.is_err());
        fixture.assert_probe_prepared().await;
        let grant = fixture
            .state
            .db
            .collection::<crate::models::credit_grant::CreditGrant>(
                crate::models::credit_grant::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": &grant_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(grant.reserved, Credits::ZERO);
        assert_eq!(grant.remaining, Credits::from_whole(1));
        assert_eq!(fixture.provider.received_requests().await.unwrap().len(), 0);
        assert!(fixture.meters().await.is_empty());
        let wallet = fixture.wallet().await;
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
        fixture.expire_and_recover().await;
        let bucket = fixture
            .state
            .db
            .collection::<PublicBudgetBucket>(BUCKETS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bucket.held, Credits::ZERO);
        assert_eq!(bucket.spent, Credits::ZERO);
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn failed_readiness_recovery_releases_real_funded_and_pre_funding_holds() {
    for mode in [NO_CALLBACK, FUND_ONLY] {
        let fixture = Handshake::new(
            "public_handshake_release",
            mode,
            200,
            "review-fixture-model",
        )
        .await;
        assert!(fixture.probe().await.is_err());
        fixture.assert_probe_prepared().await;
        if mode == FUND_ONLY {
            assert!(fixture.wallet().await.reserved_credits > Credits::ZERO);
            assert_eq!(fixture.meters().await.len(), 5);
        }
        fixture.expire_and_recover().await;
        assert_eq!(fixture.wallet().await.reserved_credits, Credits::ZERO);
        assert_eq!(fixture.wallet().await.pending_lago_debits, Credits::ZERO);
        assert!(
            fixture
                .meters()
                .await
                .iter()
                .all(|row| row.released && !row.forwarded)
        );
        assert_eq!(fixture.provider.received_requests().await.unwrap().len(), 0);
        let bucket = fixture
            .state
            .db
            .collection::<PublicBudgetBucket>(BUCKETS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bucket.held, Credits::ZERO);
        assert_eq!(bucket.spent, Credits::ZERO);
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn refresh_pins_original_proof_and_only_human_activation_adopts_upgrade() {
    let fixture = Handshake::new(
        "public_handshake_activation",
        NORMAL,
        200,
        "review-fixture-model",
    )
    .await;
    let enabled = fixture.activate().await;
    let reviewed = controls::policy(&enabled);
    let ingress = fixture.ingress(101).await;
    let original = fixture
        .producers
        .public
        .prepare_public_context(&fixture.state.db, &ingress)
        .await
        .unwrap();
    let prepared = credentials::producer(fixture.state.clone())
        .prepare(
            &fixture.state.db,
            &original,
            TurnInput {
                text: "What is NyxID?".into(),
            },
        )
        .await
        .unwrap();
    let key = fixture
        .state
        .db
        .collection::<crate::models::api_key::ApiKey>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id": &prepared.credential.api_key_id})
        .await
        .unwrap()
        .unwrap();
    // Same-contract refresh creates independent evidence without changing a live turn.
    fixture.reset_probe_rate().await;
    let PublicReadiness::Ready(refreshed) =
        probe(&fixture.state, &reviewed, Some(&original.policy_digest))
            .await
            .unwrap()
    else {
        panic!("refresh ready");
    };
    assert_ne!(refreshed.proof_record_id, original.proof_record_id);
    assert_eq!(refreshed.policy_digest, original.policy_digest);
    policy::verified_context(&fixture.state.db, &original)
        .await
        .unwrap();
    let callback_authority =
        crate::services::platform_public_tools::authenticate(&fixture.state.db, &key, None)
            .await
            .unwrap();
    assert_eq!(
        callback_authority.context().proof_record_id,
        original.proof_record_id
    );
    assert_eq!(
        callback_authority.context().turn.turn_id,
        original.turn.turn_id
    );
    // End the exact old turn without admission; sender keeps the same session.
    let mut session = fixture.state.db.client().start_session().await.unwrap();
    session
        .start_transaction()
        .and_run2(async |s| {
            tx::transaction_result(
                credentials::interrupt_turn_in_session(&fixture.state.db, s, &original, false)
                    .await,
            )
        })
        .await
        .unwrap();
    let next_ingress = fixture.ingress(101).await;
    let next = fixture
        .producers
        .public
        .prepare_public_context(&fixture.state.db, &next_ingress)
        .await
        .unwrap();
    assert_eq!(next.public_session_id, original.public_session_id);
    assert_eq!(next.proof_record_id, refreshed.proof_record_id);
    // A runtime build upgrade is rejected by automatic refresh before funding.
    assert!(refreshed.expires_at > Utc::now() + Duration::minutes(3));
    fixture.runtime.manifest.lock().await.runtime_digest = "9".repeat(64);
    fixture.reset_probe_rate().await;
    assert!(
        probe(&fixture.state, &reviewed, Some(&original.policy_digest))
            .await
            .is_err()
    );
    // Explicit human validation can prove the new build, but cannot replace activation.
    let PublicReadiness::Ready(upgraded) = fixture.probe().await.unwrap() else {
        panic!("upgrade validation");
    };
    assert_ne!(upgraded.policy_digest, original.policy_digest);
    assert_eq!(
        policy::activation(&fixture.state.db, &reviewed)
            .await
            .unwrap()
            .policy_digest,
        original.policy_digest
    );
    policy::verified_context(&fixture.state.db, &next)
        .await
        .unwrap();
    // The explicit enable/apply action adopts the validated digest and revokes old sessions atomically.
    let provider_effects = fixture.provider.received_requests().await.unwrap().len();
    assert!(matches!(
        fixture.apply(Some(original.policy_digest.clone())).await,
        Err(AppError::Conflict(message)) if message == "Validated deployment changed; validate again before applying"
    ));
    assert_eq!(
        policy::activation(&fixture.state.db, &reviewed)
            .await
            .unwrap()
            .policy_digest,
        original.policy_digest,
        "a mismatched reviewed digest must not adopt the new deployment"
    );
    let activated = fixture.activate().await;
    assert_eq!(
        fixture.provider.received_requests().await.unwrap().len(),
        provider_effects,
        "applying fresh validation must not issue a second paid probe"
    );
    assert_eq!(
        activated.public_policy_revision,
        enabled.public_policy_revision
    );
    let old = fixture
        .state
        .db
        .collection::<PlatformPublicSession>(SESSIONS)
        .find_one(doc! {"_id":&next.public_session_id})
        .await
        .unwrap()
        .unwrap();
    assert!(old.revoked_at.is_some());
    assert!(old.cleanup_record_id.is_some());
    assert!(
        policy::verified_context(&fixture.state.db, &next)
            .await
            .is_err()
    );
    let fresh_ingress = fixture.ingress(101).await;
    let fresh = fixture
        .producers
        .public
        .prepare_public_context(&fixture.state.db, &fresh_ingress)
        .await
        .unwrap();
    assert_ne!(fresh.public_session_id, next.public_session_id);
    assert_eq!(fresh.policy_digest, upgraded.policy_digest);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn near_expiry_apply_runs_a_fresh_producer_probe_and_fresh_apply_reuses_it() {
    let fixture = Handshake::new(
        "public_handshake_near_expiry",
        NORMAL,
        200,
        "review-fixture-model",
    )
    .await;
    let PublicReadiness::Ready(original) = fixture.probe().await.unwrap() else {
        panic!("initial local validation");
    };
    assert_eq!(fixture.provider.received_requests().await.unwrap().len(), 1);
    let mut record = fixture
        .state
        .db
        .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
        .find_one(doc! {"_id": &original.proof_record_id})
        .await
        .unwrap()
        .unwrap();
    let evidence = record.evidence.as_mut().unwrap();
    evidence.expires_at =
        bson::DateTime::from_chrono(Utc::now() + Duration::seconds(20)).to_chrono();
    evidence.evidence_digest = policy::evidence_digest(evidence).unwrap();
    fixture
        .state
        .db
        .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
        .replace_one(doc! {"_id": &record.id}, &record)
        .await
        .unwrap();
    let PublicReadiness::Ready(near_expiry) = policy::assess(&fixture.state.db, &fixture.reviewed)
        .await
        .unwrap()
    else {
        panic!("proof remains valid, but inadequate for a complete turn");
    };
    assert!(
        near_expiry.expires_at
            < Utc::now() + Duration::seconds(i64::from(near_expiry.limits.max_duration_secs) + 5)
    );
    fixture.reset_probe_rate().await;
    fixture.activate().await;
    assert_eq!(fixture.provider.received_requests().await.unwrap().len(), 2);
    let PublicReadiness::Ready(fresh) = policy::assess(&fixture.state.db, &fixture.reviewed)
        .await
        .unwrap()
    else {
        panic!("fresh producer probe must persist Ready");
    };
    assert_ne!(fresh.proof_record_id, original.proof_record_id);
    assert_eq!(fresh.policy_digest, original.policy_digest);
    assert!(fresh.expires_at > Utc::now() + Duration::minutes(3));
    // The same apply path can now reuse the new adequate proof, with the probe
    // rate window still closed. A duplicate paid probe would fail this action.
    fixture.activate().await;
    assert_eq!(fixture.provider.received_requests().await.unwrap().len(), 2);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_ingress_refresh_shares_one_durable_probe_and_failed_refresh_stays_unavailable()
{
    for mode in [SLOW_CALLBACK, NO_CALLBACK] {
        let fixture = Handshake::new(
            "public_handshake_refresh_concurrency",
            NORMAL,
            200,
            "review-fixture-model",
        )
        .await;
        fixture.activate().await;
        let mut record = fixture
            .state
            .db
            .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        let evidence = record.evidence.as_mut().unwrap();
        evidence.expires_at = bson::DateTime::from_chrono(
            Utc::now() + Duration::seconds(if mode == SLOW_CALLBACK { 20 } else { -1 }),
        )
        .to_chrono();
        evidence.evidence_digest = policy::evidence_digest(evidence).unwrap();
        fixture
            .state
            .db
            .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
            .replace_one(doc! {"_id": &record.id}, &record)
            .await
            .unwrap();
        let activation = policy::activation(&fixture.state.db, &fixture.reviewed)
            .await
            .unwrap();
        let mut last_assessed = None;
        for _ in 0..20 {
            assert!(
                !refreshed_proof_has_lifetime(
                    &fixture.state.db,
                    &fixture.reviewed,
                    &activation,
                    &mut last_assessed,
                )
                .await
                .unwrap()
            );
        }
        let untouched = fixture
            .state
            .db
            .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
            .find_one(doc! {"_id": &record.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            untouched.effect_revision, record.effect_revision,
            "waiting for a near-expiry proof must not run write-fenced assessments"
        );
        assert!(last_assessed.is_none());
        fixture.runtime.mode.store(mode, Ordering::SeqCst);
        fixture.reset_probe_rate().await;
        let first_ingress = fixture.ingress(201).await;
        let second_ingress = fixture.ingress(202).await;
        if mode == NO_CALLBACK {
            fixture.state.db.collection::<Document>(
                crate::models::platform_manager_ingress::COLLECTION_NAME,
            ).update_many(doc! {"_id": {"$in": [&first_ingress.ingress.receipt_id, &second_ingress.ingress.receipt_id]}},
                doc! {"$set": {"start_before": bson::DateTime::from_chrono(Utc::now() + Duration::seconds(5))}})
                .await.unwrap();
        }
        let (first, second) = tokio::join!(
            fixture
                .producers
                .public
                .prepare_public_context(&fixture.state.db, &first_ingress),
            fixture
                .producers
                .public
                .prepare_public_context(&fixture.state.db, &second_ingress),
        );
        let probes = fixture
            .state
            .db
            .collection::<PublicProbe>(PROBES)
            .count_documents(doc! {})
            .await
            .unwrap();
        assert_eq!(
            probes, 2,
            "initial validation plus exactly one shared refresh"
        );
        if mode == SLOW_CALLBACK {
            let first = first.unwrap();
            let second = second.unwrap();
            assert_eq!(first.proof_record_id, second.proof_record_id);
            assert_ne!(first.proof_record_id, record.id);
            assert_ne!(first.public_session_id, second.public_session_id);
            assert!(
                refreshed_proof_has_lifetime(
                    &fixture.state.db,
                    &fixture.reviewed,
                    &activation,
                    &mut last_assessed,
                )
                .await
                .unwrap()
            );
            let checked = fixture
                .state
                .db
                .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
                .find_one(doc! {"_id": &first.proof_record_id})
                .await
                .unwrap()
                .unwrap();
            for _ in 0..5 {
                assert!(
                    !refreshed_proof_has_lifetime(
                        &fixture.state.db,
                        &fixture.reviewed,
                        &activation,
                        &mut last_assessed,
                    )
                    .await
                    .unwrap()
                );
            }
            let unchanged = fixture
                .state
                .db
                .collection::<PublicDeploymentRecord>(DEPLOYMENT_RECORDS)
                .find_one(doc! {"_id": &first.proof_record_id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                checked.effect_revision, unchanged.effect_revision,
                "a waiter must authoritatively assess each fresh candidate only once"
            );
            assert_eq!(fixture.provider.received_requests().await.unwrap().len(), 2);
        } else {
            assert!(first.is_err() && second.is_err());
            assert_eq!(fixture.provider.received_requests().await.unwrap().len(), 1);
            assert_eq!(
                fixture
                    .state
                    .db
                    .collection::<PlatformPublicSession>(SESSIONS)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            fixture.expire_and_recover().await;
        }
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn failed_dispatched_settlement_preserves_original_unknown_handle_cleanup_deadline() {
    let fixture = Handshake::new(
        "public_handshake_cleanup_deadline",
        NORMAL,
        200,
        "review-fixture-model",
    )
    .await;
    fixture.activate().await;
    let ingress = fixture.ingress(103).await;
    let context = fixture
        .producers
        .public
        .prepare_public_context(&fixture.state.db, &ingress)
        .await
        .unwrap();
    let runtime = credentials::producer(fixture.state.clone());
    let prepared = runtime
        .prepare(
            &fixture.state.db,
            &context,
            TurnInput {
                text: "Guide me".into(),
            },
        )
        .await
        .unwrap();
    let original = fixture
        .state
        .db
        .collection::<PlatformPublicSession>(SESSIONS)
        .find_one(doc! {"_id": &context.public_session_id})
        .await
        .unwrap()
        .unwrap();
    let signed = execution_assertion(
        &fixture.state,
        &context,
        &prepared.credential.raw_key,
        b"delayed execution",
    )
    .await
    .unwrap();
    let execution =
        verify_assertion(&fixture.state, &signed, "execution", b"delayed execution").unwrap();
    assert_eq!(
        execution["session_expires_at"],
        original.expires_at.to_rfc3339()
    );
    let turn = fixture
        .state
        .db
        .collection::<crate::models::platform_public_turn::PlatformPublicTurn>(
            crate::models::platform_public_turn::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": &context.turn.turn_id})
        .await
        .unwrap()
        .unwrap();
    let mut session = fixture.state.db.client().start_session().await.unwrap();
    session
        .start_transaction()
        .and_run2(async |session| {
            tx::transaction_result(
                proxy::mark_outer_attempt_in_session(
                    &fixture.state.db,
                    session,
                    &context,
                    &prepared.credential.api_key_id,
                    &turn.nyxagent_service_id,
                    &turn.nyxagent_catalog_digest,
                )
                .await,
            )
        })
        .await
        .unwrap();
    let failure = crate::services::assistant_nyxagent::TurnResult {
        text: String::new(),
        session_id: None,
        response_id: None,
        error: Some(crate::services::assistant_nyxagent::TurnError::internal(
            "assistant_unavailable",
            "public_execution",
        )),
    };
    let SettledExecution::Saved(_) = runtime
        .settle(
            &fixture.state.db,
            &context,
            &uuid::Uuid::new_v4().to_string(),
            &failure,
        )
        .await
        .unwrap();
    let revoked = fixture
        .state
        .db
        .collection::<PlatformPublicSession>(SESSIONS)
        .find_one(doc! {"_id": &original.id})
        .await
        .unwrap()
        .unwrap();
    assert!(revoked.revoked_at.is_some());
    assert!(revoked.upstream_may_exist && revoked.upstream_binding_ciphertext.is_empty());
    assert_eq!(revoked.expires_at, original.expires_at);
    assert!(cleanup(&fixture.state, &revoked).await.unwrap());
    let authorities = fixture.runtime.cleanup_authorities.lock().await;
    assert_eq!(authorities.len(), 1);
    assert_eq!(
        authorities[0]["session_expires_at"],
        execution["session_expires_at"]
    );
    assert_eq!(authorities[0]["public_session_id"], original.id);
    drop(authorities);
    let next_ingress = fixture.ingress(103).await;
    let fresh = fixture
        .producers
        .public
        .prepare_public_context(&fixture.state.db, &next_ingress)
        .await
        .unwrap();
    assert_ne!(fresh.public_session_id, original.id);
    fixture.state.db.drop().await.unwrap();
}
