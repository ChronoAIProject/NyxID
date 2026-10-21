use super::*;
use crate::{
    models::{downstream_service::SshServiceConfig, ssh_auth_mode::SshAuthMode},
    services::machine_integration_tests::{node, ungranted_node_named},
};
use axum::{
    body::{Body, to_bytes},
    extract::{ConnectInfo, Extension, Path, State},
    http::Request,
};

pub(super) async fn ssh_fixture(
    f: &Fixture,
    owner: &str,
) -> (String, String, crate::models::node::Node) {
    let catalog = catalog(f).await;
    let n = node(f, owner).await;
    let config = SshServiceConfig {
        ca_context_version: None,
        host: "10.0.0.5".into(),
        port: 22,
        ssh_auth_mode: SshAuthMode::NodeKey,
        certificate_auth_enabled: false,
        certificate_ttl_minutes: 30,
        allowed_principals: vec!["ubuntu".into()],
        ca_private_key_encrypted: None,
        ca_public_key: None,
    };
    f.state.db.collection::<Document>("downstream_services").update_one(doc! {"_id":&catalog},doc! {"$set":{"service_type":"ssh","ssh_config":bson::to_bson(&config).unwrap(),"inference":bson::Bson::Null}}).await.unwrap();
    let id = connection(f, owner, "ssh-exact", Some(&catalog)).await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"service_type":"ssh","ssh_auth_mode":"node_key","node_id":&n.id}},
        )
        .await
        .unwrap();
    (catalog, id, n)
}
pub(super) async fn ssh(
    f: &Fixture,
    id: &str,
    ack: Option<&str>,
) -> AppResult<axum::response::Response> {
    let mut auth = f.auth.clone();
    auth.assistant_chat = Some(std::sync::Arc::new(f.chat.clone()));
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/ssh/{id}/exec"))
        .header("content-type", "application/json");
    if let Some(ack) = ack {
        request = request.header(ACK_HEADER, ack);
    }
    Box::pin(crate::handlers::ssh_exec::ssh_exec(
        State(f.state.clone()),
        Extension(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::SshExec,
            ),
        ),
        auth,
        Default::default(),
        Path(id.to_owned()),
        ConnectInfo("127.0.0.1:12345".parse().unwrap()),
        request
            .body(Body::from(r#"{"command":"pwd","principal":"ubuntu"}"#))
            .unwrap(),
    ))
    .await
}

#[tokio::test]
async fn actual_ssh_http_exact_org_connection_asks_then_dispatches_node_key_and_preserves_live_scope()
 {
    let f = orchestrator_fixture("native_ssh_exact_http").await;
    mode(&f, AccessMode::Nyxmode).await;
    let owner = org(&f).await;
    let (catalog, id, n) = ssh_fixture(&f, &owner).await;
    let other = connection(&f, &owner, "other-same-catalog", Some(&catalog)).await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":other},
            doc! {"$set":{"service_type":"ssh","ssh_auth_mode":"cert","node_id":"wrong-node"}},
        )
        .await
        .unwrap();
    let (sender, mut outbound) = tokio::sync::mpsc::channel(8);
    test_utils::register_test_node_connection(&f.state, &n.id, sender).await;
    let manager = f.state.node_ws_manager.clone();
    let node = n.id.clone();
    let (observed, mut seen) = tokio::sync::mpsc::channel(8);
    let peer = tokio::spawn(async move {
        while let Some(message) = outbound.recv().await {
            let crate::services::node_ws_manager::NodeOutboundMessage::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            let Some(request_id) = value["request_id"].as_str() else {
                continue;
            };
            manager.deliver_ssh_exec_result(
                &node,
                crate::services::node_ws_manager::NodeSshExecResult {
                    request_id: request_id.to_owned(),
                    exit_code: 0,
                    stdout: "/workspace".into(),
                    stderr: String::new(),
                    duration_ms: 1,
                    timed_out: false,
                    error: None,
                    error_code: None,
                },
            );
            observed.send(value).await.unwrap();
        }
    });
    let response = ssh(&f, &id, None).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let card: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert!(seen.try_recv().is_err());
    let ack = allow(&f, card).await;
    let response = ssh(&f, &id, Some(&ack)).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    to_bytes(response.into_body(), 4096).await.unwrap();
    let request = seen.recv().await.unwrap();
    assert_eq!(request["service_slug"], "ssh-exact");
    assert!(request["private_key_pem"].is_null());
    f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.chat.api_key_id},
            doc! {"$set":{"allow_all_nodes":false,"allowed_node_ids":[]}},
        )
        .await
        .unwrap();
    mode(&f, AccessMode::Autopilot).await;
    let result = ssh(&f, &id, None).await;
    assert!(result.is_err());
    assert!(seen.try_recv().is_err());
    peer.abort();
}

#[tokio::test]
async fn native_ssh_held_projection_rejects_connection_replacement() {
    let f = orchestrator_fixture("native_ssh_held_projection").await;
    mode(&f, AccessMode::Autopilot).await;
    let (_, id, _) = ssh_fixture(&f, &f.owner).await;
    let row = crate::services::user_service_service::find_user_service_by_id(&f.state.db, &id)
        .await
        .unwrap()
        .unwrap();
    let config = crate::services::ssh_service::get_ssh_service(
        &f.state.db,
        row.catalog_service_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    let decision = crate::services::assistant_native_execution::ssh_gate(
        &f.state,
        Some(&f.chat),
        &row,
        &config,
        &json!({"command":"pwd"}),
    )
    .await
    .unwrap();
    let admission = decision.admission.unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&id}, doc! {"$set":{"ssh_auth_mode":"cert"}})
        .await
        .unwrap();
    assert!(admission.revalidate(&f.state.db, &id, None).await.is_err());
}

#[tokio::test]
async fn preferred_offline_primary_considers_only_authorized_online_fallback_nodes() {
    let f = orchestrator_fixture("eligible_fallback_nodes").await;
    let c = catalog(&f).await;
    let a = connection(&f, &f.owner, "a", Some(&c)).await;
    let primary = ungranted_node_named(&f, &f.owner, "primary").await;
    let fallback = ungranted_node_named(&f, &f.owner, "fallback").await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&a}, doc! {"$set":{"node_id":&primary.id}})
        .await
        .unwrap();
    crate::services::node_service::create_binding(&f.state.db, &f.owner, &fallback.id, &c)
        .await
        .unwrap();
    let (sender, _receiver) = tokio::sync::mpsc::channel(4);
    test_utils::register_test_node_connection(&f.state, &fallback.id, sender).await;
    f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.chat.api_key_id},
            doc! {"$set":{"allow_all_nodes":false,"allowed_node_ids":[&primary.id]}},
        )
        .await
        .unwrap();
    let row = crate::services::user_service_service::find_user_service_by_id(&f.state.db, &a)
        .await
        .unwrap()
        .unwrap();
    let operation = json!({"path":"/read"});
    assert!(
        super::super::availability::verified_unavailable(
            &f.state.db,
            &f.state.node_ws_manager,
            &f.chat,
            &row,
            &operation
        )
        .await
        .unwrap()
    );
    f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.chat.api_key_id},
            doc! {"$set":{"allowed_node_ids":[&primary.id,&fallback.id]}},
        )
        .await
        .unwrap();
    assert!(
        !super::super::availability::verified_unavailable(
            &f.state.db,
            &f.state.node_ws_manager,
            &f.chat,
            &row,
            &operation
        )
        .await
        .unwrap()
    );
}

pub(super) async fn node_service_call(f: &Fixture, id: &str, mcp: bool) -> AppResult<u16> {
    if !mcp {
        let mut auth = f.auth.clone();
        auth.assistant_chat = Some(std::sync::Arc::new(f.chat.clone()));
        let row = crate::services::user_service_service::find_user_service_by_id(&f.state.db, id)
            .await?
            .unwrap();
        let mut request = Request::builder()
            .uri(format!("/api/v1/proxy/s/{}/read", row.slug))
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        );
        let response = Box::pin(crate::handlers::proxy::proxy_request_by_slug(
            State(f.state.clone()),
            auth,
            Default::default(),
            Path((row.slug, "read".into())),
            request,
        ))
        .await?;
        let status = response.status().as_u16();
        to_bytes(response.into_body(), 8192).await.unwrap();
        return Ok(status);
    }
    let services =
        mcp_service::load_user_tools_all(&f.state.db, &f.state.node_ws_manager, &f.owner).await?;
    let service = services
        .iter()
        .find(|service| service.service_id == id)
        .unwrap();
    let endpoint = service
        .endpoints
        .iter()
        .find(|endpoint| {
            endpoint.name == "read" && endpoint.method == "GET" && endpoint.path == "/read"
        })
        .expect("fixture publishes the exact GET /read endpoint");
    let prepared = mcp_service::prepare_proxy_tool_call(service, endpoint, &json!({}))?;
    let context = mcp_service::McpExecContext {
        assistant_chat: Some(&f.chat),
        response_body_limit: None,
        actor_user_id: None,
        caller_token: None,
        delegation_restrictions: Default::default(),
        attribution: None,
        org_agent_access: None,
        agent_owner: None,
        operation_scopes: None,
        api_key_id: Some(&f.chat.api_key_id),
        api_key_credential_id: None,
        allow_all_nodes: true,
        allowed_node_ids: &[],
    };
    let response = Box::pin(mcp_service::execute_tool_response(
        &f.state.http_client,
        &f.state.db,
        &f.state.encryption_keys,
        &f.state.node_ws_manager,
        &f.state.billing,
        &f.owner,
        &f.owner,
        service,
        endpoint,
        prepared,
        &f.state.jwt_keys,
        &f.state.config,
        &f.state.connection_expiry_notifier,
        &f.state.token_exchange_cache,
        &f.state.cloud_response_cache,
        &context,
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
    ))
    .await?;
    Ok(response.status)
}

#[tokio::test]
async fn actual_node_only_exhaustion_records_outage_only_after_all_routes_fail() {
    for mcp in [false, true] {
        for all_fail in [false, true] {
            let f = orchestrator_fixture("node_exhaustion_evidence").await;
            let c = catalog(&f).await;
            f.state
                .db
                .collection("service_endpoints")
                .insert_one(test_utils::test_service_endpoint(
                    &c, "read", "GET", "/read",
                ))
                .await
                .unwrap();
            // This connection uses a node-owned credential, not a catalog master key.
            f.state
                .db
                .collection::<Document>("downstream_services")
                .update_one(
                    doc! {"_id":&c},
                    doc! {"$set":{"requires_user_credential":true}},
                )
                .await
                .unwrap();
            let a = connection(&f, &f.owner, "node-first", Some(&c)).await;
            let b = connection(&f, &f.owner, "lower", Some(&c)).await;
            let first = ungranted_node_named(&f, &f.owner, "primary").await;
            let fallback = ungranted_node_named(&f, &f.owner, "fallback").await;
            let key = Uuid::new_v4().to_string();
            f.state.db.collection::<Document>("user_api_keys").insert_one(doc! {
                "_id":&key, "user_id":&f.owner, "label":"node only", "credential_type":"node_managed",
                "status":"active", "created_at":bson::DateTime::now(), "updated_at":bson::DateTime::now(),
            }).await.unwrap();
            f.state
                .db
                .collection::<Document>("user_services")
                .update_one(
                    doc! {"_id":&a},
                    doc! {"$set":{"node_id":&first.id,"api_key_id":key,"auth_method":"bearer"}},
                )
                .await
                .unwrap();
            crate::services::node_service::create_binding(&f.state.db, &f.owner, &fallback.id, &c)
                .await
                .unwrap();
            f.state.db.collection::<Document>("service_preferences").insert_one(doc! {
                "_id":&f.owner,"ordered":[&a,&b],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            }).await.unwrap();
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("lower result"))
                .mount(&server)
                .await;
            let lower =
                crate::services::user_service_service::find_user_service_by_id(&f.state.db, &b)
                    .await
                    .unwrap()
                    .unwrap();
            f.state
                .db
                .collection::<Document>("user_endpoints")
                .update_one(
                    doc! {"_id":lower.endpoint_id},
                    doc! {"$set":{"url":server.uri()}},
                )
                .await
                .unwrap();
            let mut peers = Vec::new();
            for (node, succeeds) in [(&first, false), (&fallback, !all_fail)] {
                let (sender, mut outbound) = tokio::sync::mpsc::channel(8);
                test_utils::register_test_node_connection(&f.state, &node.id, sender).await;
                let manager = f.state.node_ws_manager.clone();
                let node_id = node.id.clone();
                peers.push(tokio::spawn(async move {
                    while let Some(crate::services::node_ws_manager::NodeOutboundMessage::Text(
                        text,
                    )) = outbound.recv().await
                    {
                        let value: Value = serde_json::from_str(&text).unwrap();
                        let Some(request_id) = value["request_id"].as_str() else {
                            continue;
                        };
                        if succeeds {
                            manager.deliver_proxy_response(
                                &node_id,
                                crate::services::node_ws_manager::NodeProxyResponse {
                                    request_id: request_id.to_owned(),
                                    status: 200,
                                    headers: vec![],
                                    body: b"node result".to_vec(),
                                },
                            );
                        } else {
                            manager.deliver_proxy_error(
                                &node_id,
                                request_id,
                                "Credential absent",
                                502,
                                true,
                                Some("credential_missing"),
                            );
                        }
                    }
                }));
            }
            let result = node_service_call(&f, &a, mcp).await;
            let bucket = f
                .state
                .db
                .collection::<Document>(COLLECTION)
                .find_one(doc! {"kind":"unavailable_bucket"})
                .await
                .unwrap();
            if all_fail {
                assert!(
                    matches!(result, Err(AppError::NodeCredentialMissing(_))),
                    "mcp={mcp}: {result:?}"
                );
                let bucket = bucket.expect("all permitted routes failed before dispatch");
                assert_eq!(bucket.get_array("entries").unwrap().len(), 1);
                assert_eq!(node_service_call(&f, &b, mcp).await.unwrap(), 200);
                assert_eq!(server.received_requests().await.unwrap().len(), 1);
            } else {
                assert_eq!(result.unwrap(), 200);
                assert!(
                    bucket.is_none(),
                    "one failed node cannot exempt the whole connection"
                );
                assert_eq!(node_service_call(&f, &b, mcp).await.unwrap(), 409);
                assert!(server.received_requests().await.unwrap().is_empty());
            }
            for peer in peers {
                peer.abort();
            }
        }
    }
}
