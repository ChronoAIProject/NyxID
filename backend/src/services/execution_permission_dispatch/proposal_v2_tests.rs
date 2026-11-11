use super::super::proposals::{self, Decision, v2};
use super::*;
use crate::models::approval_request::ApprovalRequest;

async fn row(f: &Fixture) -> ApprovalRequest {
    let row = f
        .state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find_one(doc! {"execution_proposal.version":2})
        .await
        .unwrap()
        .expect("one v2 proposal");
    let envelope = row.execution_proposal.as_ref().unwrap();
    assert!(envelope.v3.is_none());
    assert!(!bson::to_document(envelope).unwrap().contains_key("v3"));
    for choice in &v2::proposal(&row).unwrap().choices {
        assert!(choice.conversation_scope.is_none());
        assert!(
            !bson::to_document(choice)
                .unwrap()
                .contains_key("conversation_scope")
        );
    }
    row
}
fn choice(row: &ApprovalRequest, id: &str) -> Decision {
    let p = v2::proposal(row).unwrap();
    let c = p.choices.iter().find(|c| c.id == id).unwrap();
    Decision {
        approved: true,
        expected_revision: p.revision,
        choice_id: Some(c.id.clone()),
        choice_digest: Some(c.digest.clone()),
        idempotency_key: Uuid::new_v4().to_string(),
    }
}
async fn call(f: &Fixture, path: &str, token: &str) -> (StatusCode, String) {
    let r = f
        .router()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("x-api-key", &f.raw)
                .header("idempotency-key", token)
                .header("x-nyxid-approval-mode", "async")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status();
    let body = axum::body::to_bytes(r.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}
async fn run(f: &Fixture, row: &ApprovalRequest) {
    if let Some(r) = v2::execute(&f.state, row).await.unwrap() {
        let _ = axum::body::to_bytes(r.into_body(), 1024 * 1024)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn external_proxy_uuid_and_slug_save_resume_readback_without_task_or_duplicate_send() {
    for slug in [false, true] {
        let f = Fixture::new("proposal_v2_proxy").await;
        f.enroll(true).await;
        let route = if slug {
            "/proxy/s/execution-fixture/status".into()
        } else {
            format!("/proxy/{}/status", f.catalog)
        };
        let (status, body) = call(&f, &route, "one").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        let requested = row(&f).await;
        let p = v2::proposal(&requested).unwrap();
        assert!(p.task.is_none());
        assert!(p.conversation_id.is_none());
        assert!(!p.choices.iter().any(|c| c.id == "task"));
        assert!(f.server.received_requests().await.unwrap().is_empty());
        let again = call(&f, &route, "one").await;
        assert!(again.1.contains(&requested.id), "{}", again.1);
        assert_eq!(
            f.state
                .db
                .collection::<Document>("approval_requests")
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert!(proposals::legacy_refusal(&requested).is_err());
        let decision = choice(&requested, "operation");
        let approved = proposals::decide(&f.state, &f.owner, &requested.id, &decision)
            .await
            .unwrap();
        let replay = proposals::decide(&f.state, &f.owner, &requested.id, &decision)
            .await
            .unwrap();
        assert_eq!(approved.id, replay.id);
        run(&f, &approved).await;
        run(&f, &approved).await;
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        let completed = row(&f).await;
        assert_eq!(v2::proposal(&completed).unwrap().state, "completed");
        assert_eq!(completed.activity_outcome.as_deref(), Some("completed"));
        let auth = f.auth().await;
        let view = v2::readback(&f.state, &auth, &completed).await.unwrap();
        assert!(view.result_available);
        assert_eq!(view.result.unwrap().status, 200);
        for _ in 0..3 {
            v2::readback(&f.state, &auth, &completed).await.unwrap();
        }
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        let mut other = auth.clone();
        other.api_key_id = Some(Uuid::new_v4().to_string());
        assert!(matches!(
            v2::readback(&f.state, &other, &completed).await,
            Err(AppError::NotFound(_))
        ));
        other = auth.clone();
        other.api_key_credential_id = Some(Uuid::new_v4().to_string());
        assert!(matches!(
            v2::readback(&f.state, &other, &completed).await,
            Err(AppError::NotFound(_))
        ));
        other = auth;
        other.user_id = Uuid::new_v4();
        assert!(matches!(
            v2::readback(&f.state, &other, &completed).await,
            Err(AppError::NotFound(_))
        ));
        let (status, body) = call(&f, &route, "new-occurrence").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(
            f.state
                .db
                .collection::<Document>("approval_requests")
                .count_documents(doc! {"execution_proposal.version":2})
                .await
                .unwrap(),
            2,
            "a retired operation cannot prohibit an unrelated new occurrence"
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        f.cleanup().await;
    }
}

#[tokio::test]
async fn external_idempotency_changed_effect_conflicts_and_unknown_handle_cannot_send() {
    let f = Fixture::new("proposal_v2_dedupe").await;
    f.enroll(true).await;
    call(&f, "/proxy/s/execution-fixture/status", "locator").await;
    let first = row(&f).await;
    let (status, body) = call(&f, "/proxy/s/execution-fixture/other", "locator").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        proposals::decide(
            &f.state,
            &f.owner,
            &Uuid::new_v4().to_string(),
            &choice(&first, "operation")
        )
        .await
        .is_err()
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    let mut deny = choice(&first, "operation");
    deny.approved = false;
    deny.choice_id = None;
    deny.choice_digest = None;
    proposals::decide(&f.state, &f.owner, &first.id, &deny)
        .await
        .unwrap();
    let retry = call(&f, "/proxy/s/execution-fixture/status", "locator").await;
    assert!(retry.1.contains(&first.id));
    assert_eq!(
        f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    f.cleanup().await;
}

#[tokio::test]
async fn twenty_concurrent_external_decisions_and_resumes_send_one_effect_for_each_validity() {
    for validity in ["operation", "window", "persistent"] {
        let f = Fixture::new("proposal_v2_race").await;
        f.enroll(true).await;
        // MCP's HTTP preference opts into background delivery without adding a
        // semantic header to the prepared service effect. All validity choices
        // remain enforceable; an Idempotency-Key on raw proxy HTTP would correctly
        // limit that effect to Operation-only instead.
        let (app, mut request) = connected_request(&f, "mcp").await;
        request
            .headers_mut()
            .insert("x-nyxid-approval-mode", "async".parse().unwrap());
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let r = row(&f).await;
        let input = choice(&r, validity);
        let decisions = futures::future::join_all(
            (0..20).map(|_| proposals::decide(&f.state, &f.owner, &r.id, &input)),
        )
        .await;
        for decision in decisions {
            assert_eq!(decision.unwrap().id, r.id);
        }
        let approved = row(&f).await;
        let replies = futures::future::join_all((0..20).map(|_| async {
            if let Some(r) = v2::execute(&f.state, &approved).await.unwrap() {
                let _ = axum::body::to_bytes(r.into_body(), 1024 * 1024)
                    .await
                    .unwrap();
            }
        }))
        .await;
        assert_eq!(replies.len(), 20);
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            f.state
                .db
                .collection::<Document>("approval_grants")
                .count_documents(doc! {"approval_request_id":&r.id})
                .await
                .unwrap(),
            1
        );
        let stored = f
            .state
            .db
            .collection::<ApprovalGrant>("approval_grants")
            .find_one(doc! {"approval_request_id":&r.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.execution_use.is_some(), validity == "operation");
        f.cleanup().await;
    }
}

#[tokio::test]
async fn external_revocation_and_uncertain_barrier_never_replay() {
    for scenario in ["revoked", "uncertain", "disabled"] {
        let f = Fixture::new("proposal_v2_fences").await;
        f.enroll(true).await;
        call(&f, "/proxy/s/execution-fixture/status", "async-occurrence").await;
        let r = row(&f).await;
        let approved = proposals::decide(&f.state, &f.owner, &r.id, &choice(&r, "operation"))
            .await
            .unwrap();
        match scenario {
            "revoked" => {
                f.state
                    .db
                    .collection::<Document>("api_keys")
                    .update_one(doc! {"_id":&f.key}, doc! {"$set":{"is_active":false}})
                    .await
                    .unwrap();
            }
            "disabled" => f.enroll(false).await,
            _ => {
                f.state.db.collection::<Document>("approval_requests").update_one(doc!{"_id":&r.id},doc!{"$set":{"execution_proposal.v2.state":"dispatched","execution_proposal.v2.claimed_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(6))}}).await.unwrap();
            }
        }
        let outcome = v2::execute(&f.state, &approved).await;
        assert!(outcome.is_err() || outcome.unwrap().is_none());
        v2::sweep(&f.state).await.unwrap();
        let terminal = row(&f).await;
        let terminal = v2::proposal(&terminal).unwrap();
        assert!(terminal.payload.is_none());
        assert_eq!(
            terminal.state,
            match scenario {
                "uncertain" => "unknown",
                "revoked" => "not_sent",
                _ => "cancelled",
            }
        );
        assert!(f.server.received_requests().await.unwrap().is_empty());
        f.cleanup().await;
    }
}

#[tokio::test]
async fn external_child_only_operation_and_exact_readback_binding() {
    let mut f = Fixture::new("proposal_v2_child").await;
    f.enroll(true).await;
    let raw = crate::services::api_key_credential_service::generate_secret();
    let child = Uuid::new_v4().to_string();
    let row = doc! {"_id":&child,"api_key_id":&f.key,"user_id":&f.owner,"secret_hash":crate::crypto::token::hash_token(&raw),"secret_prefix":&raw[..16],"label":"Child","login_request_id":Uuid::new_v4().to_string(),"is_active":true,"expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::minutes(8)),"created_at":bson::DateTime::now()};
    f.state
        .db
        .collection::<Document>("api_key_credentials")
        .insert_one(row)
        .await
        .unwrap();
    f.raw = raw.to_string();
    let (status, text) = call(&f, "/proxy/s/execution-fixture/status", "async-occurrence").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
    let requested = super::proposal_v2_tests::row(&f).await;
    let p = v2::proposal(&requested).unwrap();
    assert_eq!(p.source_credential_id.as_deref(), Some(child.as_str()));
    assert_eq!(p.choices.len(), 1);
    assert_eq!(p.choices[0].id, "operation");
    let key = f
        .state
        .db
        .collection::<ApiKey>("api_keys")
        .find_one(doc! {"_id":&f.key})
        .await
        .unwrap()
        .unwrap();
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, Some(child), None, None)
        .await
        .unwrap();
    v2::readback(&f.state, &auth, &requested).await.unwrap();
    assert!(matches!(
        v2::readback(&f.state, &f.auth().await, &requested).await,
        Err(AppError::NotFound(_))
    ));
    let approved = proposals::decide(
        &f.state,
        &f.owner,
        &requested.id,
        &choice(&requested, "operation"),
    )
    .await
    .unwrap();
    run(&f, &approved).await;
    let completed = super::proposal_v2_tests::row(&f).await;
    assert!(
        v2::readback(&f.state, &auth, &completed)
            .await
            .unwrap()
            .result_available
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn external_mcp_endpoint_and_meta_gaps_resume_one_exact_effect() {
    for meta in [false, true] {
        let f = Fixture::new("proposal_v2_mcp").await;
        f.enroll(true).await;
        f.state
            .db
            .collection("service_endpoints")
            .insert_one(test_utils::test_service_endpoint(
                &f.catalog, "status", "GET", "/status",
            ))
            .await
            .unwrap();
        let params = if meta {
            json!({"name":"nyx__call_tool","arguments":{"tool_name":"execution-fixture__status","arguments_json":"{}"}})
        } else {
            json!({"name":"execution-fixture__status","arguments":{}})
        };
        let r = f
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("x-nyxid-approval-mode", "async")
                    .header("x-api-key", &f.raw)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(r.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["result"]["isError"], true, "{value}");
        let r = row(&f).await;
        assert!(f.server.received_requests().await.unwrap().is_empty());
        let approved = proposals::decide(&f.state, &f.owner, &r.id, &choice(&r, "operation"))
            .await
            .unwrap();
        run(&f, &approved).await;
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        assert_eq!(v2::proposal(&row(&f).await).unwrap().state, "completed");
        f.cleanup().await;
    }
}

#[tokio::test]
async fn both_standalone_llm_waiters_stream_the_single_approved_attempt() {
    for translated in [false, true] {
        let f = Fixture::new("proposal_v2_llm").await;
        f.enroll(true).await;
        f.server.reset().await;
        Mock::given(any_request()).respond_with(ResponseTemplate::new(200)
            .insert_header("content-type","text/event-stream")
            .set_body_string("data: {\"id\":\"chatcmpl-test\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n"))
            .mount(&f.server).await;
        let provider = Uuid::new_v4().to_string();
        let config:crate::models::provider_config::ProviderConfig=bson::from_document(doc!{"_id":&provider,"slug":"openai","name":"Fixture","provider_type":"api_key","is_active":true,"created_by":&f.owner,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
        f.state
            .db
            .collection("provider_configs")
            .insert_one(config)
            .await
            .unwrap();
        f.state.db.collection::<Document>("downstream_services").update_one(doc!{"_id":&f.catalog},doc!{"$set":{"slug":"llm-openai","provider_config_id":provider,"inference":{"wire_protocol":"openai_completions","model_list":true,"realtime":false}}}).await.unwrap();
        use crate::services::billing::{BillingIngress, route_inventory::BillingRoutePolicy};
        let app = Router::new()
            .route(
                "/llm/gateway/v1/{*path}",
                any(crate::handlers::llm_gateway::gateway_request).layer(Extension(
                    BillingRoutePolicy::Metered(BillingIngress::LlmGateway),
                )),
            )
            .route(
                "/llm/{provider_slug}/v1/{*path}",
                any(crate::handlers::llm_gateway::llm_proxy_request).layer(Extension(
                    BillingRoutePolicy::Metered(BillingIngress::LlmProvider),
                )),
            )
            .with_state(f.state.clone());
        let path = if translated {
            "/llm/gateway/v1/chat/completions"
        } else {
            "/llm/openai/v1/chat/completions"
        };
        let request=Request::builder().method("POST").uri(path).header("x-api-key",&f.raw).header("content-type","application/json").body(Body::from(json!({"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"hello"}]}).to_string())).unwrap();
        let waiter = tokio::spawn(app.oneshot(request));
        let pending = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                if let Some(row) = f
                    .state
                    .db
                    .collection::<ApprovalRequest>("approval_requests")
                    .find_one(doc! {"execution_proposal.version":2})
                    .await
                    .unwrap()
                {
                    break row;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(f.server.received_requests().await.unwrap().is_empty());
        proposals::decide(
            &f.state,
            &f.owner,
            &pending.id,
            &choice(&pending, "operation"),
        )
        .await
        .unwrap();
        let r = waiter.await.unwrap().unwrap();
        let status = r.status();
        let body = axum::body::to_bytes(r.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert!(String::from_utf8_lossy(&body).contains("hello"));
        assert!(String::from_utf8_lossy(&body).contains("[DONE]"));
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        assert_eq!(v2::proposal(&row(&f).await).unwrap().state, "completed");
        f.cleanup().await;
    }
}

#[tokio::test]
async fn pool_proposal_pins_round_robin_selection_and_refuses_membership_drift() {
    use crate::models::service_pool::{PoolStrategy, ServicePoolMember};
    for drift in [false, true] {
        let f = Fixture::new("proposal_v2_pool").await;
        f.enroll(true).await;
        let mut other = f
            .state
            .db
            .collection::<crate::models::user_service::UserService>("user_services")
            .find_one(doc! {"_id":&f.connection})
            .await
            .unwrap()
            .unwrap();
        other.id = Uuid::new_v4().to_string();
        other.slug = "execution-second".into();
        f.state
            .db
            .collection::<crate::models::user_service::UserService>("user_services")
            .insert_one(&other)
            .await
            .unwrap();
        let pool = super::super::super::service_pool_service::create_pool(
            &f.state.db,
            &f.owner,
            super::super::super::service_pool_service::CreatePoolInput {
                slug: "review-pool".into(),
                name: "Review pool".into(),
                description: None,
                strategy: PoolStrategy::RoundRobin,
                tier_balance: Default::default(),
                member_contract: Default::default(),
                failover: None,
                is_active: Some(true),
                members: [f.connection.clone(), other.id.clone()]
                    .into_iter()
                    .map(|id| ServicePoolMember {
                        user_service_id: id,
                        weight: 1,
                        enabled: true,
                        priority: 0,
                        model: None,
                        same_api_compatible: false,
                        health_reset_generation: 0,
                    })
                    .collect(),
            },
        )
        .await
        .unwrap();
        let route = "/proxy/s/review-pool/status";
        let (status, body) = call(&f, route, "pool-operation").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        let pending = row(&f).await;
        for _ in 0..3 {
            let (status, body) = call(&f, route, "pool-operation").await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
            assert!(body.contains(&pending.id), "{body}");
        }
        let approved = proposals::decide(
            &f.state,
            &f.owner,
            &pending.id,
            &choice(&pending, "operation"),
        )
        .await
        .unwrap();
        if drift {
            f.state
                .db
                .collection::<Document>("service_pools")
                .update_one(
                    doc! {"_id":pool.id},
                    doc! {"$inc":{"config_revision":1_i64}},
                )
                .await
                .unwrap();
            assert!(v2::execute(&f.state, &approved).await.is_err());
            assert!(f.server.received_requests().await.unwrap().is_empty());
        } else {
            run(&f, &approved).await;
            assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        }
        f.cleanup().await;
    }
}

#[tokio::test]
async fn external_nonretainable_input_keeps_legacy_review_without_v2_write() {
    for oversized in [false, true] {
        let f = Fixture::new("proposal_v2_fallback").await;
        f.enroll(true).await;
        let channel =
            crate::services::notification_service::get_or_create_channel(&f.state.db, &f.owner)
                .await
                .unwrap();
        f.state
            .db
            .collection::<Document>("notification_channels")
            .update_one(
                doc! {"_id":channel.id},
                doc! {"$set":{"approval_timeout_secs":1}},
            )
            .await
            .unwrap();
        let input = if oversized {
            json!({"value":"a".repeat(256*1024)}).to_string()
        } else {
            "{}".into()
        };
        let mut request = Request::builder()
            .method("POST")
            .uri("/proxy/s/execution-fixture/status")
            .header("x-api-key", &f.raw)
            .header("content-type", "application/json");
        if !oversized {
            request = request
                .header("idempotency-key", "")
                .header("x-nyxid-approval-mode", "async");
        }
        let response = f
            .router()
            .oneshot(request.body(Body::from(input)).unwrap())
            .await
            .unwrap();
        assert!(!response.status().is_success());
        assert!(f.server.received_requests().await.unwrap().is_empty());
        assert_eq!(
            f.state
                .db
                .collection::<Document>("approval_requests")
                .count_documents(doc! {"execution_proposal.version":2})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            f.state
                .db
                .collection::<Document>("approval_requests")
                .count_documents(doc! {"execution_proposal":null})
                .await
                .unwrap(),
            1
        );
        f.cleanup().await;
    }
}

#[tokio::test]
async fn external_resume_recovers_only_before_send_and_never_recreates_lost_or_large_output() {
    for scenario in ["preparing", "disconnected", "oversized"] {
        let f = Fixture::new("proposal_v2_delivery").await;
        f.enroll(true).await;
        if scenario == "oversized" {
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .respond_with(
                    wiremock::ResponseTemplate::new(200).set_body_string("x".repeat(300 * 1024)),
                )
                .with_priority(1)
                .mount(&f.server)
                .await;
        }
        call(&f, "/proxy/s/execution-fixture/status", "async-occurrence").await;
        let requested = row(&f).await;
        let approved = proposals::decide(
            &f.state,
            &f.owner,
            &requested.id,
            &choice(&requested, "operation"),
        )
        .await
        .unwrap();
        if scenario == "preparing" {
            assert!(
                v2::claim_for_test(&f.state, &approved)
                    .await
                    .unwrap()
                    .is_some()
            );
            f.state.db.collection::<Document>("approval_requests").update_one(
                doc! {"_id": &approved.id},
                doc! {"$set": {"execution_proposal.v2.claimed_at": bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(3))}},
            ).await.unwrap();
            assert!(f.server.received_requests().await.unwrap().is_empty());
            run(&f, &row(&f).await).await;
        } else if scenario == "disconnected" {
            let response = v2::execute(&f.state, &approved).await.unwrap().unwrap();
            drop(response);
            f.state.db.collection::<Document>("approval_requests").update_one(
                doc! {"_id": &approved.id},
                doc! {"$set": {"execution_proposal.v2.claimed_at": bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(6))}},
            ).await.unwrap();
            v2::sweep(&f.state).await.unwrap();
        } else {
            run(&f, &approved).await;
        }
        let completed = row(&f).await;
        let p = v2::proposal(&completed).unwrap();
        assert_eq!(
            p.state,
            if scenario == "disconnected" {
                "unknown"
            } else {
                "completed"
            }
        );
        assert!(p.payload.is_none());
        if scenario == "oversized" {
            let receipt = v2::decrypt_result(&f.state, &completed)
                .await
                .unwrap()
                .unwrap();
            assert!(receipt.partial);
            assert!(receipt.body.len() <= proposals::MAX_PAYLOAD);
        }
        assert!(v2::execute(&f.state, &completed).await.unwrap().is_none());
        assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
        f.cleanup().await;
    }
}

#[tokio::test]
async fn forwarded_source_bearer_keeps_legacy_review_without_retaining_a_credential() {
    let f = Fixture::new("proposal_v2_bearer_fallback").await;
    f.enroll(true).await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id": &f.connection},
            doc! {"$set": {"forward_access_token": true}},
        )
        .await
        .unwrap();
    assert!(f.target().await.service.forward_access_token);
    let channel =
        crate::services::notification_service::get_or_create_channel(&f.state.db, &f.owner)
            .await
            .unwrap();
    f.state
        .db
        .collection::<Document>("notification_channels")
        .update_one(
            doc! {"_id": channel.id},
            doc! {"$set": {"approval_timeout_secs": 1}},
        )
        .await
        .unwrap();
    let response = f
        .router()
        .oneshot(
            Request::builder()
                .uri("/proxy/s/execution-fixture/status")
                .header("authorization", format!("Bearer {}", f.raw))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(!response.status().is_success());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {"execution_proposal": {"$ne": null}})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {"execution_proposal": null})
            .await
            .unwrap(),
        1
    );
    f.cleanup().await;
}

async fn approval_timeout(f: &Fixture, seconds: u32) {
    let channel =
        crate::services::notification_service::get_or_create_channel(&f.state.db, &f.owner)
            .await
            .unwrap();
    f.state
        .db
        .collection::<Document>("notification_channels")
        .update_one(
            doc! {"_id":channel.id},
            doc! {"$set":{"approval_timeout_secs":seconds as i32}},
        )
        .await
        .unwrap();
}

async fn pending_request(f: &Fixture) -> ApprovalRequest {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(row) = f
                .state
                .db
                .collection::<ApprovalRequest>("approval_requests")
                .find_one(doc! {"execution_proposal.version":2,"status":"pending"})
                .await
                .unwrap()
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("connected caller creates a review")
}

async fn connected_request(f: &Fixture, kind: &str) -> (Router, Request<Body>) {
    let mut app = f.router();
    let mut request = Request::builder().header("x-api-key", &f.raw);
    let body = match kind {
        "mcp" | "mcp_meta" => {
            if f.state
                .db
                .collection::<Document>("service_endpoints")
                .count_documents(doc! {})
                .await
                .unwrap()
                == 0
            {
                f.state
                    .db
                    .collection("service_endpoints")
                    .insert_one(test_utils::test_service_endpoint(
                        &f.catalog, "status", "GET", "/status",
                    ))
                    .await
                    .unwrap();
            }
            request = request
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json");
            let params = if kind == "mcp" {
                json!({"name":"execution-fixture__status","arguments":{}})
            } else {
                json!({"name":"nyx__call_tool","arguments":{"tool_name":"execution-fixture__status","arguments_json":"{}"}})
            };
            Body::from(
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params}).to_string(),
            )
        }
        "llm" | "gateway" => {
            use crate::services::billing::{BillingIngress, route_inventory::BillingRoutePolicy};
            f.server.reset().await;
            Mock::given(any_request()).respond_with(ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
                .set_body_string("data: {\"id\":\"chatcmpl-test\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n")).mount(&f.server).await;
            let provider = Uuid::new_v4().to_string();
            let config:crate::models::provider_config::ProviderConfig=bson::from_document(doc!{"_id":&provider,"slug":"openai","name":"Fixture","provider_type":"api_key","is_active":true,"created_by":&f.owner,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
            f.state
                .db
                .collection("provider_configs")
                .insert_one(config)
                .await
                .unwrap();
            f.state.db.collection::<Document>("downstream_services").update_one(doc!{"_id":&f.catalog},doc!{"$set":{"slug":"llm-openai","provider_config_id":provider,"inference":{"wire_protocol":"openai_completions","model_list":true,"realtime":false}}}).await.unwrap();
            app = app.merge(
                Router::new()
                    .route(
                        "/llm/gateway/v1/{*path}",
                        any(crate::handlers::llm_gateway::gateway_request).layer(Extension(
                            BillingRoutePolicy::Metered(BillingIngress::LlmGateway),
                        )),
                    )
                    .route(
                        "/llm/{provider_slug}/v1/{*path}",
                        any(crate::handlers::llm_gateway::llm_proxy_request).layer(Extension(
                            BillingRoutePolicy::Metered(BillingIngress::LlmProvider),
                        )),
                    )
                    .with_state(f.state.clone()),
            );
            request = request
                .method("POST")
                .uri(if kind == "llm" {
                    "/llm/openai/v1/chat/completions"
                } else {
                    "/llm/gateway/v1/chat/completions"
                })
                .header("content-type", "application/json");
            Body::from(json!({"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"hello"}]}).to_string())
        }
        "pool" => {
            use crate::models::service_pool::{PoolStrategy, ServicePoolMember};
            crate::services::service_pool_service::create_pool(
                &f.state.db,
                &f.owner,
                crate::services::service_pool_service::CreatePoolInput {
                    slug: "connected-pool".into(),
                    name: "Connected pool".into(),
                    description: None,
                    strategy: PoolStrategy::RoundRobin,
                    tier_balance: Default::default(),
                    member_contract: Default::default(),
                    failover: None,
                    is_active: Some(true),
                    members: vec![ServicePoolMember {
                        user_service_id: f.connection.clone(),
                        weight: 1,
                        enabled: true,
                        priority: 0,
                        model: None,
                        same_api_compatible: false,
                        health_reset_generation: 0,
                    }],
                },
            )
            .await
            .unwrap();
            request = request.uri("/proxy/s/connected-pool/status");
            Body::empty()
        }
        "uuid" => {
            request = request.uri(format!("/proxy/{}/status", f.catalog));
            Body::empty()
        }
        _ => {
            request = request.uri("/proxy/s/execution-fixture/status");
            Body::empty()
        }
    };
    (app, request.body(body).unwrap())
}

#[tokio::test]
async fn external_connected_wait_returns_real_proxy_pool_mcp_and_llm_responses_once() {
    for kind in ["uuid", "slug", "pool", "mcp", "mcp_meta", "llm", "gateway"] {
        let f = Fixture::new("proposal_connected").await;
        super::super::phase_b_tests::enroll_global(&f.state.db, &f.owner).await;
        approval_timeout(&f, 10).await;
        let (app, request) = connected_request(&f, kind).await;
        let waiter = tokio::spawn(app.oneshot(request));
        let pending = pending_request(&f).await;
        let p = v2::proposal(&pending).unwrap();
        assert!(!p.background_allowed);
        assert_eq!(p.waiter_until, Some(pending.expires_at));
        assert!((pending.expires_at - pending.created_at).num_seconds() <= 10);
        assert!(
            !waiter.is_finished(),
            "{kind} must keep the original connection waiting"
        );
        v2::sweep(&f.state).await.unwrap();
        assert!(f.server.received_requests().await.unwrap().is_empty());
        proposals::decide(
            &f.state,
            &f.owner,
            &pending.id,
            &choice(&pending, "operation"),
        )
        .await
        .unwrap();
        let response = waiter.await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{kind}");
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        if kind.starts_with("mcp") {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["result"]["isError"], false, "{value}");
            assert!(
                value["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("true"),
                "{value}"
            );
        } else if matches!(kind, "llm" | "gateway") {
            assert!(text.contains("hello") && text.contains("[DONE]"), "{text}");
        } else {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&text).unwrap(),
                json!({"ok":true})
            );
        }
        assert_eq!(
            f.server.received_requests().await.unwrap().len(),
            1,
            "{kind}"
        );
        assert_eq!(
            v2::proposal(&row(&f).await).unwrap().state,
            "completed",
            "{kind}"
        );
        f.cleanup().await;
    }
}

#[tokio::test]
async fn external_connected_timeout_is_undecidable_unsent_and_keeps_legacy_errors() {
    for kind in ["uuid", "slug", "pool", "mcp", "mcp_meta", "llm", "gateway"] {
        let f = Fixture::new("proposal_timeout").await;
        super::super::phase_b_tests::enroll_global(&f.state.db, &f.owner).await;
        approval_timeout(&f, 1).await;
        let (app, request) = connected_request(&f, kind).await;
        let start = std::time::Instant::now();
        let response =
            tokio::time::timeout(std::time::Duration::from_secs(5), app.oneshot(request))
                .await
                .unwrap()
                .unwrap();
        assert!(
            start.elapsed() >= std::time::Duration::from_millis(900),
            "{kind} did not wait"
        );
        let text = String::from_utf8(
            axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        if kind.starts_with("mcp") {
            assert!(
                text.contains("approval_wait_timed_out") && text.contains("sent nothing"),
                "{text}"
            );
        } else {
            assert!(text.contains("Approval request timed out"), "{text}");
        }
        assert!(
            text.contains(&format!(
                "{}/assistant/approvals",
                f.state.config.frontend_url.trim_end_matches('/')
            )),
            "{text}"
        );
        let ended = row(&f).await;
        assert_eq!(ended.status, "expired");
        assert_eq!(v2::proposal(&ended).unwrap().state, "not_sent");
        assert!(v2::proposal(&ended).unwrap().payload.is_none());
        assert!(!proposals::view(&ended).unwrap().can_decide);
        assert!(
            proposals::decide(&f.state, &f.owner, &ended.id, &choice(&ended, "operation"))
                .await
                .is_err()
        );
        v2::sweep(&f.state).await.unwrap();
        assert!(v2::execute(&f.state, &ended).await.unwrap().is_none());
        assert!(f.server.received_requests().await.unwrap().is_empty());
        f.cleanup().await;
    }
}

#[tokio::test]
async fn external_async_requires_nyxid_header_with_optional_dedupe_and_never_forwards_control_header()
 {
    for kind in ["slug", "mcp", "llm", "gateway"] {
        for idempotency in [false, true] {
            let f = Fixture::new("proposal_async_opt_in").await;
            super::super::phase_b_tests::enroll_global(&f.state.db, &f.owner).await;
            approval_timeout(&f, 10).await;
            let (app, mut request) = connected_request(&f, kind).await;
            if idempotency {
                request
                    .headers_mut()
                    .insert("idempotency-key", "explicit-occurrence".parse().unwrap());
            }
            request
                .headers_mut()
                .insert("x-nyxid-approval-mode", "async".parse().unwrap());
            let response =
                tokio::time::timeout(std::time::Duration::from_secs(3), app.oneshot(request))
                    .await
                    .unwrap()
                    .unwrap();
            let _ = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let pending = row(&f).await;
            assert!(v2::proposal(&pending).unwrap().background_allowed);
            assert!(v2::proposal(&pending).unwrap().waiter_until.is_none());
            assert!(f.server.received_requests().await.unwrap().is_empty());
            let approved = proposals::decide(
                &f.state,
                &f.owner,
                &pending.id,
                &choice(&pending, "operation"),
            )
            .await
            .unwrap();
            run(&f, &approved).await;
            let auth = f.auth().await;
            let result = v2::readback(&f.state, &auth, &row(&f).await).await.unwrap();
            assert_eq!(result.state, "completed");
            assert!(result.result_available);
            let calls = f.server.received_requests().await.unwrap();
            assert_eq!(calls.len(), 1);
            assert!(!calls[0].headers.contains_key("x-nyxid-approval-mode"));
            f.cleanup().await;
        }
    }
}

#[tokio::test]
async fn retry_after_connected_timeout_with_window_permission_sends_only_the_retry() {
    let f = Fixture::new("proposal_timeout_retry").await;
    super::super::phase_b_tests::enroll_global(&f.state.db, &f.owner).await;
    approval_timeout(&f, 1).await;
    let (app, request) = connected_request(&f, "slug").await;
    let _ = app.oneshot(request).await.unwrap();
    let expired = row(&f).await;
    assert!(
        proposals::decide(&f.state, &f.owner, &expired.id, &choice(&expired, "window"))
            .await
            .is_err()
    );
    approval_timeout(&f, 10).await;
    let (app, request) = connected_request(&f, "slug").await;
    let retry = tokio::spawn(app.oneshot(request));
    let pending = pending_request(&f).await;
    assert_ne!(pending.id, expired.id);
    proposals::decide(&f.state, &f.owner, &pending.id, &choice(&pending, "window"))
        .await
        .unwrap();
    let response = retry.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    v2::sweep(&f.state).await.unwrap();
    assert!(v2::execute(&f.state, &expired).await.unwrap().is_none());
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn external_disconnect_expires_connected_review_without_background_send() {
    use tokio::io::AsyncWriteExt;
    let f = Fixture::new("proposal_disconnect").await;
    super::super::phase_b_tests::enroll_global(&f.state.db, &f.owner).await;
    approval_timeout(&f, 60).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = f.router();
    let server = tokio::spawn(async move {
        axum::serve(
            crate::downstream_disconnect::DisconnectAwareListener::new(listener),
            crate::downstream_disconnect::DisconnectAwareMakeService::new(app),
        )
        .await
        .unwrap();
    });
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket.write_all(format!("GET /proxy/s/execution-fixture/status HTTP/1.1\r\nHost: {address}\r\nx-api-key: {}\r\n\r\n",f.raw).as_bytes()).await.unwrap();
    let pending = pending_request(&f).await;
    drop(socket);
    let ended = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let row = row(&f).await;
            if row.status == "expired" {
                break row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(v2::proposal(&ended).unwrap().state, "not_sent");
    assert!(
        proposals::decide(&f.state, &f.owner, &pending.id, &choice(&pending, "window"))
            .await
            .is_err()
    );
    v2::sweep(&f.state).await.unwrap();
    assert!(f.server.received_requests().await.unwrap().is_empty());
    server.abort();
    let _ = server.await;
    f.cleanup().await;
}

#[tokio::test]
async fn concurrent_proposals_on_one_endpoint_ignore_transaction_lock_counters() {
    let f = Fixture::new("proposal_endpoint_concurrency").await;
    super::super::phase_b_tests::enroll_global(&f.state.db, &f.owner).await;
    let (app, mut first) = connected_request(&f, "mcp").await;
    let (_, mut second) = connected_request(&f, "mcp").await;
    for request in [&mut first, &mut second] {
        request
            .headers_mut()
            .insert("x-nyxid-approval-mode", "async".parse().unwrap());
    }
    first
        .headers_mut()
        .insert("idempotency-key", "first".parse().unwrap());
    second
        .headers_mut()
        .insert("idempotency-key", "second".parse().unwrap());
    let (a, b) = tokio::join!(app.clone().oneshot(first), app.oneshot(second));
    assert_eq!(a.unwrap().status(), StatusCode::OK);
    assert_eq!(b.unwrap().status(), StatusCode::OK);
    let rows: Vec<ApprovalRequest> = f
        .state
        .db
        .collection::<ApprovalRequest>("approval_requests")
        .find(doc! {"execution_proposal.version":2,"status":"pending"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    let first = choice(&rows[0], "operation");
    let second = choice(&rows[1], "operation");
    let (a, b) = tokio::join!(
        proposals::decide(&f.state, &f.owner, &rows[0].id, &first),
        proposals::decide(&f.state, &f.owner, &rows[1].id, &second)
    );
    run(&f, &a.unwrap()).await;
    run(&f, &b.unwrap()).await;
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    f.cleanup().await;
}

#[tokio::test]
async fn connected_wait_without_notification_settings_uses_legacy_default_without_snapshot_drift() {
    let f = Fixture::new("proposal_no_channel").await;
    super::super::phase_b_tests::enroll_global(&f.state.db, &f.owner).await;
    let (app, request) = connected_request(&f, "slug").await;
    let waiter = tokio::spawn(app.oneshot(request));
    let pending = pending_request(&f).await;
    assert_eq!((pending.expires_at - pending.created_at).num_seconds(), 300);
    v2::sweep(&f.state).await.unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<Document>("notification_channels")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    proposals::decide(
        &f.state,
        &f.owner,
        &pending.id,
        &choice(&pending, "operation"),
    )
    .await
    .unwrap();
    let response = waiter.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn external_connected_rejection_preserves_http_and_mcp_legacy_wording() {
    for kind in ["slug", "mcp", "llm", "gateway"] {
        let f = Fixture::new("proposal_connected_rejected").await;
        super::super::phase_b_tests::enroll_global(&f.state.db, &f.owner).await;
        approval_timeout(&f, 10).await;
        let (app, request) = connected_request(&f, kind).await;
        let waiter = tokio::spawn(app.oneshot(request));
        let pending = pending_request(&f).await;
        let mut decision = choice(&pending, "operation");
        decision.approved = false;
        decision.choice_id = None;
        decision.choice_digest = None;
        proposals::decide(&f.state, &f.owner, &pending.id, &decision)
            .await
            .unwrap();
        let response = waiter.await.unwrap().unwrap();
        let text = String::from_utf8(
            axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        if kind == "mcp" {
            let rpc: serde_json::Value = serde_json::from_str(&text).unwrap();
            let actual: serde_json::Value =
                serde_json::from_str(rpc["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            let expected = crate::services::approval_service::wait_failure_result(
                AppError::Forbidden("Approval request was rejected".into()),
                &pending.id,
                &f.state.config.frontend_url,
            );
            assert_eq!(actual, expected);
            assert_eq!(actual["code"], "approval_denied");
            assert_eq!(rpc["result"]["isError"], true);
        } else {
            assert!(text.contains("Approval request was rejected"), "{text}");
            assert!(text.contains(&f.state.config.frontend_url), "{text}");
        }
        v2::sweep(&f.state).await.unwrap();
        assert!(f.server.received_requests().await.unwrap().is_empty());
        f.cleanup().await;
    }
}

#[tokio::test]
async fn downstream_headers_and_other_approval_modes_keep_connected_wait_and_real_response() {
    for kind in ["uuid", "slug", "pool", "mcp", "mcp_meta", "llm", "gateway"] {
        for (header, value) in [
            ("idempotency-key", "stripe-existing-client"),
            ("prefer", "respond-async"),
            ("x-nyxid-approval-mode", "other"),
        ] {
            let f = Fixture::new("proposal_downstream_headers").await;
            super::super::phase_b_tests::enroll_global(&f.state.db, &f.owner).await;
            approval_timeout(&f, 10).await;
            let (app, mut request) = connected_request(&f, kind).await;
            request.headers_mut().insert(header, value.parse().unwrap());
            let waiter = tokio::spawn(app.oneshot(request));
            let pending = pending_request(&f).await;
            assert!(
                !v2::proposal(&pending).unwrap().background_allowed,
                "{kind} {header}"
            );
            assert!(!waiter.is_finished(), "{kind} {header} must wait");
            assert!(f.server.received_requests().await.unwrap().is_empty());
            proposals::decide(
                &f.state,
                &f.owner,
                &pending.id,
                &choice(&pending, "operation"),
            )
            .await
            .unwrap();
            let response = waiter.await.unwrap().unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            if matches!(kind, "llm" | "gateway") {
                assert!(text.contains("hello") && text.contains("[DONE]"), "{text}");
            } else if kind.starts_with("mcp") {
                let result: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(result["result"]["isError"], false, "{text}");
                assert!(
                    result["result"]["content"][0]["text"]
                        .as_str()
                        .unwrap()
                        .contains("true"),
                    "{text}"
                );
            } else {
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&text).unwrap(),
                    json!({"ok":true})
                );
            }
            let calls = f.server.received_requests().await.unwrap();
            assert_eq!(calls.len(), 1, "{kind} {header}");
            assert!(!calls[0].headers.contains_key("x-nyxid-approval-mode"));
            // MCP HTTP envelope headers are not tool arguments. Ordinary proxy
            // providers must receive their downstream headers unchanged.
            if matches!(kind, "uuid" | "slug" | "pool") && header != "x-nyxid-approval-mode" {
                assert_eq!(calls[0].headers.get(header).unwrap(), value);
            }
            assert_eq!(v2::proposal(&row(&f).await).unwrap().state, "completed");
            f.cleanup().await;
        }
    }
}

#[tokio::test]
async fn connected_same_idempotency_key_retry_after_timeout_prompts_again_and_sends_once() {
    let f = Fixture::new("proposal_connected_same_key").await;
    super::super::phase_b_tests::enroll_global(&f.state.db, &f.owner).await;
    approval_timeout(&f, 1).await;
    let (app, mut first) = connected_request(&f, "slug").await;
    first
        .headers_mut()
        .insert("idempotency-key", "same-provider-key".parse().unwrap());
    let response = app.oneshot(first).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Approval request timed out"), "{text}");
    assert!(
        text.contains(&proposals::approve_url(&f.state.config.frontend_url)),
        "{text}"
    );
    let expired = row(&f).await;
    let AppError::ApprovalFailed { approve_url, .. } =
        v2::pending(&expired, &f.state.config.frontend_url)
    else {
        panic!("terminal error")
    };
    assert_eq!(
        approve_url,
        proposals::approve_url(&f.state.config.frontend_url)
    );
    approval_timeout(&f, 10).await;
    let (app, mut retry) = connected_request(&f, "slug").await;
    retry
        .headers_mut()
        .insert("idempotency-key", "same-provider-key".parse().unwrap());
    let waiter = tokio::spawn(app.oneshot(retry));
    let pending = pending_request(&f).await;
    assert_ne!(pending.id, expired.id);
    assert_ne!(
        v2::proposal(&pending).unwrap().locator,
        v2::proposal(&expired).unwrap().locator
    );
    assert!(!waiter.is_finished());
    assert!(
        proposals::decide(
            &f.state,
            &f.owner,
            &expired.id,
            &choice(&expired, "operation")
        )
        .await
        .is_err()
    );
    proposals::decide(
        &f.state,
        &f.owner,
        &pending.id,
        &choice(&pending, "operation"),
    )
    .await
    .unwrap();
    let response = waiter.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    v2::sweep(&f.state).await.unwrap();
    assert!(v2::execute(&f.state, &expired).await.unwrap().is_none());
    let calls = f.server.received_requests().await.unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].headers.get("idempotency-key").unwrap(),
        "same-provider-key"
    );
    f.cleanup().await;
}

#[tokio::test]
async fn v2_notifies_inline_before_first_wait_poll_once_and_polls_at_legacy_cadence() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut f = Fixture::new("proposal_inline_notification").await;
    f.enroll(true).await;
    approval_timeout(&f, 15).await;
    f.state.config.telegram_bot_token = Some("test-token".into());
    f.state
        .db
        .collection::<Document>("notification_channels")
        .update_one(
            doc! {"user_id": &f.owner},
            doc! {"$set": {"telegram_enabled": true, "telegram_chat_id": 123_i64}},
        )
        .await
        .unwrap();

    // Exercise the actual Telegram notification service over locally verified
    // TLS, with no public network call or production transport override.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let certificate = rcgen::generate_simple_self_signed(vec!["api.telegram.org".into()]).unwrap();
    let root = reqwest::Certificate::from_pem(certificate.cert.pem().as_bytes()).unwrap();
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.cert.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(certificate.signing_key.serialize_der().into()),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    f.state.http_client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::https(format!("http://{}", listener.local_addr().unwrap())).unwrap())
        .add_root_certificate(root)
        .build()
        .unwrap();
    let sent = Arc::new(AtomicUsize::new(0));
    let received = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let send_count = sent.clone();
    let captured = received.clone();
    let response_gate = release.clone();
    let telegram = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut connect = Vec::new();
            while !connect.ends_with(b"\r\n\r\n") {
                connect.push(socket.read_u8().await.unwrap());
            }
            assert!(connect.starts_with(b"CONNECT api.telegram.org:443 "));
            socket
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .unwrap();
            let mut stream = acceptor.accept(socket).await.unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                headers.push(stream.read_u8().await.unwrap());
            }
            let headers = String::from_utf8(headers).unwrap();
            assert!(headers.starts_with("POST /bottest-token/sendMessage "));
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            let mut bytes = vec![0; length];
            stream.read_exact(&mut bytes).await.unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(payload["chat_id"], 123);
            assert!(
                payload["text"]
                    .as_str()
                    .unwrap()
                    .contains("/assistant/approvals")
            );
            assert!(payload["reply_markup"].is_null(), "link-only review");
            let first = send_count.fetch_add(1, Ordering::SeqCst) == 0;
            captured.notify_one();
            if first {
                response_gate.notified().await;
            }
            let body = r#"{"ok":true,"result":{"message_id":1}}"#;
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
        }
    });

    // Monitor only the request-serving client. Test inspection/decisions use
    // the original client, so their reads cannot masquerade as waiter polls.
    let original_db = f.state.db.clone();
    let database = original_db.name().to_owned();
    let polls = Arc::new(Mutex::new(Vec::new()));
    let observed = polls.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && event.db == database
            && event.command.get_str("find") == Ok("approval_requests")
            && !event.command.contains_key("autocommit")
            && event
                .command
                .get_document("filter")
                .is_ok_and(|filter| filter.len() == 1 && filter.get_str("_id").is_ok())
        {
            observed.lock().unwrap().push(std::time::Instant::now());
        }
    }));
    f.state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(original_db.name());
    let (app, request) = connected_request(&f, "slug").await;
    f.state.db = original_db;
    let waiter = tokio::spawn(app.oneshot(request));
    tokio::time::timeout(std::time::Duration::from_secs(5), received.notified())
        .await
        .unwrap();
    let pending = row(&f).await;
    assert!(
        v2::proposal(&pending)
            .unwrap()
            .notification_claimed_at
            .is_some()
    );
    assert!(
        polls.lock().unwrap().is_empty(),
        "notification precedes first wait poll; no sweep invoked"
    );
    assert!(!waiter.is_finished());
    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        while polls.lock().unwrap().len() < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    {
        let polls = polls.lock().unwrap();
        assert!(
            polls[1].duration_since(polls[0]) >= std::time::Duration::from_millis(900),
            "legacy 1000ms cadence"
        );
    }
    let (one, two) = tokio::join!(v2::sweep(&f.state), v2::sweep(&f.state));
    one.unwrap();
    two.unwrap();
    assert_eq!(
        sent.load(Ordering::SeqCst),
        1,
        "sweep shares the inline notification CAS"
    );
    let mut decision = choice(&pending, "operation");
    decision.approved = false;
    decision.choice_id = None;
    decision.choice_digest = None;
    proposals::decide(&f.state, &f.owner, &pending.id, &decision)
        .await
        .unwrap();
    let response = waiter.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    telegram.abort();
    f.cleanup().await;
}

#[tokio::test]
async fn permission_mcp_receipt_uses_trusted_tool_outcome_without_changing_response() {
    for is_error in [true, false] {
        let f = Fixture::new("proposal_v2_mcp_outcome").await;
        f.enroll(true).await;
        call(&f, "/proxy/s/execution-fixture/status", "mcp-outcome").await;
        let requested = row(&f).await;
        let approved = proposals::decide(
            &f.state,
            &f.owner,
            &requested.id,
            &choice(&requested, "operation"),
        )
        .await
        .unwrap();
        let (resume, _) = v2::claim_for_test(&f.state, &approved)
            .await
            .unwrap()
            .unwrap();
        f.state
            .db
            .collection::<Document>("approval_requests")
            .update_one(
                doc! {"_id": &approved.id, "execution_proposal.v2.claim_id": &resume.claim_id},
                doc! {"$set": {"execution_proposal.v2.state": "dispatched"}},
            )
            .await
            .unwrap();
        let original =
            json!({"jsonrpc":"2.0","id":1,"result":{"isError":is_error,"content":[]}}).to_string();
        let mut response = axum::response::Response::new(Body::from(original.clone()));
        response
            .extensions_mut()
            .insert(nyxid_permissions::server::McpToolOutcome { is_error });
        let response = v2::complete_response_for_test(&f.state, &approved, &resume, response)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap()
                .as_ref(),
            original.as_bytes(),
        );
        let completed = row(&f).await;
        assert_eq!(v2::proposal(&completed).unwrap().state, "completed");
        assert_eq!(
            completed.activity_outcome.as_deref(),
            Some(if is_error { "failed" } else { "completed" })
        );
        let view = v2::readback(&f.state, &f.auth().await, &completed)
            .await
            .unwrap();
        assert_eq!(view.result.unwrap().body, original);
        f.cleanup().await;
    }
}
