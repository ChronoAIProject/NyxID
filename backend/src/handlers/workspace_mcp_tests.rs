use super::*;
use crate::services::destination_routing::tests::{Echo, connect, seed};
use crate::test_utils::*;

#[tokio::test]
async fn workspace_mcp_protocol_typed_and_universal_calls_resolve_the_same_google_target() {
    use crate::services::billing::route_inventory::*;
    let db = connect_test_database("workspace_mcp_protocol")
        .await
        .unwrap();
    seed(&db, false).await;
    let owner = uuid::Uuid::new_v4().to_string();
    connect(&db, &owner, "api-google-workspace").await;
    connect(&db, &owner, "api-google-docs").await;
    connect(&db, &owner, "api-google-drive").await;
    let echo = Echo::start().await;
    let mut state = test_app_state(db.clone());
    state.http_client = echo.client.clone();
    let auth = McpAuthContext::user(owner, AuthMethod::AccessToken);
    crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER.scope(echo.client_builder.clone(),async {
        for active in [false,true] {
            if active { seed(&db,true).await; }
            for slug in ["api-google-workspace","api-google-drive","api-google-docs"] {
                for universal in [false,true] {
                    let tool = format!("{slug}__docs_batch_update_document");
                    let args = serde_json::json!({"documentId":"protocol-doc","requests":[]});
                    let params = if universal { serde_json::json!({"name":"nyx__call_tool","arguments":{"tool_name":tool,"arguments_json":args.to_string()}}) } else { serde_json::json!({"name":tool,"arguments":args}) };
                    let request = JsonRpcRequest {jsonrpc:JSONRPC_VERSION.into(),id:Some(serde_json::json!(1)),method:"tools/call".into(),params:Some(params)};
                    let response = handle_tools_call(&state,&auth,None,&request,false,enforce_billing_egress_classification(Some(BillingRoutePolicy::Metered(BillingIngress::Mcp)),BillingIngress::Mcp).unwrap()).await;
                    assert_eq!(response.status(),200);
                    let bytes = axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();
                    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    if !active && slug != "api-google-docs" {
                        assert_eq!(value["result"]["isError"],true,"{value}");
                        assert!(value.to_string().contains("workspace_destinations_not_activated"),"{value}");
                    } else {
                        assert_ne!(value["result"]["isError"],true,"{value}");
                        assert!(value.to_string().contains("docs.googleapis.com"),"{value}");
                    }
                }
            }
        }
    }).await;
    assert_eq!(echo.calls.lock().unwrap().len(), 8);
}
