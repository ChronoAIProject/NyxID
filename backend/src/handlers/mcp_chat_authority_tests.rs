use super::*;
use crate::services::{
    assistant_acknowledgement_service as acks, assistant_agent_credential_service as credentials,
    assistant_authority_tests::{Fixture, connected, fixture, ordinary_key},
};
use axum::{Json, Router, routing::any};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

async fn authenticate(f: &Fixture) -> McpAuthContext {
    let key = credentials::load_for_conversation(
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

async fn result(response: Response, error: bool) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body.get("error").is_none(), "{body}");
    assert_eq!(body["result"]["isError"], error, "{body}");
    serde_json::from_str(body["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

async fn direct_call(f: &Fixture, auth: &McpAuthContext, name: &str, args: Value) -> Response {
    handle_tools_call(
        &f.state,
        auth,
        None,
        &JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "tools/call".into(),
            params: Some(json!({"name": name, "arguments": args})),
        },
        false,
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
    )
    .await
}

async fn call(f: &Fixture, auth: &McpAuthContext, name: &str, args: Value) -> Response {
    direct_call(
        f,
        auth,
        "nyx__call_tool",
        json!({"tool_name": name, "arguments_json": args.to_string()}),
    )
    .await
}

#[tokio::test]
async fn mounted_chat_service_edits_record_verified_actor_and_separate_request_groups() {
    use crate::models::service_change_event::{HistoryActorKind, ServiceChangeEvent};
    use futures::TryStreamExt;
    use tower::ServiceExt;

    let f = fixture("chat_service_history").await;
    let service = connected(
        &f.state.db,
        &f.owner,
        "history-target",
        "https://example.com",
    )
    .await;
    f.state
        .db
        .collection::<mongodb::bson::Document>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$set": {"active_turn": mongodb::bson::Bson::Null}},
        )
        .await
        .unwrap();
    crate::services::assistant_access_mode_service::change(
        &f.state.db,
        &f.owner,
        &f.row.id,
        crate::models::assistant_conversation::AccessMode::Full,
    )
    .await
    .unwrap();
    let credential = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let (_, private) = crate::routes::build_router_with_state(f.state.clone());
    let router = private.with_state(f.state.clone());
    for enabled in [false, false, true] {
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("x-api-key", credential.raw_key.as_str())
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        json!({
                            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                            "params": {"name": "nyxid__set_service_enabled", "arguments": {
                                "service_id": service, "enabled": enabled
                            }}
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(result(response, false).await["is_active"], enabled);
    }
    let events: Vec<ServiceChangeEvent> = f
        .state
        .db
        .collection(crate::models::service_change_event::COLLECTION_NAME)
        .find(doc! {"service_id": &service})
        .sort(doc! {"service_sequence": 1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(events.len(), 2, "a no-op toggle must not add history");
    assert_eq!(events[0].action, "service.disabled");
    assert_eq!(events[1].action, "service.enabled");
    assert_ne!(events[0].change_group_id, events[1].change_group_id);
    for event in events {
        assert_eq!(event.actor.kind, HistoryActorKind::ApiKey);
        assert_eq!(
            event.actor.api_key_id.as_deref(),
            Some(f.row.credential_api_key_id.as_str())
        );
        assert_eq!(event.actor.person_id.as_deref(), Some(f.owner.as_str()));
        assert_eq!(event.owner_id, f.owner);
        assert!(event.actor.name.starts_with("NyxID Assistant chat "));
    }
}

#[tokio::test]
async fn chat_mcp_lists_ungranted_tools_and_allow_retries_execute_without_bypassing_denial() {
    let f = fixture("chat_mcp_allow").await;
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let upstream = Router::new().route(
        "/{*path}",
        any(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { Json(json!({"ok": true})) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let id = connected(&f.state.db, &f.owner, "github", &address).await;
    let auto = connected(&f.state.db, &f.owner, "chrono-llm-public", &address).await;
    f.state
        .db
        .collection::<mongodb::bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &auto},
            doc! {"$set": {
                "source": crate::models::user_service::AUTO_PROVISION_SOURCE,
            }},
        )
        .await
        .unwrap();
    let auth = authenticate(&f).await;
    let listing = result(
        handle_meta_list_connected(&f.state, &auth, &json!({}), None).await,
        false,
    )
    .await;
    let rows = listing["services"].as_array().unwrap();
    assert_eq!(
        rows.iter().find(|r| r["service_id"] == id).unwrap()["chat_access"],
        "acknowledgement_required"
    );
    assert_eq!(
        rows.iter().find(|r| r["service_id"] == auto).unwrap()["chat_access"],
        "granted"
    );
    let search = result(
        handle_meta_search(
            &f.state,
            &auth,
            None,
            &json!({"query": "github"}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    let tool = &search["matches"][0];
    assert_eq!(tool["chat_access"], "acknowledgement_required");
    for body in [&listing, &search] {
        let hint = body["chat_access_hint"].as_str().unwrap();
        assert!(
            hint.contains("acknowledgement_required = call the tool now"),
            "{hint}"
        );
    }
    let name = tool["name"].as_str().unwrap();
    let args = json!({"method": "GET", "path": "/ok"});
    let refusal = result(call(&f, &auth, name, args.clone()).await, true).await;
    assert_eq!(refusal["error"], "acknowledgement_required");
    assert_eq!(refusal["kind"], "service");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    acks::decide(
        &f.state.db,
        &f.owner,
        &f.row.id,
        refusal["acknowledgement_id"].as_str().unwrap(),
        true,
    )
    .await
    .unwrap();
    let auth = authenticate(&f).await;
    let success = result(call(&f, &auth, name, args).await, false).await;
    assert_eq!(success["ok"], true);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let second = connected(&f.state.db, &f.owner, "denied", &address).await;
    let refused = acks::service_gate(&f.state.db, &f.chat, &second, "denied", "Denied", false)
        .await
        .unwrap()
        .unwrap();
    acks::decide(
        &f.state.db,
        &f.owner,
        &f.row.id,
        refused["acknowledgement_id"].as_str().unwrap(),
        false,
    )
    .await
    .unwrap();
    let services = load_all_services_for_meta_tools(&f.state, &auth)
        .await
        .unwrap();
    let service = services.iter().find(|s| s.service_id == second).unwrap();
    let denied = result(
        chat_service_gate(&f.state, &auth, service, None)
            .await
            .unwrap(),
        true,
    )
    .await;
    assert_eq!(denied["error"], "acknowledgement_denied");
    f.state
        .db
        .collection::<mongodb::bson::Document>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$set": {"active_turn": mongodb::bson::Bson::Null}},
        )
        .await
        .unwrap();
    crate::services::assistant_access_mode_service::change(
        &f.state.db,
        &f.owner,
        &f.row.id,
        crate::models::assistant_conversation::AccessMode::Full,
    )
    .await
    .unwrap();
    let full = authenticate(&f).await;
    assert_eq!(chat_access(&full, service), "granted");
    assert!(
        chat_service_gate(&f.state, &full, service, None)
            .await
            .is_none()
    );
    let search = result(
        handle_meta_search(
            &f.state,
            &full,
            None,
            &json!({"query": "denied"}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    let full_name = search["matches"][0]["name"].as_str().unwrap();
    result(
        call(
            &f,
            &full,
            full_name,
            json!({"method": "GET", "path": "/ok"}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let audit = f
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {
            "event_type": "assistant_mcp_tool_call",
            "event_data.conversation_id": &f.row.id,
            "event_data.access_mode": "full",
            "event_data.tool_name": "nyx__call_tool",
        })
        .await
        .unwrap()
        .unwrap();
    let metadata = audit.get_document("event_data").unwrap();
    assert!(!metadata.contains_key("arguments"));
    assert_eq!(metadata.get_str("outcome").unwrap(), "requested");
    // Existing proxy audit is asynchronous. Poll its durable row, not a guessed delay.
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let found = f
                .state
                .db
                .collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .find_one(doc! {"event_type": "mcp_tool_call", "event_data.access_mode": "full"})
                .await
                .unwrap()
                .is_some();
            if found {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    server.abort();
}

#[tokio::test]
async fn chat_mcp_native_account_and_action_refusals_are_tool_results() {
    let f = fixture("chat_mcp_account").await;
    let auth = authenticate(&f).await;
    let search = result(
        handle_meta_search(
            &f.state,
            &auth,
            None,
            &json!({"query": "list_agent_keys"}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    assert!(
        search["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "nyxid__list_agent_keys"
                && tool["chat_access"] == "acknowledgement_required")
    );
    let refusal = result(
        call(&f, &auth, "nyxid__list_agent_keys", json!({})).await,
        true,
    )
    .await;
    assert_eq!(refusal["kind"], "account");
    acks::decide(
        &f.state.db,
        &f.owner,
        &f.row.id,
        refusal["acknowledgement_id"].as_str().unwrap(),
        true,
    )
    .await
    .unwrap();
    let auth = authenticate(&f).await;
    let list = result(
        call(&f, &auth, "nyxid__list_agent_keys", json!({})).await,
        false,
    )
    .await;
    assert_eq!(list["total"], 1);
    let target = ordinary_key(&f).await;
    let action = result(
        call(
            &f,
            &auth,
            "nyxid__delete_agent_key",
            json!({"api_key_id": target}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(action["kind"], "action");
    assert_eq!(action["error"], "acknowledgement_required");
    assert!(
        action["summary"]
            .as_str()
            .unwrap()
            .contains("Delete agent key")
    );
    let mismatch = result(
        call(
            &f,
            &auth,
            "nyxid__delete_agent_key",
            json!({
                "api_key_id": target,
                "acknowledgement_id": action["acknowledgement_id"],
            }),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(mismatch["error"], "acknowledgement_invalid");
}

#[test]
fn chat_request_audit_covers_execution_and_mutation_but_not_discovery() {
    for name in [
        "nyx__call_tool",
        "nyx__connect_service",
        "nyx__wait_for_connection",
        "nyx__ssh_exec",
        "nyx__oracle_pools",
        "nyx__oracle_ask",
        "nyx__oracle_result",
        "nyx__oracle_attach",
        "nyx__oracle_extract",
        "nyx__oracle_session",
        "nyxid__delete_agent_key",
        "nyxid__list_agent_keys",
    ] {
        assert!(audit_chat_tool(name), "{name}");
    }
    for name in [
        "nyx__search_tools",
        "nyx__list_connected_services",
        "nyx__discover_services",
        "nyx__ssh_list_services",
        "github__status",
    ] {
        assert!(!audit_chat_tool(name), "{name}");
    }
}

#[tokio::test]
async fn chat_discovery_does_not_write_request_audits_but_execution_refusals_do() {
    let f = fixture("chat_discovery_audit").await;
    let auth = authenticate(&f).await;
    for name in [
        "nyx__search_tools",
        "nyx__list_connected_services",
        "nyx__discover_services",
    ] {
        result(
            direct_call(&f, &auth, name, json!({"query": "github"})).await,
            false,
        )
        .await;
    }
    let logs = f
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME);
    let filter = doc! {"event_type": "assistant_mcp_tool_call"};
    assert_eq!(logs.count_documents(filter.clone()).await.unwrap(), 0);
    for name in ["nyx__call_tool", "nyxid__list_agent_keys"] {
        let args = if name == "nyx__call_tool" {
            json!({"tool_name": "nyxid__list_agent_keys", "arguments_json": "{}"})
        } else {
            json!({})
        };
        result(direct_call(&f, &auth, name, args).await, true).await;
        let row = logs
            .find_one(doc! {"event_type": "assistant_mcp_tool_call", "event_data.tool_name": name})
            .await
            .unwrap()
            .unwrap();
        let data = row.get_document("event_data").unwrap();
        assert_eq!(data.get_str("access_mode").unwrap(), "ask");
        assert_eq!(data.get_str("conversation_id").unwrap(), f.row.id);
        assert!(!data.contains_key("arguments"));
    }
    assert_eq!(logs.count_documents(filter).await.unwrap(), 2);
}

#[tokio::test]
async fn platform_services_get_a_consent_card_in_ask_mode_and_execute_after_allow() {
    use crate::models::{
        assistant_acknowledgement::COLLECTION_NAME as ACKS,
        assistant_conversation::{AccessMode, COLLECTION_NAME as CONVERSATIONS},
        service_endpoint::ServiceEndpoint,
    };
    let f = fixture("chat_platform_full_required").await;
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let upstream = Router::new().route(
        "/status",
        any(move |headers: HeaderMap| {
            assert!(headers.contains_key("authorization"));
            count.fetch_add(1, Ordering::SeqCst);
            async { Json(json!({"ok": true})) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let mut service = crate::test_utils::test_auto_connected_catalog_service();
    service.slug = "shared-model".into();
    service.name = "Shared model".into();
    service.base_url = address;
    service.service_category = "internal".into();
    service.auth_method = "bearer".into();
    service.credential_encrypted = f
        .state
        .encryption_keys
        .encrypt(uuid::Uuid::new_v4().to_string().as_bytes())
        .await
        .unwrap();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&service)
        .await
        .unwrap();
    let now = chrono::Utc::now();
    f.state
        .db
        .collection(crate::models::service_endpoint::COLLECTION_NAME)
        .insert_one(ServiceEndpoint {
            target_id: None,
            id: uuid::Uuid::new_v4().to_string(),
            service_id: service.id.clone(),
            name: "status".into(),
            description: Some("Get shared model status".into()),
            method: "GET".into(),
            path: "/status".into(),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
            response_description: None,
            response: Default::default(),
            risk: None,
            supports_idempotency_key: false,
            is_active: true,
            operation_generation: 1,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>(crate::models::user_service::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    for mode in [AccessMode::Ask, AccessMode::Full] {
        if mode == AccessMode::Full {
            f.state
                .db
                .collection::<mongodb::bson::Document>(CONVERSATIONS)
                .update_one(
                    doc! {"_id": &f.row.id},
                    doc! {"$set": {"active_turn": mongodb::bson::Bson::Null}},
                )
                .await
                .unwrap();
            crate::services::assistant_access_mode_service::change(
                &f.state.db,
                &f.owner,
                &f.row.id,
                mode,
            )
            .await
            .unwrap();
        }
        let auth = authenticate(&f).await;
        let expected = if mode == AccessMode::Ask {
            "acknowledgement_required"
        } else {
            "granted"
        };
        let listing = result(
            direct_call(&f, &auth, "nyx__list_connected_services", json!({})).await,
            false,
        )
        .await;
        let row = listing["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["service_id"] == service.id)
            .unwrap();
        assert_eq!(row["source"], "platform");
        assert_eq!(row["chat_access"], expected);
        let search = result(
            direct_call(
                &f,
                &auth,
                "nyx__search_tools",
                json!({"query": "shared-model"}),
            )
            .await,
            false,
        )
        .await;
        let tool = &search["matches"][0];
        assert_eq!(tool["chat_access"], expected);
        let name = tool["name"].as_str().unwrap();
        if mode == AccessMode::Ask {
            // Both call shapes ask for the same card; nothing executes yet.
            let mut ids = Vec::new();
            for direct in [true, false] {
                let response = if direct {
                    direct_call(&f, &auth, name, json!({})).await
                } else {
                    call(&f, &auth, name, json!({})).await
                };
                let refusal = result(response, true).await;
                assert_eq!(refusal["error"], "acknowledgement_required");
                assert_eq!(refusal["kind"], "service");
                assert_eq!(refusal["service_slug"], service.slug);
                assert!(
                    refusal["summary"]
                        .as_str()
                        .unwrap()
                        .contains("platform credential"),
                    "{refusal}"
                );
                ids.push(refusal["acknowledgement_id"].as_str().unwrap().to_owned());
            }
            assert_eq!(ids[0], ids[1], "pending requests deduplicate");
            assert_eq!(hits.load(Ordering::SeqCst), 0);
            let rows: Vec<crate::models::assistant_acknowledgement::AssistantAcknowledgement> = f
                .state
                .db
                .collection(ACKS)
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert!(rows[0].platform);
            assert_eq!(rows[0].service_id.as_deref(), Some(service.id.as_str()));
            // The chat may also mint a hosted connect link in Ask mode.
            let connect = handle_meta_connect(
                &f.state,
                &auth,
                None,
                &json!({"service_id": service.id}),
                None,
                false,
            )
            .await;
            let bytes = axum::body::to_bytes(connect.into_body(), 1024 * 1024)
                .await
                .unwrap();
            assert!(
                !String::from_utf8_lossy(&bytes).contains("does not have access"),
                "{}",
                String::from_utf8_lossy(&bytes)
            );
            acks::decide(&f.state.db, &f.owner, &f.row.id, &ids[0], true)
                .await
                .unwrap();
            let auth = authenticate(&f).await;
            assert_eq!(
                chat_access(&auth, &{
                    let services = load_all_services_for_meta_tools(&f.state, &auth)
                        .await
                        .unwrap();
                    services
                        .into_iter()
                        .find(|s| s.service_id == service.id)
                        .unwrap()
                }),
                "granted"
            );
            for direct in [true, false] {
                let response = if direct {
                    direct_call(&f, &auth, name, json!({})).await
                } else {
                    call(&f, &auth, name, json!({})).await
                };
                let value = result(response, false).await;
                assert_eq!(value["ok"], true);
            }
            assert_eq!(hits.load(Ordering::SeqCst), 2);
        } else {
            for direct in [true, false] {
                let response = if direct {
                    direct_call(&f, &auth, name, json!({})).await
                } else {
                    call(&f, &auth, name, json!({})).await
                };
                let value = result(response, false).await;
                assert_eq!(value["ok"], true);
            }
            assert_eq!(
                f.state
                    .db
                    .collection::<mongodb::bson::Document>(ACKS)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                1,
                "Full mode creates no cards"
            );
        }
    }
    assert_eq!(hits.load(Ordering::SeqCst), 4);
    server.abort();
}

#[tokio::test]
async fn ask_service_consent_allows_its_node_route_without_a_second_grant() {
    use crate::services::{
        node_service,
        node_ws_manager::{NodeOutboundMessage, NodeProxyResponse, NodeWsManager},
    };
    let mut f = fixture("chat_node_service_consent").await;
    // This test supplies a local node responder without the cross-pod dispatcher.
    f.state.node_ws_manager = Arc::new(NodeWsManager::new(30, 100));
    let (_, token, _) =
        node_service::create_registration_token(&f.state.db, &f.owner, "my-node", 10, 3600)
            .await
            .unwrap();
    let (node, _, _) =
        node_service::register_node(&f.state.db, &f.state.encryption_keys, &token, None)
            .await
            .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    f.state.node_ws_manager.register_connection(&node.id, tx);
    let service = connected(
        &f.state.db,
        &f.owner,
        "node-service",
        "https://node.example.com",
    )
    .await;
    f.state
        .db
        .collection::<mongodb::bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(doc! {"_id": &service}, doc! {"$set": {"node_id": &node.id}})
        .await
        .unwrap();
    let auth = authenticate(&f).await;
    assert!(auth.allow_all_nodes && !auth.allow_all_services);
    assert!(auth.allowed_node_ids.is_empty());
    let search = result(
        direct_call(
            &f,
            &auth,
            "nyx__search_tools",
            json!({"query": "node-service"}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(
        search["matches"][0]["chat_access"],
        "acknowledgement_required"
    );
    let name = search["matches"][0]["name"].as_str().unwrap();
    let args = json!({"method": "GET", "path": "/ok"});
    let refusal = result(call(&f, &auth, name, args.clone()).await, true).await;
    assert_eq!(refusal["kind"], "service");
    assert!(rx.try_recv().is_err());
    acks::decide(
        &f.state.db,
        &f.owner,
        &f.row.id,
        refusal["acknowledgement_id"].as_str().unwrap(),
        true,
    )
    .await
    .unwrap();
    let auth = authenticate(&f).await;
    assert_eq!(auth.allowed_service_ids, vec![service]);
    let manager = f.state.node_ws_manager.clone();
    let responder = tokio::spawn(async move {
        for _ in 0..2 {
            let message = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
                .await
                .unwrap()
                .unwrap();
            let NodeOutboundMessage::Text(text) = message else {
                panic!("expected a proxy request");
            };
            let request: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(request["type"], "proxy_request");
            assert_eq!(request["service_slug"], "node-service");
            manager.deliver_proxy_response(
                &node.id,
                NodeProxyResponse {
                    request_id: request["request_id"].as_str().unwrap().into(),
                    status: 200,
                    headers: vec![],
                    body: br#"{"ok":true}"#.to_vec(),
                },
            );
        }
    });
    assert_eq!(
        result(call(&f, &auth, name, args.clone()).await, false).await["ok"],
        true
    );
    assert_eq!(
        result(direct_call(&f, &auth, name, args).await, false).await["ok"],
        true
    );
    responder.await.unwrap();
    let history = acks::history(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, "service");
    assert_eq!(history[0].status, "allowed");
}

#[tokio::test]
async fn chat_tool_calls_are_recorded_as_metadata_only_turn_activity() {
    let f = fixture("mcp_activity").await;
    let auth = authenticate(&f).await;
    // One entry per tools/call: the effective tool name, never arguments.
    call(
        &f,
        &auth,
        "nyxid__list_agent_keys",
        json!({"secret_argument": "nyxid_ag_do_not_record"}),
    )
    .await;
    direct_call(&f, &auth, "nyx__search_tools", json!({"query": "issues"})).await;
    direct_call(&f, &auth, "nyxid__list_agent_keys", json!({})).await;
    let row = crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    let activities = row.active_turn.as_ref().unwrap().activities.clone();
    let labels: Vec<_> = activities.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "nyxid__list_agent_keys",
            "nyx__search_tools",
            "nyxid__list_agent_keys"
        ]
    );
    assert!(
        activities
            .iter()
            .all(|a| a.status != "running" && a.ended_at.is_some())
    );
    let encoded = serde_json::to_string(&activities).unwrap();
    assert!(!encoded.contains("do_not_record") && !encoded.contains("issues"));
    // Callers outside a chat record nothing.
    let plain = McpAuthContext::user(f.owner.clone(), AuthMethod::Session);
    direct_call(&f, &plain, "nyx__search_tools", json!({"query": "x"})).await;
    let row = crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(row.active_turn.unwrap().activities.len(), 3);
}
