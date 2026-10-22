use super::*;
use crate::{
    handlers::nyxbot,
    models::{
        platform_entry_identity::EntryPlatform,
        platform_manager_delivery::{
            COLLECTION_NAME as DELIVERIES, DeliveryState, PlatformManagerDelivery,
        },
        platform_manager_ingress::COLLECTION_NAME as INGRESS,
    },
    services::{
        platform_entry_native::tests as native,
        platform_native_reply_service::{NativeTestTransport, with_transport_for_test},
    },
};
use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::Response,
    routing::{post, put},
};
use chrono::Utc;
use std::time::Duration;
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct Settled;
#[async_trait::async_trait]
impl crate::services::assistant_nyxagent_runtime::OwnerTurnSettledHook for Settled {
    async fn settled(
        &self,
        _: crate::models::assistant_conversation::AssistantConversation,
        _: crate::services::platform_manager_contract::ReplyRef,
    ) {
    }
}

struct Fixture {
    state: AppState,
    row: PlatformEntryGateway,
    provider: ApiKey,
    raw: Zeroizing<String>,
    request: Value,
    transport: Arc<NativeTestTransport>,
    router: Router,
}

impl Fixture {
    async fn new(server: &MockServer) -> Self {
        let base = server.uri();
        let (state, bot, original) =
            crate::services::platform_native_reply_service::tests::discord_fixture(&base).await;
        ensure_indexes(&state.db).await.unwrap();
        crate::services::platform_manager_ingress_service::ensure_indexes(&state.db)
            .await
            .unwrap();
        crate::services::platform_manager_delivery::ensure_indexes(&state.db)
            .await
            .unwrap();
        let exported: Value = serde_json::from_str(include_str!(
            "../../../docs/fixtures/platform-entry-cma/discord-verified-activity-input.json"
        ))
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let route = exported["activity"]["source"]["route_id"].as_str().unwrap();
        let mut configuration = original.configuration;
        configuration.transport = EntryTransport::NyxagentGateway {
            channel_id: id.clone(),
            gateway_channel_id: exported["channel_id"].as_str().unwrap().into(),
            gateway_version: 1,
        };
        let (provider, raw) = transport_key(&bot.user_id);
        let (mut route_key, _) = transport_key(&bot.user_id);
        let callback = format!("{base}/relay-callback");
        route_key.callback_url = Some(callback.clone());
        let policy = request_policy(
            &state.config.jwt_issuer,
            &bot.user_id,
            &id,
            &route_key.id,
            route,
            EntryPlatform::Discord,
        )
        .unwrap();
        let now = Utc::now();
        let row = PlatformEntryGateway {
            id,
            profile_id: original.profile_id.clone(),
            configuration: configuration.clone(),
            configured_by: bot.user_id.clone(),
            credential_owner_id: bot.user_id.clone(),
            route_api_key_id: route_key.id.clone(),
            route_id: route.into(),
            provider_api_key_id: provider.id.clone(),
            callback_url: callback,
            binding_id: None,
            gateway_base: base.clone(),
            setup_state: "observed".into(),
            setup_attempt_id: uuid::Uuid::new_v4().to_string(),
            setup_dispatch_deadline: None,
            request_hash: sha256(canonical_json(policy.clone()).as_bytes()),
            creation_fingerprint: "fixture-observed".into(),
            request_policy: policy,
            request_ciphertext: vec![],
            state: "active".into(),
            revision: 1,
            created_at: now,
            updated_at: now,
        };
        // This fixture starts at an already activated transport. Setup/recovery
        // has separate authority tests; here both real keys and receipt fences apply.
        for key in [&provider, &route_key] {
            crate::services::api_key_mutation_service::insert_one(&state.db, key, None)
                .await
                .unwrap();
        }
        state
            .db
            .collection::<PlatformEntryGateway>(GATEWAYS)
            .insert_one(&row)
            .await
            .unwrap();
        state
            .db
            .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.profile_id},
                doc! {"$set":{"native_entry":bson::to_bson(&configuration).unwrap()}},
            )
            .await
            .unwrap();
        let adapter = Arc::new(
            crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(&base),
        );
        let signing = ed25519_dalek::SigningKey::from_bytes(&[37; 32]);
        let mut interaction = native::discord_message("status", None);
        interaction["user"]["id"] = exported["activity"]["actor"]["id"].clone();
        interaction["channel_id"] = exported["activity"]["conversation"]["id"].clone();
        interaction["channel"]["id"] = interaction["channel_id"].clone();
        let body = serde_json::to_vec(&interaction).unwrap();
        let headers = native::discord_headers(&signing, &body, Utc::now().timestamp());
        let batch = crate::services::platform_entry_native::prepare(
            &state.db,
            &state.encryption_keys,
            adapter.as_ref(),
            &bot,
            &headers,
            &body,
            &configuration,
        )
        .await
        .unwrap();
        let mut session = state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let admission = crate::services::platform_entry_receipts::record_in_session(
            &state.db,
            &mut session,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            crate::services::platform_entry_receipts::NativeReceiptInput {
                profile_id: &row.profile_id,
                epoch: 1,
                configuration: &configuration,
                batch: &batch,
                index: 0,
                relay_message_id: exported["activity"]["event_id"].as_str().unwrap(),
            },
        )
        .await
        .unwrap();
        session.commit_transaction().await.unwrap();
        assert_eq!(admission.text(), "/status");
        assert_eq!(exported["activity"]["actor"]["kind"], "unknown");
        let partition = format!("conv_{}", uuid::Uuid::new_v4().simple());
        let request = json!({"stream":true,"conversation":partition,"input":exported["input"],
            "event_context":{"schema_version":1,"channel_id":exported["channel_id"],
                "conversation_id":partition,"activity":exported["activity"]}});
        let producers = crate::services::platform_manager_transport::compose(
            state.clone(),
            Arc::new(Settled),
            crate::handlers::agent_skills::preload_context(state.clone()),
        );
        let router = Router::new()
            .route(
                "/bindings/{binding}",
                put(nyxbot::put_binding).delete(nyxbot::delete_binding),
            )
            .route(
                "/bindings/{binding}/conversations/{conversation}",
                put(nyxbot::put_conversation).delete(nyxbot::delete_conversation),
            )
            .route("/responses", post(nyxbot::responses_with_runtime))
            .layer(Extension(producers))
            .with_state(state.clone());
        Self {
            state,
            row,
            provider,
            raw,
            request,
            transport: Arc::new(NativeTestTransport {
                adapter,
                discord_base: base,
            }),
            router,
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", self.raw.as_str()).parse().unwrap(),
        );
        headers
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> Response {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {}", self.raw.as_str()))
            .header("content-type", "application/json")
            .header("idempotency-key", "native-retained-event")
            .body(body.map_or_else(Body::empty, |body| {
                Body::from(serde_json::to_vec(&body).unwrap())
            }))
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(30),
            with_transport_for_test(self.transport.clone(), self.router.clone().oneshot(request)),
        )
        .await
        .expect("gateway handler deadline")
        .unwrap()
    }

    async fn bind(&self) {
        let body = json!({"agent_key":self.raw.as_str(),"profile":self.row.request_policy["profile"],
            "profile_digest":sha256(canonical_json(self.row.request_policy["profile"].clone()).as_bytes())});
        assert_eq!(
            self.call("PUT", "/bindings/native-binding", Some(body))
                .await
                .status(),
            StatusCode::OK
        );
    }

    async fn ensure_conversation(&self) {
        let uri = format!(
            "/bindings/native-binding/conversations/{}",
            self.request["conversation"].as_str().unwrap()
        );
        assert_eq!(self.call("PUT", &uri, None).await.status(), StatusCode::OK);
    }

    async fn ingress_count(&self) -> u64 {
        self.state
            .db
            .collection::<Document>(INGRESS)
            .count_documents(doc! {"profile_id":&self.row.profile_id})
            .await
            .unwrap()
    }
}

async fn json_body(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn actual_gateway_handlers_create_first_partition_then_accept_one_native_control_reply() {
    let server = MockServer::start().await;
    let fixture = Fixture::new(&server).await;
    fixture.bind().await;
    assert!(fixture.provider.scopes.is_empty());
    assert!(fixture.provider.allowed_service_ids.is_empty());
    assert!(fixture.provider.assistant_agent_owner_id.is_none());
    assert!(
        crate::services::key_service::require_ordinary_purpose(fixture.provider.purpose).is_err()
    );
    assert_eq!(
        fixture
            .state
            .db
            .collection::<Document>(PARTITIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let first = fixture
        .call("POST", "/responses", Some(fixture.request.clone()))
        .await;
    assert_eq!(first.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(first).await["code"], "conversation_not_found");
    assert_eq!(fixture.ingress_count().await, 0);
    assert!(server.received_requests().await.unwrap().is_empty());
    let mut forged = fixture.request.clone();
    forged["input"][0]["content"][0]["text"] = json!("/logout");
    assert!(
        !fixture
            .call("POST", "/responses", Some(forged))
            .await
            .status()
            .is_success()
    );
    assert_eq!(fixture.ingress_count().await, 0);
    // The provider's typed 404 is CMA's signal to ensure the first partition.
    fixture.ensure_conversation().await;
    Mock::given(method("POST"))
        .and(path("/webhooks/100/synthetic-interaction-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"gateway-native-sent"})))
        .expect(1)
        .mount(&server)
        .await;
    let response = fixture
        .call("POST", "/responses", Some(fixture.request.clone()))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let stream = String::from_utf8(
        to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let frames: Vec<Value> = stream
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str(data).ok())
        .collect();
    assert_eq!(frames[0]["type"], "response.created");
    assert_eq!(frames[0]["response"]["status"], "in_progress");
    let completed = frames
        .iter()
        .find(|frame| frame["type"] == "response.completed")
        .unwrap();
    assert_eq!(frames[0]["response"]["id"], completed["response"]["id"]);
    assert_eq!(completed["response"]["output"], json!([]));
    assert_eq!(fixture.ingress_count().await, 1);
    let sent = fixture
        .state
        .db
        .collection::<PlatformManagerDelivery>(DELIVERIES)
        .find_one(doc! {"profile_id":&fixture.row.profile_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sent.state, DeliveryState::Sent);
    assert_eq!(sent.platform_message_ids, ["gateway-native-sent"]);
    assert!(sent.telegram_message_ids.is_empty());
    assert!(
        fixture
            .call("POST", "/responses", Some(fixture.request.clone()))
            .await
            .status()
            .is_success()
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "all native/generic send routes point to this local server"
    );
    assert_eq!(
        requests[0].url.path(),
        "/webhooks/100/synthetic-interaction-token"
    );
    assert!(requests[0].headers.get("authorization").is_none());
    for collection in [
        crate::models::nyxbot_channel::THREADS_COLLECTION_NAME,
        crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME,
        crate::models::assistant_conversation::COLLECTION_NAME,
    ] {
        assert_eq!(
            fixture
                .state
                .db
                .collection::<Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "ordinary owner state: {collection}"
        );
    }
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn authenticated_gateway_allows_benign_touches_but_fences_rotation_revocation_and_retirement()
{
    for change in ["binding", "provider", "route"] {
        let server = MockServer::start().await;
        let fixture = Fixture::new(&server).await;
        fixture.bind().await;
        fixture.ensure_conversation().await;
        // Authentication returns a post-touch snapshot. Later benign touches
        // must preserve this authority without guessing the next state version.
        let authority = authenticate(&fixture.state, &fixture.headers())
            .await
            .unwrap()
            .unwrap();
        let touch =
            crate::services::key_service::validate_api_key(&fixture.state.db, fixture.raw.as_str());
        let touch2 =
            crate::services::key_service::validate_api_key(&fixture.state.db, fixture.raw.as_str());
        let (one, two) = tokio::join!(touch, touch2);
        one.unwrap();
        two.unwrap();
        conversation(
            &fixture.state,
            authority,
            "native-binding",
            fixture.request["conversation"].as_str().unwrap(),
        )
        .await
        .unwrap();
        let before_rotation = fixture
            .state
            .db
            .collection::<ApiKey>(KEYS)
            .find_one(doc! {"_id":&fixture.provider.id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            crate::services::key_service::rotate_api_key(
                &fixture.state.db,
                &fixture.state.encryption_keys,
                &fixture.provider.user_id,
                &fixture.provider.id
            )
            .await
            .is_err()
        );
        let after_rotation = fixture
            .state
            .db
            .collection::<ApiKey>(KEYS)
            .find_one(doc! {"_id":&fixture.provider.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            before_rotation, after_rotation,
            "ordinary rotation cannot replace a shared transport key"
        );
        let gate = Arc::new(NativeDispatchGate {
            entered: Default::default(),
            release: Default::default(),
        });
        let run = BEFORE_NATIVE_DISPATCH.scope(
            gate.clone(),
            fixture.call("POST", "/responses", Some(fixture.request.clone())),
        );
        tokio::pin!(run);
        tokio::select! {
            entered = tokio::time::timeout(Duration::from_secs(20), gate.entered.notified()) => entered.expect("actual gateway verification must reach the dispatch gate"),
            _ = &mut run => panic!("gateway returned before native dispatch gate"),
        }
        if change == "binding" {
            assert_eq!(
                fixture
                    .call("DELETE", "/bindings/native-binding", None)
                    .await
                    .status(),
                StatusCode::NO_CONTENT
            );
        } else {
            let id = if change == "route" {
                &fixture.row.route_api_key_id
            } else {
                &fixture.row.provider_api_key_id
            };
            let update = doc! {"$set":{"is_active":false}};
            let changed = crate::services::api_key_mutation_service::update_one(
                &fixture.state.db,
                doc! {"_id":id},
                update,
                None,
            )
            .await
            .unwrap();
            assert_eq!(changed.matched_count, 1);
        }
        gate.release.notify_one();
        let response = tokio::time::timeout(Duration::from_secs(30), &mut run)
            .await
            .expect("gateway resume deadline");
        assert!(!response.status().is_success());
        assert_eq!(
            fixture.ingress_count().await,
            0,
            "{change} must fence before command effects"
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(
            fixture
                .state
                .db
                .collection::<Document>(crate::models::platform_link_intent::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        fixture.state.db.drop().await.unwrap();
    }
}

#[path = "platform_entry_gateway_setup_tests.rs"]
pub(crate) mod setup;

#[tokio::test]
async fn retired_native_primary_acknowledges_only_exact_delete_without_restoring_authority() {
    use axum::extract::{Path, State};
    let server = MockServer::start().await;
    let (state, bot, receipt) =
        crate::services::platform_native_reply_service::tests::discord_fixture(&server.uri()).await;
    ensure_indexes(&state.db).await.unwrap();
    let (mut provider, raw) = transport_key(&bot.user_id);
    provider.is_active = false;
    crate::services::api_key_mutation_service::insert_one(&state.db, &provider, None)
        .await
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let mut configuration = receipt.configuration;
    configuration.transport = EntryTransport::NyxagentGateway {
        channel_id: id.clone(),
        gateway_channel_id: "cmaeg1.ch.retired-native".into(),
        gateway_version: 1,
    };
    let now = Utc::now();
    let row = PlatformEntryGateway {
        id,
        profile_id: receipt.profile_id,
        configuration,
        configured_by: bot.user_id.clone(),
        credential_owner_id: bot.user_id,
        route_api_key_id: uuid::Uuid::new_v4().to_string(),
        route_id: uuid::Uuid::new_v4().to_string(),
        provider_api_key_id: provider.id,
        callback_url: format!("{}/callback", server.uri()),
        binding_id: Some("native-binding".into()),
        gateway_base: server.uri(),
        setup_state: "observed".into(),
        setup_attempt_id: uuid::Uuid::new_v4().to_string(),
        setup_dispatch_deadline: None,
        request_policy: json!({}),
        request_hash: "retired-captured-request".into(),
        creation_fingerprint: "retired-captured-identity".into(),
        request_ciphertext: vec![],
        state: "retired".into(),
        revision: 1,
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .collection::<PlatformEntryGateway>(GATEWAYS)
        .insert_one(&row)
        .await
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {}", raw.as_str()).parse().unwrap(),
    );
    assert_eq!(
        nyxbot::delete_binding(
            State(state.clone()),
            Path("native-binding".into()),
            headers.clone()
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_ne!(
        nyxbot::delete_binding(
            State(state.clone()),
            Path("wrong-binding".into()),
            headers.clone()
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert!(authenticate(&state, &headers).await.is_err());
    assert_ne!(
        nyxbot::put_binding(
            State(state.clone()),
            Path("native-binding".into()),
            headers,
            axum::Json(json!({}))
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        state
            .db
            .collection::<PlatformEntryGateway>(GATEWAYS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap()
            .state,
        "retired"
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}
