use super::{binding, coordinator, profiles, protocol, session};
use crate::{
    AppState,
    models::{
        assistant_agent::{AgentKind, AssistantAgent, GuestAccess},
        assistant_conversation::AssistantConversation,
        channel_adapter::*,
        downstream_service::DownstreamService,
        user::UserType,
    },
    services::{
        assistant_nyxagent as engine, assistant_team_service as team, channel_adapters,
        channel_capability,
    },
    test_utils::{
        connect_transaction_test_database, test_app_state, test_user, test_user_endpoint,
        test_user_service,
    },
};
use axum::{
    Json, Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
    response::IntoResponse,
    routing::{any, get, post},
};
use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Mutex, RwLock, Semaphore};
use tower::ServiceExt;
use uuid::Uuid;

fn binding(profile: Profile, owner: &str) -> Binding {
    let now = Utc::now();
    Binding {
        id: Uuid::new_v4().to_string(),
        user_id: owner.into(),
        profile,
        contract_version: 1,
        revision: 1,
        label: "FAQ".into(),
        agent_id: Uuid::new_v4().to_string(),
        credential: CredentialBinding::Connection {
            user_service_id: Uuid::new_v4().to_string(),
        },
        account_id: format!("AC{}", Uuid::new_v4().simple()),
        resource_id: format!("PN{}", Uuid::new_v4().simple()),
        phone_number: "+18005550123".into(),
        application_id: Uuid::new_v4().to_string(),
        webhook_public_key: None,
        voice: "en-US-Standard-A".into(),
        language: "en-US".into(),
        enabled: true,
        verified_credential_identity: String::new(),
        trusted_account_revision: None,
        verified_at: now,
        created_at: now,
        updated_at: now,
    }
}
#[test]
fn twilio_public_sid_cannot_carry_auth_token_or_path_data() {
    let digits = format!("{:032x}", 0x0123456789abcdef0123456789abcdef_u128);
    let public_sid = format!("AC{digits}");
    let sid = protocol::TwilioAccountSid::parse(&public_sid).unwrap();
    assert_eq!(sid.to_string(), public_sid);
    let raw = zeroize::Zeroizing::new(format!("{public_sid}:fixture-auth-token"));
    let (routing, token) = protocol::twilio_key(&raw, &public_sid).unwrap();
    assert!(routing == sid);
    assert!(token == "fixture-auth-token");
    for invalid in [
        "ACshort".to_owned(),
        digits.clone(),
        format!("SK{digits}"),
        format!("AC{}g", &digits[..31]),
        format!("AC{}", digits.to_ascii_uppercase()),
        format!("AC{}F{}", &digits[..15], &digits[16..]),
        format!("{public_sid}:fixture-auth-token"),
        format!("{public_sid}/Calls"),
        format!("{public_sid}?token=fixture"),
    ] {
        assert!(protocol::TwilioAccountSid::parse(&invalid).is_err());
    }
    assert!(
        protocol::twilio_key(
            &format!("AC{:032x}:fixture-auth-token", 1_u128),
            &public_sid,
        )
        .is_err()
    );
    assert!(protocol::twilio_key(&format!("{public_sid}:"), &public_sid).is_err());
}
#[test]
fn provider_request_urls_allow_only_compiled_https_or_test_loopback() {
    for (profile, base) in [
        (Profile::TelnyxSms, "https://api.telnyx.com"),
        (Profile::TwilioRelay, "https://api.twilio.com"),
        (Profile::ElevenlabsCustomLlm, "https://api.elevenlabs.io"),
    ] {
        let url = profiles::provider_url(profile, base, "/resource?cursor=123").unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.path(), "/resource");
        assert_eq!(url.query(), Some("cursor=123"));
        for path in [
            "https://elsewhere.invalid/resource",
            "//elsewhere.invalid/resource",
            "/\\elsewhere.invalid/resource",
            "/resource#fragment",
            "resource",
        ] {
            assert!(profiles::provider_url(profile, base, path).is_err());
        }
        for invalid in [
            "http://api.twilio.com",
            "https://elsewhere.invalid",
            "http://localhost:12345",
            "http://127.0.0.1:12345@elsewhere.invalid",
            "http://user@127.0.0.1:12345",
            "http://127.0.0.1:12345/path",
            "http://127.0.0.1:12345?query=1",
        ] {
            assert!(profiles::provider_url(profile, invalid, "/resource").is_err());
        }
        for fixture in ["http://127.0.0.1:12345", "http://[::1]:12345"] {
            assert!(profiles::provider_url(profile, fixture, "/resource").is_ok());
        }
    }
}
async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (origin, task)
}
async fn connection(
    state: &AppState,
    owner: &str,
    slug: &str,
    url: &str,
    auth: &str,
    raw: &str,
) -> String {
    let id = Uuid::new_v4().to_string();
    let endpoint = Uuid::new_v4().to_string();
    let kid = Uuid::new_v4().to_string();
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = slug.into();
    service.base_url = url.into();
    service.auth_method = auth.into();
    service.requires_user_credential = true;
    service.credential_encrypted.clear();
    service.service_category = "public".into();
    service.identity_propagation_mode = "none".into();
    service.forward_access_token = false;
    service.inject_delegation_token = false;
    state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(&service)
        .await
        .unwrap();
    state
        .db
        .collection(crate::models::user_endpoint::COLLECTION_NAME)
        .insert_one(test_user_endpoint(
            &endpoint,
            owner,
            slug,
            url,
            None,
            Some(&service.id),
        ))
        .await
        .unwrap();
    let mut row = test_user_service(&id, owner, slug, &endpoint, Some(&service.id), None);
    row.auth_method = auth.into();
    row.api_key_id = Some(kid.clone());
    state
        .db
        .collection(crate::models::user_service::COLLECTION_NAME)
        .insert_one(row)
        .await
        .unwrap();
    let encrypted = state.encryption_keys.encrypt(raw.as_bytes()).await.unwrap();
    state.db.collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME).insert_one(doc!{"_id":kid,"user_id":owner,"label":slug,"credential_type":auth,"credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},"status":"active","credential_epoch":1_i64,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    id
}
async fn persist_binding(state: &AppState, b: &Binding) {
    let mut tx = state.db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    binding::claim_resources(&state.db, &mut tx, b)
        .await
        .unwrap();
    state
        .db
        .collection::<Binding>(BINDINGS)
        .insert_one(b)
        .session(&mut tx)
        .await
        .unwrap();
    tx.commit_transaction().await.unwrap();
}
struct Fixture {
    state: AppState,
    b: Binding,
    faq: Arc<RwLock<String>>,
    reads: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Value>>>,
    callback_codes: Arc<Mutex<Vec<u16>>>,
    upstream_finished: tokio_util::sync::CancellationToken,
    gate: Arc<Semaphore>,
    chunks: Arc<AtomicUsize>,
    chunk_delay_ms: Arc<AtomicUsize>,
    fail_turn: Arc<AtomicBool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        profiles::FIXTURE_ORIGINS
            .lock()
            .unwrap()
            .remove(&self.b.account_id);
    }
}
async fn fixture(profile: Profile, gated: bool) -> Fixture {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let db = connect_transaction_test_database("adapter_faq").await;
    engine::ensure_indexes(&db).await.unwrap();
    session::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db);
    crate::services::feature_flag_service::set_platform_override(
        &state.db,
        crate::services::feature_flag_service::NYXBOT_ADAPTERS_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::User(owner.clone()),
        true,
        &owner,
    )
    .await
    .unwrap();
    let faq = Arc::new(RwLock::new("The office opens at nine.".to_string()));
    let reads = Arc::new(AtomicUsize::new(0));
    let f = faq.clone();
    let n = reads.clone();
    let (notion_origin, notion_task) = serve(Router::new().route(
        "/faq",
        get(move || {
            let f = f.clone();
            let n = n.clone();
            async move {
                n.fetch_add(1, Ordering::SeqCst);
                Json(json!({"faq":*f.read().await}))
            }
        }),
    ))
    .await;
    let notion = connection(
        &state,
        &owner,
        "api-notion",
        &notion_origin,
        "bearer",
        "notion-fixture",
    )
    .await;
    let mut b = binding(profile, &owner);
    let auth = if profile == Profile::ElevenlabsCustomLlm {
        "header"
    } else if profiles::provider(profile) == "twilio" {
        "basic"
    } else {
        "bearer"
    };
    let raw = if auth == "basic" {
        format!("{}:fixture-auth-token", b.account_id)
    } else {
        "carrier-fixture".into()
    };
    let carrier_connection = connection(
        &state,
        &owner,
        profiles::provider(profile),
        if profile == Profile::ElevenlabsCustomLlm {
            "https://api.elevenlabs.io"
        } else if auth == "basic" {
            "https://api.twilio.com"
        } else {
            "https://api.telnyx.com/v2"
        },
        auth,
        &raw,
    )
    .await;
    b.credential = CredentialBinding::Connection {
        user_service_id: carrier_connection,
    };
    let mut agent = team::ensure_nyxbot(&state.db, &owner).await.unwrap();
    agent.id = b.agent_id.clone();
    agent.kind = AgentKind::Specialist;
    agent.name = "faq".into();
    agent.persona = Some("TRUSTED_ROLE_ONE. Read the FAQ on every question.".into());
    agent.grants.service_ids = vec![notion.clone()];
    agent.guest_access.insert(notion.clone(), GuestAccess::Read);
    state
        .db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .insert_one(agent)
        .await
        .unwrap();
    b.verified_credential_identity = binding::resolve(&state, &owner, &b.credential, profile)
        .await
        .unwrap()
        .identity;
    persist_binding(&state, &b).await;
    let proxy = Router::new()
        .route(
            "/proxy/s/{slug}/{*path}",
            any(crate::handlers::proxy::proxy_request_by_slug),
        )
        .layer(axum::Extension(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        ))
        .with_state(state.clone());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let callback_codes = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(Semaphore::new(if gated { 0 } else { 16 }));
    let captures = requests.clone();
    let codes = callback_codes.clone();
    let g = gate.clone();
    let chunks = Arc::new(AtomicUsize::new(1));
    let chunk_delay_ms = Arc::new(AtomicUsize::new(0));
    let fail_turn = Arc::new(AtomicBool::new(false));
    let stream_chunks = chunks.clone();
    let stream_delay = chunk_delay_ms.clone();
    let stream_fail = fail_turn.clone();
    let upstream_finished = tokio_util::sync::CancellationToken::new();
    let finished = upstream_finished.clone();
    let upstream=Router::new().route("/v1/capabilities",get(||async{StatusCode::NOT_FOUND})).route("/v1/models",get(||async{Json(json!({"data":[{"id":engine::DEFAULT_MODEL}]}))})).route("/v1/responses",post(move|headers:HeaderMap,Json(body):Json<Value>|{let proxy=proxy.clone();let notion=notion.clone();let captures=captures.clone();let codes=codes.clone();let g=g.clone();let chunks=stream_chunks.clone();let delay=stream_delay.clone();let failed=stream_fail.clone();let finished=finished.clone();async move{
        let _finished = finished.drop_guard();
        captures.lock().await.push(body);g.acquire().await.unwrap().forget();
        let authorization=headers.get("authorization").unwrap().clone();
        let response=proxy.oneshot(Request::builder().uri(format!("/proxy/s/api-notion/faq?_nyxid_via={notion}")).header("authorization",authorization).body(Body::empty()).unwrap()).await.unwrap();
        let status=response.status();codes.lock().await.push(status.as_u16());let bytes=axum::body::to_bytes(response.into_body(),65536).await.unwrap();
        let text=if status.is_success(){serde_json::from_slice::<Value>(&bytes).unwrap()["faq"].as_str().unwrap().to_owned()}else{"Callback denied.".into()};
        let stream=async_stream::stream! {
            let count=chunks.load(Ordering::SeqCst).max(1);let chars:Vec<char>=text.chars().collect();let width=chars.len().div_ceil(count).max(1);let mut sequence=0;
            for part in chars.chunks(width) {if delay.load(Ordering::SeqCst)>0 {tokio::time::sleep(std::time::Duration::from_millis(delay.load(Ordering::SeqCst) as u64)).await;}let delta=json!({"type":"response.output_text.delta","sequence_number":sequence,"delta":part.iter().collect::<String>()});sequence+=1;yield Ok::<_,Infallible>(axum::body::Bytes::from(format!("data: {delta}\n\n")));}
            let completed=if failed.load(Ordering::SeqCst){json!({"type":"response.failed","sequence_number":sequence,"response":{"id":"resp_fixture","status":"failed","error":{"code":"fixture_failed","message":"controlled failure"}}})}else{json!({"type":"response.completed","sequence_number":sequence,"response":{"id":"resp_11111111111111111111111111111111_22222222222222222222222222222222","conversation":{"id":"conv_11111111111111111111111111111111"},"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}]}})};yield Ok(axum::body::Bytes::from(format!("data: {completed}\n\n")));
        };
        ([("content-type","text/event-stream")],Body::from_stream(stream)).into_response()
    }}));
    let (upstream_origin, upstream_task) = serve(upstream).await;
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = engine::SERVICE_SLUG.into();
    service.base_url = upstream_origin;
    service.auth_method = "none".into();
    service.requires_user_credential = false;
    service.forward_access_token = true;
    service.inject_delegation_token = false;
    service.service_category = "internal".into();
    service.credential_encrypted.clear();
    state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .insert_one(service)
        .await
        .unwrap();
    Fixture {
        state,
        b,
        faq,
        reads,
        requests,
        callback_codes,
        upstream_finished,
        gate,
        chunks,
        chunk_delay_ms,
        fail_turn,
        tasks: vec![notion_task, upstream_task],
    }
}
async fn active_session(f: &Fixture, call: &str) -> CallSession {
    let (s, fresh) = session::prepare(&f.state, &f.b, call, "+14155550199", &f.b.phone_number)
        .await
        .unwrap();
    assert!(fresh);
    f.state.db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id},doc!{"$set":{"state":"active","claim_id":&s.id,"lease_expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(30))}}).await.unwrap();
    session::load(&f.state, &s.id).await.unwrap()
}
async fn run_turn(f: &Fixture, s: &CallSession, event: &str, text: &str) -> String {
    let (mut output, worker) = coordinator::turn(f.state.clone(), &f.b, s, event, text)
        .await
        .unwrap();
    let mut text = String::new();
    let mut done = false;
    let mut output_failed = false;
    while let Some(out) = output.recv().await {
        output_failed |= out.error;
        text.push_str(&out.text);
        done |= out.last;
    }
    let settlement = worker.await.unwrap().unwrap();
    assert!(
        !output_failed && settlement.error.is_none(),
        "turn error: {:?}; upstream requests: {}; callback statuses: {:?}",
        settlement
            .error
            .as_ref()
            .map(|error| (error.code, error.stage)),
        f.requests.lock().await.len(),
        f.callback_codes.lock().await
    );
    assert!(done);
    assert!(!text.is_empty());
    assert_eq!(
        f.callback_codes.lock().await.last().copied(),
        Some(200),
        "the normal turn must complete its authenticated FAQ callback"
    );
    text
}

#[tokio::test]
async fn coordinator_faq_is_fresh_and_specialist_instructions_refresh_for_both_carriers() {
    for profile in [Profile::TelnyxRelay, Profile::TwilioRelay] {
        let f = fixture(profile, false).await;
        let s = active_session(&f, "fixture-call").await;
        assert_eq!(
            run_turn(&f, &s, "utterance-1", "When do you open?").await,
            "The office opens at nine."
        );
        *f.faq.write().await = "The office now opens at ten.".into();
        f.state.db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME).update_one(doc!{"_id":&f.b.agent_id},doc!{"$set":{"persona":"TRUSTED_ROLE_TWO. Read the FAQ on every question.","updated_at":bson::DateTime::now()}}).await.unwrap();
        let s = session::load(&f.state, &s.id).await.unwrap();
        assert_eq!(
            run_turn(&f, &s, "utterance-2", "When do you open now?").await,
            "The office now opens at ten."
        );
        assert_eq!(f.reads.load(Ordering::SeqCst), 2);
        assert_eq!(*f.callback_codes.lock().await, vec![200, 200]);
        let requests = f.requests.lock().await;
        assert!(requests[0].to_string().contains("TRUSTED_ROLE_ONE"));
        assert!(requests[1].to_string().contains("TRUSTED_ROLE_TWO"));
        let row = f
            .state
            .db
            .collection::<AssistantConversation>(
                crate::models::assistant_conversation::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&s.conversation_id})
            .await
            .unwrap()
            .unwrap();
        assert!(row.guest_turn);
        assert_ne!(row.id, f.b.user_id);
        drop(requests);
        f.state.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn disabling_after_nyxagent_acceptance_denies_actual_notion_callback() {
    let f = fixture(Profile::TelnyxRelay, true).await;
    let s = active_session(&f, "revoke-call").await;
    let (mut output, worker) =
        coordinator::turn(f.state.clone(), &f.b, &s, "utterance-1", "Read the FAQ")
            .await
            .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if !f.requests.lock().await.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    f.state
        .db
        .collection::<Binding>(BINDINGS)
        .update_one(
            doc! {"_id":&f.b.id},
            doc! {"$set":{"enabled":false},"$inc":{"revision":1}},
        )
        .await
        .unwrap();
    f.gate.add_permits(1);
    while output.recv().await.is_some() {}
    let _ = worker.await.unwrap();
    assert_eq!(f.reads.load(Ordering::SeqCst), 0);
    let codes = f.callback_codes.lock().await;
    assert_eq!(codes.len(), 1);
    assert!(codes[0] >= 400);
    drop(codes);
    f.state.db.drop().await.unwrap();
}
#[test]
fn full_registered_catalog_contract_and_legacy_truth() {
    let cache =
        Arc::new(crate::services::provider_token_exchange_service::TokenExchangeCache::new());
    let descriptors: Vec<_> = channel_adapters::registered_interfaces(&cache)
        .into_iter()
        .map(|a| match a {
            channel_adapters::RegisteredInterface::Messaging(a) => {
                channel_capability::describe(a.as_ref())
            }
            channel_adapters::RegisteredInterface::Nyxbot(a) => a.descriptor(),
        })
        .collect();
    let open = descriptors
        .iter()
        .find(|a| a.adapter_id == "openclaw")
        .unwrap();
    assert!(!open.messaging.outbound_text);
    assert!(!open.instructions.external_nyxbot);
    assert_eq!(
        open.messaging.delivery,
        channel_capability::Evidence::Unsupported
    );
    assert!(open.setup.documentation.is_none());
    let reply = descriptors
        .iter()
        .find(|a| a.adapter_id == "aurinko")
        .unwrap();
    assert!(reply.messaging.outbound_text);
    assert!(reply.messaging.reply_to);
    assert!(!reply.messaging.initiated_send);
    assert!(reply.messaging.max_text_bytes.is_none());
    let value = json!({"adapters":descriptors});
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../frontend/src/schemas/fixtures/channel-adapter-catalog.json");
    if std::env::var_os("NYXID_UPDATE_CONTRACT_FIXTURES").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(path)
            .expect("catalog fixture must be generated from the executable registry"),
    )
    .unwrap();
    assert_eq!(fixture, value);
}
#[test]
fn sms_toll_free_unicode_and_explicit_overflow() {
    assert_eq!(
        profiles::sms_parts(Profile::TwilioSms, &"文".repeat(134)).unwrap(),
        3
    );
    assert_eq!(
        profiles::sms_parts(Profile::TwilioSms, &"x".repeat(700)).unwrap(),
        11
    );
    assert!(profiles::sms_parts(Profile::TwilioSms, &"x".repeat(701)).is_err());
    assert!(profiles::sms_parts(Profile::TwilioSms, &"😀".repeat(351)).is_err());
    assert!(profiles::sms_parts(Profile::TelnyxRelay, "hello").is_err());
}
#[tokio::test]
async fn provider_sms_refusal_is_distinct_from_uncertain_acceptance() {
    let b = binding(Profile::TwilioSms, &Uuid::new_v4().to_string());
    let key = format!("{}:fixture", b.account_id);
    let (origin, task) = serve(Router::new().fallback(|| async {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"code":21617,"message":"too long"})),
        )
    }))
    .await;
    let result = profiles::send_sms_at(
        crate::services::proxy_service::no_redirect_http_client(),
        &b,
        &key,
        "+14155550199",
        "hello",
        "https://example.com/status",
        Some(&origin),
    )
    .await;
    assert!(matches!(result, Err(profiles::SmsFailure::Rejected(_))));
    task.abort();
    let result = profiles::send_sms_at(
        crate::services::proxy_service::no_redirect_http_client(),
        &b,
        &key,
        "+14155550199",
        "hello",
        "https://example.com/status",
        Some("http://127.0.0.1:1"),
    )
    .await;
    assert!(matches!(result, Err(profiles::SmsFailure::Uncertain)));
}

#[tokio::test]
async fn provider_redirect_never_forwards_credentials_or_effect_to_second_server() {
    use base64::Engine;
    for profile in [Profile::TelnyxSms, Profile::TwilioSms] {
        let target_calls = Arc::new(AtomicUsize::new(0));
        let target_count = target_calls.clone();
        let (target, target_task) = serve(Router::new().fallback(move || {
            let count = target_count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Json(json!({"sid":"SMredirected","num_segments":"1","data":{"id":"redirected","parts":1}}))
            }
        })).await;
        let b = binding(profile, &Uuid::new_v4().to_string());
        let key = if profile == Profile::TwilioSms {
            format!("{}:fixture-auth-token", b.account_id)
        } else {
            "fixture-bearer-token".into()
        };
        let expected_auth = if profile == Profile::TwilioSms {
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(&key)
            )
        } else {
            format!("Bearer {key}")
        };
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let authenticated = Arc::new(AtomicBool::new(false));
        let count = provider_calls.clone();
        let auth = authenticated.clone();
        let (origin, provider_task) = serve(Router::new().fallback(move |headers: HeaderMap| {
            let count = count.clone();
            let auth = auth.clone();
            let expected = expected_auth.clone();
            let location = format!("{target}/redirect-effect");
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                auth.store(
                    headers.get("authorization").and_then(|v| v.to_str().ok())
                        == Some(expected.as_str()),
                    Ordering::SeqCst,
                );
                (StatusCode::FOUND, [("location", location)], Json(json!({})))
            }
        }))
        .await;
        let result = profiles::send_sms_at(
            crate::services::proxy_service::no_redirect_http_client(),
            &b,
            &key,
            "+14155550199",
            "hello",
            "https://example.com/status",
            Some(&origin),
        )
        .await;
        assert!(matches!(result, Err(profiles::SmsFailure::Uncertain)));
        assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
        assert!(
            authenticated.load(Ordering::SeqCst),
            "fixture must exercise credential-bearing dispatch"
        );
        assert_eq!(
            target_calls.load(Ordering::SeqCst),
            0,
            "even an unauthenticated redirected request would be an unintended effect"
        );
        provider_task.abort();
        target_task.abort();
    }
}
#[test]
fn telnyx_padded_call_id_is_a_single_encoded_path_segment() {
    assert_eq!(
        protocol::call_segment(Profile::TelnyxRelay, "v2:abc_123==").unwrap(),
        "v2%3Aabc_123%3D%3D"
    );
    for bad in ["../other", "v2:a/b==", "abc===", "abc?token=x", "abc#x"] {
        assert!(protocol::call_segment(Profile::TelnyxRelay, bad).is_err());
    }
}
#[tokio::test]
async fn durable_terminal_quantity_is_frozen_and_noncall_sessions_finalize() {
    let f = fixture(Profile::TelnyxRelay, false).await;
    let s = active_session(&f, "terminal-call").await;
    let now = Utc::now();
    let ended = session::terminal(&f.state, &s, now, Some(12), "signed_provider_duration")
        .await
        .unwrap();
    let retry = session::terminal(
        &f.state,
        &s,
        now + Duration::hours(2),
        Some(7212),
        "late_duplicate",
    )
    .await
    .unwrap();
    assert_eq!(retry.observed_seconds, 12);
    assert_eq!(retry.ended_at, ended.ended_at);
    let mut b = f.b.clone();
    b.profile = Profile::TwilioSms;
    b.account_id.push_str("sms");
    let (sms, _) = session::prepare(&f.state, &b, "sms-1", "+14155550199", &b.phone_number)
        .await
        .unwrap();
    session::finish_noncall(&f.state, &sms.id, "sms_finished")
        .await
        .unwrap();
    let sms = session::load(&f.state, &sms.id).await.unwrap();
    assert_eq!(sms.state, SessionState::Closed);
    assert!(sms.ended_at.is_some());
    assert!(sms.billing_finalized);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn global_resource_claim_rejects_cross_owner_account_alias_takeover() {
    let db = connect_transaction_test_database("adapter_resource_claim").await;
    session::ensure_indexes(&db).await.unwrap();
    let state = test_app_state(db);
    let first = binding(Profile::TelnyxSms, &Uuid::new_v4().to_string());
    persist_binding(&state, &first).await;
    let mut attacker = first.clone();
    attacker.id = Uuid::new_v4().to_string();
    attacker.user_id = Uuid::new_v4().to_string();
    attacker.account_id = "different-supplied-account-alias".into();
    let mut tx = state.db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    assert!(
        binding::claim_resources(&state.db, &mut tx, &attacker)
            .await
            .is_err()
    );
    tx.abort_transaction().await.unwrap();
    state.db.drop().await.unwrap();
}

async fn seed_private_context(f: &Fixture) {
    let row = engine::begin_turn(
        &f.state.db,
        &f.b.user_id,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            agent_id: None,
            conversation_id: None,
            text: "OWNER_PRIVATE_HISTORY_DO_NOT_SHARE".into(),
            attachment_ids: vec![],
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    engine::finish_turn(
        &f.state.db,
        &row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "OWNER_PRIVATE_REPLY_DO_NOT_SHARE".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let now = bson::DateTime::now();
    f.state.db.collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME).update_one(doc!{"user_id":&f.b.user_id,"kind":"nyxbot"},doc!{"$set":{"memory":[{"id":Uuid::new_v4().to_string(),"text":"OWNER_PRIVATE_MEMORY_DO_NOT_SHARE","created_at":now,"updated_at":now}]}}).await.unwrap();
}
async fn attach_carrier(f: &mut Fixture, call: &str) -> Arc<Mutex<Vec<(String, Value)>>> {
    attach_carrier_state(f, call, Arc::new(RwLock::new("in-progress".to_owned()))).await
}
async fn attach_carrier_state(
    f: &mut Fixture,
    call: &str,
    status: Arc<RwLock<String>>,
) -> Arc<Mutex<Vec<(String, Value)>>> {
    attach_carrier_behavior(f, call, status, 0).await
}
async fn attach_carrier_behavior(
    f: &mut Fixture,
    call: &str,
    status: Arc<RwLock<String>>,
    cap_outcome: u8,
) -> Arc<Mutex<Vec<(String, Value)>>> {
    let b = f.b.clone();
    let call = call.to_owned();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let captures = commands.clone();
    let provider=Router::new().fallback(any(move|request:Request<Body>|{let b=b.clone();let call=call.clone();let captures=captures.clone();let status=status.clone();async move {
        let path=request.uri().path().to_owned();let method=request.method().clone();let body=axum::body::to_bytes(request.into_body(),65536).await.unwrap();
        if method==axum::http::Method::POST {
            let value=serde_json::from_slice::<Value>(&body).unwrap_or_else(|_|json!(url::form_urlencoded::parse(&body).into_owned().collect::<std::collections::BTreeMap<_,_>>()));if path.ends_with("/hangup") || value["Status"]=="completed"{*status.write().await="completed".into();}let cap=value["TimeLimit"].is_string();captures.lock().await.push((path.clone(),value));
            if cap && cap_outcome==1 {return StatusCode::BAD_REQUEST.into_response();}
            if cap && cap_outcome==2 {return (StatusCode::OK,"not-json: ambiguous cap response").into_response();}
            return Json(json!({"data":{"result":"ok"},"sid":call,"account_sid":b.account_id})).into_response();
        }
        if path.contains("/IncomingPhoneNumbers/"){return Json(json!({"sid":b.resource_id,"account_sid":b.account_id,"phone_number":b.phone_number,"voice_application_sid":b.application_id,"voice_fallback_url":""})).into_response();}
        if path.contains("/Applications/"){return Json(json!({"sid":b.application_id,"account_sid":b.account_id,"voice_fallback_url":""})).into_response();}
        if path.ends_with("/active_calls"){return Json(json!({"data":[{"call_control_id":call}],"meta":{"next":null}})).into_response();}
        if path.starts_with("/v2/calls/"){return Json(json!({"data":{"call_control_id":call,"is_alive":false,"call_duration":12,"start_time":(Utc::now()-Duration::seconds(12)).to_rfc3339(),"end_time":Utc::now().to_rfc3339()}})).into_response();}
        if path.contains("/Calls/"){return Json(json!({"sid":call,"account_sid":b.account_id,"phone_number_sid":b.resource_id,"from":"+14155550199","to":b.phone_number,"direction":"inbound","status":*status.read().await,"duration":"12","end_time":Utc::now().to_rfc2822()})).into_response();}
        StatusCode::NOT_FOUND.into_response()
    }}));
    let (origin, task) = serve(provider).await;
    profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(f.b.account_id.clone(), origin);
    f.tasks.push(task);
    commands
}
async fn expose_ingress(f: &mut Fixture) -> String {
    expose_ingress_scheme(f, "http").await
}
async fn expose_ingress_scheme(f: &mut Fixture, scheme: &str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let mut config = f.state.config.clone();
    config.base_url = format!("{scheme}://{}", listener.local_addr().unwrap());
    f.state.config = config;
    let router = Router::new()
        .route(
            "/api/v1/webhooks/call-adapters/{id}",
            post(crate::handlers::call_adapters::webhook),
        )
        .route(
            "/api/v1/webhooks/call-adapters/{id}/sms-receipts/{send}",
            post(crate::handlers::call_adapters::sms_receipt),
        )
        .route(
            "/api/v1/webhooks/call-adapters/{id}/socket/{sid}",
            get(crate::handlers::call_adapters::socket),
        )
        .route(
            "/api/v1/webhooks/call-adapters/{id}/v1/chat/completions",
            post(crate::handlers::call_adapters::completions),
        )
        .with_state(f.state.clone());
    f.tasks.push(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    origin
}
fn twilio_signature(
    url: &str,
    fields: &std::collections::BTreeMap<String, String>,
    secret: &str,
) -> String {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(url.as_bytes());
    for (k, v) in fields {
        mac.update(k.as_bytes());
        mac.update(v.as_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

#[test]
fn twilio_upgrade_signature_accepts_only_exact_endpoint_and_scheme_mapping() {
    for (websocket, http) in [
        (
            "wss://relay.example.test:8443/relay/%2F/socket?ticket=abc%2B123&order=1&order=2",
            "https://relay.example.test:8443/relay/%2F/socket?ticket=abc%2B123&order=1&order=2",
        ),
        (
            "ws://relay.example.test:8080/relay/%2F/socket?ticket=abc%2B123&order=1&order=2",
            "http://relay.example.test:8080/relay/%2F/socket?ticket=abc%2B123&order=1&order=2",
        ),
    ] {
        let headers_for = |signed_url: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-twilio-signature",
                twilio_signature(signed_url, &Default::default(), "fixture-secret")
                    .parse()
                    .unwrap(),
            );
            headers.insert(
                "x-forwarded-host",
                "untrusted.example.test".parse().unwrap(),
            );
            headers
        };
        for signed_url in [websocket, http] {
            protocol::verify_twilio_upgrade(&headers_for(signed_url), websocket, "fixture-secret")
                .unwrap();
        }
        for altered in [
            http.replace("relay.example.test", "attacker.example.test"),
            http.replace("/relay/", "/other/"),
            http.replace(":8443", ":9443").replace(":8080", ":9080"),
            http.replace("%2F", "%2f"),
            http.replace("abc%2B123", "abc+123"),
            http.replace("order=1&order=2", "order=2&order=1"),
            format!("{http}&extra=1"),
        ] {
            assert!(
                protocol::verify_twilio_upgrade(
                    &headers_for(&altered),
                    websocket,
                    "fixture-secret"
                )
                .is_err()
            );
        }
        assert!(
            protocol::verify_twilio_upgrade(&headers_for(http), http, "fixture-secret").is_err()
        );
    }
}
async fn telnyx_post(
    f: &Fixture,
    signer: &ed25519_dalek::SigningKey,
    kind: &str,
    call: &str,
) -> reqwest::Response {
    use base64::Engine;
    use ed25519_dalek::Signer;
    let body=json!({"data":{"id":Uuid::new_v4().to_string(),"event_type":kind,"occurred_at":Utc::now().to_rfc3339(),"payload":{"call_control_id":call,"connection_id":f.b.application_id,"direction":"incoming","from":"+14155550199","to":f.b.phone_number}}}).to_string();
    let timestamp = Utc::now().timestamp().to_string();
    let signed = format!("{timestamp}|{body}");
    let signature =
        base64::engine::general_purpose::STANDARD.encode(signer.sign(signed.as_bytes()).to_bytes());
    reqwest::Client::new()
        .post(coordinator::webhook_url(&f.state, &f.b))
        .header("telnyx-timestamp", timestamp)
        .header("telnyx-signature-ed25519", signature)
        .body(body)
        .send()
        .await
        .unwrap()
}
#[tokio::test]
async fn signed_carrier_ingress_websocket_finals_and_fifo_reach_real_faq_tools() {
    use base64::Engine;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    for profile in [Profile::TelnyxRelay, Profile::TwilioRelay] {
        let mut f = fixture(profile, false).await;
        seed_private_context(&f).await;
        let call = if profile == Profile::TelnyxRelay {
            "v2:fixture=="
        } else {
            "CA11111111111111111111111111111111"
        };
        let carrier_status = Arc::new(RwLock::new("ringing".to_owned()));
        let commands = attach_carrier_state(&mut f, call, carrier_status.clone()).await;
        expose_ingress(&mut f).await;
        let ws_url = if profile == Profile::TelnyxRelay {
            let signer = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
            f.b.webhook_public_key = Some(
                base64::engine::general_purpose::STANDARD.encode(signer.verifying_key().to_bytes()),
            );
            f.state
                .db
                .collection::<Binding>(BINDINGS)
                .replace_one(doc! {"_id":&f.b.id}, &f.b)
                .await
                .unwrap();
            assert!(
                telnyx_post(&f, &signer, "call.initiated", call)
                    .await
                    .status()
                    .is_success()
            );
            assert_eq!(commands.lock().await.len(), 1);
            assert_eq!(
                commands.lock().await[0].0,
                "/v2/calls/v2%3Afixture%3D%3D/actions/answer"
            );
            // Answer command acceptance cannot start relay; only signed answered can.
            assert!(
                telnyx_post(&f, &signer, "call.answered", call)
                    .await
                    .status()
                    .is_success()
            );
            let rows = commands.lock().await;
            assert_eq!(rows.len(), 2);
            assert_eq!(
                rows[1].0,
                "/v2/calls/v2%3Afixture%3D%3D/actions/conversation_relay_start"
            );
            rows[1].1["url"].as_str().unwrap().to_owned()
        } else {
            let fields = std::collections::BTreeMap::from([
                ("AccountSid".into(), f.b.account_id.clone()),
                ("CallSid".into(), call.into()),
                ("From".into(), "+14155550199".into()),
                ("To".into(), f.b.phone_number.clone()),
                ("Direction".into(), "inbound".into()),
                ("CallStatus".into(), "ringing".into()),
            ]);
            let url = coordinator::webhook_url(&f.state, &f.b);
            let signature = twilio_signature(&url, &fields, "fixture-auth-token");
            let response = reqwest::Client::new()
                .post(&url)
                .header("x-twilio-signature", signature)
                .form(&fields)
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
            let bootstrap = response.text().await.unwrap();
            assert!(bootstrap.contains("<Pause length=\"1\"/>"));
            assert!(bootstrap.contains("#rc=0&amp;rt=5000&amp;tt=5000"));
            assert!(
                commands.lock().await.is_empty(),
                "no active-call command while ringing"
            );
            *carrier_status.write().await = "in-progress".into();
            let mut fields = fields;
            fields.insert("CallStatus".into(), "in-progress".into());
            let response = reqwest::Client::new()
                .post(&url)
                .header(
                    "x-twilio-signature",
                    twilio_signature(&url, &fields, "fixture-auth-token"),
                )
                .form(&fields)
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
            let xml = response.text().await.unwrap();
            let retried = reqwest::Client::new()
                .post(&url)
                .header(
                    "x-twilio-signature",
                    twilio_signature(&url, &fields, "fixture-auth-token"),
                )
                .form(&fields)
                .send()
                .await
                .unwrap();
            assert_eq!(retried.status(), 200);
            assert_eq!(
                retried.text().await.unwrap(),
                xml,
                "retry returns identical admitted TwiML, without hanging up or issuing a new cap"
            );
            assert_eq!(commands.lock().await.len(), 1);
            assert!(xml.contains("ttsProvider=\"Google\""));
            assert!(
                commands
                    .lock()
                    .await
                    .iter()
                    .any(|(_, body)| body["TimeLimit"] == "1800")
            );
            xml.split("url=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap()
                .replace("&amp;", "&")
        };
        let mut request = ws_url.clone().into_client_request().unwrap();
        if profile == Profile::TwilioRelay {
            request.headers_mut().insert(
                "x-twilio-signature",
                twilio_signature(&ws_url, &Default::default(), "fixture-auth-token")
                    .parse()
                    .unwrap(),
            );
        }
        let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        ws.send(Message::Text(json!({"type":"setup","sessionId":"provider-session","accountSid":f.b.account_id,"callSid":call,"from":"+14155550199","to":f.b.phone_number,"direction":"inbound"}).to_string().into())).await.unwrap();
        ws.send(Message::Text(
            json!({"type":"prompt","voicePrompt":"Do not execute partial text","last":false})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(f.reads.load(Ordering::SeqCst), 0);
        for text in ["When do you open?", "Please confirm opening time."] {
            ws.send(Message::Text(
                json!({"type":"prompt","voicePrompt":text,"last":true})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        }
        let mut finals = 0;
        let mut tokens = String::new();
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while finals < 2 {
                let item = ws.next().await.unwrap().unwrap();
                let value: Value = serde_json::from_slice(&item.into_data()).unwrap();
                if value["type"] == "text" {
                    tokens.push_str(value["token"].as_str().unwrap_or_default());
                    if value["last"] == true {
                        finals += 1;
                    }
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(f.reads.load(Ordering::SeqCst), 2);
        assert!(tokens.contains("The office opens at nine."));
        for body in f.requests.lock().await.iter() {
            let body = body.to_string();
            assert!(!body.contains("OWNER_PRIVATE_HISTORY_DO_NOT_SHARE"));
            assert!(!body.contains("OWNER_PRIVATE_REPLY_DO_NOT_SHARE"));
            assert!(!body.contains("OWNER_PRIVATE_MEMORY_DO_NOT_SHARE"));
        }
        ws.close(None).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let closed = session::by_call(&f.state, &f.b, call).await.unwrap();
                if closed.state == SessionState::Closed && closed.billing_finalized {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("socket cleanup must terminate carrier and finalize admitted usage");
        assert!(
            commands
                .lock()
                .await
                .iter()
                .any(|(path, body)| path.ends_with("/hangup") || body["Status"] == "completed")
        );
        f.state.db.drop().await.unwrap();
    }
}
#[test]
fn custom_llm_evidence_distinguishes_repeated_text_and_refuses_history_reset() {
    let first = json!({"messages":[{"role":"user","content":"yes"}]});
    let h = super::custom_llm::history(&first, &[]).unwrap();
    let (_, one) = super::custom_llm::final_utterance(&first).unwrap();
    let next = json!({"messages":[{"role":"user","content":"yes"},{"role":"assistant","content":"Proceed?"},{"role":"user","content":"yes"}]});
    let (_, two) = super::custom_llm::final_utterance(&next).unwrap();
    assert_ne!(one, two);
    assert!(super::custom_llm::history(&next, &h).is_ok());
    assert!(super::custom_llm::history(&first, &h).is_err());
    let trimmed = json!({"messages":[{"role":"assistant","content":"Proceed?"},{"role":"user","content":"yes"}]});
    assert!(super::custom_llm::history(&trimmed, &h).is_err());
}
#[test]
fn late_prior_reply_interrupt_cannot_relabel_new_reply_and_ambiguity_is_unverified() {
    let mut playback = coordinator::Playback::default();
    playback
        .append("turn-old", "Previous answer still playing.")
        .unwrap();
    playback.append("turn-new", "New answer.").unwrap();
    assert_eq!(
        playback.correlate("Previous answer"),
        Some(("turn-old", "Previous answer still playing."))
    );
    assert!(playback.correlate("").is_none());
    playback.append("turn-third", "New answer.").unwrap();
    assert!(playback.correlate("New").is_none());
}

#[tokio::test]
async fn custom_llm_http_sse_correlates_isolated_calls_refreshes_faq_and_refuses_replay() {
    let mut f = fixture(Profile::ElevenlabsCustomLlm, false).await;
    seed_private_context(&f).await;
    expose_ingress(&mut f).await;
    let setup = super::custom_llm::setup(&f.state, &f.b).unwrap();
    let one = super::custom_llm::create_session(&f.state, &f.b)
        .await
        .unwrap();
    let two = super::custom_llm::create_session(&f.state, &f.b)
        .await
        .unwrap();
    let url = setup["url"].as_str().unwrap();
    let key = setup["api_key"].as_str().unwrap();
    let first = json!({"stream":true,"messages":[{"role":"system","content":"UNTRUSTED_OWNER_INSTRUCTIONS"},{"role":"user","content":"When do you open?"}],"elevenlabs_extra_body":one["extra_body"]});
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .bearer_auth(key)
        .json(&first)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let stream = response.text().await.unwrap();
    assert!(stream.contains("The office opens at nine."));
    assert!(stream.contains("[DONE]"));
    assert!(stream.contains("\"finish_reason\":\"stop\""));
    let response = client
        .post(url)
        .bearer_auth(key)
        .json(&first)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(f.reads.load(Ordering::SeqCst), 1);
    let mut other = first.clone();
    other["elevenlabs_extra_body"] = two["extra_body"].clone();
    let response = client
        .post(url)
        .bearer_auth(key)
        .json(&other)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.text().await.unwrap().contains("[DONE]"));
    assert_eq!(f.reads.load(Ordering::SeqCst), 2);
    *f.faq.write().await = "The office now opens at ten.".into();
    let mut next = first.clone();
    next["messages"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":"The office opens at nine."}),
        json!({"role":"user","content":"When do you open?"}),
    ]);
    let response = client
        .post(url)
        .bearer_auth(key)
        .json(&next)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(
        response
            .text()
            .await
            .unwrap()
            .contains("The office now opens at ten.")
    );
    assert_eq!(f.reads.load(Ordering::SeqCst), 3);
    let mut trimmed = next.clone();
    trimmed["messages"] = json!([{"role":"user","content":"When do you open?"}]);
    assert_eq!(
        client
            .post(url)
            .bearer_auth(key)
            .json(&trimmed)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    for body in f.requests.lock().await.iter() {
        let body = body.to_string();
        for marker in [
            "UNTRUSTED_OWNER_INSTRUCTIONS",
            "OWNER_PRIVATE_HISTORY_DO_NOT_SHARE",
            "OWNER_PRIVATE_REPLY_DO_NOT_SHARE",
            "OWNER_PRIVATE_MEMORY_DO_NOT_SHARE",
        ] {
            assert!(!body.contains(marker), "leaked {marker}");
        }
    }
    for session in [&one, &two] {
        session::finish_noncall(
            &f.state,
            session["session_id"].as_str().unwrap(),
            "test_end",
        )
        .await
        .unwrap();
    }
    super::management::disable(&f.state, &f.b.user_id, &f.b.id)
        .await
        .unwrap();
    super::management::remove(&f.state, &f.b.user_id, &f.b.id)
        .await
        .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn admitted_funded_call_settles_frozen_usage_once_after_price_and_credential_revocation() {
    use crate::models::{
        billing_rate_cache::BillingRateCache,
        billing_wallet::{BillingWallet, CollectionState, PlanKind},
        credits::Credits,
        service_billing::{BillingMetric, PlatformUsage},
        usage_meter::{BillingLayer, CredentialClass, UsageMeterRow},
    };
    use crate::services::billing::{
        BillingIngress, BillingRouteContext, NodeIntent, meter,
        reservation::{BillingReservation, LayerReservation},
    };
    let f = fixture(Profile::TwilioRelay, false).await;
    let s = active_session(&f, "funded-call").await;
    let now = Utc::now();
    let owner = &f.b.user_id;
    f.state
        .db
        .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"transaction_id":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let mut ctx = BillingRouteContext::new(
        BillingIngress::ChannelOutbound,
        s.billing_request_id.clone(),
        owner.clone(),
        owner.clone(),
        None,
        None,
        Some(Uuid::new_v4().to_string()),
        Some("twilio".into()),
        NodeIntent::Direct,
        "basic".into(),
        CredentialClass::UserOwned,
        BillingMetric::VoiceSeconds,
        None::<&crate::models::service_billing::ServiceBilling>,
        false,
    )
    .with_platform_metering(true);
    ctx.requested_voice_seconds = 1800;
    ctx.voice_initial_window = true;
    f.state
        .db
        .collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
        .insert_one(BillingRateCache {
            id: BillingRateCache::cache_id(&ctx.platform_lago_metric_code, None),
            lago_metric_code: ctx.platform_lago_metric_code.clone(),
            model: None,
            credits_per_unit_micros: 2_000_000,
            credits_per_unit_pico: None,
            synced_at: now,
            retired_at: None,
        })
        .await
        .unwrap();
    let wallet = BillingWallet {
        id: Uuid::new_v4().to_string(),
        owner_id: owner.clone(),
        lago_customer_id: owner.clone(),
        lago_wallet_id: Some(Uuid::new_v4().to_string()),
        lago_subscription_id: Some(Uuid::new_v4().to_string()),
        plan_kind: PlanKind::Prepaid,
        balance_credits: Credits::from_whole(5000),
        reserved_credits: Credits::ZERO,
        pending_lago_debits: Credits::ZERO,
        pending_topup_expiry_credits: Credits::ZERO,
        has_payment_instrument: false,
        overdraft_cap_credits: Credits::ZERO,
        suspended: false,
        collection_state: CollectionState::Good,
        topup_expiry_checked_at: None,
        active_topup_expiry: None,
        balance_synced_at: now,
        created_at: now,
        updated_at: now,
    };
    f.state
        .db
        .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
        .insert_one(&wallet)
        .await
        .unwrap();
    let hold = Credits::from_whole(3600);
    crate::services::billing::reservation::try_reserve_prepaid(&f.state.db, owner, hold)
        .await
        .unwrap()
        .unwrap();
    let reservation = BillingReservation {
        owner_id: owner.clone(),
        wallet_id: wallet.id.clone(),
        total_reserved_credits: hold,
        layers: vec![LayerReservation {
            metric: BillingMetric::VoiceSeconds,
            lago_metric_code: ctx.platform_lago_metric_code.clone(),
            layer: BillingLayer::Platform,
            estimated_quantity: 1800,
            credits_per_unit_micros: 2_000_000,
            credits_per_unit_pico: None,
            reserved_credits: hold,
            allowance_reservations: vec![],
            grant_reservations: vec![],
        }],
    };
    let meter = meter::open(&f.state.db, &ctx, Some(&reservation))
        .await
        .unwrap();
    meter::mark_forwarded(&f.state.db, &meter).await.unwrap();
    // No current credential or current price is needed to settle admitted funding.
    f.state
        .db
        .collection::<bson::Document>(crate::models::billing_rate_cache::COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$set":{"retired_at":bson::DateTime::now()}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$set":{"status":"revoked"}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Binding>(BINDINGS)
        .update_one(
            doc! {"_id":&f.b.id},
            doc! {"$set":{"enabled":false},"$inc":{"revision":1}},
        )
        .await
        .unwrap();
    let frozen = session::terminal(&f.state, &s, now, Some(12), "provider_duration")
        .await
        .unwrap();
    coordinator::finalize_call(&f.state, &frozen).await.unwrap();
    let replay = session::terminal(
        &f.state,
        &s,
        now + Duration::minutes(20),
        Some(1200),
        "late_duplicate",
    )
    .await
    .unwrap();
    coordinator::finalize_call(&f.state, &replay).await.unwrap();
    super::billing::settle_admitted(&f.state, &s.billing_request_id, owner, 12, 1)
        .await
        .unwrap();
    let wallet = f
        .state
        .db
        .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
        .find_one(doc! {"_id":wallet.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    assert_eq!(wallet.pending_lago_debits, Credits::from_whole(24));
    let row = f
        .state
        .db
        .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
        .find_one(doc! {"billing_request_id":&s.billing_request_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.quantity, Some(12));
    assert!(
        session::load(&f.state, &s.id)
            .await
            .unwrap()
            .billing_finalized
    );
    let _ = PlatformUsage::default();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn funded_twilio_sms_signed_ingress_real_turn_unknown_parts_receipt_and_lifecycle() {
    use futures::TryStreamExt;
    let mut f = fixture(Profile::TwilioSms, false).await;
    fund_sms(&mut f).await;
    let b = f.b.clone();
    let callback = Arc::new(Mutex::new(String::new()));
    let captured = callback.clone();
    let sends = Arc::new(AtomicUsize::new(0));
    let count = sends.clone();
    let message = "SM11111111111111111111111111111111";
    let provider=Router::new().fallback(any(move|req:Request<Body>|{let b=b.clone();let captured=captured.clone();let count=count.clone();async move{
        let method=req.method().clone();let bytes=axum::body::to_bytes(req.into_body(),65536).await.unwrap();
        if method==axum::http::Method::POST {let fields=url::form_urlencoded::parse(&bytes).into_owned().collect::<std::collections::BTreeMap<_,_>>();*captured.lock().await=fields["StatusCallback"].clone();assert_eq!(fields["Body"],"The office opens at nine.");count.fetch_add(1,Ordering::SeqCst);Json(json!({"sid":message,"account_sid":b.account_id,"num_segments":"0","status":"queued"}))}
        else {Json(json!({"sid":message,"account_sid":b.account_id,"from":b.phone_number,"to":"+14155550199","num_segments":"1","status":"delivered"}))}
    }}));
    let (origin, task) = serve(provider).await;
    f.tasks.push(task);
    profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(f.b.account_id.clone(), origin);
    expose_ingress(&mut f).await;
    let fields = std::collections::BTreeMap::from([
        ("AccountSid".into(), f.b.account_id.clone()),
        (
            "MessageSid".into(),
            "SM22222222222222222222222222222222".into(),
        ),
        ("From".into(), "+14155550199".into()),
        ("To".into(), f.b.phone_number.clone()),
        ("SmsStatus".into(), "received".into()),
        ("NumSegments".into(), "1".into()),
        ("NumMedia".into(), "0".into()),
        ("Body".into(), "When do you open?".into()),
    ]);
    let url = coordinator::webhook_url(&f.state, &f.b);
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header(
            "x-twilio-signature",
            twilio_signature(&url, &fields, "fixture-auth-token"),
        )
        .form(&fields)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if let Some(row) = f
                .state
                .db
                .collection::<EventReceipt>(EVENTS)
                .find_one(doc! {"event_kind":"sms_send","billing_finalized":true})
                .await
                .unwrap()
            {
                assert_eq!(row.observed_parts, Some(1));
                let session = session::load(&f.state, row.session_id.as_deref().unwrap())
                    .await
                    .unwrap();
                if session.ended_at.is_some() && session.billing_finalized {
                    break;
                }
            }
            super::sms_delivery::sweep(&f.state).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let row = f
        .state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find_one(doc! {"event_kind":"sms_send"})
        .await
        .unwrap()
        .unwrap();
    let s = session::load(&f.state, row.session_id.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(s.state, SessionState::Closed);
    assert!(s.ended_at.is_some() && s.billing_finalized);
    let wallet = f
        .state
        .db
        .collection::<crate::models::billing_wallet::BillingWallet>(
            crate::models::billing_wallet::COLLECTION_NAME,
        )
        .find_one(doc! {"owner_id":&f.b.user_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        wallet.reserved_credits,
        crate::models::credits::Credits::ZERO
    );
    assert_eq!(
        wallet.pending_lago_debits,
        crate::models::credits::Credits::from_whole(4)
    );
    let receipt = std::collections::BTreeMap::from([
        ("AccountSid".into(), f.b.account_id.clone()),
        ("MessageSid".into(), message.into()),
        ("MessageStatus".into(), "delivered".into()),
    ]);
    let url = callback.lock().await.clone();
    assert!(url.ends_with(&format!("/sms-receipts/{}", row.id)));
    let response = client
        .post(&url)
        .header(
            "x-twilio-signature",
            twilio_signature(&url, &receipt, "fixture-auth-token"),
        )
        .form(&receipt)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let after = f
        .state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find_one(doc! {"_id":&row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.delivery, "delivered");
    assert!(after.billing_finalized);
    let url = coordinator::webhook_url(&f.state, &f.b);
    assert_eq!(
        client
            .post(&url)
            .header(
                "x-twilio-signature",
                twilio_signature(&url, &fields, "fixture-auth-token")
            )
            .form(&fields)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    assert_eq!(f.reads.load(Ordering::SeqCst), 1);
    let events: Vec<EventReceipt> = f
        .state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find(doc! {"event_kind":"sms_send"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    super::management::disable(&f.state, &f.b.user_id, &f.b.id)
        .await
        .unwrap();
    super::management::remove(&f.state, &f.b.user_id, &f.b.id)
        .await
        .unwrap();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn receipt_recovers_response_lost_send_without_replay_and_rejects_second_join() {
    let f = fixture(Profile::TwilioSms, false).await;
    let s = active_session(&f, "sms-lost").await;
    let id = Uuid::new_v4().to_string();
    let message = "SM33333333333333333333333333333333";
    let sent = EventReceipt {
        id: id.clone(),
        identity_key: session::identity(&[&s.id, "sms_send"]),
        user_id: f.b.user_id.clone(),
        binding_id: f.b.id.clone(),
        binding_revision: f.b.revision,
        session_id: Some(s.id.clone()),
        event_kind: "sms_send".into(),
        status: "uncertain".into(),
        provider_message_id: None,
        delivery: "unknown".into(),
        billing_request_id: None,
        observed_parts: None,
        reserved_parts: Some(3),
        billing_finalized: false,
        from: Some(f.b.phone_number.clone()),
        to: Some(s.from.clone()),
        created_at: Utc::now(),
    };
    f.state
        .db
        .collection::<EventReceipt>(EVENTS)
        .insert_one(&sent)
        .await
        .unwrap();
    // A signed callback carries the durable send ID even when POST response was lost.
    super::sms_delivery::record_correlated(
        &f.state,
        &f.b,
        super::sms_delivery::Receipt {
            event: "receipt-first",
            message,
            from: &f.b.phone_number,
            to: &s.from,
            status: "delivered",
            parts: Some(2),
            send: Some(&id),
        },
    )
    .await
    .unwrap();
    let row = f
        .state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.provider_message_id.as_deref(), Some(message));
    assert!(row.billing_finalized);
    assert_eq!(row.observed_parts, Some(2));
    super::sms_delivery::record_correlated(
        &f.state,
        &f.b,
        super::sms_delivery::Receipt {
            event: "receipt-first",
            message,
            from: &f.b.phone_number,
            to: &s.from,
            status: "delivered",
            parts: Some(2),
            send: Some(&id),
        },
    )
    .await
    .unwrap();
    super::sms_delivery::record_correlated(
        &f.state,
        &f.b,
        super::sms_delivery::Receipt {
            event: "late-queued",
            message,
            from: "",
            to: "",
            status: "queued",
            parts: None,
            send: Some(&id),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<EventReceipt>(EVENTS)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap()
            .delivery,
        "delivered"
    );
    let mut other = sent;
    other.id = Uuid::new_v4().to_string();
    other.identity_key = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<EventReceipt>(EVENTS)
        .insert_one(&other)
        .await
        .unwrap();
    assert!(
        super::sms_delivery::record_correlated(
            &f.state,
            &f.b,
            super::sms_delivery::Receipt {
                event: "replay-other",
                message,
                from: &f.b.phone_number,
                to: &s.from,
                status: "delivered",
                parts: Some(2),
                send: Some(&other.id),
            },
        )
        .await
        .is_err()
    );
    assert_eq!(f.reads.load(Ordering::SeqCst), 0);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn prepared_session_cannot_outlive_removed_binding_and_reused_claim_is_write_fenced() {
    let f = fixture(Profile::TwilioSms, false).await;
    super::management::disable(&f.state, &f.b.user_id, &f.b.id)
        .await
        .unwrap();
    super::management::remove(&f.state, &f.b.user_id, &f.b.id)
        .await
        .unwrap();
    assert!(
        session::prepare(
            &f.state,
            &f.b,
            "late-after-delete",
            "+14155550199",
            &f.b.phone_number
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<CallSession>(SESSIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let mut replacement = f.b.clone();
    replacement.id = Uuid::new_v4().to_string();
    persist_binding(&f.state, &replacement).await;
    let claim = f
        .state
        .db
        .collection::<ResourceClaim>(CLAIMS)
        .find_one(doc! {"resource_id":&replacement.resource_id})
        .await
        .unwrap()
        .unwrap();
    let mut release = f.state.db.client().start_session().await.unwrap();
    release.start_transaction().await.unwrap();
    f.state
        .db
        .collection::<ResourceClaim>(CLAIMS)
        .find_one(doc! {"_id":&claim.id})
        .session(&mut release)
        .await
        .unwrap();
    let mut reuse = f.state.db.client().start_session().await.unwrap();
    reuse.start_transaction().await.unwrap();
    binding::claim_resources(&f.state.db, &mut reuse, &replacement)
        .await
        .unwrap();
    reuse.commit_transaction().await.unwrap();
    assert!(
        f.state
            .db
            .collection::<ResourceClaim>(CLAIMS)
            .delete_one(doc! {"_id":&claim.id})
            .session(&mut release)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_prepare_retries_distinct_calls_and_deduplicates_same_call() {
    let f = fixture(Profile::TwilioRelay, false).await;
    let (a, b) = tokio::join!(
        session::prepare(
            &f.state,
            &f.b,
            "distinct-a",
            "+14155550199",
            &f.b.phone_number
        ),
        session::prepare(
            &f.state,
            &f.b,
            "distinct-b",
            "+14155550199",
            &f.b.phone_number
        )
    );
    assert!(a.unwrap().1);
    assert!(b.unwrap().1);
    let (a, b) = tokio::join!(
        session::prepare(
            &f.state,
            &f.b,
            "same-call",
            "+14155550199",
            &f.b.phone_number
        ),
        session::prepare(
            &f.state,
            &f.b,
            "same-call",
            "+14155550199",
            &f.b.phone_number
        )
    );
    let (a, af) = a.unwrap();
    let (b, bf) = b.unwrap();
    assert_eq!(a.id, b.id);
    assert_ne!(af, bf);
    assert_eq!(
        f.state
            .db
            .collection::<CallSession>(SESSIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        3
    );
    // Either removal sees the admitted session or prepare sees disabled authority.
    let (prepared, disabled) = tokio::join!(
        session::prepare(
            &f.state,
            &f.b,
            "disable-race",
            "+14155550199",
            &f.b.phone_number
        ),
        super::management::disable(&f.state, &f.b.user_id, &f.b.id)
    );
    disabled.unwrap();
    if let Ok((s, _)) = prepared {
        assert!(session::fence(&f.state, &s).await.is_err());
    }
    assert!(
        session::prepare(
            &f.state,
            &f.b,
            "after-disable",
            "+14155550199",
            &f.b.phone_number
        )
        .await
        .is_err()
    );
    f.state.db.drop().await.unwrap();
}

struct FundedLago;
#[async_trait::async_trait]
impl crate::services::billing::lago_client::LagoApi for FundedLago {
    async fn ensure_customer(
        &self,
        o: &crate::services::billing::lago_client::OwnerProvisionInput,
    ) -> crate::errors::AppResult<String> {
        Ok(o.external_customer_id.clone())
    }
    async fn ensure_subscription(&self, c: &str, _: &str) -> crate::errors::AppResult<String> {
        Ok(c.into())
    }
    async fn record_event(
        &self,
        e: &crate::services::billing::lago_client::LagoEvent,
    ) -> Result<
        crate::services::billing::lago_client::LagoAck,
        crate::services::billing::lago_client::LagoError,
    > {
        Ok(crate::services::billing::lago_client::LagoAck {
            transaction_id: e.transaction_id.clone(),
        })
    }
    async fn record_events_batch(
        &self,
        es: &[crate::services::billing::lago_client::LagoEvent],
    ) -> Result<
        Vec<crate::services::billing::lago_client::LagoAck>,
        crate::services::billing::lago_client::LagoError,
    > {
        Ok(es
            .iter()
            .map(|e| crate::services::billing::lago_client::LagoAck {
                transaction_id: e.transaction_id.clone(),
            })
            .collect())
    }
    async fn current_usage(
        &self,
        c: &str,
        s: &str,
    ) -> crate::errors::AppResult<crate::services::billing::lago_client::LagoUsage> {
        Ok(crate::services::billing::lago_client::LagoUsage {
            customer_id: c.into(),
            subscription_id: s.into(),
            raw: json!({}),
        })
    }
    async fn wallet_balance(&self, _: &str) -> crate::errors::AppResult<i64> {
        Ok(100)
    }
    async fn entitlements(
        &self,
        _: &str,
    ) -> crate::errors::AppResult<Vec<crate::services::billing::lago_client::Entitlement>> {
        Ok(vec![crate::services::billing::lago_client::Entitlement {
            code: "*".into(),
            raw: json!({}),
        }])
    }
}
async fn fund_sms(f: &mut Fixture) {
    use crate::models::{
        billing_rate_cache::BillingRateCache,
        billing_wallet::{BillingWallet, CollectionState, PlanKind},
        credits::Credits,
        service_billing::{BillingMetric, LanePricing, PricingSyncStatus, ServiceBilling},
    };
    let mut config = f.state.config.clone();
    config.billing_enabled = true;
    f.state.billing = Arc::new(crate::services::billing::BillingService::new_with_lago(
        f.state.db.clone(),
        Arc::new(config.clone()),
        Arc::new(FundedLago),
    ));
    f.state.config = config;
    crate::services::feature_flag_service::set_platform_override(
        &f.state.db,
        crate::services::feature_flag_service::BILLING_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::User(f.b.user_id.clone()),
        true,
        &f.b.user_id,
    )
    .await
    .unwrap();
    let r = binding::live(&f.state, &f.b).await.unwrap();
    let code = "platform_svc_twilio_byok";
    let billing = ServiceBilling {
        byok_pricing: Some(LanePricing {
            metric: BillingMetric::Requests,
            credits_per_unit: "2".into(),
            lago_metric_code: code.into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
            components: vec![],
            operations: vec![],
        }),
        ..Default::default()
    };
    f.state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":r.service.id},
            doc! {"$set":{"billing":bson::to_bson(&billing).unwrap()}},
        )
        .await
        .unwrap();
    let now = Utc::now();
    f.state
        .db
        .collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
        .insert_one(BillingRateCache {
            id: BillingRateCache::cache_id(code, None),
            lago_metric_code: code.into(),
            model: None,
            credits_per_unit_micros: 2_000_000,
            credits_per_unit_pico: None,
            synced_at: now,
            retired_at: None,
        })
        .await
        .unwrap();
    f.state
        .db
        .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
        .insert_one(BillingWallet {
            id: Uuid::new_v4().to_string(),
            owner_id: f.b.user_id.clone(),
            lago_customer_id: f.b.user_id.clone(),
            lago_wallet_id: Some(Uuid::new_v4().to_string()),
            lago_subscription_id: Some(Uuid::new_v4().to_string()),
            plan_kind: PlanKind::Prepaid,
            balance_credits: Credits::from_whole(100),
            reserved_credits: Credits::ZERO,
            pending_lago_debits: Credits::ZERO,
            pending_topup_expiry_credits: Credits::ZERO,
            has_payment_instrument: false,
            overdraft_cap_credits: Credits::ZERO,
            suspended: false,
            collection_state: CollectionState::Good,
            topup_expiry_checked_at: None,
            active_topup_expiry: None,
            balance_synced_at: now,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::usage_meter::COLLECTION_NAME)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"transaction_id":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
}
#[tokio::test]
async fn configured_funded_sms_multipart_releases_remainder_after_revocation_and_duplicate_receipts()
 {
    use crate::models::{billing_wallet::BillingWallet, credits::Credits};
    let mut f = fixture(Profile::TwilioSms, false).await;
    fund_sms(&mut f).await;
    let s = active_session(&f, "funded-sms").await;
    let r = binding::live(&f.state, &f.b).await.unwrap();
    for part in 0..3 {
        let meter = super::billing::reserve(
            &f.state,
            &f.b,
            &r,
            &format!("adapter-sms-out:{}:{part}", s.id),
        )
        .await
        .unwrap();
        f.state.billing.mark_forwarded(&meter).await.unwrap();
    }
    let wallets = f
        .state
        .db
        .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME);
    let wallet = || wallets.find_one(doc! {"owner_id":&f.b.user_id});
    assert_eq!(
        wallet().await.unwrap().unwrap().reserved_credits,
        Credits::from_whole(6)
    );
    let send = EventReceipt {
        id: Uuid::new_v4().to_string(),
        identity_key: session::identity(&[&s.id, "sms_send"]),
        user_id: f.b.user_id.clone(),
        binding_id: f.b.id.clone(),
        binding_revision: f.b.revision,
        session_id: Some(s.id.clone()),
        event_kind: "sms_send".into(),
        status: "accepted".into(),
        provider_message_id: Some("SMfunded".into()),
        delivery: "accepted".into(),
        billing_request_id: None,
        observed_parts: None,
        reserved_parts: Some(3),
        billing_finalized: false,
        from: Some(f.b.phone_number.clone()),
        to: Some(s.from.clone()),
        created_at: Utc::now(),
    };
    f.state
        .db
        .collection::<EventReceipt>(EVENTS)
        .insert_one(&send)
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::billing_rate_cache::COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$set":{"retired_at":bson::DateTime::now()}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
        .update_many(doc! {}, doc! {"$set":{"status":"revoked"}})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Binding>(BINDINGS)
        .update_one(
            doc! {"_id":&f.b.id},
            doc! {"$set":{"enabled":false},"$inc":{"revision":1}},
        )
        .await
        .unwrap();
    for event in ["delivered", "delivered", "reordered"] {
        super::sms_delivery::record_correlated(
            &f.state,
            &f.b,
            super::sms_delivery::Receipt {
                event,
                message: "SMfunded",
                from: &f.b.phone_number,
                to: &s.from,
                status: if event == "reordered" {
                    "queued"
                } else {
                    "delivered"
                },
                parts: Some(2),
                send: Some(&send.id),
            },
        )
        .await
        .unwrap();
    }
    let w = wallet().await.unwrap().unwrap();
    assert_eq!(w.reserved_credits, Credits::ZERO);
    assert_eq!(w.pending_lago_debits, Credits::from_whole(4));
    let saved = f
        .state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find_one(doc! {"_id":&send.id})
        .await
        .unwrap()
        .unwrap();
    assert!(saved.billing_finalized);
    assert_eq!(saved.delivery, "delivered");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn terminal_output_survives_exact_queue_capacity_and_slow_consumer() {
    let f = fixture(Profile::TwilioRelay, false).await;
    *f.faq.write().await = "x".repeat(64);
    f.chunks.store(64, Ordering::SeqCst);
    let s = active_session(&f, "queue-boundary").await;
    let (mut rx, worker) = coordinator::turn(f.state.clone(), &f.b, &s, "capacity", "FAQ")
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    let mut deltas = 0;
    let mut terminal = 0;
    while let Some(out) = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .unwrap()
    {
        assert!(!out.error);
        if out.last {
            terminal += 1;
        } else {
            deltas += 1;
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    assert_eq!(deltas, 64);
    assert_eq!(terminal, 1);
    assert!(worker.await.unwrap().unwrap().error.is_none());
    assert_eq!(f.reads.load(Ordering::SeqCst), 1);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn custom_llm_failed_turn_sends_error_without_success_or_done() {
    let mut f = fixture(Profile::ElevenlabsCustomLlm, false).await;
    f.fail_turn.store(true, Ordering::SeqCst);
    expose_ingress(&mut f).await;
    let setup = super::custom_llm::setup(&f.state, &f.b).unwrap();
    let issued = super::custom_llm::create_session(&f.state, &f.b)
        .await
        .unwrap();
    let body = json!({"stream":true,"elevenlabs_extra_body":issued["extra_body"],"messages":[{"role":"user","content":"FAQ"}]});
    let response = reqwest::Client::new()
        .post(setup["url"].as_str().unwrap())
        .bearer_auth(setup["api_key"].as_str().unwrap())
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let stream = response.text().await.unwrap();
    assert!(stream.contains("agent_response_failed"), "{stream}");
    assert!(!stream.contains("[DONE]"));
    assert!(!stream.contains("\"finish_reason\":\"stop\""));
    assert_eq!(f.requests.lock().await.len(), 1);
    assert_eq!(*f.callback_codes.lock().await, vec![200]);
    assert_eq!(f.reads.load(Ordering::SeqCst), 1);
    f.state.db.drop().await.unwrap();
}

async fn twilio_relay_url(
    f: &mut Fixture,
    call: &str,
) -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
    let commands = attach_carrier(f, call).await;
    expose_ingress(f).await;
    let fields = std::collections::BTreeMap::from([
        ("AccountSid".into(), f.b.account_id.clone()),
        ("CallSid".into(), call.into()),
        ("From".into(), "+14155550199".into()),
        ("To".into(), f.b.phone_number.clone()),
        ("Direction".into(), "inbound".into()),
        ("CallStatus".into(), "in-progress".into()),
    ]);
    let url = coordinator::webhook_url(&f.state, &f.b);
    let response = reqwest::Client::new()
        .post(&url)
        .header(
            "x-twilio-signature",
            twilio_signature(&url, &fields, "fixture-auth-token"),
        )
        .form(&fields)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let xml = response.text().await.unwrap();
    (
        xml.split("url=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .replace("&amp;", "&"),
        commands,
    )
}
fn signed_ws_request(url: &str) -> tokio_tungstenite::tungstenite::http::Request<()> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        "x-twilio-signature",
        twilio_signature(url, &Default::default(), "fixture-auth-token")
            .parse()
            .unwrap(),
    );
    request
}

#[tokio::test]
async fn actual_twilio_upgrade_accepts_https_signature_for_configured_wss_endpoint() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let mut f = fixture(Profile::TwilioRelay, false).await;
    let call = "CA22222222222222222222222222222222";
    let commands = attach_carrier(&mut f, call).await;
    // Model a TLS-terminating ingress: the backend fixture is local HTTP, while
    // signatures and issued URLs bind the configured public HTTPS endpoint.
    let backend_origin = expose_ingress_scheme(&mut f, "https").await;
    let public_webhook = coordinator::webhook_url(&f.state, &f.b);
    let fields = std::collections::BTreeMap::from([
        ("AccountSid".into(), f.b.account_id.clone()),
        ("CallSid".into(), call.into()),
        ("From".into(), "+14155550199".into()),
        ("To".into(), f.b.phone_number.clone()),
        ("Direction".into(), "inbound".into()),
        ("CallStatus".into(), "in-progress".into()),
    ]);
    let response = reqwest::Client::new()
        .post(format!(
            "{backend_origin}/api/v1/webhooks/call-adapters/{}",
            f.b.id
        ))
        .header(
            "x-twilio-signature",
            twilio_signature(&public_webhook, &fields, "fixture-auth-token"),
        )
        .form(&fields)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let xml = response.text().await.unwrap();
    let websocket = xml
        .split("url=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .replace("&amp;", "&");
    assert!(websocket.starts_with("wss://"));
    let mut request = signed_ws_request(&websocket);
    let https = format!("https://{}", websocket.strip_prefix("wss://").unwrap());
    request.headers_mut().insert(
        "x-twilio-signature",
        twilio_signature(&https, &Default::default(), "fixture-auth-token")
            .parse()
            .unwrap(),
    );
    let public = url::Url::parse(&websocket).unwrap();
    let stream =
        tokio::net::TcpStream::connect((public.host_str().unwrap(), public.port().unwrap()))
            .await
            .unwrap();
    let (mut ws, response) = tokio_tungstenite::client_async(request, stream)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    ws.send(ws_setup(&f, call)).await.unwrap();
    ws.send(Message::Text(
        json!({"type":"prompt","voicePrompt":"What does the FAQ say?","last":true})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let mut answer = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let message = ws.next().await.unwrap().unwrap();
            let value: Value = serde_json::from_slice(&message.into_data()).unwrap();
            if value["type"] == "text" {
                answer.push_str(value["token"].as_str().unwrap_or_default());
                if value["last"] == true {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(answer.contains("The office opens at nine."));
    assert_eq!(f.reads.load(Ordering::SeqCst), 1);
    assert_eq!(f.callback_codes.lock().await.as_slice(), &[200]);
    ws.close(None).await.unwrap();
    wait_closed(&f, call).await;
    assert!(
        commands
            .lock()
            .await
            .iter()
            .any(|(_, body)| body["Status"] == "completed")
    );
    f.state.db.drop().await.unwrap();
}
fn ws_setup(f: &Fixture, call: &str) -> tokio_tungstenite::tungstenite::Message {
    tokio_tungstenite::tungstenite::Message::Text(json!({"type":"setup","sessionId":"provider-session","accountSid":f.b.account_id,"callSid":call,"from":"+14155550199","to":f.b.phone_number,"direction":"inbound"}).to_string().into())
}
async fn wait_closed(f: &Fixture, call: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let s = session::by_call(&f.state, &f.b, call).await.unwrap();
            if s.state == SessionState::Closed && s.billing_finalized {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn websocket_burst_interrupt_cancels_stale_output_without_repeating_faq_effect() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let mut f = fixture(Profile::TwilioRelay, false).await;
    *f.faq.write().await = format!("OLD_REPLY_{}", "a".repeat(500));
    f.chunks.store(200, Ordering::SeqCst);
    f.chunk_delay_ms.store(4, Ordering::SeqCst);
    let call = "CA44444444444444444444444444444444";
    let (url, _) = twilio_relay_url(&mut f, call).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(signed_ws_request(&url))
        .await
        .unwrap();
    ws.send(ws_setup(&f, call)).await.unwrap();
    ws.send(Message::Text(
        json!({"type":"prompt","voicePrompt":"FAQ","last":true})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let first = tokio::time::timeout(std::time::Duration::from_secs(15), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let frame: Value = serde_json::from_slice(&first.into_data()).unwrap();
    let prefix = frame["token"].as_str().unwrap().to_owned();
    assert!(!prefix.is_empty());
    assert_eq!(f.reads.load(Ordering::SeqCst), 1);
    ws.send(Message::Text(
        json!({"type":"interrupt","utteranceUntilInterrupt":prefix,"durationUntilInterruptMs":40})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if session::by_call(&f.state, &f.b, call)
                .await
                .unwrap()
                .generation
                == 1
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("interrupt stays prompt during output burst");
    *f.faq.write().await = "NEXT_REPLY_ONLY".into();
    f.chunks.store(1, Ordering::SeqCst);
    f.chunk_delay_ms.store(0, Ordering::SeqCst);
    ws.send(Message::Text(
        json!({"type":"prompt","voicePrompt":"Read updated FAQ","last":true})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let mut replies = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let frame = ws.next().await.unwrap().unwrap();
            let v: Value = serde_json::from_slice(&frame.into_data()).unwrap();
            replies.push_str(v["token"].as_str().unwrap_or(""));
            if v["last"] == true {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(replies.contains("NEXT_REPLY_ONLY"));
    assert!(!replies.contains(&"a".repeat(500)));
    assert_eq!(f.reads.load(Ordering::SeqCst), 2);
    ws.close(None).await.unwrap();
    wait_closed(&f, call).await;
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn concurrent_duplicate_websocket_loser_cannot_cleanup_winner() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let mut f = fixture(Profile::TwilioRelay, false).await;
    let call = "CA55555555555555555555555555555555";
    let (url, commands) = twilio_relay_url(&mut f, call).await;
    let (a, b) = tokio::join!(
        tokio_tungstenite::connect_async(signed_ws_request(&url)),
        tokio_tungstenite::connect_async(signed_ws_request(&url))
    );
    let mut sockets = Vec::new();
    for (mut ws, _) in [a, b].into_iter().flatten() {
        let _ = ws.send(ws_setup(&f, call)).await;
        let _ = ws
            .send(Message::Text(
                json!({"type":"prompt","voicePrompt":"FAQ","last":true})
                    .to_string()
                    .into(),
            ))
            .await;
        sockets.push(ws);
    }
    assert!(!sockets.is_empty());
    let winner = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            for (index, ws) in sockets.iter_mut().enumerate() {
                if let Ok(Some(Ok(frame))) =
                    tokio::time::timeout(std::time::Duration::from_millis(100), ws.next()).await
                    && frame.is_text()
                {
                    let v: Value = serde_json::from_slice(&frame.into_data()).unwrap();
                    if v["last"] == true {
                        return index;
                    }
                }
            }
            // Closed sockets can remain immediately ready; let the outer
            // deadline run even if neither contender produces another frame.
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(f.reads.load(Ordering::SeqCst), 1);
    assert!(
        !commands
            .lock()
            .await
            .iter()
            .any(|(_, b)| b["Status"] == "completed")
    );
    sockets[winner].close(None).await.unwrap();
    wait_closed(&f, call).await;
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn websocket_disconnect_stops_accepted_turn_before_tool_callback() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let mut f = fixture(Profile::TwilioRelay, true).await;
    let call = "CA66666666666666666666666666666666";
    let (url, _) = twilio_relay_url(&mut f, call).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(signed_ws_request(&url))
        .await
        .unwrap();
    ws.send(ws_setup(&f, call)).await.unwrap();
    ws.send(Message::Text(
        json!({"type":"prompt","voicePrompt":"FAQ","last":true})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while f.requests.lock().await.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    ws.close(None).await.unwrap();
    let _ = ws.next().await;
    wait_closed(&f, call).await;
    f.gate.add_permits(1);
    // Disconnect may cancel NyxAgent's HTTP handler before it can issue a
    // callback. Wait for that handler to finish or be dropped, so an empty
    // status list proves cancellation rather than a callback still pending.
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        f.upstream_finished.cancelled(),
    )
    .await
    .unwrap();
    assert_eq!(f.requests.lock().await.len(), 1);
    let codes = f.callback_codes.lock().await;
    match codes.as_slice() {
        [] => {} // The accepted handler was cancelled before a response.
        [code] => assert!(!(200..300).contains(code)),
        _ => panic!("expected at most one callback from the accepted turn"),
    }
    assert_eq!(f.reads.load(Ordering::SeqCst), 0);
    drop(codes);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn custom_llm_disconnected_body_stops_slow_tool_and_full_queue_is_bounded() {
    for disconnect in [true, false] {
        let f = fixture(Profile::ElevenlabsCustomLlm, disconnect).await;
        *f.faq.write().await = "x".repeat(200);
        f.chunks.store(200, Ordering::SeqCst);
        let issued = super::custom_llm::create_session(&f.state, &f.b)
            .await
            .unwrap();
        let setup = super::custom_llm::setup(&f.state, &f.b).unwrap();
        let app = Router::new()
            .route(
                "/llm/{id}",
                post(crate::handlers::call_adapters::completions),
            )
            .with_state(f.state.clone());
        let body = json!({"stream":true,"elevenlabs_extra_body":issued["extra_body"],"messages":[{"role":"user","content":"FAQ"}]});
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/llm/{}", f.b.id))
                    .header(
                        "authorization",
                        format!("Bearer {}", setup["api_key"].as_str().unwrap()),
                    )
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            while f.requests.lock().await.is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let held = if disconnect {
            drop(response);
            None
        } else {
            Some(response)
        };
        tokio::time::timeout(std::time::Duration::from_secs(13), async {
            loop {
                let s = session::load(&f.state, issued["session_id"].as_str().unwrap())
                    .await
                    .unwrap();
                let row = f
                    .state
                    .db
                    .collection::<AssistantConversation>(
                        crate::models::assistant_conversation::COLLECTION_NAME,
                    )
                    .find_one(doc! {"_id":s.conversation_id})
                    .await
                    .unwrap()
                    .unwrap();
                if row.active_turn.as_ref().is_none_or(|t| t.stop_requested) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect(
            "disconnect or blocked consumer must cancel producer within bounded terminal waits",
        );
        if disconnect {
            f.gate.add_permits(1);
            // Cancellation can finish or drop the accepted upstream handler
            // before it returns a callback status. Observe completion first.
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                f.upstream_finished.cancelled(),
            )
            .await
            .unwrap();
            assert_eq!(f.requests.lock().await.len(), 1);
            let codes = f.callback_codes.lock().await;
            match codes.as_slice() {
                [] => {} // The accepted handler was cancelled before a response.
                [code] => assert!(*code >= 400, "the stopped turn must reject its callback"),
                _ => panic!("expected at most one callback from the accepted turn"),
            }
            assert_eq!(f.reads.load(Ordering::SeqCst), 0);
        }
        drop(held);
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn final_call_dispatch_rechecks_revocation_and_exact_catalog_policy_after_proof() {
    use crate::models::downstream_service::{ProxyOperationPolicy, ProxyOperationRule};
    for revoke in [true, false] {
        let mut f = fixture(Profile::TwilioRelay, false).await;
        let s = active_session(&f, "CA77777777777777777777777777777777").await;
        let r = binding::live(&f.state, &f.b).await.unwrap();
        let entered = Arc::new(Semaphore::new(0));
        let released = Arc::new(Semaphore::new(0));
        let e = entered.clone();
        let release = released.clone();
        let b = f.b.clone();
        let call = s.provider_call_id.clone();
        let posts = Arc::new(AtomicUsize::new(0));
        let count = posts.clone();
        let provider=Router::new().fallback(any(move|req:Request<Body>|{let b=b.clone();let call=call.clone();let e=e.clone();let release=release.clone();let count=count.clone();async move{if req.method()==axum::http::Method::POST{count.fetch_add(1,Ordering::SeqCst);return Json(json!({"sid":call,"account_sid":b.account_id}));}e.add_permits(1);release.acquire().await.unwrap().forget();Json(json!({"sid":call,"account_sid":b.account_id,"phone_number_sid":b.resource_id,"from":"+14155550199","to":b.phone_number,"direction":"inbound","status":"in-progress"}))}}));
        let (origin, task) = serve(provider).await;
        f.tasks.push(task);
        profiles::FIXTURE_ORIGINS
            .lock()
            .unwrap()
            .insert(f.b.account_id.clone(), origin);
        let state = f.state.clone();
        let bound = f.b.clone();
        let session = s.clone();
        let effect = tokio::spawn(async move {
            super::carrier::twilio_update(&state, &bound, &r, &session, &[("TimeLimit", "1800")])
                .await
        });
        entered.acquire().await.unwrap().forget();
        if revoke {
            f.state
                .db
                .collection::<Binding>(BINDINGS)
                .update_one(
                    doc! {"_id":&f.b.id},
                    doc! {"$set":{"enabled":false},"$inc":{"revision":1}},
                )
                .await
                .unwrap();
        } else {
            let r = binding::live(&f.state, &f.b).await.unwrap();
            let policy = ProxyOperationPolicy {
                rules: vec![ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/2010-04-01/Accounts/{account}/Calls/{call}".into(),
                    ..Default::default()
                }],
            };
            f.state
                .db
                .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":r.service.id},
                    doc! {"$set":{"proxy_operation_policy":bson::to_bson(&policy).unwrap()}},
                )
                .await
                .unwrap();
        }
        released.add_permits(1);
        assert!(effect.await.unwrap().is_err());
        assert_eq!(posts.load(Ordering::SeqCst), 0);
        f.state.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn telnyx_proof_paginates_and_read_only_policy_blocks_call_and_sms_effects() {
    use crate::models::downstream_service::{ProxyOperationPolicy, ProxyOperationRule};
    let mut f = fixture(Profile::TelnyxRelay, false).await;
    let b = f.b.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let provider=Router::new().fallback(any(move|req:Request<Body>|{let b=b.clone();let seen=seen.clone();async move{seen.fetch_add(1,Ordering::SeqCst);if req.uri().query().is_none(){Json(json!({"data":[{"call_control_id":"other-call"}],"meta":{"next":format!("/v2/connections/{}/active_calls?page[after]=cursor==",b.application_id)}}))}else{Json(json!({"data":[{"call_control_id":"v2:later=="}],"meta":{"next":null}}))}}}));
    let (origin, task) = serve(provider).await;
    f.tasks.push(task);
    profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(f.b.account_id.clone(), origin);
    let r = binding::live(&f.state, &f.b).await.unwrap();
    assert!(
        super::carrier::prove_call(
            &f.state,
            &f.b,
            &r,
            "v2:later==",
            "+14155550199",
            &f.b.phone_number
        )
        .await
        .unwrap()
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert!(
        super::carrier::prove_call(
            &f.state,
            &f.b,
            &r,
            "v2:nonmember==",
            "+14155550199",
            &f.b.phone_number
        )
        .await
        .is_err()
    );
    let policy = ProxyOperationPolicy {
        rules: vec![ProxyOperationRule {
            method: "GET".into(),
            path_template: "/connections/{id}/active_calls".into(),
            ..Default::default()
        }],
    };
    f.state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&r.service.id},
            doc! {"$set":{"proxy_operation_policy":bson::to_bson(&policy).unwrap()}},
        )
        .await
        .unwrap();
    let r = binding::live(&f.state, &f.b).await.unwrap();
    assert!(
        super::carrier::request(
            &f.state,
            &f.b,
            &r,
            "POST",
            "/v2/calls/v2:later/actions/answer",
            Some(json!({}))
        )
        .await
        .is_err()
    );
    assert!(
        binding::authorize(&f.state, &f.b, &r, "POST", "/v2/messages", None)
            .await
            .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 4);
    let ready = binding::availability(&f.state, &f.b).await;
    assert!(!ready.available);
    for operation in [
        channel_capability::Operation::ReceiveCall,
        channel_capability::Operation::EndSession,
    ] {
        let availability = ready
            .operations
            .iter()
            .find(|entry| entry.operation == operation)
            .unwrap();
        assert!(availability.implemented);
        assert!(!availability.available);
        assert!(matches!(
            availability.reason,
            Some(binding::UnavailableReason::ResourcePolicyUnresolved)
        ));
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn twilio_call_policy_readiness_requires_unresolved_live_call_prerequisites() {
    use crate::models::downstream_service::{ProxyOperationPolicy, ProxyOperationRule};
    let f = fixture(Profile::TwilioRelay, false).await;
    let resolved = binding::live(&f.state, &f.b).await.unwrap();
    let policy = ProxyOperationPolicy {
        rules: vec![
            ProxyOperationRule {
                method: "GET".into(),
                path_template: "/2010-04-01/Accounts/{account}/Calls/{call}".into(),
                ..Default::default()
            },
            ProxyOperationRule {
                method: "POST".into(),
                path_template: "/2010-04-01/Accounts/{account}/Calls/{call}".into(),
                ..Default::default()
            },
        ],
    };
    f.state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":resolved.service.id},
            doc! {"$set":{"proxy_operation_policy":bson::to_bson(&policy).unwrap()}},
        )
        .await
        .unwrap();
    let ready = binding::availability(&f.state, &f.b).await;
    for operation in [
        channel_capability::Operation::ReceiveCall,
        channel_capability::Operation::EndSession,
    ] {
        let availability = ready
            .operations
            .iter()
            .find(|entry| entry.operation == operation)
            .unwrap();
        assert!(availability.implemented);
        assert!(!availability.available);
        assert!(matches!(
            availability.reason,
            Some(binding::UnavailableReason::ResourcePolicyUnresolved)
        ));
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxagent_dispatch_after_forwarded_mark_rechecks_turn_and_key_and_settles_zero() {
    use crate::models::usage_meter::{UsageMeterRow, UsageStatus};
    use crate::services::{
        assistant_nyxagent_egress::{self as egress, DispatchBoundary},
        platform_manager_contract::{
            NyxAgentExecutionContext, NyxAgentRequest, RuntimeCredential, TurnRef,
        },
    };
    for mutation in ["stop", "replace_turn", "revoke_key"] {
        let f = fixture(Profile::TwilioRelay, false).await;
        let s = active_session(&f, "dispatch-boundary-call").await;
        let claim = session::admit(&f.state, &f.b, &s, "utterance", "Read the FAQ", None)
            .await
            .unwrap();
        let turn = TurnRef {
            conversation_id: claim.conversation.id,
            turn_id: claim.conversation.active_turn.unwrap().turn_id,
        };
        let key_id = claim.credential.api_key_id.clone();
        let credential = RuntimeCredential {
            api_key_id: key_id.clone(),
            raw_key: claim.credential.raw_key,
        };
        let authority = NyxAgentExecutionContext::Owner {
            user_id: f.b.user_id.clone(),
            turn: turn.clone(),
            platform_origin: None,
        };
        let mut state = f.state.clone();
        let mut config = state.config.clone();
        config.billing_enabled = true;
        state.billing = Arc::new(crate::services::billing::BillingService::new(
            state.db.clone(),
            Arc::new(config),
        ));
        let boundary = Arc::new(DispatchBoundary {
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
        });
        let transport = egress::channel_at_dispatch_boundary(state, s, boundary.clone());
        let pending = tokio::spawn(async move {
            transport
                .request(
                    &authority,
                    &credential,
                    NyxAgentRequest {
                        method: axum::http::Method::POST,
                        relative_path: "v1/responses".into(),
                        body: Some(json!({"input":"Read the FAQ"})),
                        idempotency_key: "dispatch-boundary".into(),
                        billing_policy: Some(
                            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                                crate::services::billing::BillingIngress::Proxy,
                            ),
                        ),
                    },
                )
                .await
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            boundary.entered.acquire(),
        )
        .await
        .unwrap()
        .unwrap()
        .forget();
        let meters = f
            .state
            .db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME);
        let before = meters.find_one(doc! {}).await.unwrap().unwrap();
        assert_eq!(before.status, UsageStatus::Forwarded);
        assert!(before.forwarded);
        assert_eq!(f.requests.lock().await.len(), 0);
        match mutation {
            "stop" => engine::request_stop(&f.state.db, &f.b.user_id, &turn.conversation_id)
                .await
                .unwrap(),
            "replace_turn" => {
                f.state
                    .db
                    .collection::<AssistantConversation>(
                        crate::models::assistant_conversation::COLLECTION_NAME,
                    )
                    .update_one(
                        doc! {"_id":&turn.conversation_id,"active_turn.turn_id":&turn.turn_id},
                        doc! {"$set":{"active_turn.turn_id":Uuid::new_v4().to_string()}},
                    )
                    .await
                    .unwrap();
            }
            "revoke_key" => {
                f.state
                    .db
                    .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
                    .update_one(doc! {"_id":key_id}, doc! {"$set":{"is_active":false}})
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        boundary.release.add_permits(1);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(10), pending)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert_eq!(
            f.requests.lock().await.len(),
            0,
            "{mutation} must prevent upstream dispatch"
        );
        assert_eq!(f.reads.load(Ordering::SeqCst), 0);
        assert!(f.callback_codes.lock().await.is_empty());
        let after = meters
            .find_one(doc! {"_id":&before.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, UsageStatus::Finalized);
        assert_eq!(after.quantity, Some(0));
        assert!(after.released);
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn twilio_rejected_or_uncertain_cap_never_releases_relay_and_never_replays_cap() {
    for outcome in [1, 2] {
        let mut f = fixture(Profile::TwilioRelay, false).await;
        let call = "CA88888888888888888888888888888888";
        let commands = attach_carrier_behavior(
            &mut f,
            call,
            Arc::new(RwLock::new("in-progress".into())),
            outcome,
        )
        .await;
        expose_ingress(&mut f).await;
        let fields = std::collections::BTreeMap::from([
            ("AccountSid".into(), f.b.account_id.clone()),
            ("CallSid".into(), call.into()),
            ("From".into(), "+14155550199".into()),
            ("To".into(), f.b.phone_number.clone()),
            ("Direction".into(), "inbound".into()),
            ("CallStatus".into(), "in-progress".into()),
        ]);
        let url = coordinator::webhook_url(&f.state, &f.b);
        let client = reqwest::Client::new();
        for _ in 0..2 {
            let response = client
                .post(&url)
                .header(
                    "x-twilio-signature",
                    twilio_signature(&url, &fields, "fixture-auth-token"),
                )
                .form(&fields)
                .send()
                .await
                .unwrap();
            assert!(!response.status().is_success());
            assert!(!response.text().await.unwrap().contains("ConversationRelay"));
        }
        let s = session::by_call(&f.state, &f.b, call).await.unwrap();
        assert!(!s.cap_confirmed);
        assert!(s.socket_ticket_hash.is_none());
        assert_eq!(commands.lock().await.len(), 1);
        assert_eq!(f.reads.load(Ordering::SeqCst), 0);
        coordinator::close(&f.state, &f.b, &s, false).await.unwrap();
        wait_closed(&f, call).await;
        f.state.db.drop().await.unwrap();
    }
}
async fn platform_binding(f: &mut Fixture) -> ProviderAccount {
    use crate::models::downstream_service::{PlatformKeyAudience, PlatformKeyConfig};
    use base64::Engine;
    let r = binding::live(&f.state, &f.b).await.unwrap();
    let mut catalog = r.service;
    catalog.requires_user_credential = false;
    catalog.credential_encrypted = f
        .state
        .encryption_keys
        .encrypt(b"carrier-fixture")
        .await
        .unwrap();
    catalog.platform_key = Some(PlatformKeyConfig {
        enabled: true,
        audience: PlatformKeyAudience::Restricted,
        allowed_owner_ids: vec![f.b.user_id.clone()],
    });
    f.state
        .db
        .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
        .replace_one(doc! {"_id":&catalog.id}, &catalog)
        .await
        .unwrap();
    f.b.credential = CredentialBinding::Platform {
        catalog_service_id: catalog.id.clone(),
    };
    let signer = ed25519_dalek::SigningKey::from_bytes(&[8; 32]);
    let key = base64::engine::general_purpose::STANDARD.encode(signer.verifying_key().to_bytes());
    let account = ProviderAccount {
        id: Uuid::new_v4().to_string(),
        catalog_service_id: catalog.id.clone(),
        provider: profiles::provider(f.b.profile).into(),
        account_id: f.b.account_id.clone(),
        webhook_public_key: Some(key.clone()),
        revision: 1,
        configured_by: f.b.user_id.clone(),
        updated_at: Utc::now(),
    };
    f.state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .insert_one(&account)
        .await
        .unwrap();
    let allocation = ResourceAllocation {
        id: Uuid::new_v4().to_string(),
        catalog_service_id: catalog.id,
        user_id: f.b.user_id.clone(),
        provider: profiles::provider(f.b.profile).into(),
        account_id: f.b.account_id.clone(),
        resource_id: f.b.resource_id.clone(),
        phone_number: f.b.phone_number.clone(),
        application_id: f.b.application_id.clone(),
        allocated_by: f.b.user_id.clone(),
        revision: 1,
        tariff_version: "fixture".into(),
        tariff_source: "https://example.com/tariff".into(),
        includes_all_provider_charges: true,
        tariff_verified_at: Utc::now(),
        created_at: Utc::now(),
    };
    f.state
        .db
        .collection::<ResourceAllocation>(ALLOCATIONS)
        .insert_one(&allocation)
        .await
        .unwrap();
    f.b.webhook_public_key = Some(key);
    f.b.trusted_account_revision = Some(1);
    f.b.verified_credential_identity =
        binding::resolve(&f.state, &f.b.user_id, &f.b.credential, f.b.profile)
            .await
            .unwrap()
            .identity;
    f.state
        .db
        .collection::<Binding>(BINDINGS)
        .replace_one(doc! {"_id":&f.b.id}, &f.b)
        .await
        .unwrap();
    account
}
#[tokio::test]
async fn trusted_platform_key_rotation_and_forged_ingress_never_control_another_resource() {
    use base64::Engine;
    let mut f = fixture(Profile::TelnyxRelay, false).await;
    let account = platform_binding(&mut f).await;
    let commands = attach_carrier(&mut f, "v2:allocated==").await;
    expose_ingress(&mut f).await;
    assert!(binding::read_authority(&f.state.db, &f.b).await.is_ok());
    let forger = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let response = telnyx_post(&f, &forger, "call.initiated", "v2:another-tenant==").await;
    assert!(!response.status().is_success());
    assert!(commands.lock().await.is_empty());
    // Even a correctly signed resource-shaped callback cannot prove call membership.
    let r = binding::live(&f.state, &f.b).await.unwrap();
    assert!(
        super::carrier::prove_call(
            &f.state,
            &f.b,
            &r,
            "v2:another-tenant==",
            "+14155550199",
            &f.b.phone_number
        )
        .await
        .is_err()
    );
    assert!(commands.lock().await.is_empty());
    let input = binding::SetupInput {
        adapter_id: profiles::id(f.b.profile).into(),
        contract_version: 1,
        label: "forged key".into(),
        agent_id: f.b.agent_id.clone(),
        credential: f.b.credential.clone(),
        account_id: f.b.account_id.clone(),
        resource_id: f.b.resource_id.clone(),
        phone_number: f.b.phone_number.clone(),
        application_id: f.b.application_id.clone(),
        webhook_public_key: Some(
            base64::engine::general_purpose::STANDARD.encode(forger.verifying_key().to_bytes()),
        ),
        voice: f.b.voice.clone(),
        language: f.b.language.clone(),
    };
    assert!(matches!(
        binding::create(&f.state, &f.b.user_id, input).await,
        Err(crate::errors::AppError::Forbidden(_))
    ));
    let s = active_session(&f, "v2:allocated==").await;
    f.state
        .db
        .collection::<ProviderAccount>(ACCOUNTS)
        .update_one(doc! {"_id":&account.id}, doc! {"$inc":{"revision":1}})
        .await
        .unwrap();
    assert!(binding::read_authority(&f.state.db, &f.b).await.is_err());
    assert!(binding::verification_key(&f.state, &f.b).await.is_err());
    assert!(session::fence(&f.state, &s).await.is_err());
    // Signature material stays out of Debug even for rejected records.
    assert!(!format!("{:?}", f.b).contains(&f.b.verified_credential_identity));
    f.state.db.drop().await.unwrap();
}

fn pending_sms_send(b: &Binding, s: &CallSession) -> EventReceipt {
    EventReceipt {
        id: Uuid::new_v4().to_string(),
        identity_key: session::identity(&[&s.id, "sms_send"]),
        user_id: b.user_id.clone(),
        binding_id: b.id.clone(),
        binding_revision: b.revision,
        session_id: Some(s.id.clone()),
        event_kind: "sms_send".into(),
        status: "dispatching".into(),
        provider_message_id: None,
        delivery: "unknown".into(),
        billing_request_id: None,
        observed_parts: None,
        reserved_parts: Some(1),
        billing_finalized: false,
        from: Some(b.phone_number.clone()),
        to: Some(s.from.clone()),
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn deletion_closes_owner_admission_before_pending_check_and_remains_retryable() {
    let f = fixture(Profile::TwilioSms, false).await;
    let s = active_session(&f, "deletion-pending").await;
    let error = crate::services::admin_user_service::delete_user_cascade(
        &f.state.db,
        &Uuid::new_v4().to_string(),
        &f.b.user_id,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, crate::errors::AppError::Conflict(_)));
    let owner = f
        .state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.b.user_id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        owner.is_active,
        "Pending cleanup must retain authentication for retry"
    );
    assert!(binding::read_authority(&f.state.db, &f.b).await.is_err());
    assert!(
        session::prepare(&f.state, &f.b, "after-close", &s.from, &s.to)
            .await
            .is_err()
    );
    let mut replacement = f.b.clone();
    replacement.id = Uuid::new_v4().to_string();
    replacement.resource_id = "PNreplacement".into();
    replacement.application_id = "APreplacement".into();
    replacement.phone_number = "+18005550124".into();
    assert!(binding::persist(&f.state, &replacement).await.is_err());
    assert!(
        session::insert_sms_send(&f.state, &f.b, &s, &pending_sms_send(&f.b, &s))
            .await
            .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<EventReceipt>(EVENTS)
            .count_documents(doc! {"event_kind":"sms_send"})
            .await
            .unwrap(),
        0
    );
    session::finish_noncall(&f.state, &s.id, "owner_deleted")
        .await
        .unwrap();
    crate::services::admin_user_service::delete_user_cascade(
        &f.state.db,
        &Uuid::new_v4().to_string(),
        &f.b.user_id,
    )
    .await
    .unwrap();
    assert!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":&f.b.user_id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        session::prepare(&f.state, &f.b, "after-purge", &s.from, &s.to)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn deletion_and_sms_send_preparation_serialize_without_orphan_effect_identity() {
    let f = fixture(Profile::TwilioSms, false).await;
    let s = active_session(&f, "deletion-send-race").await;
    let send = pending_sms_send(&f.b, &s);
    let admin = Uuid::new_v4().to_string();
    let (admitted, deletion) = tokio::join!(
        session::insert_sms_send(&f.state, &f.b, &s, &send),
        crate::services::admin_user_service::delete_user_cascade(&f.state.db, &admin, &f.b.user_id),
    );
    assert!(
        deletion.is_err(),
        "The existing session preserves cleanup evidence"
    );
    let saved = f
        .state
        .db
        .collection::<EventReceipt>(EVENTS)
        .find_one(doc! {"_id":&send.id})
        .await
        .unwrap();
    assert_eq!(saved.is_some(), admitted.is_ok());
    assert!(
        super::owner_admission::check(&f.state.db, &f.b.user_id)
            .await
            .is_err()
    );
    session::finish_noncall(&f.state, &s.id, "closed_during_send")
        .await
        .unwrap();
    assert!(
        session::insert_sms_send(&f.state, &f.b, &s, &pending_sms_send(&f.b, &s))
            .await
            .is_err()
    );
    if admitted.is_ok() {
        // The send identity remains even after local session closure: deletion
        // waits for the admitted outcome, and never invents or repeats a POST.
        assert!(
            crate::services::admin_user_service::delete_user_cascade(
                &f.state.db,
                &admin,
                &f.b.user_id
            )
            .await
            .is_err()
        );
        assert!(
            f.state
                .db
                .collection::<EventReceipt>(EVENTS)
                .find_one(doc! {"_id":&send.id})
                .await
                .unwrap()
                .is_some()
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn closed_session_refuses_late_sms_send_record_and_rollout_readiness_is_disabled() {
    let f = fixture(Profile::TwilioSms, false).await;
    let s = active_session(&f, "closed-sms").await;
    session::finish_noncall(&f.state, &s.id, "disabled")
        .await
        .unwrap();
    assert!(
        session::insert_sms_send(&f.state, &f.b, &s, &pending_sms_send(&f.b, &s))
            .await
            .is_err()
    );
    crate::services::feature_flag_service::set_platform_override(
        &f.state.db,
        crate::services::feature_flag_service::NYXBOT_ADAPTERS_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::User(f.b.user_id.clone()),
        false,
        &f.b.user_id,
    )
    .await
    .unwrap();
    let readiness = binding::availability(&f.state, &f.b).await;
    assert!(matches!(
        readiness.reason,
        Some(binding::UnavailableReason::Disabled)
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn sms_final_dispatch_uses_refreshed_material_without_replacing_credential_epoch() {
    use base64::Engine;
    let mut f = fixture(Profile::TwilioSms, false).await;
    let s = active_session(&f, "refresh-sms").await;
    assert_eq!(
        run_turn(&f, &s, "faq", "When do you open?").await,
        "The office opens at nine."
    );
    let admitted = binding::live(&f.state, &f.b).await.unwrap();
    let CredentialBinding::Connection { user_service_id } = &f.b.credential else {
        panic!("BYOK fixture");
    };
    let connection = f
        .state
        .db
        .collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":user_service_id})
        .await
        .unwrap()
        .unwrap();
    let raw = format!("{}:refreshed-auth-token", f.b.account_id);
    let encrypted = f
        .state
        .encryption_keys
        .encrypt(raw.as_bytes())
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME)
        .update_one(doc! {"_id":connection.api_key_id.as_deref().unwrap()}, doc! {"$set":{"credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}}}).await.unwrap();
    let seen = Arc::new(AtomicUsize::new(0));
    let count = seen.clone();
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(raw)
    );
    let (origin, task) = serve(Router::new().fallback(post(move |headers: HeaderMap| {
        let expected = expected.clone();
        let count = count.clone();
        async move {
            assert_eq!(
                headers.get("authorization").unwrap().to_str().unwrap(),
                expected
            );
            count.fetch_add(1, Ordering::SeqCst);
            Json(json!({"sid":"SMrefreshed","num_segments":"1"}))
        }
    })))
    .await;
    f.tasks.push(task);
    profiles::FIXTURE_ORIGINS
        .lock()
        .unwrap()
        .insert(f.b.account_id.clone(), origin);
    let path = format!("/2010-04-01/Accounts/{}/Messages.json", f.b.account_id);
    let payload =
        profiles::sms_payload(&f.b, &s.from, "FAQ answer", "https://example.com/receipt").unwrap();
    let live = coordinator::sms_dispatch_authority(
        &f.state,
        &f.b,
        &s,
        &admitted.identity,
        &path,
        &payload,
    )
    .await
    .unwrap();
    assert_eq!(live.identity, admitted.identity);
    assert_ne!(&*live.key, &*admitted.key);
    let channel_adapters::RegisteredInterface::Nyxbot(profiles::Adapter::Messaging(adapter)) = channel_adapters::registered_interfaces(&f.state.token_exchange_cache)
        .into_iter().find(|a| matches!(a, channel_adapters::RegisteredInterface::Nyxbot(a) if a.profile()==Profile::TwilioSms)).unwrap() else { panic!("executable SMS adapter"); };
    adapter
        .send(
            crate::services::proxy_service::no_redirect_http_client(),
            &f.b,
            &live.key,
            &s.from,
            "FAQ answer",
            "https://example.com/receipt",
        )
        .await
        .unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn noncall_activation_never_resurrects_closed_sessions_and_disable_expires_excess() {
    for profile in [Profile::TwilioSms, Profile::ElevenlabsCustomLlm] {
        let f = fixture(profile, false).await;
        let (prepared, _) = session::prepare(
            &f.state,
            &f.b,
            "closed-before-active",
            "external_guest",
            "external_speech",
        )
        .await
        .unwrap();
        session::finish_noncall(&f.state, &prepared.id, "closed_before_activation")
            .await
            .unwrap();
        assert!(
            session::activate_noncall(&f.state, &f.b, &prepared, Some("unused_digest".into()))
                .await
                .is_err()
        );
        let closed = session::load(&f.state, &prepared.id).await.unwrap();
        assert_eq!(closed.state, SessionState::Closed);
        assert!(closed.ended_at.is_some() && closed.billing_finalized);
        assert!(closed.socket_ticket_hash.is_none());
        f.state.db.drop().await.unwrap();
    }
    let f = fixture(Profile::ElevenlabsCustomLlm, false).await;
    let template = active_session(&f, "many-active").await;
    let mut rows = Vec::new();
    for _ in 0..100 {
        let mut s = template.clone();
        s.id = Uuid::new_v4().to_string();
        s.identity_key = Uuid::new_v4().to_string();
        s.conversation_id = Uuid::new_v4().to_string();
        rows.push(s);
    }
    f.state
        .db
        .collection::<CallSession>(SESSIONS)
        .insert_many(rows)
        .await
        .unwrap();
    super::management::disable(&f.state, &f.b.user_id, &f.b.id)
        .await
        .unwrap();
    let remaining = f
        .state
        .db
        .collection::<CallSession>(SESSIONS)
        .find_one(doc! {"state":{"$ne":"closed"}})
        .await
        .unwrap()
        .unwrap();
    assert!(remaining.lease_expires_at <= Utc::now());
    assert!(remaining.generation > template.generation);
    assert!(session::fence(&f.state, &remaining).await.is_err());
    assert!(
        session::expired(&f.state)
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == remaining.id)
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn late_authenticated_receipts_cannot_recreate_data_after_binding_or_owner_removal() {
    let f = fixture(Profile::TwilioSms, false).await;
    super::management::disable(&f.state, &f.b.user_id, &f.b.id)
        .await
        .unwrap();
    // Disabled bindings still retain cleanup evidence.
    assert!(
        session::receipt(&f.state, &f.b, "before-remove", "call_ended")
            .await
            .unwrap()
    );
    super::management::remove(&f.state, &f.b.user_id, &f.b.id)
        .await
        .unwrap();
    assert!(
        session::receipt(&f.state, &f.b, "late-call", "call_ended")
            .await
            .is_err()
    );
    assert!(
        super::sms_delivery::record_correlated(
            &f.state,
            &f.b,
            super::sms_delivery::Receipt {
                event: "late-sms",
                message: "SMlate",
                from: &f.b.phone_number,
                to: "+14155550199",
                status: "delivered",
                parts: Some(1),
                send: None,
            },
        )
        .await
        .is_err()
    );
    assert_eq!(f.state.db.collection::<EventReceipt>(EVENTS).count_documents(doc! {"identity_key":{"$in":[session::identity(&[&f.b.id,"1","late-call"]),session::identity(&[&f.b.id,"sms_receipt","late-sms"])]}}).await.unwrap(), 0);
    f.state.db.drop().await.unwrap();

    let f = fixture(Profile::TwilioSms, false).await;
    let admin = Uuid::new_v4().to_string();
    let (call, sms, deleted) = tokio::join!(
        session::receipt(&f.state, &f.b, "racing-call", "call_ended"),
        super::sms_delivery::record_correlated(
            &f.state,
            &f.b,
            super::sms_delivery::Receipt {
                event: "racing-sms",
                message: "SMrace",
                from: &f.b.phone_number,
                to: "+14155550199",
                status: "delivered",
                parts: Some(1),
                send: None,
            },
        ),
        crate::services::admin_user_service::delete_user_cascade(&f.state.db, &admin, &f.b.user_id),
    );
    deleted.unwrap();
    // Either ordering is valid; the person cascade removes every receipt
    // admitted before binding deletion, and rejects those arriving after it.
    let _ = (call, sms);
    assert_eq!(
        f.state
            .db
            .collection::<EventReceipt>(EVENTS)
            .count_documents(doc! {"user_id":&f.b.user_id})
            .await
            .unwrap(),
        0
    );
    assert!(
        session::receipt(&f.state, &f.b, "after-purge", "call_ended")
            .await
            .is_err()
    );
    assert!(
        super::sms_delivery::record_correlated(
            &f.state,
            &f.b,
            super::sms_delivery::Receipt {
                event: "after-purge",
                message: "SMlate",
                from: &f.b.phone_number,
                to: "+14155550199",
                status: "delivered",
                parts: Some(1),
                send: None,
            },
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.state
            .db
            .collection::<EventReceipt>(EVENTS)
            .count_documents(doc! {"user_id":&f.b.user_id})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}
