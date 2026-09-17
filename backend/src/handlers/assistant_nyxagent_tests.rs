use super::*;
use crate::{
    models::{
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        user::{COLLECTION_NAME as USERS, UserType},
    },
    test_utils::{connect_transaction_test_database, test_app_state, test_auth_user, test_user},
};
use axum::{
    Router,
    http::{HeaderMap, Uri},
    routing::post,
};
use mongodb::bson::doc;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const OWNER: &str = "12345678-1234-4123-8123-123456789012";
const SESSION: &str = "conv_11111111111111111111111111111111";
const RESPONSE: &str = "resp_11111111111111111111111111111111_22222222222222222222222222222222";
struct Capture {
    uri: Uri,
    headers: HeaderMap,
    body: Value,
}
type Captures = Arc<Mutex<Vec<Capture>>>;

async fn setup(
    error: Option<(u16, &'static str)>,
    delay: Duration,
) -> (AppState, Captures, tokio::task::JoinHandle<()>) {
    let db = connect_transaction_test_database("nyxa_http").await;
    engine::ensure_indexes(&db).await.unwrap();
    db.collection(USERS)
        .insert_one(test_user(OWNER, UserType::Person))
        .await
        .unwrap();
    let calls: Captures = Arc::new(Mutex::new(vec![]));
    let sink = calls.clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let upstream = Router::new().route(
        "/v1/responses",
        post(
            move |uri: Uri, headers: HeaderMap, Json(body): Json<Value>| {
                let sink = sink.clone();
                let attempts = attempts.clone();
                async move {
                    sink.lock().await.push(Capture { uri, headers, body });
                    if attempts.fetch_add(1, Ordering::SeqCst) == 0
                        && let Some((status, code)) = error
                    {
                        return (
                            StatusCode::from_u16(status).unwrap(),
                            Json(json!({"error": {
                                "code": code,
                                "message": "SECRET upstream failure detail",
                            }})),
                        )
                            .into_response();
                    }
                    let stream = async_stream::stream! {
                        let delta = json!({
                            "type": "response.output_text.delta",
                            "sequence_number": 0,
                            "delta": "Partial answer",
                        });
                        yield Ok::<_, Infallible>(Event::default().data(delta.to_string()));
                        tokio::time::sleep(delay).await;
                        let completed = json!({
                            "type": "response.completed",
                            "sequence_number": 1,
                            "response": {
                                "id": RESPONSE,
                                "conversation": {"id": SESSION},
                                "status": "completed",
                                "output": [{
                                    "type": "message",
                                    "role": "assistant",
                                    "content": [{
                                        "type": "output_text",
                                        "text": "Partial answer completed",
                                    }],
                                }],
                            },
                        });
                        yield Ok::<_, Infallible>(Event::default().data(completed.to_string()));
                    };
                    Sse::new(stream).into_response()
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    let mut row = crate::models::downstream_service::test_helpers::dummy_service();
    row.id = Uuid::new_v4().to_string();
    row.slug = engine::SERVICE_SLUG.into();
    row.base_url = format!("http://{address}");
    row.service_category = "internal".into();
    row.auth_method = "none".into();
    row.requires_user_credential = false;
    row.forward_access_token = true;
    row.inject_delegation_token = false;
    row.credential_encrypted.clear();
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(row)
        .await
        .unwrap();
    (test_app_state(db), calls, server)
}
fn turn_request(id: Option<&str>) -> Request<Body> {
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/v1/assistant/nyxagent/turns?api_key=CALLER_SECRET&stream=false")
        .header("cookie", "session=CALLER_SECRET")
        .header("authorization", "Bearer CALLER_JWT")
        .body(Body::from(
            json!({"text":"hello","conversation_id":id}).to_string(),
        ))
        .unwrap();
    req.extensions_mut().insert(BillingRoutePolicy::Metered(
        crate::services::billing::BillingIngress::Proxy,
    ));
    req
}
async fn settled(state: &AppState) -> AssistantConversation {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(row) = engine::list(&state.db, OWNER, 1, None)
                .await
                .unwrap()
                .into_iter()
                .next()
                && row.active_turn.is_none()
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("turn settled")
}

#[tokio::test]
async fn browser_disconnect_does_not_cancel_detached_turn_and_headers_are_server_owned() {
    let (state, calls, server) = setup(None, Duration::from_millis(150)).await;
    let response = turns(
        State(state.clone()),
        test_auth_user(OWNER),
        turn_request(None),
    )
    .await
    .unwrap();
    drop(response);
    let row = settled(&state).await;
    assert_eq!(row.nyxagent_session_id.as_deref(), Some(SESSION));
    assert_eq!(row.nyxagent_last_response_id.as_deref(), Some(RESPONSE));
    let messages = engine::messages(&state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].text, "Partial answer completed");
    let calls = calls.lock().await;
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.uri.path(), "/v1/responses");
    assert!(call.uri.query().is_none());
    assert!(call.headers.get("cookie").is_none());
    let key = credentials::load_existing(&state.db, &state.encryption_keys, OWNER)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        call.headers["authorization"],
        format!("Bearer {}", key.raw_key.as_str())
    );
    assert_eq!(call.headers["idempotency-key"], messages[0].turn_id);
    assert_eq!(call.body["input"], "hello");
    assert!(call.body.get("previous_response_id").is_none());
    assert!(
        !serde_json::to_string(&ConversationResponse::from(row))
            .unwrap()
            .contains(key.raw_key.as_str())
    );
    server.abort();
}

#[tokio::test]
async fn stop_persists_partial_reply_clears_binding_and_emits_cancelled() {
    let (state, calls, server) = setup(None, Duration::from_secs(30)).await;
    let response = turns(
        State(state.clone()),
        test_auth_user(OWNER),
        turn_request(None),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while calls.lock().await.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let row = engine::list(&state.db, OWNER, 1, None)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        stop(
            State(state.clone()),
            test_auth_user(OWNER),
            Path(row.id.clone())
        )
        .await
        .unwrap(),
        StatusCode::NO_CONTENT
    );
    let bytes = tokio::time::timeout(
        Duration::from_secs(5),
        axum::body::to_bytes(response.into_body(), 100_000),
    )
    .await
    .unwrap()
    .unwrap();
    let events = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(events.contains("\"status\":\"cancelled\""));
    let row = settled(&state).await;
    assert!(row.nyxagent_session_id.is_none());
    assert_eq!(row.context_reset_reason.as_deref(), Some("turn_failed"));
    let messages = engine::messages(&state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    assert_eq!(messages[1].text, "Partial answer");
    assert_eq!(messages[1].error_code.as_deref(), Some("cancelled"));
    assert_eq!(
        stop(State(state), test_auth_user(OWNER), Path(row.id))
            .await
            .unwrap(),
        StatusCode::NO_CONTENT
    );
    server.abort();
}

#[tokio::test]
async fn lost_session_rebinds_with_recap_and_same_turn_id() {
    let (state, calls, server) = setup(Some((404, "not_found")), Duration::ZERO).await;
    let row = engine::begin_turn(
        &state.db,
        OWNER,
        &engine::TurnRequest {
            conversation_id: None,
            text: "old question".into(),
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let key = credentials::load_for_conversation(&state.db, &state.encryption_keys, OWNER, &row.id)
        .await
        .unwrap()
        .unwrap();
    engine::finish_turn(
        &state.db,
        &row,
        &key.api_key_id,
        &Uuid::new_v4().to_string(),
        &TurnResult {
            text: "old answer".into(),
            session_id: Some(SESSION.into()),
            response_id: Some(RESPONSE.into()),
            error: None,
        },
    )
    .await
    .unwrap();
    let response = turns(
        State(state.clone()),
        test_auth_user(OWNER),
        turn_request(Some(&row.id)),
    )
    .await
    .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 100_000)
        .await
        .unwrap();
    let events = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(events.contains("turn.notice"));
    assert!(!events.contains("SECRET"));
    let saved = engine::get(&state.db, OWNER, &row.id).await.unwrap();
    let messages = engine::messages(&state.db, OWNER, &row.id, 10, None)
        .await
        .unwrap();
    let reset_at = saved
        .context_reset_at
        .expect("rebind records reset timestamp");
    assert!(messages[2].created_at <= reset_at);
    assert!(messages[3].created_at > reset_at);
    let calls = calls.lock().await;
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].body["conversation"], SESSION);
    assert!(calls[1].body.get("conversation").is_none());
    assert_eq!(
        calls[0].headers["idempotency-key"],
        calls[1].headers["idempotency-key"]
    );
    assert!(
        calls[1].body["instructions"]
            .as_str()
            .unwrap()
            .contains("old answer")
    );
    assert_eq!(
        settled(&state).await.nyxagent_session_id.as_deref(),
        Some(SESSION)
    );
    server.abort();
}

#[tokio::test]
async fn invalid_credential_replaces_once_without_exposing_upstream_error() {
    let (state, calls, server) = setup(Some((401, "agent_key_required")), Duration::ZERO).await;
    let response = turns(
        State(state.clone()),
        test_auth_user(OWNER),
        turn_request(None),
    )
    .await
    .unwrap();
    let events = String::from_utf8(
        axum::body::to_bytes(response.into_body(), 100_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(events.contains("turn.notice"));
    assert!(!events.contains("SECRET"));
    let calls = calls.lock().await;
    assert_eq!(calls.len(), 2);
    assert_ne!(
        calls[0].headers["authorization"],
        calls[1].headers["authorization"]
    );
    assert_eq!(
        calls[0].headers["idempotency-key"],
        calls[1].headers["idempotency-key"]
    );
    for call in calls.iter() {
        assert!(
            !events.contains(
                call.headers["authorization"]
                    .to_str()
                    .unwrap()
                    .trim_start_matches("Bearer ")
            )
        );
    }
    assert!(settled(&state).await.nyxagent_session_id.is_some());
    server.abort();
}

#[tokio::test]
async fn uncertain_error_is_generic_persisted_and_never_retried() {
    let (state, calls, server) = setup(Some((409, "outcome_unknown")), Duration::ZERO).await;
    let response = turns(
        State(state.clone()),
        test_auth_user(OWNER),
        turn_request(None),
    )
    .await
    .unwrap();
    let events = String::from_utf8(
        axum::body::to_bytes(response.into_body(), 100_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(events.contains("outcome_unknown"));
    assert!(!events.contains("SECRET"));
    assert_eq!(calls.lock().await.len(), 1);
    let row = settled(&state).await;
    assert!(row.nyxagent_session_id.is_none());
    assert_eq!(row.context_reset_reason.as_deref(), Some("turn_failed"));
    server.abort();
}

#[tokio::test]
async fn all_routes_are_human_only_flag_gated_and_owner_scoped() {
    use crate::services::feature_flag_service::{self, FlagTarget};
    use tower::ServiceExt;
    let db = connect_transaction_test_database("nyxa_gates").await;
    db.collection(USERS)
        .insert_one(test_user(OWNER, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db.clone());
    let token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(OWNER).unwrap(),
        "",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, private) = crate::routes::build_router();
    let app = private.with_state(state.clone());
    feature_flag_service::set_platform_override(
        &db,
        feature_flag_service::NYXAGENT_ENGINE_FLAG_KEY,
        &FlagTarget::Global,
        false,
        "admin",
    )
    .await
    .unwrap();
    let id = "nyxa-11111111111111111111111111111111";
    let routes = [
        ("GET", "conversations".into()),
        ("GET", format!("conversations/{id}")),
        ("PATCH", format!("conversations/{id}")),
        ("DELETE", format!("conversations/{id}")),
        ("POST", format!("conversations/{id}/stop")),
        ("PATCH", format!("conversations/{id}/access-mode")),
        (
            "POST",
            format!("conversations/{id}/acknowledgements/missing"),
        ),
        ("POST", "turns".into()),
        ("GET", "models".into()),
    ];
    use base64::Engine;
    let forbidden_tokens: Vec<_> = ["sa", "delegated", "relay"]
        .into_iter()
        .map(|kind| {
            let payload = json!({kind:true,"scope":"account:read"});
            format!(
                "e30.{}.signature",
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string())
            )
        })
        .collect();
    for (method, path) in routes {
        for (auth, status) in [
            (None, StatusCode::UNAUTHORIZED),
            (Some("nyx_invalid"), StatusCode::FORBIDDEN),
            (Some(token.as_str()), StatusCode::NOT_FOUND),
        ]
        .into_iter()
        .chain(
            forbidden_tokens
                .iter()
                .map(|token| (Some(token.as_str()), StatusCode::FORBIDDEN)),
        ) {
            let mut request = Request::builder()
                .method(method)
                .uri(format!("/api/v1/assistant/nyxagent/{path}"))
                .header("content-type", "application/json");
            if let Some(auth) = auth {
                request = request.header("authorization", format!("Bearer {auth}"));
            }
            let response = app
                .clone()
                .oneshot(
                    request
                        .body(Body::from(if path.ends_with("/access-mode") {
                            "{\"access_mode\":\"full\"}"
                        } else if method == "PATCH" {
                            "{\"title\":\"name\"}"
                        } else if path.contains("/acknowledgements/") {
                            "{\"decision\":\"allow\"}"
                        } else {
                            "{\"text\":\"hello\"}"
                        }))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{method} {path}");
        }
    }
    assert_eq!(
        db.collection::<mongodb::bson::Document>(
            crate::models::assistant_agent_credential::COLLECTION_NAME
        )
        .count_documents(doc! {})
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn stale_fences_are_hidden_in_index_and_history_dtos() {
    let (state, _, server) = setup(None, Duration::ZERO).await;
    let row = engine::begin_turn(
        &state.db,
        OWNER,
        &engine::TurnRequest {
            conversation_id: None,
            text: "interrupted".into(),
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    )
    .await
    .unwrap();
    state
        .db
        .collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {
                "active_turn.started_at": mongodb::bson::DateTime::from_chrono(
                    Utc::now() - chrono::Duration::seconds(engine::ACTIVE_TURN_TTL_SECS),
                ),
            }},
        )
        .await
        .unwrap();
    let Json(index) = list(
        State(state.clone()),
        test_auth_user(OWNER),
        Query(PageQuery::default()),
    )
    .await
    .unwrap();
    assert!(index.conversations[0].active_turn.is_none());
    let Json(page) = history(
        State(state),
        test_auth_user(OWNER),
        Path(row.id),
        Query(HistoryQuery::default()),
    )
    .await
    .unwrap();
    assert!(page.conversation.active_turn.is_none());
    server.abort();
}

#[tokio::test]
async fn settlement_failure_is_bounded_emits_terminal_error_and_releases_permit() {
    let (state, _, server) = setup(None, Duration::ZERO).await;
    let row = engine::begin_turn(
        &state.db,
        OWNER,
        &engine::TurnRequest {
            conversation_id: None,
            text: "question".into(),
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let limiter = Arc::new(crate::mw::rate_limit::DirectChatRateLimiter::new(10, 60, 1));
    let permit = limiter.try_acquire(OWNER).await.unwrap();
    let (sender, receiver) = broadcast::channel(256);
    let response = subscribe_events(receiver);
    let result = TurnResult {
        text: "partial reply".into(),
        session_id: None,
        response_id: None,
        error: None,
    };
    let attempts = AtomicUsize::new(0);
    tokio::time::timeout(
        Duration::from_secs(2),
        complete_turn(
            &row,
            &row.active_turn.as_ref().unwrap().turn_id,
            "message",
            "block",
            &result,
            permit,
            Events { sender, cursor: 0 },
            Duration::from_millis(450),
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Err(AppError::Internal(
                    "injected validation failure".into(),
                )))
            },
        ),
    )
    .await
    .unwrap();
    assert!(attempts.load(Ordering::SeqCst) >= 2);
    assert!(limiter.try_acquire(OWNER).await.is_ok());
    let bytes = axum::body::to_bytes(response.into_body(), 10_000)
        .await
        .unwrap();
    let events = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(events.contains("turn.completed"));
    assert!(events.contains("\"status\":\"failed\""));
    assert!(events.contains("assistant_unavailable"));
    assert!(!events.contains("injected validation failure"));
    assert!(
        engine::get(&state.db, OWNER, &row.id)
            .await
            .unwrap()
            .active_turn
            .is_some()
    );
    server.abort();
}

#[tokio::test]
async fn lagged_browser_subscription_keeps_full_text_and_terminal_event() {
    let (sender, receiver) = broadcast::channel(256);
    let response = subscribe_events(receiver);
    let mut events = Events { sender, cursor: 0 };
    // A slow reader does not poll until more than the entire channel capacity
    // has arrived. The initial recv reports Lagged, not Closed.
    for _ in 0..1024 {
        events.emit("block.delta", json!({"block_id": "block", "text": "x"}));
    }
    let text = "x".repeat(1024);
    events.emit(
        "block.completed",
        json!({
            "block_id": "block",
            "block": {"type": "text", "block_id": "block", "text": text},
        }),
    );
    events.emit(
        "turn.completed",
        json!({
            "turn_id": "turn", "status": "completed", "error": null,
        }),
    );
    drop(events);
    let bytes = axum::body::to_bytes(response.into_body(), 100_000)
        .await
        .unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains(&text));
    assert!(body.contains("turn.completed"));
    assert!(body.contains("\"status\":\"completed\""));
}

#[tokio::test]
async fn model_fallbacks_are_uncached_and_successes_are_cached() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let (state, _, server) = setup(None, Duration::ZERO).await;
    let upstream = MockServer::start().await;
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .update_one(
            doc! {"slug": engine::SERVICE_SLUG},
            doc! {"$set": {"base_url": upstream.uri()}},
        )
        .await
        .unwrap();
    let request = || {
        let mut req = Request::new(Body::empty());
        req.extensions_mut().insert(BillingRoutePolicy::Metered(
            crate::services::billing::BillingIngress::Proxy,
        ));
        req
    };
    let Json(fallback) = models(State(state.clone()), test_auth_user(OWNER), request())
        .await
        .unwrap();
    assert_eq!(fallback[0].id, engine::DEFAULT_MODEL);
    assert!(!MODELS.lock().await.contains_key(state.db.name()));
    engine::begin_turn(
        &state.db,
        OWNER,
        &engine::TurnRequest {
            conversation_id: None,
            text: "models".into(),
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    )
    .await
    .unwrap();
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&upstream)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "nyxagent/chat"}, {"id": "nyxagent/research"}],
        })))
        .with_priority(2)
        .mount(&upstream)
        .await;
    let _ = models(State(state.clone()), test_auth_user(OWNER), request())
        .await
        .unwrap();
    assert!(!MODELS.lock().await.contains_key(state.db.name()));
    for _ in 0..2 {
        let Json(rows) = models(State(state.clone()), test_auth_user(OWNER), request())
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
    }
    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
    server.abort();
}

#[tokio::test]
async fn invalid_and_wrong_owner_turns_do_not_consume_rate_limit() {
    let (mut state, _, server) = setup(None, Duration::ZERO).await;
    state.direct_chat_limiter =
        Arc::new(crate::mw::rate_limit::DirectChatRateLimiter::new(10, 60, 1));
    let row = engine::begin_turn(
        &state.db,
        "other",
        &engine::TurnRequest {
            conversation_id: None,
            text: "private".into(),
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    )
    .await
    .unwrap();
    for _ in 0..11 {
        assert!(matches!(
            turns(
                State(state.clone()),
                test_auth_user(OWNER),
                Request::new(Body::from("{\"text\":\"\",\"unknown\":true}")),
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            turns(
                State(state.clone()),
                test_auth_user(OWNER),
                turn_request(Some(&row.id)),
            )
            .await,
            Err(AppError::NotFound(_))
        ));
    }
    for _ in 0..10 {
        drop(state.direct_chat_limiter.try_acquire(OWNER).await.unwrap());
    }
    assert!(matches!(
        state.direct_chat_limiter.try_acquire(OWNER).await,
        Err(AppError::RateLimited)
    ));
    server.abort();
}

#[tokio::test]
async fn acknowledgements_are_owner_scoped_sanitized_decided_once_and_audited() {
    use crate::services::assistant_authority_tests::fixture;
    let f = fixture("ack_route").await;
    let refusal = acknowledgements::account_gate(&f.state.db, &f.chat)
        .await
        .unwrap()
        .unwrap();
    let ack = refusal["acknowledgement_id"].as_str().unwrap().to_owned();
    let history = history(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Path(f.row.id.clone()),
        Query(HistoryQuery::default()),
    )
    .await
    .unwrap()
    .0;
    let dto = serde_json::to_value(history).unwrap();
    assert_eq!(dto["conversation"]["pending_acknowledgements"], 1);
    assert_eq!(dto["acknowledgements"][0]["id"], ack);
    for forbidden in [
        "arguments",
        "arguments_digest",
        "api_key_id",
        "key_ciphertext",
        "raw_key",
    ] {
        assert!(
            !dto["acknowledgements"][0]
                .as_object()
                .unwrap()
                .contains_key(forbidden)
        );
    }
    let index = list(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Query(PageQuery::default()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(index.conversations[0].pending_acknowledgements, 1);
    let other = Uuid::new_v4().to_string();
    let result = decide_acknowledgement(
        State(f.state.clone()),
        test_auth_user(&other),
        Path((f.row.id.clone(), ack.clone())),
        Json(AcknowledgementDecision {
            decision: Decision::Allow,
        }),
    )
    .await;
    assert!(matches!(result, Err(AppError::NotFound(_))));
    let result = decide_acknowledgement(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Path((f.row.id.clone(), ack.clone())),
        Json(AcknowledgementDecision {
            decision: Decision::Allow,
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(result.status, "allowed");
    assert!(matches!(
        decide_acknowledgement(
            State(f.state.clone()),
            test_auth_user(&f.owner),
            Path((f.row.id.clone(), ack)),
            Json(AcknowledgementDecision {
                decision: Decision::Deny
            }),
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let audit = f
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"event_type": "assistant_acknowledgement_decided"})
        .await
        .unwrap()
        .unwrap();
    let metadata = audit.get_document("event_data").unwrap();
    assert_eq!(metadata.get_str("conversation_id").unwrap(), f.row.id);
    assert_eq!(metadata.get_str("decision").unwrap(), "allow");
    assert_eq!(metadata.len(), 5);
    let response = crate::handlers::api_keys::list_keys(
        State(f.state.clone()),
        test_auth_user(&f.owner),
        Query(Default::default()),
    )
    .await
    .unwrap();
    let dto = serde_json::to_value(response.0).unwrap();
    assert!(dto.to_string().contains(&f.row.id));
    assert_eq!(dto["keys"][0]["allow_all_nodes"], true);
    assert_eq!(dto["keys"][0]["allow_all_services"], false);
    assert!(!dto.to_string().contains("key_ciphertext"));
}

#[tokio::test]
async fn mode_switch_response_and_audit_expose_only_owner_metadata() {
    use crate::models::assistant_conversation::AccessMode::{Ask, Full};
    let f = crate::services::assistant_authority_tests::fixture("mode_route").await;
    f.state
        .db
        .collection::<mongodb::bson::Document>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$set": {"active_turn": mongodb::bson::Bson::Null}},
        )
        .await
        .unwrap();
    for mode in [Full, Ask] {
        let response = change_access_mode(
            State(f.state.clone()),
            test_auth_user(&f.owner),
            Path(f.row.id.clone()),
            Json(AccessModeRequest { access_mode: mode }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(response.access_mode, mode);
        let value = serde_json::to_value(response).unwrap();
        assert!(value.get("credential_api_key_id").is_none());
        assert!(value.get("key_ciphertext").is_none());
    }
    let count = f
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .count_documents(doc! {"event_type": "assistant_access_mode_changed",
        "event_data.conversation_id": &f.row.id})
        .await
        .unwrap();
    assert_eq!(count, 2);
}
