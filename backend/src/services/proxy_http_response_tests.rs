use super::*;
use crate::{
    models::{
        assistant_conversation::AssistantConversation,
        audit_log::AuditLog,
        usage_meter::{CredentialClass, UsageMeterRow},
    },
    services::{assistant_model_observer::ModelRequest, assistant_nyxagent as engine},
    test_utils::{connect_transaction_test_database, test_app_state},
};
use axum::{Router, routing::get};
use mongodb::bson::doc;

fn oversized_completed_event() -> Vec<u8> {
    let event = serde_json::json!({
        "type": "response.completed",
        "response": {
            "output": [{"type": "image_generation_call", "status": "completed", "result":
                "x".repeat(crate::services::assistant_engine_policy::MODEL_STREAM_OBSERVER_MAX_BUFFER_BYTES)}],
            "usage": {"input_tokens": 25, "output_tokens": 15, "total_tokens": 40}
        }
    });
    format!("event: response.completed\ndata: {event}\n\n").into_bytes()
}

async fn model_request(state: &AppState) -> (AssistantConversation, ModelRequest) {
    let input: engine::TurnRequest =
        serde_json::from_value(serde_json::json!({"text": "Large image response"})).unwrap();
    let row = Box::pin(engine::begin_turn(
        &state.db,
        "owner",
        &input,
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    let request = ModelRequest::capture(
        &state.db,
        &row.id,
        &row.credential_api_key_id,
        &row.active_turn.as_ref().unwrap().turn_id,
    )
    .await
    .unwrap()
    .unwrap();
    (row, request)
}

async fn assert_diagnostic_fault(state: &AppState, row: &AssistantConversation) {
    let saved = engine::get(&state.db, "owner", &row.id).await.unwrap();
    let turn = saved.active_turn.unwrap();
    let fault = turn.model_stream_fault.unwrap();
    assert_eq!(fault.kind, "unknown");
    assert!(!fault.attributable);
    assert_eq!(turn.model_requests_active, 0);
}

#[tokio::test]
async fn direct_stream_retains_oversized_usage_with_and_without_model_observation() {
    let db = connect_transaction_test_database("direct_oversized_usage").await;
    let state = test_app_state(db.clone());
    engine::ensure_indexes(&db).await.unwrap();
    let event = oversized_completed_event();
    for (observe_model, status) in [
        (true, StatusCode::OK),
        (false, StatusCode::OK),
        (false, StatusCode::BAD_GATEWAY),
    ] {
        let chunks: Vec<_> = event
            .chunks(64 * 1024)
            .map(bytes::Bytes::copy_from_slice)
            .collect();
        let upstream = Router::new().route(
            "/",
            get(move || {
                let chunks = chunks.clone();
                async move {
                    (
                        status,
                        [("content-type", "text/event-stream")],
                        Body::from_stream(futures::stream::iter(
                            chunks.into_iter().map(Ok::<_, std::io::Error>),
                        )),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, upstream).await.unwrap();
        });
        let downstream = state
            .http_client
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap();
        let (row, request) = if observe_model {
            let (row, request) = model_request(&state).await;
            (Some(row), Some(request))
        } else {
            (None, None)
        };
        let service_id = uuid::Uuid::new_v4().to_string();
        let billing_request_id = uuid::Uuid::new_v4().to_string();
        let route = crate::services::billing::BillingRouteContext::new(
            crate::services::billing::BillingIngress::Proxy,
            billing_request_id.clone(),
            "owner".into(),
            "owner".into(),
            None,
            None,
            Some(service_id.clone()),
            Some("test-model".into()),
            crate::services::billing::route_context::NodeIntent::Direct,
            "none".into(),
            CredentialClass::NoAuth,
            BillingMetric::Tokens,
            None,
            false,
        )
        .with_platform_metering(false);
        let metered = state.billing.open(&route).await.unwrap();
        state.billing.mark_forwarded(&metered).await.unwrap();
        let trace = row
            .as_ref()
            .and_then(|row| row.active_turn.as_ref())
            .and_then(|turn| crate::turn_trace::TurnContext::for_turn(&turn.turn_id));
        let response = crate::turn_trace::scope(
            trace,
            relay_response(
                &state,
                HttpResponseContext {
                    service_id: service_id.clone(),
                    path: "v1/responses".into(),
                    nyxagent_admission: false,
                    model_request: request,
                    response_builder: Response::builder()
                        .status(status)
                        .header("content-type", "text/event-stream"),
                    status,
                    is_sse: true,
                    should_stream: true,
                    usage_context: Some(llm_usage_service::UsageAuditContext {
                        db: db.clone(),
                        user_id: "owner".into(),
                        provider_slug: None,
                        service_id: Some(service_id.clone()),
                        model: Some("test".into()),
                        path: "v1/responses".into(),
                        api_key_id: None,
                        api_key_name: None,
                    }),
                    metered,
                    request_body_len: 0,
                    resale_metric: None,
                    downstream_cancellation: CancellationToken::new(),
                    exchange_diagnostics: ProxyExchangeDiagnostics::new(
                        std::time::Instant::now(),
                        0,
                        None,
                        &Method::POST,
                        status,
                        "stream",
                        &http::HeaderMap::new(),
                    ),
                },
                downstream,
            ),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), status);
        let forwarded = axum::body::to_bytes(response.into_body(), event.len())
            .await
            .unwrap();
        assert_eq!(forwarded.as_ref(), event.as_slice());
        // This audit is produced by the relay's own accumulator, not by re-parsing the client copy.
        let usage = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(row) = db.collection::<AuditLog>(crate::models::audit_log::COLLECTION_NAME)
                    .find_one(doc! {"event_type": "llm_usage_reported", "event_data.service_id": &service_id})
                    .await.unwrap() { break row.event_data.unwrap(); }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        if let Some(trace) = trace {
            assert_eq!(usage["turn_id"], trace.turn_id());
        } else {
            assert!(usage.get("turn_id").is_none());
        }
        assert_eq!(usage["prompt_tokens"], 25);
        assert_eq!(usage["completion_tokens"], 15);
        assert_eq!(usage["total_tokens"], 40);
        let settled = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(row) = db
                    .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
                    .find_one(
                        doc! {"billing_request_id": &billing_request_id, "status": "finalized"},
                    )
                    .await
                    .unwrap()
                {
                    break row;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(settled.quantity, Some(40));
        let breakdown = settled.token_breakdown.unwrap();
        assert_eq!(breakdown.prompt_tokens, 25);
        assert_eq!(breakdown.completion_tokens, 15);
        if let Some(row) = row {
            assert_diagnostic_fault(&state, &row).await;
        }
        server.abort();
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn wrapped_stream_leaves_oversized_usage_available_to_independent_collectors() {
    let db = connect_transaction_test_database("wrapped_oversized_usage").await;
    let state = test_app_state(db.clone());
    engine::ensure_indexes(&db).await.unwrap();
    let event = oversized_completed_event();
    let chunks: Vec<_> = event
        .chunks(64 * 1024)
        .map(|chunk| Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(chunk)))
        .collect();
    let (row, request) = model_request(&state).await;
    let response = Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(futures::stream::iter(chunks)))
        .unwrap();
    let response = observe_model_response(Some(request), "v1/responses", response)
        .await
        .unwrap();
    let forwarded = axum::body::to_bytes(response.into_body(), event.len())
        .await
        .unwrap();
    assert_eq!(forwarded.as_ref(), event.as_slice());
    let usage = llm_usage_service::usage_from_body(&forwarded, "v1/responses", true).unwrap();
    assert_eq!(usage.prompt_tokens, 25);
    assert_eq!(usage.completion_tokens, 15);
    assert_eq!(usage.total_tokens, 40);
    assert_diagnostic_fault(&state, &row).await;
    db.drop().await.unwrap();
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetupResponseEnd {
    CallerDropped,
    ClientDisconnected,
    BodyTimeout,
    BodyCap,
    SseClientDisconnected,
}

struct SetupBodyDropped(std::sync::Arc<tokio::sync::Notify>);

impl Drop for SetupBodyDropped {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}

async fn setup_response_lifetime(end: SetupResponseEnd) {
    use crate::billing_integration_tests::{
        FakeLago, billing_service, create_usage_index, rate, wait_for_charged_usage,
    };
    use crate::models::{service_billing::ServiceBilling, user::UserType};
    use crate::services::billing::{
        BillingRouteContext, SettlementGate, route_context::NodeIntent,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::{Notify, Semaphore},
        time::{Duration, timeout},
    };

    let db = connect_transaction_test_database("setup_response_lifetime").await;
    let mut config = crate::test_utils::test_app_config();
    if end == SetupResponseEnd::BodyTimeout {
        config.proxy_stream_idle_timeout_secs = 1;
    }
    let mut state = crate::test_utils::test_app_state_with_config(db.clone(), config);
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection("users")
        .insert_one(crate::test_utils::test_user(&owner, UserType::Person))
        .await
        .unwrap();
    engine::ensure_indexes(&db).await.unwrap();
    create_usage_index(&db).await;
    db.collection("billing_rate_cache")
        .insert_one(rate("platform_tokens", chrono::Utc::now()))
        .await
        .unwrap();
    let settlement = Arc::new(SettlementGate {
        entered: Notify::new(),
        release: Semaphore::new(0),
        fail: false,
        used: AtomicBool::new(false),
    });
    let mut billing = billing_service(&db, Arc::new(FakeLago::default()), 1_000_000);
    billing.settlement_gate = Some(settlement.clone());
    state.billing = Arc::new(billing);
    let row = Box::pin(engine::begin_turn(
        &db,
        &owner,
        &serde_json::from_value::<engine::TurnRequest>(serde_json::json!({"text":"Setup"}))
            .unwrap(),
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"guest_model_setup":true}},
        )
        .await
        .unwrap();
    let request = ModelRequest::capture(
        &db,
        &row.id,
        &row.credential_api_key_id,
        &row.active_turn.as_ref().unwrap().turn_id,
    )
    .await
    .unwrap()
    .unwrap();
    let setup_id = uuid::Uuid::new_v4().to_string();
    db.collection::<bson::Document>(crate::services::assistant_service_execution::COLLECTION)
        .insert_one(doc! {"_id":&setup_id,"kind":"guest_model_setup","conversation_id":&row.id,
        "status":"setting_up","until":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(1))})
        .await
        .unwrap();
    let sse = end == SetupResponseEnd::SseClientDisconnected;
    let body = if end == SetupResponseEnd::BodyCap {
        vec![b'x'; USAGE_CAPTURE_MAX_BYTES + 1]
    } else {
        br#"{"model":"setup-model","usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}"#
            .to_vec()
    };
    let body_len = body.len();
    let release_body = Arc::new(Notify::new());
    let body_dropped = Arc::new(Notify::new());
    let body_finished = Arc::new(AtomicBool::new(false));
    let provider = Router::new().route(
        "/",
        get({
            let (release, dropped, finished) = (
                release_body.clone(),
                body_dropped.clone(),
                body_finished.clone(),
            );
            move || {
                let (release, dropped, finished, body) = (
                    release.clone(),
                    dropped.clone(),
                    finished.clone(),
                    body.clone(),
                );
                async move {
                    let guard = SetupBodyDropped(dropped);
                    let stream = futures::stream::once(async move {
                        let _guard = guard;
                        release.notified().await;
                        finished.store(true, Ordering::SeqCst);
                        Ok::<_, std::io::Error>(bytes::Bytes::from(body))
                    });
                    let mut response = Response::builder().header(
                        "content-type",
                        if sse {
                            "text/event-stream"
                        } else {
                            "application/json"
                        },
                    );
                    if !sse {
                        response = response.header("content-length", body_len);
                    }
                    response.body(Body::from_stream(stream)).unwrap()
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_url = format!("http://{}/", listener.local_addr().unwrap());
    let provider_server = tokio::spawn(async move {
        axum::serve(listener, provider).await.unwrap();
    });
    let request_id = uuid::Uuid::new_v4().to_string();
    let service_id = uuid::Uuid::new_v4().to_string();
    let mut route = BillingRouteContext::new(
        crate::services::billing::BillingIngress::Proxy,
        request_id.clone(),
        owner.clone(),
        owner.clone(),
        Some(row.credential_api_key_id.clone()),
        None,
        Some(service_id.clone()),
        Some("setup-model".into()),
        NodeIntent::Direct,
        "api_key".into(),
        CredentialClass::NyxidManagedMaster,
        BillingMetric::Tokens,
        Some(&ServiceBilling {
            platform_billable: true,
            ..Default::default()
        }),
        false,
    );
    route.request_bytes = 8;
    let metered = state.billing.open(&route).await.unwrap();
    state.billing.mark_forwarded(&metered).await.unwrap();
    let caller_dropped = Arc::new(Notify::new());
    let client_token = Arc::new(std::sync::Mutex::new(None));
    let proxy = Router::new().route(
        "/",
        get({
            let (state, dropped, token) =
                (state.clone(), caller_dropped.clone(), client_token.clone());
            move |incoming: http::Request<Body>| {
                let (state, dropped, token, request, metered, service_id, provider_url, owner) = (
                    state.clone(),
                    dropped.clone(),
                    token.clone(),
                    request.clone(),
                    metered.clone(),
                    service_id.clone(),
                    provider_url.clone(),
                    owner.clone(),
                );
                async move {
                    let _guard = SetupBodyDropped(dropped);
                    let cancellation =
                        crate::downstream_disconnect::request_cancellation(&incoming);
                    *token.lock().unwrap() = Some(cancellation.clone());
                    let downstream = state.http_client.get(provider_url).send().await.unwrap();
                    relay_response(
                        &state,
                        HttpResponseContext {
                            service_id: service_id.clone(),
                            path: "v1/responses".into(),
                            nyxagent_admission: false,
                            model_request: Some(request),
                            response_builder: Response::builder().status(200),
                            status: StatusCode::OK,
                            is_sse: sse,
                            should_stream: sse || end == SetupResponseEnd::BodyCap,
                            usage_context: Some(llm_usage_service::UsageAuditContext {
                                db: state.db.clone(),
                                user_id: owner.clone(),
                                provider_slug: None,
                                service_id: Some(service_id),
                                model: Some("setup-model".into()),
                                path: "v1/responses".into(),
                                api_key_id: None,
                                api_key_name: None,
                            }),
                            metered,
                            request_body_len: 8,
                            resale_metric: None,
                            downstream_cancellation: cancellation,
                            exchange_diagnostics: ProxyExchangeDiagnostics::new(
                                std::time::Instant::now(),
                                0,
                                None,
                                &Method::GET,
                                StatusCode::OK,
                                "buffered",
                                &http::HeaderMap::new(),
                            ),
                        },
                        downstream,
                    )
                    .await
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let real_disconnect = matches!(
        end,
        SetupResponseEnd::ClientDisconnected | SetupResponseEnd::SseClientDisconnected
    );
    let proxy_server = tokio::spawn(async move {
        if real_disconnect {
            axum::serve(
                crate::downstream_disconnect::DisconnectAwareListener::new(listener),
                crate::downstream_disconnect::DisconnectAwareMakeService::new(proxy),
            )
            .await
            .unwrap();
        } else {
            axum::serve(listener, proxy).await.unwrap();
        }
    });
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            if db
                .collection::<bson::Document>(
                    crate::services::assistant_service_execution::COLLECTION,
                )
                .find_one(doc! {"_id":&setup_id,"model_accepted":true})
                .await
                .unwrap()
                .is_some()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("setup must accept before the held body completes");
    assert!(!body_finished.load(Ordering::SeqCst));
    if matches!(
        end,
        SetupResponseEnd::CallerDropped
            | SetupResponseEnd::ClientDisconnected
            | SetupResponseEnd::SseClientDisconnected
    ) {
        engine::request_stop(&db, &row.user_id, &row.id)
            .await
            .unwrap();
        drop(client);
        timeout(Duration::from_secs(10), caller_dropped.notified())
            .await
            .unwrap();
        if real_disconnect {
            let token = client_token.lock().unwrap().clone().unwrap();
            timeout(Duration::from_secs(10), token.cancelled())
                .await
                .unwrap();
            timeout(Duration::from_secs(10), body_dropped.notified())
                .await
                .unwrap();
            assert!(!body_finished.load(Ordering::SeqCst));
        } else {
            release_body.notify_one();
        }
    } else {
        if end == SetupResponseEnd::BodyCap {
            release_body.notify_one();
        }
        let mut response = Vec::new();
        timeout(Duration::from_secs(10), client.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.starts_with(b"HTTP/1.1 500 "));
        if end == SetupResponseEnd::BodyTimeout {
            assert!(!body_finished.load(Ordering::SeqCst));
            timeout(Duration::from_secs(10), body_dropped.notified())
                .await
                .unwrap();
        }
    }
    let filter = doc! {"billing_request_id":&request_id};
    // Intent must survive caller loss/IO failure even while funding is held.
    timeout(Duration::from_secs(10), settlement.entered.notified()).await.expect("accepted setup relay must persist settlement intent after caller cancellation or response IO failure");
    let intent = db
        .collection::<UsageMeterRow>("usage_meter")
        .find_one(filter.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        intent.status,
        crate::models::usage_meter::UsageStatus::Finalized
    );
    let expected_tokens = match end {
        SetupResponseEnd::CallerDropped => 10,
        SetupResponseEnd::BodyCap => llm_platform_usage(None, 8 + body_len as i64).tokens,
        _ => llm_platform_usage(None, 8).tokens,
    };
    assert_eq!(intent.quantity, Some(expected_tokens));
    assert!(!intent.released && !intent.funding.as_ref().unwrap().settled);
    assert_eq!(
        intent.token_breakdown.is_some(),
        end == SetupResponseEnd::CallerDropped
    );
    settlement.release.add_permits(1);
    let charged = wait_for_charged_usage(&db, filter, 1).await;
    assert_eq!(charged[0].billing_owner_id, row.user_id);
    assert_eq!(charged[0].quantity, Some(expected_tokens));
    assert_eq!(
        charged[0].credential_class,
        CredentialClass::NyxidManagedMaster
    );
    proxy_server.abort();
    provider_server.abort();
    let _ = proxy_server.await;
    let _ = provider_server.await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn setup_2187_caller_cancellation_after_acceptance_retains_settlement() {
    setup_response_lifetime(SetupResponseEnd::CallerDropped).await;
}

#[tokio::test]
async fn setup_2187_client_disconnect_after_acceptance_settles_observed_usage() {
    setup_response_lifetime(SetupResponseEnd::ClientDisconnected).await;
}

#[tokio::test]
async fn setup_2187_body_timeout_retains_settlement() {
    setup_response_lifetime(SetupResponseEnd::BodyTimeout).await;
}

#[tokio::test]
async fn setup_2187_body_cap_retains_settlement() {
    setup_response_lifetime(SetupResponseEnd::BodyCap).await;
}

#[tokio::test]
async fn setup_2187_sse_client_disconnect_retains_settlement() {
    setup_response_lifetime(SetupResponseEnd::SseClientDisconnected).await;
}
