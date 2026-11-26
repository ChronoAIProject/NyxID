//! #1974: connect a real personal specialist before exercising its provider.
//! All services, OAuth clients, Telegram messages and credentials are local fixtures.
use super::*;
use crate::models::oauth_client::{COLLECTION_NAME as CLIENTS, OauthClient};
use crate::services::assistant_team_service as team;
use std::sync::atomic::AtomicBool;

type CreateRefusal = (u16, Value, Option<i64>);

const CLIENT_SECRET: &str = "local-telegram-gateway-client";

#[derive(Clone)]
pub(super) struct CreatorAuth {
    state: AppState,
    client: OauthClient,
    calls: Arc<Mutex<Vec<(String, bool, bool)>>>,
    pub(super) refusal: Arc<Mutex<Option<CreateRefusal>>>,
    deny_delete: Arc<AtomicBool>,
    delete_refusal: Arc<Mutex<Option<(u16, Value)>>>,
    record_absence: Arc<Mutex<Option<(u16, Value)>>>,
    create_cancellation: Arc<Mutex<Option<(u16, Value)>>>,
}

pub(super) async fn authorize(
    auth: CreatorAuth,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let method = request.method().as_str().to_owned();
    let request_path = request.uri().path().to_owned();
    let cancellation = request_path == "/channel-records/cancel-create-capability"
        || request_path.ends_with("/cancel-create");
    let permission = match method.as_str() {
        _ if cancellation => "cma-eg:channels:delete",
        "POST" => "cma-eg:channels:create",
        "PUT" => "cma-eg:channels:update",
        "DELETE" => "cma-eg:channels:delete",
        _ => "cma-eg:channels:read",
    };
    let raw = request.headers()["authorization"]
        .to_str()
        .unwrap()
        .strip_prefix("Bearer ")
        .unwrap();
    let form = serde_json::from_value(json!({
        "token": raw, "client_id": auth.client.id, "client_secret": CLIENT_SECRET,
    }))
    .unwrap();
    let response = crate::handlers::oauth::introspect(
        State(auth.state.clone()),
        axum_extra::extract::Form(form),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let result: Value = serde_json::from_str(&body_text(response).await).unwrap();
    let active = result["active"] == true;
    let allowed = result["permissions"]
        .as_array()
        .is_some_and(|permissions| permissions.iter().any(|p| p == permission));
    auth.calls
        .lock()
        .await
        .push((method.clone(), active, allowed));
    if !active {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !allowed {
        return StatusCode::FORBIDDEN.into_response();
    }
    if cancellation && let Some((status, receipt)) = auth.create_cancellation.lock().await.clone() {
        if method == "GET" {
            return Json(json!({"contract_version":1})).into_response();
        }
        let record = request_path
            .strip_prefix("/channel-records/")
            .unwrap()
            .strip_suffix("/cancel-create")
            .unwrap();
        let receipt = if receipt.is_null() {
            json!({"contract_version":1,"status":"creation_fenced_absent",
            "record_id":record,"owner":{"issuer":auth.state.config.jwt_issuer,"kind":"human","subject":OWNER},
            "provider":{"slug":"nyxbot","binding_id":format!("bnd_{}",record.replace('-',""))}})
        } else {
            receipt
        };
        return (StatusCode::from_u16(status).unwrap(), Json(receipt)).into_response();
    }
    if method == "DELETE"
        && let Some((status, body)) = auth.delete_refusal.lock().await.clone()
    {
        return (StatusCode::from_u16(status).unwrap(), Json(body)).into_response();
    }
    if method == "DELETE" && auth.deny_delete.load(std::sync::atomic::Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let record_lookup = method == "GET" && request.uri().path().starts_with("/channel-records/");
    let response = next.run(request).await;
    if record_lookup
        && response.status() == StatusCode::NOT_FOUND
        && let Some((status, body)) = auth.record_absence.lock().await.clone()
    {
        return (StatusCode::from_u16(status).unwrap(), Json(body)).into_response();
    }
    response
}

/// Independent validator for the exercised Telegram request contract at CMA
/// 70761e9b (definition.rs, definition_sources.rs and core/profile.rs). The
/// fixture also checks the actual management schema before invoking this.
pub(super) fn request_refusal(body: &Value) -> Option<String> {
    let refuse = |path: &str, message: &str| Some(format!("{path}: {message}"));
    for (pointer, min, max) in [
        ("/name", 1, 80),
        ("/profile/metadata/name", 1, 80),
        ("/profile/metadata/description", 0, 2000),
        ("/profile/metadata/owner/subject", 1, 256),
    ] {
        let value = body.pointer(pointer)?.as_str()?;
        if value.len() < min || value.len() > max {
            return refuse(
                pointer,
                if pointer == "/name" {
                    "must contain 1..=80 bytes"
                } else {
                    "must contain at most 80 bytes"
                },
            );
        }
    }
    for pointer in ["/schema_version", "/profile/schema_version"] {
        if body.pointer(pointer) != Some(&json!(1)) {
            return refuse(pointer, "unsupported schema version");
        }
    }
    let issuer = body.pointer("/sources/0/issuer")?.as_str()?;
    let parsed = url::Url::parse(issuer);
    if !parsed.as_ref().is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
            && u.path() == "/"
    }) {
        return refuse("/sources/0/issuer", "must be an HTTPS origin");
    }
    if let Some(username) = body
        .pointer("/sources/0/bot_username")
        .and_then(Value::as_str)
        && (!(5..=32).contains(&username.len())
            || !username
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    {
        return refuse("/sources/0/bot_username", "invalid Telegram bot username");
    }
    for key in ["agent_key", "channel_key"] {
        if let Some(value) = body
            .get("credentials")
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
            && (value.is_empty()
                || value.len() > 16384
                || !value.bytes().all(|b| (0x21..=0x7e).contains(&b)))
        {
            return refuse(&format!("/credentials/{key}"), "invalid bearer credential");
        }
    }
    None
}

struct Fixture {
    state: AppState,
    bot: ChannelBot,
    specialist: crate::models::assistant_agent::AssistantAgent,
    home: AssistantConversation,
    model_calls: Calls,
    gateway_calls: GatewayCalls,
    auth: CreatorAuth,
    gateway: tokio::task::JoinHandle<()>,
    model: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.gateway.abort();
        self.model.abort();
    }
}

async fn fixture(platform: &str) -> Fixture {
    let (mut state, model_calls, model) = setup("telegram_personal_connect").await;
    // The gateway helper selects the same synthetic issuer before minting.
    state.config.jwt_issuer = "https://nyxid.test".into();
    let client: OauthClient = bson::from_document(doc! {
        "_id": Uuid::new_v4().to_string(), "client_name": "Local gateway",
        "client_secret_hash": sha256_hex(CLIENT_SECRET), "redirect_uris": [],
        "allowed_scopes": "openid", "grant_types": "authorization_code",
        "client_type": "confidential", "is_active": true,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    })
    .unwrap();
    state
        .db
        .collection::<OauthClient>(CLIENTS)
        .insert_one(&client)
        .await
        .unwrap();
    let auth = CreatorAuth {
        state: state.clone(),
        client,
        calls: Default::default(),
        refusal: Default::default(),
        deny_delete: Default::default(),
        delete_refusal: Default::default(),
        record_absence: Default::default(),
        create_cancellation: Default::default(),
    };
    let (gateway_calls, gateway) = mock_gateway_with_creator_auth(
        &mut state,
        Arc::new(AtomicBool::new(false)),
        Default::default(),
        Some(auth.clone()),
    )
    .await;
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .update_one(
            doc! {"slug":GATEWAY_SLUG},
            doc! {"$set":{"introspection_client_ids":[&auth.client.id]}},
        )
        .await
        .unwrap();
    let mut bot = bot_doc(platform, &"客户服务🤖".repeat(7));
    if platform == "telegram-new" {
        bot.insert("credential_source", "platform");
    }
    let bot: ChannelBot = bson::from_document(bot).unwrap();
    state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(&bot)
        .await
        .unwrap();
    let (specialist, home) = team::create_specialist(
        &state.db,
        &state.encryption_keys,
        OWNER,
        team::CreateRequest {
            machines: None,
            logins: None,
            name: "telegram-support".into(),
            description: "Answer the local Telegram connection regression".into(),
            display_name: None,
            persona: None,
            voice_instructions: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    full_access(&state, &specialist.id).await;
    Fixture {
        state,
        bot,
        specialist,
        home,
        model_calls,
        gateway_calls,
        auth,
        gateway,
        model,
    }
}

async fn full_access(state: &AppState, agent_id: &str) {
    state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id":agent_id,"user_id":OWNER},
            doc! {"$set":{"access_mode_override":"autopilot"}},
        )
        .await
        .unwrap();
}

async fn provider_message(
    f: &Fixture,
    row: &NyxbotChannel,
    text: &str,
    sender: i64,
    number: i64,
) -> String {
    let route = f
        .state
        .db
        .collection::<crate::models::channel_conversation::ChannelConversation>(
            crate::models::channel_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&row.route_id})
        .await
        .unwrap()
        .unwrap();
    let adapter = crate::services::channel_adapters::resolve_adapter(
        &f.bot.platform,
        &f.state.token_exchange_cache,
    )
    .unwrap();
    let inbound = adapter.parse_inbound(&serde_json::to_vec(&json!({
        "update_id":number,"message":{"message_id":number,"chat":{"id":sender,"type":"private"},
        "from":{"id":sender,"is_bot":false},"text":text},
    })).unwrap()).await.unwrap().remove(0);
    let id = Uuid::new_v4().to_string();
    let source = crate::services::channel_relay_service::store_inbound_message_with_id(
        &f.state.db,
        &f.bot.id,
        &route.id,
        OWNER,
        &f.bot.platform,
        &inbound,
        &row.route_api_key_id,
        &id,
    )
    .await
    .unwrap();
    assert_eq!(source.platform, f.bot.platform);
    let raw = decrypt_agent_key(&f.state, row).await.unwrap().unwrap();
    let mut body = event(text, &sender.to_string(), &id);
    body["event_context"]["channel_id"] = json!(row.gateway_channel_id);
    let activity = &mut body["event_context"]["activity"];
    activity.as_object_mut().unwrap().remove("telegram_input");
    activity["source"]["route_id"] = json!(route.id);
    activity["conversation"]["id"] = json!(sender.to_string());
    let response = respond(&f.state, &raw, &body, &id).await;
    assert_eq!(response.status(), StatusCode::OK);
    body_text(response).await
}

#[test]
fn telegram_personal_specialist_connect_introspection_owner_link_and_turn() {
    crate::test_utils::run_native_journey_test(
        "telegram_personal_specialist_connect_introspection_owner_link_and_turn",
        || async {
            for platform in ["telegram-new", "telegram"] {
                let f = fixture(platform).await;
                let (row, link) = connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
                    .await
                    .unwrap();
                for (_, _, body) in f
                    .gateway_calls
                    .lock()
                    .await
                    .iter()
                    .filter(|(method, _, _)| method == "POST" || method == "PUT")
                {
                    let name = body["name"].as_str().unwrap();
                    assert!(name.len() <= 80);
                    assert_eq!(body["profile"]["metadata"]["name"], name);
                    let mut old = body.clone();
                    old["name"] = json!(
                        format!("NyxBot {}", f.bot.label)
                            .chars()
                            .take(60)
                            .collect::<String>()
                    );
                    assert_eq!(
                        request_refusal(&old).as_deref(),
                        Some("/name: must contain 1..=80 bytes")
                    );
                    old["name"] = json!(name);
                    old["profile"]["metadata"]["name"] = json!(f.bot.label);
                    assert_eq!(
                        request_refusal(&old).as_deref(),
                        Some("/profile/metadata/name: must contain at most 80 bytes")
                    );
                }
                assert_eq!(row.status, "active");
                assert_eq!(row.platform, "telegram");
                assert_eq!(row.transport, "gateway");
                if f.state.encryption_keys.contextual_record_writes() {
                    assert_eq!(
                        row.secret_context_versions.get(channel_key::FIELD),
                        Some(&3)
                    );
                    let raw = decrypt_agent_key(&f.state, &row).await.unwrap().unwrap();
                    let mut wrong_scope = row.clone();
                    wrong_scope.id = Uuid::new_v4().to_string();
                    assert!(decrypt_agent_key(&f.state, &wrong_scope).await.is_err());
                    assert!(!raw.is_empty());
                }
                assert!(row.org_specialist.is_none());
                assert!(row.owner_sender_ids.is_empty());
                assert_eq!(row.agent_id.as_deref(), Some(f.specialist.id.as_str()));
                assert!(
                    link.url
                        .as_ref()
                        .unwrap()
                        .starts_with("https://t.me/helper_bot?start=nyxlink_")
                );
                assert_eq!(
                    f.auth.calls.lock().await.as_slice(),
                    &[
                        ("POST".to_owned(), true, true),
                        ("PUT".to_owned(), true, true),
                        ("GET".to_owned(), true, true),
                        ("PUT".to_owned(), true, true),
                    ],
                    "create, route attachment, capability read and group policy must each pass real registered-client introspection"
                );
                let raw = decrypt_agent_key(&f.state, &row).await.unwrap().unwrap();
                let prepared = put_conversation(
                    State(f.state.clone()),
                    Path((row.binding_id.clone().unwrap(), PARTITION.into())),
                    bearer(&raw),
                )
                .await;
                assert_eq!(prepared.status(), StatusCode::OK);
                // A real adapter parse and authenticated relay metadata precede each
                // local gateway event; no sealed-input test shortcut grants ownership.
                let refused =
                    provider_message(&f, &row, "who are you, before pairing", 77, 1).await;
                assert!(refused.contains("only its owner"), "{refused}");
                assert!(f.model_calls.lock().await.is_empty());
                let linked =
                    provider_message(&f, &row, &format!("/start {}", link.code.as_str()), 77, 2)
                        .await;
                assert!(linked.contains("Linked."), "{linked}");
                assert!(f.model_calls.lock().await.is_empty());
                let row = load_channel(&f.state, OWNER, &row.id).await.unwrap();
                assert_eq!(row.owner_sender_ids, ["77"]);
                assert!(row.link_code_hash.is_none());
                let answer =
                    provider_message(&f, &row, "who are you, local specialist check", 77, 3).await;
                assert!(answer.contains("Here is your answer"), "{answer}");
                assert_eq!(f.model_calls.lock().await.len(), 1);
                let saved = engine::get(&f.state.db, OWNER, &f.home.id).await.unwrap();
                assert_eq!(saved.agent_id.as_deref(), Some(f.specialist.id.as_str()));
                assert!(!saved.guest_turn);
                assert_eq!(
                    saved.reply_channel.as_ref().unwrap().nyxbot_channel_id,
                    row.id
                );
                assert_eq!(list(&f.state, OWNER).await.unwrap().len(), 1);
                assert_eq!(
                    f.gateway_calls
                        .lock()
                        .await
                        .iter()
                        .filter(|c| c.0 == "POST")
                        .count(),
                    1
                );
                disconnect(&f.state, OWNER, &row.id).await.unwrap();
                assert!(list(&f.state, OWNER).await.unwrap().is_empty());
                f.state.db.drop().await.unwrap();
            }
        },
    );
}

#[test]
fn telegram_personal_failed_permission_retries_keep_one_current_row_and_recover() {
    crate::test_utils::run_native_journey_test(
        "telegram_personal_failed_permission_retries_keep_one_current_row_and_recover",
        || async {
            for platform in ["telegram-new", "telegram"] {
                let f = fixture(platform).await;
                f.state
                    .db
                    .collection::<DownstreamService>(SERVICES)
                    .update_one(
                        doc! {"slug":GATEWAY_SLUG},
                        doc! {"$set":{"introspection_client_ids":[]}},
                    )
                    .await
                    .unwrap();
                for count in 1..=3 {
                    let error = connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
                        .await
                        .err()
                        .unwrap();
                    assert!(
                        matches!(error, AppError::ValidationError(ref text) if text.starts_with("gateway_permission_required:"))
                    );
                    let rows = list(&f.state, OWNER).await.unwrap();
                    assert_eq!(rows.len(), 1);
                    assert_eq!(rows[0].status, "failed");
                    assert_eq!(
                        rows[0].last_error.as_deref(),
                        Some("gateway_permission_required")
                    );
                    let health =
                        crate::handlers::channel_bots::health_loaded(&f.state, &f.bot, false, true)
                            .await
                            .unwrap();
                    assert!(
                        health
                            .recommended_action_text
                            .starts_with("gateway_permission_required:")
                    );
                    let evidence = crate::services::channel_bot_health::setup_evidence(
                        &f.state.db,
                        &f.bot,
                        false,
                    )
                    .await
                    .unwrap();
                    assert_eq!(
                        evidence["recommended_action_text"],
                        health.recommended_action_text
                    );
                    assert!(rows[0].agent_key_ciphertext.is_none());
                    assert!(rows[0].link_code_hash.is_none());
                    assert_eq!(
                        f.state
                            .db
                            .collection::<NyxbotChannel>(CHANNELS)
                            .count_documents(doc! {"channel_bot_id":&f.bot.id},)
                            .await
                            .unwrap(),
                        count
                    );
                    assert_eq!(f.state.db.collection::<ApiKey>(crate::models::api_key::COLLECTION_NAME)
                .count_documents(doc! {"user_id":OWNER,"is_active":true,"name":{"$regex":"^NyxBot (channel route|gateway agent) "}})
                .await.unwrap(), 0);
                }
                assert_eq!(
                    f.auth
                        .calls
                        .lock()
                        .await
                        .iter()
                        .filter(|(m, _, _)| m == "POST")
                        .count(),
                    3
                );
                assert!(
                    f.gateway_calls.lock().await.is_empty(),
                    "permission refusals precede remote creation"
                );
                f.state
                    .db
                    .collection::<DownstreamService>(SERVICES)
                    .update_one(
                        doc! {"slug":GATEWAY_SLUG},
                        doc! {"$set":{"introspection_client_ids":[&f.auth.client.id]}},
                    )
                    .await
                    .unwrap();
                let (row, first_link) = connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
                    .await
                    .unwrap();
                let (again, next_link) = connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
                    .await
                    .unwrap();
                assert_eq!(again.id, row.id);
                assert_ne!(first_link.code.as_str(), next_link.code.as_str());
                assert_eq!(list(&f.state, OWNER).await.unwrap()[0].id, row.id);
                assert_eq!(
                    f.gateway_calls
                        .lock()
                        .await
                        .iter()
                        .filter(|c| c.0 == "POST")
                        .count(),
                    1
                );
                disconnect(&f.state, OWNER, &row.id).await.unwrap();
                assert!(list(&f.state, OWNER).await.unwrap().is_empty());
                f.state.db.drop().await.unwrap();
            }
        },
    );
}

#[test]
fn telegram_gateway_400_contract_classification_and_redaction() {
    for (body, expected, absent, field) in [
        (
            json!({"code":"invalid_request","detail":"nyxbot"}),
            "gateway_provider_not_registered",
            false,
            None,
        ),
        (
            json!({"code":"invalid_request","detail":"provider is not registered"}),
            "gateway_provider_not_registered",
            false,
            None,
        ),
        (
            json!({"code":"invalid_request","detail":"/name: must contain 1..=80 bytes"}),
            "gateway_rejected_request",
            true,
            Some("/name"),
        ),
        (
            json!({"code":"invalid_request","detail":"/profile/metadata/name: must contain at most 80 bytes"}),
            "gateway_rejected_request",
            true,
            Some("/profile/metadata/name"),
        ),
        (
            json!({"code":"invalid_request","detail":"/sources/0/issuer: must be an HTTPS origin"}),
            "gateway_rejected_request",
            true,
            Some("/sources/0/issuer"),
        ),
        (
            json!({"code":"invalid_request","detail":"/profile/metadata/provider: PRIVATE secret"}),
            "gateway_rejected_request",
            false,
            Some("/profile/metadata/provider"),
        ),
        (
            json!({"code":"invalid_request","detail":"some provider was PRIVATE"}),
            "gateway_rejected_request",
            false,
            None,
        ),
        (
            json!({"code":"provider_rejected","detail":"The provider refused the binding profile or target. Check the channel configuration before retrying."}),
            "gateway_rejected_request",
            false,
            None,
        ),
        (
            json!({"code":"provider_rejected","detail":"/name: must contain 1..=80 bytes"}),
            "gateway_rejected_request",
            false,
            None,
        ),
        (
            json!({"code":"invalid_request","detail":"/private-secret: /name: PRIVATE"}),
            "gateway_rejected_request",
            false,
            None,
        ),
        (
            json!({"code":"invalid_request","detail":"/name/PRIVATE: secret"}),
            "gateway_rejected_request",
            false,
            None,
        ),
    ] {
        let response = GatewayResponse { status: 400, body };
        assert_eq!(gateway_error_code(&response), expected);
        for platform in ["telegram", "telegram-new"] {
            assert_eq!(gateway_create_refused(platform, &response), absent);
        }
        let diagnostic = crate::services::channel_bot_health::nyxbot::SetupDiagnostic::new(
            "create",
            Some(&response),
        )
        .unwrap();
        assert_eq!(diagnostic.problem_code, response.body["code"].as_str());
        assert_eq!(diagnostic.field, field);
        assert_eq!(diagnostic.http_status, Some(400));
        let encoded = serde_json::to_string(&diagnostic).unwrap();
        assert!(!encoded.contains("PRIVATE"));
        assert_eq!(
            error_code(&AppError::ValidationError(expected.into())),
            expected
        );
        assert!(
            matches!(present_gateway_error(AppError::ValidationError(expected.into())),AppError::ValidationError(message) if message.starts_with(&format!("{expected}:")))
        );
    }
    for (status, code) in [
        (401, "not_authenticated"),
        (403, "forbidden"),
        (429, "provider_busy"),
        (503, "provider_unavailable"),
        (500, "internal"),
    ] {
        let response = GatewayResponse {
            status,
            body: json!({"code":code,"detail":"PRIVATE"}),
        };
        let diagnostic = crate::services::channel_bot_health::nyxbot::SetupDiagnostic::new(
            "create",
            Some(&response),
        )
        .unwrap();
        assert_eq!(diagnostic.problem_code, Some(code));
        assert!(diagnostic.field.is_none());
    }
    let private = GatewayResponse {
        status: 400,
        body: json!({"code":"private_identifier", "detail":"/name: PRIVATE"}),
    };
    let diagnostic =
        crate::services::channel_bot_health::nyxbot::SetupDiagnostic::new("create", Some(&private))
            .unwrap();
    assert!(diagnostic.problem_code.is_none());
    assert!(diagnostic.field.is_none());
    let old = crate::services::channel_bot_health::nyxbot::SetupDiagnostic::from_document(
        &doc!{"operation":"create","outcome":"rejected","http_status":400,"problem_code":"invalid_binding","field":"PRIVATE"}).unwrap();
    assert!(old.problem_code.is_none());
    assert!(old.field.is_none());
    assert!(
        crate::services::channel_bot_health::nyxbot::SetupDiagnostic::new("PRIVATE", None)
            .is_none()
    );
    assert!(
        crate::services::channel_bot_health::nyxbot::SetupDiagnostic::from_document(
            &doc! {"operation":"PRIVATE","outcome":"unknown"}
        )
        .is_none()
    );
}

#[test]
fn telegram_real_connect_configuration_refusal_records_attempt_without_keys() {
    crate::test_utils::run_native_journey_test("telegram_preflight_configuration", || async {
        let f = fixture("telegram").await;
        let services = f.state.db.collection::<bson::Document>(SERVICES);
        let original = services
            .find_one(doc! {"slug":GATEWAY_SLUG})
            .await
            .unwrap()
            .unwrap();
        let keys = f
            .state
            .db
            .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME);
        let count = keys.count_documents(doc! {}).await.unwrap();
        for invalid in ["missing", "inactive", "malformed"] {
            services
                .delete_many(doc! {"slug":GATEWAY_SLUG})
                .await
                .unwrap();
            let mut service = original.clone();
            match invalid {
                "missing" => {}
                "inactive" => {
                    service.insert("is_active", false);
                }
                _ => {
                    service.insert("base_url", "http://not-an-allowed-host.invalid");
                }
            }
            if invalid != "missing" {
                services.insert_one(service).await.unwrap();
            }
            let error = connect_tool(&f.state, OWNER, &f.home.id, &f.bot.id, &f.specialist)
                .await
                .err()
                .unwrap();
            assert!(
                matches!(error,AppError::ValidationError(message) if message.starts_with("gateway_configuration_required:"))
            );
            let row = list(&f.state, OWNER).await.unwrap().remove(0);
            assert_eq!(row.status, "failed");
            assert!(row.route_api_key_id.is_empty()); // no credential was ever allocated
            assert!(row.agent_api_key_id.is_none());
            assert!(row.agent_key_ciphertext.is_none());
            assert!(row.secret_context_versions.is_empty());
            assert_eq!(keys.count_documents(doc! {}).await.unwrap(), count);
            assert!(f.gateway_calls.lock().await.is_empty());
            let health =
                crate::handlers::channel_bots::health_loaded(&f.state, &f.bot, false, true)
                    .await
                    .unwrap();
            let value = serde_json::to_value(health).unwrap();
            assert_eq!(
                value["nyxbot_setup_diagnostic"]["operation"],
                "configuration"
            );
            assert_eq!(value["nyxbot_setup_diagnostic"]["outcome"], "not_sent");
            assert!(
                value["recommended_action_text"]
                    .as_str()
                    .unwrap()
                    .contains("Last connection attempt")
            );
        }
        services
            .delete_many(doc! {"slug":GATEWAY_SLUG})
            .await
            .unwrap();
        services.insert_one(original).await.unwrap();
        let health = crate::handlers::channel_bots::health_loaded(&f.state, &f.bot, false, true)
            .await
            .unwrap();
        assert!(
            health
                .recommended_action_text
                .contains("Last connection attempt")
        );
        // A failed preflight has no route key. Cleanup must never match the
        // empty key value on an unrelated legacy route.
        let failed = list(&f.state, OWNER).await.unwrap().remove(0);
        let route = Uuid::new_v4().to_string();
        let routes = f
            .state
            .db
            .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME);
        routes.insert_one(doc!{"_id":&route,"user_id":OWNER,"channel_bot_id":&f.bot.id,"platform":"telegram",
            "platform_conversation_id":"legacy","platform_conversation_type":"private","agent_api_key_id":"",
            "is_active":true,"default_agent":false,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
        disconnect(&f.state, OWNER, &failed.id).await.unwrap();
        assert_eq!(
            routes
                .find_one(doc! {"_id":&route})
                .await
                .unwrap()
                .unwrap()
                .get_bool("is_active"),
            Ok(true)
        );
        routes.delete_one(doc! {"_id":&route}).await.unwrap();
        // Fixing configuration does not make a historical refusal a fresh probe.
        let (row, _) = connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
            .await
            .unwrap();
        let health = crate::handlers::channel_bots::health_loaded(&f.state, &f.bot, false, true)
            .await
            .unwrap();
        assert!(health.nyxbot_setup_diagnostic.is_none());
        assert!(
            !health
                .recommended_action_text
                .contains("gateway_configuration_required")
        );
        disconnect(&f.state, OWNER, &row.id).await.unwrap();
        f.state.db.drop().await.unwrap();
    });
}

#[test]
fn telegram_terminal_provider_refusal_settles_only_authoritative_original_record_absence() {
    crate::test_utils::run_native_journey_test("telegram_provider_absence", || async {
        for (create_problem, absence_status, absence, cancellation, settled) in [
            (
                json!({"code":"invalid_request","detail":" nyxbot\n"}),
                404,
                json!({"status":404,"code":"channel_not_found","detail":"The channel does not exist."}),
                Some((200, Value::Null)),
                true,
            ),
            (
                json!({"code":"invalid_request","detail":"nyxbot"}),
                404,
                Value::Null,
                None,
                false,
            ),
            (
                json!({"code":"invalid_request","detail":"nyxbot"}),
                404,
                json!({"code":"channel_not_found","status":400}),
                None,
                false,
            ),
            (
                json!({"code":"invalid_request","detail":"nyxbot"}),
                503,
                json!({"status":404,"code":"channel_not_found"}),
                None,
                false,
            ),
            (
                json!({"code":"invalid_request","detail":"unknown provider prose"}),
                404,
                json!({"status":404,"code":"channel_not_found"}),
                None,
                false,
            ),
            (
                json!({"code":"provider_rejected","detail":"The provider refused the binding profile or target. Check the channel configuration before retrying."}),
                404,
                json!({"status":404,"code":"channel_not_found"}),
                None,
                false,
            ),
            // A legacy gateway cannot prove absence after origin/key drift.
            (
                json!({"code":"invalid_request","detail":"nyxbot"}),
                404,
                json!({"status":404,"code":"channel_not_found"}),
                None,
                false,
            ),
            // The durable record-store fence sees a retained record even when
            // the origin-dependent GET returns the ordinary missing problem.
            (
                json!({"code":"invalid_request","detail":"nyxbot"}),
                404,
                json!({"status":404,"code":"channel_not_found"}),
                Some((
                    409,
                    json!({"status":409,"code":"conflict","detail":"record_present"}),
                )),
                false,
            ),
            (
                json!({"code":"invalid_request","detail":"nyxbot"}),
                404,
                json!({"status":404,"code":"channel_not_found"}),
                Some((
                    200,
                    json!({"contract_version":1,"status":"creation_fenced_absent","record_id":"other"}),
                )),
                false,
            ),
        ] {
            let f = fixture("telegram-new").await;
            let provider_refused = unknown_nyxbot_provider(&create_problem);
            let proof_candidate = provider_refused
                && absence_status == 404
                && absence["status"] == 404
                && absence["code"] == "channel_not_found";
            let cancel_sent = proof_candidate && cancellation.is_some();
            *f.auth.create_cancellation.lock().await = cancellation;
            *f.auth.refusal.lock().await = Some((400, create_problem, None));
            *f.auth.record_absence.lock().await = Some((absence_status, absence));
            let error = connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
                .await
                .err()
                .unwrap();
            assert!(matches!(error, AppError::ValidationError(_)));
            let row = list(&f.state, OWNER).await.unwrap().remove(0);
            assert_eq!(row.status, "failed");
            let stored = f
                .state
                .db
                .collection::<crate::models::nyxbot_gateway_release::NyxbotGatewayRelease>(
                    crate::models::nyxbot_gateway_release::COLLECTION_NAME,
                )
                .find_one(doc! {"channel_id":&row.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.create_provider_refused, provider_refused);
            assert_eq!(stored.owner_user_id, OWNER);
            assert_eq!(
                stored.state,
                if settled {
                    crate::models::nyxbot_gateway_release::ReleaseState::Settled
                } else {
                    crate::models::nyxbot_gateway_release::ReleaseState::Due
                }
            );
            assert_eq!(
                stored.resolution.as_deref(),
                if settled { Some("missing") } else { None }
            );
            assert!(
                f.gateway_calls
                    .lock()
                    .await
                    .iter()
                    .any(|(method, id, _)| method == "GET"
                        && Some(id.as_str()) == stored.record_id.as_deref())
            );
            let calls = f.auth.calls.lock().await;
            assert_eq!(
                calls
                    .iter()
                    .filter(|(method, active, allowed)| method == "POST" && *active && *allowed)
                    .count(),
                1 + usize::from(cancel_sent)
            );
            assert_eq!(
                calls
                    .iter()
                    .filter(|(method, active, allowed)| method == "GET" && *active && *allowed)
                    .count(),
                1 + usize::from(proof_candidate)
            );
            assert!(!calls.iter().any(|(method, _, _)| method == "DELETE"));
            drop(calls);
            for key in [&stored.agent_api_key_id, &stored.route_api_key_id] {
                let key = f
                    .state
                    .db
                    .collection::<crate::models::api_key::ApiKey>(
                        crate::models::api_key::COLLECTION_NAME,
                    )
                    .find_one(doc! {"_id":key})
                    .await
                    .unwrap()
                    .unwrap();
                assert!(!key.is_active);
            }
            f.state.db.drop().await.unwrap();
        }
    });
}

#[test]
fn telegram_real_connect_post_reservation_400_retains_and_reconciles_custody() {
    crate::test_utils::run_native_journey_test("telegram_400_custody", || async {
        for (problem, delete_problem, reason) in [
            (
                json!({"code":"invalid_request","detail":"nyxbot"}),
                Some((
                    409,
                    json!({"code":"conflict","detail":"expected version 4, actual Some(4)"}),
                )),
                "gateway_provider_setup_required",
            ),
            (
                json!({"code":"invalid_request","detail":"nyxbot"}),
                Some((400, json!({"code":"invalid_request","detail":"nyxbot"}))),
                "gateway_provider_setup_required",
            ),
            (
                json!({"code":"provider_rejected","detail":"The provider refused the binding profile or target. Check the channel configuration before retrying."}),
                Some((
                    409,
                    json!({"code":"conflict","detail":"expected version 4, actual Some(4)"}),
                )),
                "gateway_setup_custody_required",
            ),
            (
                json!({"code":"invalid_request","detail":"PRIVATE provider prose"}),
                None,
                "release_unconfirmed",
            ),
        ] {
            let f = fixture("telegram").await;
            let held = delete_problem
                .as_ref()
                .is_some_and(|(status, _)| *status == 409);
            *f.auth.refusal.lock().await =
                Some((400, problem.clone(), Some(if held { 4 } else { 1 })));
            *f.auth.delete_refusal.lock().await = delete_problem;
            f.auth
                .deny_delete
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let error = connect_channel(
                State(f.state.clone()),
                crate::test_utils::test_auth_user(OWNER),
                Json(ConnectRequest {
                    bot_id: f.bot.id.clone(),
                    agent_id: Some(f.specialist.id.clone()),
                }),
            )
            .await
            .err()
            .unwrap();
            let code = if problem["detail"] == "nyxbot" {
                "gateway_provider_not_registered"
            } else {
                "gateway_rejected_request"
            };
            assert!(
                matches!(&error,AppError::ValidationError(message) if message.starts_with(code) && !message.contains("PRIVATE"))
            );
            let error_body = body_text(error.into_response()).await;
            assert!(error_body.contains("1008") && error_body.contains(code));
            assert!(!error_body.contains("PRIVATE"));
            let row = list(&f.state, OWNER).await.unwrap().remove(0);
            assert_eq!(row.last_error.as_deref(), Some(code));
            let release = f
                .state
                .db
                .collection::<bson::Document>(
                    crate::models::nyxbot_gateway_release::COLLECTION_NAME,
                )
                .find_one(doc! {"channel_id":&row.id,"native_drain_for":bson::Bson::Null})
                .await
                .unwrap()
                .unwrap();
            assert_ne!(release.get_str("resolution"), Ok("not_created"));
            let calls = f.gateway_calls.lock().await;
            assert!(
                calls
                    .iter()
                    .any(|(m, id, _)| m == "GET" && id == release.get_str("record_id").unwrap()),
                "must recover the exact reserved record before release"
            );
            // Unavailable unbind cannot settle custody. The worker/admin review
            // keeps the exact reservation, owner and original authority.
            assert_eq!(release.get_str("last_error"), Ok(reason));
            if reason == "release_unconfirmed" {
                assert_eq!(release.get_str("state"), Ok("due"));
            } else {
                assert_eq!(release.get_str("state"), Ok("review_required"));
                assert_eq!(release.get_str("review_reason"), Ok(reason));
            }
            drop(calls);
            let health =
                crate::handlers::channel_bots::health_loaded(&f.state, &f.bot, false, true)
                    .await
                    .unwrap();
            let health = serde_json::to_value(health).unwrap();
            assert_eq!(
                health["nyxbot_setup_diagnostic"],
                json!({"operation":"create","outcome":"rejected","http_status":400,"field":null,"problem_code":problem["code"]})
            );
            let evidence =
                crate::services::channel_bot_health::setup_evidence(&f.state.db, &f.bot, false)
                    .await
                    .unwrap();
            assert_eq!(
                evidence["nyxbot_setup_diagnostic"],
                health["nyxbot_setup_diagnostic"]
            );
            assert!(!evidence.to_string().contains("PRIVATE"));
            let audit: Vec<bson::Document> = f
                .state
                .db
                .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .find(doc! {"event_type":"nyxbot_gateway_setup_observed"})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(audit.len(), 1);
            let diagnostic = audit[0]
                .get_document("event_data")
                .unwrap()
                .get_document("diagnostic")
                .unwrap();
            assert_eq!(
                diagnostic.get_str("problem_code").unwrap(),
                problem["code"].as_str().unwrap()
            );
            assert!(!serde_json::to_string(&audit).unwrap().contains("PRIVATE"));
            let before = f
                .gateway_calls
                .lock()
                .await
                .iter()
                .filter(|(m, _, _)| m == "POST")
                .count();
            if code == "gateway_provider_not_registered" {
                *f.auth.refusal.lock().await = None; // operator has re-registered provider
                for _ in 0..2 {
                    assert!(
                        connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
                            .await
                            .is_err()
                    );
                }
                assert_eq!(
                    f.gateway_calls
                        .lock()
                        .await
                        .iter()
                        .filter(|(m, _, _)| m == "POST")
                        .count(),
                    before
                );
            }
            if held {
                // Re-registering the provider cannot cancel CMA's held CREATE
                // intent. Keep that exact record in review; a normal owner retry
                // after the short backoff still connects without replay support.
                *f.auth.refusal.lock().await = None;
                f.state.db.collection::<bson::Document>(CHANNELS).update_one(doc!{"_id":&row.id},doc!{"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(61))}}).await.unwrap();
                let (recovered, _) = connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
                    .await
                    .unwrap();
                let retained = releases::get(&f.state.db, release.get_str("_id").unwrap())
                    .await
                    .unwrap();
                assert_eq!(
                    retained.state,
                    crate::models::nyxbot_gateway_release::ReleaseState::ReviewRequired
                );
                assert_eq!(retained.last_error.as_deref(), Some(reason));
                assert!(
                    releases::claim(&f.state.db, Some(&retained.id))
                        .await
                        .unwrap()
                        .is_none()
                );
                assert!(matches!(
                    releases::observe(&f.state, &retained).await.unwrap(),
                    releases::Observation::Matching { .. }
                ));
                assert!(
                    crate::handlers::channel_bots::health_loaded(&f.state, &f.bot, false, true)
                        .await
                        .unwrap()
                        .nyxbot_setup_diagnostic
                        .is_none()
                );
                if code == "gateway_provider_not_registered" {
                    use crate::handlers::nyxbot_gateway_releases as review;
                    // Model the actual old 400 path: no identity/version was
                    // acknowledged locally, but the exact remote record exists.
                    f.state.db.collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME).update_one(
                        doc!{"_id":&retained.id},doc!{"$set":{"state":"settled","reason":"setup","resolution":"not_created","gateway_channel_id":null,"expected_version":null,"due_at":null},"$unset":{"review_reason":""}}).await.unwrap();
                    f.state
                        .db
                        .collection::<bson::Document>(CHANNELS)
                        .update_one(
                            doc! {"_id":&row.id},
                            doc! {"$set":{"last_error":"gateway_rejected_request"}},
                        )
                        .await
                        .unwrap();
                    assert!(
                        review::inspect(
                            State(f.state.clone()),
                            crate::test_utils::test_auth_user(&Uuid::new_v4().to_string()),
                            Path(retained.id.clone())
                        )
                        .await
                        .is_err()
                    );
                    crate::services::role_service::seed_system_roles(&f.state.db)
                        .await
                        .unwrap();
                    let roles = crate::services::role_service::get_platform_role_ids(&f.state.db)
                        .await
                        .unwrap();
                    f.state
                        .db
                        .collection::<bson::Document>(USERS)
                        .update_one(doc! {"_id":OWNER}, doc! {"$set":{"role_ids":[roles.admin]}})
                        .await
                        .unwrap();
                    let inspected = review::inspect(
                        State(f.state.clone()),
                        crate::test_utils::test_auth_user(OWNER),
                        Path(retained.id.clone()),
                    )
                    .await
                    .unwrap()
                    .0;
                    let inspected = serde_json::to_value(inspected).unwrap();
                    assert_eq!(inspected["captured"]["state"], "settled");
                    assert_eq!(inspected["observation"]["status"], "matching");
                    let revision = inspected["captured"]["revision"].as_i64().unwrap();
                    assert!(
                        releases::resolve(
                            &f.state,
                            OWNER,
                            &retained.id,
                            revision,
                            releases::Observation::Missing,
                            releases::Resolution::RetryOriginal,
                            false
                        )
                        .await
                        .is_err()
                    );
                    let body=serde_json::from_value(json!({"revision":revision,"observation":inspected["observation"],"action":"retry_original"})).unwrap();
                    assert_eq!(
                        review::resolve(
                            State(f.state.clone()),
                            crate::test_utils::test_auth_user(OWNER),
                            Path(retained.id.clone()),
                            Json(body)
                        )
                        .await
                        .unwrap(),
                        StatusCode::NO_CONTENT
                    );
                    assert!(
                        releases::get(&f.state.db, &retained.id)
                            .await
                            .unwrap()
                            .due_at
                            .is_some()
                    );
                    let claimed = releases::claim(&f.state.db, Some(&retained.id))
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(!releases::execute(&f.state, claimed).await.unwrap());
                    let repaired = releases::get(&f.state.db, &retained.id).await.unwrap();
                    assert_eq!(
                        repaired.state,
                        crate::models::nyxbot_gateway_release::ReleaseState::ReviewRequired
                    );
                    assert_eq!(
                        repaired.last_error.as_deref(),
                        Some("gateway_setup_custody_required")
                    );
                    assert_ne!(repaired.resolution.as_deref(), Some("not_created"));
                }
                *f.auth.delete_refusal.lock().await = None;
                f.auth
                    .deny_delete
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                disconnect(&f.state, OWNER, &recovered.id).await.unwrap();
                f.state.db.drop().await.unwrap();
                continue;
            }
            *f.auth.delete_refusal.lock().await = None;
            f.auth
                .deny_delete
                .store(false, std::sync::atomic::Ordering::SeqCst);
            let release_id = release.get_str("_id").unwrap();
            if reason != "release_unconfirmed" {
                crate::services::role_service::seed_system_roles(&f.state.db)
                    .await
                    .unwrap();
                let roles = crate::services::role_service::get_platform_role_ids(&f.state.db)
                    .await
                    .unwrap();
                f.state
                    .db
                    .collection::<bson::Document>(USERS)
                    .update_one(doc! {"_id":OWNER}, doc! {"$set":{"role_ids":[roles.admin]}})
                    .await
                    .unwrap();
                let retained = releases::get(&f.state.db, release_id).await.unwrap();
                let observed = releases::observe(&f.state, &retained).await.unwrap();
                releases::resolve(
                    &f.state,
                    OWNER,
                    release_id,
                    retained.revision,
                    observed,
                    releases::Resolution::RetryOriginal,
                    false,
                )
                .await
                .unwrap();
            }
            f.state
                .db
                .collection::<bson::Document>(
                    crate::models::nyxbot_gateway_release::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":release_id},
                    doc! {"$set":{"next_attempt_at":bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
            let claimed = releases::claim(&f.state.db, Some(release_id))
                .await
                .unwrap()
                .unwrap();
            assert!(releases::execute(&f.state, claimed).await.unwrap());
            let settled = releases::get(&f.state.db, release_id).await.unwrap();
            assert_eq!(
                settled.state,
                crate::models::nyxbot_gateway_release::ReleaseState::Settled
            );
            assert_ne!(settled.resolution.as_deref(), Some("not_created"));
            assert!(
                f.gateway_calls
                    .lock()
                    .await
                    .iter()
                    .any(|(m, _, _)| m == "DELETE")
            );
            *f.auth.refusal.lock().await = None;
            f.state.db.collection::<bson::Document>(CHANNELS).update_one(doc!{"_id":&row.id},doc!{"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(61))}}).await.unwrap();
            let (recovered, _) = connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
                .await
                .unwrap();
            assert_eq!(list(&f.state, OWNER).await.unwrap().len(), 1);
            assert_eq!(list(&f.state, OWNER).await.unwrap()[0].id, recovered.id);
            assert!(
                crate::handlers::channel_bots::health_loaded(&f.state, &f.bot, false, true)
                    .await
                    .unwrap()
                    .nyxbot_setup_diagnostic
                    .is_none()
            );
            disconnect(&f.state, OWNER, &recovered.id).await.unwrap();
            f.state.db.drop().await.unwrap();
        }
    });
}

#[test]
fn telegram_historical_generic_400_reconnects_without_replay_advertisement() {
    crate::test_utils::run_native_journey_test("telegram_legacy_400", || async {
        let f = fixture("telegram").await;
        // CMA's real pre-effect refusal for the formerly overlong Unicode name.
        *f.auth.refusal.lock().await = Some((
            400,
            json!({"code":"invalid_request","detail":"/name: must contain 1..=80 bytes"}),
            None,
        ));
        assert!(
            connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
                .await
                .is_err()
        );
        let old = list(&f.state, OWNER).await.unwrap().remove(0);
        let release = f
            .state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_gateway_release::COLLECTION_NAME)
            .find_one(doc! {"channel_id":&old.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(release.get_str("resolution"), Ok("not_created"));
        // Model a row written by the old release, before bounded diagnostics.
        f.state
            .db
            .collection::<bson::Document>(CHANNELS)
            .update_one(
                doc! {"_id":&old.id},
                doc! {"$unset":{"setup_diagnostic":"","bot_ownership_version":""}},
            )
            .await
            .unwrap();
        *f.auth.refusal.lock().await = None;
        let (recovered, _) = connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
            .await
            .unwrap();
        assert_eq!(recovered.status, "active");
        assert_eq!(
            f.gateway_calls
                .lock()
                .await
                .iter()
                .filter(|(m, _, _)| m == "POST")
                .count(),
            2
        );
        // Public mock capabilities deliberately omit response_recovery_providers.
        assert!(
            !f.gateway_calls
                .lock()
                .await
                .iter()
                .any(|(_, _, body)| body.get("response_recovery_providers").is_some())
        );
        disconnect(&f.state, OWNER, &recovered.id).await.unwrap();
        f.state.db.drop().await.unwrap();
    });
}

#[test]
fn telegram_connect_transaction_uses_live_generation_after_real_aba_transfer() {
    crate::test_utils::run_native_journey_test("telegram_connect_transfer", || async {
        use crate::services::ownership_transfer_service as transfer;
        let f = fixture("telegram").await;
        crate::services::role_service::seed_system_roles(&f.state.db)
            .await
            .unwrap();
        let other = Uuid::new_v4().to_string();
        f.state
            .db
            .collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&other, UserType::Person))
            .await
            .unwrap();
        let gate = Arc::new(ConnectInsertGate {
            bot_id: f.bot.id.clone(),
            arrived: Arc::new(tokio::sync::Barrier::new(2)),
            release: Default::default(),
        });
        let state = f.state.clone();
        let agent = f.specialist.clone();
        let bot_id = f.bot.id.clone();
        let held = gate.clone();
        let connect = tokio::spawn(async move {
            CONNECT_INSERT_GATE
                .scope(
                    held,
                    Box::pin(super::connect(&state, OWNER, None, &bot_id, &agent)),
                )
                .await
        });
        gate.arrived.wait().await;
        for (actor, destination) in [(OWNER, other.as_str()), (other.as_str(), OWNER)] {
            let preview = transfer::preview(
                &f.state.db,
                actor,
                transfer::ResourceKind::ChannelBot,
                &f.bot.id,
                destination,
                5,
            )
            .await
            .unwrap();
            transfer::transfer_with_keys(
                &f.state.db,
                None,
                transfer::TransferCommand {
                    actor,
                    api_key_id: None,
                    first_party_human: true,
                    kind: transfer::ResourceKind::ChannelBot,
                    resource_id: &f.bot.id,
                    destination,
                    request_id: &Uuid::new_v4().to_string(),
                    expected_version: &preview.version,
                    capacity: 5,
                },
            )
            .await
            .unwrap();
        }
        gate.release.notify_one();
        let (row, _) = connect.await.unwrap().unwrap();
        let raw = f
            .state
            .db
            .collection::<bson::Document>(CHANNELS)
            .find_one(doc! {"_id":&row.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(raw.get_i64("bot_ownership_version"), Ok(2));
        let bot = crate::services::channel_bot_service::get_bot(&f.state.db, &f.bot.id)
            .await
            .unwrap();
        assert_eq!(bot.ownership_version, 2);
        assert_eq!(
            crate::handlers::channel_bots::health_loaded(&f.state, &bot, false, true)
                .await
                .unwrap()
                .nyxbot_status
                .as_deref(),
            Some("active")
        );
        disconnect(&f.state, OWNER, &row.id).await.unwrap();
        f.state.db.drop().await.unwrap();
    });
}

#[test]
fn telegram_connect_transport_failure_keeps_http_status_unknown() {
    crate::test_utils::run_native_journey_test("telegram_transport_unknown", || async {
        let f = fixture("telegram").await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        f.state
            .db
            .collection::<bson::Document>(SERVICES)
            .update_one(doc! {"slug":GATEWAY_SLUG}, doc! {"$set":{"base_url":base}})
            .await
            .unwrap();
        let error = connect(&f.state, OWNER, None, &f.bot.id, &f.specialist)
            .await
            .err()
            .unwrap();
        assert!(
            matches!(error,AppError::ValidationError(message) if message.starts_with("gateway_unavailable:"))
        );
        let health = crate::handlers::channel_bots::health_loaded(&f.state, &f.bot, false, true)
            .await
            .unwrap();
        let diagnostic = serde_json::to_value(health.nyxbot_setup_diagnostic).unwrap();
        assert_eq!(
            diagnostic,
            json!({"operation":"create","outcome":"unknown","http_status":null,"field":null,"problem_code":null})
        );
        f.state.db.drop().await.unwrap();
    });
}
