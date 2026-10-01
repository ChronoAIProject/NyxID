use super::*;
use crate::services::assistant_authority_tests::{fixture, orchestrator_fixture};
use serde_json::json;

#[tokio::test]
async fn machine_tools_add_no_database_work_for_non_chat_callers() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{Arc, Mutex};
    let recorded = Arc::new(Mutex::new(Vec::<mongodb::bson::Document>::new()));
    let commands = recorded.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            commands.lock().unwrap().push(event.command);
        }
    });
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "machine_non_chat_queries",
        handler,
    )
    .await
    .unwrap();
    let state = crate::test_utils::test_app_state(db.clone());
    let auth = McpAuthContext::user(uuid::Uuid::new_v4().to_string(), AuthMethod::Session);
    recorded.lock().unwrap().clear();
    handle_machine_tool(
        &state,
        &auth,
        "nyx__machine_exec",
        json!({}),
        Some(json!(1)),
    )
    .await;
    assert!(
        recorded.lock().unwrap().is_empty(),
        "unavailable tools must not query machine authority"
    );
    let list = JsonRpcRequest {
        jsonrpc: JSONRPC_VERSION.into(),
        id: Some(json!(2)),
        method: "tools/list".into(),
        params: None,
    };
    handle_tools_list(&state, &auth, None, &list).await;
    for command in recorded.lock().unwrap().iter() {
        for key in ["find", "aggregate", "count"] {
            if let Ok(collection) = command.get_str(key) {
                assert!(
                    !collection.starts_with("machine_")
                        && collection != "nodes"
                        && collection != "saved_logins",
                    "non-chat discovery queried {collection}"
                );
            }
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_mcp_tools_are_only_discovered_and_called_by_owner_chat_keys() {
    let f = orchestrator_fixture("machine_mcp_audience").await;
    let list = JsonRpcRequest {
        jsonrpc: JSONRPC_VERSION.into(),
        id: Some(json!(1)),
        method: "tools/list".into(),
        params: None,
    };
    for method in [
        AuthMethod::Session,
        AuthMethod::AccessToken,
        AuthMethod::ApiKey,
        AuthMethod::Delegated,
        AuthMethod::Relay,
        AuthMethod::ServiceAccount,
    ] {
        let auth = McpAuthContext::user(f.owner.clone(), method);
        let response = handle_tools_list(&f.state, &auth, None, &list).await;
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if let Some(tools) = value["result"]["tools"].as_array() {
            assert!(!tools.iter().any(|t| {
                t["name"]
                    .as_str()
                    .is_some_and(crate::services::machine_tools::is_tool)
            }));
        }
        let response = handle_machine_tool(
            &f.state,
            &auth,
            "nyx__machine_list",
            json!({}),
            Some(json!(2)),
        )
        .await;
        let body = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["result"]["isError"], true);
    }
    let specialist = fixture("machine_mcp_specialist").await;
    for (state, chat) in [(&f.state, &f.chat), (&specialist.state, &specialist.chat)] {
        for guest in [false, true] {
            let mut auth = McpAuthContext::user(chat.user_id.clone(), AuthMethod::ApiKey);
            auth.api_key_id = Some(chat.api_key_id.clone());
            let mut chat = chat.clone();
            chat.guest = guest;
            auth.chat = Some(chat);
            let response = handle_tools_list(state, &auth, None, &list).await;
            let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let tools = value["result"]["tools"].as_array().expect("tool list");
            assert_eq!(
                tools.iter().any(|tool| tool["name"] == "nyx__machine_exec"),
                !guest
            );
            let response =
                handle_machine_tool(state, &auth, "nyx__machine_list", json!({}), Some(json!(2)))
                    .await;
            let body = axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(value["result"]["isError"], guest);
        }
    }
    specialist.state.db.drop().await.unwrap();
    f.state.db.drop().await.unwrap();
}
