//! Policy isolation for the request-local assistant model transport exemption.
use crate::models::service_approval_config::{ApprovalEffect, ApprovalMode};
use crate::services::assistant_model_observer::*;
use crate::services::{approval_service as approvals, assistant_authority_tests::direct_fixture};
use serde_json::json;

#[tokio::test]
async fn model_approval_personal_config_variants_and_non_assistant_callers() {
    let mut f = Box::pin(direct_fixture("model_approval_policy_scope")).await;
    f.auth.assistant_chat = Some(Arc::new(f.chat.clone()));
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.inference =
        Some(serde_json::from_value(json!({"wire_protocol":"openai_responses"})).unwrap());
    let valid = json!({"model":"test","store":false,"input":"test","stream":true});
    let operation =
        crate::services::operation_descriptor::build_http_descriptor("POST", "responses", None);
    for policy in ["required", "default", "rule", "deny", "deny_rule"] {
        let rules = if policy.contains("rule") {
            vec![serde_json::from_value(json!({
            "methods":["POST"],"resource_pattern":"*","effect":if policy == "deny_rule" {"deny"} else {"require_approval"},"mode":"per_request"
        })).unwrap()]
        } else {
            vec![]
        };
        approvals::set_service_approval_config(
            &f.state.db,
            &f.owner,
            &service.id,
            "model",
            Some(policy == "required"),
            Some(&ApprovalMode::PerRequest),
            Some(&rules),
            Some(&if policy == "deny" {
                ApprovalEffect::Deny
            } else if policy == "default" {
                ApprovalEffect::RequireApproval
            } else {
                ApprovalEffect::AutoAllow
            }),
        )
        .await
        .unwrap();
        // The legacy boolean variant must not be masked by an explicit default.
        if policy == "required" {
            f.state
                .db
                .collection::<bson::Document>("service_approval_configs")
                .update_one(
                    doc! {"user_id":&f.owner,"service_id":&service.id},
                    doc! {"$unset":{"default_effect":""}},
                )
                .await
                .unwrap();
        }
        for caller in [
            "assistant",
            "api_key",
            "mcp",
            "direct_owner",
            "idle_key",
            "override",
            "pool",
            "stored",
            "hosted_tool",
            "handle",
            "unknown",
            "get",
            "not_inference",
        ] {
            let mut auth = f.auth.clone();
            let mut body = valid.clone();
            let mut headers = axum::http::HeaderMap::new();
            let mut catalog = service.clone();
            match caller {
                "api_key" | "mcp" | "direct_owner" => auth.assistant_chat = None,
                "idle_key" => {
                    Arc::make_mut(auth.assistant_chat.as_mut().unwrap()).turn_live = false
                }
                "override" => {
                    headers.insert("x-http-method-override", "DELETE".parse().unwrap());
                }
                "stored" => body["store"] = json!(true),
                "hosted_tool" => body["tools"] = json!([{"type":"file_search"}]),
                "handle" => body["previous_response_id"] = json!("test-handle"),
                "unknown" => body["unknown_field"] = json!(true),
                "not_inference" => catalog.inference = None,
                _ => {}
            }
            let observer = ModelRequest::capture(
                &f.state.db,
                &f.row.id,
                &f.row.credential_api_key_id,
                &f.row.active_turn.as_ref().unwrap().turn_id,
            )
            .await
            .unwrap()
            .unwrap();
            observer.configure_policy(
                &auth,
                &catalog,
                if caller == "get" { "GET" } else { "POST" },
                "responses",
                None,
                &headers,
                &serde_json::to_vec(&body).unwrap(),
                (caller == "pool").then_some("pool"),
            );
            let outcome = CURRENT_MODEL
                .scope(
                    Some(observer.clone()),
                    approvals::evaluate_and_check_with_options(
                        &f.state.db,
                        &f.owner,
                        &f.owner,
                        &service.id,
                        &operation,
                        Some("api_key"),
                        &f.row.credential_api_key_id,
                        false,
                        false,
                        approvals::ApprovalOptions::default(),
                    ),
                )
                .await
                .unwrap();
            let deny = policy.starts_with("deny");
            assert_eq!(
                matches!(outcome, approvals::ApprovalOutcome::Denied),
                deny,
                "{policy}/{caller}"
            );
            assert_eq!(
                matches!(outcome, approvals::ApprovalOutcome::Allowed { .. }),
                !deny && caller == "assistant",
                "{policy}/{caller}"
            );
            assert_eq!(
                observer.approval_exempt(),
                !deny && caller == "assistant",
                "{policy}/{caller}"
            );
            observer.complete().await.unwrap();
        }
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn model_approval_live_specialist_llm_policy_uses_same_exemption() {
    let mut f = Box::pin(direct_fixture("model_approval_llm_policy")).await;
    f.auth.assistant_chat = Some(Arc::new(f.chat.clone()));
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.inference =
        Some(serde_json::from_value(json!({"wire_protocol":"openai_responses"})).unwrap());
    approvals::set_service_approval_config(
        &f.state.db,
        &f.owner,
        &service.id,
        "model",
        Some(true),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let bytes = serde_json::to_vec(&json!({"model":"test","store":false,"input":"test"})).unwrap();
    let observer = ModelRequest::capture(
        &f.state.db,
        &f.row.id,
        &f.row.credential_api_key_id,
        &f.row.active_turn.as_ref().unwrap().turn_id,
    )
    .await
    .unwrap()
    .unwrap();
    observer.configure_policy(
        &f.auth,
        &service,
        "POST",
        "responses",
        None,
        &axum::http::HeaderMap::new(),
        &bytes,
        None,
    );
    CURRENT_MODEL
        .scope(
            Some(observer.clone()),
            crate::handlers::llm_gateway::check_llm_approval(
                &f.state,
                &f.auth,
                &service.id,
                &service,
                "responses",
                "POST",
                Some(&bytes),
                Some(&f.owner),
                false,
                false,
                false,
            ),
        )
        .await
        .unwrap();
    assert!(observer.approval_exempt());
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    observer.complete().await.unwrap();
    f.state.db.drop().await.unwrap();
}
