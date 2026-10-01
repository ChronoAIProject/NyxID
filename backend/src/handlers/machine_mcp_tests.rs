use super::*;
use crate::services::assistant_authority_tests::{fixture, orchestrator_fixture};
use serde_json::{Value, json};

use crate::services::{
    assistant_acknowledgement_service as acks,
    assistant_authority_tests::Fixture,
    machine_integration_tests::{node, peer},
};
use mongodb::bson::{self, doc};

/// Persist the same turn/run binding that trigger admission creates, then use
/// actual chat-key authentication to recover its snapshotted policy and grants.
async fn trigger_auth(
    f: &Fixture,
    policy: Option<crate::models::trigger_schedule::ConfirmationPolicy>,
) -> McpAuthContext {
    let run = uuid::Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<bson::Document>(crate::models::trigger_run::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &run, "trigger_id": uuid::Uuid::new_v4().to_string(),
            "user_id": &f.owner, "scheduled_at": now, "deadline": now,
            "outcome": "started", "confirmation_policy": bson::to_bson(&policy).unwrap(),
            "thread_id": &f.row.id, "fence": "test", "lease_until": now, "expires_at": now,
        })
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$set": {
                "active_turn.trigger_run_id": run, "active_turn.origin": "trigger",
                "guest_turn": false,
            }},
        )
        .await
        .unwrap();
    authenticated_machine_chat(f).await
}

async fn authenticated_machine_chat(f: &Fixture) -> McpAuthContext {
    let key = crate::services::assistant_agent_credential_service::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", key.raw_key.parse().unwrap());
    authenticate_mcp(&f.state, &headers, false).await.unwrap()
}

async fn machine_mcp_call(
    f: &Fixture,
    auth: &McpAuthContext,
    tool: &str,
    arguments: Value,
) -> Value {
    let response = Box::pin(dispatch_tools_call(
        &f.state,
        auth,
        None,
        &JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.into(),
            id: Some(json!(1)),
            method: "tools/call".into(),
            params: Some(json!({"name":tool,"arguments":arguments})),
        },
        false,
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
    ))
    .await;
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let result: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(result.get("error").is_none(), "{result}");
    serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn machine_webhook_changes_share_one_exact_card_with_machine_confirmation() {
    use crate::models::trigger_schedule::ConfirmationPolicy;
    for confirmation in ["none", "changes"] {
        let f = orchestrator_fixture("machine_webhook_exact_card").await;
        let node = node(&f, &f.owner).await;
        f.state
            .db
            .collection::<bson::Document>(crate::models::node::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &node.id},
                doc! {"$set": {"machine_confirm": confirmation}},
            )
            .await
            .unwrap();
        let auth = trigger_auth(&f, Some(ConfirmationPolicy::Changes)).await;
        let (task, mut requests) =
            peer(&f, &node, json!({"exit_code":0,"status":"finished"})).await;
        let args = json!({"machine":node.id,"command":"true","services":[]});
        let card = machine_mcp_call(&f, &auth, "nyx__machine_exec", args.clone()).await;
        assert_eq!(card["error"], "acknowledgement_required");
        assert!(requests.try_recv().is_err());
        let id = card["acknowledgement_id"].as_str().unwrap();
        let history = acks::history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].trigger_run_id.is_some());
        assert!(history[0].summary.contains("declared services"));
        assert_eq!(history[0].decider, "user");
        acks::decide(&f.state.db, &f.owner, &f.row.id, id, true)
            .await
            .unwrap();
        let mut approved = args;
        approved["acknowledgement_id"] = json!(id);
        let mut changed = approved.clone();
        changed["command"] = json!("echo changed");
        assert_eq!(
            machine_mcp_call(&f, &auth, "nyx__machine_exec", changed).await["error"],
            "acknowledgement_invalid"
        );
        // Universal-tool dispatch cannot bypass the same gate or consume twice.
        let result = machine_mcp_call(
            &f,
            &auth,
            "nyx__call_tool",
            json!({
                "tool_name":"nyx__machine_exec", "arguments_json":approved.to_string(),
            }),
        )
        .await;
        assert_eq!(result["exit_code"], 0);
        assert_eq!(
            requests.recv().await.unwrap().operation,
            nyxid_machine::Operation::Exec
        );
        assert_eq!(
            machine_mcp_call(&f, &auth, "nyx__machine_exec", approved).await["error"],
            "acknowledgement_invalid"
        );
        assert_eq!(
            acks::history(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap()
                .len(),
            1
        );
        task.abort();
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn machine_scheduled_run_without_confirmation_executes_and_keeps_owner_control() {
    let f = orchestrator_fixture("machine_schedule_authority").await;
    let node = node(&f, &f.owner).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &node.id},
            doc! {"$set": {"machine_confirm": "none"}},
        )
        .await
        .unwrap();
    let auth = trigger_auth(&f, None).await;
    assert!(!auth.chat.as_ref().unwrap().guest);
    let (task, mut requests) = peer(&f, &node, json!({"exit_code":0,"status":"finished"})).await;
    let args = json!({"machine":node.id,"command":"true"});
    assert_eq!(
        machine_mcp_call(&f, &auth, "nyx__machine_exec", args.clone()).await["exit_code"],
        0
    );
    requests.recv().await.unwrap();
    assert!(
        acks::history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .is_empty()
    );
    use crate::services::machine_desktop_service as desktop;
    let row = desktop::open(&f.state.db, &f.owner, &node.id, Some(&f.row.id))
        .await
        .unwrap();
    let row = desktop::take(&f.state.db, &row, "owner").await.unwrap();
    desktop::controlled(&f.state.db, &row, "owner")
        .await
        .unwrap();
    assert_eq!(
        machine_mcp_call(&f, &auth, "nyx__machine_exec", args).await["error_code"],
        12408
    );
    assert!(requests.try_recv().is_err());
    task.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_read_only_tools_pass_webhook_gate_and_specialist_grants_stay_required() {
    use crate::models::trigger_schedule::ConfirmationPolicy;
    let f = fixture("machine_webhook_reads_grants").await;
    let node = node(&f, &f.owner).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::node::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &node.id},
            doc! {"$set": {"machine_confirm": "none"}},
        )
        .await
        .unwrap();
    let mut auth = trigger_auth(&f, Some(ConfirmationPolicy::Changes)).await;
    let denied = machine_mcp_call(
        &f,
        &auth,
        "nyx__machine_read_file",
        json!({"machine":node.id,"path":"a"}),
    )
    .await;
    assert_eq!(denied["error"], "acknowledgement_required");
    let history = acks::history(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(history[0].kind, "machine");
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$set": {"machine_node_ids": [&node.id]}},
        )
        .await
        .unwrap();
    auth.chat = acks::for_key(&f.state.db, &f.owner, Some(&f.chat.api_key_id))
        .await
        .unwrap();
    let job = crate::services::machine_service::issue_job(
        &f.state.db,
        auth.chat.as_ref().unwrap(),
        &node,
        120,
        Vec::new(),
    )
    .await
    .unwrap();
    let (task, _) = peer(
        &f,
        &node,
        json!({"status":"running","content":"ok","files":[]}),
    )
    .await;
    for (tool, args) in [
        ("nyx__machine_list", json!({})),
        ("nyx__saved_logins", json!({})),
        (
            "nyx__machine_list_files",
            json!({"machine":node.id,"path":"."}),
        ),
        (
            "nyx__machine_read_file",
            json!({"machine":node.id,"path":"a"}),
        ),
        (
            "nyx__machine_job",
            json!({"machine":node.id,"job_id":job.id}),
        ),
    ] {
        let result = machine_mcp_call(&f, &auth, tool, args).await;
        assert!(result.get("error").is_none(), "{tool}: {result}");
    }
    assert_eq!(
        acks::history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .len(),
        1
    );
    task.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_webhook_login_and_control_are_changing_but_not_destructive() {
    use crate::models::trigger_schedule::ConfirmationPolicy;
    use crate::services::saved_login_service;
    for policy in [ConfirmationPolicy::Changes, ConfirmationPolicy::Destructive] {
        let f = orchestrator_fixture("machine_webhook_effects").await;
        let node = node(&f, &f.owner).await;
        f.state
            .db
            .collection::<bson::Document>(crate::models::node::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &node.id},
                doc! {"$set": {
                    "machine_confirm": "none", "machine.browser_isolated": true,
                }},
            )
            .await
            .unwrap();
        let login = saved_login_service::put(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &f.owner,
            None,
            serde_json::from_value(json!({
                "label":"Fixture", "allowed_origins":["https://fixture.example"],
                "username":"fixture-user", "password":"fixture-secret",
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let auth = trigger_auth(&f, Some(policy)).await;
        let (task, mut requests) = peer(&f, &node, json!({"filled":true})).await;
        let exec = machine_mcp_call(
            &f,
            &auth,
            "nyx__machine_exec",
            json!({"machine":node.id,"command":"true"}),
        )
        .await;
        assert_eq!(
            exec["error"], "acknowledgement_required",
            "arbitrary shell remains destructive"
        );
        for (tool, args) in [
            (
                "nyx__machine_fill_login",
                json!({"machine":node.id,"login":login.id,"field":"password"}),
            ),
            (
                "nyx__machine_request_control",
                json!({"machine":node.id,"reason":"Please review the page"}),
            ),
        ] {
            let result = machine_mcp_call(&f, &auth, tool, args).await;
            assert_eq!(
                result["error"] == "acknowledgement_required",
                policy == ConfirmationPolicy::Changes,
                "{tool}: {result}"
            );
            assert!(!result.to_string().contains("fixture-secret"));
        }
        if policy == ConfirmationPolicy::Changes {
            assert!(requests.try_recv().is_err());
        } else {
            assert_eq!(
                requests.recv().await.unwrap().operation,
                nyxid_machine::Operation::FillLogin
            );
        }
        task.abort();
        f.state.db.drop().await.unwrap();
    }
}

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
