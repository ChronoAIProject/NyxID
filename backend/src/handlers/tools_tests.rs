use super::curation_tests::{fixture, request};
use crate::models::{
    downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService, OfferingKind},
    service_endpoint::{COLLECTION_NAME as ENDPOINTS, ServiceEndpoint},
};
use axum::http::StatusCode;
use mongodb::bson::doc;
use serde_json::json;

#[tokio::test]
async fn tool_publication_gates_all_discovery_and_proxy() {
    let mut f = fixture("tools_end_to_end", false).await;
    f.human_token = proxy_token(&f.state, &f.owner);
    f.service.offering_kind = OfferingKind::Tool;
    f.service.service_category = "internal".into();
    f.service.auth_method = "none".into();
    f.service.requires_user_credential = false;
    f.service.proxy_operation_policy = None;
    f.service.visibility = "public".into();
    f.state
        .db
        .collection::<DownstreamService>(CATALOG)
        .replace_one(doc! {"_id":&f.service.id}, &f.service)
        .await
        .unwrap();
    let path = format!("/api/v1/services/{}/endpoints", f.service.id);
    let (status, endpoint) = request(
        &f.state,
        "POST",
        &path,
        &f.human_token,
        Some(json!({"name":"draft_operation","method":"GET","path":"/draft"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{endpoint}");
    assert_eq!(endpoint["publication"], "draft");
    assert_eq!(endpoint["is_active"], false);
    let mcp_session = f
        .state
        .mcp_sessions
        .create_with_proxy_access(&f.owner, true, false)
        .await
        .unwrap()
        .unwrap();
    for (method, path, body) in [
        ("GET", "/api/v1/mcp/config", None),
        (
            "POST",
            "/mcp",
            Some(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})),
        ),
        (
            "POST",
            "/mcp",
            Some(
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"nyx__search_tools","arguments":{"query":f.service.slug}}}),
            ),
        ),
        (
            "POST",
            "/mcp",
            Some(
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nyx__discover_services","arguments":{}}}),
            ),
        ),
    ] {
        let (status, result) = if path == "/mcp" {
            mcp_request(&f.state, &f.human_token, &mcp_session, body.unwrap()).await
        } else {
            request(&f.state, method, path, &f.human_token, body).await
        };
        assert_eq!(status, StatusCode::OK, "{path}: {result}");
        assert!(!result.to_string().contains("draft_operation"), "{result}");
    }
    let (status, result) = request(
        &f.state,
        "GET",
        &format!("/api/v1/proxy/s/{}/draft", f.service.slug),
        &f.human_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{result}");
    assert_eq!(result["error_code"], 12600, "{result}");
    let (_,result)=mcp_request(&f.state,&f.human_token,&mcp_session,json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nyx__call_tool","arguments":{"tool_name":format!("{}__draft_operation",f.service.slug),"arguments_json":"{}"}}})).await;
    assert_eq!(result["result"]["isError"], true, "{result}");
    assert!(result.to_string().contains("12600"), "{result}");
    let (status, published) = request(
        &f.state,
        "POST",
        &format!("/api/v1/services/{}/publication", f.service.id),
        &f.human_token,
        Some(json!({"endpoint_names":["draft_operation"],"state":"published"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{published}");
    let rows = f
        .state
        .db
        .collection::<ServiceEndpoint>(ENDPOINTS)
        .find_one(doc! {"_id":endpoint["id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert!(rows.is_active);
    assert_eq!(rows.operation_generation, 2);
    f.state.db.drop().await.unwrap();
}

async fn mcp_request(
    state: &crate::AppState,
    bearer: &str,
    session: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    use tower::ServiceExt;
    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("authorization", format!("Bearer {bearer}"))
        .header("mcp-session-id", session)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    let response = super::curation_tests::router(state)
        .oneshot(req)
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn tool_twins_copy_contracts_as_drafts_without_mutating_source_or_copying_secrets() {
    let mut f = fixture("tool_twins", false).await;
    f.human_token = proxy_token(&f.state, &f.owner);
    f.service.auth_method = "header".into();
    f.service.auth_key_name = "xi-api-key".into();
    f.service.provider_config_id = Some("source-provider".into());
    f.service.credential_encrypted = vec![1, 2, 3];
    f.service.custom_user_agent = Some("NyxID tools".into());
    f.service.description = Some("Source description".into());
    f.service.proxy_operation_policy = None;
    f.state
        .db
        .collection::<DownstreamService>(CATALOG)
        .replace_one(doc! {"_id":&f.service.id}, &f.service)
        .await
        .unwrap();
    let (status, source_endpoint) = request(&f.state,"POST",&format!("/api/v1/services/{}/endpoints",f.service.id),&f.human_token,Some(json!({"name":"search","method":"GET","path":"/search","data_scope":"public","cost_class":"free"}))).await;
    assert_eq!(status, StatusCode::OK, "{source_endpoint}");
    let before = f
        .state
        .db
        .collection::<mongodb::bson::Document>(CATALOG)
        .find_one(doc! {"_id":&f.service.id})
        .await
        .unwrap()
        .unwrap();
    for reference in [&f.service.slug, &f.service.id] {
        let slug = format!("tools-{}", uuid::Uuid::new_v4());
        let (status, twin) = request(&f.state,"POST","/api/v1/services",&f.human_token,Some(json!({"twin_of_service_id":reference,"slug":slug,"supplier":"Vendor","description":"Override"}))).await;
        assert_eq!(status, StatusCode::OK, "{twin}");
        assert_eq!(twin["offering_kind"], "tool");
        assert_eq!(twin["service_category"], "internal");
        assert_eq!(twin["requires_user_credential"], false);
        assert_eq!(twin["provider_config_id"], serde_json::Value::Null);
        assert_eq!(twin["description"], "Override");
        assert_eq!(twin["auth_key_name"], "xi-api-key");
        assert_eq!(twin["custom_user_agent"], "NyxID tools");
        assert_eq!(twin["import_source"]["kind"], "catalog_twin");
        let imported_at = chrono::DateTime::parse_from_rfc3339(
            twin["import_source"]["imported_at"].as_str().unwrap(),
        )
        .unwrap();
        assert!(
            (chrono::Utc::now() - imported_at.with_timezone(&chrono::Utc))
                .num_seconds()
                .abs()
                < 60
        );
        assert_eq!(twin["import_source"]["reference"], f.service.slug);
        let (status, error) = request(
            &f.state,
            "GET",
            &format!("/api/v1/proxy/s/{slug}/search"),
            &f.human_token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
        assert_eq!(error["error_code"], 12600, "{error}");
        let id = twin["id"].as_str().unwrap();
        let stored = f
            .state
            .db
            .collection::<DownstreamService>(CATALOG)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        assert!(stored.credential_encrypted.is_empty());
        assert!(stored.platform_key.unwrap().enabled);
        let (_, endpoints) = request(
            &f.state,
            "GET",
            &format!("/api/v1/services/{id}/endpoints"),
            &f.human_token,
            None,
        )
        .await;
        assert_eq!(endpoints["endpoints"].as_array().unwrap().len(), 1);
        let copied = &endpoints["endpoints"][0];
        assert_eq!(copied["publication"], "draft");
        assert_eq!(copied["is_active"], false);
        let persisted = f
            .state
            .db
            .collection::<crate::models::service_endpoint::ServiceEndpoint>(
                crate::models::service_endpoint::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": copied["id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(persisted.operation_generation, 1);
        assert_ne!(copied["id"], source_endpoint["id"]);
        assert_eq!(copied["data_scope"], "public");
        let (status, cleared) = request(
            &f.state,
            "PUT",
            &format!("/api/v1/services/{id}"),
            &f.human_token,
            Some(json!({"supplier":null,"import_source":null})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cleared}");
        let stored = f
            .state
            .db
            .collection::<DownstreamService>(CATALOG)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        assert!(stored.supplier.is_none());
        assert!(stored.import_source.is_none());
    }
    assert_eq!(
        before,
        f.state
            .db
            .collection::<mongodb::bson::Document>(CATALOG)
            .find_one(doc! {"_id":&f.service.id})
            .await
            .unwrap()
            .unwrap()
    );
    let (_, source) = request(
        &f.state,
        "GET",
        &format!("/api/v1/services/{}/endpoints", f.service.id),
        &f.human_token,
        None,
    )
    .await;
    assert_eq!(source["endpoints"][0]["publication"], "published");
    for extra in [
        json!({"credential":"secret"}),
        json!({"provider_config_id":"provider"}),
        json!({"service_category":"connection"}),
    ] {
        let mut body = json!({"twin_of_service_id":f.service.slug,"slug":"tools-rejected"});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (status, error) = request(
            &f.state,
            "POST",
            "/api/v1/services",
            &f.human_token,
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    }
    f.state.db.drop().await.unwrap();
}

fn proxy_token(state: &crate::AppState, owner: &str) -> String {
    crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &uuid::Uuid::parse_str(owner).unwrap(),
        "openid profile proxy",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap()
}
