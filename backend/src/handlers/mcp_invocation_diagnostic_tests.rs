use super::*;
use crate::services::invocation_diagnostic_service::tests::{fixture, mount_healthy};
use crate::test_utils::set_test_approval_preference;
use axum::body::to_bytes;
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn invocation_diagnostic_direct_universal_and_agent_key_errors_preserve_original_content() {
    for (universal, agent_key) in [(false, false), (true, false), (false, true)] {
        let (state, actor, id, server) = fixture().await;
        let downstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/invalid-operation"))
            .respond_with(
                ResponseTemplate::new(403).set_body_string("original tool permission error"),
            )
            .expect(1)
            .mount(&downstream)
            .await;
        state
            .db
            .collection::<mongodb::bson::Document>("user_endpoints")
            .update_many(doc! {}, doc! {"$set":{"url":downstream.uri()}})
            .await
            .unwrap();
        set_test_approval_preference(&state.db, &actor, false).await;
        if !agent_key {
            mount_healthy(&server, &id).await;
        }
        let mut auth = McpAuthContext::user(
            actor,
            if agent_key {
                AuthMethod::ApiKey
            } else {
                AuthMethod::AccessToken
            },
        );
        auth.scope = "proxy mcp account:read".into();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer original-caller".parse().unwrap());
        auth.caller_token = proxy_service::caller_bearer_token_for_downstream(&headers, false);
        let args = json!({"method":"GET","path":"invalid-operation"});
        let params = if universal {
            json!({"name":"nyx__call_tool","arguments":{"tool_name":"same-slug__request","arguments_json":args.to_string()}})
        } else {
            json!({"name":"same-slug__request","arguments":args})
        };
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "tools/call".into(),
            params: Some(params),
        };
        let permit =
            crate::services::billing::route_inventory::enforce_billing_egress_classification(
                Some(
                    crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                        crate::services::billing::BillingIngress::Mcp,
                    ),
                ),
                crate::services::billing::BillingIngress::Mcp,
            )
            .unwrap();
        let response = Box::pin(handle_tools_call(
            &state, &auth, None, &request, false, permit,
        ))
        .await;
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
        assert_eq!(body["result"]["isError"], true, "{body}");
        let content = body["result"]["content"].as_array().unwrap();
        assert!(
            content[0]["text"]
                .as_str()
                .unwrap()
                .contains("original tool permission error"),
            "{body}"
        );
        let evidence: Value =
            serde_json::from_str(content.last().unwrap()["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            body["result"]["structuredContent"]["nyxid_service_diagnostic"],
            evidence["nyxid_service_diagnostic"]
        );
        assert_eq!(
            evidence["nyxid_service_diagnostic"]["observation"],
            if agent_key { "unknown" } else { "caller" },
            "{body}"
        );
        if agent_key {
            assert_eq!(
                evidence["nyxid_service_diagnostic"]["outcome"],
                "non_delegable"
            );
            assert!(server.received_requests().await.unwrap().is_empty());
        } else {
            assert_eq!(
                evidence["nyxid_service_diagnostic"]["assessment"],
                "operation_specific_likely"
            );
        }
        assert!(!body.to_string().contains("original-caller"));
        assert!(!body.to_string().contains("must_not_leak"));
    }
}
