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
        let response = relay_response(
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
