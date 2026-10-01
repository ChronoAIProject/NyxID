use axum::{
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::{Document, doc};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

use crate::models::downstream_service::{InferenceWireProtocol, ServiceInference};
use crate::models::user::UserType;
use crate::services::billing::{BillingIngress, route_inventory::BillingRoutePolicy};
use crate::test_utils::{
    connect_transaction_test_database, test_app_state, test_auth_user,
    test_auto_connected_catalog_service, test_user, test_user_endpoint, test_user_service,
};
use crate::{AppState, mw::auth::AuthUser};

struct Fixture {
    state: AppState,
    auth: AuthUser,
    pool_id: String,
    first: MockServer,
    second: MockServer,
}

fn anthropic_text_response() -> Value {
    json!({
        "id": "msg_pool_review", "type": "message", "role": "assistant",
        "model": "review-backup-model", "content": [{"type": "text", "text": "from backup"}],
        "stop_reason": "end_turn", "stop_sequence": null,
        "usage": {"input_tokens": 7, "output_tokens": 3}
    })
}

async fn fixture(label: &str, backup: ResponseTemplate) -> Fixture {
    let db = connect_transaction_test_database(label).await;
    crate::db::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "60")
                .set_body_json(
                    json!({"error":{"message":"review quota exhausted","type":"rate_limit_error"}}),
                ),
        )
        .mount(&first)
        .await;
    Mock::given(method("POST"))
        .respond_with(backup)
        .mount(&second)
        .await;
    let mut members = Vec::new();
    for (index, server, protocol) in [
        (0, &first, InferenceWireProtocol::OpenaiCompletions),
        (1, &second, InferenceWireProtocol::AnthropicMessages),
    ] {
        let mut catalog = test_auto_connected_catalog_service();
        catalog.slug = format!("review-inference-{index}");
        catalog.base_url = format!("https://catalog-{index}.invalid/v1");
        catalog.default_request_headers = Some(vec![
            crate::models::default_request_header::DefaultRequestHeader {
                name: "x-review-trace".into(),
                value: "catalog-trace".into(),
                overridable: true,
                sensitive: false,
            },
            crate::models::default_request_header::DefaultRequestHeader {
                name: "x-review-default".into(),
                value: "catalog-default".into(),
                overridable: false,
                sensitive: false,
            },
        ]);
        catalog.inference = Some(ServiceInference {
            wire_protocol: protocol,
            model_list: false,
            realtime: false,
        });
        db.collection::<crate::models::downstream_service::DownstreamService>(
            "downstream_services",
        )
        .insert_one(&catalog)
        .await
        .unwrap();
        let service_id = Uuid::new_v4().to_string();
        let endpoint_id = Uuid::new_v4().to_string();
        let slug = format!("review-ai-{index}");
        db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &owner,
                &slug,
                &format!("{}/v1", server.uri()),
                None,
                Some(&catalog.id),
            ))
            .await
            .unwrap();
        db.collection::<crate::models::user_service::UserService>("user_services")
            .insert_one(test_user_service(
                &service_id,
                &owner,
                &slug,
                &endpoint_id,
                Some(&catalog.id),
                None,
            ))
            .await
            .unwrap();
        members.push(doc! {
            "user_service_id": service_id, "weight": 1, "enabled": true,
            "priority": index,
            "model": if index == 0 { "review-primary-model" } else { "review-backup-model" },
        });
    }
    let pool_id = Uuid::new_v4().to_string();
    db.collection::<Document>("service_pools").insert_one(doc! {
        "_id": &pool_id, "user_id": &owner, "slug": "review-ai-route", "name": "Review AI route",
        "strategy": "priority", "member_contract": "ai_chat", "config_revision": 0_i64,
        "is_active": true, "members": members,
        "failover": { "max_attempts": 2, "retry_on": ["http_429"] },
        "created_at": mongodb::bson::DateTime::now(), "updated_at": mongodb::bson::DateTime::now(),
    }).await.unwrap();
    Fixture {
        state: test_app_state(db),
        auth: test_auth_user(&owner),
        pool_id,
        first,
        second,
    }
}

#[derive(Clone, Copy)]
enum Entry {
    Slug,
    Gateway,
}

async fn call(
    fixture: &Fixture,
    entry: Entry,
    path: &str,
    body: Value,
) -> crate::errors::AppResult<Response> {
    let (uri, ingress) = match entry {
        Entry::Slug => (
            format!("/api/v1/proxy/s/review-ai-route/{path}?trace=review"),
            BillingIngress::Proxy,
        ),
        Entry::Gateway => (
            format!("/api/v1/llm/gateway/v1/{path}?trace=review"),
            BillingIngress::LlmGateway,
        ),
    };
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("x-review-trace", "pool-ai")
        .header("accept-encoding", "gzip")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    request
        .extensions_mut()
        .insert(BillingRoutePolicy::Metered(ingress));
    match entry {
        Entry::Slug => {
            super::proxy::proxy_request_by_slug(
                State(fixture.state.clone()),
                fixture.auth.clone(),
                crate::telemetry::TelemetryContext::default(),
                Path(("review-ai-route".into(), path.into())),
                request,
            )
            .await
        }
        Entry::Gateway => {
            super::llm_gateway::gateway_request(
                State(fixture.state.clone()),
                fixture.auth.clone(),
                Path(path.into()),
                request,
            )
            .await
        }
    }
}

fn basic_request() -> Value {
    json!({"model":"pool:review-ai-route", "max_tokens": 64,
        "messages":[{"role":"system","content":"Be brief"},{"role":"user","content":"hello"}]})
}

#[tokio::test]
async fn pool_ai_gateway_accepts_llm_scope_without_widening_slug_access() {
    let mut fixture = fixture(
        "pool_ai_llm_scope",
        ResponseTemplate::new(200).set_body_json(anthropic_text_response()),
    )
    .await;
    fixture.auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    fixture.auth.scope = crate::mw::auth::LLM_PROXY_SCOPE.into();
    let denied = call(&fixture, Entry::Slug, "chat/completions", basic_request())
        .await
        .expect_err("llm:proxy alone must not authorize a generic slug route");
    assert_eq!(denied.into_response().status(), StatusCode::FORBIDDEN);
    assert!(fixture.first.received_requests().await.unwrap().is_empty());
    assert!(fixture.second.received_requests().await.unwrap().is_empty());

    let response = call(
        &fixture,
        Entry::Gateway,
        "chat/completions",
        basic_request(),
    )
    .await
    .expect("LLM gateway aliases must retain the gateway scope contract");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "from backup");
    assert_eq!(fixture.first.received_requests().await.unwrap().len(), 1);
    assert_eq!(fixture.second.received_requests().await.unwrap().len(), 1);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_ai_native_stream_error_never_becomes_an_empty_success() {
    let upstream = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"review provider overloaded\"}}\n\n";
    for entry in [Entry::Slug, Entry::Gateway] {
        let fixture = fixture(
            "pool_ai_native_error",
            ResponseTemplate::new(200).set_body_raw(upstream, "text/event-stream"),
        )
        .await;
        let mut request = basic_request();
        request["stream"] = json!(true);
        let response = match call(&fixture, entry, "chat/completions", request).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        let status = response.status();
        if let Ok(body) = to_bytes(response.into_body(), 64 * 1024).await {
            if status.is_success() {
                let text = std::str::from_utf8(&body).unwrap();
                let explicit_error = text
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:"))
                    .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
                    .any(|event| event.get("error").is_some());
                assert!(
                    explicit_error,
                    "a native SSE error must reach the caller as an error"
                );
                assert!(
                    !text.contains("[DONE]"),
                    "a failed native stream cannot claim successful completion"
                );
            } else {
                assert!(status.is_client_error() || status.is_server_error());
            }
        }
        assert_eq!(fixture.first.received_requests().await.unwrap().len(), 1);
        assert_eq!(fixture.second.received_requests().await.unwrap().len(), 1);
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_ai_truncated_native_stream_surfaces_failure_after_partial_output() {
    let upstream = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_partial\",\"model\":\"review-backup-model\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n",
    );
    for entry in [Entry::Slug, Entry::Gateway] {
        let fixture = fixture(
            "pool_ai_truncated_stream",
            ResponseTemplate::new(200).set_body_raw(upstream, "text/event-stream"),
        )
        .await;
        let mut request = basic_request();
        request["stream"] = json!(true);
        let response = match call(&fixture, entry, "chat/completions", request).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.into_body().into_data_stream();
        let mut delivered = Vec::new();
        let mut failed = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => delivered.extend_from_slice(&bytes),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        let delivered = String::from_utf8(delivered).unwrap();
        assert!(
            delivered.contains("partial"),
            "partial output must reach the client before failure"
        );
        assert!(
            failed,
            "EOF without native completion must surface a body error"
        );
        assert!(!delivered.contains("[DONE]"));
        assert_eq!(fixture.first.received_requests().await.unwrap().len(), 1);
        assert_eq!(fixture.second.received_requests().await.unwrap().len(), 1);
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_ai_slug_and_gateway_preserve_destination_and_translate_each_member() {
    for entry in [Entry::Slug, Entry::Gateway] {
        let fixture = fixture(
            "pool_ai_entry",
            ResponseTemplate::new(200).set_body_json(anthropic_text_response()),
        )
        .await;
        let response = call(&fixture, entry, "chat/completions", basic_request())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
        assert_eq!(response.headers()["x-nyxid-pool-member"], "review-ai-1");
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(body["object"], "chat.completion");
        assert_eq!(body["choices"][0]["message"]["content"], "from backup");
        assert_eq!(body["choices"][0]["finish_reason"], "stop");
        assert_eq!(body["usage"]["prompt_tokens"], 7);
        assert_eq!(body["usage"]["completion_tokens"], 3);
        let primary = fixture.first.received_requests().await.unwrap();
        let backup = fixture.second.received_requests().await.unwrap();
        assert_eq!(primary.len(), 1);
        assert_eq!(backup.len(), 1);
        assert_eq!(primary[0].url.path(), "/v1/chat/completions");
        assert_eq!(backup[0].url.path(), "/v1/messages");
        assert!(backup[0].headers.contains_key("anthropic-version"));
        assert_eq!(backup[0].headers["x-review-default"], "catalog-default");
        for request in [&primary[0], &backup[0]] {
            assert_eq!(request.url.query(), Some("trace=review"));
            assert_eq!(request.headers["x-review-trace"], "pool-ai");
        }
        let primary: Value = serde_json::from_slice(&primary[0].body).unwrap();
        let backup: Value = serde_json::from_slice(&backup[0].body).unwrap();
        assert_eq!(primary["model"], "review-primary-model");
        assert_eq!(backup["model"], "review-backup-model");
        assert_eq!(backup["system"], "Be brief");
        assert_eq!(backup["messages"][0]["role"], "user");
        assert_eq!(backup["max_tokens"], 64);
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_ai_tools_preserve_call_result_and_function_schema() {
    for entry in [Entry::Slug, Entry::Gateway] {
        let fixture = fixture("pool_ai_tools", ResponseTemplate::new(200).set_body_json(json!({
            "id":"msg_pool_tool", "type":"message", "role":"assistant", "model":"review-backup-model",
            "content":[{"type":"tool_use", "id":"call_next", "name":"lookup", "input":{"city":"Paris"}}],
            "stop_reason":"tool_use", "usage":{"input_tokens":10,"output_tokens":5}
        }))).await;
        let request = json!({
            "model":"pool:review-ai-route", "max_tokens":64,
            "messages":[
                {"role":"user","content":"Look up Paris"},
                {"role":"assistant", "content":null, "tool_calls":[{"id":"call_previous","type":"function",
                    "function":{"name":"lookup","arguments":"{\"city\":\"Paris\"}"}}]},
                {"role":"tool","tool_call_id":"call_previous","content":"18 C"},
                {"role":"user","content":"Check again"}
            ],
            "tools":[{"type":"function","function":{"name":"lookup","description":"City weather",
                "parameters":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}}}]
        });
        let response = call(&fixture, entry, "chat/completions", request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(body["choices"][0]["finish_reason"], "tool_calls");
        let call = &body["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(call["id"], "call_next");
        assert_eq!(call["function"]["name"], "lookup");
        assert_eq!(
            serde_json::from_str::<Value>(call["function"]["arguments"].as_str().unwrap()).unwrap(),
            json!({"city":"Paris"})
        );
        let backup = fixture.second.received_requests().await.unwrap();
        assert_eq!(backup.len(), 1);
        let body: Value = serde_json::from_slice(&backup[0].body).unwrap();
        assert_eq!(body["tools"][0]["name"], "lookup");
        assert_eq!(
            body["tools"][0]["input_schema"]["required"],
            json!(["city"])
        );
        let blocks: Vec<_> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["content"].as_array().into_iter().flatten())
            .collect();
        assert!(blocks.iter().any(|block| block["type"] == "tool_use"
            && block["id"] == "call_previous"
            && block["input"] == json!({"city":"Paris"})));
        assert!(blocks.iter().any(|block| block["type"] == "tool_result"
            && block["tool_use_id"] == "call_previous"
            && block["content"] == "18 C"));
        assert!(
            !body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["role"] == "tool")
        );
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_ai_unsupported_lossy_request_is_rejected_before_dispatch() {
    for entry in [Entry::Slug, Entry::Gateway] {
        let fixture = fixture(
            "pool_ai_lossy",
            ResponseTemplate::new(200).set_body_json(anthropic_text_response()),
        )
        .await;
        fixture
            .state
            .db
            .collection::<Document>("service_pools")
            .update_one(
                doc! {"_id":&fixture.pool_id},
                doc! {"$set":{"members.0.enabled":false}},
            )
            .await
            .unwrap();
        let mut request = basic_request();
        request["n"] = json!(2);
        let response = match call(&fixture, entry, "chat/completions", request).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        assert!(response.status().is_client_error());
        assert!(fixture.first.received_requests().await.unwrap().is_empty());
        assert!(fixture.second.received_requests().await.unwrap().is_empty());
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_ai_unsupported_operation_is_rejected_before_dispatch() {
    for entry in [Entry::Slug, Entry::Gateway] {
        let fixture = fixture(
            "pool_ai_operation",
            ResponseTemplate::new(200).set_body_json(anthropic_text_response()),
        )
        .await;
        let response = match call(
            &fixture,
            entry,
            "embeddings",
            json!({"model":"pool:review-ai-route", "input":"hello"}),
        )
        .await
        {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        assert!(response.status().is_client_error());
        assert!(fixture.first.received_requests().await.unwrap().is_empty());
        assert!(fixture.second.received_requests().await.unwrap().is_empty());
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_ai_anthropic_stream_has_one_openai_completion_sequence() {
    let events = [
        (
            "message_start",
            json!({"type":"message_start", "message": {
                "id":"msg_stream", "type":"message", "role":"assistant", "model":"review-backup-model",
                "content":[], "stop_reason":null, "usage":{"input_tokens":7,"output_tokens":0}
            }}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start", "index":0,
            "content_block":{"type":"text","text":""}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta", "index":0,
            "delta":{"type":"text_delta","text":"from "}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta", "index":0,
            "delta":{"type":"text_delta","text":"backup"}}),
        ),
        (
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        (
            "message_delta",
            json!({"type":"message_delta", "delta":{"stop_reason":"end_turn","stop_sequence":null},
            "usage":{"output_tokens":3}}),
        ),
        ("message_stop", json!({"type":"message_stop"})),
    ];
    let upstream: String = events
        .iter()
        .map(|(event, data)| format!("event: {event}\ndata: {data}\n\n"))
        .collect();
    for entry in [Entry::Slug, Entry::Gateway] {
        let framed = match entry {
            Entry::Slug => upstream.clone(),
            Entry::Gateway => upstream.replace('\n', "\r\n"),
        };
        let fixture = fixture(
            "pool_ai_stream",
            ResponseTemplate::new(200).set_body_raw(framed, "text/event-stream"),
        )
        .await;
        let mut request = basic_request();
        request["stream"] = json!(true);
        let response = call(&fixture, entry, "chat/completions", request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream")
        );
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        let data: Vec<_> = text
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim))
            .collect();
        assert_eq!(data.iter().filter(|line| **line == "[DONE]").count(), 1);
        let chunks: Vec<Value> = data
            .iter()
            .filter(|line| **line != "[DONE]")
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk["object"] == "chat.completion.chunk")
        );
        let content: String = chunks
            .iter()
            .filter_map(|chunk| chunk["choices"][0]["delta"]["content"].as_str())
            .collect();
        assert_eq!(content, "from backup");
        assert_eq!(
            chunks
                .iter()
                .filter(|chunk| chunk["choices"][0]["finish_reason"] == "stop")
                .count(),
            1
        );
        assert_eq!(fixture.first.received_requests().await.unwrap().len(), 1);
        assert_eq!(fixture.second.received_requests().await.unwrap().len(), 1);
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_ai_codex_catalog_with_custom_connection_slug_uses_local_destination() {
    for entry in [Entry::Slug, Entry::Gateway] {
        for stream in [false, true] {
            let result = json!({"id":"resp_local","status":"completed","model":"review-backup-model", "output":[{"type":"function_call","id":"fc_item","call_id":"call_correlation","name":"lookup","arguments":"{}"}],"usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}});
            let native = format!(
                "data: {}\n\ndata: {}\n\n",
                json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_item","call_id":"call_correlation","name":"lookup"}}),
                json!({"type":"response.completed","response":result})
            );
            let fixture = fixture(
                "pool_codex_destination",
                ResponseTemplate::new(200).set_body_raw(native, "text/event-stream"),
            )
            .await;
            fixture.state.db.collection::<Document>("downstream_services").update_one(doc! {"slug":"review-inference-1"},doc! {"$set":{"slug":"llm-openai-codex","inference.wire_protocol":"openai_responses"}}).await.unwrap();
            if stream {
                fixture
                    .state
                    .db
                    .collection::<Document>("user_services")
                    .update_one(
                        doc! {"slug":"review-ai-1"},
                        doc! {"$set":{"custom_user_agent":"pool-configured-UA"}},
                    )
                    .await
                    .unwrap();
            }
            let body = json!({"model":"pool:review-ai-route","stream":stream,"messages":[{"role":"user","content":"lookup"},{"role":"assistant","tool_calls":[{"id":"call_previous","type":"function","function":{"name":"lookup","arguments":"{}"}}]},{"role":"tool","tool_call_id":"call_previous","content":"result"}]});
            let response = call(&fixture, entry, "chat/completions", body)
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(text.contains("call_correlation"));
            assert!(!text.contains("fc_item"));
            if stream {
                assert_eq!(text.matches("[DONE]").count(), 1);
            } else {
                let body: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(
                    body["choices"][0]["message"]["tool_calls"][0]["id"],
                    "call_correlation"
                );
            }
            let sent = fixture.second.received_requests().await.unwrap();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].url.path(), "/v1/responses");
            assert_eq!(sent[0].headers["originator"], "codex_cli_rs");
            assert_eq!(sent[0].headers["accept"], "text/event-stream");
            if stream {
                assert_eq!(sent[0].headers["user-agent"], "pool-configured-UA");
            } else {
                assert!(
                    sent[0].headers["user-agent"]
                        .to_str()
                        .unwrap()
                        .starts_with("codex_cli_rs/")
                );
            }
            assert_eq!(sent[0].headers["accept-encoding"], "identity");
            let request: Value = serde_json::from_slice(&sent[0].body).unwrap();
            assert_eq!(request["stream"], true);
            assert!(
                request["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["type"] == "function_call_output"
                        && item["call_id"] == "call_previous")
            );
            fixture.state.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn pool_ai_codex_node_transport_preserves_headers_and_local_destination() {
    use crate::services::node_ws_manager::{NodeCapabilitiesMsg, NodeOutboundMessage};
    use base64::Engine;
    for custom_ua in [false, true] {
        let native = format!(
            "data: {}\n\n",
            json!({"type":"response.completed","response":{
                "id":"resp_node","status":"completed","model":"review-backup-model",
                "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"node result"}]}],
                "usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}
            }})
        );
        let fixture = fixture(
            "pool_codex_node",
            ResponseTemplate::new(200).set_body_raw(native, "text/event-stream"),
        )
        .await;
        let node = Uuid::new_v4().to_string();
        let now = bson::DateTime::now();
        fixture.state.db.collection::<Document>("nodes").insert_one(doc! {"_id":&node,"user_id":fixture.auth.user_id.to_string(),"name":"Codex node","status":"online","is_active":true,"auth_token_hash":"test","created_at":now,"updated_at":now,"connection_owner":{"instance_name":&fixture.state.replica_identity.instance_name,"generation_id":&fixture.state.replica_identity.generation_id,"connection_id":"socket","internal_base_url":"http://127.0.0.1","claimed_at":now,"renewed_at":now,"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(5)),"http_cancellation":true}}).await.unwrap();
        let signing_secret = fixture
            .state
            .encryption_keys
            .encrypt("11".repeat(32).as_bytes())
            .await
            .unwrap();
        fixture.state.db.collection::<Document>("nodes").update_one(doc! {"_id":&node},
            doc! {"$set":{"signing_secret_encrypted":bson::Binary { subtype:bson::spec::BinarySubtype::Generic, bytes:signing_secret }}}
        ).await.unwrap();
        fixture.state.db.collection::<Document>("downstream_services").update_one(doc! {"slug":"review-inference-1"},doc! {"$set":{"slug":"llm-openai-codex","inference.wire_protocol":"openai_responses"}}).await.unwrap();
        fixture.state.db.collection::<Document>("user_services").update_one(doc! {"slug":"review-ai-1"},doc! {"$set":{"node_id":&node,"custom_user_agent": if custom_ua {Some("node-configured-UA")} else {None}}}).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let manager = fixture.state.node_ws_manager.clone();
        manager.register_connection_with_id(&node, "socket".into(), tx);
        manager.record_capabilities(
            &node,
            &NodeCapabilitiesMsg {
                http_cancellation: true,
                ..Default::default()
            },
        );
        let local_base = format!("{}/v1", fixture.second.uri());
        let responder = tokio::spawn(async move {
            let Some(NodeOutboundMessage::Text(message)) = rx.recv().await else {
                panic!("node request")
            };
            let request: Value = serde_json::from_str(&message).unwrap();
            assert_eq!(request["type"], "proxy_request");
            assert!(request["signature"].is_string());
            assert_eq!(request["base_url"], local_base);
            assert_eq!(
                request["path"].as_str().unwrap().trim_start_matches('/'),
                "responses"
            );
            let headers = request["headers"].as_object().unwrap();
            assert_eq!(headers["originator"], "codex_cli_rs");
            assert_eq!(headers["accept"], "text/event-stream");
            assert_eq!(headers["accept-encoding"], "identity");
            if custom_ua {
                assert_eq!(headers["user-agent"], "node-configured-UA");
            } else {
                assert!(
                    headers["user-agent"]
                        .as_str()
                        .unwrap()
                        .starts_with("codex_cli_rs/")
                );
            }
            let body = base64::engine::general_purpose::STANDARD
                .decode(request["body"].as_str().unwrap())
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap()["stream"],
                true
            );
            let mut outbound = reqwest::Client::new()
                .post(format!("{local_base}/responses"))
                .body(body);
            for (name, value) in headers {
                outbound = outbound.header(name, value.as_str().unwrap());
            }
            let upstream = outbound.send().await.unwrap();
            let request_id = request["request_id"].as_str().unwrap();
            assert!(
                manager.deliver_stream_start(
                    &node,
                    request_id,
                    upstream.status().as_u16(),
                    upstream
                        .headers()
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_owned()))
                        .collect()
                )
            );
            manager.deliver_stream_chunk(
                &node,
                request_id,
                upstream.bytes().await.unwrap().to_vec(),
            );
            manager.deliver_stream_end(&node, request_id);
        });
        let mut request = basic_request();
        // Codex cannot represent an explicit token limit; use its supported
        // request contract so this test reaches the node transport.
        request.as_object_mut().unwrap().remove("max_tokens");
        let response = call(&fixture, Entry::Slug, "chat/completions", request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "node result");
        responder.await.unwrap();
        let sent = fixture.second.received_requests().await.unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].url.path(), "/v1/responses");
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn service_pool_inspection_draft_contract_pagination_and_alias_scope_are_read_only() {
    use super::service_pools_handler as handler;
    use axum::{Json, extract::Query};
    let fixture = fixture(
        "pool_inspection_draft",
        ResponseTemplate::new(200).set_body_json(anthropic_text_response()),
    )
    .await;
    let members: Vec<crate::models::user_service::UserService> = fixture
        .state
        .db
        .collection("user_services")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let first = members.iter().find(|m| m.slug == "review-ai-0").unwrap();
    let second = members.iter().find(|m| m.slug == "review-ai-1").unwrap();
    let before = fixture.state.encryption_keys.decrypt_stats();
    let query = |contract: &str| {
        serde_json::from_value(json!({"member_contract":contract,"peer_ids":first.id,"method":"POST","path":"chat/completions"})).unwrap()
    };
    let Json(same) = handler::pool_candidates(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Query(query("same_api")),
    )
    .await
    .unwrap();
    assert_eq!(
        same.candidates
            .iter()
            .find(|row| row.user_service_id == second.id)
            .unwrap()
            .reason
            .as_deref(),
        Some("incompatible_protocol")
    );
    let Json(ai) = handler::pool_candidates(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Query(query("ai_chat")),
    )
    .await
    .unwrap();
    let candidate = ai
        .candidates
        .iter()
        .find(|row| row.user_service_id == second.id)
        .unwrap();
    assert!(candidate.eligible);
    assert!(!candidate.requires_compatibility_declaration);
    let Json(invalid) = handler::candidates(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Query(
            serde_json::from_value(
                json!({"member_contract":"ai_chat","method":"GET","path":"embeddings"}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    assert!(invalid.candidates.iter().all(|row| !row.eligible));
    let mut restricted = fixture.auth.clone();
    restricted.allow_all_services = false;
    restricted.allowed_service_ids = vec![second.id.clone()];
    let Json(page) = handler::candidates(
        State(fixture.state.clone()),
        restricted.clone(),
        Query(serde_json::from_value(json!({"member_contract":"ai_chat","limit":1})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(page.candidates.len(), 1);
    assert_eq!(page.candidates[0].user_service_id, second.id);
    assert!(!page.has_more);
    assert!(page.next_cursor.is_none());
    let Json(aliases) = super::llm_gateway::pool_aliases(
        State(fixture.state.clone()),
        restricted.clone(),
        Query(super::llm_gateway::PoolAliasesQuery {
            offset: None,
            limit: None,
        }),
    )
    .await
    .unwrap();
    assert_eq!(aliases.pools.len(), 1);
    assert_eq!(aliases.pools[0].id, "pool:review-ai-route");
    restricted.allowed_service_ids.clear();
    let Json(aliases) = super::llm_gateway::pool_aliases(
        State(fixture.state.clone()),
        restricted,
        Query(super::llm_gateway::PoolAliasesQuery {
            offset: None,
            limit: None,
        }),
    )
    .await
    .unwrap();
    assert!(aliases.pools.is_empty());
    assert_eq!(before, fixture.state.encryption_keys.decrypt_stats());
    assert!(fixture.first.received_requests().await.unwrap().is_empty());
    assert!(fixture.second.received_requests().await.unwrap().is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_stale_member_health_reset_remove_and_disable_remain_available() {
    use super::service_pools_handler as handler;
    use axum::{Json, extract::Query};
    let fixture = fixture("pool_stale_management", ResponseTemplate::new(200)).await;
    let member = fixture
        .state
        .db
        .collection::<crate::models::user_service::UserService>("user_services")
        .find_one(doc! {"slug":"review-ai-0"})
        .await
        .unwrap()
        .unwrap();
    fixture
        .state
        .db
        .collection::<Document>("user_services")
        .delete_one(doc! {"_id":&member.id})
        .await
        .unwrap();
    let Json(health) = handler::health(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Query(serde_json::from_value(json!({})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(health.candidates.len(), 2);
    assert_eq!(
        health
            .candidates
            .iter()
            .find(|row| row.user_service_id == member.id)
            .unwrap()
            .reason
            .as_deref(),
        Some("unavailable")
    );
    let Json(reset) = handler::reset_health(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Json(handler::ResetPoolHealthRequest {
            user_service_id: Some(member.id.clone()),
        }),
    )
    .await
    .unwrap();
    assert!(reset.reset);
    let Json(remaining) = handler::remove_member(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path((fixture.pool_id.clone(), member.id)),
    )
    .await
    .unwrap();
    assert_eq!(remaining.members.len(), 1);
    fixture
        .state
        .db
        .collection::<Document>("downstream_services")
        .delete_many(doc! {})
        .await
        .unwrap();
    let Json(disabled) = handler::update_pool(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Json(
            serde_json::from_value(
                json!({"is_active":false,"expected_revision":remaining.config_revision}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    assert!(!disabled.is_active);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_old_node_is_ineligible_before_attempt_cap_and_inspection_explains_upgrade() {
    use super::service_pools_handler as handler;
    use axum::{Json, extract::Query};
    let fixture = fixture("pool_old_node", ResponseTemplate::new(200)).await;
    let node_id = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    fixture.state.db.collection::<Document>("nodes").insert_one(doc! {"_id":&node_id,"user_id":fixture.auth.user_id.to_string(),"name":"Old node","status":"online","is_active":true,"auth_token_hash":"test","created_at":now,"updated_at":now,"connection_owner":{"instance_name":"test","generation_id":"generation","connection_id":"connection","internal_base_url":"http://127.0.0.1","claimed_at":now,"renewed_at":now,"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(5)),"http_cancellation":false}}).await.unwrap();
    fixture
        .state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"slug":"review-ai-0"},
            doc! {"$set":{"node_id":&node_id}},
        )
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! {"_id":&fixture.pool_id},
            doc! {"$set":{"failover.max_attempts":1}},
        )
        .await
        .unwrap();
    let Json(candidates) = handler::health(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Query(serde_json::from_value(json!({})).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(
        candidates
            .candidates
            .iter()
            .find(|row| row.slug == "review-ai-0")
            .unwrap()
            .reason
            .as_deref(),
        Some("node_upgrade_required")
    );
    let plan = crate::services::service_pool_service::plan_candidates_with_allowlist(
        &fixture.state.db,
        &fixture.state.encryption_keys,
        &fixture.auth.user_id.to_string(),
        None,
        &fixture.auth.user_id.to_string(),
        "review-ai-route",
        &http::Method::POST,
        Some("chat/completions"),
        0,
        Some(&serde_json::to_vec(&basic_request()).unwrap()),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(plan.candidates.len(), 1);
    assert_eq!(plan.candidates[0].service.slug, "review-ai-1");
    assert_eq!(plan.max_attempts, 1);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn service_pool_inspection_honors_draft_declarations_and_saved_health_policy() {
    use super::service_pools_handler as handler;
    use axum::{Json, extract::Query};
    let fixture = fixture("pool_draft_declarations", ResponseTemplate::new(200)).await;
    let services: Vec<crate::models::user_service::UserService> = fixture
        .state
        .db
        .collection("user_services")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let peers = services
        .iter()
        .map(|s| s.id.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let request = |strategy: &str| {
        serde_json::from_value(json!({
            "strategy":strategy, "member_contract":"same_api", "peer_ids":peers,
            "declared_peer_ids":peers, "path":"chat/completions"
        }))
        .unwrap()
    };
    let Json(mismatch) = handler::pool_candidates(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Query(request("priority")),
    )
    .await
    .unwrap();
    assert!(
        mismatch
            .candidates
            .iter()
            .all(|row| row.reason.as_deref() == Some("incompatible_protocol"))
    );
    // Legacy balancing never acquires the priority contract constraints.
    let Json(legacy) = handler::pool_candidates(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Query(request("round_robin")),
    )
    .await
    .unwrap();
    assert!(
        legacy
            .candidates
            .iter()
            .all(|row| row.eligible && !row.requires_compatibility_declaration)
    );
    fixture
        .state
        .db
        .collection::<Document>("downstream_services")
        .update_many(
            doc! {},
            doc! {"$set":{"inference.wire_protocol":"openai_completions"}},
        )
        .await
        .unwrap();
    let Json(confirmed) = handler::pool_candidates(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Query(request("priority")),
    )
    .await
    .unwrap();
    assert_eq!(confirmed.candidates.len(), 2);
    assert!(
        confirmed
            .candidates
            .iter()
            .all(|row| row.eligible && row.reason.is_none())
    );
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! {"_id":&fixture.pool_id},
            doc! {"$set":{"member_contract":"same_api", "members.0.same_api_compatible":false,
            "members.1.same_api_compatible":false}},
        )
        .await
        .unwrap();
    let Json(health) = handler::health(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        Path(fixture.pool_id.clone()),
        Query(request("round_robin")),
    )
    .await
    .unwrap();
    assert!(
        health
            .candidates
            .iter()
            .all(|row| row.reason.as_deref() == Some("compatibility_declaration_required"))
    );
    for declarations in [
        "not-a-uuid".to_owned(),
        vec![services[0].id.as_str(); 51].join(","),
    ] {
        let invalid = handler::candidates(
            State(fixture.state.clone()),
            fixture.auth.clone(),
            Query(serde_json::from_value(json!({"declared_peer_ids":declarations})).unwrap()),
        )
        .await;
        assert!(matches!(
            invalid,
            Err(crate::errors::AppError::BadRequest(_))
        ));
    }
    assert!(fixture.first.received_requests().await.unwrap().is_empty());
    assert!(fixture.second.received_requests().await.unwrap().is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_ai_unknown_openai_provider_preserves_explicit_stream_options_without_injection() {
    for explicit in [false, true] {
        let fixture = fixture("pool_unknown_stream_options", ResponseTemplate::new(500)).await;
        fixture
            .state
            .db
            .collection::<Document>("downstream_services")
            .update_one(
                doc! {"slug":"review-inference-1"},
                doc! {"$set":{"inference.wire_protocol":"openai_completions"}},
            )
            .await
            .unwrap();
        fixture.second.reset().await;
        Mock::given(method("POST"))
            .and(wiremock::matchers::header("accept-encoding", "identity"))
            .respond_with(move |request: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let correct = if explicit {
                    body["stream_options"]
                        == json!({"include_usage":false,"provider_option":"keep"})
                } else {
                    body.get("stream_options").is_none()
                };
                if !correct {
                    return ResponseTemplate::new(400)
                        .set_body_string("unsolicited stream extension");
                }
                ResponseTemplate::new(200).set_body_raw(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
                    "text/event-stream",
                )
            })
            .expect(1)
            .mount(&fixture.second)
            .await;
        let mut body = json!({"model":"pool:review-ai-route","stream":true,"messages":[{"role":"user","content":"hi"}]});
        if explicit {
            body["stream_options"] = json!({"include_usage":false,"provider_option":"keep"});
        }
        let response = call(&fixture, Entry::Slug, "chat/completions", body)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            String::from_utf8(to_bytes(response.into_body(), 4096).await.unwrap().to_vec())
                .unwrap()
                .contains("[DONE]")
        );
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_ai_custom_api_prefixes_match_actual_native_dispatch() {
    for prefix in [
        "",
        "/v1beta/openai",
        "/openai/deployments/name",
        "/backend-api/codex",
        "/openai",
    ] {
        let fixture = fixture(
            "pool_api_prefix",
            ResponseTemplate::new(200).set_body_json(anthropic_text_response()),
        )
        .await;
        fixture
            .state
            .db
            .collection::<Document>("user_endpoints")
            .update_one(
                doc! {"label":"review-ai-1"},
                doc! {"$set":{"url":format!("{}{prefix}",fixture.second.uri())}},
            )
            .await
            .unwrap();
        let response = call(
            &fixture,
            Entry::Slug,
            "chat/completions",
            json!({"messages":[{"role":"user","content":"hi"}]}),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), 4096).await.unwrap();
        let sent = fixture.second.received_requests().await.unwrap();
        assert_eq!(
            sent[0].url.path(),
            if prefix.is_empty() {
                "/v1/messages".into()
            } else {
                format!("{prefix}/messages")
            }
        );
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_ai_planning_serializes_only_the_active_attempt() {
    let fixture = fixture(
        "pool_lazy_prepare",
        ResponseTemplate::new(200).set_body_json(anthropic_text_response()),
    )
    .await;
    let body =
        serde_json::to_vec(&json!({"messages":[{"role":"user","content":"x".repeat(1024*1024)}]}))
            .unwrap();
    let owner = fixture.auth.user_id.to_string();
    let plan = crate::services::service_pool_service::plan_candidates_with_allowlist(
        &fixture.state.db,
        &fixture.state.encryption_keys,
        &owner,
        None,
        &owner,
        "review-ai-route",
        &http::Method::POST,
        Some("chat/completions"),
        body.len(),
        Some(&body),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(plan.candidates.len(), 2);
    let request = plan.chat_request.as_ref().unwrap();
    assert_eq!(
        request
            .serializations
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    let prepared = plan.candidates[0]
        .chat_plan
        .as_ref()
        .unwrap()
        .prepare(request)
        .unwrap();
    assert!(prepared.body.len() > 1024 * 1024);
    assert_eq!(
        request
            .serializations
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        prepared.path,
        plan.candidates[0].chat_plan.as_ref().unwrap().path
    );
    fixture.state.db.drop().await.unwrap();
}
