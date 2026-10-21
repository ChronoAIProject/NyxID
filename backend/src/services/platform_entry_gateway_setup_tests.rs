use super::*;
use crate::services::platform_entry_admin_service::{
    self as admin, NativeConversionOutcome, NativeTransportChoice,
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::get,
};
use futures::FutureExt;
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone)]
struct GatewayHarness {
    state: AppState,
    base: String,
    records: Arc<Mutex<HashMap<String, Value>>>,
    commands: Arc<Mutex<Vec<Value>>>,
    posts: Arc<AtomicUsize>,
    binding_puts: Arc<AtomicUsize>,
    mode: Arc<AtomicUsize>,
    received: Arc<Mutex<Vec<Zeroizing<Vec<u8>>>>>,
    validation_failures: Arc<AtomicUsize>,
    completed_responses: Arc<AtomicUsize>,
}

impl GatewayHarness {
    async fn response(&self, request: Value) -> Value {
        self.posts.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request["sources"][0]["admission"]["groups"], "deny");
        assert_eq!(request["sources"][0]["trust_normalized"], true);
        assert_eq!(request["reply"]["auto_final"], false);
        let row = self
            .state
            .db
            .collection::<PlatformEntryGateway>(GATEWAYS)
            .find_one(doc! {"_id":request["record_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.setup_state, "dispatched");
        for (field, id) in [
            ("agent_key", &row.provider_api_key_id),
            ("channel_key", &row.route_api_key_id),
        ] {
            let key = self
                .state
                .db
                .collection::<ApiKey>(KEYS)
                .find_one(doc! {"_id":id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                key.purpose,
                crate::models::api_key::ApiKeyPurpose::SharedEntryTransport
            );
            assert!(key.scopes.is_empty());
            assert!(!key.allow_all_services);
            assert_eq!(
                format!("sha256:{}", key.key_hash),
                sha256(request["credentials"][field].as_str().unwrap().as_bytes())
            );
        }
        let profile = canonical_json(request["profile"].clone());
        if self.mode.load(Ordering::SeqCst) != 2 {
            let uri = format!("/bindings/{}", row.binding_id.as_ref().unwrap());
            let body = json!({"agent_key":request["credentials"]["agent_key"],"profile":request["profile"],"profile_digest":sha256(profile.as_bytes())});
            let request = Request::builder()
                .method("PUT")
                .uri(uri)
                .header(
                    "authorization",
                    format!(
                        "Bearer {}",
                        request["credentials"]["agent_key"].as_str().unwrap()
                    ),
                )
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap();
            let result = Router::new()
                .route("/bindings/{binding}", put(nyxbot::put_binding))
                .with_state(self.state.clone())
                .oneshot(request)
                .await
                .unwrap();
            assert_eq!(
                result.status(),
                StatusCode::OK,
                "actual provider binding callback"
            );
            self.binding_puts.fetch_add(1, Ordering::SeqCst);
        }
        // Begin with an actual consumer-produced sealed 264-byte ready record.
        // Pure real-CMA parser/definition agreement is separately tested; here
        // dynamic producer credentials drive the actual backend callback exchange.
        let mut body: Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/platform-entry-cma/discord-ready-record.json"
        ))
        .unwrap();
        let remote = body["channel_id"].as_str().unwrap().to_owned();
        body["record_id"] = row.id.clone().into();
        body["endpoints"]["nyxid_callback_url"] =
            format!("{}/callbacks/{remote}", self.base).into();
        body["provider"]["binding_id"] = row.binding_id.clone().unwrap().into();
        let definition = &mut body["definition"];
        definition["record_id"] = row.id.clone().into();
        definition["name"] = request["name"].clone();
        definition["owner"] = request["profile"]["metadata"]["owner"].clone();
        definition["profile"] = request["profile"].clone();
        definition["profile_digest"] = sha256(profile.as_bytes()).into();
        definition["sources"] = request["sources"].clone();
        definition["sources"][0]["jwks_url"] = format!(
            "{}/.well-known/jwks.json",
            request["sources"][0]["issuer"].as_str().unwrap()
        )
        .into();
        definition["partition"] = request["partition"].clone();
        definition["reply"] = request["reply"].clone();
        definition["provider"]["binding_id"] = row.binding_id.clone().unwrap().into();
        definition["secrets"]["agent_key_hash"] = sha256(
            request["credentials"]["agent_key"]
                .as_str()
                .unwrap()
                .as_bytes(),
        )
        .into();
        definition["creation"]["fingerprint"] = row.creation_fingerprint.clone().into();
        definition["creation"]["ready"] = (self.mode.load(Ordering::SeqCst) != 2).into();
        self.records
            .lock()
            .unwrap()
            .insert(row.id.clone(), body.clone());
        if self.mode.load(Ordering::SeqCst) == 3 {
            self.state
                .db
                .collection::<Document>(crate::models::user::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&row.configured_by},
                    doc! {"$set":{"is_active":false}},
                )
                .await
                .unwrap();
        }
        body
    }
}
async fn create(
    State(h): State<GatewayHarness>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    match h.mode.load(Ordering::SeqCst) {
        4 => return axum::response::Redirect::temporary("/credential-leak").into_response(),
        5 => return "x".repeat(64 * 1024 + 1).into_response(),
        6 => {
            let chunks = futures::stream::iter([
                Ok::<_, std::io::Error>(Bytes::from(vec![b'x'; 64 * 1024])),
                Ok(Bytes::from_static(b"x")),
            ]);
            return axum::body::Body::from_stream(chunks).into_response();
        }
        _ => {}
    }
    match std::panic::AssertUnwindSafe(create_checked(State(h.clone()), headers, body))
        .catch_unwind()
        .await
    {
        Ok(response) => {
            h.completed_responses.fetch_add(1, Ordering::SeqCst);
            response.into_response()
        }
        Err(_) => {
            h.validation_failures.fetch_add(1, Ordering::SeqCst);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"fixture validation failed"})),
            )
                .into_response()
        }
    }
}
async fn create_checked(
    State(h): State<GatewayHarness>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let request: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        headers["idempotency-key"].to_str().unwrap(),
        request["record_id"].as_str().unwrap()
    );
    assert_eq!(headers["content-type"], "application/json");
    let bearer = headers["authorization"]
        .to_str()
        .unwrap()
        .strip_prefix("Bearer ")
        .unwrap();
    let creator =
        crate::crypto::jwt::verify_token(&h.state.jwt_keys, &h.state.config, bearer).unwrap();
    assert_eq!(
        creator.sub,
        request["profile"]["metadata"]["owner"]["subject"]
    );
    assert_eq!(creator.scope, crate::mw::auth::ACCOUNT_READ_SCOPE);
    assert_eq!(creator.delegated, Some(true));
    assert!(creator.exp - creator.iat <= 120);
    h.received
        .lock()
        .unwrap()
        .push(Zeroizing::new(body.to_vec()));
    let response = h.response(request).await;
    (
        if h.mode.load(Ordering::SeqCst) == 1 {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::OK
        },
        Json(response),
    )
}
async fn retained(
    State(h): State<GatewayHarness>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    h.records
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .map_or((StatusCode::NOT_FOUND, Json(json!({}))), |value| {
            (StatusCode::OK, Json(value))
        })
}
async fn application(State(h): State<GatewayHarness>) -> Json<Value> {
    let key = ed25519_dalek::SigningKey::from_bytes(&[37; 32]);
    let _ = &h;
    Json(
        json!({"id":"100","bot":{"id":"200"},"verify_key":hex::encode(key.verifying_key().as_bytes())}),
    )
}
async fn commands(State(h): State<GatewayHarness>) -> Json<Value> {
    Json(json!(h.commands.lock().unwrap().clone()))
}
async fn install(State(h): State<GatewayHarness>, Json(mut command): Json<Value>) -> Json<Value> {
    command["id"] = "300".into();
    command["application_id"] = "100".into();
    h.commands.lock().unwrap().push(command.clone());
    Json(command)
}

pub(crate) struct SetupFixture {
    harness: GatewayHarness,
    bot: crate::models::channel_bot::ChannelBot,
    funding: String,
    transport: Arc<NativeTestTransport>,
    shutdown: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<()>,
}
impl SetupFixture {
    pub(crate) fn state(&self) -> &AppState {
        &self.harness.state
    }
    pub(crate) fn actor(&self) -> &str {
        &self.bot.user_id
    }
    pub(crate) fn funding(&self) -> &str {
        &self.funding
    }
    pub(crate) async fn new(mode: usize) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (mut state, bot, old) =
            crate::services::platform_native_reply_service::tests::discord_fixture(&base).await;
        state.config.jwt_issuer = "https://nyxid.example".into();
        for collection in [
            crate::models::platform_bot_config::COLLECTION_NAME,
            crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
        ] {
            state
                .db
                .collection::<Document>(collection)
                .delete_many(
                    doc! {"$or":[{"_id":&old.profile_id},{"platform_config_id":&old.profile_id}]},
                )
                .await
                .unwrap();
        }
        state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&bot.id},
                doc! {"$unset":{"platform_profile_id":"","platform_entry_version":""}},
            )
            .await
            .unwrap();
        crate::services::platform_bot_config_service::ensure_indexes(&state.db)
            .await
            .unwrap();
        crate::services::role_service::seed_system_roles(&state.db)
            .await
            .unwrap();
        let role = crate::services::role_service::get_platform_role_ids(&state.db)
            .await
            .unwrap()
            .admin;
        state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id":&bot.user_id}, doc! {"$set":{"role_ids":[role]}})
            .await
            .unwrap();
        let funding = crate::services::org_service::create_org_user(
            &state.db,
            "Native gateway funding",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
        crate::services::org_service::create_membership(
            &state.db,
            &funding,
            &bot.user_id,
            crate::models::org_membership::OrgRole::Admin,
            crate::models::org_membership::MemberScopeSource::Inherit,
            None,
        )
        .await
        .unwrap();
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = "cmaeg".into();
        service.base_url = base.clone();
        state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .insert_one(service)
            .await
            .unwrap();
        let harness = GatewayHarness {
            state,
            base: base.clone(),
            records: Arc::default(),
            commands: Arc::default(),
            posts: Arc::new(AtomicUsize::new(0)),
            binding_puts: Arc::new(AtomicUsize::new(0)),
            mode: Arc::new(AtomicUsize::new(mode)),
            received: Arc::default(),
            validation_failures: Arc::default(),
            completed_responses: Arc::default(),
        };
        let router = Router::new()
            .route("/channels", post(create))
            .route(
                "/credential-leak",
                post(|State(h): State<GatewayHarness>| async move {
                    h.posts.fetch_add(1, Ordering::SeqCst);
                    StatusCode::NO_CONTENT
                }),
            )
            .route("/channel-records/{id}", get(retained))
            .route("/oauth2/applications/@me", get(application))
            .route("/applications/100/commands", get(commands).post(install))
            .with_state(harness.clone());
        let (shutdown, rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .unwrap();
        });
        let transport = Arc::new(NativeTestTransport {
            adapter: Arc::new(
                crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                    &base,
                ),
            ),
            discord_base: base,
        });
        Self {
            harness,
            bot,
            funding,
            transport,
            shutdown,
            server,
        }
    }
    pub(crate) async fn convert(
        &self,
        recovery: Option<&GatewayRecoveryReview>,
    ) -> AppResult<NativeConversionOutcome> {
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            crate::services::platform_entry_admin_service::PROVIDER_BASE.scope(
                self.harness.base.clone(),
                with_transport_for_test(
                    self.transport.clone(),
                    admin::convert(
                        &self.harness.state,
                        &self.bot.user_id,
                        &self.bot.id,
                        &self.funding,
                        NativeTransportChoice::Gateway,
                        None,
                        recovery,
                    ),
                ),
            ),
        )
        .await
        .expect("setup bounded lifetime");
        assert_eq!(
            self.harness.validation_failures.load(Ordering::SeqCst),
            0,
            "gateway request fixture validation failed"
        );
        result
    }
    async fn admission(
        &self,
        profile: &crate::models::platform_bot_config::PlatformBotConfig,
        epoch: i64,
        event: u64,
    ) -> crate::services::platform_entry_receipts::NativeAdmission {
        let configuration = profile.native_entry.as_ref().unwrap();
        let bot = crate::services::channel_bot_service::get_bot(&self.state().db, &self.bot.id)
            .await
            .unwrap();
        let mut payload = native::discord_message("status", None);
        let millis = Utc::now().timestamp_millis() - 1_420_070_400_000;
        payload["id"] = (((millis as u64) << 22) | event).to_string().into();
        let body = serde_json::to_vec(&payload).unwrap();
        let signing = ed25519_dalek::SigningKey::from_bytes(&[37; 32]);
        let headers = native::discord_headers(&signing, &body, Utc::now().timestamp());
        let batch = crate::services::platform_entry_native::prepare(
            &self.state().db,
            &self.state().encryption_keys,
            self.transport.adapter.as_ref(),
            &bot,
            &headers,
            &body,
            configuration,
        )
        .await
        .unwrap();
        let mut session = self.state().db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let admission = crate::services::platform_entry_receipts::record_in_session(
            &self.state().db,
            &mut session,
            &self.state().encryption_keys,
            self.state().auth_device_hmac_key.as_slice(),
            crate::services::platform_entry_receipts::NativeReceiptInput {
                profile_id: &profile.id,
                epoch,
                configuration,
                batch: &batch,
                index: 0,
                relay_message_id: &uuid::Uuid::new_v4().to_string(),
            },
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        admission
    }
    pub(crate) async fn close(self) {
        let _ = self.shutdown.send(());
        tokio::time::timeout(Duration::from_secs(20), self.server)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            self.harness.validation_failures.load(Ordering::SeqCst),
            0,
            "gateway request fixture validation failed"
        );
        self.harness.state.db.drop().await.unwrap();
    }
}

#[test]
fn reviewed_gateway_update_tolerates_real_command_and_key_touches_and_recovers_selected_transport()
{
    crate::test_utils::run_native_journey_test(
        "reviewed_gateway_update_tolerates_real_command_and_key_touches_and_recovers_selected_transport",
        || async {
            use crate::services::platform_entry_admin_service::tests::{
                enable_personal_as, execute_command,
            };
            use crate::services::platform_manager_command_service::PersonalCommandKind;
            let f = SetupFixture::new(0).await;
            let NativeConversionOutcome::Converted(first) = f.convert(None).await.unwrap() else {
                panic!("initial gateway")
            };
            let controls = enable_personal_as(f.state(), f.actor(), &first).await;
            crate::services::platform_manager_ingress_service::ensure_indexes(&f.state().db)
                .await
                .unwrap();
            let old_admission = f
                .admission(&first, controls.manager_authority_epoch, 1)
                .await;
            let old = f
                .state()
                .db
                .collection::<PlatformEntryGateway>(GATEWAYS)
                .find_one(doc! {"profile_id":&first.id,"state":"active"})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(old.setup_state, "observed");
            assert!(
                old.setup_dispatch_deadline.unwrap() + chrono::Duration::seconds(120) > Utc::now()
            );
            let review = admin::review_lifecycle(f.state(), f.actor(), &f.bot.id)
                .await
                .unwrap();
            assert!(
                execute_command(f.state(), &old_admission, PersonalCommandKind::Status)
                    .await
                    .unwrap()
                    .is_some()
            );
            let raw = {
                let requests = f.harness.received.lock().unwrap();
                let request: Value = serde_json::from_slice(&requests[0]).unwrap();
                Zeroizing::new(
                    request["credentials"]["agent_key"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                )
            };
            let (touch1, touch2) = tokio::join!(
                crate::services::key_service::validate_api_key(&f.state().db, raw.as_str()),
                crate::services::key_service::validate_api_key(&f.state().db, raw.as_str()),
            );
            touch1.unwrap();
            touch2.unwrap();
            assert_eq!(
                admin::review_lifecycle(f.state(), f.actor(), &f.bot.id)
                    .await
                    .unwrap(),
                review
            );
            // An observed setup with a recent deadline is complete and may be updated.
            with_transport_for_test(
                f.transport.clone(),
                admin::reserve_reviewed_update(
                    f.state(),
                    f.actor(),
                    f.funding(),
                    NativeTransportChoice::Gateway,
                    &review,
                    None,
                    None,
                ),
            )
            .await
            .unwrap();
            let stopped = crate::services::platform_bot_runtime_control_service::get(
                &f.state().db,
                &first.id,
            )
            .await
            .unwrap()
            .unwrap();
            assert!(!stopped.personal_enabled && !stopped.public_enabled);
            assert!(stopped.manager_authority_epoch > controls.manager_authority_epoch);
            assert!(
                f.state()
                    .db
                    .collection::<PlatformEntryGateway>(GATEWAYS)
                    .find_one(doc! {"_id":&old.id,"state":"retired"})
                    .await
                    .unwrap()
                    .is_some()
            );
            for id in [&old.provider_api_key_id, &old.route_api_key_id] {
                let key = f
                    .state()
                    .db
                    .collection::<ApiKey>(KEYS)
                    .find_one(doc! {"_id":id})
                    .await
                    .unwrap()
                    .unwrap();
                assert!(!key.is_active);
            }
            let NativeConversionOutcome::Converted(updated) = f.convert(None).await.unwrap() else {
                panic!("replacement gateway")
            };
            assert_eq!(updated.id, first.id);
            assert!(updated.generation > first.generation);
            let replacement = f
                .state()
                .db
                .collection::<PlatformEntryGateway>(GATEWAYS)
                .find_one(doc! {"profile_id":&first.id,"state":"active"})
                .await
                .unwrap()
                .unwrap();
            assert_ne!(replacement.id, old.id);
            assert_ne!(replacement.provider_api_key_id, old.provider_api_key_id);
            assert_ne!(replacement.route_api_key_id, old.route_api_key_id);
            assert!(matches!(
                &replacement.configuration.transport,
                EntryTransport::NyxagentGateway { .. }
            ));
            let enabled = enable_personal_as(f.state(), f.actor(), &updated).await;
            let update = old_admission.verified_update();
            let mut session = f.state().db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            assert!(
                crate::services::platform_bot_runtime_control_service::fence_manager_in_session(
                    &f.state().db,
                    &mut session,
                    &first.id,
                    &update.manager_bot_id,
                    &update.entry,
                    Some(&update.observation_id),
                )
                .await
                .unwrap()
                .is_none()
            );
            session.abort_transaction().await.unwrap();
            let fresh = f
                .admission(&updated, enabled.manager_authority_epoch, 2)
                .await;
            assert!(
                execute_command(f.state(), &fresh, PersonalCommandKind::Status)
                    .await
                    .unwrap()
                    .is_some()
            );
            // Retire the selected transport through its actual provider handler, then
            // recover explicitly under a new review; no direct path becomes available.
            let interrupted = f
                .admission(&updated, enabled.manager_authority_epoch, 3)
                .await;
            let raw = {
                let requests = f.harness.received.lock().unwrap();
                let request: Value = serde_json::from_slice(&requests[1]).unwrap();
                Zeroizing::new(
                    request["credentials"]["agent_key"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                )
            };
            let request = Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/bindings/{}",
                    replacement.binding_id.as_ref().unwrap()
                ))
                .header("authorization", format!("Bearer {}", raw.as_str()))
                .body(Body::empty())
                .unwrap();
            let retired = Router::new()
                .route(
                    "/bindings/{binding}",
                    put(nyxbot::put_binding).delete(nyxbot::delete_binding),
                )
                .with_state(f.state().clone())
                .oneshot(request)
                .await
                .unwrap();
            assert_eq!(retired.status(), StatusCode::NO_CONTENT);
            assert!(
                execute_command(f.state(), &interrupted, PersonalCommandKind::Status)
                    .await
                    .is_err()
            );
            let review = admin::review_lifecycle(f.state(), f.actor(), &f.bot.id)
                .await
                .unwrap();
            with_transport_for_test(
                f.transport.clone(),
                admin::reserve_reviewed_update(
                    f.state(),
                    f.actor(),
                    f.funding(),
                    NativeTransportChoice::Gateway,
                    &review,
                    None,
                    None,
                ),
            )
            .await
            .unwrap();
            let NativeConversionOutcome::Converted(recovered) = f.convert(None).await.unwrap()
            else {
                panic!("selected gateway recovery")
            };
            let enabled = enable_personal_as(f.state(), f.actor(), &recovered).await;
            assert!(matches!(
                &recovered.native_entry.as_ref().unwrap().transport,
                EntryTransport::NyxagentGateway { .. }
            ));
            assert!(
                execute_command(f.state(), &interrupted, PersonalCommandKind::Status)
                    .await
                    .is_err()
            );
            let fresh = f
                .admission(&recovered, enabled.manager_authority_epoch, 4)
                .await;
            assert!(
                execute_command(f.state(), &fresh, PersonalCommandKind::Status)
                    .await
                    .unwrap()
                    .is_some()
            );
            assert!(
                f.state()
                    .db
                    .collection::<PlatformEntryGateway>(GATEWAYS)
                    .find_one(doc! {"_id":&replacement.id,"state":"retired"})
                    .await
                    .unwrap()
                    .is_some()
            );
            for id in [
                &replacement.provider_api_key_id,
                &replacement.route_api_key_id,
            ] {
                assert!(
                    !f.state()
                        .db
                        .collection::<ApiKey>(KEYS)
                        .find_one(doc! {"_id":id})
                        .await
                        .unwrap()
                        .unwrap()
                        .is_active
                );
            }
            assert_eq!(f.harness.posts.load(Ordering::SeqCst), 3);
            assert_eq!(f.harness.binding_puts.load(Ordering::SeqCst), 3);
            f.close().await;
        },
    );
}

#[test]
fn reviewed_gateway_update_refuses_recent_or_missing_dispatched_deadline_without_retiring_keys() {
    crate::test_utils::run_native_journey_test(
        "reviewed_gateway_update_refuses_recent_or_missing_dispatched_deadline_without_retiring_keys",
        || async {
            for missing in [false, true] {
                let f = SetupFixture::new(2).await;
                assert!(f.convert(None).await.is_err());
                let before = f
                    .state()
                    .db
                    .collection::<PlatformEntryGateway>(GATEWAYS)
                    .find_one(doc! {"state":"prepared"})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(before.setup_state, "dispatched");
                assert_eq!(f.harness.completed_responses.load(Ordering::SeqCst), 1);
                assert_eq!(
                    f.harness.records.lock().unwrap()[&before.id]["definition"]["creation"]["ready"],
                    false
                );
                if missing {
                    f.state()
                        .db
                        .collection::<Document>(GATEWAYS)
                        .update_one(
                            doc! {"_id":&before.id},
                            doc! {"$unset":{"setup_dispatch_deadline":""}},
                        )
                        .await
                        .unwrap();
                }
                let retained = f
                    .state()
                    .db
                    .collection::<Document>(GATEWAYS)
                    .find_one(doc! {"_id":&before.id})
                    .await
                    .unwrap()
                    .unwrap();
                let bot = f
                    .state()
                    .db
                    .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                    .find_one(doc! {"_id":&f.bot.id})
                    .await
                    .unwrap()
                    .unwrap();
                let review = admin::review_lifecycle(f.state(), f.actor(), &f.bot.id)
                    .await
                    .unwrap();
                assert!(
                    with_transport_for_test(
                        f.transport.clone(),
                        admin::reserve_reviewed_update(
                            f.state(),
                            f.actor(),
                            f.funding(),
                            NativeTransportChoice::Gateway,
                            &review,
                            None,
                            None,
                        )
                    )
                    .await
                    .is_err()
                );
                assert_eq!(
                    f.state()
                        .db
                        .collection::<Document>(GATEWAYS)
                        .find_one(doc! {"_id":&before.id})
                        .await
                        .unwrap()
                        .unwrap(),
                    retained
                );
                assert_eq!(
                    f.state()
                        .db
                        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                        .find_one(doc! {"_id":&f.bot.id})
                        .await
                        .unwrap()
                        .unwrap(),
                    bot
                );
                for id in [&before.provider_api_key_id, &before.route_api_key_id] {
                    assert!(
                        f.state()
                            .db
                            .collection::<ApiKey>(KEYS)
                            .find_one(doc! {"_id":id})
                            .await
                            .unwrap()
                            .unwrap()
                            .is_active
                    );
                }
                assert_eq!(f.harness.posts.load(Ordering::SeqCst), 1);
                f.close().await;
            }
        },
    );
}

#[test]
fn native_gateway_creation_real_binding_callback_observation_and_activation() {
    crate::test_utils::run_native_journey_test(
        "native_gateway_creation_real_binding_callback_observation_and_activation",
        || async {
            let f = SetupFixture::new(0).await;
            let issued =
                chrono::DateTime::from_timestamp(Utc::now().timestamp(), 123_456_789).unwrap();
            assert_ne!(issued.timestamp_subsec_nanos() % 1_000_000, 0);
            let NativeConversionOutcome::Converted(profile) = super::TEST_TRANSPORT_ISSUED_AT
                .scope(issued, f.convert(None))
                .await
                .unwrap()
            else {
                panic!("gateway conversion")
            };
            let row = f
                .harness
                .state
                .db
                .collection::<PlatformEntryGateway>(GATEWAYS)
                .find_one(doc! {"profile_id":&profile.id,"state":"active"})
                .await
                .unwrap()
                .unwrap();
            for id in [&row.provider_api_key_id, &row.route_api_key_id] {
                let key = f
                    .state()
                    .db
                    .collection::<ApiKey>(KEYS)
                    .find_one(doc! {"_id":id})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(key.created_at.timestamp_millis(), issued.timestamp_millis());
                assert_eq!(key.created_at.timestamp_subsec_nanos() % 1_000_000, 0);
            }
            assert_eq!(row.setup_state, "observed");
            assert_eq!(f.harness.posts.load(Ordering::SeqCst), 1);
            assert_eq!(f.harness.binding_puts.load(Ordering::SeqCst), 1);
            assert_eq!(profile.native_entry.as_ref().unwrap(), &row.configuration);
            assert!(!profile.enabled);
            assert!(
                f.harness
                    .state
                    .db
                    .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
                    .find_one(doc! {"channel_bot_id":&f.bot.id})
                    .await
                    .unwrap()
                    .is_none()
            );
            f.close().await;
        },
    );
}

#[test]
fn native_gateway_lost_create_response_recovers_exact_ready_record_without_duplicate_post() {
    crate::test_utils::run_native_journey_test(
        "native_gateway_lost_create_response_recovers_exact_ready_record_without_duplicate_post",
        || async {
            let f = SetupFixture::new(1).await;
            assert!(f.convert(None).await.is_err());
            assert_eq!(f.harness.posts.load(Ordering::SeqCst), 1);
            let before = f
                .harness
                .state
                .db
                .collection::<PlatformEntryGateway>(GATEWAYS)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(before.state, "prepared");
            assert_eq!(before.setup_state, "dispatched");
            f.harness.mode.store(0, Ordering::SeqCst);
            let NativeConversionOutcome::Converted(profile) = f.convert(None).await.unwrap() else {
                panic!("retained ready recovery")
            };
            let after = f
                .harness
                .state
                .db
                .collection::<PlatformEntryGateway>(GATEWAYS)
                .find_one(doc! {"_id":&before.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(before.setup_attempt_id, after.setup_attempt_id);
            assert_eq!(before.request_hash, after.request_hash);
            assert_eq!(before.request_ciphertext, after.request_ciphertext);
            assert_eq!(after.state, "active");
            assert_eq!(after.profile_id, profile.id);
            assert_eq!(f.harness.posts.load(Ordering::SeqCst), 1);
            assert_eq!(f.harness.binding_puts.load(Ordering::SeqCst), 1);
            f.close().await;
        },
    );
}

#[test]
fn native_gateway_reserved_record_requires_exact_review_and_reuses_request_once() {
    crate::test_utils::run_native_journey_test(
        "native_gateway_reserved_record_requires_exact_review_and_reuses_request_once",
        || async {
            let f = SetupFixture::new(2).await;
            assert!(f.convert(None).await.is_err());
            let before = f
                .state()
                .db
                .collection::<PlatformEntryGateway>(GATEWAYS)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(f.harness.posts.load(Ordering::SeqCst), 1);
            assert_eq!(f.harness.binding_puts.load(Ordering::SeqCst), 0);
            let NativeConversionOutcome::GatewayRecovery(review) = f.convert(None).await.unwrap()
            else {
                panic!("incomplete reserved record requires human recovery review")
            };
            assert!(!review.remote_ready);
            assert_eq!(review.record_id, before.id);
            assert!(
                f.convert(Some(&review)).await.is_err(),
                "in-flight uncertainty window"
            );
            assert_eq!(f.harness.posts.load(Ordering::SeqCst), 1);
            // Advance only the retained fixture deadline, preserving the exact request,
            // credentials, reservation and observed remote record under review.
            f.state().db.collection::<Document>(GATEWAYS).update_one(doc! {"_id":&before.id},
        doc! {"$set":{"setup_dispatch_deadline":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(180))}})
        .await.unwrap();
            let NativeConversionOutcome::GatewayRecovery(review) = f.convert(None).await.unwrap()
            else {
                panic!("review remains explicit after deadline")
            };
            let mut wrong = review.clone();
            wrong.request_hash = "00".repeat(32);
            assert!(matches!(
                f.convert(Some(&wrong)).await.unwrap(),
                NativeConversionOutcome::GatewayRecovery(_)
            ));
            assert_eq!(f.harness.posts.load(Ordering::SeqCst), 1);
            f.harness.mode.store(0, Ordering::SeqCst);
            let (first, second) = tokio::join!(f.convert(Some(&review)), f.convert(Some(&review)));
            assert!(
                matches!(first, Ok(NativeConversionOutcome::Converted(_)))
                    || matches!(second, Ok(NativeConversionOutcome::Converted(_)))
            );
            assert_eq!(
                f.harness.posts.load(Ordering::SeqCst),
                2,
                "one reviewed retry despite two callers"
            );
            assert_eq!(f.harness.binding_puts.load(Ordering::SeqCst), 1);
            let after = f
                .state()
                .db
                .collection::<PlatformEntryGateway>(GATEWAYS)
                .find_one(doc! {"_id":&before.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(after.state, "active");
            assert_eq!(after.setup_state, "observed");
            assert_ne!(after.setup_attempt_id, before.setup_attempt_id);
            assert_eq!(after.request_ciphertext, before.request_ciphertext);
            assert_eq!(after.request_hash, before.request_hash);
            assert_eq!(after.provider_api_key_id, before.provider_api_key_id);
            assert_eq!(after.route_api_key_id, before.route_api_key_id);
            {
                let requests = f.harness.received.lock().unwrap();
                assert_eq!(requests.len(), 2);
                assert!(
                    requests[0].as_slice() == requests[1].as_slice(),
                    "reviewed retry preserves exact outbound body bytes"
                );
            }
            f.close().await;
        },
    );
}

#[test]
fn native_gateway_failed_final_activation_retains_provider_ownership_and_separation() {
    crate::test_utils::run_native_journey_test(
        "native_gateway_failed_final_activation_retains_provider_ownership_and_separation",
        || async {
            let f = SetupFixture::new(3).await;
            assert!(f.convert(None).await.is_err());
            let row = f
                .harness
                .state
                .db
                .collection::<PlatformEntryGateway>(GATEWAYS)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.state, "prepared");
            assert!(row.binding_id.is_some());
            assert!(f.harness.state.db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).find_one(doc! {"_id":&f.bot.id,"platform_entry_pending":{"$exists":true},"platform_profile_id":{"$exists":false}}).await.unwrap().is_some());
            assert_eq!(f.harness.binding_puts.load(Ordering::SeqCst), 1);
            f.close().await;
        },
    );
}

#[test]
fn gateway_setup_refuses_redirects_and_advertised_or_chunked_oversize() {
    crate::test_utils::run_native_journey_test(
        "gateway_setup_refuses_redirects_and_advertised_or_chunked_oversize",
        || async {
            for mode in [4, 5, 6] {
                let f = SetupFixture::new(mode).await;
                assert!(f.convert(None).await.is_err());
                assert_eq!(
                    f.harness.posts.load(Ordering::SeqCst),
                    0,
                    "redirect target must receive no credentials"
                );
                let row = f
                    .state()
                    .db
                    .collection::<PlatformEntryGateway>(GATEWAYS)
                    .find_one(doc! {"configured_by":f.actor()})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(row.setup_state, "dispatched");
                assert_ne!(row.state, "active");
                f.close().await;
            }
        },
    );
}
