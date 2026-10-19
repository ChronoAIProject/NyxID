//! Real tools/call, REST proxy, and node-wire regression for repository paths.
use super::*;
use crate::services::{catalog_spec_registry, mcp_service, openapi_parser};

async fn github_fixture() -> Fixture {
    let f = Box::pin(Fixture::new()).await;
    let spec = catalog_spec_registry::spec_for_key("github").unwrap();
    let endpoint = openapi_parser::parse_hosted_openapi_spec_value(&spec)
        .unwrap()
        .into_iter()
        .find(|op| op.name == "get_file_contents")
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("service_endpoints")
        .update_one(
            doc! {"service_id": &f.catalog},
            doc! {"$set": {
                "name": endpoint.name, "method": endpoint.method, "path": endpoint.path,
                "parameters": bson::to_bson(&endpoint.parameters).unwrap(),
            }},
        )
        .await
        .unwrap();
    f
}

async fn request_path(f: &Fixture, path: &str, mcp: bool, universal: bool) -> Value {
    let args = json!({"owner":"ChronoAIProject", "repo":"NyxID", "path":path});
    let params = if universal {
        json!({"name":"nyx__call_tool", "arguments":{
            "tool_name":"sandbox-alias__get_file_contents", "arguments_json":args.to_string()}})
    } else {
        json!({"name":"sandbox-alias__get_file_contents", "arguments":args})
    };
    let mut request = Request::builder()
        .method(if mcp { "POST" } else { "GET" })
        .uri(if mcp {
            "/mcp".into()
        } else {
            format!("/api/v1/proxy/s/sandbox-alias/repos/ChronoAIProject/NyxID/contents/{path}")
        })
        .body(if mcp {
            Body::from(
                json!({"jsonrpc":"2.0", "id":1,
            "method":"tools/call", "params":params})
                .to_string(),
            )
        } else {
            Body::empty()
        })
        .unwrap();
    *request.headers_mut() = f.key_headers();
    request
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    let response = Box::pin(f.router().oneshot(request)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap()).unwrap()
}

#[tokio::test]
async fn mcp_proxy_github_multisegment_paths_reach_mock_and_refuse_traversal() {
    let f = Box::pin(github_fixture()).await;
    for path in [
        "",
        "backend",
        "backend/src",
        "backend/src/services/mcp_service.rs",
    ] {
        for (mcp, universal) in [(false, false), (true, false), (true, true)] {
            let response = Box::pin(request_path(&f, path, mcp, universal)).await;
            if mcp {
                assert_eq!(response["result"]["isError"], false, "{response}");
            }
            assert_eq!(
                f.paths.lock().unwrap().pop().unwrap(),
                format!("/repos/ChronoAIProject/NyxID/contents/{path}")
            );
        }
    }
    for path in ["../x", "a/../b", "a//b", "a/%2e%2e/b", "a/%2Fb", "a\\b"] {
        for universal in [false, true] {
            let response = Box::pin(request_path(&f, path, true, universal)).await;
            assert_eq!(response["result"]["isError"], true, "{response}");
        }
    }
    assert!(
        f.paths.lock().unwrap().is_empty(),
        "invalid paths never reach GitHub"
    );

    // A remote producer's standard allowReserved flag uses the same wire path.
    let row = f
        .state
        .db
        .collection::<bson::Document>("service_endpoints")
        .find_one(doc! {"service_id":&f.catalog})
        .await
        .unwrap()
        .unwrap();
    let mut parameters: Value = bson::from_bson(row.get("parameters").unwrap().clone()).unwrap();
    let path = parameters
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|p| p["name"] == "path")
        .unwrap();
    path.as_object_mut()
        .unwrap()
        .remove("x-nyxid-path-segments");
    path["allowReserved"] = json!(true);
    f.state
        .db
        .collection::<bson::Document>("service_endpoints")
        .update_one(
            doc! {"_id":row.get_str("_id").unwrap()},
            doc! {"$set":{"parameters":bson::to_bson(&parameters).unwrap()}},
        )
        .await
        .unwrap();
    let response = Box::pin(request_path(&f, "backend/src", true, false)).await;
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert_eq!(
        f.paths.lock().unwrap().pop().unwrap(),
        "/repos/ChronoAIProject/NyxID/contents/backend/src"
    );
}

#[tokio::test]
async fn mcp_proxy_github_node_routing_preserves_the_same_multisegment_path() {
    let f = Box::pin(github_fixture()).await;
    f.state
        .db
        .collection::<UserService>(crate::models::user_service::COLLECTION_NAME)
        .update_one(doc! {"_id":&f.service}, doc! {"$set":{"node_id":&f.node}})
        .await
        .unwrap();
    let responder = super::proxy_parity::node_responder(&f, 6).await;
    for path in ["backend/src", "backend/src/services/mcp_service.rs"] {
        for (mcp, universal) in [(false, false), (true, false), (true, true)] {
            let response = Box::pin(request_path(&f, path, mcp, universal)).await;
            if mcp {
                assert_eq!(response["result"]["isError"], false, "{response}");
            }
            assert_eq!(
                f.paths
                    .lock()
                    .unwrap()
                    .pop()
                    .unwrap()
                    .trim_start_matches('/'),
                format!("repos/ChronoAIProject/NyxID/contents/{path}")
            );
        }
    }
    responder.await.unwrap();
}

#[tokio::test]
async fn mcp_proxy_approvals_path_and_digests_match_published_contract() {
    let f = Box::pin(github_fixture()).await;
    let catalog = mcp_service::load_operation_catalog(
        &f.state.db,
        f.state.node_ws_manager.as_ref(),
        &f.owner,
        mcp_service::NodeScope::Unrestricted,
        mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    let service = catalog
        .services
        .iter()
        .find(|s| s.service_id == f.service)
        .unwrap();
    let endpoint = &service.endpoints[0];
    let args = json!({"path":"backend/src", "repo":"NyxID", "owner":"ChronoAIProject"});
    let published_contract = mcp_service::endpoint_contract_digest(endpoint);
    let published_digest = mcp_service::exact_operation_digest_from_parts(
        &f.service,
        &endpoint.endpoint_id,
        &published_contract,
        &args,
    );
    for idempotency in [None, Some(uuid::Uuid::new_v4().to_string())] {
        let exact = mcp_service::prepare_exact_proxy_tool_call(
            service,
            endpoint,
            &args,
            idempotency.as_deref(),
        )
        .unwrap();
        let mcp = mcp_service::prepare_proxy_tool_call(service, endpoint, &args).unwrap();
        assert_eq!(
            exact.canonical_path().unwrap(),
            mcp.canonical_path().unwrap()
        );
        assert_eq!(
            mcp.operation_descriptor().resource.as_deref(),
            Some("/repos/ChronoAIProject/NyxID/contents/backend/src")
        );
        assert_eq!(
            mcp_service::exact_operation_digest(&f.service, endpoint, &args),
            published_digest
        );
    }
    // Serialization and key order do not drift; removing the opt-in does.
    let reordered: Value =
        serde_json::from_str(r#"{"owner":"ChronoAIProject","repo":"NyxID","path":"backend/src"}"#)
            .unwrap();
    assert_eq!(
        mcp_service::exact_operation_digest(&f.service, endpoint, &reordered),
        published_digest
    );
    let mut old = (*endpoint).clone();
    for p in old.parameters.as_mut().unwrap().as_array_mut().unwrap() {
        p.as_object_mut().unwrap().remove("x-nyxid-path-segments");
    }
    assert_ne!(
        mcp_service::endpoint_contract_digest(&old),
        published_contract
    );
    let (_, old_path, _, _, _) = mcp_service::build_proxy_args(&old, &args).unwrap();
    assert!(old_path.ends_with("backend%2Fsrc"));
    assert!(crate::services::proxy_service::validate_requested_proxy_path(&old_path).is_err());
}

async fn exact_request(f: &Fixture, path: &str, method: &str, body: Value) -> Value {
    use crate::handlers::exact_service_approvals as routes;
    let router = Router::new()
        .route("/exact", post(routes::create_request))
        .route(
            "/exact/{request_id}",
            axum::routing::get(routes::observe_request),
        )
        .route("/exact/{request_id}/redeem", post(routes::redeem_request))
        .layer(Extension(BillingRoutePolicy::Metered(BillingIngress::Mcp)))
        .with_state(f.state.clone());
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::from(body.to_string()))
        .unwrap();
    *request.headers_mut() = f.key_headers();
    request
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    let response = Box::pin(router.oneshot(request)).await.unwrap();
    let status = response.status();
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 32768).await.unwrap()).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn mcp_proxy_approvals_multisegment_redeem_keeps_authority_fences() {
    use crate::models::approval_request::{ApprovalRequest, COLLECTION_NAME};
    use crate::services::approval_service;
    let f = Box::pin(github_fixture()).await;
    let catalog = mcp_service::load_operation_catalog(
        &f.state.db,
        f.state.node_ws_manager.as_ref(),
        &f.owner,
        mcp_service::NodeScope::Unrestricted,
        mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    let service = catalog
        .services
        .iter()
        .find(|s| s.service_id == f.service)
        .unwrap();
    let endpoint = &service.endpoints[0];
    Box::pin(approval_service::set_service_approval_config(
        &f.state.db,
        &f.owner,
        &f.catalog,
        "Mock GitHub",
        Some(true),
        None,
        None,
        None,
    ))
    .await
    .unwrap();
    let args = json!({"path":"backend/src", "owner":"ChronoAIProject", "repo":"NyxID"});
    let digest = mcp_service::exact_operation_digest(&f.service, endpoint, &args);
    let created = Box::pin(exact_request(&f, "/exact", "POST", json!({
        "user_service_id":f.service, "endpoint_id":endpoint.endpoint_id,
        "catalog_digest":mcp_service::operation_catalog_digest(&catalog.services),
        "exact_view_digest":mcp_service::exact_operation_view_digest(&mcp_service::exact_operation_view(&catalog.services)),
        "endpoint_contract_digest":mcp_service::endpoint_contract_digest(endpoint),
        "operation_digest":digest, "operation_id":endpoint.endpoint_id,
        "idempotency_key":uuid::Uuid::new_v4().to_string(), "arguments":args,
    }))).await;
    assert_eq!(created["state"], "pending");
    let id = created["request_id"].as_str().unwrap();
    let row = f
        .state
        .db
        .collection::<ApprovalRequest>(COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    let authority = row
        .exact_service
        .as_ref()
        .unwrap()
        .execution_authority_binding
        .clone()
        .unwrap();
    Box::pin(approval_service::process_decision(
        &f.state.db,
        &f.state.config,
        &f.state.http_client,
        None,
        None,
        id,
        true,
        None,
        None,
        "web",
    ))
    .await
    .unwrap();
    let observed = Box::pin(exact_request(
        &f,
        &format!("/exact/{id}"),
        "GET",
        Value::Null,
    ))
    .await;
    assert_eq!(observed["state"], "approved", "{observed}");
    let result = Box::pin(exact_request(
        &f,
        &format!("/exact/{id}/redeem"),
        "POST",
        created.clone(),
    ))
    .await;
    assert_eq!(result["state"], "redeemed", "{result}");
    assert_eq!(result["operation_digest"], digest);
    assert_eq!(
        f.paths.lock().unwrap().pop().unwrap(),
        "/repos/ChronoAIProject/NyxID/contents/backend/src"
    );
    let row = f
        .state
        .db
        .collection::<ApprovalRequest>(COLLECTION_NAME)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap();
    let after = row
        .exact_service
        .as_ref()
        .unwrap()
        .execution_authority_binding
        .as_ref()
        .unwrap();
    assert_eq!(after.digest, authority.digest);
    assert_eq!(after.projection_version, authority.projection_version);
}
