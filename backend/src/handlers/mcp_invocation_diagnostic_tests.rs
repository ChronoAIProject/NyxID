use super::*;
use crate::services::invocation_diagnostic_service::tests::fixture;
use crate::test_utils::set_test_approval_preference;
use axum::body::to_bytes;
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn conversation_auth(
    actor: &str,
    key_id: &str,
    credential_id: &str,
    raw_key: &str,
) -> McpAuthContext {
    use crate::{
        models::assistant_conversation::AgentRole,
        services::assistant_acknowledgement_service::ChatAuthority,
    };

    let mut auth = McpAuthContext::user(actor.to_owned(), AuthMethod::ApiKey);
    auth.scope = "proxy".into();
    auth.is_api_key = true;
    auth.api_key_id = Some(key_id.to_owned());
    auth.api_key_credential_id = Some(credential_id.to_owned());
    auth.diagnostic_credential = Some(proxy_service::CallerToken::from_verified(raw_key));
    auth.chat = Some(ChatAuthority {
        platform_model_context: None,
        automation_work: None,
        execution_group_id: None,
        execution_unsupported: false,
        execution_voice_task: false,
        execution_task: None,
        execution_permission: None,
        business_system_success: false,
        collaboration_thread: false,
        channel_adapter_origin: None,
        invocation_controlled: true,
        invocation_id: None,
        platform_thread: false,
        platform_origin: None,
        report_to: None,
        with_nyxbot: true,
        org_agent_access: None,
        turn_id: Some("turn-1".into()),
        turn_stopped: false,
        turn_live: true,
        machine_node_ids: Vec::new(),
        saved_login_ids: Vec::new(),
        conversation_id: "conversation-1".into(),
        user_id: actor.to_owned(),
        api_key_id: key_id.to_owned(),
        role: AgentRole::Orchestrator,
        agent_id: "nyxbot".into(),
        agent_name: "NyxBot".into(),
        guest: false,
        confirmation_policy: None,
    });
    auth
}

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

#[tokio::test]
async fn conversation_agent_key_redeems_with_the_same_child_and_raw_key() {
    let (state, actor, id, server) = fixture().await;
    let key_id = uuid::Uuid::new_v4().to_string();
    let credential_id = uuid::Uuid::new_v4().to_string();
    let raw_key = "nyxid_ag_current_conversation_key";
    // The probe endpoint is bound to the exact UserService id, not to any
    // service identifier supplied by the MCP caller.
    crate::services::invocation_diagnostic_service::tests::mount_healthy_agent(
        &server, &id, raw_key,
    )
    .await;
    let issued = crate::services::invocation_diagnostic_service::issue(
        &state.db,
        &state.config,
        crate::services::invocation_diagnostic_service::IssueIdentity::new(
            &actor,
            &AuthMethod::ApiKey,
            Some(&key_id),
            Some(&credential_id),
            None,
        )
        .with_api_key_purpose(crate::models::api_key::ApiKeyPurpose::General),
        &id,
    )
    .await
    .unwrap()
    .expect("conversation diagnostic handle");
    let auth = conversation_auth(&actor, &key_id, &credential_id, raw_key);

    let response = handle_invocation_diagnostic(
        &state,
        &auth,
        &json!({"handle": issued.handle}),
        Some(json!(1)),
    )
    .await;
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    assert_eq!(
        body["result"]["structuredContent"]["nyxid_invocation_diagnostic_result"]["observation"],
        "agent"
    );
    assert_eq!(
        body["result"]["structuredContent"]["nyxid_invocation_diagnostic_result"]["outcome"],
        "completed"
    );
    assert!(!body.to_string().contains(raw_key));
    assert!(!body.to_string().contains(&id));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn stopped_and_missing_conversation_turns_cannot_redeem_or_probe() {
    for stopped in [false, true] {
        let (state, actor, _, server) = fixture().await;
        let key_id = uuid::Uuid::new_v4().to_string();
        let credential_id = uuid::Uuid::new_v4().to_string();
        let mut auth = conversation_auth(
            &actor,
            &key_id,
            &credential_id,
            "nyxid_ag_stale_conversation_key",
        );
        let chat = auth.chat.as_mut().unwrap();
        chat.turn_live = false;
        chat.turn_stopped = stopped;
        if !stopped {
            chat.turn_id = None;
        }

        let response = handle_invocation_diagnostic(
            &state,
            &auth,
            &json!({"handle":"nyx_diag_untrusted"}),
            Some(json!(1)),
        )
        .await;
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(body["result"]["isError"], true);
        assert_eq!(
            body["result"]["content"][0]["text"],
            "Invocation diagnostic not found"
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        state.db.drop().await.unwrap();
    }
}
