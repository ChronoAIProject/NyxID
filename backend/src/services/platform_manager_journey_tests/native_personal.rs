//! Seeded native profile, actual signed ingress and first-party browser handlers.
//! Provider followups and the browser continuation's upstream stay on loopback.
use super::*;
use crate::{
    handlers::{assistant_nyxagent as browser, platform_personal_bindings as consent},
    models::channel_bot::ChannelBot,
    services::{
        platform_entry_native::tests as native,
        platform_entry_receipts::NativeAdmission,
        platform_native_reply_service::{NativeTestTransport, with_transport_for_test},
    },
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::{get, post},
};
use tower::ServiceExt;
use wiremock::matchers::path_regex;
use zeroize::Zeroizing;

struct ObservedTransport {
    inner: Arc<dyn ManagerTransportProducer>,
    native: Arc<NativeTestTransport>,
    finished: tokio::sync::mpsc::Sender<(ReplyRef, DeliveryOutcome)>,
}
#[async_trait]
impl ManagerTransportProducer for ObservedTransport {
    async fn admit_manager_update(
        &self,
        db: &Database,
        update: &VerifiedManagerUpdate,
    ) -> AppResult<IngressAdmission> {
        self.inner.admit_manager_update(db, update).await
    }
    async fn deliver_platform_reply(
        &self,
        db: &Database,
        destination: &ReplyDestination,
        reference: &DeliveryRef,
        content: ReplyContent,
    ) -> AppResult<DeliveryOutcome> {
        // The automatic consumer runs in its own task. Install the local
        // transport dependency there as well as at the ingress caller.
        let outcome = with_transport_for_test(
            self.native.clone(),
            self.inner
                .deliver_platform_reply(db, destination, reference, content),
        )
        .await?;
        if let DeliveryRef::Reply(reply) = reference {
            self.finished
                .send((reply.clone(), outcome.clone()))
                .await
                .unwrap();
        }
        Ok(outcome)
    }
}

struct NativeJourney {
    state: AppState,
    bot: ChannelBot,
    profile: PlatformBotConfig,
    server: MockServer,
    producers: Arc<PlatformManagerProducers>,
    transport: Arc<NativeTestTransport>,
    egress: Arc<SavedReplyEgress>,
    finished: tokio::sync::mpsc::Receiver<(ReplyRef, DeliveryOutcome)>,
    browser: Router,
    personal: String,
    bearer: Zeroizing<String>,
    operator_bearer: Zeroizing<String>,
}
impl NativeJourney {
    async fn new() -> Self {
        let server = MockServer::start().await;
        let (state, bot, retained) =
            crate::services::platform_native_reply_service::tests::discord_fixture(&server.uri())
                .await;
        let profile = state
            .db
            .collection::<PlatformBotConfig>(PROFILES)
            .find_one(doc! {"_id":&retained.profile_id})
            .await
            .unwrap()
            .unwrap();
        for result in [
            bindings::ensure_indexes(&state.db).await,
            crate::services::platform_link_intent_service::ensure_indexes(&state.db).await,
            crate::services::platform_manager_ingress_service::ensure_indexes(&state.db).await,
            delivery::ensure_indexes(&state.db).await,
            engine::ensure_indexes(&state.db).await,
        ] {
            result.unwrap();
        }
        crate::services::feature_flag_service::set_platform_override(
            &state.db,
            crate::services::feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            "admin",
        )
        .await
        .unwrap();
        Mock::given(method("POST"))
            .and(path_regex("^/webhooks/100/native-journey-"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id":"native-personal-sent"})),
            )
            .mount(&server)
            .await;
        let egress = Arc::new(SavedReplyEgress(
            AtomicUsize::new(0),
            tokio::sync::Mutex::new(Vec::new()),
        ));
        let execution = crate::services::assistant_nyxagent_runtime::producer(
            state.clone(),
            egress.clone(),
            crate::services::platform_public_credential_service::producer(state.clone()),
        );
        let mut producers = assemble(
            state.clone(),
            execution,
            egress.clone(),
            Arc::new(TestSettledHook),
        );
        let transport = Arc::new(NativeTestTransport {
            adapter: Arc::new(
                crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                    &server.uri(),
                ),
            ),
            discord_base: server.uri(),
        });
        let (sender, finished) = tokio::sync::mpsc::channel(4);
        let inner = producers.transport.clone();
        Arc::get_mut(&mut producers).unwrap().transport = Arc::new(ObservedTransport {
            inner,
            native: transport.clone(),
            finished: sender,
        });
        let browser = Router::new()
            .route("/api/v1/platform-personal/review", post(consent::review))
            .route("/api/v1/platform-personal/approve", post(consent::approve))
            .route(
                "/api/v1/assistant/nyxagent/conversations",
                get(browser::list),
            )
            .route(
                "/api/v1/assistant/nyxagent/conversations/{id}/messages",
                get(browser::history),
            )
            .route("/api/v1/assistant/nyxagent/turns", post(browser::turns))
            .with_state(state.clone());
        let personal = uuid::Uuid::new_v4().to_string();
        assert_ne!(personal, bot.user_id);
        state
            .db
            .collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &personal,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let access_token = |user: &str| {
            Zeroizing::new(
                crate::crypto::jwt::generate_access_token(
                    &state.jwt_keys,
                    &state.config,
                    &uuid::Uuid::parse_str(user).unwrap(),
                    "openid profile email",
                    None,
                    Some(600),
                    None,
                    None,
                    None,
                )
                .unwrap(),
            )
        };
        let bearer = access_token(&personal);
        let operator_bearer = access_token(&bot.user_id);
        Self {
            state,
            bot,
            profile,
            server,
            producers,
            transport,
            egress,
            finished,
            browser,
            personal,
            bearer,
            operator_bearer,
        }
    }
    async fn admission(
        &self,
        number: u64,
        command: &str,
        argument: Option<(&str, &str)>,
    ) -> NativeAdmission {
        let mut payload = native::discord_message(command, argument);
        let millis = Utc::now().timestamp_millis() - 1_420_070_400_000;
        payload["id"] = (((millis as u64) << 22) | number).to_string().into();
        payload["token"] = format!("native-journey-{number}").into();
        let body = serde_json::to_vec(&payload).unwrap();
        let key = ed25519_dalek::SigningKey::from_bytes(&[37; 32]);
        let headers = native::discord_headers(&key, &body, Utc::now().timestamp());
        crate::services::platform_entry_native::authenticate_discord_ack(
            &self.state.db,
            self.transport.adapter.as_ref(),
            &self.bot.id,
            &headers,
            &body,
        )
        .await
        .unwrap();
        let configuration = self.profile.native_entry.as_ref().unwrap();
        let batch = crate::services::platform_entry_native::prepare(
            &self.state.db,
            &self.state.encryption_keys,
            self.transport.adapter.as_ref(),
            &self.bot,
            &headers,
            &body,
            configuration,
        )
        .await
        .unwrap();
        let controls = crate::services::platform_bot_runtime_control_service::get(
            &self.state.db,
            &self.profile.id,
        )
        .await
        .unwrap()
        .unwrap();
        let mut session = self.state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let admission = crate::services::platform_entry_receipts::record_in_session(
            &self.state.db,
            &mut session,
            &self.state.encryption_keys,
            self.state.auth_device_hmac_key.as_slice(),
            crate::services::platform_entry_receipts::NativeReceiptInput {
                profile_id: &self.profile.id,
                epoch: controls.manager_authority_epoch,
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
    async fn dispatch(&self, admission: NativeAdmission) {
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            with_transport_for_test(
                self.transport.clone(),
                dispatch_native_admission(&self.state, Some(self.producers.clone()), admission),
            ),
        )
        .await
        .unwrap()
        .unwrap();
    }
    async fn browser_as(
        &self,
        bearer: Option<&str>,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(bearer) = bearer {
            request = request.header("authorization", format!("Bearer {bearer}"));
        }
        let request = request
            .body(body.map_or_else(Body::empty, |body| {
                Body::from(serde_json::to_vec(&body).unwrap())
            }))
            .unwrap();
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.browser.clone().oneshot(request),
        )
        .await
        .unwrap()
        .unwrap();
        let status = response.status();
        let body = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            to_bytes(response.into_body(), 1_000_000),
        )
        .await
        .unwrap()
        .unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }
    async fn browser(&self, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, body) = self
            .browser_as(Some(self.bearer.as_str()), method, path, body)
            .await;
        assert_eq!(status, StatusCode::OK);
        body
    }
}

#[test]
fn discord_browser_consent_saved_personal_reply_history_continuation_and_logout_use_real_native_producers()
 {
    crate::test_utils::run_native_journey_test(
        "discord_browser_consent_saved_personal_reply_history_continuation_and_logout_use_real_native_producers",
        || async {
            let mut j = NativeJourney::new().await;
            j.dispatch(j.admission(1, "login", None).await).await;
            let requests = j.server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            let login: Value = serde_json::from_slice(&requests[0].body).unwrap();
            let url = reqwest::Url::parse(
                login["content"]
                    .as_str()
                    .unwrap()
                    .split_whitespace()
                    .find(|word| word.starts_with("https://") || word.starts_with("http://"))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                url.origin(),
                reqwest::Url::parse(&j.state.config.frontend_url)
                    .unwrap()
                    .origin()
            );
            let token = Zeroizing::new(
                url.fragment()
                    .unwrap()
                    .strip_prefix("intent_token=")
                    .unwrap()
                    .to_owned(),
            );
            let (status, _) = j
                .browser_as(
                    None,
                    "POST",
                    "/api/v1/platform-personal/review",
                    Some(json!({"intent_token":token.as_str()})),
                )
                .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let review = j
                .browser(
                    "POST",
                    "/api/v1/platform-personal/review",
                    Some(json!({"intent_token":token.as_str()})),
                )
                .await;
            assert_eq!(review["chat"]["platform"], "discord");
            assert_eq!(review["account"]["id"], j.personal);
            assert_eq!(review["disclosures"]["telegram_transport"], false);
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let approved = j.browser("POST", "/api/v1/platform-personal/approve", Some(json!({
        "challenge_id":review["challenge_id"],"review_digest":review["review_digest"],"replace_existing":false}))).await;
            let confirmation = Zeroizing::new(
                approved["confirmation_command"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
            let code = confirmation.strip_prefix("/nyx confirm code:").unwrap();
            j.dispatch(j.admission(2, "confirm", Some(("code", code))).await)
                .await;
            let binding = j
                .state
                .db
                .collection::<PlatformPersonalBinding>(
                    crate::models::platform_personal_binding::COLLECTION_NAME,
                )
                .find_one(doc! {"platform_config_id":&j.profile.id,"state":"active"})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(binding.user_id, j.personal);
            assert!(matches!(binding.sender, SenderIdentity::NativeV2 { .. }));
            j.dispatch(
                j.admission(
                    3,
                    "ask",
                    Some(("question", "Remember this native conversation")),
                )
                .await,
            )
            .await;
            let (reply, outcome) =
                tokio::time::timeout(std::time::Duration::from_secs(30), j.finished.recv())
                    .await
                    .unwrap()
                    .unwrap();
            assert!(matches!(outcome, DeliveryOutcome::NativeSent { .. }));
            let saved = j
                .state
                .db
                .collection::<AssistantMessage>(MESSAGES)
                .find_one(doc! {"_id":&reply.reply_id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved.text, "Retained personal answer");
            let native_conversation =
                engine::get(&j.state.db, &j.personal, &reply.turn.conversation_id)
                    .await
                    .unwrap();
            assert_eq!(native_conversation.user_id, j.personal);
            let native_session = native_conversation.nyxagent_session_id.clone().unwrap();
            assert_eq!(native_session, "conv_1234567890abcdef1234567890abcdef");
            let stored_credential = j
                .state
                .db
                .collection::<crate::models::assistant_agent_credential::AssistantAgentCredential>(
                    crate::models::assistant_agent_credential::COLLECTION_NAME,
                )
                .find_one(doc! {"conversation_id":&native_conversation.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored_credential.user_id, j.personal);
            assert_eq!(
                stored_credential.api_key_id,
                native_conversation.credential_api_key_id
            );
            let native_key = j
                .state
                .db
                .collection::<crate::models::api_key::ApiKey>(
                    crate::models::api_key::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":&stored_credential.api_key_id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(native_key.user_id, j.personal);
            let credential_count = j
                .state
                .db
                .collection::<Document>(crate::models::assistant_agent_credential::COLLECTION_NAME)
                .count_documents(doc! {"user_id":&j.personal})
                .await
                .unwrap();
            assert_eq!(credential_count, 1);
            let destination = delivery::captured_destination(&saved).unwrap().unwrap();
            let expected = delivery::render_personal(&j.state, &saved, None)
                .await
                .unwrap();
            let requests = j.server.received_requests().await.unwrap();
            assert_eq!(
                requests.len(),
                3,
                "login, confirmation, first automatic personal send"
            );
            let posted: Value = serde_json::from_slice(&requests[2].body).unwrap();
            assert_eq!(posted["content"].as_str().unwrap(), expected);
            assert!(requests.iter().all(|request| {
                request
                    .url
                    .path()
                    .starts_with("/webhooks/100/native-journey-")
            }));
            let duplicate = with_transport_for_test(
                j.transport.clone(),
                j.producers.transport.deliver_platform_reply(
                    &j.state.db,
                    &destination,
                    &DeliveryRef::Reply(reply.clone()),
                    ReplyContent::Text(expected),
                ),
            )
            .await
            .unwrap();
            assert!(matches!(duplicate, DeliveryOutcome::Duplicate));
            assert_eq!(j.server.received_requests().await.unwrap().len(), 3);
            let history_path = format!(
                "/api/v1/assistant/nyxagent/conversations/{}/messages",
                reply.turn.conversation_id
            );
            let (operator_history, _) = j
                .browser_as(Some(j.operator_bearer.as_str()), "GET", &history_path, None)
                .await;
            assert_eq!(operator_history, StatusCode::NOT_FOUND);
            let (operator_turn, _) = j.browser_as(Some(j.operator_bearer.as_str()), "POST", "/api/v1/assistant/nyxagent/turns",
        Some(json!({"conversation_id":&native_conversation.id,"text":"Operator must not enter this personal thread"}))).await;
            assert_eq!(operator_turn, StatusCode::NOT_FOUND);
            assert_eq!(j.egress.0.load(Ordering::SeqCst), 1);
            let history = j.browser("GET", &history_path, None).await;
            assert_eq!(
                history["conversation"]["platform_thread"]["platform"],
                "discord"
            );
            assert!(
                history["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|message| message["id"] == saved.id)
            );
            let index = j
                .browser("GET", "/api/v1/assistant/nyxagent/conversations", None)
                .await;
            assert!(
                index["conversations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["id"] == reply.turn.conversation_id
                        && row["platform_thread"]["platform"] == "discord")
            );

            // The real browser turn handler keeps this thread and encrypted credential.
            let mut upstream = crate::test_utils::test_auto_connected_catalog_service();
            upstream.slug = engine::SERVICE_SLUG.into();
            upstream.base_url = j.server.uri();
            upstream.forward_access_token = true;
            upstream.streaming_supported = true;
            assert!(engine::row_contract(Some(&upstream), None).valid());
            j.state
                .db
                .collection::<crate::models::downstream_service::DownstreamService>(
                    crate::models::downstream_service::COLLECTION_NAME,
                )
                .insert_one(upstream)
                .await
                .unwrap();
            crate::services::unified_key_service::auto_provision_no_auth_services(
                &j.state.db,
                &j.personal,
            )
            .await
            .unwrap();
            let session = "1234567890abcdef1234567890abcdef";
            let completed = json!({"type":"response.completed","sequence_number":0,"response":{"id":format!("resp_{session}_abcdef1234567890abcdef1234567890"),"conversation":{"id":format!("conv_{session}")},"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Continued from the browser"}]}]}});
            Mock::given(method("POST"))
                .and(path("/v1/responses"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    format!("data: {completed}\n\n").into_bytes(),
                    "text/event-stream",
                ))
                .expect(1)
                .mount(&j.server)
                .await;
            let mut request = Request::builder().method("POST").uri("/api/v1/assistant/nyxagent/turns")
        .header("authorization",format!("Bearer {}",j.bearer.as_str())).header("content-type","application/json")
        .body(Body::from(serde_json::to_vec(&json!({"conversation_id":reply.turn.conversation_id,"text":"Continue from the website"})).unwrap())).unwrap();
            request.extensions_mut().insert(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy,
                ),
            );
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                j.browser.clone().oneshot(request),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let stream = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                to_bytes(response.into_body(), 1_000_000),
            )
            .await
            .unwrap()
            .unwrap();
            let events: Vec<Value> = std::str::from_utf8(&stream)
                .unwrap()
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|data| serde_json::from_str(data.trim()).unwrap())
                .collect();
            let terminal: Vec<_> = events
                .iter()
                .filter(|event| event["event"] == "turn.completed")
                .collect();
            assert_eq!(terminal.len(), 1, "one exact browser terminal event");
            let terminal = terminal[0];
            let browser_saved = j
                .state
                .db
                .collection::<AssistantMessage>(MESSAGES)
                .find_one(
                    doc! {"conversation_id":&native_conversation.id,"user_id":&j.personal,
                    "turn_id":terminal["turn_id"].as_str().unwrap(),"role":"assistant"},
                )
                .await
                .unwrap()
                .unwrap();
            eprintln!(
                "native browser terminal status={} has_error={} saved_status={} error_code={:?} failure_stage={:?}",
                terminal["status"].as_str().unwrap(),
                !terminal["error"].is_null(),
                browser_saved.status,
                browser_saved.error_code,
                browser_saved
                    .error_detail
                    .as_ref()
                    .map(|detail| detail.stage.as_str())
            );
            assert_eq!(
                terminal["status"], "completed",
                "browser continuation must succeed before credential reuse is claimed"
            );
            assert!(terminal["error"].is_null());
            assert_eq!(browser_saved.status, "completed");
            assert!(browser_saved.error_code.is_none());
            assert_eq!(browser_saved.text, "Continued from the browser");
            let browser_requests = j.server.received_requests().await.unwrap();
            let actual_browser = browser_requests
                .iter()
                .find(|request| request.url.path() == "/v1/responses")
                .unwrap();
            let bearer_hash = crate::crypto::token::hash_token(
                actual_browser
                    .headers
                    .get("authorization")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .strip_prefix("Bearer ")
                    .unwrap(),
            );
            assert!(
                bearer_hash == native_key.key_hash,
                "browser must use the native conversation credential"
            );
            let browser_payload: Value = serde_json::from_slice(&actual_browser.body).unwrap();
            assert_eq!(browser_payload["conversation"], native_session);
            let final_conversation = engine::get(&j.state.db, &j.personal, &native_conversation.id)
                .await
                .unwrap();
            assert_eq!(final_conversation.id, native_conversation.id);
            assert_eq!(
                final_conversation.nyxagent_session_id.as_deref(),
                Some(native_session.as_str())
            );
            assert_eq!(
                final_conversation.credential_api_key_id,
                native_conversation.credential_api_key_id
            );
            let final_credential = j
                .state
                .db
                .collection::<crate::models::assistant_agent_credential::AssistantAgentCredential>(
                    crate::models::assistant_agent_credential::COLLECTION_NAME,
                )
                .find_one(doc! {"conversation_id":&native_conversation.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(final_credential.id, stored_credential.id);
            assert_eq!(final_credential.user_id, j.personal);
            assert_eq!(final_credential.api_key_id, stored_credential.api_key_id);
            assert!(final_credential.key_ciphertext == stored_credential.key_ciphertext);
            assert_eq!(
                j.state
                    .db
                    .collection::<Document>(
                        crate::models::assistant_agent_credential::COLLECTION_NAME
                    )
                    .count_documents(doc! {"user_id":&j.personal})
                    .await
                    .unwrap(),
                credential_count
            );
            let continued = j.browser("GET", &history_path, None).await;
            assert_eq!(
                continued["conversation"]["platform_thread"]["platform"],
                "discord"
            );
            assert!(
                continued["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|message| message["text"] == "Continued from the browser")
            );
            assert_eq!(j.egress.0.load(Ordering::SeqCst), 1);
            j.dispatch(j.admission(4, "logout", None).await).await;
            let binding = j
                .state
                .db
                .collection::<PlatformPersonalBinding>(
                    crate::models::platform_personal_binding::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":&binding.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(binding.state, PersonalBindingState::Revoked);
            assert!(
                j.browser("GET", &history_path, None).await["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|message| message["id"] == saved.id)
            );
            let sends = j.server.received_requests().await.unwrap();
            assert_eq!(
                sends
                    .iter()
                    .filter(|request| request.url.path().starts_with("/webhooks/"))
                    .count(),
                4
            );
            assert_eq!(
                sends
                    .iter()
                    .filter(|request| request.url.path() == "/v1/responses")
                    .count(),
                1
            );
            assert_eq!(sends.len(), 5, "all possible test egress is counted local");
            j.state.db.drop().await.unwrap();
        },
    );
}

#[test]
fn same_owner_links_each_native_and_telegram_bot_independently_and_logout_is_local() {
    crate::test_utils::run_native_journey_test("same_owner_independent_entries", || async {
        let mut native = NativeJourney::new().await;
        let mut telegram = Journey::new().await;
        let old_db = telegram.state.db.clone();
        // Both production paths must resolve one person's existing NyxBot in the same database.
        for collection in [
            PROFILES,
            crate::models::channel_bot::COLLECTION_NAME,
            crate::models::platform_credential::COLLECTION_NAME,
            crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
        ] {
            let rows = old_db
                .collection::<Document>(collection)
                .find(doc! {})
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
            if !rows.is_empty() {
                native
                    .state
                    .db
                    .collection::<Document>(collection)
                    .insert_many(rows)
                    .await
                    .unwrap();
            }
        }
        telegram.state = native.state.clone();
        telegram.owner = native.personal.clone();
        let egress = telegram.egress.clone();
        let execution = crate::services::assistant_nyxagent_runtime::producer(
            telegram.state.clone(),
            egress.clone(),
            crate::services::platform_public_credential_service::producer(telegram.state.clone()),
        );
        let mut producers = assemble(
            telegram.state.clone(),
            execution,
            egress,
            Arc::new(TestSettledHook),
        );
        let identity = producers.identity.clone();
        Arc::get_mut(&mut producers).unwrap().transport = Arc::new(LocalTransport {
            state: telegram.state.clone(),
            identity,
            base: telegram.server.uri(),
            personal_gate: None,
        });
        telegram.producers = producers;
        telegram.link().await;
        let binding_rows = native.state.db.collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        );
        let count =
            || binding_rows.count_documents(doc! {"user_id":&native.personal,"state":"active"});
        assert_eq!(
            count().await.unwrap(),
            1,
            "Telegram proof must not claim Discord"
        );
        native
            .dispatch(native.admission(101, "login", None).await)
            .await;
        let requests = native.server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
        let url = reqwest::Url::parse(
            body["content"]
                .as_str()
                .unwrap()
                .split_whitespace()
                .find(|word| word.starts_with("https://") || word.starts_with("http://"))
                .unwrap(),
        )
        .unwrap();
        let token = Zeroizing::new(
            url.fragment()
                .unwrap()
                .strip_prefix("intent_token=")
                .unwrap()
                .to_owned(),
        );
        let review = native
            .browser(
                "POST",
                "/api/v1/platform-personal/review",
                Some(json!({"intent_token":token.as_str()})),
            )
            .await;
        assert_eq!(
            count().await.unwrap(),
            1,
            "Browser sign-in and review alone cannot link Discord"
        );
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let proof=native.browser("POST","/api/v1/platform-personal/approve",Some(json!({"challenge_id":review["challenge_id"],"review_digest":review["review_digest"],"replace_existing":false}))).await;
        let confirmation =
            Zeroizing::new(proof["confirmation_command"].as_str().unwrap().to_owned());
        let code = confirmation.strip_prefix("/nyx confirm code:").unwrap();
        telegram
            .webhook(&telegram.update(102, &format!("/confirm {code}"), false))
            .await;
        assert_eq!(
            count().await.unwrap(),
            1,
            "Proof from a different platform/bot cannot confirm"
        );
        native
            .dispatch(native.admission(103, "confirm", Some(("code", code))).await)
            .await;
        assert_eq!(count().await.unwrap(), 2);
        telegram
            .webhook(&telegram.update(104, "Remember the Telegram channel", false))
            .await;
        native
            .dispatch(
                native
                    .admission(
                        105,
                        "ask",
                        Some(("question", "Remember the Discord channel")),
                    )
                    .await,
            )
            .await;
        let (_, outcome) =
            tokio::time::timeout(std::time::Duration::from_secs(30), native.finished.recv())
                .await
                .unwrap()
                .unwrap();
        assert!(matches!(outcome, DeliveryOutcome::NativeSent { .. }));
        let rows = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let rows = native
                    .state
                    .db
                    .collection::<PlatformPersonalBinding>(
                        crate::models::platform_personal_binding::COLLECTION_NAME,
                    )
                    .find(doc! {"user_id":&native.personal,"state":"active"})
                    .await
                    .unwrap()
                    .try_collect::<Vec<_>>()
                    .await
                    .unwrap();
                if rows.len() == 2 && rows.iter().all(|row| row.conversation_id.is_some()) {
                    break rows;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            rows[0].nyxbot_agent_id, rows[1].nyxbot_agent_id,
            "Reuse the existing personal NyxBot"
        );
        assert_ne!(
            rows[0].conversation_id, rows[1].conversation_id,
            "Each proved channel has its own saved thread"
        );
        telegram
            .webhook(&telegram.update(106, "/logout", false))
            .await;
        assert_eq!(count().await.unwrap(), 1);
        let remaining = native
            .state
            .db
            .collection::<PlatformPersonalBinding>(
                crate::models::platform_personal_binding::COLLECTION_NAME,
            )
            .find_one(doc! {"user_id":&native.personal,"state":"active"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(remaining.platform_config_id, native.profile.id);
        native
            .dispatch(native.admission(107, "status", None).await)
            .await;
        let requests = native.server.received_requests().await.unwrap();
        assert!(String::from_utf8_lossy(&requests.last().unwrap().body).contains("logged in"));
        old_db.drop().await.unwrap();
        native.state.db.drop().await.unwrap();
    });
}

#[test]
fn confirmed_native_proof_rechecks_disable_logout_and_repair_and_new_proof_is_independent() {
    crate::test_utils::run_native_journey_test("native_proof_live_replay", || async {
        let j = NativeJourney::new().await;
        async fn prove(j: &NativeJourney, sequence: u64) -> Zeroizing<String> {
            j.dispatch(j.admission(sequence, "login", None).await).await;
            let requests = j.server.received_requests().await.unwrap();
            let body: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
            let url = reqwest::Url::parse(
                body["content"]
                    .as_str()
                    .unwrap()
                    .split_whitespace()
                    .find(|word| word.starts_with("https://") || word.starts_with("http://"))
                    .unwrap(),
            )
            .unwrap();
            let token = Zeroizing::new(
                url.fragment()
                    .unwrap()
                    .strip_prefix("intent_token=")
                    .unwrap()
                    .to_owned(),
            );
            let review = j
                .browser(
                    "POST",
                    "/api/v1/platform-personal/review",
                    Some(json!({"intent_token":token.as_str()})),
                )
                .await;
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let approved=j.browser("POST","/api/v1/platform-personal/approve",Some(json!({"challenge_id":review["challenge_id"],"review_digest":review["review_digest"],"replace_existing":false}))).await;
            let code = Zeroizing::new(
                approved["confirmation_command"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("/nyx confirm code:")
                    .unwrap()
                    .to_owned(),
            );
            j.dispatch(
                j.admission(sequence + 1, "confirm", Some(("code", code.as_str())))
                    .await,
            )
            .await;
            code
        }
        async fn reconfirm(j: &NativeJourney, code: &str, sequence: u64) -> bool {
            // Keep the production 750ms per-sender decision fence, including
            // unsuccessful replays, rather than confusing rate refusal with revocation.
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
            let admission = j.admission(sequence, "confirm", Some(("code", code))).await;
            let lifecycle = bindings::LifecycleProducer {
                personal: j.producers.personal.clone(),
                public: j.producers.public.clone(),
            };
            let mut session = j.state.db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            let result = crate::services::platform_link_intent_service::confirm_in_session(
                &j.state.db,
                &mut session,
                j.state.auth_device_hmac_key.as_slice(),
                j.state.audit_chain_hmac_key.as_slice(),
                &admission.verified_update(),
                false,
                code,
                &lifecycle,
            )
            .await;
            session.abort_transaction().await.unwrap();
            match result {
                Ok(_) => true,
                Err(AppError::NotFound(message)) => {
                    assert_eq!(message, "Personal linking request unavailable");
                    false
                }
                Err(error) => panic!("Unexpected proof result: {error:?}"),
            }
        }
        let code = prove(&j, 201).await;
        assert!(reconfirm(&j, &code, 203).await);
        let rows = j.state.db.collection::<PlatformPersonalBinding>(
            crate::models::platform_personal_binding::COLLECTION_NAME,
        );
        let old = rows
            .find_one(doc! {"platform_config_id":&j.profile.id,"state":"active"})
            .await
            .unwrap()
            .unwrap();
        let controls = j
            .state
            .db
            .collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME);
        controls
            .update_one(
                doc! {"platform_config_id":&j.profile.id},
                doc! {"$set":{"personal_enabled":false}},
            )
            .await
            .unwrap();
        assert!(!reconfirm(&j, &code, 204).await);
        controls
            .update_one(
                doc! {"platform_config_id":&j.profile.id},
                doc! {"$set":{"personal_enabled":true}},
            )
            .await
            .unwrap();
        j.dispatch(j.admission(205, "logout", None).await).await;
        assert!(!reconfirm(&j, &code, 206).await);
        let second_code = prove(&j, 207).await;
        let before_repair = rows
            .find_one(doc! {"platform_config_id":&j.profile.id,"state":"active"})
            .await
            .unwrap()
            .unwrap();
        // Same durable epoch invalidation used by reviewed repair. Old active
        // inventory cannot say Linked; a later separately proved row can.
        controls
            .update_one(
                doc! {"platform_config_id":&j.profile.id},
                doc! {"$inc":{"manager_authority_epoch":1_i64}},
            )
            .await
            .unwrap();
        assert_eq!(
            bindings::inventory_state(&j.state.db, &before_repair)
                .await
                .unwrap(),
            PersonalBindingState::Revoked
        );
        assert!(!reconfirm(&j, &second_code, 209).await);
        let third_code = prove(&j, 210).await;
        assert!(reconfirm(&j, &third_code, 212).await);
        let current=rows.find_one(doc!{"platform_config_id":&j.profile.id,"manager_authority_epoch":2,"state":"active"}).await.unwrap().unwrap();
        assert_ne!(current.id, old.id);
        assert_ne!(current.id, before_repair.id);
        assert_eq!(
            bindings::inventory_state(&j.state.db, &current)
                .await
                .unwrap(),
            PersonalBindingState::Active
        );
        assert_eq!(
            bindings::inventory_state(&j.state.db, &before_repair)
                .await
                .unwrap(),
            PersonalBindingState::Revoked
        );
        j.state.db.drop().await.unwrap();
    });
}
