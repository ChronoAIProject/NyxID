use super::*;
use crate::services::invocation_diagnostic_service::tests::fixture;
use crate::test_utils::set_test_approval_preference;
use axum::body::to_bytes;
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn invocation_diagnostic_direct_universal_and_agent_key_errors_preserve_original_content() {
    for (universal, enabled, agent_key, status) in [
        (false, true, false, 503),
        (true, true, false, 503),
        (false, false, false, 503),
        (true, false, false, 503),
        (false, true, true, 503),
        (true, true, true, 503),
        (false, true, false, 400),
        (false, true, false, 401),
        (false, true, false, 403),
        (true, true, false, 404),
        (true, true, false, 422),
        (true, true, false, 429),
    ] {
        let (mut state, actor, id, server) = fixture().await;
        state.config.invocation_diagnostics_enabled = enabled;
        let downstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/invalid-operation"))
            .respond_with(
                ResponseTemplate::new(status).set_body_string("original tool permission error"),
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
        let mut auth = McpAuthContext::user(
            actor.clone(),
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
        let should_issue = enabled && status >= 500;
        if should_issue {
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
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
        if !should_issue {
            assert_eq!(content.len(), 1, "no diagnostic prompt noise: {body}");
            assert!(body["result"].get("structuredContent").is_none());
            assert_eq!(
                state
                    .db
                    .collection::<mongodb::bson::Document>(
                        crate::models::invocation_diagnostic::COLLECTION_NAME
                    )
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            assert!(server.received_requests().await.unwrap().is_empty());
            continue;
        }
        let evidence: Value =
            serde_json::from_str(content.last().unwrap()["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            body["result"]["structuredContent"]["nyxid_invocation_diagnostic"],
            evidence["nyxid_invocation_diagnostic"]
        );
        let handle = evidence["nyxid_invocation_diagnostic"]["handle"]
            .as_str()
            .unwrap();
        assert!(handle.starts_with("nyx_diag_"));
        assert!(!handle.contains(&id));
        assert_eq!(evidence["nyxid_invocation_diagnostic"]["expires_in"], 60);
        let stored = state
            .db
            .collection::<crate::models::invocation_diagnostic::InvocationDiagnostic>(
                crate::models::invocation_diagnostic::COLLECTION_NAME,
            )
            .find_one(doc! {"handle_hash":crate::crypto::token::hash_token(handle)})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.exact_user_service_id, id);
        assert_eq!(stored.actor_user_id, actor);
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(!body.to_string().contains("original-caller"));
        assert!(!body.to_string().contains("must_not_leak"));
        assert!(!body.to_string().contains(&id));
    }
}

#[tokio::test]
async fn invocation_diagnostic_random_handles_do_not_change_progress_digest() {
    use crate::services::{assistant_continuation, invocation_diagnostic_service::IssuedHandle};
    let content = || vec![json!({"type":"text","text":"same provider failure"})];
    let arguments = json!({"method":"GET","path":"broken"});
    for sse in [false, true] {
        let mut digests = Vec::new();
        for handle in [None, Some("nyx_diag_first"), Some("nyx_diag_second")] {
            let diagnostic = handle.map(|handle| IssuedHandle {
                handle: handle.into(),
                expires_in: 60,
            });
            let response = if sse {
                content_result_with_notifications_and_diagnostic(
                    Some(json!(1)),
                    content(),
                    true,
                    vec![json!({"jsonrpc":"2.0","method":"notifications/test"})],
                    diagnostic.as_ref(),
                )
            } else {
                content_result_with_diagnostic(Some(json!(1)), content(), true, diagnostic.as_ref())
            };
            let digest = response
                .extensions()
                .get::<ToolResultDigest>()
                .unwrap()
                .0
                .clone();
            let call = assistant_continuation::call_digest("same-tool", &arguments, &digest);
            digests.push(
                assistant_continuation::window_digest(
                    "",
                    &crate::models::assistant_conversation::ToolProgress {
                        started: 1,
                        calls: 1,
                        digest: call,
                    },
                )
                .unwrap(),
            );
            let body = to_bytes(response.into_body(), 16384).await.unwrap();
            if let Some(handle) = handle {
                assert!(
                    String::from_utf8_lossy(&body).contains(handle),
                    "handle must still reach the client"
                );
            }
        }
        assert_eq!(digests[0], digests[1]);
        assert_eq!(digests[1], digests[2]);
    }
}
