//! Durable specialist grants must not depend on temporary node availability.
use super::*;
use crate::{
    services::{
        assistant_agent_credential_service as credentials, assistant_team_service as team,
        feature_flag_service as flags, org_agent_tests::Fixture,
    },
    test_utils::{register_test_node_connection, test_auth_user, test_user},
};
use axum::{
    Json,
    extract::{Path, State},
};
use mongodb::bson::{self, doc};
use serde_json::{Value, json};

async fn save(
    f: &Fixture,
    actor: &str,
    agent: &str,
    service: &str,
) -> crate::errors::AppResult<Json<Value>> {
    Box::pin(crate::handlers::assistant_team::set_agent_grants(
        State(f.state.clone()),
        test_auth_user(actor),
        Path(agent.into()),
        Json(serde_json::from_value(json!({"services":[service],"account_read":false})).unwrap()),
    ))
    .await
}

async fn fixture(name: &str) -> (Fixture, String, String, String, String) {
    let mut f = Fixture::new(name).await;
    f.state.config.node_hmac_signing_enabled = false;
    flags::set_platform_override(
        &f.state.db,
        flags::NYXAGENT_ENGINE_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.admin,
    )
    .await
    .unwrap();
    let (agent, row) = f.create().await;
    let nyxbot = crate::services::assistant_authority_tests::new_orchestrator(
        &f.state,
        &f.admin,
        "Coordinate",
    )
    .await;
    crate::services::assistant_authority_tests::pin_assignment(
        &f.state.db,
        &f.admin,
        &row.id,
        Some(&nyxbot.id),
    )
    .await;
    let service = f.service(&f.org, "home-assistant").await;
    let node = uuid::Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<bson::Document>("nodes")
        .insert_one(doc! {
            "_id": &node, "user_id": &f.org, "name":"home-server", "status":"offline",
            "auth_token_hash":"test", "signing_secret_hash":"test", "metrics":{},
            "is_active":true, "created_at":now,"updated_at":now,
        })
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(doc! {"_id":&service}, doc! {"$set":{"node_id":&node}})
        .await
        .unwrap();
    (f, agent.id, row.id, service, node)
}

async fn call(f: &Fixture, thread: &str, name: &str, arguments: Value) -> Value {
    let key =
        credentials::load_for_conversation(&f.state.db, &f.state.encryption_keys, &f.admin, thread)
            .await
            .unwrap()
            .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", key.raw_key.parse().unwrap());
    let auth = authenticate_mcp(&f.state, &headers, false).await.unwrap();
    let request = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/call".into(),
        params: Some(json!({"name":"nyx__call_tool","arguments":{
            "tool_name":name,"arguments_json":arguments.to_string(),
        }})),
    };
    let response = Box::pin(handle_tools_call(
        &f.state,
        &auth,
        None,
        &request,
        false,
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
    ))
    .await;
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn assistant_team_offline_grant_saves_and_same_grant_executes_only_after_node_returns() {
    use crate::services::node_ws_manager::{NodeOutboundMessage, NodeProxyResponse};
    let (f, agent, thread, service, node) = fixture("assistant_team_offline_grant").await;
    let direct = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200))
        .expect(0)
        .mount(&direct)
        .await;
    f.state
        .db
        .collection::<bson::Document>("user_endpoints")
        .update_many(doc! {"user_id":&f.org}, doc! {"$set":{"url":direct.uri()}})
        .await
        .unwrap();
    let _ = save(&f, &f.admin, &agent, &service).await.unwrap();
    let stored = team::agent(&f.state.db, &f.admin, &agent).await.unwrap();
    assert_eq!(stored.grants.service_ids, vec![service.clone()]);
    let catalog = mcp_service::load_operation_catalog(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.org,
        mcp_service::NodeScope::Unrestricted,
        mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    assert!(catalog.services.iter().all(|row| row.service_id != service));
    let Json(keys) = crate::handlers::keys::list_keys_with_tool_bindings(
        f.state.clone(),
        test_auth_user(&f.admin),
        true,
    )
    .await
    .unwrap();
    let key = keys.keys.iter().find(|key| key.id == service).unwrap();
    assert_eq!(
        key.execution_unavailable_reason.as_deref(),
        Some("node home-server offline")
    );
    let result = call(
        &f,
        &thread,
        "home-assistant__request",
        json!({"method":"GET","path":"/status"}),
    )
    .await;
    assert_eq!(result["result"]["isError"], true, "{result}");
    let text = result["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("node_offline") && text.contains("home-server"),
        "{text}"
    );
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    register_test_node_connection(&f.state, &node, tx).await;
    f.state
        .db
        .collection::<bson::Document>("nodes")
        .update_one(
            doc! {"_id":&node},
            doc! {"$set":{"status":"online","last_heartbeat_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    let manager = f.state.node_ws_manager.clone();
    let node_id = node.clone();
    let responder = tokio::spawn(async move {
        let message = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let NodeOutboundMessage::Text(text) = message else {
            panic!("expected proxy request")
        };
        let message: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(message["type"], "proxy_request");
        manager.deliver_proxy_response(
            &node_id,
            NodeProxyResponse {
                request_id: message["request_id"].as_str().unwrap().into(),
                status: 200,
                headers: vec![("content-type".into(), "application/json".into())],
                body: br#"{"source":"node-ok"}"#.to_vec(),
            },
        );
    });
    let result = call(
        &f,
        &thread,
        "home-assistant__request",
        json!({"method":"GET","path":"/status"}),
    )
    .await;
    assert_eq!(result["result"]["isError"], false, "{result}");
    assert!(result.to_string().contains("node-ok"), "{result}");
    responder.await.unwrap();
    // Inventory heartbeat diagnostics must not override the dispatcher's live
    // owner/socket decision when computing the execution advisory.
    f.state.db.collection::<bson::Document>("nodes").update_one(doc! {"_id":&node},doc! {"$set":{"last_heartbeat_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::days(1))}}).await.unwrap();
    let Json(keys) = crate::handlers::keys::list_keys_with_tool_bindings(
        f.state.clone(),
        test_auth_user(&f.admin),
        true,
    )
    .await
    .unwrap();
    let key = keys.keys.iter().find(|key| key.id == service).unwrap();
    assert_eq!(key.node_status.as_deref(), Some("offline"));
    assert!(key.execution_unavailable_reason.is_none());
    assert_eq!(
        team::agent(&f.state.db, &f.admin, &agent)
            .await
            .unwrap()
            .grants
            .service_ids,
        stored.grants.service_ids
    );
    direct.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_team_offline_grants_preserve_owner_scope_and_disabled_boundaries() {
    let (f, agent, _, service, _) = fixture("assistant_team_offline_boundaries").await;
    let other = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection("users")
        .insert_one(test_user(&other, crate::models::user::UserType::Org))
        .await
        .unwrap();
    for owner in [&f.admin, &f.outsider, &other] {
        let id = f.service(owner, "outside").await;
        let error = save(&f, &f.admin, &agent, &id)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("Unknown service or no access"), "{error}");
    }
    let missing = uuid::Uuid::new_v4().to_string();
    assert!(
        save(&f, &f.admin, &agent, &missing)
            .await
            .unwrap_err()
            .to_string()
            .contains("Unknown service or no access")
    );
    for actor in [&f.viewer, &f.outsider] {
        assert!(save(&f, actor, &agent, &service).await.is_err());
    }
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(doc! {"_id":&service}, doc! {"$set":{"admin_only":true}})
        .await
        .unwrap();
    assert!(
        save(&f, &f.member, &agent, &service)
            .await
            .unwrap_err()
            .to_string()
            .contains("Unknown service or no access")
    );
    let _ = save(&f, &f.admin, &agent, &service).await.unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&service},
            doc! {"$set":{"admin_only":true,"is_active":false}},
        )
        .await
        .unwrap();
    assert!(
        save(&f, &f.member, &agent, &service)
            .await
            .unwrap_err()
            .to_string()
            .contains("Unknown service or no access")
    );
    assert!(
        save(&f, &f.admin, &agent, &service)
            .await
            .unwrap_err()
            .to_string()
            .contains("Service disabled")
    );
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&service},
            doc! {"$set":{"admin_only":false,"is_active":true}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("org_memberships")
        .update_one(
            doc! {"org_user_id":&f.org,"member_user_id":&f.member},
            doc! {"$set":{"scope_source":"override","allowed_service_ids":[]}},
        )
        .await
        .unwrap();
    assert!(
        save(&f, &f.member, &agent, &service)
            .await
            .unwrap_err()
            .to_string()
            .contains("Unknown service or no access")
    );
    let _ = save(&f, &f.admin, &agent, &service).await.unwrap();
    f.revoke(&f.admin).await;
    assert!(save(&f, &f.admin, &agent, &service).await.is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_team_grants_preserve_live_platform_audience_and_deleted_boundaries() {
    use crate::models::downstream_service::{PlatformKeyAudience, PlatformKeyConfig};
    let (f, agent, _, _, _) = fixture("assistant_team_platform_boundaries").await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.credential_encrypted = vec![1]; // Only metadata is read; never decrypted.
    catalog.platform_key = Some(PlatformKeyConfig {
        purpose: Default::default(),
        enabled: true,
        audience: PlatformKeyAudience::Restricted,
        allowed_owner_ids: vec![f.org.clone()],
    });
    let catalog_id = catalog.id.clone();
    f.state
        .db
        .collection("downstream_services")
        .insert_one(catalog)
        .await
        .unwrap();
    f.state
        .db
        .collection("service_endpoints")
        .insert_one(crate::test_utils::test_service_endpoint(
            &catalog_id,
            "status",
            "GET",
            "/status",
        ))
        .await
        .unwrap();
    let service = f.service(&f.org, "org-platform").await;
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&service},
            doc! {"$set":{"credential_binding":"platform","catalog_service_id":&catalog_id}},
        )
        .await
        .unwrap();
    let _ = save(&f, &f.admin, &agent, &service).await.unwrap();
    // A catalog ID cannot substitute for the organization's own connection.
    assert!(save(&f, &f.admin, &agent, &catalog_id).await.is_err());
    f.state
        .db
        .collection::<bson::Document>("downstream_services")
        .update_one(
            doc! {"_id":&catalog_id},
            doc! {"$set":{"platform_key.allowed_owner_ids":[]}},
        )
        .await
        .unwrap();
    assert!(
        save(&f, &f.admin, &agent, &service)
            .await
            .unwrap_err()
            .to_string()
            .contains("Unknown service or no access")
    );
    f.state
        .db
        .collection::<bson::Document>("downstream_services")
        .update_one(
            doc! {"_id":&catalog_id},
            doc! {"$set":{"platform_key.allowed_owner_ids":[&f.org]}},
        )
        .await
        .unwrap();
    let _ = save(&f, &f.admin, &agent, &service).await.unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&service},
            doc! {"$set":{"deleted_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert!(
        save(&f, &f.admin, &agent, &service)
            .await
            .unwrap_err()
            .to_string()
            .contains("Unknown service or no access")
    );
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .delete_one(doc! {"_id":&service})
        .await
        .unwrap();
    assert!(
        save(&f, &f.admin, &agent, &service)
            .await
            .unwrap_err()
            .to_string()
            .contains("Unknown service or no access")
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_team_grants_keep_missing_credentials_without_exposing_ungranted_diagnostics() {
    let (f, agent, thread, service, _) = fixture("assistant_team_missing_credential").await;
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&service},
            doc! {"$set":{"node_id":null,"auth_method":"bearer","api_key_id":null}},
        )
        .await
        .unwrap();
    let ungranted = call(
        &f,
        &thread,
        "home-assistant__request",
        json!({"method":"GET","path":"/status"}),
    )
    .await;
    assert!(
        !ungranted
            .to_string()
            .contains("Service credential unavailable"),
        "{ungranted}"
    );
    let _ = save(&f, &f.admin, &agent, &service).await.unwrap();
    let result = call(
        &f,
        &thread,
        "home-assistant__request",
        json!({"method":"GET","path":"/status"}),
    )
    .await;
    assert_eq!(result["result"]["isError"], true, "{result}");
    assert!(
        result
            .to_string()
            .contains("Service credential unavailable"),
        "{result}"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_team_offline_primary_with_online_fallback_remains_executable() {
    use crate::services::node_ws_manager::{NodeOutboundMessage, NodeProxyResponse};
    let (f, agent, thread, service, node) = fixture("assistant_team_online_fallback").await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.requires_user_credential = true;
    let catalog_id = catalog.id.clone();
    f.state
        .db
        .collection("downstream_services")
        .insert_one(catalog)
        .await
        .unwrap();
    f.state
        .db
        .collection("service_endpoints")
        .insert_one(crate::test_utils::test_service_endpoint(
            &catalog_id,
            "request",
            "GET",
            "/status",
        ))
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&service},
            doc! {"$set":{"catalog_service_id":&catalog_id}},
        )
        .await
        .unwrap();
    let fallback = uuid::Uuid::new_v4().to_string();
    let mut row = f
        .state
        .db
        .collection::<bson::Document>("nodes")
        .find_one(doc! {"_id":&node})
        .await
        .unwrap()
        .unwrap();
    row.insert("_id", &fallback);
    row.insert("name", "fallback");
    row.insert("status", "online");
    row.insert("last_heartbeat_at", bson::DateTime::now());
    f.state
        .db
        .collection("nodes")
        .insert_one(row)
        .await
        .unwrap();
    f.state
        .db
        .collection("node_service_bindings")
        .insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(), "node_id":&fallback, "user_id":&f.org,
            "service_id":&catalog_id,"is_active":true,"priority":1,
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    register_test_node_connection(&f.state, &fallback, tx).await;
    let _ = save(&f, &f.admin, &agent, &service).await.unwrap();
    let Json(keys) = crate::handlers::keys::list_keys_with_tool_bindings(
        f.state.clone(),
        test_auth_user(&f.admin),
        true,
    )
    .await
    .unwrap();
    let key = keys.keys.iter().find(|key| key.id == service).unwrap();
    assert_eq!(key.node_status.as_deref(), Some("offline"));
    assert!(
        key.execution_unavailable_reason.is_none(),
        "fallback keeps the service available"
    );
    let manager = f.state.node_ws_manager.clone();
    let responder = tokio::spawn(async move {
        let message = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let NodeOutboundMessage::Text(text) = message else {
            panic!("expected proxy request")
        };
        let message: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(message["type"], "proxy_request");
        manager.deliver_proxy_response(
            &fallback,
            NodeProxyResponse {
                request_id: message["request_id"].as_str().unwrap().into(),
                status: 200,
                headers: vec![],
                body: b"fallback-ok".to_vec(),
            },
        );
    });
    let result = call(&f, &thread, "home-assistant__request", json!({})).await;
    assert_eq!(result["result"]["isError"], false, "{result}");
    assert!(result.to_string().contains("fallback-ok"), "{result}");
    responder.await.unwrap();
    f.state.db.drop().await.unwrap();
}
