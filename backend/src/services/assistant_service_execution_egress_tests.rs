use super::*;
use axum::{
    Json, Router,
    routing::{get, post},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

async fn grant_service(f: &mut Fixture, id: &str) {
    f.auth.allowed_service_ids.push(id.to_owned());
    f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.chat.api_key_id},
            doc! {"$addToSet":{"allowed_service_ids":id}},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn actual_assistant_proxy_without_settings_waits_for_web_approval_before_dispatch() {
    use crate::models::{
        approval_request::ApprovalRequest, notification_channel::NotificationChannel,
    };
    use axum::{
        body::Body,
        extract::{Path, State},
        http::Request,
    };
    let mut f = orchestrator_fixture("assistant_first_request_web_approval").await;
    mode(&f, AccessMode::Nyxmode).await;
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("approved dispatch"))
        .mount(&server)
        .await;
    let id = connected(&f.state.db, &f.owner, "first-approval", &server.uri()).await;
    grant_service(&mut f, &id).await;
    // Remove the explicit helper preference: no GET settings or channel exists
    // when the actual assistant-authenticated proxy handler receives this call.
    let channels = f
        .state
        .db
        .collection::<NotificationChannel>("notification_channels");
    channels
        .delete_many(doc! {"user_id": &f.owner})
        .await
        .unwrap();
    assert_eq!(
        channels
            .count_documents(doc! {"user_id": &f.owner})
            .await
            .unwrap(),
        0
    );
    assert!(
        crate::services::approval_service::user_requires_approval(
            &f.state.db,
            &f.owner,
            &crate::models::service_approval_config::ApprovalVerb::Write
        )
        .await
        .unwrap()
    );
    assert_eq!(
        channels
            .count_documents(doc! {"user_id": &f.owner})
            .await
            .unwrap(),
        0
    );

    let mut request = Request::builder()
        .method("POST")
        .uri("/api/v1/proxy/s/first-approval/perform")
        .body(Body::from("{}"))
        .unwrap();
    request.extensions_mut().insert(
        crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
            crate::services::billing::BillingIngress::Proxy,
        ),
    );
    let mut auth = f.auth.clone();
    auth.assistant_chat = Some(Arc::new(f.chat.clone()));
    let execution = Box::pin(crate::handlers::proxy::proxy_request_by_slug(
        State(f.state.clone()),
        auth,
        Default::default(),
        Path(("first-approval".into(), "perform".into())),
        request,
    ));
    let web_decision = async {
        let pending = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(pending) = f
                    .state
                    .db
                    .collection::<ApprovalRequest>("approval_requests")
                    .find_one(doc! {"user_id": &f.owner, "service_id": &id, "status": "pending"})
                    .await
                    .unwrap()
                {
                    break pending;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("actual proxy must persist a pending generic approval");
        assert_eq!((pending.expires_at - pending.created_at).num_seconds(), 300);
        assert!(pending.notification_channel.is_none());
        assert!(server.received_requests().await.unwrap().is_empty());
        let stored = channels
            .find_one(doc! {"user_id": &f.owner})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.approval_required, None);
        assert_eq!(stored.approval_timeout_secs, 300);
        let Json(result) = crate::handlers::approvals::decide_request(
            State(f.state.clone()),
            test_utils::test_auth_user(&f.owner),
            Path(pending.id.clone()),
            Default::default(),
            HeaderMap::new(),
            Json(crate::handlers::approvals::DecideRequest {
                approved: true,
                duration_sec: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(result.status, "approved");
        assert_eq!(
            crate::services::approval_service::get_request(&f.state.db, &pending.id)
                .await
                .unwrap()
                .decision_channel
                .as_deref(),
            Some("web")
        );
    };
    let (response, ()) = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        tokio::join!(execution, web_decision)
    })
    .await
    .expect("web decision must release the actual proxy execution");
    let response = response.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
        "approved dispatch"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn actual_http_consent_ignores_correlation_drift_but_binds_account_and_idempotency() {
    let mut f = orchestrator_fixture("http_consent_correlation").await;
    mode(&f, AccessMode::AskEveryTime).await;
    let owner = org(&f).await;
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("done"))
        .mount(&server)
        .await;
    let id = connected(&f.state.db, &owner, "correlated-action", &server.uri()).await;
    grant_service(&mut f, &id).await;
    f.state.db.collection::<Document>("user_services").update_one(doc! {"_id":&id},doc! {"$set":{
        "default_request_headers":[{"name":"x-account-id","value":"account-a","overridable":true}]
    }}).await.unwrap();
    async fn send(
        f: &Fixture,
        ack: Option<&str>,
        account: &str,
        idempotency: &str,
        trace: &str,
    ) -> axum::response::Response {
        use axum::{
            body::Body,
            extract::{Path, State},
            http::Request,
        };
        let mut builder = Request::builder()
            .method("POST")
            .uri("/api/v1/proxy/s/correlated-action/perform")
            .header("content-type", "application/json")
            .header("x-account-id", account)
            .header("idempotency-key", idempotency)
            .header("traceparent", trace);
        if let Some(ack) = ack {
            builder = builder.header(ACK_HEADER, ack);
        }
        let mut request = builder.body(Body::from("{\"action\":1}")).unwrap();
        request.extensions_mut().insert(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        );
        let mut auth = f.auth.clone();
        auth.assistant_chat = Some(Arc::new(f.chat.clone()));
        Box::pin(crate::handlers::proxy::proxy_request_by_slug(
            State(f.state.clone()),
            auth,
            Default::default(),
            Path(("correlated-action".into(), "perform".into())),
            request,
        ))
        .await
        .unwrap()
    }
    let first_trace = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let next_trace = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let proposed = send(&f, None, "account-a", "action-1", first_trace).await;
    assert_eq!(proposed.status(), 409);
    let bytes = axum::body::to_bytes(proposed.into_body(), 8192)
        .await
        .unwrap();
    let card: Value = serde_json::from_slice(&bytes).unwrap();
    let ack = card["acknowledgement_id"].as_str().unwrap();
    acks::decide(&f.state.db, &f.owner, &f.row.id, ack, true)
        .await
        .unwrap();
    for (account, idempotency) in [("account-b", "action-1"), ("account-a", "action-2")] {
        let refused = send(&f, Some(ack), account, idempotency, next_trace).await;
        assert_eq!(refused.status(), 409);
        axum::body::to_bytes(refused.into_body(), 8192)
            .await
            .unwrap();
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    // The public handler generates another x-request-id for this invocation.
    let allowed = send(&f, Some(ack), "account-a", "action-1", next_trace).await;
    let status = allowed.status();
    let bytes = axum::body::to_bytes(allowed.into_body(), 8192)
        .await
        .unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    let calls = server.received_requests().await.unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].headers["x-account-id"], "account-a");
    assert_eq!(calls[0].headers["idempotency-key"], "action-1");
    assert!(!calls[0].headers.contains_key(ACK_HEADER));
}

#[tokio::test]
async fn actual_proxy_and_mcp_delayed_exchange_revocation_prevents_operation_send() {
    for mcp in [false, true] {
        for drift in ["stop", "replace"] {
            let mut f = orchestrator_fixture("final_send_exchange").await;
            mode(&f, AccessMode::Autopilot).await;
            crate::billing_integration_tests::create_usage_index(&f.state.db).await;
            f.state
                .db
                .collection("billing_rate_cache")
                .insert_one(crate::billing_integration_tests::rate(
                    "platform_requests",
                    Utc::now(),
                ))
                .await
                .unwrap();
            f.state.billing = Arc::new(crate::billing_integration_tests::billing_service(
                &f.state.db,
                Arc::new(crate::billing_integration_tests::FakeLago::default()),
                10,
            ));
            let owner = org(&f).await;
            let catalog = catalog(&f).await;
            f.state
                .db
                .collection("service_endpoints")
                .insert_one(test_utils::test_service_endpoint(
                    &catalog, "read", "GET", "/read",
                ))
                .await
                .unwrap();
            let id = connection(&f, &owner, "exact-exchange", Some(&catalog)).await;
            grant_service(&mut f, &id).await;
            let entered = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let effects = Arc::new(AtomicUsize::new(0));
            let app = Router::new()
                .route(
                    "/token",
                    post({
                        let entered = entered.clone();
                        let release = release.clone();
                        move || {
                            let entered = entered.clone();
                            let release = release.clone();
                            async move {
                                entered.notify_one();
                                release.notified().await;
                                Json(json!({"token":"test-exchanged-token","expires_in":60}))
                            }
                        }
                    }),
                )
                .route(
                    "/read",
                    get({
                        let effects = effects.clone();
                        move || {
                            effects.fetch_add(1, Ordering::SeqCst);
                            async { "operation" }
                        }
                    }),
                );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let row =
                crate::services::user_service_service::find_user_service_by_id(&f.state.db, &id)
                    .await
                    .unwrap()
                    .unwrap();
            f.state
                .db
                .collection::<Document>("user_endpoints")
                .update_one(doc! {"_id":row.endpoint_id}, doc! {"$set":{"url":url}})
                .await
                .unwrap();
            let key = Uuid::new_v4().to_string();
            let encrypted = f.state.encryption_keys.encrypt(b"{}").await.unwrap();
            f.state.db.collection::<Document>("user_api_keys").insert_one(doc!{
                "_id":&key,"user_id":&owner,"label":"exchange","credential_type":"api_key","status":"active",
                "credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
                "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            }).await.unwrap();
            f.state
                .db
                .collection::<Document>("user_services")
                .update_one(
                    doc! {"_id":&id},
                    doc! {"$set":{"api_key_id":&key,"auth_method":"token_exchange"}},
                )
                .await
                .unwrap();
            let config = crate::models::downstream_service::TokenExchangeConfig {
                endpoint: "{base_url}/token".into(),
                request_encoding: "json".into(),
                request_template: json!({}),
                token_response_path: "token".into(),
                ttl_response_path: Some("expires_in".into()),
                default_ttl_secs: 60,
                injection: "bearer".into(),
                error_code_path: None,
                error_message_path: None,
                credential_fields: vec![],
            };
            let billing = crate::models::service_billing::ServiceBilling {
                platform_billable: true,
                platform_charge_nyxid_credentials_only: false,
                ..Default::default()
            };
            f.state.db.collection::<Document>("downstream_services").update_one(doc!{"_id":&catalog},doc!{"$set":{"token_exchange_config":bson::to_bson(&config).unwrap(),"inference":bson::Bson::Null,"billing":bson::to_bson(&billing).unwrap()}}).await.unwrap();
            let mut request = Box::pin(super::native::node_service_call(&f, &id, mcp));
            tokio::select! {
                result=&mut request => panic!("operation finished before delayed exchange: {result:?}"),
                ()=entered.notified()=>{},
                ()=tokio::time::sleep(std::time::Duration::from_secs(10))=>panic!("exchange never started"),
            }
            use crate::models::{
                billing_wallet::BillingWallet,
                credits::Credits,
                usage_meter::{UsageMeterRow, UsageStatus},
            };
            let held = f
                .state
                .db
                .collection::<UsageMeterRow>("usage_meter")
                .find_one(doc! {"user_service_id":&id})
                .await
                .unwrap()
                .expect("billable request reserved");
            assert_eq!(held.status, UsageStatus::Forwarded);
            assert!(!held.released);
            assert!(held.reserved_credits > Credits::ZERO);
            if drift == "stop" {
                f.state
                    .db
                    .collection::<Document>("assistant_conversations")
                    .update_one(
                        doc! {"_id":&f.row.id},
                        doc! {"$set":{"active_turn.stop_requested":true}},
                    )
                    .await
                    .unwrap();
            } else {
                let before =
                    crate::services::user_api_key_service::get_api_key(&f.state.db, &owner, &key)
                        .await
                        .unwrap()
                        .credential_epoch;
                f.state.db.collection::<Document>("user_api_keys").update_one(doc!{"_id":&key},vec![doc!{"$set":{"credential_epoch":{"$add":[{"$ifNull":["$credential_epoch",1]},1]}}}]).await.unwrap();
                let after =
                    crate::services::user_api_key_service::get_api_key(&f.state.db, &owner, &key)
                        .await
                        .unwrap()
                        .credential_epoch;
                assert_eq!(
                    after,
                    before + 1,
                    "replace must change actual credential generation"
                );
            }
            release.notify_one();
            let result = tokio::time::timeout(std::time::Duration::from_secs(10), request)
                .await
                .unwrap();
            assert!(result.is_err(), "mcp={mcp},drift={drift}: {result:?}");
            assert_eq!(effects.load(Ordering::SeqCst), 0);
            let failed = f
                .state
                .db
                .collection::<UsageMeterRow>("usage_meter")
                .find_one(doc! {"_id":&held.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(failed.status, UsageStatus::Failed);
            assert!(failed.released, "final-send refusal must release its hold");
            assert_eq!(
                failed.last_error.as_deref(),
                Some("assistant_authority_changed")
            );
            let wallet = f
                .state
                .db
                .collection::<BillingWallet>("billing_wallet")
                .find_one(doc! {"_id":held.wallet_id.unwrap()})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(wallet.reserved_credits, Credits::ZERO);
            assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>(COLLECTION)
                    .count_documents(doc! {"kind":"unavailable_bucket"})
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>("assistant_acknowledgements")
                    .count_documents(doc! {"status":"allowed","decided_by":"access_mode"})
                    .await
                    .unwrap(),
                0
            );
            server.abort();
        }
    }
}

#[tokio::test]
async fn dispatched_node_read_then_refused_fallback_keeps_real_dispatch_and_billing_hold() {
    use crate::models::{
        billing_wallet::BillingWallet,
        credits::Credits,
        usage_meter::{UsageMeterRow, UsageStatus},
    };
    use crate::services::machine_integration_tests::ungranted_node_named;
    let mut f = orchestrator_fixture("refused_read_fallback_billing").await;
    mode(&f, AccessMode::Autopilot).await;
    crate::billing_integration_tests::create_usage_index(&f.state.db).await;
    f.state
        .db
        .collection("billing_rate_cache")
        .insert_one(crate::billing_integration_tests::rate(
            "platform_requests",
            Utc::now(),
        ))
        .await
        .unwrap();
    f.state.billing = Arc::new(crate::billing_integration_tests::billing_service(
        &f.state.db,
        Arc::new(crate::billing_integration_tests::FakeLago::default()),
        10,
    ));
    let catalog = catalog(&f).await;
    let id = connection(&f, &f.owner, "read-fallback", Some(&catalog)).await;
    grant_service(&mut f, &id).await;
    let billing = crate::models::service_billing::ServiceBilling {
        platform_billable: true,
        platform_charge_nyxid_credentials_only: false,
        ..Default::default()
    };
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&catalog},
            doc! {"$set":{"inference":bson::Bson::Null,"billing":bson::to_bson(&billing).unwrap()}},
        )
        .await
        .unwrap();
    let first = ungranted_node_named(&f, &f.owner, "first").await;
    let next = ungranted_node_named(&f, &f.owner, "next").await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&id}, doc! {"$set":{"node_id":&first.id}})
        .await
        .unwrap();
    crate::services::node_service::create_binding(&f.state.db, &f.owner, &next.id, &catalog)
        .await
        .unwrap();
    let (sender, mut outbound) = tokio::sync::mpsc::channel(8);
    test_utils::register_test_node_connection(&f.state, &first.id, sender).await;
    let (next_sender, mut next_outbound) = tokio::sync::mpsc::channel(8);
    test_utils::register_test_node_connection(&f.state, &next.id, next_sender).await;
    let manager = f.state.node_ws_manager.clone();
    let db = f.state.db.clone();
    let conversation = f.row.id.clone();
    let node = first.id.clone();
    let peer = tokio::spawn(async move {
        while let Some(crate::services::node_ws_manager::NodeOutboundMessage::Text(text)) =
            outbound.recv().await
        {
            let value: Value = serde_json::from_str(&text).unwrap();
            let Some(request) = value["request_id"].as_str() else {
                continue;
            };
            db.collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":conversation},
                    doc! {"$set":{"active_turn.stop_requested":true}},
                )
                .await
                .unwrap();
            // A retryable transport error without credential_missing means the
            // node may have sent the GET. Read retry is safe, zero-send is false.
            manager.deliver_proxy_error(
                &node,
                request,
                "Downstream transport lost",
                502,
                true,
                None,
            );
            return;
        }
        panic!("first node received no operation");
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        super::native::node_service_call(&f, &id, false),
    )
    .await
    .expect("read and refused fallback must settle");
    assert!(result.is_err(), "revoked fallback must refuse: {result:?}");
    tokio::time::timeout(std::time::Duration::from_secs(2), peer)
        .await
        .expect("first node must have received the read")
        .unwrap();
    assert!(
        next_outbound.try_recv().is_err(),
        "revoked turn must not reach next node"
    );
    let usage = f
        .state
        .db
        .collection::<UsageMeterRow>("usage_meter")
        .find_one(doc! {"user_service_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(usage.status, UsageStatus::Forwarded);
    assert!(usage.forwarded);
    assert!(
        !usage.released,
        "a real prior dispatch must retain uncertain accounting"
    );
    let wallet = f
        .state
        .db
        .collection::<BillingWallet>("billing_wallet")
        .find_one(doc! {"_id":usage.wallet_id.unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert!(wallet.reserved_credits > Credits::ZERO);
    assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
}

#[tokio::test]
async fn native_ssh_final_refusal_clears_only_presend_meter_bookkeeping() {
    use crate::models::usage_meter::{UsageMeterRow, UsageStatus};
    let mut f = orchestrator_fixture("native_ssh_presend_meter").await;
    mode(&f, AccessMode::Autopilot).await;
    let (_, id, node) = super::native::ssh_fixture(&f, &f.owner).await;
    let (sender, mut outbound) = tokio::sync::mpsc::channel(8);
    test_utils::register_test_node_connection(&f.state, &node.id, sender).await;
    let gate = Arc::new(crate::services::billing::SettlementGate {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
        fail: false,
        used: false.into(),
    });
    let mut billing = crate::billing_integration_tests::billing_service(
        &f.state.db,
        Arc::new(crate::billing_integration_tests::FakeLago::default()),
        10,
    );
    billing.before_send_gate = Some(gate.clone());
    f.state.billing = Arc::new(billing);
    let mut request = Box::pin(super::native::ssh(&f, &id, None));
    tokio::select! {
        result=&mut request => panic!("native request ended before final fence: {result:?}"),
        ()=gate.entered.notified()=>{},
        ()=tokio::time::sleep(std::time::Duration::from_secs(10))=>panic!("native request did not mark forwarding"),
    }
    let held = f
        .state
        .db
        .collection::<UsageMeterRow>("usage_meter")
        .find_one(doc! {"service_slug":"ssh-exact"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.status, UsageStatus::Forwarded);
    f.state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
    gate.release.add_permits(1);
    assert!(request.await.is_err());
    assert!(
        outbound.try_recv().is_err(),
        "refused command must never reach node"
    );
    let failed = f
        .state
        .db
        .collection::<UsageMeterRow>("usage_meter")
        .find_one(doc! {"_id":held.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, UsageStatus::Failed);
    assert!(!failed.forwarded);
    assert!(failed.released);
    assert_eq!(
        failed.last_error.as_deref(),
        Some("assistant_authority_changed")
    );
}
