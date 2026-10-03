use super::*;
use crate::models::assistant_agent::GuestAccess;
use crate::services::{
    assistant_acknowledgement_service as acks, assistant_agent_credential_service as credentials,
    assistant_authority_tests::{
        Fixture, connected, fixture, orchestrator_fixture, ordinary_key, service_gate,
    },
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
    authenticate_id(f, &f.row.id).await
}

/// Authenticate as another conversation of the same owner (e.g. the team's
/// orchestrator when the fixture is a subagent).
async fn authenticate_id(f: &Fixture, id: &str) -> McpAuthContext {
    let key =
        credentials::load_for_conversation(&f.state.db, &f.state.encryption_keys, &f.owner, id)
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
    // Boxed: the handler's future is large, and long tests await it often.
    Box::pin(handle_tools_call(
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
    ))
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

async fn assert_scoped_thread_attachment_access(f: &Fixture, auth: &McpAuthContext) {
    use crate::services::assistant_upload_service as uploads;

    assert!(!auth.assistant_operation_scopes.is_empty());
    let item = uploads::upload(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
        "notes.txt",
        b"Owner-provided thread notes".to_vec(),
    )
    .await
    .unwrap();
    let message = f
        .state
        .db
        .collection::<crate::models::assistant_message::AssistantMessage>(
            crate::models::assistant_message::COLLECTION_NAME,
        )
        .find_one(doc! {"conversation_id": &f.row.id, "role": "user"})
        .await
        .unwrap()
        .unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    uploads::bind(
        &f.state.db,
        &f.owner,
        &f.row.id,
        &message.id,
        std::slice::from_ref(&item.id),
        &mut session,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();

    let search = result(
        direct_call(
            f,
            auth,
            "nyx__search_tools",
            json!({"query": "attachment read"}),
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
            .any(|tool| tool["name"] == "nyx__attachment_read")
    );
    for generic in [false, true] {
        let args = json!({"attachment_id": item.id});
        let response = if generic {
            call(f, auth, "nyx__attachment_read", args).await
        } else {
            direct_call(f, auth, "nyx__attachment_read", args).await
        };
        assert_eq!(
            result(response, false).await["text"],
            "Owner-provided thread notes"
        );
    }
}

#[tokio::test]
async fn mounted_chat_service_edits_record_verified_actor_and_separate_request_groups() {
    use crate::models::service_change_event::{HistoryActorKind, ServiceChangeEvent};
    use futures::TryStreamExt;
    use tower::ServiceExt;

    let f = orchestrator_fixture("chat_service_history").await;
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
    // Subagents hold explicit grants only: auto-connected services too.
    assert_eq!(
        rows.iter().find(|r| r["service_id"] == auto).unwrap()["chat_access"],
        "acknowledgement_required"
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
        assert!(hint.contains("call the tool now"), "{hint}");
    }
    let name = tool["name"].as_str().unwrap();
    let args = json!({"method": "GET", "path": "/ok"});
    let refusal = result(call(&f, &auth, name, args.clone()).await, true).await;
    assert_eq!(refusal["error"], "acknowledgement_required");
    assert_eq!(refusal["kind"], "service");
    assert_eq!(refusal["decider"], "orchestrator");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    // The request reached the orchestrator as a wake-up event (it is busy
    // with its own turn, so the event waits in its queue).
    let team_id = f.nyxbot_thread.clone();
    let orchestrator = crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &team_id)
        .await
        .unwrap();
    assert!(orchestrator.pending_events.iter().any(|event| {
        event.kind == "permission_requested"
            && event.agent_id.as_deref() == Some(f.chat.agent_id.as_str())
            && event
                .text
                .contains(refusal["acknowledgement_id"].as_str().unwrap())
    }));
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
    let refused = service_gate(&f.state.db, &f.chat, &second, "denied", "Denied", false)
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
    // The orchestrator runs with Full access: no gates, every service granted.
    let full = authenticate_id(&f, &team_id).await;
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
            "event_data.conversation_id": &team_id,
            "event_data.agent_role": "orchestrator",
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
                .find_one(doc! {"event_type": "mcp_tool_call",
                "event_data.agent_role": "orchestrator"})
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
    assert!(list["total"].as_u64().unwrap() >= 1);
    let target = ordinary_key(&f).await;
    // Subagents cannot change or delete account resources at all.
    let refused = result(
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
    assert_eq!(refused["error"], "orchestrator_only");
    // The orchestrator confirms destructive actions with the user by default.
    let auth = authenticate_id(&f, &f.nyxbot_thread).await;
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
        assert_eq!(data.get_str("agent_role").unwrap(), "subagent");
        assert_eq!(data.get_str("conversation_id").unwrap(), f.row.id);
        assert!(!data.contains_key("arguments"));
    }
    assert_eq!(logs.count_documents(filter).await.unwrap(), 2);
}

#[tokio::test]
async fn subagents_request_platform_services_and_execute_after_allow() {
    use crate::models::{
        assistant_acknowledgement::COLLECTION_NAME as ACKS,
        assistant_conversation::COLLECTION_NAME as CONVERSATIONS,
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
    let _ = CONVERSATIONS;
    let team_id = f.nyxbot_thread.clone();
    // A subagent asks its orchestrator for the platform service; the
    // orchestrator itself runs with Full access and uses it directly.
    for orchestrator in [false, true] {
        let auth = if orchestrator {
            authenticate_id(&f, &team_id).await
        } else {
            authenticate(&f).await
        };
        let expected = if orchestrator {
            "granted"
        } else {
            "acknowledgement_required"
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
        if !orchestrator {
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
                "the orchestrator creates no cards"
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
    let progress = &row.active_turn.as_ref().unwrap().tool_progress;
    assert_eq!(progress.calls, 3);
    assert_eq!(progress.digest.len(), 64);
    let encoded_progress = serde_json::to_string(progress).unwrap();
    assert!(!encoded_progress.contains("do_not_record") && !encoded_progress.contains("issues"));
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
    assert_eq!(row.active_turn.as_ref().unwrap().tool_progress.calls, 3);
    assert_eq!(row.active_turn.unwrap().activities.len(), 3);
}

#[tokio::test]
async fn chat_tool_images_become_mcp_image_content_and_owner_only_turn_attachments() {
    use crate::models::assistant_attachment::COLLECTION_NAME as ATTACHMENTS;
    let f = fixture("chat_tool_images").await;
    // A camera-like service: one PNG snapshot, one image-typed body that is not an image.
    let png: Vec<u8> = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 64]].concat();
    let body = png.clone();
    let upstream = Router::new()
        .route(
            "/snapshot",
            any(move || {
                let body = body.clone();
                async move { ([("content-type", "image/png")], body) }
            }),
        )
        .route(
            "/spoofed",
            any(|| async { ([("content-type", "image/png")], "<html>not an image</html>") }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let service = connected(&f.state.db, &f.owner, "lobby-camera", &address).await;
    let auth = authenticate(&f).await;
    let services = load_all_services_for_meta_tools(&f.state, &auth)
        .await
        .unwrap();
    let tool = services
        .iter()
        .find(|s| s.service_id == service)
        .map(|s| format!("{}__{}", s.service_slug, s.endpoints[0].name))
        .unwrap();
    let refusal = result(
        call(
            &f,
            &auth,
            &tool,
            json!({"method": "GET", "path": "/snapshot"}),
        )
        .await,
        true,
    )
    .await;
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
    let raw = |response: Response| async move {
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice::<Value>(&bytes).unwrap()["result"].clone()
    };
    let image = raw(call(
        &f,
        &auth,
        &tool,
        json!({"method": "GET", "path": "/snapshot"}),
    )
    .await)
    .await;
    assert_eq!(image["isError"], false, "{image}");
    // Chat keys get the note only: NyxAgent stringifies and truncates results
    // and cannot pass pixels to its model, so base64 would crowd out the note.
    let content = image["content"].as_array().unwrap();
    assert_eq!(content.len(), 1, "{image}");
    assert_eq!(content[0]["type"], "text");
    let note = content[0]["text"].as_str().unwrap();
    assert!(
        note.starts_with("The tool returned an image (image/png"),
        "{note}"
    );
    assert!(
        note.contains("already displays this image to the user"),
        "{note}"
    );
    assert!(
        note.contains("Do not say it could not be displayed"),
        "{note}"
    );
    // Small enough that NyxAgent's 10,000-character truncation never reaches the note.
    assert!(image.to_string().len() < 1024, "{image}");
    // A body that claims image/png without PNG magic stays text.
    let spoofed = raw(call(
        &f,
        &auth,
        &tool,
        json!({"method": "GET", "path": "/spoofed"}),
    )
    .await)
    .await;
    assert_eq!(spoofed["content"].as_array().unwrap().len(), 1, "{spoofed}");
    assert_eq!(spoofed["content"][0]["type"], "text");

    // The image is attached to the live turn, encrypted at rest, and owner-only.
    let row = crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    let attachments = row.active_turn.as_ref().unwrap().attachments.clone();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].content_type, "image/png");
    assert_eq!(attachments[0].size, png.len() as i64);
    assert_eq!(attachments[0].label, tool);
    let stored = f
        .state
        .db
        .collection::<mongodb::bson::Document>(ATTACHMENTS)
        .find_one(doc! {"_id": &attachments[0].id})
        .await
        .unwrap()
        .unwrap();
    let ciphertext = stored.get_binary_generic("data_encrypted").unwrap();
    assert!(
        !ciphertext.windows(8).any(|w| w == &png[..8]),
        "stored encrypted"
    );
    let response = crate::handlers::assistant_nyxagent::attachment(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((f.row.id.clone(), attachments[0].id.clone())),
    )
    .await
    .unwrap();
    assert_eq!(response.headers()["content-type"], "image/png");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let served = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(served.to_vec(), png);
    let stranger = uuid::Uuid::new_v4().to_string();
    assert!(matches!(
        crate::handlers::assistant_nyxagent::attachment(
            axum::extract::State(f.state.clone()),
            crate::test_utils::test_auth_user(&stranger),
            axum::extract::Path((f.row.id.clone(), attachments[0].id.clone())),
        )
        .await,
        Err(crate::errors::AppError::NotFound(_))
    ));

    // Callers outside a chat still get the image block, with nothing stored.
    let plain = McpAuthContext::user(f.owner.clone(), AuthMethod::Session);
    let direct = raw(direct_call(
        &f,
        &plain,
        &tool,
        json!({"method": "GET", "path": "/snapshot"}),
    )
    .await)
    .await;
    // Other MCP callers get the note first, then the image block.
    assert_eq!(direct["content"][0]["type"], "text", "{direct}");
    assert!(
        !direct["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("displays this image")
    );
    assert_eq!(direct["content"][1]["type"], "image");
    assert_eq!(direct["content"][1]["mimeType"], "image/png");
    {
        use base64::Engine as _;
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(direct["content"][1]["data"].as_str().unwrap())
                .unwrap(),
            png
        );
    }
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>(ATTACHMENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    server.abort();
}

async fn mark_guest(f: &Fixture, guest: bool) {
    f.state
        .db
        .collection::<mongodb::bson::Document>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$set": {"guest_turn": guest}},
        )
        .await
        .unwrap();
}

/// NyxBot holds every service of the owner, so a turn for someone else
/// calls no tools at all.
#[tokio::test]
async fn nyxbot_guest_turns_call_no_tools() {
    let f = orchestrator_fixture("chat_mcp_guest_nyxbot").await;
    mark_guest(&f, true).await;
    let auth = authenticate(&f).await;
    assert!(auth.chat.as_ref().unwrap().guest);
    for (name, args) in [
        ("nyx__search_tools", json!({"query": "mail"})),
        ("nyx__list_connected_services", json!({})),
        (
            "nyx__call_tool",
            json!({"tool_name": "github__list_repos", "arguments_json": "{}"}),
        ),
        ("nyxid__list_agent_keys", json!({})),
        ("github__list_repos", json!({})),
    ] {
        let refused = result(direct_call(&f, &auth, name, args).await, true).await;
        assert_eq!(refused["error"], "owner_only", "{name}: {refused}");
        assert!(
            refused["instructions"]
                .as_str()
                .unwrap()
                .contains("no tools"),
            "{name}: {refused}"
        );
    }
    // The owner's next turn has everything back.
    mark_guest(&f, false).await;
    let auth = authenticate(&f).await;
    let listed = direct_call(&f, &auth, "nyxid__list_channel_chats", json!({})).await;
    let bytes = axum::body::to_bytes(listed.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("owner_only"));
}

/// A specialist's turn for someone other than the owner only discovers and
/// reads: account, team, memory, connection and Oracle tools and every
/// service change are refused, whichever way they are called.
#[tokio::test]
async fn specialist_guest_turns_never_use_owner_tools_or_ssh() {
    let f = fixture("chat_mcp_guest").await;
    mark_guest(&f, true).await;
    let auth = authenticate(&f).await;
    assert!(auth.chat.as_ref().unwrap().guest);
    for (name, args) in [
        ("nyxid__list_agent_keys", json!({})),
        (
            "nyxid__remember",
            json!({"text": "the owner's secret plan"}),
        ),
        ("nyxid__post_to_chat", json!({"chat_id": "c", "text": "hi"})),
        ("nyx__connect_service", json!({"service": "github"})),
        ("nyx__oracle_pools", json!({})),
    ] {
        let refused = result(direct_call(&f, &auth, name, args).await, true).await;
        assert_eq!(refused["error"], "owner_only", "{name}: {refused}");
    }
    // Through the universal proxy tool too.
    let refused = result(
        call(&f, &auth, "nyxid__list_agent_keys", json!({})).await,
        true,
    )
    .await;
    assert_eq!(refused["error"], "owner_only");
    // Discovery still works.
    let search = handle_meta_search(
        &f.state,
        &auth,
        None,
        &json!({"query": "list"}),
        None,
        false,
    )
    .await;
    let bytes = axum::body::to_bytes(search.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("owner_only"));
    // SSH (a shell can do anything) is the owner's; HTTP operations pass on
    // to the usual checks (what guests may do with a service is decided
    // before, from the owner's guest access).
    let target = crate::services::mcp_approval::McpApprovalTarget {
        service_id: uuid::Uuid::new_v4().to_string(),
        service_name: "Example".into(),
        service_slug: "example".into(),
        service_owner_user_id: f.owner.clone(),
        is_auto_connected: false,
    };
    let delete = operation_descriptor::build_mcp_descriptor("DELETE", "/items/1", None);
    let ssh = operation_descriptor::build_ssh_descriptor(
        operation_descriptor::SshOperationKind::Exec,
        Some("ls"),
    );
    let refused = authorize_mcp_operation(&f.state, &auth, target.clone(), &ssh, Some(json!(1)))
        .await
        .unwrap_err();
    assert_eq!(result(refused, true).await["error"], "owner_only");
    authorize_mcp_operation(&f.state, &auth, target.clone(), &delete, Some(json!(1)))
        .await
        .unwrap_or_else(|_| panic!("DELETE"));
    for method in ["GET", "POST", "PUT", "PATCH"] {
        let operation =
            operation_descriptor::build_mcp_descriptor(method, "/api/services/light/turn_on", None);
        authorize_mcp_operation(&f.state, &auth, target.clone(), &operation, Some(json!(1)))
            .await
            .unwrap_or_else(|_| panic!("{method}"));
    }
}

/// A specialist's guest turns use its granted services as far as the owner
/// lets guests (read, use without changing or deleting, or all), refused before anything
/// is sent, and never ask the owner to approve or run on the owner's
/// approvals: a guest's request would look like the owner's.
#[tokio::test]
async fn specialist_guests_use_a_granted_service_as_far_as_the_owner_lets_them() {
    let f = fixture("chat_mcp_guest_calls").await;
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
    let service = connected(&f.state.db, &f.owner, "home", &address).await;
    // The owner grants the service to the specialist.
    let auth = authenticate(&f).await;
    let search = result(
        handle_meta_search(
            &f.state,
            &auth,
            None,
            &json!({"query": "home"}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    let name = search["matches"][0]["name"].as_str().unwrap().to_string();
    let ask = result(
        call(&f, &auth, &name, json!({"method": "GET", "path": "/ok"})).await,
        true,
    )
    .await;
    acks::decide(
        &f.state.db,
        &f.owner,
        &f.row.id,
        ask["acknowledgement_id"].as_str().unwrap(),
        true,
    )
    .await
    .unwrap();
    mark_guest(&f, true).await;
    let guest = authenticate(&f).await;
    // By default guests look things up, create and act, but never change or
    // remove what exists (PUT, PATCH, DELETE), whatever the path says.
    for args in [
        json!({"method": "DELETE", "path": "/items/1"}),
        json!({"method": "PUT", "path": "/items/1", "body": {"name": "x"}}),
        json!({"method": "PATCH", "path": "/items/1", "body": {"name": "x"}}),
    ] {
        let refused = result(call(&f, &guest, &name, args.clone()).await, true).await;
        assert_eq!(refused["error"], "owner_only", "{args}");
        assert_eq!(refused["guest_access"], "use", "{args}");
    }
    // And never send a method override, whichever way it points.
    for args in [
        json!({"method": "POST", "path": "/items/1?_method=DELETE"}),
        json!({"method": "POST", "path": "/items/1", "body": {"_method": "delete"}}),
        json!({"method": "DELETE", "path": "/items/1", "query": "_method=GET"}),
        json!({"method": "POST", "path": "/items/1", "query": ".method=DELETE"}),
    ] {
        let refused = result(call(&f, &guest, &name, args.clone()).await, true).await;
        assert_eq!(refused["error"], "owner_only", "{args}");
        assert!(
            refused["instructions"]
                .as_str()
                .unwrap()
                .contains("method override"),
            "{args}"
        );
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    for args in [
        json!({"method": "GET", "path": "/api/states"}),
        json!({"method": "POST", "path": "/api/services/light/turn_on",
            "body": {"entity_id": "light.office"}}),
        json!({"method": "POST", "path": "/api/services/remove_note/run"}),
    ] {
        let used = result(call(&f, &guest, &name, args.clone()).await, false).await;
        assert_eq!(used["ok"], true, "{args}");
    }
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    // The owner lets guests only look things up...
    set_guest_access(&f, &service, GuestAccess::Read).await;
    let refused = result(
        call(
            &f,
            &guest,
            &name,
            json!({"method": "POST", "path": "/api/services/light/turn_on"}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(refused["guest_access"], "read");
    // Nor passes a change off as a read with an override.
    let refused = result(
        call(
            &f,
            &guest,
            &name,
            json!({"method": "POST", "path": "/api/services/light/turn_on",
                "body": {"_method": "GET"}}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(refused["error"], "owner_only");
    let used = result(
        call(
            &f,
            &guest,
            &name,
            json!({"method": "GET", "path": "/api/states"}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(used["ok"], true);
    assert_eq!(hits.load(Ordering::SeqCst), 4);
    // ...or do everything the specialist may.
    set_guest_access(&f, &service, GuestAccess::All).await;
    let used = result(
        call(
            &f,
            &guest,
            &name,
            json!({"method": "DELETE", "path": "/items/1"}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(used["ok"], true);
    assert_eq!(hits.load(Ordering::SeqCst), 5);
    // Approvals stay the owner's at every level.
    set_guest_access(&f, &service, GuestAccess::All).await;
    // A service the owner has put behind approval: the guest is refused and
    // no approval request reaches the owner.
    let now = chrono::Utc::now();
    f.state
        .db
        .collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::service_approval_config::ServiceApprovalConfig {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: f.owner.clone(),
                service_id: service.clone(),
                service_name: "home".into(),
                approval_required: true,
                approval_mode: Default::default(),
                rules: Vec::new(),
                default_effect: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    let refused = result(
        call(
            &f,
            &guest,
            &name,
            json!({"method": "GET", "path": "/api/states"}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(refused["error"], "owner_only");
    assert_eq!(hits.load(Ordering::SeqCst), 5);
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::approval_request::COLLECTION_NAME,
            )
            .count_documents(doc! {"service_id": &service})
            .await
            .unwrap(),
        0
    );
    // An approval the owner granted for their own requests lets the owner in,
    // never a guest sharing the chat's key.
    f.state
        .db
        .collection::<mongodb::bson::Document>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .update_one(
            doc! {"service_id": &service},
            doc! {"$set": {"approval_mode": "grant"}},
        )
        .await
        .unwrap();
    let now = chrono::Utc::now();
    f.state
        .db
        .collection::<crate::models::approval_grant::ApprovalGrant>(
            crate::models::approval_grant::COLLECTION_NAME,
        )
        .insert_one(crate::models::approval_grant::ApprovalGrant {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: f.owner.clone(),
            service_id: service.clone(),
            service_name: "home".into(),
            requester_type: guest.approval_requester_type().unwrap().to_string(),
            requester_id: guest.approval_requester_id(),
            requester_label: None,
            approval_request_id: uuid::Uuid::new_v4().to_string(),
            scope: None,
            granted_at: now,
            expires_at: now + chrono::Duration::days(1),
            revoked: false,
            org_scoped: false,
        })
        .await
        .unwrap();
    let refused = result(
        call(
            &f,
            &guest,
            &name,
            json!({"method": "GET", "path": "/api/states"}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(refused["error"], "owner_only");
    assert_eq!(hits.load(Ordering::SeqCst), 5);
    mark_guest(&f, false).await;
    let owner = authenticate(&f).await;
    let used = result(
        call(
            &f,
            &owner,
            &name,
            json!({"method": "GET", "path": "/api/states"}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(used["ok"], true);
    assert_eq!(hits.load(Ordering::SeqCst), 6);
    // Audit rows say whose turn it was.
    let guests = f
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .count_documents(doc! {
            "event_type": "assistant_mcp_tool_call",
            "event_data.conversation_id": &f.row.id,
            "event_data.guest": true,
        })
        .await
        .unwrap();
    assert!(guests >= 7, "{guests}");
    server.abort();
}

async fn set_guest_access(f: &Fixture, service: &str, access: GuestAccess) {
    crate::services::assistant_team_service::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        crate::services::assistant_team_service::GrantChange::Guests(
            [(service.to_string(), access)].into(),
        ),
    )
    .await
    .unwrap();
}

/// The owner's services as a guest's key sees them, with this service's
/// operation metadata adjusted.
async fn services_for(
    f: &Fixture,
    auth: &McpAuthContext,
    service: &str,
    adjust: impl Fn(&mut mcp_service::McpDurableEndpointMetadata),
) -> Vec<mcp_service::McpToolService> {
    let mut services = load_all_services_for_meta_tools(&f.state, auth)
        .await
        .unwrap();
    let target = services
        .iter_mut()
        .find(|candidate| candidate.service_id == service)
        .unwrap();
    let endpoint_id = target.endpoints[0].endpoint_id.clone();
    adjust(
        target
            .durable_endpoint_metadata
            .entry(endpoint_id)
            .or_default(),
    );
    services
}

/// Guest access is judged from the operation's spec: what it marks as
/// deleting or replacing data is beyond "use", and what its stored catalog
/// contract marks read-only is a read even as a POST (a remote spec may only
/// narrow); a method override is never sent.
#[tokio::test]
async fn guest_access_follows_spec_markers() {
    let f = fixture("chat_mcp_guest_markers").await;
    let service = connected(&f.state.db, &f.owner, "home", "http://127.0.0.1:9").await;
    crate::services::assistant_team_service::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        crate::services::assistant_team_service::GrantChange::Add(
            crate::models::assistant_agent::AgentGrants {
                service_ids: vec![service.clone()],
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    set_guest_access(&f, &service, GuestAccess::All).await;
    mark_guest(&f, true).await;
    let guest = authenticate(&f).await;
    // An operation its spec marks destructive (deletes or overwrites) is
    // beyond "use" too, whatever its method.
    let mut services = load_all_services_for_meta_tools(&f.state, &guest)
        .await
        .unwrap();
    let marked = services
        .iter_mut()
        .find(|candidate| candidate.service_id == service)
        .unwrap();
    let endpoint_id = marked.endpoints[0].endpoint_id.clone();
    marked
        .durable_endpoint_metadata
        .entry(endpoint_id)
        .or_default()
        .destructive = true;
    let marked = services
        .iter()
        .find(|candidate| candidate.service_id == service)
        .unwrap();
    let endpoint = &marked.endpoints[0];
    let prepared = mcp_service::prepare_proxy_tool_call(
        marked,
        endpoint,
        &json!({"method": "POST", "path": "/sheet/values"}),
    )
    .unwrap();
    assert!(
        guest_service_refusal(&f.state, &guest, marked, endpoint, &prepared, None)
            .await
            .is_none()
    );
    set_guest_access(&f, &service, GuestAccess::Use).await;
    assert!(
        guest_service_refusal(&f.state, &guest, marked, endpoint, &prepared, None)
            .await
            .is_some()
    );
    // The owner's own turns are never limited by guest access.
    mark_guest(&f, false).await;
    let owner = authenticate(&f).await;
    assert!(
        guest_service_refusal(&f.state, &owner, marked, endpoint, &prepared, None)
            .await
            .is_none()
    );
    mark_guest(&f, true).await;
    let guest = authenticate(&f).await;
    // An operation its spec marks read-only is a read, even as a POST (a
    // search); a method override still counts.
    let mut services = load_all_services_for_meta_tools(&f.state, &guest)
        .await
        .unwrap();
    let search = services
        .iter_mut()
        .find(|candidate| candidate.service_id == service)
        .unwrap();
    let endpoint_id = search.endpoints[0].endpoint_id.clone();
    let metadata = search
        .durable_endpoint_metadata
        .entry(endpoint_id.clone())
        .or_default();
    metadata.risk = Some(crate::models::service_endpoint::EndpointRisk::Read);
    // As a stored catalog contract says.
    metadata.catalog_contract = true;
    let search = services
        .iter()
        .find(|candidate| candidate.service_id == service)
        .unwrap();
    let endpoint = &search.endpoints[0];
    set_guest_access(&f, &service, GuestAccess::Read).await;
    for (args, refused) in [
        (
            json!({"method": "POST", "path": "/search", "body": {"q": "lights"}}),
            false,
        ),
        (
            json!({"method": "POST", "path": "/search", "body": {"_method": "DELETE"}}),
            true,
        ),
    ] {
        let prepared = mcp_service::prepare_proxy_tool_call(search, endpoint, &args).unwrap();
        assert_eq!(
            guest_service_refusal(&f.state, &guest, search, endpoint, &prepared, None)
                .await
                .is_some(),
            refused,
            "{args}"
        );
    }
    // NyxID's marker says what a method does not: a PUT that only acts is
    // use, a POST that edits is not; a DELETE never is, whatever its spec says.
    set_guest_access(&f, &service, GuestAccess::Use).await;
    for (method, changes, contract, refused) in [
        ("PUT", Some(false), true, false),
        // "Only acts" widens: a remote spec cannot say it.
        ("PUT", Some(false), false, true),
        ("PUT", None, true, true),
        ("POST", Some(true), false, true),
        ("DELETE", Some(false), true, true),
    ] {
        let mut marked = services_for(&f, &guest, &service, |metadata| {
            metadata.changes_existing = changes;
            metadata.catalog_contract = contract;
        })
        .await;
        let search = marked
            .iter_mut()
            .find(|candidate| candidate.service_id == service)
            .unwrap();
        let endpoint = &search.endpoints[0];
        let prepared = mcp_service::prepare_proxy_tool_call(
            search,
            endpoint,
            &json!({"method": method, "path": "/player/play"}),
        )
        .unwrap();
        assert_eq!(
            guest_service_refusal(&f.state, &guest, search, endpoint, &prepared, None)
                .await
                .is_some(),
            refused,
            "{method} {changes:?} {contract}"
        );
    }
    // A read-only DELETE row is still a DELETE.
    set_guest_access(&f, &service, GuestAccess::Read).await;
    let marked = services_for(&f, &guest, &service, |metadata| {
        metadata.risk = Some(crate::models::service_endpoint::EndpointRisk::Read);
        metadata.catalog_contract = true;
    })
    .await;
    let search = marked
        .iter()
        .find(|candidate| candidate.service_id == service)
        .unwrap();
    let endpoint = &search.endpoints[0];
    let prepared = mcp_service::prepare_proxy_tool_call(
        search,
        endpoint,
        &json!({"method": "DELETE", "path": "/items/1"}),
    )
    .unwrap();
    assert!(
        guest_service_refusal(&f.state, &guest, search, endpoint, &prepared, None)
            .await
            .is_some()
    );
    // A remote spec read at call time may say read-only too, but only narrows.
    let mut remote = services;
    remote
        .iter_mut()
        .find(|candidate| candidate.service_id == service)
        .unwrap()
        .durable_endpoint_metadata
        .get_mut(&endpoint_id)
        .unwrap()
        .catalog_contract = false;
    let search = remote
        .iter()
        .find(|candidate| candidate.service_id == service)
        .unwrap();
    let endpoint = &search.endpoints[0];
    let prepared = mcp_service::prepare_proxy_tool_call(
        search,
        endpoint,
        &json!({"method": "POST", "path": "/search", "body": {"q": "lights"}}),
    )
    .unwrap();
    assert!(
        guest_service_refusal(&f.state, &guest, search, endpoint, &prepared, None)
            .await
            .is_some()
    );
}

/// A guest never widens what a specialist may use: an ungranted service is
/// refused without a permission request for NyxBot to grant.
#[tokio::test]
async fn guests_never_ask_for_more_access() {
    let f = fixture("chat_guest_gate").await;
    let mut chat = f.chat.clone();
    chat.guest = true;
    let (value, request) =
        acks::service_gate(&f.state.db, &chat, "service-1", "example", "Example", false)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(value["error"], "owner_only");
    assert!(request.is_none());
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::assistant_acknowledgement::COLLECTION_NAME,
            )
            .count_documents(doc! {"conversation_id": &f.row.id})
            .await
            .unwrap(),
        0
    );
    // The owner's own turn still asks.
    chat.guest = false;
    let (_, request) =
        acks::service_gate(&f.state.db, &chat, "service-1", "example", "Example", false)
            .await
            .unwrap()
            .unwrap();
    assert!(request.is_some());
}

#[tokio::test]
async fn webhook_calls_require_exact_action_cards_from_http_and_catalog_effects() {
    use crate::models::trigger_schedule::ConfirmationPolicy;
    let f = orchestrator_fixture("webhook_action_cards").await;
    let service_id = connected(
        &f.state.db,
        &f.owner,
        "webhook-target",
        "http://127.0.0.1:9",
    )
    .await;
    let mut auth = authenticate(&f).await;
    auth.chat.as_mut().unwrap().confirmation_policy = Some(ConfirmationPolicy::Changes);
    f.state
        .db
        .collection::<mongodb::bson::Document>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .update_one(
            doc! {"_id": &f.row.id},
            doc! {"$set": {"active_turn.trigger_run_id": "webhook-run"}},
        )
        .await
        .unwrap();
    let mut services = load_all_services_for_meta_tools(&f.state, &auth)
        .await
        .unwrap();
    let service = services
        .iter_mut()
        .find(|candidate| candidate.service_id == service_id)
        .unwrap();
    let endpoint = service.endpoints.remove(0);
    let tool = "webhook-target__proxy";
    let args = json!({"method": "PATCH", "path": "/settings", "body": {"value": true}});
    let prepared = mcp_service::prepare_proxy_tool_call(service, &endpoint, &args).unwrap();
    let response = webhook_service_gate(
        &f.state,
        &auth,
        service,
        &endpoint,
        &prepared,
        tool,
        &args,
        Some(json!(1)),
    )
    .await
    .unwrap();
    let card = result(response, true).await;
    assert_eq!(card["error"], "acknowledgement_required");
    let id = card["acknowledgement_id"].as_str().unwrap();
    let stored = acks::history(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(stored[0].status, "pending");
    assert_eq!(stored[0].trigger_run_id.as_deref(), Some("webhook-run"));
    let get = json!({"method": "GET", "path": "/settings"});
    let prepared_get = mcp_service::prepare_proxy_tool_call(service, &endpoint, &get).unwrap();
    assert!(
        webhook_service_gate(
            &f.state,
            &auth,
            service,
            &endpoint,
            &prepared_get,
            tool,
            &get,
            None
        )
        .await
        .is_none()
    );
    acks::decide(&f.state.db, &f.owner, &f.row.id, id, true)
        .await
        .unwrap();
    let mut approved = args.clone();
    approved["acknowledgement_id"] = json!(id);
    assert_eq!(webhook_execution_arguments(&auth, &approved), args);
    let schema = webhook_tool_schema(&auth, &json!({"type": "object", "properties": {}}));
    assert_eq!(schema["properties"]["acknowledgement_id"]["type"], "string");
    assert!(
        webhook_service_gate(
            &f.state, &auth, service, &endpoint, &prepared, tool, &approved, None
        )
        .await
        .is_none()
    );
    assert!(
        webhook_service_gate(
            &f.state, &auth, service, &endpoint, &prepared, tool, &approved, None
        )
        .await
        .is_some()
    );
    auth.chat.as_mut().unwrap().confirmation_policy = Some(ConfirmationPolicy::Destructive);
    assert!(
        webhook_service_gate(
            &f.state, &auth, service, &endpoint, &prepared, tool, &args, None
        )
        .await
        .is_none()
    );
    service
        .durable_endpoint_metadata
        .entry(endpoint.endpoint_id.clone())
        .or_default()
        .destructive = true;
    assert!(
        webhook_service_gate(
            &f.state, &auth, service, &endpoint, &prepared, tool, &args, None
        )
        .await
        .is_some()
    );
}

#[tokio::test]
async fn webhook_native_changes_require_owner_cards_even_with_skip_destructive() {
    use crate::models::trigger_schedule::ConfirmationPolicy;
    let f = orchestrator_fixture("webhook_native_cards").await;
    let mut auth = authenticate(&f).await;
    let chat = auth.chat.as_mut().unwrap();
    chat.confirmation_policy = Some(ConfirmationPolicy::Changes);
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            skip_destructive_confirmation: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (card, refused) = crate::handlers::assistant_team::execute_tool(
        &f.state,
        chat,
        "nyxid__remember",
        &json!({"text": "Attacker supplied memory"}),
    )
    .await;
    assert!(refused);
    assert_eq!(card["error"], "acknowledgement_required");
    let cards = acks::history(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    assert_eq!(cards[0].decider, "user");
    chat.confirmation_policy = Some(ConfirmationPolicy::Destructive);
    let (result, refused) = crate::handlers::assistant_team::execute_tool(
        &f.state,
        chat,
        "nyxid__remember",
        &json!({"text": "Owner allowed ordinary changes"}),
    )
    .await;
    assert!(!refused, "{result}");
}

#[tokio::test]
async fn assistant_operation_scopes_block_direct_universal_raw_and_guest_bypasses() {
    use crate::models::{
        agent_operation_scope::OperationSelection, assistant_agent::AgentGrants,
        downstream_service::ProxyOperationRule,
    };
    use crate::services::{
        agent_operation_scope_service as scopes, assistant_team_service as team,
    };
    use tower::ServiceExt;
    let f = fixture("operation_scope_paths").await;
    crate::test_utils::set_agent_operation_scopes_enabled(&f.state.db, &f.owner, true).await;
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let upstream = Router::new().route(
        "/{*path}",
        any(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { Json(json!({"ok":true})) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let id = connected(&f.state.db, &f.owner, "scoped-http", &address).await;
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    scopes::set(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &id,
        &OperationSelection {
            expected_revision: 0,
            all_operations: false,
            endpoint_ids: vec![],
            rules: vec![
                ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/items/{id}".into(),
                    ..Default::default()
                },
                ProxyOperationRule {
                    method: "DELETE".into(),
                    path_template: "/items/{id}".into(),
                    ..Default::default()
                },
            ],
        },
        true,
    )
    .await
    .unwrap();
    // Disabling configuration during rollout/rollback never disables enforcement.
    crate::test_utils::set_agent_operation_scopes_enabled(&f.state.db, &f.owner, false).await;
    crate::services::service_pool_service::create_pool(
        &f.state.db,
        &f.owner,
        crate::services::service_pool_service::CreatePoolInput {
            slug: "scoped-pool".into(),
            name: "Scoped pool".into(),
            description: None,
            strategy: Default::default(),
            tier_balance: Default::default(),
            member_contract: Default::default(),
            failover: None,
            is_active: None,
            members: vec![crate::models::service_pool::ServicePoolMember {
                user_service_id: id.clone(),
                weight: 1,
                enabled: true,
                priority: 0,
                model: None,
                same_api_compatible: true,
                health_reset_generation: 0,
            }],
        },
    )
    .await
    .unwrap();
    let auth = authenticate(&f).await;
    let services = load_all_services_for_meta_tools(&f.state, &auth)
        .await
        .unwrap();
    let service = services.iter().find(|s| s.service_id == id).unwrap();
    let tool = format!("{}__{}", service.service_slug, service.endpoints[0].name);
    for universal in [false, true] {
        let args = json!({"method":"POST","path":"/items/1"});
        let response = if universal {
            call(&f, &auth, &tool, args).await
        } else {
            direct_call(&f, &auth, &tool, args).await
        };
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["result"]["isError"], true);
        assert!(
            value["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Allowed operations")
        );
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert_eq!(
        result(
            call(&f, &auth, &tool, json!({"method":"GET","path":"/items/1"})).await,
            false
        )
        .await["ok"],
        true
    );
    let mut webhook = auth.clone();
    webhook.chat.as_mut().unwrap().confirmation_policy =
        Some(crate::models::trigger_schedule::ConfirmationPolicy::Changes);
    let card = result(
        call(
            &f,
            &webhook,
            &tool,
            json!({"method":"DELETE","path":"/items/1"}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(card["kind"], "action");
    assert_eq!(card["decider"], "user");
    let before = hits.load(Ordering::SeqCst);
    mark_guest(&f, true).await;
    let guest = authenticate(&f).await;
    let refused = result(
        call(
            &f,
            &guest,
            &tool,
            json!({"method":"DELETE","path":"/items/1"}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(refused["error"], "owner_only");
    mark_guest(&f, false).await;
    let credential = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let (_, router) = crate::routes::build_router_with_state(f.state.clone());
    let router = router.with_state(f.state.clone());
    for route in [
        format!("/api/v1/proxy/{id}"),
        "/api/v1/proxy/s/scoped-http".into(),
        "/api/v1/proxy/s/scoped-pool".into(),
    ] {
        for (method, path, override_header) in [
            ("POST", "/items/1", false),
            ("HEAD", "/items/1", false),
            ("GET", "/Items/1", false),
            ("GET", "/items/1/", false),
            ("GET", "/items//1", false),
            ("GET", "/items/%2e%2e/secret", false),
            ("GET", "/items/a%2fb", false),
            ("GET", "/items/%252e%252e", false),
            ("GET", "/items/1?_method=DELETE", false),
            ("GET", "/items/1", true),
        ] {
            let mut request = axum::http::Request::builder()
                .method(method)
                .uri(format!("{route}{path}"))
                .header(
                    "authorization",
                    format!("Bearer {}", credential.raw_key.as_str()),
                );
            if override_header {
                request = request.header("x-http-method-override", "DELETE");
            }
            let response = router
                .clone()
                .oneshot(request.body(axum::body::Body::empty()).unwrap())
                .await
                .unwrap();
            assert!(
                response.status().is_client_error(),
                "{method} {path}: {}",
                response.status()
            );
        }
    }
    let ws = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/proxy/s/scoped-http/items/1")
                .header(
                    "authorization",
                    format!("Bearer {}", credential.raw_key.as_str()),
                )
                .header("connection", "upgrade")
                .header("upgrade", "websocket")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ws.status(), axum::http::StatusCode::FORBIDDEN);
    let relay = crate::crypto::jwt::generate_relay_access_token(
        &f.state.jwt_keys,
        &f.state.config,
        &uuid::Uuid::parse_str(&f.owner).unwrap(),
        "proxy llm:proxy",
        None,
        &crate::crypto::jwt::RelayAgentScope {
            api_key_id: f.chat.api_key_id.clone(),
            api_key_name: "test".into(),
            allowed_service_ids: vec![id.clone()],
            allowed_node_ids: vec![],
            allow_all_services: false,
            allow_all_nodes: false,
        },
    )
    .unwrap();
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/proxy/s/scoped-http/items/1")
                .header("authorization", format!("Bearer {relay}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    let mut relay_headers = HeaderMap::new();
    relay_headers.insert("authorization", format!("Bearer {relay}").parse().unwrap());
    let relay_auth = authenticate_mcp(&f.state, &relay_headers, false)
        .await
        .unwrap();
    assert_eq!(
        relay_auth.assistant_operation_scopes,
        auth.assistant_operation_scopes
    );
    // Node routing cannot outrank the policy, even before a node is online.
    f.state
        .db
        .collection::<mongodb::bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"node_id":uuid::Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/proxy/s/scoped-http/items/1")
                .header(
                    "authorization",
                    format!("Bearer {}", credential.raw_key.as_str()),
                )
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    assert_eq!(hits.load(Ordering::SeqCst), before);
    server.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_operation_scopes_require_owner_card_even_when_skip_destructive_is_enabled() {
    use crate::models::{agent_operation_scope::OperationSelection, assistant_agent::AgentGrants};
    use crate::services::{
        agent_operation_scope_service as scopes, assistant_team_service as team,
    };
    let f = fixture("operation_scope_card").await;
    crate::test_utils::set_agent_operation_scopes_enabled(&f.state.db, &f.owner, true).await;
    let id = connected(&f.state.db, &f.owner, "scoped-card", "http://127.0.0.1:9").await;
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    scopes::set(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &id,
        &OperationSelection {
            expected_revision: 0,
            all_operations: false,
            endpoint_ids: vec![],
            rules: vec![],
        },
        true,
    )
    .await
    .unwrap();
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            skip_destructive_confirmation: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let auth = authenticate_id(&f, &f.nyxbot_thread).await;
    let args = json!({"subagent":"worker","service_id":id,"selection":{"expected_revision":1,"all_operations":true}});
    let card = result(
        call(&f, &auth, "nyxid__set_agent_operations", args.clone()).await,
        true,
    )
    .await;
    assert_eq!(card["kind"], "action");
    assert_eq!(card["decider"], "user");
    let key = f
        .state
        .db
        .collection::<crate::models::api_key::ApiKey>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.row.credential_api_key_id})
        .await
        .unwrap()
        .unwrap();
    assert!(key.assistant_operation_scopes.contains_key(&id));
    let ack = card["acknowledgement_id"].as_str().unwrap();
    acks::decide(&f.state.db, &f.owner, &f.nyxbot_thread, ack, true)
        .await
        .unwrap();
    let mut confirmed = args;
    confirmed["acknowledgement_id"] = json!(ack);
    let _ = result(
        call(&f, &auth, "nyxid__set_agent_operations", confirmed.clone()).await,
        false,
    )
    .await;
    let replay = result(
        call(&f, &auth, "nyxid__set_agent_operations", confirmed).await,
        true,
    )
    .await;
    assert_eq!(replay["error"], "acknowledgement_invalid");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_operation_scopes_hide_typed_tools_and_preserve_guest_and_webhook_fences() {
    use crate::models::{
        agent_operation_scope::OperationSelection, assistant_agent::AgentGrants,
        service_endpoint::ServiceEndpoint,
    };
    use crate::services::{
        agent_operation_scope_service as scopes, assistant_team_service as team,
    };
    let f = fixture("operation_catalog").await;
    crate::test_utils::set_agent_operation_scopes_enabled(&f.state.db, &f.owner, true).await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    catalog.base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/items", any(|| async { Json(json!({"ok":true})) })),
        )
        .await
        .unwrap()
    });
    catalog.slug = "scoped-catalog".into();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    let mut ids = Vec::new();
    for (name, method) in [("read", "GET"), ("write", "DELETE")] {
        let id = uuid::Uuid::new_v4().to_string();
        let endpoint: ServiceEndpoint = mongodb::bson::from_document(doc! {
            "_id":&id,"service_id":&catalog.id,"name":name,"method":method,"path":"/items",
            "is_active":true,"created_at":mongodb::bson::DateTime::now(),"updated_at":mongodb::bson::DateTime::now()
        }).unwrap();
        f.state
            .db
            .collection(crate::models::service_endpoint::COLLECTION_NAME)
            .insert_one(endpoint)
            .await
            .unwrap();
        ids.push(id);
    }
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            platform_service_ids: vec![catalog.id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    let selection = OperationSelection {
        expected_revision: 0,
        all_operations: false,
        endpoint_ids: vec![ids[0].clone()],
        rules: vec![],
    };
    scopes::set(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &catalog.id,
        &selection,
        true,
    )
    .await
    .unwrap();
    let auth = authenticate(&f).await;
    let services = load_all_services_for_meta_tools(&f.state, &auth)
        .await
        .unwrap();
    let visible = services
        .iter()
        .find(|row| row.service_id == catalog.id)
        .unwrap();
    assert_eq!(visible.endpoints.len(), 1);
    assert_eq!(visible.endpoints[0].endpoint_id, ids[0]);
    let listed = handle_tools_list(
        &f.state,
        &auth,
        None,
        &JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "tools/list".into(),
            params: None,
        },
    )
    .await;
    let bytes = axum::body::to_bytes(listed.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let listed: Value = serde_json::from_slice(&bytes).unwrap();
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == "scoped-catalog__read")
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "scoped-catalog__write")
    );
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == "nyx__attachment_read")
    );
    // Operation scopes restrict service operations, never a thread's native
    // attachment tools. Exercise discovery plus both typed and generic calls.
    Box::pin(assert_scoped_thread_attachment_access(&f, &auth)).await;
    for query in ["scoped catalog", "write", "DELETE items"] {
        let searched = result(
            direct_call(&f, &auth, "nyx__search_tools", json!({"query":query})).await,
            false,
        )
        .await;
        assert!(
            searched["matches"]
                .as_array()
                .unwrap()
                .iter()
                .all(|tool| tool["name"] != "scoped-catalog__write")
        );
    }
    for universal in [true, false] {
        let denied = if universal {
            call(&f, &auth, "scoped-catalog__write", json!({})).await
        } else {
            direct_call(&f, &auth, "scoped-catalog__write", json!({})).await
        };
        let bytes = axum::body::to_bytes(denied.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["result"]["isError"], true);
        assert!(
            value["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Allowed operations")
        );
    }
    assert_eq!(
        result(
            direct_call(&f, &auth, "scoped-catalog__read", json!({})).await,
            false
        )
        .await["ok"],
        true
    );
    let credential = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let (_, router) = crate::routes::build_router_with_state(f.state.clone());
    let router = router.with_state(f.state.clone());
    use tower::ServiceExt;
    for (method, status) in [("GET", 200), ("DELETE", 403), ("HEAD", 403)] {
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(method)
                    .uri(format!("/api/v1/proxy/{}/items", catalog.id))
                    .header(
                        "authorization",
                        format!("Bearer {}", credential.raw_key.as_str()),
                    )
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
    }
    // Raw calls obey the same guest approval fence as MCP: neither create an
    // approval card nor spend a grant previously given to the owner.
    let now = chrono::Utc::now();
    f.state
        .db
        .collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::service_approval_config::ServiceApprovalConfig {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: f.owner.clone(),
                service_id: catalog.id.clone(),
                service_name: catalog.name.clone(),
                approval_required: true,
                approval_mode: crate::models::service_approval_config::ApprovalMode::Grant,
                rules: vec![],
                default_effect: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    set_guest_access(&f, &catalog.id, GuestAccess::All).await;
    mark_guest(&f, true).await;
    for with_owner_grant in [false, true] {
        if with_owner_grant {
            f.state
                .db
                .collection::<crate::models::approval_grant::ApprovalGrant>(
                    crate::models::approval_grant::COLLECTION_NAME,
                )
                .insert_one(crate::models::approval_grant::ApprovalGrant {
                    id: uuid::Uuid::new_v4().to_string(),
                    user_id: f.owner.clone(),
                    service_id: catalog.id.clone(),
                    service_name: catalog.name.clone(),
                    requester_type: auth.approval_requester_type().unwrap().to_string(),
                    requester_id: auth.approval_requester_id(),
                    requester_label: None,
                    approval_request_id: uuid::Uuid::new_v4().to_string(),
                    scope: None,
                    granted_at: now,
                    expires_at: now + chrono::Duration::days(1),
                    revoked: false,
                    org_scoped: false,
                })
                .await
                .unwrap();
        }
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            router.clone().oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/proxy/{}/items", catalog.id))
                    .header(
                        "authorization",
                        format!("Bearer {}", credential.raw_key.as_str()),
                    )
                    .body(axum::body::Body::empty())
                    .unwrap(),
            ),
        )
        .await
        .expect("guest must not wait for an owner approval")
        .unwrap();
        assert_eq!(response.status().as_u16(), 403);
    }
    assert_eq!(f.state.db.collection::<mongodb::bson::Document>(
        crate::models::approval_request::COLLECTION_NAME,
    ).count_documents(doc! {"service_id": &catalog.id}).await.unwrap(), 0);
    mark_guest(&f, false).await;
    // A stored endpoint edit cannot widen its pinned method/path contract.
    f.state
        .db
        .collection::<mongodb::bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .update_one(doc! {"_id":&ids[0]}, doc! {"$set":{"path":"/admin"}})
        .await
        .unwrap();
    assert!(
        !load_all_services_for_meta_tools(&f.state, &auth)
            .await
            .unwrap()
            .iter()
            .any(|row| row.service_id == catalog.id)
    );
    let bad = OperationSelection {
        expected_revision: 1,
        endpoint_ids: vec![],
        rules: vec![crate::models::downstream_service::ProxyOperationRule {
            method: "GET".into(),
            path_template: "/admin".into(),
            ..Default::default()
        }],
        ..selection
    };
    assert!(
        scopes::set(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            &catalog.id,
            &bad,
            true
        )
        .await
        .is_err()
    );
    server.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_operation_scopes_specialist_request_cannot_silently_widen() {
    use crate::models::{agent_operation_scope::OperationSelection, assistant_agent::AgentGrants};
    use crate::services::{
        agent_operation_scope_service as scopes, assistant_team_service as team,
    };
    let f = fixture("operation_permission").await;
    crate::test_utils::set_agent_operation_scopes_enabled(&f.state.db, &f.owner, true).await;
    let id = connected(
        &f.state.db,
        &f.owner,
        "requested-operations",
        "http://127.0.0.1:9",
    )
    .await;
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    scopes::set(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &id,
        &OperationSelection {
            expected_revision: 0,
            all_operations: false,
            endpoint_ids: vec![],
            rules: vec![],
        },
        true,
    )
    .await
    .unwrap();
    let auth = authenticate(&f).await;
    let args = json!({"subagent":"worker","service_id":id,"selection":{"expected_revision":1,"all_operations":true}});
    let request = result(
        call(&f, &auth, "nyxid__request_agent_operations", args.clone()).await,
        true,
    )
    .await;
    assert_eq!(request["decider"], "orchestrator");
    let ack = request["acknowledgement_id"].as_str().unwrap();
    assert!(
        acks::decide_as(
            &f.state.db,
            &f.owner,
            None,
            ack,
            true,
            acks::Decider::Nyxbot,
            None
        )
        .await
        .is_err()
    );
    let agent = team::live_specialist(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert!(agent.operation_scopes[&id].operations.is_empty());
    let own = result(
        call(&f, &auth, "nyxid__set_agent_operations", args).await,
        true,
    )
    .await;
    assert!(own.get("error").is_some());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_operation_scopes_rollout_gate_blocks_configuration_and_pending_requests() {
    use crate::errors::AppError;
    use crate::models::{agent_operation_scope::OperationSelection, assistant_agent::AgentGrants};
    use crate::services::assistant_team_service as team;
    use crate::test_utils::set_agent_operation_scopes_enabled as rollout;
    use axum::extract::Path;
    let f = fixture("operation_scope_rollout").await;
    let id = connected(&f.state.db, &f.owner, "rollout-scope", "http://127.0.0.1:9").await;
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    let owner_auth = crate::test_utils::test_auth_user(&f.owner);
    let selection: OperationSelection = serde_json::from_value(json!({
        "expected_revision":0,"all_operations":false,
        "rules":[{"method":"GET","path_template":"/items/{id}"}]
    }))
    .unwrap();
    let denied = crate::handlers::assistant_team::set_agent_operations(
        State(f.state.clone()),
        owner_auth.clone(),
        Path((f.chat.agent_id.clone(), id.clone())),
        Json(selection.clone()),
    )
    .await;
    assert!(
        matches!(denied, Err(AppError::ValidationError(message)) if message.contains("not enabled yet"))
    );
    let nyxbot = authenticate_id(&f, &f.nyxbot_thread).await;
    let specialist = authenticate(&f).await;
    let args = json!({"subagent":"worker","service_id":id,"selection":selection});
    for (auth, tool) in [
        (&nyxbot, "nyxid__set_agent_operations"),
        (&specialist, "nyxid__request_agent_operations"),
    ] {
        let refusal = result(call(&f, auth, tool, args.clone()).await, true).await;
        assert!(refusal.to_string().contains("not enabled yet"));
        assert!(refusal.get("acknowledgement_id").is_none());
    }
    assert_eq!(
        f.state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::assistant_acknowledgement::COLLECTION_NAME
            )
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let agent = team::live_specialist(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert!(agent.operation_scopes.is_empty());
    assert!(agent.operation_scope_revisions.is_empty());

    rollout(&f.state.db, &f.owner, true).await;
    let saved = crate::handlers::assistant_team::set_agent_operations(
        State(f.state.clone()),
        owner_auth,
        Path((f.chat.agent_id.clone(), id.clone())),
        Json(selection),
    )
    .await
    .unwrap();
    assert_eq!(saved.0["revision"], 1);
    let narrow = json!({"subagent":"worker","service_id":id,
        "selection":{"expected_revision":1,"all_operations":false}});
    result(
        call(&f, &nyxbot, "nyxid__set_agent_operations", narrow).await,
        false,
    )
    .await;
    let pending = result(call(&f, &specialist, "nyxid__request_agent_operations", json!({
        "subagent":"worker","service_id":id,"selection":{"expected_revision":2,"all_operations":false}
    })).await, true).await;
    let ack = pending["acknowledgement_id"].as_str().unwrap();
    rollout(&f.state.db, &f.owner, false).await;
    let refused = result(
        call(
            &f,
            &nyxbot,
            "nyxid__decide_permission",
            json!({
                "request_id":ack,"decision":"allow","reason":"Allow the requested narrowing"
            }),
        )
        .await,
        true,
    )
    .await;
    assert!(refused.to_string().contains("not enabled yet"));
    assert!(refused.get("acknowledgement_id").is_none());
    let pending = f
        .state
        .db
        .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
            crate::models::assistant_acknowledgement::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":ack})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.status, "pending");
    let agent = team::live_specialist(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert_eq!(agent.operation_scope_revisions[&id], 2);
    assert!(agent.operation_scopes[&id].operations.is_empty());
    let key = f
        .state
        .db
        .collection::<crate::models::api_key::ApiKey>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id":&f.row.credential_api_key_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(key.assistant_operation_scopes, agent.operation_scopes);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn specialist_machine_update_dispatch_preserves_one_owner_card() {
    use crate::services::assistant_team_service as team;
    for universal in [false, true] {
        let f = fixture("mcp_specialist_update_card").await;
        let node = crate::services::machine_integration_tests::node(&f, &f.owner).await;
        team::set_grants(
            &f.state.db,
            &f.owner,
            &f.chat.agent_id,
            team::GrantChange::Machine {
                base: Box::new(team::GrantChange::Add(Default::default())),
                machines: Some(vec![node.id.clone()]),
                logins: None,
                mode: team::MachineGrantMode::Add,
            },
        )
        .await
        .unwrap();
        crate::services::assistant_settings_service::update(
            &f.state.db,
            &f.owner,
            crate::services::assistant_settings_service::Update {
                skip_destructive_confirmation: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut auth = authenticate(&f).await;
        auth.chat.as_mut().unwrap().confirmation_policy =
            Some(crate::models::trigger_schedule::ConfirmationPolicy::Changes);
        let invoke = |args| async {
            if universal {
                call(&f, &auth, "nyxid__machine_update", args).await
            } else {
                direct_call(&f, &auth, "nyxid__machine_update", args).await
            }
        };
        let card = result(invoke(json!({"machine":node.id})).await, true).await;
        assert_eq!(card["kind"], "action");
        assert_eq!(card["decider"], "user");
        let id = card["acknowledgement_id"].as_str().unwrap();
        assert_eq!(
            acks::history(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap()
                .len(),
            1
        );
        acks::decide(&f.state.db, &f.owner, &f.row.id, id, true)
            .await
            .unwrap();
        let finished = result(
            invoke(json!({"machine":node.id,"acknowledgement_id":id})).await,
            false,
        )
        .await;
        assert_eq!(finished["status"], "manual_step");
        // The tool's exact card satisfies the webhook gate as well; dispatch
        // must not add a second generic confirmation before or after it.
        assert_eq!(
            acks::history(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap()
                .len(),
            1
        );
        f.state.db.drop().await.unwrap();
    }
}
