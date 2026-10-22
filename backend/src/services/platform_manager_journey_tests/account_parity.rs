//! Exercises the actual owner egress and ordinary encrypted credentials. Only
//! the upstream inference HTTP response is synthetic; no paid model is called.
use super::*;
use crate::models::{
    api_key::ApiKeyPurpose,
    assistant_agent::{AssistantAgent, MemoryNote},
    assistant_settings::{AccessMode, AssistantSettings},
    usage_meter::{CredentialClass, UsageMeterRow, UsageStatus},
};
use crate::models::{billing_wallet::BillingWallet, credits::Credits};
use crate::models::{service_billing::BillingMetric, usage_meter::BillingLayer};
use crate::services::{
    assistant_account_tools::AccountTools, assistant_acknowledgement_service as acks,
    assistant_agent_credential_service as credentials, assistant_authority_tests::connected,
    assistant_team_service as team, billing, key_service,
};
use tokio::sync::Mutex;

#[derive(Clone)]
struct PersonalModelCallback {
    state: AppState,
    requests: Arc<Mutex<Vec<(String, String, Value)>>>,
}

// Local stand-in for NyxAgent's authenticated inner model callback. Both the
// personal chat turns and the real web continuation use the forwarded ordinary
// key, live turn context, production proxy and actual billing settlement.
async fn model_then_reply(
    axum::extract::State(runtime): axum::extract::State<PersonalModelCallback>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> AppResult<axum::response::Response> {
    use axum::{
        body::{Body, to_bytes},
        extract::{Path, State},
        http::Request,
        response::IntoResponse,
    };
    let raw = zeroize::Zeroizing::new(
        headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .strip_prefix("Bearer ")
            .unwrap()
            .to_owned(),
    );
    let (user, key, child) = key_service::validate_api_key(&runtime.state.db, &raw).await?;
    assert!(child.is_none());
    let mut auth =
        crate::mw::auth::api_key_auth_user(&runtime.state.db, &key, None, None, None).await?;
    let chat = acks::for_key(&runtime.state.db, &user, Some(&key.id))
        .await?
        .unwrap();
    assert!(chat.turn_live && !chat.guest && chat.is_orchestrator());
    auth.assistant_chat = Some(Arc::new(chat));
    let mut model_request = Request::builder().method("POST")
        .uri("/api/v1/proxy/s/personal-payer/responses")
        .header("content-type", "application/json")
        .body(Body::from(json!({"model":"account-model","input":"Use my account context","max_output_tokens":16,"stream":false,"store":false}).to_string())).unwrap();
    model_request
        .extensions_mut()
        .insert(billing::route_inventory::BillingRoutePolicy::Metered(
            billing::BillingIngress::Proxy,
        ));
    let response = crate::handlers::proxy::proxy_request_by_slug(
        State(runtime.state.clone()),
        auth,
        Default::default(),
        Path(("personal-payer".into(), "responses".into())),
        model_request,
    )
    .await?;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let model_body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(
        model_body["output"][0]["content"][0]["text"],
        "Paid by this account"
    );
    runtime.requests.lock().await.push((user, key.id, body));
    let frame = json!({"type":"response.completed","sequence_number":0,"response":{
        "id":"resp_1234567890abcdef1234567890abcdef_1234567890abcdef1234567890abcdef",
        "conversation":{"id":"conv_1234567890abcdef1234567890abcdef"},"status":"completed",
        "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Account-scoped answer"}]}]}});
    Ok((
        [("content-type", "text/event-stream")],
        format!("data: {frame}\n\n"),
    )
        .into_response())
}

struct InspectOwnerEgress {
    state: AppState,
    inner: Arc<dyn NyxAgentEgress>,
    accounts: Vec<(String, String, AccessMode)>,
    observed: Mutex<Vec<(String, String, String, String)>>,
    stage: Mutex<&'static str>,
    failure: Mutex<Option<(&'static str, u32)>>,
}

#[async_trait]
impl NyxAgentEgress for InspectOwnerEgress {
    async fn request(
        &self,
        context: &NyxAgentExecutionContext,
        credential: &RuntimeCredential,
        request: NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        let result = self.inspect_request(context, credential, request).await;
        if let Err(error) = &result {
            *self.failure.lock().await = Some((*self.stage.lock().await, error.error_code()));
        }
        result
    }
}

impl InspectOwnerEgress {
    async fn inspect_request(
        &self,
        context: &NyxAgentExecutionContext,
        credential: &RuntimeCredential,
        request: NyxAgentRequest,
    ) -> AppResult<axum::response::Response> {
        *self.stage.lock().await = "credential_validation";
        let (user, key, child) =
            key_service::validate_api_key(&self.state.db, &credential.raw_key).await?;
        assert!(child.is_none());
        assert_eq!(key.id, credential.api_key_id);
        assert_eq!(key.purpose, ApiKeyPurpose::General);
        assert!(key.allow_all_services && key.allow_all_nodes && key.allow_auto_connected_services);
        assert!(
            key.scopes
                .split_whitespace()
                .any(|s| s == crate::mw::auth::ASSISTANT_ACCOUNT_SCOPE)
        );
        let NyxAgentExecutionContext::Owner {
            user_id,
            turn,
            platform_origin,
        } = context
        else {
            panic!("expected admitted personal owner turn");
        };
        assert_eq!(&user, user_id);
        if let Some(origin) = platform_origin {
            assert_eq!(origin.authority.user_id, user);
        }
        *self.stage.lock().await = "live_chat";
        let chat = acks::for_key(&self.state.db, &user, Some(&key.id))
            .await?
            .unwrap();
        assert!(chat.turn_live && !chat.guest && chat.is_orchestrator());
        assert_eq!(chat.conversation_id, turn.conversation_id);
        *self.stage.lock().await = "access_mode";
        let (_, service, mode) = self.accounts.iter().find(|a| a.0 == user).unwrap();
        assert_eq!(
            acks::effective_access_mode(&self.state.db, &chat).await?,
            *mode
        );
        *self.stage.lock().await = "account_authentication";
        let auth =
            crate::mw::auth::api_key_auth_user(&self.state.db, &key, None, None, None).await?;
        let tools = AccountTools {
            db: &self.state.db,
            keys: &self.state.encryption_keys,
            http: &self.state.http_client,
            config: &self.state.config,
            token_exchange_cache: &self.state.token_exchange_cache,
            node_manager: &self.state.node_ws_manager,
        };
        *self.stage.lock().await = "account_service_tools";
        let list = tools
            .execute(&auth, "nyxid__list_my_services", &json!({}))
            .await;
        assert!(!list.is_error);
        assert!(list.value.to_string().contains(service));
        for (other_user, other_service, _) in self.accounts.iter().filter(|a| a.0 != user) {
            assert!(!list.value.to_string().contains(other_service));
            let denied = tools
                .execute(
                    &auth,
                    "nyxid__set_service_enabled",
                    &json!({"service_id":other_service,"enabled":false}),
                )
                .await;
            assert!(denied.is_error);
            assert!(!denied.value.to_string().contains(other_user));
        }
        let change = tools
            .execute(
                &auth,
                "nyxid__set_service_enabled",
                &json!({"service_id":service,"enabled":true}),
            )
            .await;
        if *mode == AccessMode::AskEveryTime {
            assert!(change.is_error);
            assert_eq!(change.value["error"], "acknowledgement_required");
        } else {
            assert!(!change.is_error);
        }
        *self.stage.lock().await = "owned_bot_lookup";
        let owned_bot = self
            .state
            .db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME,
            )
            .find_one(doc! {"user_id":&user,"label":{"$regex":"^account-owned-bot-"}})
            .await?
            .unwrap();
        let shared_snapshot = self
            .state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(doc! {"user_id":&user,"label":"same-owner-platform-manager"})
            .await?
            .unwrap();
        let shared: crate::models::channel_bot::ChannelBot =
            bson::from_document(shared_snapshot.clone()).map_err(|_| {
                AppError::Internal("Unable to decode shared manager fixture".into())
            })?;
        *self.stage.lock().await = "shared_manager_refusal";
        let mut shared_args = json!({"bot_id":shared.id,"label":"Forbidden shared mutation"});
        let mut shared_result = tools
            .execute(&auth, "nyxid__update_channel_bot", &shared_args)
            .await;
        let mut shared_confirmation = None;
        if *mode == AccessMode::AskEveryTime {
            assert!(shared_result.is_error);
            assert_eq!(shared_result.value["error"], "acknowledgement_required");
        }
        if shared_result.value["error"] == "acknowledgement_required" {
            let card = shared_result.value["acknowledgement_id"]
                .as_str()
                .unwrap()
                .to_owned();
            acks::decide(&self.state.db, &user, &turn.conversation_id, &card, true).await?;
            shared_args["acknowledgement_id"] = json!(card);
            shared_result = tools
                .execute(&auth, "nyxid__update_channel_bot", &shared_args)
                .await;
            shared_confirmation = Some(card);
        }
        assert!(
            shared_result.is_error,
            "same-owner platform manager stays confined after normal confirmation"
        );
        let forbidden = AppError::Forbidden(String::new()).response_body();
        assert_eq!(shared_result.value["error"], forbidden.error);
        assert_eq!(shared_result.value["error_code"], forbidden.error_code);
        if let Some(card) = shared_confirmation {
            assert!(
                self.state
                    .db
                    .collection::<Document>(
                        crate::models::assistant_acknowledgement::COLLECTION_NAME,
                    )
                    .find_one(doc! {"_id":card,"user_id":&user,
                    "conversation_id":&turn.conversation_id,"status":"used"})
                    .projection(doc! {"_id":1})
                    .await?
                    .is_some(),
                "shared manager mutation is refused after consuming the exact owner confirmation"
            );
        }
        let shared_unchanged = self
            .state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(doc! {"_id":&shared.id})
            .await?
            .unwrap();
        assert_eq!(shared_unchanged.get_str("label").unwrap(), shared.label);
        assert!(
            shared_unchanged == shared_snapshot,
            "shared manager stored row stays unchanged"
        );
        *self.stage.lock().await = "foreign_bot_refusal";
        for target in self
            .state
            .db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME,
            )
            .find(doc! {"user_id":{"$ne":&user}})
            .await?
            .try_collect::<Vec<_>>()
            .await?
        {
            let denied = tools
                .execute(
                    &auth,
                    "nyxid__update_channel_bot",
                    &json!({"bot_id":target.id,"label":"Forbidden mutation"}),
                )
                .await;
            assert!(denied.is_error);
            let unchanged = self
                .state
                .db
                .collection::<crate::models::channel_bot::ChannelBot>(
                    crate::models::channel_bot::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":&target.id})
                .await?
                .unwrap();
            assert_eq!(
                unchanged.label, target.label,
                "foreign/shared-manager mutation refused"
            );
        }
        *self.stage.lock().await = "owned_bot_confirmation";
        let mut args =
            json!({"bot_id":owned_bot.id,"label":format!("account-owned-bot-{}-managed",user)});
        let mut managed = tools
            .execute(&auth, "nyxid__update_channel_bot", &args)
            .await;
        if *mode == AccessMode::AskEveryTime {
            assert_eq!(managed.value["error"], "acknowledgement_required");
            let card = managed.value["acknowledgement_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let unchanged = self
                .state
                .db
                .collection::<crate::models::channel_bot::ChannelBot>(
                    crate::models::channel_bot::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":&owned_bot.id})
                .await?
                .unwrap();
            assert_eq!(
                unchanged.label, owned_bot.label,
                "confirmation is required before the mutation"
            );
            acks::decide(&self.state.db, &user, &turn.conversation_id, &card, true).await?;
            args["acknowledgement_id"] = json!(card);
            managed = tools
                .execute(&auth, "nyxid__update_channel_bot", &args)
                .await;
            assert!(
                !managed.is_error,
                "confirmed owned bot change: {}",
                managed.value
            );
            let replay = tools
                .execute(&auth, "nyxid__update_channel_bot", &args)
                .await;
            assert!(
                replay.is_error,
                "a consumed confirmation cannot authorize another change"
            );
        } else {
            assert!(
                !managed.is_error,
                "normal permitted owned bot change: {}",
                managed.value
            );
        }
        *self.stage.lock().await = "owned_bot_verification";
        let changed = self
            .state
            .db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&owned_bot.id})
            .await?
            .unwrap();
        assert_eq!(changed.label, args["label"].as_str().unwrap());
        assert_eq!(changed.user_id, user);
        *self.stage.lock().await = "observed";
        self.observed.lock().await.push((
            user,
            turn.conversation_id.clone(),
            key.id,
            turn.turn_id.clone(),
        ));
        // The production implementation validates the credential/origin again,
        // resolves the live catalog, chooses the payer and performs the HTTP hop.
        *self.stage.lock().await = "owner_http_egress";
        self.inner.request(context, credential, request).await
    }
}

pub(super) async fn confirm_account(j: &Journey, sender: i64, account: &str, sequence: i64) {
    let login = j.control_as(sequence, "/login", sender).await.unwrap();
    let commands::PersonalCommandReply::Link { token } = login.reply else {
        panic!("fresh login")
    };
    let review = super::super::super::platform_link_intent_service::review(
        &j.state.db,
        j.state.auth_device_hmac_key.as_slice(),
        account,
        &token,
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let approved = super::super::super::platform_link_intent_service::approve(
        &j.state.db,
        j.state.auth_device_hmac_key.as_slice(),
        account,
        &review.id,
        &review.review_digest,
        false,
    )
    .await
    .unwrap();
    assert!(matches!(
        j.control_as(
            sequence + 1,
            approved.confirmation_command.as_ref().unwrap(),
            sender
        )
        .await
        .unwrap()
        .effect,
        commands::ManagerCommandEffect::Confirmed { .. }
    ));
}

async fn wait_observed(
    inspected: &InspectOwnerEgress,
    account: &str,
    count: usize,
) -> Vec<(String, String, String, String)> {
    let mut worker_state = (false, false, false);
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let failure = *inspected.failure.lock().await;
            assert!(failure.is_none(), "owner inspection failed: {failure:?}");
            let observed = inspected.observed.lock().await.clone();
            if observed.len() >= count {
                return observed;
            }
            if let Some(saved) = inspected
                .state
                .db
                .collection::<AssistantMessage>(MESSAGES)
                .find_one(doc! {"user_id":account,"role":"assistant","status":"failed"})
                .sort(doc! {"created_at":-1,"_id":-1})
                .await
                .unwrap()
            {
                panic!(
                    "personal turn failed before observation: code={:?}, stage={:?}, inspection={}",
                    saved.error_code,
                    saved
                        .error_detail
                        .as_ref()
                        .map(|detail| detail.stage.as_str()),
                    *inspected.stage.lock().await
                );
            }
            let row = inspected
                .state
                .db
                .collection::<crate::models::assistant_conversation::AssistantConversation>(
                    crate::models::assistant_conversation::COLLECTION_NAME,
                )
                .find_one(doc! {"user_id":account})
                .sort(doc! {"created_at":-1,"_id":-1})
                .await
                .unwrap();
            worker_state = (
                row.is_some(),
                row.as_ref().is_some_and(|row| row.active_turn.is_some()),
                row.as_ref()
                    .and_then(|row| row.active_turn.as_ref())
                    .is_some_and(|turn| turn.execution_started),
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    match result {
        Ok(observed) => observed,
        Err(_) => panic!(
            "owner observation timed out: inspection={}, conversation_exists={}, active_turn={}, execution_started={}",
            *inspected.stage.lock().await,
            worker_state.0,
            worker_state.1,
            worker_state.2
        ),
    }
}

async fn wait_reply(
    j: &Journey,
    account: &str,
    conversation: &str,
    turn: &str,
) -> AssistantMessage {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if let Some(row) = j.state.db.collection::<AssistantMessage>(MESSAGES)
                .find_one(doc! {"user_id":account,"conversation_id":conversation,"turn_id":turn,"role":"assistant"})
                .sort(doc! {"seq":-1,"_id":1}).await.unwrap() {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.unwrap()
}

#[tokio::test]
async fn two_accounts_keep_real_credentials_memory_tools_payers_and_web_continuity() {
    let mut j = Journey::new().await;
    j.state.config.billing_enabled = true;
    j.state.billing = Arc::new(crate::billing_integration_tests::billing_service(
        &j.state.db,
        Arc::new(crate::billing_integration_tests::FakeLago::default()),
        0,
    ));
    billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    assert!(
        j.state
            .db
            .collection::<Document>("billing_migrations")
            .find_one(doc! {"_id":"exact-v2","completed_at":{"$type":"date"}})
            .await
            .unwrap()
            .is_some(),
        "shared test fixture has completed exact-accounting readiness"
    );
    crate::db::ensure_indexes(&j.state.db).await.unwrap();
    let provider = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"model":"account-model",
            "output":[{"type":"message","content":[{"type":"output_text","text":"Paid by this account"}]}],
            "usage":{"input_tokens":20,"output_tokens":3}}))).expect(3).mount(&provider).await;
    // Each first owner exchange may start a best-effort title task. Unavailable
    // model discovery refuses before generation and settles its funding at zero.
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(404))
        .expect(0..=2)
        .mount(&provider)
        .await;
    let requests = Arc::new(Mutex::new(vec![]));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_url = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route("/v1/responses", axum::routing::post(model_then_reply))
        .with_state(PersonalModelCallback {
            state: j.state.clone(),
            requests: requests.clone(),
        });
    let upstream_task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut outer = crate::test_utils::test_auto_connected_catalog_service();
    outer.slug = engine::SERVICE_SLUG.into();
    outer.base_url = upstream_url;
    outer.forward_access_token = true;
    outer.streaming_supported = true;
    assert!(engine::row_contract(Some(&outer), None).valid());
    j.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&outer)
        .await
        .unwrap();
    let accounts = [
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    ];
    let mut model = crate::test_utils::test_auto_connected_catalog_service();
    model.slug = "personal-payer".into();
    model.base_url = format!("{}/v1", provider.uri());
    model.service_category = "internal".into();
    model.offering_kind = crate::models::downstream_service::OfferingKind::AiService;
    model.inference = Some(crate::models::downstream_service::ServiceInference {
        wire_protocol: crate::models::downstream_service::InferenceWireProtocol::OpenaiResponses,
        model_list: true,
        realtime: false,
        voice: None,
    });
    model.auth_method = "bearer".into();
    model.credential_encrypted = j
        .state
        .encryption_keys
        .encrypt(b"synthetic-inner-model-key")
        .await
        .unwrap();
    model.platform_key = Some(crate::models::downstream_service::PlatformKeyConfig {
        enabled: true,
        audience: crate::models::downstream_service::PlatformKeyAudience::Restricted,
        allowed_owner_ids: accounts.to_vec(),
    });
    model.billing = Some(serde_json::from_value(json!({"platform_billable":true,"platform_key_pricing":{
        "metric":"requests","credits_per_unit":"0.1","lago_metric_code":"personal_payer_request","sync_status":"synced"}})).unwrap());
    j.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&model)
        .await
        .unwrap();
    j.state
        .db
        .collection::<crate::models::billing_rate_cache::BillingRateCache>("billing_rate_cache")
        .insert_one(crate::models::billing_rate_cache::BillingRateCache {
            id: "personal_payer_request:*".into(),
            lago_metric_code: "personal_payer_request".into(),
            model: None,
            credits_per_unit_micros: 100_000,
            credits_per_unit_pico: Some(100_000_000_000),
            synced_at: Utc::now(),
            retired_at: None,
        })
        .await
        .unwrap();
    for owner in accounts.iter().chain(std::iter::once(&j.owner)) {
        let now = bson::DateTime::now();
        let wallet: BillingWallet = bson::from_document(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"owner_id":owner,"lago_customer_id":format!("customer-{owner}"),
            "lago_subscription_id":format!("subscription-{owner}"),"lago_wallet_id":format!("wallet-{owner}"),
            "plan_kind":"prepaid","balance_credits":Credits::from_whole(10).decimal(),"reserved_credits":Credits::ZERO.decimal(),
            "collection_state":"good","balance_synced_at":now,"created_at":now,"updated_at":now,
        }).unwrap();
        j.state
            .db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(wallet)
            .await
            .unwrap();
    }
    let mut specs = vec![];
    for (index, account) in accounts.iter().enumerate() {
        j.state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                account,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let mut owned_bot = j
            .state
            .db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&j.profile.channel_bot_id})
            .await
            .unwrap()
            .unwrap();
        owned_bot.id = uuid::Uuid::new_v4().to_string();
        owned_bot.user_id = account.clone();
        owned_bot.credential_source = "manual".into();
        owned_bot.platform = "telegram".into();
        owned_bot.platform_bot_id = (9000 + index).to_string();
        owned_bot.label = format!("account-owned-bot-{index}");
        let mut same_owner_manager = bson::to_document(&owned_bot).unwrap();
        same_owner_manager.insert("_id", uuid::Uuid::new_v4().to_string());
        same_owner_manager.insert("label", "same-owner-platform-manager");
        same_owner_manager.insert("platform_profile_id", j.profile.id.clone());
        j.state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(same_owner_manager)
            .await
            .unwrap();
        j.state
            .db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME,
            )
            .insert_one(owned_bot)
            .await
            .unwrap();
        let mut settings = AssistantSettings::defaults(account);
        settings.access_mode = if index == 0 {
            AccessMode::AskEveryTime
        } else {
            AccessMode::Autopilot
        };
        settings.timezone = Some(
            if index == 0 {
                "Asia/Singapore"
            } else {
                "Europe/London"
            }
            .into(),
        );
        j.state
            .db
            .collection::<AssistantSettings>(crate::models::assistant_settings::COLLECTION_NAME)
            .insert_one(&settings)
            .await
            .unwrap();
        let mut agent = team::ensure_nyxbot(&j.state.db, account).await.unwrap();
        agent.memory.push(MemoryNote {
            id: uuid::Uuid::new_v4().to_string(),
            text: format!("Private account {index} remembers lunar-orbit-{index}"),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        j.state
            .db
            .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
            .replace_one(doc! {"_id":&agent.id}, &agent)
            .await
            .unwrap();
        let service = connected(
            &j.state.db,
            account,
            &format!("account-{index}-only"),
            "https://example.test",
        )
        .await;
        specs.push((account.clone(), service, settings.access_mode));
    }
    for account in &accounts {
        crate::services::unified_key_service::auto_provision_no_auth_services(&j.state.db, account)
            .await
            .unwrap();
    }
    let grants = billing::grants::issue_grants(
        &j.state.db,
        billing::grants::IssueCreditGrantInput {
            amount: None,
            service_limits: Vec::new(),
            request_id: None,
            amount_credits: 1,
            target_kind: crate::models::billing_target::BillingTargetKind::SelectedUsers,
            target_user_ids: vec![accounts[0].clone()],
            target_org_ids: vec![],
            target_group_ids: vec![],
            all_services: false,
            service_refs: vec![model.id.clone()],
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            reason: Some("Account A personal model funding".into()),
            granted_by: j.owner.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(grants.len(), 1);
    assert!(grants[0].issued_ledgered_at.is_some());
    let identity = bindings::producer(j.state.clone());
    let inspected = Arc::new(InspectOwnerEgress {
        state: j.state.clone(),
        inner: crate::services::assistant_nyxagent_egress::producer(
            j.state.clone(),
            identity,
            crate::handlers::agent_skills::preload_context(j.state.clone()),
        ),
        accounts: specs,
        observed: Mutex::new(vec![]),
        stage: Mutex::new("not_entered"),
        failure: Mutex::new(None),
    });
    let public = crate::services::platform_public_credential_service::producer(j.state.clone());
    let execution = crate::services::assistant_nyxagent_runtime::producer(
        j.state.clone(),
        inspected.clone(),
        public,
    );
    let mut producers = assemble(
        j.state.clone(),
        execution,
        inspected.clone(),
        Arc::new(TestSettledHook),
    );
    let identity = producers.identity.clone();
    Arc::get_mut(&mut producers).unwrap().transport = Arc::new(LocalTransport {
        state: j.state.clone(),
        identity,
        base: j.server.uri(),
        personal_gate: None,
    });
    j.producers = producers;
    for (index, account) in accounts.iter().enumerate() {
        let sender = 700 + index as i64;
        confirm_account(&j, sender, account, 4000 + index as i64 * 10).await;
        let mut payload = j.update(4100 + index as i64, "Use my remembered preference", false);
        payload["message"]["from"]["id"] = json!(sender);
        payload["message"]["chat"]["id"] = json!(sender);
        j.webhook(&payload).await;
        let observed = wait_observed(&inspected, account, index + 1).await;
        let captured = &observed[index];
        assert_eq!(captured.0, *account);
        let saved = wait_reply(&j, account, &captured.1, &captured.3).await;
        assert_eq!(saved.text, "Account-scoped answer");
        let row = engine::get(&j.state.db, account, &saved.conversation_id)
            .await
            .unwrap();
        assert_eq!(row.user_id, *account);
        assert!(engine::get(&j.state.db, &j.owner, &row.id).await.is_err());
        assert!(
            engine::get(&j.state.db, &accounts[1 - index], &row.id)
                .await
                .is_err()
        );
        let credential = credentials::load_for_conversation(
            &j.state.db,
            &j.state.encryption_keys,
            account,
            &row.id,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(credential.api_key_id, row.credential_api_key_id);
    }
    let observed = inspected.observed.lock().await.clone();
    assert_eq!(observed.len(), 2);
    assert_ne!(observed[0].2, observed[1].2);
    let owner_row = engine::get(&j.state.db, &accounts[0], &observed[0].1)
        .await
        .unwrap();
    let web = engine::TurnStart::from(&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: Some(owner_row.id.clone()),
        agent_id: None,
        text: "Continue the same conversation from the website".into(),
        model: None,
        access_mode: None,
    });
    let permit = j
        .state
        .direct_chat_limiter
        .try_acquire(&accounts[0])
        .await
        .unwrap();
    let (web_row, mut events) = crate::handlers::assistant_nyxagent::start_turn(
        &j.state,
        crate::test_utils::test_auth_user(&accounts[0]),
        &web,
        Some(crate::handlers::assistant_nyxagent::SERVER_TURN_POLICY),
        permit,
    )
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while let Ok(event) = events.recv().await {
            if event["event"] == "turn.completed" {
                break;
            }
        }
    })
    .await
    .unwrap();
    let web_turn = web_row.active_turn.as_ref().unwrap();
    let continued = wait_reply(&j, &accounts[0], &owner_row.id, &web_turn.turn_id).await;
    assert!(continued.seq > 1);
    assert_eq!(continued.conversation_id, owner_row.id);
    let final_row = engine::get(&j.state.db, &accounts[0], &owner_row.id)
        .await
        .unwrap();
    assert_eq!(
        final_row.credential_api_key_id,
        owner_row.credential_api_key_id
    );
    let captured_requests = requests.lock().await.clone();
    assert_eq!(captured_requests.len(), 3);
    for (index, (user, key, body)) in captured_requests.iter().enumerate() {
        let owner_index = if index == 1 { 1 } else { 0 };
        assert_eq!(user, &accounts[owner_index]);
        assert_eq!(key, &observed[owner_index].2);
        assert!(
            body.to_string()
                .contains(&format!("lunar-orbit-{owner_index}"))
        );
        assert!(
            !body
                .to_string()
                .contains(&format!("lunar-orbit-{}", 1 - owner_index))
        );
        if index == 2 {
            assert_eq!(
                body["conversation"],
                "conv_1234567890abcdef1234567890abcdef"
            );
        }
    }
    let (inner_meters, metadata_meters) =
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let provider_requests = provider.received_requests().await.unwrap();
                let mut model_requests = 0;
                let mut model_list_requests = 0;
                for request in &provider_requests {
                    assert!(request.url.query().is_none());
                    match (request.method.as_str(), request.url.path()) {
                        ("POST", "/v1/responses") => {
                            model_requests += 1;
                            assert_eq!(
                                request.body_json::<Value>().unwrap(),
                                json!({
                                    "model":"account-model", "input":"Use my account context",
                                    "max_output_tokens":16, "stream":false, "store":false,
                                })
                            );
                        }
                        ("GET", "/v1/models") => {
                            model_list_requests += 1;
                            assert!(request.body.is_empty());
                        }
                        _ => panic!("unexpected provider request method or path"),
                    }
                }
                assert_eq!(model_requests, 3);
                assert!(model_list_requests <= accounts.len());
                let rows = j
                    .state
                    .db
                    .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
                    .find(doc! {"service_id":&model.id})
                    .await
                    .unwrap()
                    .try_collect::<Vec<_>>()
                    .await
                    .unwrap();
                let (paid, metadata): (Vec<_>, Vec<_>) =
                    rows.iter().partition(|m| m.api_key_id.is_some());
                assert!(paid.len() <= 3);
                assert!(metadata.len() <= accounts.len());
                if paid.len() == 3
                    && metadata.len() == model_list_requests
                    && rows.iter().all(|m| {
                        m.status == UsageStatus::Finalized
                            && m.funding.as_ref().is_some_and(|f| f.settled)
                    })
                {
                    break rows
                        .into_iter()
                        .partition::<Vec<_>, _>(|m| m.api_key_id.is_some());
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    let mut metadata_owners = std::collections::BTreeSet::new();
    for meter in &metadata_meters {
        assert!(accounts.contains(&meter.billing_owner_id));
        assert_eq!(meter.actor_user_id, meter.billing_owner_id);
        assert_ne!(meter.billing_owner_id, j.owner);
        assert!(metadata_owners.insert(&meter.billing_owner_id));
        assert!(meter.api_key_id.is_none());
        assert!(meter.model.is_none());
        assert_eq!(meter.layer, BillingLayer::Platform);
        assert_eq!(meter.metric, BillingMetric::Requests);
        assert_eq!(meter.credential_class, CredentialClass::NyxidManagedMaster);
        assert!(meter.forwarded);
        assert_eq!(meter.quantity, Some(0));
        let funding = meter.funding.as_ref().unwrap();
        assert!(funding.settled);
        assert_eq!(funding.total_charge, Some(Credits::ZERO));
        assert_eq!(funding.grant_funded, Some(Credits::ZERO));
        assert_eq!(funding.wallet_funded, Some(Credits::ZERO));
        assert_eq!(funding.allowance_funded, Some(Credits::ZERO));
        assert!(funding.grant_consumptions.iter().all(|c| {
            c.amount == Credits::ZERO
                && c.grant_id == grants[0].id
                && meter.billing_owner_id == accounts[0]
        }));
        assert!(funding.allowance_consumptions.is_empty());
    }
    assert_eq!(inner_meters.len(), 3);
    for meter in &inner_meters {
        assert_eq!(meter.layer, BillingLayer::Platform);
        assert_eq!(meter.metric, BillingMetric::Requests);
        assert_eq!(meter.quantity, Some(1));
        let index = accounts
            .iter()
            .position(|a| a == &meter.billing_owner_id)
            .unwrap();
        assert_eq!(meter.actor_user_id, accounts[index]);
        assert_ne!(meter.billing_owner_id, j.owner);
        assert_eq!(
            meter.api_key_id.as_deref(),
            Some(observed[index].2.as_str())
        );
        assert_eq!(meter.credential_class, CredentialClass::NyxidManagedMaster);
        let f = meter.funding.as_ref().unwrap();
        assert_eq!(f.total_charge, Some("0.1".parse().unwrap()));
        assert_eq!(
            f.grant_funded,
            Some(if index == 0 {
                "0.1".parse().unwrap()
            } else {
                Credits::ZERO
            })
        );
        assert_eq!(
            f.wallet_funded,
            Some(if index == 1 {
                "0.1".parse().unwrap()
            } else {
                Credits::ZERO
            })
        );
        assert!(
            f.grant_consumptions
                .iter()
                .all(|c| c.grant_id == grants[0].id && index == 0)
        );
    }
    assert_eq!(
        inner_meters
            .iter()
            .filter(|m| m.billing_owner_id == accounts[0])
            .count(),
        2
    );
    let remaining = j
        .state
        .db
        .collection::<crate::models::credit_grant::CreditGrant>(
            crate::models::credit_grant::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&grants[0].id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(remaining.remaining, "0.8".parse().unwrap());
    for (owner, pending) in [(&accounts[0], "0"), (&accounts[1], "0.1"), (&j.owner, "0")] {
        let wallet = j
            .state
            .db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! {"owner_id":owner})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(wallet.balance_credits, Credits::from_whole(10));
        assert_eq!(wallet.pending_lago_debits, pending.parse().unwrap());
        assert_eq!(
            wallet.available_credits().unwrap(),
            Credits::from_whole(10)
                .checked_sub(pending.parse().unwrap())
                .unwrap()
        );
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
    }
    let meters = j
        .state
        .db
        .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
        .find(doc! {"service_id":&outer.id})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(meters.len(), 3);
    for meter in &meters {
        assert!(accounts.contains(&meter.billing_owner_id));
        assert_eq!(meter.actor_user_id, meter.billing_owner_id);
        assert_ne!(meter.billing_owner_id, j.owner);
        assert_eq!(meter.credential_class, CredentialClass::NoAuth);
        assert_eq!(meter.status, UsageStatus::Finalized);
    }
    // Native owner egress bills with its conversation key. The browser proxy
    // retains human authentication even while forwarding that key to NyxAgent.
    let mut identities = meters
        .iter()
        .map(|meter| (meter.billing_owner_id.as_str(), meter.api_key_id.as_deref()))
        .collect::<Vec<_>>();
    identities.sort_unstable();
    let mut expected = [
        (accounts[0].as_str(), Some(observed[0].2.as_str())),
        (accounts[1].as_str(), Some(observed[1].2.as_str())),
        (accounts[0].as_str(), None),
    ];
    expected.sort_unstable();
    assert_eq!(identities, expected);
    upstream_task.abort();
    j.state.db.drop().await.unwrap();
}
