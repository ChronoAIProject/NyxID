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

// Keep the fixture future's construction outside these test drivers as well as
// its polling; boxing at an await site still leaves a large debug temporary.
fn boxed_chat_fixture(name: &'static str) -> futures::future::BoxFuture<'static, Fixture> {
    Box::pin(fixture(name))
}

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
    serde_json::from_str(body["result"]["content"][0]["text"].as_str().unwrap()).unwrap_or_else(
        |error| panic!("Expected structured tool result: {error}; envelope: {body}"),
    )
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

async fn webhook_policy(f: &Fixture, policy: crate::models::trigger_schedule::ConfirmationPolicy) {
    let now = bson::DateTime::now();
    f.state.db.collection::<bson::Document>(crate::models::trigger_run::COLLECTION_NAME)
        .replace_one(doc! {"_id":"webhook-run"}, doc! {
            "_id":"webhook-run","trigger_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.owner,
            "scheduled_at":now,"deadline":now,"outcome":"started","confirmation_policy":bson::to_bson(&policy).unwrap(),
            "thread_id":&f.row.id,"turn_id":&f.row.active_turn.as_ref().unwrap().turn_id,
            "fence":"test","lease_until":now,"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::hours(4)),
        }).upsert(true).await.unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn.trigger_run_id":"webhook-run"}},
        )
        .await
        .unwrap();
}

/// Exercise the real resolved MCP executor; user connections are gated there,
/// after the operation and exact destination have been resolved.
#[allow(clippy::too_many_arguments)]
async fn execute_webhook_connection(
    state: &AppState,
    auth: &McpAuthContext,
    service: &mcp_service::McpToolService,
    endpoint: &mcp_service::McpToolEndpoint,
    _prepared: &mcp_service::PreparedProxyCall,
    _tool: &str,
    args: &Value,
    request_id: Option<Value>,
) -> Result<(), Box<Response>> {
    let prepared = mcp_service::prepare_proxy_tool_call(
        service,
        endpoint,
        &webhook_execution_arguments(auth, args),
    )
    .unwrap()
    .with_acknowledgement(args["acknowledgement_id"].as_str());
    let exec = mcp_exec_context(auth);
    let response = Box::pin(mcp_service::execute_tool_response(
        &state.http_client,
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &state.billing,
        &auth.user_id,
        auth.billing_principal_user_id(),
        service,
        endpoint,
        prepared,
        &state.jwt_keys,
        &state.config,
        &state.connection_expiry_notifier,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        &exec,
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
    ))
    .await
    .expect("resolved webhook execution");
    if (200..300).contains(&response.status) {
        Ok(())
    } else {
        Err(Box::new(tool_result(request_id, &response.text, true)))
    }
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
    // Once the turn ends, the same key cannot make an unrecorded edit.
    crate::services::assistant_nyxagent::force_stop(
        &f.state.db,
        &f.owner,
        &f.row.id,
        &f.row.active_turn.as_ref().unwrap().turn_id,
    )
    .await
    .unwrap();
    let auth = authenticate(&f).await;
    let refused = direct_call(
        &f,
        &auth,
        "nyxid__set_service_enabled",
        json!({"service_id": service, "enabled": false}),
    )
    .await;
    let body = axum::body::to_bytes(refused.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        body["error"]["message"],
        "Assistant turn is no longer active"
    );
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
    let f = boxed_chat_fixture("chat_mcp_allow").await;
    check_mcp_allow(f).await;
}

// Keep fixture construction out of this measured large scenario's poll frame.
fn check_mcp_allow(f: Fixture) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
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
        let orchestrator =
            crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &team_id)
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
                    .collection::<mongodb::bson::Document>(
                        crate::models::audit_log::COLLECTION_NAME,
                    )
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
    })
}

#[tokio::test]
async fn chat_mcp_native_account_and_action_refusals_are_tool_results() {
    let f = fixture("chat_mcp_account").await;
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::Nyxmode),
            ..Default::default()
        },
    )
    .await
    .unwrap();
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
            data_scope: None,
            cost_class: None,
            execution: Default::default(),
            publication: Default::default(),
            async_operation: None,
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
            // Specialist chat keys cannot mint hosted connect links, even in routed mode.
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
            assert!(String::from_utf8_lossy(&bytes).contains("access_request_denied"));
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
    // One entry per tools/call; compiled native tools establish label provenance.
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
async fn b4_duplicate_action_fence_through_tools_call() {
    let f = fixture("mcp_retry_fence").await;
    let auth = authenticate(&f).await;
    let failed_turn = f.row.active_turn.as_ref().unwrap().turn_id.clone();
    // Turn 1 completes a search, then fails.
    let first = direct_call(
        &f,
        &auth,
        "nyx__search_tools",
        json!({"query": "issues", "limit": 3}),
    )
    .await;
    let first = axum::body::to_bytes(first.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let first: Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(first["result"]["isError"], false, "{first}");
    crate::services::assistant_nyxagent::finish_turn(
        &f.state.db,
        &f.row,
        &f.row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &crate::services::assistant_nyxagent::TurnResult {
            text: String::new(),
            session_id: None,
            response_id: None,
            error: Some(crate::services::assistant_nyxagent::TurnError::new(
                "idle_timeout",
            )),
        },
    )
    .await
    .unwrap();
    // Turn 2 is the owner's Retry of turn 1.
    let retry = crate::services::assistant_nyxagent::begin_turn(
        &f.state.db,
        &f.owner,
        &crate::services::assistant_nyxagent::TurnRequest {
            acknowledgement_id: None,
            retry_of: Some(failed_turn.clone()),
            attachment_ids: Vec::new(),
            agent_id: None,
            conversation_id: Some(f.row.id.clone()),
            text: "Please manage my account".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let auth = authenticate(&f).await;
    let refused = |response: Response| async move {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["result"]["isError"], true, "{body}");
        let text: Value =
            serde_json::from_str(body["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text["error"], "retry_duplicate_action");
    };
    // The same call is refused, including with its arguments in another key order.
    refused(
        direct_call(
            &f,
            &auth,
            "nyx__search_tools",
            json!({"query": "issues", "limit": 3}),
        )
        .await,
    )
    .await;
    refused(
        direct_call(
            &f,
            &auth,
            "nyx__search_tools",
            json!({"limit": 3, "query": "issues"}),
        )
        .await,
    )
    .await;
    let live = crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    let turn = live.active_turn.unwrap();
    assert_eq!(turn.turn_id, retry.active_turn.as_ref().unwrap().turn_id);
    assert_eq!(
        turn.tool_progress.calls, 0,
        "a refused duplicate is not progress"
    );
    assert_eq!(turn.activities.len(), 2, "each refusal is recorded");
    assert!(
        turn.activities
            .iter()
            .all(|a| a.label == "nyx__search_tools" && a.status == "completed")
    );
    let audits: Vec<mongodb::bson::Document> = f
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find(doc! {"event_type": "assistant_mcp_tool_call", "event_data.error_code": "retry_duplicate_action"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(audits.len(), 2);
    let data = audits[0].get_document("event_data").unwrap();
    assert_eq!(data.get_str("tool").unwrap(), "nyx__search_tools");
    assert_eq!(data.get_str("conversation_id").unwrap(), f.row.id);
    assert!(data.get_bool("metadata_only").unwrap());
    assert!(
        !audits[0].to_string().contains("issues"),
        "no arguments in audit"
    );
    // A different call, and a call the failed turn never made, still run.
    let different = direct_call(
        &f,
        &auth,
        "nyx__search_tools",
        json!({"query": "calendar", "limit": 3}),
    )
    .await;
    let different = axum::body::to_bytes(different.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&different).unwrap()["result"]["isError"],
        false
    );
    let other = direct_call(&f, &auth, "nyx__list_connected_services", json!({})).await;
    let other = axum::body::to_bytes(other.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&other).unwrap()["result"]["isError"],
        false
    );
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
        note.contains("shows this image to the user under your reply in the NyxID app"),
        "{note}"
    );
    assert!(
        !note.contains("Delivered") && !note.contains("shown below"),
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
        (
            "nyxid__read_tool_result",
            json!({"ref": uuid::Uuid::new_v4().to_string(), "part": 0}),
        ),
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
            execution_review: None,
            execution_last_used_at: None,
            execution_use: None,
            execution: None,
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
    let f = boxed_chat_fixture("chat_mcp_guest_markers").await;
    check_guest_spec(f).await;
}

// Keep fixture construction out of this measured large scenario's poll frame.
fn check_guest_spec(f: Fixture) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
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
    })
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
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .mount(&server)
        .await;
    let service_id = connected(&f.state.db, &f.owner, "webhook-target", &server.uri()).await;
    webhook_policy(&f, ConfirmationPolicy::Changes).await;
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
    let response = execute_webhook_connection(
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
    .unwrap_err();
    let card = result(*response, true).await;
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
        execute_webhook_connection(
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
        .is_ok()
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
        execute_webhook_connection(
            &f.state, &auth, service, &endpoint, &prepared, tool, &approved, None
        )
        .await
        .is_ok()
    );
    assert!(
        execute_webhook_connection(
            &f.state, &auth, service, &endpoint, &prepared, tool, &approved, None
        )
        .await
        .is_err()
    );
    webhook_policy(&f, ConfirmationPolicy::Destructive).await;
    auth = authenticate(&f).await;
    assert!(
        execute_webhook_connection(
            &f.state, &auth, service, &endpoint, &prepared, tool, &args, None
        )
        .await
        .is_ok()
    );
    service
        .durable_endpoint_metadata
        .entry(endpoint.endpoint_id.clone())
        .or_default()
        .destructive = true;
    assert!(
        execute_webhook_connection(
            &f.state, &auth, service, &endpoint, &prepared, tool, &args, None
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn webhook_native_changes_keep_owner_cards_in_auto() {
    use crate::models::trigger_schedule::ConfirmationPolicy;
    let f = orchestrator_fixture("webhook_native_cards").await;
    let mut auth = authenticate(&f).await;
    let chat = auth.chat.as_mut().unwrap();
    chat.confirmation_policy = Some(ConfirmationPolicy::Changes);
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::Nyxmode),
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
    let f = boxed_chat_fixture("operation_scope_paths").await;
    check_scope_paths(f).await;
}

// Keep fixture construction out of this measured large scenario's poll frame.
fn check_scope_paths(f: Fixture) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        use crate::models::{
            agent_operation_scope::OperationSelection, assistant_agent::AgentGrants,
            downstream_service::ProxyOperationRule,
        };
        use crate::services::{
            agent_operation_scope_service as scopes, assistant_team_service as team,
        };
        use tower::ServiceExt;
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
                contract_digest: None,
                inputs: Default::default(),
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
        webhook_policy(
            &f,
            crate::models::trigger_schedule::ConfirmationPolicy::Changes,
        )
        .await;
        let webhook = authenticate(&f).await;
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
    })
}

#[tokio::test]
async fn assistant_operation_scopes_auto_requires_exact_one_use_owner_card() {
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
            contract_digest: None,
            inputs: Default::default(),
        },
        true,
    )
    .await
    .unwrap();
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::Nyxmode),
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
    // The card binds the compiled contract: the retry sends retry_arguments.
    assert!(card["retry_arguments"]["selection"]["contract_digest"].is_string());
    let mut stale = args;
    stale["acknowledgement_id"] = json!(ack);
    let refused = result(
        call(&f, &auth, "nyxid__set_agent_operations", stale).await,
        true,
    )
    .await;
    assert_eq!(refused["error"], "acknowledgement_invalid");
    let mut confirmed = card["retry_arguments"].clone();
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
async fn assistant_operation_scopes_native_input_rules_save_and_enforce_without_partial_write() {
    use crate::models::{
        agent_operation_scope::OperationSelection, assistant_agent::AgentGrants,
        service_endpoint::ServiceEndpoint,
    };
    use crate::services::{
        agent_operation_scope_service as scopes, assistant_team_service as team,
        proxy_authorization::CanonicalPath,
    };
    let f = fixture("operation_scope_input_rules").await;
    crate::test_utils::set_agent_operation_scopes_enabled(&f.state.db, &f.owner, true).await;
    let service = connected(
        &f.state.db,
        &f.owner,
        "receipt-service",
        "http://127.0.0.1:9",
    )
    .await;
    let endpoint_id = uuid::Uuid::new_v4().to_string();
    let endpoint: ServiceEndpoint = mongodb::bson::from_document(doc! {
        "_id": &endpoint_id,
        "service_id": &service,
        "name": "record_create",
        "description": null,
        "method": "POST",
        "path": "/items/{item_id}",
        "parameters": null,
        "request_body_schema": null,
        "request_content_type": "application/json",
        "request_body_required": true,
        "response_description": null,
        "risk": "write",
        "supports_idempotency_key": false,
        "is_active": true,
        "created_at": mongodb::bson::DateTime::now(),
        "updated_at": mongodb::bson::DateTime::now()
    })
    .unwrap();
    f.state
        .db
        .collection::<ServiceEndpoint>(crate::models::service_endpoint::COLLECTION_NAME)
        .insert_one(endpoint)
        .await
        .unwrap();
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![service.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    scopes::set(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &service,
        &OperationSelection {
            expected_revision: 0,
            all_operations: false,
            endpoint_ids: vec![],
            rules: vec![],
            contract_digest: None,
            inputs: Default::default(),
        },
        true,
    )
    .await
    .unwrap();
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::Nyxmode),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let auth = authenticate_id(&f, &f.nyxbot_thread).await;
    let input_rules = json!({
        "path": {"item_id": {"type": "exact", "value": "receipt"}},
        "body": {"type": "object", "properties": {
            "fields": {"type": "object", "required": ["Type"], "properties": {
                "Type": {"type": "one_of", "values": ["Expense", "Refund"]}
            }}
        }, "required": ["fields"]}
    });
    let mut inputs = serde_json::Map::new();
    inputs.insert(endpoint_id.clone(), input_rules.clone());
    let args = json!({
        "subagent": "worker",
        "service_id": service,
        "selection": {
            "expected_revision": 1,
            "all_operations": false,
            "endpoint_ids": [&endpoint_id],
            "inputs": inputs
        }
    });
    let card = result(
        call(&f, &auth, "nyxid__set_agent_operations", args.clone()).await,
        true,
    )
    .await;
    assert_eq!(card["kind"], "action");
    let acknowledgement_id = card["acknowledgement_id"].as_str().unwrap();
    acks::decide(
        &f.state.db,
        &f.owner,
        &f.nyxbot_thread,
        acknowledgement_id,
        true,
    )
    .await
    .unwrap();
    let mut retry = card["retry_arguments"].clone();
    retry["acknowledgement_id"] = json!(acknowledgement_id);
    result(
        call(&f, &auth, "nyxid__set_agent_operations", retry).await,
        false,
    )
    .await;

    let agent = team::live_specialist(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    let saved_scope = agent.operation_scopes.get(&service).unwrap();
    assert_eq!(saved_scope.revision, 2);
    assert_eq!(
        saved_scope.operations[0].inputs,
        Some(serde_json::from_value(input_rules.clone()).unwrap())
    );

    let path = CanonicalPath::from_mcp_literal("/items/receipt").unwrap();
    let scopes_map = agent.operation_scopes.clone();
    assert!(
        scopes::check_inputs(
            &scopes_map,
            &service,
            None,
            Some(&endpoint_id),
            "POST",
            &path,
            None,
            br#"{"fields":{"Type":"Expense"}}"#,
            Some("application/json"),
        )
        .is_ok()
    );
    assert!(
        scopes::check_inputs(
            &scopes_map,
            &service,
            None,
            Some(&endpoint_id),
            "POST",
            &path,
            None,
            br#"{"fields":{"Type":"Other"}}"#,
            Some("application/json"),
        )
        .is_err()
    );

    let mut invalid_inputs = serde_json::Map::new();
    invalid_inputs.insert(
        endpoint_id.clone(),
        json!({"body": {
            "type": "object",
            "properties": {"fields": {"type": "string"}}
        }}),
    );
    let invalid = json!({
        "subagent": "worker",
        "service_id": service,
        "selection": {
            "expected_revision": 2,
            "endpoint_ids": [&endpoint_id],
            "inputs": invalid_inputs
        }
    });
    let refused = result(
        call(&f, &auth, "nyxid__set_agent_operations", invalid).await,
        true,
    )
    .await;
    assert_eq!(refused["error"], "validation_error");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("selection.inputs")
    );
    let unchanged = team::live_specialist(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert_eq!(unchanged.operation_scopes[&service], *saved_scope);
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
        contract_digest: None,
        inputs: Default::default(),
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
                    execution_review: None,
                    execution_last_used_at: None,
                    execution_use: None,
                    execution: None,
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
            contract_digest: None,
            inputs: Default::default(),
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
async fn specialist_machine_update_dispatch_auto_preserves_one_owner_card() {
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
                access_mode: Some(crate::models::assistant_settings::AccessMode::Nyxmode),
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

#[tokio::test]
async fn assistant_skills_card_is_one_use_and_cannot_be_bypassed() {
    use crate::models::catalog_skill_revision::SkillReference;
    use sha2::{Digest, Sha256};
    use std::io::{Cursor, Write};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };
    let f = fixture("assistant_skills_cards").await;
    let upstream = MockServer::start().await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "ornn-api".into();
    catalog.base_url = upstream.uri();
    catalog.identity_propagation_mode = "jwt".into();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("SKILL.md", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"Pinned guidance only").unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let id = uuid::Uuid::new_v4().to_string();
    let pin = SkillReference {
        source: "ornn".into(),
        skill_id: id.clone(),
        name: "card-skill".into(),
        version: "1.0".into(),
        sha256: hex::encode(Sha256::digest(&bytes)),
        dependencies: vec![],
        disabled: false,
    };
    Mock::given(method("GET")).and(path(format!("/api/v1/skills/{id}"))).and(query_param("version","1.0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"guid":id,"name":pin.name,"version":"1.0","description":"Bounded guidance","skillHash":pin.sha256}}))).mount(&upstream).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/closure")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"items":[]}})))
        .mount(&upstream)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/skills/{id}/versions/1.0/download")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
        .mount(&upstream)
        .await;
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::Nyxmode),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let auth = authenticate_id(&f, &f.nyxbot_thread).await;
    let args = json!({"agent":f.chat.agent_id,"selection":{"expected_revision":0,"skills":[pin]}});
    let card = result(
        call(&f, &auth, "nyxid__set_agent_skills", args.clone()).await,
        true,
    )
    .await;
    assert_eq!(card["kind"], "action");
    assert_eq!(card["decider"], "user");
    let ack = card["acknowledgement_id"].as_str().unwrap();
    acks::decide(&f.state.db, &f.owner, &f.nyxbot_thread, ack, true)
        .await
        .unwrap();
    let mut confirmed = args.clone();
    confirmed["acknowledgement_id"] = json!(ack);
    let mut tampered = confirmed.clone();
    tampered["selection"]["skills"][0]["version"] = json!("2.0");
    let invalid = result(
        direct_call(&f, &auth, "nyxid__set_agent_skills", tampered).await,
        true,
    )
    .await;
    assert_eq!(invalid["error"], "acknowledgement_invalid");
    let applied = result(
        call(&f, &auth, "nyxid__set_agent_skills", confirmed.clone()).await,
        false,
    )
    .await;
    assert_eq!(applied["revision"], 1);
    let replay = result(
        call(&f, &auth, "nyxid__set_agent_skills", confirmed).await,
        true,
    )
    .await;
    assert!(replay.get("error").is_some());
    let specialist = authenticate(&f).await;
    let read = result(
        call(&f, &specialist, "nyxid__skill_read", json!({"skill":id})).await,
        false,
    )
    .await;
    assert_eq!(read["content"], "Pinned guidance only");
    let unbound = result(
        call(
            &f,
            &specialist,
            "nyxid__skill_read",
            json!({"skill": uuid::Uuid::new_v4().to_string()}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(unbound["error"], "forbidden");
    // A concrete personal Ornn connection is deliberately absent from the
    // specialist grant. Bound skill reads may use this exact owner connection
    // after the server has verified the pin; ordinary Ornn calls still require
    // a grant.
    let endpoint_id = uuid::Uuid::new_v4().to_string();
    let connection_id = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user_endpoint::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user_endpoint(
            &endpoint_id,
            &f.owner,
            "Ornn",
            &upstream.uri(),
            None,
            Some(&catalog.id),
        ))
        .await
        .unwrap();
    f.state
        .db
        .collection(crate::models::user_service::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user_service(
            &connection_id,
            &f.owner,
            "ornn-owner",
            &endpoint_id,
            Some(&catalog.id),
            None,
        ))
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &connection_id},
            doc! {"$set": {"identity_propagation_mode": "jwt"}},
        )
        .await
        .unwrap();
    let before_concrete = upstream.received_requests().await.unwrap().len();
    let concrete_auth = authenticate(&f).await;
    assert!(!concrete_auth.allow_all_services);
    assert!(!concrete_auth.allowed_service_ids.contains(&connection_id));
    let concrete_read = result(
        call(&f, &concrete_auth, "nyxid__skill_read", json!({"skill":id})).await,
        false,
    )
    .await;
    assert_eq!(concrete_read["content"], "Pinned guidance only");
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), before_concrete + 1);
    assert!(
        requests
            .last()
            .unwrap()
            .headers
            .contains_key("x-nyxid-identity-token")
    );
    // The actual ordinary HTTP proxy ingress cannot borrow the server marker.
    {
        use tower::ServiceExt;
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
        let response = Box::pin(
            private.with_state(f.state.clone()).oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/api/v1/proxy/{connection_id}/api/v1/skills/{id}/versions/1.0/download"
                    ))
                    .header("x-api-key", credential.raw_key.as_str())
                    .body(axum::body::Body::empty())
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
        assert_eq!(
            upstream.received_requests().await.unwrap().len(),
            requests.len()
        );
    }
    let ordinary_ornn = result(
        call(
            &f,
            &specialist,
            "nyxid__search_agent_skills",
            json!({"query": "private", "page": 1}),
        )
        .await,
        true,
    )
    .await;
    assert!(ordinary_ornn.get("error").is_some());
    f.state
        .db
        .collection::<crate::models::assistant_agent::AssistantAgent>(
            crate::models::assistant_agent::COLLECTION_NAME,
        )
        .update_one(
            doc! { "_id": &f.chat.agent_id },
            doc! { "$set": { "skills.0.disabled": true } },
        )
        .await
        .unwrap();
    let disabled_read = result(
        call(&f, &specialist, "nyxid__skill_read", json!({"skill":id})).await,
        true,
    )
    .await;
    assert_eq!(disabled_read["error"], "forbidden");
    f.state
        .db
        .collection::<crate::models::assistant_agent::AssistantAgent>(
            crate::models::assistant_agent::COLLECTION_NAME,
        )
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$set": {"skills.0.disabled": false}},
        )
        .await
        .unwrap();
    let other = result(
        direct_call(&f, &auth, "nyxid__skill_read", json!({"skill":id})).await,
        true,
    )
    .await;
    assert!(other.get("error").is_some());
    let own = result(
        call(
            &f,
            &specialist,
            "nyxid__set_agent_skills",
            json!({"agent":f.chat.agent_id,"selection":{"expected_revision":1,"skills":[]}}),
        )
        .await,
        true,
    )
    .await;
    assert!(own.get("error").is_some());
    let mut guest = authenticate(&f).await;
    guest.chat.as_mut().unwrap().guest = true;
    let refusal = result(
        direct_call(&f, &guest, "nyxid__skill_read", json!({"skill":id})).await,
        true,
    )
    .await;
    assert!(refusal.get("error").is_some());
    let mut scopes = std::collections::BTreeMap::from([(
        catalog.id.clone(),
        crate::models::agent_operation_scope::AgentOperationScope {
            revision: 1,
            catalog_service_id: Some(catalog.id.clone()),
            operations: vec![crate::models::agent_operation_scope::ScopedOperation {
                rule: crate::models::downstream_service::ProxyOperationRule {
                    method: "GET".into(),
                    path_template: format!("/api/v1/skills/{id}/versions/1.0/download"),
                    ..Default::default()
                },
                ..Default::default()
            }],
        },
    )]);
    f.state
        .db
        .collection::<mongodb::bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$set": {"operation_scopes": mongodb::bson::to_bson(&scopes).unwrap()}},
        )
        .await
        .unwrap();
    let allowed = result(
        call(&f, &specialist, "nyxid__skill_read", json!({"skill": id})).await,
        false,
    )
    .await;
    assert_eq!(allowed["content"], "Pinned guidance only");
    let before_scope = upstream.received_requests().await.unwrap().len();
    scopes.get_mut(&catalog.id).unwrap().operations.clear();
    f.state
        .db
        .collection::<mongodb::bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$set": {"operation_scopes": mongodb::bson::to_bson(&scopes).unwrap()}},
        )
        .await
        .unwrap();
    let scoped = result(
        call(&f, &specialist, "nyxid__skill_read", json!({"skill": id})).await,
        true,
    )
    .await;
    assert!(scoped.get("error").is_some());
    assert_eq!(
        upstream.received_requests().await.unwrap().len(),
        before_scope
    );
    // Revoking the real thread key still fences this internal capability.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$set": {"operation_scopes": doc! {}}},
        )
        .await
        .unwrap();
    for (collection, target) in [
        (
            crate::models::downstream_service::COLLECTION_NAME,
            &catalog.id,
        ),
        (crate::models::user_service::COLLECTION_NAME, &connection_id),
    ] {
        f.state
            .db
            .collection::<bson::Document>(collection)
            .update_one(
                doc! {"_id": target},
                doc! {"$set": {"identity_propagation_mode": "none"}},
            )
            .await
            .unwrap();
    }
    assert!(
        Box::pin(crate::handlers::agent_skills::dispatch(
            &f.state,
            &f.chat,
            "skill_read",
            &json!({"skill": id}),
        ))
        .await
        .is_err()
    );
    assert_eq!(
        upstream.received_requests().await.unwrap().len(),
        before_scope
    );
    for (collection, target) in [
        (
            crate::models::downstream_service::COLLECTION_NAME,
            &catalog.id,
        ),
        (crate::models::user_service::COLLECTION_NAME, &connection_id),
    ] {
        f.state
            .db
            .collection::<bson::Document>(collection)
            .update_one(
                doc! {"_id": target},
                doc! {"$set": {"identity_propagation_mode": "jwt"}},
            )
            .await
            .unwrap();
    }
    f.state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.chat.api_key_id},
            doc! {"$set": {"is_active": false}},
        )
        .await
        .unwrap();
    assert!(
        Box::pin(crate::handlers::agent_skills::dispatch(
            &f.state,
            &f.chat,
            "skill_read",
            &json!({"skill": id}),
        ))
        .await
        .is_err()
    );
    assert_eq!(
        upstream.received_requests().await.unwrap().len(),
        before_scope
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.chat.api_key_id},
            doc! {"$set": {"is_active": true}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &catalog.id},
            doc! {"$set": {"is_active": false}},
        )
        .await
        .unwrap();
    let unavailable = result(
        call(&f, &specialist, "nyxid__skill_read", json!({"skill": id})).await,
        true,
    )
    .await;
    assert!(unavailable.get("error").is_some());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let records: Vec<crate::models::audit_log::AuditLog> = f
                .state
                .db
                .collection(crate::models::audit_log::COLLECTION_NAME)
                .find(doc! {"event_type": "assistant_skill_read_refused"})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            if ["skill_not_bound", "grant_refused", "operation_restriction"]
                .iter()
                .all(|reason| {
                    records
                        .iter()
                        .any(|row| row.event_data == Some(json!({"reason": reason})))
                })
            {
                assert!(
                    records.iter().all(|row| row
                        .event_data
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .len()
                        == 1)
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    // Removing needs neither an owner card nor an available Ornn.
    upstream.reset().await;
    let removed = result(
        call(
            &f,
            &auth,
            "nyxid__set_agent_skills",
            json!({"agent":f.chat.agent_id,"selection":{"expected_revision":1,"skills":[]}}),
        )
        .await,
        false,
    )
    .await;
    assert_eq!(removed["revision"], 2);
    assert_eq!(removed["skills"], json!([]));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_skills_specialist_permission_is_advisory_and_agent_scoped() {
    let f = fixture("assistant_skill_request").await;
    let specialist = authenticate(&f).await;
    let args = json!({"agent":f.chat.agent_id,"selection":{"expected_revision":0,"skills":[]}});
    let request = result(
        call(&f, &specialist, "nyxid__request_agent_skills", args).await,
        true,
    )
    .await;
    assert_eq!(request["decider"], "orchestrator");
    let ack = request["acknowledgement_id"].as_str().unwrap();
    acks::decide_as(
        &f.state.db,
        &f.owner,
        None,
        ack,
        true,
        acks::Decider::Nyxbot,
        None,
    )
    .await
    .unwrap();
    let agent =
        crate::services::assistant_team_service::agent(&f.state.db, &f.owner, &f.chat.agent_id)
            .await
            .unwrap();
    assert!(agent.skills.is_empty());
    assert_eq!(agent.skills_revision, 0);
    let by_name = result(
        call(
            &f,
            &specialist,
            "nyxid__request_agent_skills",
            json!({"agent": f.chat.agent_id, "skill": "research-guide", "version": "1.0"}),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(by_name["decider"], "orchestrator");
    assert!(
        by_name["summary"]
            .as_str()
            .unwrap()
            .contains("research-guide")
    );
    let denied = result(
        call(
            &f,
            &specialist,
            "nyxid__get_agent_skills",
            json!({"agent":"nyxbot"}),
        )
        .await,
        true,
    )
    .await;
    assert!(denied.get("error").is_some());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn service_preference_discovery_guest_dense_and_explicit_target_unchanged() {
    let f = boxed_chat_fixture("preference_mcp").await;
    check_preference_discovery(f).await;
}

// Keep fixture construction out of this measured large scenario's poll frame.
struct PreferenceScenario {
    f: Fixture,
    hits: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
    a: String,
    slack: String,
    b: String,
    hidden: String,
    group: String,
}

// Preparation and assertions return before the next phase is polled. Retain the
// same fixture, upstream, snapshots and audit ordering through the final abort.
fn check_preference_discovery(f: Fixture) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        let s = prepare_preference_discovery(f).await;
        let auth = authenticate(&s.f).await;
        let baseline = preference_explicit_call(&s, &auth).await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "tools/list".into(),
            params: None,
        };
        let before = preference_tool_list(&s, &auth, &request).await;
        check_preference_order(&s, &auth).await;
        let after = preference_tool_list(&s, &auth, &request).await;
        assert_eq!(before, after);
        assert_eq!(preference_explicit_call(&s, &auth).await, baseline);
        assert_eq!(s.hits.load(Ordering::SeqCst), 2);
        check_preference_audits(&s, &auth).await;
        check_preference_scoped(&s).await;
        s.server.abort();
    })
}

fn prepare_preference_discovery(
    f: Fixture,
) -> futures::future::BoxFuture<'static, PreferenceScenario> {
    Box::pin(async move {
        let hits = Arc::new(AtomicUsize::new(0));
        let observed = hits.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/{*path}",
                    any(move || {
                        observed.fetch_add(1, Ordering::SeqCst);
                        async { Json(json!({"target":"alpha"})) }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let a = connected(&f.state.db, &f.owner, "alpha", &address).await;
        let slack = connected(&f.state.db, &f.owner, "slack", "https://example.com").await;
        let b = connected(&f.state.db, &f.owner, "beta", "https://example.com").await;
        let hidden = connected(&f.state.db, &f.owner, "hidden", "https://example.com").await;
        let catalog = uuid::Uuid::new_v4().to_string();
        let group = format!("catalog:{catalog}");
        f.state
            .db
            .collection::<bson::Document>("user_services")
            .update_many(
                doc! {"_id":{"$in":[&a,&b,&hidden]}},
                doc! {"$set":{"catalog_service_id":&catalog}},
            )
            .await
            .unwrap();
        let slack_catalog = uuid::Uuid::new_v4().to_string();
        for catalog_id in [&catalog, &slack_catalog] {
            let mut service = crate::test_utils::test_auto_connected_catalog_service();
            service.id = catalog_id.clone();
            service.slug = if catalog_id == &catalog {
                "anthropic"
            } else {
                "slack"
            }
            .into();
            service.requires_user_credential = true;
            f.state
                .db
                .collection(crate::models::downstream_service::COLLECTION_NAME)
                .insert_one(service)
                .await
                .unwrap();
            let now = bson::DateTime::now();
            f.state.db.collection::<bson::Document>("service_endpoints").insert_one(doc! {
                "_id":uuid::Uuid::new_v4().to_string(), "service_id":catalog_id,
                "name":"request", "description":"Request service data", "method":"GET", "path":"/ok",
                "risk":"read", "is_active":true, "created_at":now,"updated_at":now,
            }).await.unwrap();
        }
        f.state
            .db
            .collection::<bson::Document>("user_services")
            .update_one(
                doc! {"_id":&slack},
                doc! {"$set":{"catalog_service_id":&slack_catalog}},
            )
            .await
            .unwrap();
        for (i, id) in [&a, &slack, &b, &hidden].iter().enumerate() {
            f.state.db.collection::<bson::Document>("user_services").update_one(doc! {"_id":*id},doc! {"$set":{"created_at":bson::DateTime::from_millis(2000000000000_i64-i as i64*1000)}}).await.unwrap();
        }
        let revoked = crate::services::user_api_key_service::create_api_key(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            crate::services::user_api_key_service::CreateApiKeyParams {
                label: "revoked fixture",
                credential_type: "bearer",
                credential: "fixture-only",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "revoked",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();
        f.state
            .db
            .collection::<bson::Document>("user_services")
            .update_one(
                doc! {"_id":&hidden},
                doc! {"$set":{"auth_method":"bearer","api_key_id":revoked.id}},
            )
            .await
            .unwrap();
        for id in [&a, &b, &slack] {
            let ask = service_gate(
                &f.state.db,
                &f.chat,
                id,
                if id == &a {
                    "alpha"
                } else if id == &b {
                    "beta"
                } else {
                    "slack"
                },
                "Service",
                false,
            )
            .await
            .unwrap()
            .unwrap();
            acks::decide(
                &f.state.db,
                &f.owner,
                &f.row.id,
                ask["acknowledgement_id"].as_str().unwrap(),
                true,
            )
            .await
            .unwrap();
        }
        PreferenceScenario {
            f,
            hits,
            server,
            a,
            slack,
            b,
            hidden,
            group,
        }
    })
}

fn preference_explicit_call<'a>(
    s: &'a PreferenceScenario,
    auth: &'a McpAuthContext,
) -> futures::future::BoxFuture<'a, Value> {
    Box::pin(async move {
        let f = &s.f;
        let call_args = json!({
            "tool_name":"alpha__request",
            "arguments":{}
        });
        result(
            direct_call(f, auth, "nyx__call_tool", call_args).await,
            false,
        )
        .await
    })
}

fn preference_tool_list<'a>(
    s: &'a PreferenceScenario,
    auth: &'a McpAuthContext,
    request: &'a JsonRpcRequest,
) -> futures::future::BoxFuture<'a, axum::body::Bytes> {
    Box::pin(async move {
        let f = &s.f;
        axum::body::to_bytes(
            handle_tools_list(&f.state, auth, None, request)
                .await
                .into_body(),
            2 * 1024 * 1024,
        )
        .await
        .unwrap()
    })
}

fn check_preference_order<'a>(
    s: &'a PreferenceScenario,
    auth: &'a McpAuthContext,
) -> futures::future::BoxFuture<'a, ()> {
    Box::pin(async move {
        use crate::services::service_preference_service as preferences;
        let f = &s.f;
        let slack = s.slack.clone();
        let b = s.b.clone();
        let hidden = s.hidden.clone();
        let group = s.group.clone();
        preferences::replace_group(
            &f.state.db,
            &f.owner,
            &group,
            &[b.clone(), hidden.clone()],
            0,
        )
        .await
        .unwrap();
        let listed = result(
            direct_call(f, auth, "nyx__list_connected_services", json!({})).await,
            false,
        )
        .await;
        assert_eq!(listed["services"][0]["service_id"], b);
        assert_eq!(listed["services"][0]["preference_rank"], 1);
        assert!(listed.to_string().contains(&hidden));
        assert_eq!(listed["services"][1]["service_id"], slack);
        assert!(listed["services"][1]["preference_rank"].is_null());
        let unavailable = listed["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["service_id"] == hidden)
            .unwrap();
        assert_eq!(unavailable["executable"], false);
        assert_eq!(unavailable["preference_rank"], 2);
        let searched = result(
            direct_call(f, auth, "nyx__search_tools", json!({"query":"request"})).await,
            false,
        )
        .await;
        assert!(
            searched["matches"][0]["name"]
                .as_str()
                .unwrap()
                .starts_with("beta__")
        );
        assert_eq!(searched["matches"][0]["preference_rank"], 1);
        assert!(
            searched["matches"][1]["name"]
                .as_str()
                .unwrap()
                .starts_with("slack__")
        );
        assert!(
            searched["matches"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row.get("executable").is_some())
        );
        assert_eq!(
            searched["matches"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["name"].as_str().unwrap().starts_with("hidden__"))
                .unwrap()["executable"],
            false
        );
        assert!(
            searched["matches"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row.get("preference_rank").is_some())
        );
        let native = result(
            direct_call(
                f,
                auth,
                "nyx__search_tools",
                json!({"query":"uploaded document"}),
            )
            .await,
            false,
        )
        .await;
        let attachment = native["matches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == "nyx__attachment_read")
            .expect("native upload search extra");
        assert_eq!(attachment["preference_rank"], Value::Null);
        assert_eq!(attachment["executable"], true);
        assert_eq!(
            attachment["hint"],
            "Call this native tool directly by name."
        );
    })
}

fn check_preference_audits<'a>(
    s: &'a PreferenceScenario,
    auth: &'a McpAuthContext,
) -> futures::future::BoxFuture<'a, ()> {
    Box::pin(async move {
        let f = &s.f;
        let a = s.a.clone();
        let audits = f.state.db.collection::<crate::models::audit_log::AuditLog>(
            crate::models::audit_log::COLLECTION_NAME,
        );
        let filter = doc! {"event_type":"mcp_tool_call", "event_data.tool":"alpha__request"};
        let events = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let events: Vec<_> = audits
                    .find(filter.clone())
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                if events.len() >= 2 {
                    break events;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("both asynchronous execution audits arrive");
        assert_eq!(events.len(), 2, "no execution retry or extra audit");
        assert_eq!(events[0].event_data, events[1].event_data);
        for event in &events {
            assert_eq!(event.user_id.as_deref(), Some(f.owner.as_str()));
            assert_eq!(event.api_key_id.as_deref(), auth.api_key_id.as_deref());
            assert_eq!(event.event_data.as_ref().unwrap()["service_id"], a);
            assert_eq!(event.event_data.as_ref().unwrap()["via"], "nyx__call_tool");
        }
    })
}

fn check_preference_scoped(s: &PreferenceScenario) -> futures::future::BoxFuture<'_, ()> {
    Box::pin(async move {
        use crate::services::service_preference_service as preferences;
        let f = &s.f;
        let a = s.a.clone();
        let b = s.b.clone();
        let hidden = s.hidden.clone();
        let group = s.group.clone();
        preferences::replace_group(
            &f.state.db,
            &f.owner,
            &group,
            &[hidden.clone(), b.clone()],
            1,
        )
        .await
        .unwrap();
        let scoped_key = crate::services::key_service::create_api_key_with_scope_authorization(
            &f.state.db,
            &f.owner,
            Some(&f.owner),
            "preference-scoped",
            "proxy",
            None,
            None,
            Some(&[a.clone(), b.clone()]),
            Some(&[]),
            Some(false),
            Some(false),
            Some(false),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", scoped_key.full_key.parse().unwrap());
        let scoped = authenticate_mcp(&f.state, &headers, false).await.unwrap();
        let listed = result(
            direct_call(f, &scoped, "nyx__list_connected_services", json!({})).await,
            false,
        )
        .await;
        assert_eq!(listed["services"][0]["service_id"], b);
        assert_eq!(listed["services"][0]["preference_rank"], 1);
        assert!(!listed.to_string().contains(&hidden));
        let relay = crate::crypto::jwt::generate_relay_access_token(
            &f.state.jwt_keys,
            &f.state.config,
            &uuid::Uuid::parse_str(&f.owner).unwrap(),
            "proxy",
            None,
            &crate::crypto::jwt::RelayAgentScope {
                api_key_id: scoped.api_key_id.clone().unwrap(),
                api_key_name: "preference-scoped".into(),
                allowed_service_ids: vec![a.clone(), b.clone()],
                allowed_node_ids: vec![],
                allow_all_services: false,
                allow_all_nodes: false,
            },
        )
        .unwrap();
        let mut relay_headers = HeaderMap::new();
        relay_headers.insert("authorization", format!("Bearer {relay}").parse().unwrap());
        let relay_auth = authenticate_mcp(&f.state, &relay_headers, false)
            .await
            .unwrap();
        assert_eq!(relay_auth.user_id, f.owner);
        let relay_listed = result(
            direct_call(f, &relay_auth, "nyx__list_connected_services", json!({})).await,
            false,
        )
        .await;
        assert_eq!(relay_listed["services"], listed["services"]);
        let relay_search = result(
            direct_call(
                f,
                &relay_auth,
                "nyx__search_tools",
                json!({"query":"request"}),
            )
            .await,
            false,
        )
        .await;
        assert_eq!(relay_search["matches"][0]["preference_rank"], 1);
        assert!(
            relay_search["matches"][0]["name"]
                .as_str()
                .unwrap()
                .starts_with("beta__")
        );
        assert!(!relay_search.to_string().contains(&hidden));
        mark_guest(f, true).await;
        let guest = authenticate(f).await;
        let listed = result(
            direct_call(f, &guest, "nyx__list_connected_services", json!({})).await,
            false,
        )
        .await;
        assert_eq!(listed["services"][0]["service_id"], b);
        assert_eq!(listed["services"][0]["preference_rank"], 1);
        assert!(!listed.to_string().contains(&hidden));
    })
}
#[path = "async_service_operation_tests.rs"]
mod async_service_operation_tests;

#[tokio::test]
async fn specialist_request_tool_is_denied_audited_and_routed_when_allowed() {
    use crate::models::assistant_agent::AgentGrants;
    use crate::models::assistant_settings::SpecialistAccessRequests;
    use crate::services::{assistant_settings_service as settings, assistant_team_service as team};
    let f = fixture("strict_request_tool_audit").await;
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
    let specialist = authenticate(&f).await;
    let args = json!({"subagent":"worker","service_id":id,"selection":{"expected_revision":0,"all_operations":true}});
    settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            specialist_access_requests: Some(SpecialistAccessRequests::Denied),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let denied = result(
        call(
            &f,
            &specialist,
            "nyxid__request_agent_operations",
            args.clone(),
        )
        .await,
        true,
    )
    .await;
    assert_eq!(denied, acks::access_request_denied());
    let audits = f
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME);
    let audit = audits
        .find_one(doc! {"event_type":"assistant_access_request_denied"})
        .await
        .unwrap()
        .expect("denial audit");
    let data = audit.get_document("event_data").unwrap();
    assert_eq!(data.get_str("requester_agent").unwrap(), f.chat.agent_id);
    assert_eq!(data.get_str("thread").unwrap(), f.row.id);
    assert_eq!(data.get_str("key").unwrap(), f.row.credential_api_key_id);
    assert_eq!(data.get_str("target").unwrap(), id);
    assert_eq!(data.get_str("scope").unwrap(), "operations");
    assert_eq!(data.get_str("decision").unwrap(), "denied");
    let cards = f.state.db.collection::<mongodb::bson::Document>(
        crate::models::assistant_acknowledgement::COLLECTION_NAME,
    );
    assert_eq!(
        cards
            .count_documents(doc! {"conversation_id": &f.row.id})
            .await
            .unwrap(),
        0
    );
    settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            specialist_access_requests: Some(SpecialistAccessRequests::RoutedToNyxbot),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let routed = result(
        call(&f, &specialist, "nyxid__request_agent_operations", args).await,
        true,
    )
    .await;
    assert_eq!(routed["error"], "acknowledgement_required");
    assert_eq!(routed["decider"], "orchestrator");
    let ack = routed["acknowledgement_id"].as_str().unwrap();
    assert!(
        cards
            .find_one(doc! {"_id":ack,"conversation_id":&f.row.id})
            .await
            .unwrap()
            .is_some()
    );
    let queued = f.state.db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id": &f.nyxbot_thread, "pending_events": {"$elemMatch": {"text": {"$regex": ack}}}})
        .await.unwrap();
    assert!(queued.is_some(), "NyxBot pending event");
}

#[tokio::test]
async fn specialist_service_card_request_denied_audited_and_routed() {
    use crate::models::assistant_settings::SpecialistAccessRequests;
    use crate::services::assistant_settings_service as settings;
    let f = fixture("strict_service_gate_card").await;
    let specialist = authenticate(&f).await;
    let service = connected(
        &f.state.db,
        &f.owner,
        "strict-service",
        "http://127.0.0.1:9",
    )
    .await;
    settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            specialist_access_requests: Some(SpecialistAccessRequests::Denied),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (denied, card) = acks::service_gate(
        &f.state.db,
        &f.chat,
        &service,
        "strict-service",
        "Strict service",
        false,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(denied["error"], "access_request_denied");
    assert!(card.is_none());
    let audit = f
        .state
        .db
        .collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(
            doc! {"event_type":"assistant_access_request_denied","event_data.scope":"service"},
        )
        .await
        .unwrap()
        .expect("service denial audit");
    let audit_data = audit.get_document("event_data").unwrap();
    assert_eq!(
        audit_data.get_str("requester_agent").unwrap(),
        f.chat.agent_id
    );
    assert_eq!(audit_data.get_str("thread").unwrap(), f.row.id);
    assert_eq!(
        audit_data.get_str("key").unwrap(),
        f.row.credential_api_key_id
    );
    assert_eq!(audit_data.get_str("target").unwrap(), service);
    assert_eq!(audit_data.get_str("scope").unwrap(), "service");
    assert_eq!(audit_data.get_str("decision").unwrap(), "denied");
    settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            specialist_access_requests: Some(SpecialistAccessRequests::RoutedToNyxbot),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (_, card) = acks::service_gate(
        &f.state.db,
        &f.chat,
        &service,
        "strict-service",
        "Strict service",
        false,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(card.is_some());
    let _ = specialist;
}

#[tokio::test]
async fn specialist_connect_service_is_denied_audited_and_routed() {
    use crate::models::assistant_settings::SpecialistAccessRequests;
    use crate::services::assistant_settings_service as settings;
    let f = fixture("strict_connect_link").await;
    let auth = authenticate(&f).await;
    let service = public_connect_catalog(&f, "connect-service").await.id;
    settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            specialist_access_requests: Some(SpecialistAccessRequests::Denied),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let response = handle_meta_connect(
        &f.state,
        &auth,
        None,
        &json!({"service_id":service}),
        None,
        false,
    )
    .await;
    let body = String::from_utf8(
        axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("access_request_denied"));
    assert!(f.state.db.collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME).find_one(doc! {"event_type":"assistant_access_request_denied","event_data.scope":"connect_service"}).await.unwrap().is_some());
    settings::update(
        &f.state.db,
        &f.owner,
        settings::Update {
            specialist_access_requests: Some(SpecialistAccessRequests::RoutedToNyxbot),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let routed = handle_meta_connect(
        &f.state,
        &auth,
        None,
        &json!({"service_id":service}),
        None,
        false,
    )
    .await;
    let routed_body = String::from_utf8(
        axum::body::to_bytes(routed.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(routed_body.contains("access_request_denied"));
    assert!(routed_body.contains("connect-service"));
    assert!(routed_body.contains("NyxBot send one"));
    assert!(!routed_body.contains("/keys"));
    f.state
        .db
        .collection::<bson::Document>("downstream_services")
        .update_one(doc! {"_id":&service}, doc! {"$set":{"service_type":"ssh"}})
        .await
        .unwrap();
    let ssh = result(
        handle_meta_connect(
            &f.state,
            &auth,
            None,
            &json!({"service_id":service}),
            None,
            false,
        )
        .await,
        true,
    )
    .await;
    assert_eq!(ssh["connect_own"]["reason"], "ssh_requires_advanced_setup");
    assert!(
        ssh["instructions"]
            .as_str()
            .unwrap()
            .contains("advanced configuration")
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("connect_links")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn specialist_channel_bot_tools_remain_orchestrator_only() {
    let f = fixture("strict_channel_bot_tools").await;
    let specialist = authenticate(&f).await;
    for tool in [
        "nyxid__channel_bot_setup_link",
        "nyxid__connect_channel_bot",
        "nyxid__link_channel_bot",
    ] {
        let denied = result(call(&f, &specialist, tool, json!({})).await, true).await;
        assert_eq!(denied["error"], "orchestrator_only", "{tool}");
        assert_eq!(
            f.state
                .db
                .collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type":"assistant_access_request_denied"})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn s9_specialist_guest_disabled_pin_denial() {
    let f = fixture("s9_mcp_disabled_pin").await;
    // The specialist already has one attached pin.
    let pin = json!({"source":"ornn","skill_id":uuid::Uuid::new_v4().to_string(),"name":"checklist","version":"1.0","sha256":"a".repeat(64),"dependencies":[]});
    f.state
        .db
        .collection::<mongodb::bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.chat.agent_id},
            doc! {"$set": {"skills": [mongodb::bson::to_bson(&pin).unwrap()], "skills_revision": 0}},
        )
        .await
        .unwrap();
    let mut disabled = pin.clone();
    disabled["disabled"] = json!(true);
    let args =
        json!({"agent":f.chat.agent_id,"selection":{"expected_revision":0,"skills":[disabled]}});
    let stored = || async {
        f.state
            .db
            .collection::<crate::models::assistant_agent::AssistantAgent>(
                crate::models::assistant_agent::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": &f.chat.agent_id})
            .await
            .unwrap()
            .unwrap()
    };
    // The specialist cannot change its own skills; a guest turn cannot either.
    let specialist = authenticate(&f).await;
    let refusal = result(
        call(&f, &specialist, "nyxid__set_agent_skills", args.clone()).await,
        true,
    )
    .await;
    assert_eq!(refusal["error"], "orchestrator_only");
    let mut guest = authenticate(&f).await;
    guest.chat.as_mut().unwrap().guest = true;
    let guest_refusal = result(
        direct_call(&f, &guest, "nyxid__set_agent_skills", args.clone()).await,
        true,
    )
    .await;
    assert_eq!(guest_refusal["error"], "owner_only");
    // NyxBot is refused while the rollout flag is off...
    let owner = authenticate_id(&f, &f.nyxbot_thread).await;
    let flag_off = result(
        call(&f, &owner, "nyxid__set_agent_skills", args.clone()).await,
        true,
    )
    .await;
    assert_eq!(flag_off["error"], "forbidden", "{flag_off}");
    let agent = stored().await;
    assert_eq!(agent.skills_revision, 0);
    assert!(!agent.skills[0].disabled);
    // ...and disables the pin without a card once it is on.
    crate::services::feature_flag_service::set_platform_override(
        &f.state.db,
        crate::services::feature_flag_service::SKILL_PIN_DISABLE_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::User(f.owner.clone()),
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let owner_result = result(
        call(&f, &owner, "nyxid__set_agent_skills", args.clone()).await,
        false,
    )
    .await;
    assert_eq!(owner_result["revision"], 1);
    let agent = stored().await;
    assert_eq!(agent.skills_revision, 1);
    assert!(agent.skills[0].disabled);
    // Turning it back on needs the owner's card.
    let enable =
        json!({"agent":f.chat.agent_id,"selection":{"expected_revision":1,"skills":[pin]}});
    let card = result(
        call(&f, &owner, "nyxid__set_agent_skills", enable).await,
        true,
    )
    .await;
    assert_eq!(card["error"], "acknowledgement_required", "{card}");
    assert!(stored().await.skills[0].disabled);
    f.state.db.drop().await.unwrap();
}

async fn set_access_mode(f: &Fixture, mode: crate::models::assistant_settings::AccessMode) {
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(mode),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

/// Owner turns (no webhook policy) are gated by the access mode from the
/// operation's HTTP/catalog effects, at the shared service gate both the
/// typed and universal call paths use.
#[tokio::test]
async fn owner_access_mode_gates_service_operations_by_effect() {
    use crate::models::assistant_settings::AccessMode;
    let f = orchestrator_fixture("owner_mode_service_gate").await;
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .mount(&server)
        .await;
    connected(&f.state.db, &f.owner, "mode-target", &server.uri()).await;
    let auth = authenticate(&f).await;
    assert!(auth.chat.as_ref().unwrap().confirmation_policy.is_none());
    let tool = "mode-target__request";
    let gate = |method: &'static str, meta: bool| {
        let args = json!({"method": method, "path": "/items/1"});
        let (f, auth) = (&f, &auth);
        async move {
            let response = if meta {
                call(f, auth, tool, args).await
            } else {
                direct_call(f, auth, tool, args).await
            };
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            let text = body["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_default();
            text.contains("acknowledgement_required")
        }
    };
    let matrix = [
        (AccessMode::Autopilot, [false, false, false, false]),
        (AccessMode::Nyxmode, [false, false, false, true]),
        (AccessMode::AskEveryTime, [false, true, true, true]),
    ];
    for (mode, expected) in matrix {
        set_access_mode(&f, mode).await;
        for (method, asks) in ["GET", "POST", "PATCH", "DELETE"].into_iter().zip(expected) {
            for meta in [false, true] {
                assert_eq!(
                    gate(method, meta).await,
                    asks,
                    "{mode:?} {method} meta={meta}"
                );
            }
        }
    }
    let rows = acks::history(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap();
    let delete = rows
        .iter()
        .find(|row| row.summary.starts_with("DELETE"))
        .expect("delete card");
    let confirmation = delete.confirmation.as_ref().unwrap();
    assert_eq!(confirmation.risk, "destructive");
    assert_eq!(confirmation.reason, "access_mode");
    assert!(
        delete.summary.ends_with("on mode-target"),
        "{}",
        delete.summary
    );
    // Owner chats are offered the card id on every tool; it never reaches the service.
    let schema = webhook_tool_schema(&auth, &json!({"type": "object", "properties": {}}));
    assert_eq!(schema["properties"]["acknowledgement_id"]["type"], "string");
    let args = json!({"method": "POST", "acknowledgement_id": "x"});
    assert!(
        webhook_execution_arguments(&auth, &args)
            .get("acknowledgement_id")
            .is_none()
    );
    f.state.db.drop().await.unwrap();
}

/// Meta tools are classified by name at dispatch: SSH commands are
/// destructive (Auto asks), while Full access proceeds to the tool itself.
#[tokio::test]
async fn owner_access_mode_gates_meta_tools_at_dispatch() {
    use crate::models::assistant_settings::AccessMode;
    let f = orchestrator_fixture("owner_mode_meta_gate").await;
    let auth = authenticate(&f).await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.service_type = "ssh".into();
    catalog.ssh_config = Some(
        crate::services::ssh_service::build_ssh_config(
            &f.state.encryption_keys,
            &catalog.id,
            None,
            crate::services::ssh_service::SshConfigInput {
                host: "127.0.0.1",
                port: 22,
                certificate_auth_enabled: true,
                ssh_auth_mode: Some(crate::models::ssh_auth_mode::SshAuthMode::Cert),
                certificate_ttl_minutes: 30,
                allowed_principals: &["ubuntu".into()],
            },
        )
        .await
        .unwrap(),
    );
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&catalog)
        .await
        .unwrap();
    let connection = connected(&f.state.db, &f.owner, "mode-ssh", "ssh://127.0.0.1").await;
    f.state.db.collection::<bson::Document>("user_services").update_one(doc! {"_id":&connection},
        doc! {"$set":{"service_type":"ssh","catalog_service_id":&catalog.id,"ssh_auth_mode":"cert"}}).await.unwrap();
    let args = json!({"service": connection, "command": "uptime"});
    let text = |response: Response| async move {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        body["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    set_access_mode(&f, AccessMode::Nyxmode).await;
    let carded = text(direct_call(&f, &auth, "nyx__ssh_exec", args.clone()).await).await;
    let card: Value = serde_json::from_str(&carded).unwrap();
    assert_eq!(card["error"], "acknowledgement_required");
    let row = acks::history(&f.state.db, &f.owner, &f.row.id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(row.tool_name.as_deref(), Some("nyxid.service_execution"));
    assert_eq!(row.confirmation.as_ref().unwrap().risk, "destructive");
    assert!(
        !row.summary.contains("uptime"),
        "commands never enter a card"
    );
    // Discovery never asks, even in Ask every time.
    set_access_mode(&f, AccessMode::AskEveryTime).await;
    let listed =
        text(direct_call(&f, &auth, "nyx__list_connected_services", json!({})).await).await;
    let card_error = |text: &str| {
        serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| value["error"].as_str().map(str::to_owned))
    };
    assert_ne!(
        card_error(&listed).as_deref(),
        Some("acknowledgement_required"),
        "{listed}"
    );
    set_access_mode(&f, AccessMode::Autopilot).await;
    let ran = text(direct_call(&f, &auth, "nyx__ssh_exec", args).await).await;
    assert_ne!(
        card_error(&ran).as_deref(),
        Some("acknowledgement_required"),
        "{ran}"
    );
    assert_eq!(
        acks::history(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .len(),
        1
    );
    f.state.db.drop().await.unwrap();
}

/// A specialist's own conversation key cannot reach assistant management
/// routes, even with the live-turn rollout flag off: grants, skills,
/// settings, channels, agent profile and destroy all refuse it.
#[tokio::test]
async fn specialist_conversation_key_cannot_call_assistant_management_routes() {
    use tower::ServiceExt;
    let f = fixture("specialist_management_routes").await;
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
    let agent = f.chat.agent_id.clone();
    let send = |method: &str, path: String, body: Value| {
        let router = router.clone();
        let request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("x-api-key", credential.raw_key.as_str())
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        async move {
            let response = router.oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    let base = "/api/v1/assistant/nyxagent";
    for (method, path, body) in [
        (
            "PUT",
            format!("{base}/agents/{agent}/grants"),
            json!({"services": [], "account_read": true}),
        ),
        (
            "PUT",
            format!("{base}/agents/{agent}/skills"),
            json!({"expected_revision": 0, "skills": []}),
        ),
        (
            "PATCH",
            format!("{base}/agents/{agent}"),
            json!({"access_mode_override": "autopilot"}),
        ),
        ("POST", format!("{base}/agents/{agent}/destroy"), json!({})),
        (
            "PUT",
            format!("{base}/settings"),
            json!({"access_mode": "autopilot", "specialist_access_requests": "routed_to_nyxbot"}),
        ),
        (
            "POST",
            format!("{base}/channels"),
            json!({"bot_id": uuid::Uuid::new_v4().to_string(), "agent_id": agent}),
        ),
        (
            "PATCH",
            format!("{base}/channels/{}", uuid::Uuid::new_v4()),
            json!({"agent_id": agent}),
        ),
    ] {
        let (status, body) = send(method, path.clone(), body).await;
        assert_eq!(
            status,
            axum::http::StatusCode::FORBIDDEN,
            "{method} {path}: {body}"
        );
        // The human-only router refuses every API key, conversation keys
        // included, before any handler; the extractor (independent of the
        // live-turn flag) and the handlers' human guards refuse them again.
        assert_eq!(
            body["message"], "Forbidden: API keys cannot access this endpoint",
            "{method} {path}"
        );
    }
    // Nothing changed.
    let settings = crate::services::assistant_settings_service::get(&f.state.db, &f.owner)
        .await
        .unwrap();
    assert_ne!(
        settings.access_mode,
        crate::models::assistant_settings::AccessMode::Autopilot
    );
    let row = crate::services::assistant_team_service::agent(&f.state.db, &f.owner, &agent)
        .await
        .unwrap();
    assert!(row.destroyed_at.is_none() && !row.grants.account_read);
    assert!(row.access_mode_override.is_none());
    // Positive control: the same key still reaches its execution route.
    let (status, body) = send(
        "POST",
        "/mcp".into(),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert!(
        body["result"]["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty())
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn full_access_service_approval_executes_with_receipt_while_auto_waits() {
    use crate::models::{
        assistant_settings::AccessMode,
        service_approval_config::{COLLECTION_NAME as CONFIGS, ServiceApprovalConfig},
    };
    for (mode, asks) in [(AccessMode::Autopilot, false), (AccessMode::Nyxmode, true)] {
        let f = orchestrator_fixture("mode_service_approval").await;
        set_access_mode(&f, mode).await;
        let hits = Arc::new(AtomicUsize::new(0));
        let hit = hits.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().fallback(any(move || {
                    hit.fetch_add(1, Ordering::SeqCst);
                    async { Json(json!({"ok":true})) }
                })),
            )
            .await
            .unwrap();
        });
        let service = connected(&f.state.db, &f.owner, "mode-approval", &address).await;
        let now = chrono::Utc::now();
        f.state
            .db
            .collection::<ServiceApprovalConfig>(CONFIGS)
            .insert_one(ServiceApprovalConfig {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: f.owner.clone(),
                service_id: service.clone(),
                service_name: "Mode approval".into(),
                approval_required: true,
                approval_mode: Default::default(),
                rules: vec![],
                default_effect: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        let auth = authenticate(&f).await;
        let args = json!({"method":"GET","path":"/work"});
        if asks {
            let future = call(&f, &auth, "mode-approval__request", args);
            tokio::pin!(future);
            tokio::time::timeout(std::time::Duration::from_secs(10),async {
                loop {
                    tokio::select! { value=&mut future => panic!("Approval was not awaited: {}",value.status()), _=tokio::time::sleep(std::time::Duration::from_millis(25))=>{} }
                    if f.state.db.collection::<mongodb::bson::Document>(crate::models::approval_request::COLLECTION_NAME).count_documents(doc! {"service_id":&service}).await.unwrap()==1 {break;}
                }
            }).await.unwrap();
            assert_eq!(hits.load(Ordering::SeqCst), 0);
            assert!(
                acks::history(&f.state.db, &f.owner, &f.row.id)
                    .await
                    .unwrap()
                    .is_empty()
            );
        } else {
            let value = result(call(&f, &auth, "mode-approval__request", args).await, false).await;
            assert_eq!(value["ok"], true);
            assert_eq!(hits.load(Ordering::SeqCst), 1);
            assert_eq!(
                f.state
                    .db
                    .collection::<mongodb::bson::Document>(
                        crate::models::approval_request::COLLECTION_NAME
                    )
                    .count_documents(doc! {"service_id":&service})
                    .await
                    .unwrap(),
                0
            );
            let rows = acks::history(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].status, "allowed");
            assert_eq!(rows[0].confirmation.as_ref().unwrap().reason, "approval");
            assert_eq!(rows[0].summary, "mcp:GET /work on GitHub");
            let value = result(
                call(
                    &f,
                    &auth,
                    "mode-approval__request",
                    json!({"method":"DELETE","path":"/remove"}),
                )
                .await,
                false,
            )
            .await;
            assert_eq!(value["ok"], true);
            assert_eq!(hits.load(Ordering::SeqCst), 2);
            let rows = acks::history(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap();
            assert_eq!(rows.len(), 2, "one receipt for each successful operation");
            let destructive = rows.iter().find(|r| r.summary.contains("/remove")).unwrap();
            assert_eq!(
                destructive.confirmation.as_ref().unwrap().risk,
                "destructive"
            );
            assert_eq!(
                destructive.confirmation.as_ref().unwrap().reason,
                "approval"
            );
            assert!(rows.iter().all(|r| r.status == "allowed"));
        }
        server.abort();
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn missing_access_mcp_service_and_connect_results_have_absolute_instruction_links() {
    let f = crate::services::assistant_authority_tests::direct_fixture("mcp_missing_access_links")
        .await;
    let service_id = connected(&f.state.db, &f.owner, "github", "https://api.github.com").await;
    let catalog = public_connect_catalog(&f, "personal-github").await;
    let auth = authenticate(&f).await;
    let services = load_all_services_for_meta_tools(&f.state, &auth)
        .await
        .unwrap();
    let service = services
        .iter()
        .find(|service| service.service_id == service_id)
        .unwrap();
    let service = result(
        chat_service_gate(&f.state, &auth, service, None)
            .await
            .unwrap(),
        true,
    )
    .await;
    let connect = result(
        handle_meta_connect(
            &f.state,
            &auth,
            None,
            &json!({"service_id": catalog.id}),
            None,
            false,
        )
        .await,
        true,
    )
    .await;
    assert_eq!(service["error"], "missing_access");
    assert!(service["instructions"].as_str().unwrap().contains(&format!(
        "({}/assistant?",
        f.state.config.frontend_url.trim_end_matches('/')
    )));
    assert_eq!(connect["error"], "missing_access");
    assert_eq!(connect["catalog_service_id"], catalog.id);
    assert_eq!(connect["service_slug"], catalog.slug);
    assert!(
        connect["instructions"]
            .as_str()
            .unwrap()
            .contains("NyxBot send one")
    );
    assert!(!connect.to_string().contains("/keys"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn invocation_cancel_during_approval_wait_denies_direct_meta_and_http_dispatch() {
    use crate::models::{approval_request as approvals, service_approval_config as configs};
    use crate::services::{assistant_invocation_service as receipts, assistant_nyxagent as engine};
    use mongodb::bson::Document;
    use tower::ServiceExt;
    for (dispatch, credential_header) in [
        ("mcp_direct", "x-api-key"),
        ("mcp_meta", "x-api-key"),
        ("proxy_uuid", "x-api-key"),
        ("proxy_slug", "x-api-key"),
        ("llm_provider", "x-api-key"),
        ("llm_provider", "authorization"),
        ("llm_gateway", "x-api-key"),
        ("llm_gateway", "authorization"),
    ] {
        let mut f = orchestrator_fixture(&format!(
            "invocation_approval_wait_{dispatch}_{credential_header}"
        ))
        .await;
        receipts::ensure_indexes(&f.state.db).await.unwrap();
        engine::force_stop(
            &f.state.db,
            &f.owner,
            &f.row.id,
            &f.row.active_turn.as_ref().unwrap().turn_id,
        )
        .await
        .unwrap();
        let request = engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
            agent_id: None,
            conversation_id: Some(f.row.id.clone()),
            text: "approval fixture".into(),
            model: None,
            access_mode: None,
        };
        let start = engine::TurnStart::from(&request);
        let key = uuid::Uuid::new_v4().to_string();
        let admission = receipts::admit(
            &f.state.db,
            &f.owner,
            &key,
            "fixture",
            &start,
            &f.state.encryption_keys,
        )
        .await
        .unwrap();
        f.row = admission.claim.unwrap().conversation;
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
        let service = connected(&f.state.db, &f.owner, "approval-fixture", &address).await;
        let mut approval_services = vec![service.clone()];
        let mut provider_config_id = None;
        if dispatch.starts_with("llm_") {
            let provider_id = uuid::Uuid::new_v4().to_string();
            let now = mongodb::bson::DateTime::now();
            let provider: crate::models::provider_config::ProviderConfig =
                mongodb::bson::from_document(doc! {
                    "_id": &provider_id,
                    "slug": "openai",
                    "name": "Approval fixture provider",
                    "provider_type": "api_key",
                    "is_active": true,
                    "created_by": &f.owner,
                    "created_at": now,
                    "updated_at": now,
                })
                .unwrap();
            f.state
                .db
                .collection::<crate::models::provider_config::ProviderConfig>(
                    crate::models::provider_config::COLLECTION_NAME,
                )
                .insert_one(provider)
                .await
                .unwrap();
            provider_config_id = Some(provider_id);
        }
        let route_catalog_id = if dispatch.starts_with("proxy_") || dispatch.starts_with("llm_") {
            let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
            catalog.id = uuid::Uuid::new_v4().to_string();
            catalog.slug = if dispatch.starts_with("llm_") {
                "llm-openai"
            } else {
                "approval-fixture"
            }
            .into();
            catalog.base_url = address.clone();
            catalog.provider_config_id = provider_config_id;
            f.state
                .db
                .collection::<crate::models::downstream_service::DownstreamService>(
                    crate::models::downstream_service::COLLECTION_NAME,
                )
                .insert_one(&catalog)
                .await
                .unwrap();
            f.state
                .db
                .collection::<Document>(crate::models::user_service::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &service},
                    doc! {"$set": {"catalog_service_id": &catalog.id}},
                )
                .await
                .unwrap();
            approval_services.push(catalog.id.clone());
            Some(catalog.id)
        } else {
            None
        };
        set_access_mode(
            &f,
            crate::models::assistant_settings::AccessMode::AskEveryTime,
        )
        .await;
        let now = chrono::Utc::now();
        for service_id in &approval_services {
            f.state
                .db
                .collection(configs::COLLECTION_NAME)
                .insert_one(configs::ServiceApprovalConfig {
                    id: uuid::Uuid::new_v4().to_string(),
                    user_id: f.owner.clone(),
                    service_id: service_id.clone(),
                    service_name: "fixture".into(),
                    approval_required: true,
                    approval_mode: Default::default(),
                    rules: vec![],
                    default_effect: None,
                    created_at: now,
                    updated_at: now,
                })
                .await
                .unwrap();
        }
        let auth = authenticate(&f).await;
        assert_eq!(
            acks::effective_access_mode(&f.state.db, auth.chat.as_ref().unwrap())
                .await
                .unwrap(),
            crate::models::assistant_settings::AccessMode::AskEveryTime
        );
        let name = if dispatch.starts_with("mcp_") {
            let search = result(
                handle_meta_search(
                    &f.state,
                    &auth,
                    None,
                    &json!({"query":"approval-fixture"}),
                    None,
                    false,
                )
                .await,
                false,
            )
            .await;
            Some(
                search["matches"][0]["name"]
                    .as_str()
                    .expect("MCP fixture must expose its generic HTTP tool")
                    .to_owned(),
            )
        } else {
            None
        };
        let credential = credentials::load_for_conversation(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &f.row.id,
        )
        .await
        .unwrap()
        .unwrap();
        let credential_value = zeroize::Zeroizing::new(if credential_header == "authorization" {
            format!("Bearer {}", credential.raw_key.as_str())
        } else {
            credential.raw_key.to_string()
        });
        let (_, private) = crate::routes::build_router_with_state(f.state.clone());
        let mounted = private.with_state(f.state.clone());
        for denied_path in [
            format!("/api/v1/ssh/{service}"),
            "/api/v1/assistant/nyxagent/settings".into(),
        ] {
            let refusal = mounted
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(denied_path)
                        .header(credential_header, credential_value.as_str())
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(refusal.status(), axum::http::StatusCode::FORBIDDEN);
        }
        let action = async {
            let args = json!({"method":"GET","path":"/effect"});
            match dispatch {
                "mcp_direct" => {
                    direct_call(&f, &auth, name.as_deref().expect("MCP fixture tool"), args).await
                }
                "mcp_meta" => {
                    call(&f, &auth, name.as_deref().expect("MCP fixture tool"), args).await
                }
                _ => {
                    let path = match dispatch {
                        "proxy_uuid" => format!(
                            "/api/v1/proxy/{}/effect",
                            route_catalog_id.as_deref().expect("proxy catalog fixture")
                        ),
                        "proxy_slug" => "/api/v1/proxy/s/approval-fixture/effect".into(),
                        "llm_provider" => "/api/v1/llm/openai/v1/chat/completions".into(),
                        "llm_gateway" => "/api/v1/llm/gateway/v1/chat/completions".into(),
                        _ => unreachable!(),
                    };
                    let llm = dispatch.starts_with("llm_");
                    let body = if llm {
                        axum::body::Body::from(
                            json!({"model":"gpt-fixture","messages":[{"role":"user","content":"fixture"}]})
                                .to_string(),
                        )
                    } else {
                        axum::body::Body::empty()
                    };
                    mounted
                        .oneshot(
                            axum::http::Request::builder()
                                .method(if llm { "POST" } else { "GET" })
                                .uri(path)
                                .header(credential_header, credential_value.as_str())
                                .header("content-type", "application/json")
                                .body(body)
                                .unwrap(),
                        )
                        .await
                        .unwrap()
                }
            }
        };
        tokio::pin!(action);
        let pending_approval = async {
            loop {
                if let Some(card) = f
                    .state
                    .db
                    .collection::<Document>(approvals::COLLECTION_NAME)
                    .find_one(doc! {"service_id": {"$in": &approval_services}, "status": "pending"})
                    .await
                    .unwrap()
                {
                    break card;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        };
        let pending = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::select! {
                response = &mut action => {
                    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                        .await
                        .unwrap();
                    let body: Value = serde_json::from_slice(&bytes).unwrap();
                    // These fixtures send no secret-bearing arguments. Report
                    // only the tool's safe error text, never auth/fixture state.
                    panic!(
                        "{dispatch} ({credential_header}) returned before service approval: {}",
                        body["result"]["content"][0]["text"]
                            .as_str()
                            .or_else(|| body["message"].as_str())
                            .unwrap_or("missing error result")
                    );
                }
                pending = pending_approval => pending,
            }
        })
        .await
        .expect("tool must reach service approval before cancellation");
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        receipts::cancel(&f.state, &f.owner, &key).await.unwrap();
        f.state
            .db
            .collection::<Document>(approvals::COLLECTION_NAME)
            .update_one(
                doc! {"_id":pending.get_str("_id").unwrap()},
                doc! {"$set":{"status":"approved"}},
            )
            .await
            .unwrap();
        let response = tokio::time::timeout(std::time::Duration::from_secs(5), action)
            .await
            .expect("approved cancelled request must resume and refuse dispatch");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        if dispatch.starts_with("mcp_") {
            assert_eq!(body["result"]["isError"], true);
            assert!(
                body["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("originating invocation")
            );
        } else {
            assert_eq!(
                status,
                axum::http::StatusCode::CONFLICT,
                "{dispatch} ({credential_header})"
            );
            assert_eq!(
                body["error_code"], 12102,
                "{dispatch} ({credential_header})"
            );
        }
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        server.abort();
        f.state.db.drop().await.unwrap();
    }
}

async fn configure_guest_with_owner_card(
    f: &Fixture,
    auth: &McpAuthContext,
    mut args: Value,
) -> Value {
    let card = result(
        call(f, auth, "nyxid__set_guest_access", args.clone()).await,
        true,
    )
    .await;
    assert_eq!(card["error"], "acknowledgement_required");
    assert_eq!(card["decider"], "user");
    let ack = card["acknowledgement_id"].as_str().unwrap();
    acks::decide(&f.state.db, &f.owner, &f.nyxbot_thread, ack, true)
        .await
        .unwrap();
    args["acknowledgement_id"] = json!(ack);
    result(call(f, auth, "nyxid__set_guest_access", args).await, false).await
}

#[tokio::test]
async fn guest_access_none_then_clear_selection_preserves_read_level() {
    use crate::models::assistant_agent::AgentGrants;
    use crate::services::{
        agent_operation_scope_service as scopes, assistant_team_service as team,
        feature_flag_service as flags,
    };
    let f = boxed_chat_fixture("guest_none_preserves_read").await;
    let id = connected(&f.state.db, &f.owner, "read-only", "http://127.0.0.1:9").await;
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
    flags::set_platform_override(
        &f.state.db,
        flags::GUEST_OPERATION_SCOPES_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let nyxbot = authenticate_id(&f, &f.nyxbot_thread).await;
    configure_guest_with_owner_card(
        &f,
        &nyxbot,
        json!({"subagent":"worker", "services":[id], "access":"read"}),
    )
    .await;
    let mut guest = f.auth.clone();
    let mut chat = f.chat.clone();
    chat.guest = true;
    guest.assistant_chat = Some(Arc::new(chat));
    let path =
        crate::services::proxy_authorization::CanonicalPath::from_mcp_literal("/items").unwrap();
    let allowed = |method: &'static str| {
        scopes::check_non_mcp_context(
            &f.state.db,
            &guest,
            false,
            &id,
            None,
            method,
            Some(&path),
            Default::default(),
        )
    };
    assert!(allowed("GET").await.is_ok());
    assert!(allowed("POST").await.is_err());

    configure_guest_with_owner_card(
        &f,
        &nyxbot,
        json!({"subagent":"worker", "services":[id], "access":"none",
            "selection":{"expected_revision":0}}),
    )
    .await;
    let withheld = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert_eq!(withheld.guest_access[&id], GuestAccess::Read);
    assert!(withheld.guest_operation_scopes[&id].operations.is_empty());
    assert!(allowed("GET").await.is_err());

    // Clear only the selection through the human route. Supplying `read` again
    // would hide a regression where `none` had silently widened the stored level.
    let Json(cleared) = crate::handlers::assistant_team::set_guest_operations(
        axum::extract::State(f.state.clone()),
        crate::test_utils::test_auth_user(&f.owner),
        axum::extract::Path((f.chat.agent_id.clone(), id.clone())),
        Json(
            serde_json::from_value(json!({
                "expected_revision":1, "all_operations":true
            }))
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(cleared["revision"], 2);
    let restored = team::agent(&f.state.db, &f.owner, &f.chat.agent_id)
        .await
        .unwrap();
    assert_eq!(restored.guest_access[&id], GuestAccess::Read);
    assert!(!restored.guest_operation_scopes.contains_key(&id));
    assert_eq!(restored.guest_operation_scope_revisions[&id], 2);
    assert!(allowed("GET").await.is_ok());
    assert!(allowed("POST").await.is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_operation_selection_filters_discovery_and_refuses_direct_and_universal_calls() {
    use crate::models::assistant_agent::AgentGrants;
    use crate::services::{assistant_team_service as team, feature_flag_service as flags};
    let f = boxed_chat_fixture("guest_scope_mcp").await;
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let server_app = Router::new().route(
        "/{*path}",
        any(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { Json(json!({"ok":true})) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, server_app).await.unwrap() });
    let id = connected(&f.state.db, &f.owner, "limited", &url).await;
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
    let generic_auth = authenticate(&f).await;
    let generic = load_all_services_for_meta_tools(&f.state, &generic_auth)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.service_id == id)
        .unwrap();
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "guest-limited-catalog".into();
    catalog.base_url = url.clone();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"catalog_service_id":&catalog.id}},
        )
        .await
        .unwrap();
    let read = crate::test_utils::test_service_endpoint(&catalog.id, "read_one", "GET", "/files");
    let send = crate::test_utils::test_service_endpoint(&catalog.id, "send", "POST", "/mail");
    f.state
        .db
        .collection::<crate::models::service_endpoint::ServiceEndpoint>(
            crate::models::service_endpoint::COLLECTION_NAME,
        )
        .insert_many([&read, &send])
        .await
        .unwrap();
    mark_guest(&f, true).await;
    let guest = authenticate(&f).await;
    let services = load_all_services_for_meta_tools(&f.state, &guest)
        .await
        .unwrap();
    let svc = services.iter().find(|s| s.service_id == id).unwrap();
    assert_eq!(svc.endpoints.len(), 2);
    let send_name = format!("{}__{}", svc.service_slug, send.name);
    let read_name = format!("{}__{}", svc.service_slug, read.name);
    flags::set_platform_override(
        &f.state.db,
        flags::GUEST_OPERATION_SCOPES_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let nyxbot = authenticate_id(&f, &f.nyxbot_thread).await;
    configure_guest_with_owner_card(&f,&nyxbot,json!({"subagent":"worker","services":[id],"access":"use","selection":{"expected_revision":0,"endpoint_ids":[read.id]}})).await;
    let services = load_all_services_for_meta_tools(&f.state, &guest)
        .await
        .unwrap();
    let visible = services.iter().find(|s| s.service_id == id).unwrap();
    assert_eq!(visible.endpoints.len(), 1);
    assert_eq!(visible.endpoints[0].endpoint_id, read.id);
    // Known tool names remain refused even though they are now hidden.
    result(direct_call(&f, &guest, &send_name, json!({})).await, true).await;
    result(call(&f, &guest, &send_name, json!({})).await, true).await;
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    result(call(&f, &guest, &read_name, json!({})).await, false).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    // The guest selection cannot widen the specialist's ordinary B1 scope.
    flags::set_platform_override(
        &f.state.db,
        flags::AGENT_OPERATION_SCOPES_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let deny: crate::models::agent_operation_scope::OperationSelection =
        serde_json::from_value(json!({"expected_revision":0})).unwrap();
    crate::services::agent_operation_scope_service::set(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &id,
        &deny,
        true,
    )
    .await
    .unwrap();
    let scoped_guest = authenticate(&f).await;
    let refused = call(&f, &scoped_guest, &read_name, json!({})).await;
    let bytes = axum::body::to_bytes(refused.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let refused: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(refused["result"]["isError"], true);
    assert!(
        refused["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Specialist operation scope denied this call")
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let restore: crate::models::agent_operation_scope::OperationSelection =
        serde_json::from_value(json!({"expected_revision":1,"all_operations":true})).unwrap();
    crate::services::agent_operation_scope_service::set(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        &id,
        &restore,
        true,
    )
    .await
    .unwrap();
    // A generic path cannot spend the typed endpoint's scope on another path.
    assert!(generic.is_generic_proxy);
    let endpoint = &generic.endpoints[0];
    let prepared = mcp_service::prepare_proxy_tool_call(
        &generic,
        endpoint,
        &json!({"method":"POST","path":"/mail"}),
    )
    .unwrap();
    assert!(
        guest_service_refusal(&f.state, &guest, &generic, endpoint, &prepared, None)
            .await
            .is_some()
    );
    // NyxBot configures none on the existing key with the same durable revision.
    let configured=configure_guest_with_owner_card(&f,&nyxbot,json!({"subagent":"worker","services":[id],"access":"none","selection":{"expected_revision":1}})).await;
    assert_eq!(configured["guest_operations"][0]["endpoint_ids"], json!([]));
    let services = load_all_services_for_meta_tools(&f.state, &guest)
        .await
        .unwrap();
    assert!(!services.iter().any(|s| s.service_id == id));
    result(call(&f, &guest, &read_name, json!({})).await, true).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    // Guests and specialists cannot use the configuration tool to escape.
    result(
        call(
            &f,
            &guest,
            "nyxid__set_guest_access",
            json!({"subagent":"worker","access":"all","services":[id]}),
        )
        .await,
        true,
    )
    .await;
    mark_guest(&f, false).await;
    let owner = authenticate(&f).await;
    result(
        call(
            &f,
            &owner,
            "nyxid__set_guest_access",
            json!({"subagent":"worker","access":"all","services":[id]}),
        )
        .await,
        true,
    )
    .await;
    assert!(
        guest_service_refusal(&f.state, &owner, &generic, endpoint, &prepared, None)
            .await
            .is_none()
    );
    server.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_operations_do_not_follow_redirects_or_mint_delegation_tokens() {
    use crate::models::{agent_operation_scope::OperationSelection, assistant_agent::AgentGrants};
    use crate::services::{assistant_team_service as team, feature_flag_service as flags};
    let f = boxed_chat_fixture("guest_scope_redirect").await;
    let hits = Arc::new(AtomicUsize::new(0));
    let redirects = hits.clone();
    let escaped = Arc::new(AtomicUsize::new(0));
    let escape_count = escaped.clone();
    let app = Router::new()
        .route(
            "/allowed",
            any(move || {
                redirects.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        axum::http::StatusCode::TEMPORARY_REDIRECT,
                        [("location", "/other")],
                    )
                }
            }),
        )
        .route(
            "/other",
            any(move || {
                escape_count.fetch_add(1, Ordering::SeqCst);
                async { Json(json!({"ok": true})) }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let id = connected(&f.state.db, &f.owner, "redirect", &url).await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "guest-redirect-catalog".into();
    catalog.base_url = url;
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &id},
            doc! {"$set": {"catalog_service_id": &catalog.id}},
        )
        .await
        .unwrap();
    let endpoint =
        crate::test_utils::test_service_endpoint(&catalog.id, "allowed", "GET", "/allowed");
    f.state
        .db
        .collection::<crate::models::service_endpoint::ServiceEndpoint>(
            crate::models::service_endpoint::COLLECTION_NAME,
        )
        .insert_one(&endpoint)
        .await
        .unwrap();
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
    flags::set_platform_override(
        &f.state.db,
        flags::GUEST_OPERATION_SCOPES_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let selection: OperationSelection =
        serde_json::from_value(json!({"expected_revision":0,"endpoint_ids":[endpoint.id]}))
            .unwrap();
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::GuestOperations {
            base: Box::new(team::GrantChange::Guests(Default::default())),
            selections: [(id.clone(), selection)].into(),
        },
    )
    .await
    .unwrap();
    mark_guest(&f, true).await;
    let guest = authenticate(&f).await;
    let response = call(&f, &guest, "redirect__allowed", json!({})).await;
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["result"]["isError"], true);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "the selected route was reached"
    );
    assert_eq!(
        escaped.load(Ordering::SeqCst),
        0,
        "a redirect cannot escape guest limits"
    );
    // Delegation tokens carry key scopes, but no guest policy. Refuse issuance.
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &id},
            doc! {"$set": {"inject_delegation_token":true}},
        )
        .await
        .unwrap();
    let response = call(&f, &guest, "redirect__allowed", json!({})).await;
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["result"]["isError"], true);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    server.abort();
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_append_mcp_checks_actual_query_and_intersects_guest_level() {
    use crate::services::assistant_team_service as team;
    let f = boxed_chat_fixture("guest_append_mcp").await;
    let id = connected(&f.state.db, &f.owner, "sheets", "http://127.0.0.1:9").await;
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(crate::models::assistant_agent::AgentGrants {
            service_ids: vec![id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    let owner = authenticate(&f).await;
    let mut generic = load_all_services_for_meta_tools(&f.state, &owner)
        .await
        .unwrap()
        .into_iter()
        .find(|service| service.service_id == id)
        .unwrap();
    assert!(generic.is_generic_proxy);
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "api-google-workspace".into();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &id},
            doc! {"$set": {"catalog_service_id": &catalog.id}},
        )
        .await
        .unwrap();
    if let mcp_service::McpToolSource::UserManaged {
        catalog_service_id, ..
    } = &mut generic.source
    {
        *catalog_service_id = Some(catalog.id.clone());
    } else {
        panic!("user-managed fixture");
    }
    let path = "/v4/spreadsheets/{spreadsheetId}/values/{range}:append";
    let spec = crate::services::catalog_spec_registry::spec_for_slug(&catalog.slug).unwrap();
    let mut row =
        crate::test_utils::test_service_endpoint(&catalog.id, "sheets_append_values", "POST", path);
    row.parameters = Some(spec["paths"][path]["post"]["parameters"].clone());
    row.request_body_schema = Some(
        spec["paths"][path]["post"]["requestBody"]["content"]["application/json"]["schema"].clone(),
    );
    f.state
        .db
        .collection::<crate::models::service_endpoint::ServiceEndpoint>(
            crate::models::service_endpoint::COLLECTION_NAME,
        )
        .insert_one(&row)
        .await
        .unwrap();
    mark_guest(&f, true).await;
    let guest = authenticate(&f).await;
    let services = load_all_services_for_meta_tools(&f.state, &guest)
        .await
        .unwrap();
    let service = services.iter().find(|s| s.service_id == id).unwrap();
    let endpoint = &service.endpoints[0];
    let metadata = service.durable_endpoint_metadata[&endpoint.endpoint_id];
    assert!(metadata.append_only && metadata.catalog_contract && metadata.destructive);
    for (insert, allowed) in [
        (None, false),
        (Some("OVERWRITE"), false),
        (Some("INSERT_ROWS"), true),
    ] {
        let mut args = json!({"spreadsheetId":"sheet","range":"A1:B2","valueInputOption":"RAW","body":{"values":[["new","row"]]}});
        if let Some(insert) = insert {
            args["insertDataOption"] = json!(insert);
        }
        let prepared = mcp_service::prepare_proxy_tool_call(service, endpoint, &args).unwrap();
        assert_eq!(
            guest_service_refusal(&f.state, &guest, service, endpoint, &prepared, None)
                .await
                .is_none(),
            allowed
        );
        assert!(
            prepared.guest_effects(metadata).destructive,
            "owner approval classification stays destructive"
        );
    }
    // A generic fallback must still classify the actual compiled route even
    // after durable endpoint rows are unavailable.
    f.state
        .db
        .collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .delete_one(doc! {"_id": &row.id})
        .await
        .unwrap();
    for (query, allowed) in [
        ("", false),
        ("insertDataOption=OVERWRITE", false),
        ("insertDataOption=INSERT_ROWS", true),
    ] {
        let endpoint = &generic.endpoints[0];
        let prepared = mcp_service::prepare_proxy_tool_call(
            &generic,
            endpoint,
            &json!({
                "method":"POST","path":"/v4/spreadsheets/sheet/values/A1:append","query":query,
                "body":{"values":[["new"]]}
            }),
        )
        .unwrap();
        assert_eq!(
            guest_service_refusal(&f.state, &guest, &generic, endpoint, &prepared, None)
                .await
                .is_none(),
            allowed
        );
    }
    set_guest_access(&f, &id, GuestAccess::Read).await;
    let args = json!({"spreadsheetId":"sheet","range":"A1","valueInputOption":"RAW","insertDataOption":"INSERT_ROWS","body":{"values":[["new"]]}});
    let prepared = mcp_service::prepare_proxy_tool_call(service, endpoint, &args).unwrap();
    assert!(
        guest_service_refusal(&f.state, &guest, service, endpoint, &prepared, None)
            .await
            .is_some()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn guest_scopes_raw_proxy_and_pool_member_keep_owner_access() {
    use crate::models::{agent_operation_scope::OperationSelection, assistant_agent::AgentGrants};
    use crate::services::{assistant_team_service as team, feature_flag_service as flags};
    use tower::ServiceExt;
    let f = boxed_chat_fixture("guest_scope_raw_pool").await;
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let app = Router::new().route(
        "/{*path}",
        any(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { Json(json!({"ok":true})) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let id = connected(&f.state.db, &f.owner, "guest-member", &url).await;
    let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
    catalog.slug = "guest-member-catalog".into();
    catalog.base_url = url;
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&catalog)
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &id},
            doc! {"$set":{"catalog_service_id": &catalog.id}},
        )
        .await
        .unwrap();
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
    crate::services::service_pool_service::create_pool(
        &f.state.db,
        &f.owner,
        crate::services::service_pool_service::CreatePoolInput {
            slug: "guest-pool".into(),
            name: "Guest pool".into(),
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
    flags::set_platform_override(
        &f.state.db,
        flags::GUEST_OPERATION_SCOPES_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let none: OperationSelection = serde_json::from_value(json!({"expected_revision":0})).unwrap();
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::GuestOperations {
            base: Box::new(team::GrantChange::Guests(Default::default())),
            selections: [(id.clone(), none)].into(),
        },
    )
    .await
    .unwrap();
    let key = credentials::load_for_conversation(
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
    for guest in [true, false] {
        mark_guest(&f, guest).await;
        for route in [
            format!("/api/v1/proxy/{}/read", catalog.id),
            "/api/v1/proxy/s/guest-member/read".into(),
            "/api/v1/proxy/s/guest-pool/read".into(),
        ] {
            let response = router
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(&route)
                        .header("x-api-key", key.raw_key.as_str())
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            if guest {
                assert!(
                    response.status().is_client_error(),
                    "{route}: {}",
                    response.status()
                );
            } else {
                assert!(
                    response.status().is_success(),
                    "{route}: {}",
                    response.status()
                );
            }
            axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
        }
        assert_eq!(hits.load(Ordering::SeqCst), if guest { 0 } else { 3 });
    }
    server.abort();
    f.state.db.drop().await.unwrap();
}

async fn public_connect_catalog(
    f: &Fixture,
    slug: &str,
) -> crate::models::downstream_service::DownstreamService {
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = uuid::Uuid::new_v4().to_string();
    catalog.slug = slug.into();
    catalog.visibility = "public".into();
    catalog.requires_user_credential = true;
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&catalog)
        .await
        .unwrap();
    catalog
}

async fn seeded_github(f: &Fixture) -> crate::models::downstream_service::DownstreamService {
    crate::services::provider_service::seed_default_providers(
        &f.state.db,
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    crate::services::provider_service::seed_default_services(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .find_one(doc! {"slug":"api-github"})
        .await
        .unwrap()
        .expect("seeded GitHub catalog")
}

async fn catalog_connection(
    f: &Fixture,
    owner: &str,
    catalog: &str,
    slug: &str,
    active: bool,
) -> String {
    let id = connected(&f.state.db, owner, slug, "https://example.test").await;
    let row = f
        .state
        .db
        .collection::<crate::models::user_service::UserService>("user_services")
        .find_one(doc! {"_id": &id})
        .await
        .unwrap()
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id": &id},
            doc! {"$set":{"catalog_service_id":catalog,"is_active":active}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_endpoints")
        .update_one(doc! {"_id": &row.endpoint_id}, doc! {"$set":{"label":slug}})
        .await
        .unwrap();
    id
}

async fn legacy_catalog_connection(
    f: &Fixture,
    owner: &str,
    catalog: &str,
    label: &str,
    active: bool,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service_connection::COLLECTION_NAME)
        .insert_one(doc! {
            "_id":&id,"user_id":owner,"service_id":catalog,
            "credential_label":label,"is_active":active,"created_at":now,"updated_at":now,
            // Inventory must succeed without even deserializing these fields.
            "credential_encrypted":{"excluded_secret":"legacy-secret-never-returned"},
            "metadata":{"excluded_metadata":"legacy-metadata-never-returned"},
        })
        .await
        .unwrap();
    id
}

async fn connect_test_org(f: &Fixture, active: bool) -> String {
    use crate::models::org_membership::{MemberScopeSource, OrgRole};
    let id = uuid::Uuid::new_v4().to_string();
    let mut org = crate::test_utils::test_user(&id, crate::models::user::UserType::Org);
    org.display_name = Some("Example Organization".into());
    f.state
        .db
        .collection("users")
        .insert_one(org)
        .await
        .unwrap();
    crate::services::org_service::create_membership(
        &f.state.db,
        &id,
        &f.owner,
        OrgRole::Member,
        MemberScopeSource::Inherit,
        None,
    )
    .await
    .unwrap();
    if !active {
        f.state
            .db
            .collection::<bson::Document>("users")
            .update_one(doc! {"_id":&id}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
    }
    id
}

#[tokio::test]
async fn chat_hosted_connect_returns_both_personal_and_org_facts_and_skips_inactive_owners() {
    let f = orchestrator_fixture("connect_existing_facts").await;
    let catalog = seeded_github(&f).await;
    let org = connect_test_org(&f, true).await;
    let inactive = connect_test_org(&f, false).await;
    let personal = catalog_connection(&f, &f.owner, &catalog.id, "personal-github", false).await;
    let shared = catalog_connection(&f, &org, &catalog.id, "work-github", true).await;
    catalog_connection(&f, &inactive, &catalog.id, "inactive-owner", true).await;
    let other = public_connect_catalog(&f, "unrelated-service").await;
    catalog_connection(&f, &f.owner, &other.id, "unrelated", true).await;
    // User-owned label metadata takes precedence; secret fields never enter the response.
    let key = crate::services::user_api_key_service::create_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        crate::services::user_api_key_service::CreateApiKeyParams {
            label: "Personal PAT",
            credential_type: "api_key",
            credential: "must-never-appear",
            access_token: None,
            refresh_token: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            status: "active",
            source: None,
            source_id: None,
        },
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_api_keys")
        .update_one(
            doc! {"_id":&key.id},
            doc! {"$set":{"metadata":{"provider_account":"do-not-infer"}}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(doc! {"_id":&personal}, doc! {"$set":{"api_key_id":&key.id}})
        .await
        .unwrap();
    let auth = authenticate(&f).await;
    let value = result(
        handle_meta_connect(
            &f.state,
            &auth,
            None,
            &json!({"service_slug":catalog.slug}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    assert_eq!(value["status"], "pending_connection", "{value}");
    assert!(
        value["connect_url"]
            .as_str()
            .unwrap()
            .contains("/connect/nyx_clk_")
    );
    let existing = value["existing_connections"].as_array().unwrap();
    assert_eq!(existing.len(), 2, "{value}");
    let own = existing
        .iter()
        .find(|v| v["connection_id"] == personal)
        .unwrap();
    assert_eq!(own["relation"], "self");
    assert_eq!(own["label"], "Personal PAT");
    assert_eq!(own["is_active"], false);
    let org = existing
        .iter()
        .find(|v| v["connection_id"] == shared)
        .unwrap();
    assert_eq!(org["relation"], "organization");
    assert_eq!(org["organization_name"], "Example Organization");
    assert_eq!(org["label"], "work-github");
    assert_eq!(org["is_active"], true);
    for fact in existing {
        assert_eq!(fact["account"]["status"], "unknown");
    }
    assert_eq!(value["existing_connections_truncated"], false);
    assert!(!value.to_string().contains("must-never-appear"));
    assert!(!value.to_string().contains("do-not-infer"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn chat_connect_inventory_excludes_viewers_revoked_scopes_and_both_tombstone_shapes() {
    use crate::models::org_membership::OrgRole;
    let f = orchestrator_fixture("connect_inventory_visibility").await;
    let catalog = public_connect_catalog(&f, "visible-connect").await;
    let viewer = connect_test_org(&f, true).await;
    let revoked = connect_test_org(&f, true).await;
    let scoped = connect_test_org(&f, true).await;
    catalog_connection(&f, &viewer, &catalog.id, "viewer-hidden", true).await;
    catalog_connection(&f, &revoked, &catalog.id, "revoked-hidden", true).await;
    catalog_connection(&f, &scoped, &catalog.id, "scope-hidden", true).await;
    let in_scope = catalog_connection(&f, &scoped, &catalog.id, "in-scope", true).await;
    f.state
        .db
        .collection::<bson::Document>("org_memberships")
        .update_one(
            doc! {"org_user_id":&viewer,"member_user_id":&f.owner},
            doc! {"$set":{"role":"viewer"}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("org_memberships")
        .update_one(
            doc! {"org_user_id":&revoked,"member_user_id":&f.owner},
            doc! {"$set":{"revoked_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    crate::services::org_role_scope_service::set_scope(
        &f.state.db,
        &scoped,
        OrgRole::Member,
        Some(vec![in_scope.clone()]),
        &f.owner,
    )
    .await
    .unwrap();
    let disabled = catalog_connection(&f, &f.owner, &catalog.id, "disabled-retained", false).await;
    let deleted = catalog_connection(&f, &f.owner, &catalog.id, "deleted-marker", false).await;
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&deleted},
            doc! {"$set":{"deleted_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    let tombstone =
        catalog_connection(&f, &f.owner, &catalog.id, "deleted-without-marker", false).await;
    let row = f
        .state
        .db
        .collection::<crate::models::user_service::UserService>("user_services")
        .find_one(doc! {"_id":&tombstone})
        .await
        .unwrap()
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_endpoints")
        .delete_one(doc! {"_id":&row.endpoint_id})
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(doc! {"_id":&tombstone}, doc! {"$unset":{"deleted_at":""}})
        .await
        .unwrap();
    let auth = authenticate(&f).await;
    let value = result(
        handle_meta_connect(
            &f.state,
            &auth,
            None,
            &json!({"service_id":catalog.id}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    let facts = value["existing_connections"].as_array().unwrap();
    assert_eq!(facts.len(), 2, "{value}");
    assert!(facts.iter().any(|fact| fact["connection_id"] == in_scope));
    assert!(
        facts
            .iter()
            .any(|fact| fact["connection_id"] == disabled && fact["is_active"] == false)
    );
    assert_eq!(value["existing_connections_truncated"], false);
    for hidden in [
        "viewer-hidden",
        "revoked-hidden",
        "scope-hidden",
        "deleted-marker",
        "deleted-without-marker",
    ] {
        assert!(!value.to_string().contains(hidden), "{hidden}: {value}");
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn chat_connect_facts_are_bounded_and_respect_org_scopes_and_deleted_rows() {
    use crate::models::org_membership::{MemberScopeSource, OrgMembership};
    let f = orchestrator_fixture("connect_bounded_facts").await;
    let catalog = public_connect_catalog(&f, "bounded-connect").await;
    let org = connect_test_org(&f, true).await;
    catalog_connection(&f, &org, &catalog.id, "hidden-by-scope", true).await;
    f.state.db.collection::<OrgMembership>("org_memberships").update_many(doc! {"org_user_id":&org}, doc! {"$set":{"scope_source":bson::to_bson(&MemberScopeSource::Override).unwrap(),"allowed_service_ids":[]}}).await.unwrap();
    let deleted = catalog_connection(&f, &f.owner, &catalog.id, "deleted", false).await;
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&deleted},
            doc! {"$set":{"deleted_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    let empty = crate::services::assistant_connection_service::existing_connections(
        &f.state.db,
        &f.owner,
        &catalog,
        None,
    )
    .await
    .unwrap();
    assert!(empty.connections.is_empty());
    for n in 0..12 {
        catalog_connection(
            &f,
            &f.owner,
            &catalog.id,
            &format!("account-{n}"),
            n % 2 == 0,
        )
        .await;
    }
    let auth = authenticate(&f).await;
    let value = result(
        handle_meta_connect(
            &f.state,
            &auth,
            None,
            &json!({"service_id":catalog.id}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    assert_eq!(value["existing_connections"].as_array().unwrap().len(), 10);
    assert_eq!(value["existing_connections_truncated"], true);
    assert!(!value.to_string().contains("hidden-by-scope"));
    assert!(!value.to_string().contains("\"deleted\""));
    assert!(serde_json::to_string_pretty(&value).unwrap().len() < 10_000);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn chat_legacy_inventory_filters_owner_catalog_and_activity_and_shares_the_bound() {
    use crate::services::assistant_connection_service::existing_connections;
    let f = orchestrator_fixture("connect_legacy_inventory_bound").await;
    let catalog = public_connect_catalog(&f, "legacy-bound").await;
    let other_catalog = public_connect_catalog(&f, "legacy-other-catalog").await;
    let org = connect_test_org(&f, true).await;
    legacy_catalog_connection(&f, &org, &catalog.id, "organization-legacy", true).await;
    legacy_catalog_connection(
        &f,
        &f.owner,
        &other_catalog.id,
        "other-catalog-legacy",
        true,
    )
    .await;
    let legacy =
        legacy_catalog_connection(&f, &f.owner, &catalog.id, "personal-legacy", false).await;
    let empty = existing_connections(&f.state.db, &f.owner, &catalog, None)
        .await
        .unwrap();
    assert!(empty.connections.is_empty());
    assert!(!empty.truncated);

    let timestamp = bson::DateTime::from_millis(1_700_000_000_000);
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service_connection::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&legacy},
            doc! {"$set":{"is_active":true,"created_at":timestamp}},
        )
        .await
        .unwrap();
    let mut unified_ids = Vec::new();
    for n in 0..10 {
        unified_ids.push(
            catalog_connection(&f, &f.owner, &catalog.id, &format!("account-{n}"), true).await,
        );
        if n == 8 {
            let exact = existing_connections(&f.state.db, &f.owner, &catalog, None)
                .await
                .unwrap();
            assert_eq!(exact.connections.len(), 10);
            assert!(
                !exact.truncated,
                "nine unified plus one legacy fits exactly"
            );
            assert!(
                exact
                    .connections
                    .iter()
                    .any(|fact| fact["connection_id"] == legacy)
            );
        }
    }
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_many(
            doc! {"_id":{"$in":&unified_ids}},
            doc! {"$set":{"created_at":timestamp}},
        )
        .await
        .unwrap();
    // The legacy row participates in the same timestamp/UUID ordering, whether
    // it is newest, oldest, or tied with the unified rows at the ten-row boundary.
    for offset in [1_000, -1_000, 0] {
        f.state
            .db
            .collection::<bson::Document>(crate::models::user_service_connection::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&legacy},
                doc! {"$set":{"created_at":bson::DateTime::from_millis(timestamp.timestamp_millis()+offset)}},
            )
            .await
            .unwrap();
        let inventory = existing_connections(&f.state.db, &f.owner, &catalog, None)
            .await
            .unwrap();
        let mut expected = unified_ids.clone();
        expected.sort();
        if offset > 0 {
            expected.insert(0, legacy.clone());
        } else if offset < 0 {
            expected.push(legacy.clone());
        } else {
            expected.push(legacy.clone());
            expected.sort();
        }
        expected.truncate(10);
        assert_eq!(
            inventory
                .connections
                .iter()
                .map(|fact| fact["connection_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert!(inventory.truncated);
        let serialized = serde_json::to_string(&inventory.connections).unwrap();
        for excluded in [
            "organization-legacy",
            "other-catalog-legacy",
            "legacy-secret-never-returned",
            "legacy-metadata-never-returned",
        ] {
            assert!(!serialized.contains(excluded), "{serialized}");
        }
    }
    let selected = existing_connections(&f.state.db, &f.owner, &catalog, Some(&[]))
        .await
        .unwrap();
    assert!(selected.connections.is_empty());
    assert!(!selected.truncated);
    // Legacy and unified facts share the same active-owner check.
    f.state
        .db
        .collection::<bson::Document>("users")
        .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert!(
        existing_connections(&f.state.db, &f.owner, &catalog, None)
            .await
            .unwrap()
            .connections
            .is_empty()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn chat_inventory_lists_a_migrated_legacy_connection_once_without_hiding_independent_rows() {
    use crate::services::assistant_connection_service::existing_connections;
    let f = orchestrator_fixture("connect_legacy_migration_dedup").await;
    let catalog = public_connect_catalog(&f, "legacy-migration").await;
    let legacy =
        legacy_catalog_connection(&f, &f.owner, &catalog.id, "Migrated credential", true).await;
    // Give the real startup migration a valid credential-free source. The
    // other inventory fixtures deliberately use an unreadable credential field.
    f.state
        .db
        .collection::<bson::Document>(crate::models::user_service_connection::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&legacy},
            doc! {"$set":{"credential_encrypted":null}},
        )
        .await
        .unwrap();
    crate::db::migrate_service_connections(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let migrated = f.state
        .db
        .collection::<bson::Document>("user_services")
        .find_one(doc! {"user_id":&f.owner,"catalog_service_id":&catalog.id,"source":"migration_connection","source_id":&legacy})
        .projection(doc! {"_id":1})
        .await
        .unwrap()
        .unwrap();
    let migrated_id = migrated.get_str("_id").unwrap();
    for n in 0..9 {
        catalog_connection(&f, &f.owner, &catalog.id, &format!("independent-{n}"), true).await;
    }
    for active in [true, false] {
        f.state
            .db
            .collection::<bson::Document>("user_services")
            .update_one(doc! {"_id":migrated_id}, doc! {"$set":{"is_active":active}})
            .await
            .unwrap();
        let inventory = existing_connections(&f.state.db, &f.owner, &catalog, None)
            .await
            .unwrap();
        assert_eq!(inventory.connections.len(), 10);
        assert!(
            !inventory.truncated,
            "the mirrored legacy row must not count twice"
        );
        assert_eq!(
            inventory
                .connections
                .iter()
                .filter(|fact| fact["connection_id"] == migrated_id)
                .count(),
            1
        );
        assert!(
            inventory
                .connections
                .iter()
                .all(|fact| fact["connection_id"] != legacy)
        );
        assert!(
            inventory
                .connections
                .iter()
                .all(|fact| fact["source"] != "legacy_connection")
        );
        let fact = inventory
            .connections
            .iter()
            .find(|fact| fact["connection_id"] == migrated_id)
            .unwrap();
        assert_eq!(fact["label"], "Migrated credential");
        assert_eq!(fact["is_active"], active);
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn specialist_connect_refusal_inventory_follows_effective_connection_and_auto_grants() {
    use crate::models::assistant_agent::AgentGrants;
    use crate::services::assistant_team_service::{self as team, GrantChange};
    let f = fixture("specialist_connect_inventory").await;
    let catalog = public_connect_catalog(&f, "specialist-connect").await;
    let legacy =
        legacy_catalog_connection(&f, &f.owner, &catalog.id, "legacy-ungranted-account", true)
            .await;
    let granted = catalog_connection(&f, &f.owner, &catalog.id, "granted-account", true).await;
    catalog_connection(&f, &f.owner, &catalog.id, "ungranted-account", true).await;
    let auto = catalog_connection(&f, &f.owner, &catalog.id, "auto-account", true).await;
    let platform = catalog_connection(&f, &f.owner, &catalog.id, "platform-account", true).await;
    let disabled_auto =
        catalog_connection(&f, &f.owner, &catalog.id, "disabled-auto-account", false).await;
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_many(
            doc! {"_id":{"$in":[&auto,&disabled_auto]}},
            doc! {"$set":{"source":crate::models::user_service::AUTO_PROVISION_SOURCE}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&platform},
            doc! {"$set":{"credential_binding":"platform"}},
        )
        .await
        .unwrap();
    team::set_grants(
        &f.state.db,
        &f.owner,
        f.row.agent_id.as_deref().unwrap(),
        GrantChange::Add(AgentGrants {
            service_ids: vec![granted.clone()],
            platform_service_ids: vec![catalog.id.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    // Managed specialists normally set this false. If a live key has the opt-in,
    // authentication and the real execution scope include its active auto rows.
    for allow_auto in [false, true] {
        f.state
            .db
            .collection::<bson::Document>("api_keys")
            .update_one(
                doc! {"_id":&f.row.credential_api_key_id},
                doc! {"$set":{"allow_auto_connected_services":allow_auto}},
            )
            .await
            .unwrap();
        let auth = authenticate(&f).await;
        assert!(!auth.allow_all_services);
        assert!(auth.allowed_service_ids.contains(&granted));
        // Personal effective scopes also merge platform catalog IDs. Their
        // presence must never grant inventory of all that catalog's connections.
        assert!(auth.allowed_service_ids.contains(&catalog.id));
        assert_eq!(auth.allowed_service_ids.contains(&auto), allow_auto);
        assert_eq!(auth.allowed_service_ids.contains(&platform), allow_auto);
        assert!(!auth.allowed_service_ids.contains(&disabled_auto));
        let value = result(
            handle_meta_connect(
                &f.state,
                &auth,
                None,
                &json!({"service_slug":catalog.slug}),
                None,
                false,
            )
            .await,
            true,
        )
        .await;
        let facts = value["existing_connections"].as_array().unwrap();
        assert_eq!(facts.len(), if allow_auto { 3 } else { 1 }, "{value}");
        assert!(facts.iter().all(|fact| fact["connection_id"] != catalog.id));
        for id in auth
            .allowed_service_ids
            .iter()
            .filter(|id| **id != catalog.id)
        {
            assert!(
                facts.iter().any(|fact| fact["connection_id"] == *id),
                "{id}: {value}"
            );
        }
        assert!(!value.to_string().contains("ungranted-account"));
        assert!(!value.to_string().contains("disabled-auto-account"));
        assert!(facts.iter().all(|fact| fact["connection_id"] != legacy));
        assert!(
            facts
                .iter()
                .all(|fact| fact["source"] != "legacy_connection")
        );
        assert!(!value.to_string().contains("legacy-ungranted-account"));
        assert!(value.get("connect_url").is_none());
        assert!(
            value["instructions"]
                .as_str()
                .unwrap()
                .contains("Specialists cannot mint connect links")
        );
    }
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("connect_links")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_specialist_connect_inventory_keeps_the_member_identity_and_only_org_grants() {
    use crate::models::assistant_agent::AgentGrants;
    use crate::services::{assistant_nyxagent as engine, assistant_team_service as team};
    let f =
        crate::services::org_agent_tests::Fixture::new("org_specialist_connect_inventory").await;
    let (agent, _) = f.create().await;
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = uuid::Uuid::new_v4().to_string();
    catalog.slug = "org-specialist-connect".into();
    catalog.visibility = "public".into();
    catalog.requires_user_credential = true;
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&catalog)
        .await
        .unwrap();
    let org_service = f.service(&f.org, "org-granted").await;
    let personal = f.service(&f.member, "member-personal").await;
    let org_ungranted = f.service(&f.org, "org-ungranted").await;
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&personal},
            doc! {"$set":{"source":crate::models::user_service::AUTO_PROVISION_SOURCE}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_many(
            doc! {"_id":{"$in":[&org_service,&personal,&org_ungranted]}},
            doc! {"$set":{"catalog_service_id":&catalog.id}},
        )
        .await
        .unwrap();
    team::set_grants(
        &f.state.db,
        &f.admin,
        &agent.id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![org_service.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    let thread = team::home_thread_for(&f.state.db, &f.state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    let thread = engine::begin_turn(
        &f.state.db,
        &f.member,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: Vec::new(),
            agent_id: None,
            conversation_id: Some(thread.id),
            text: "Connect this service".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let key = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &thread.id,
    )
    .await
    .unwrap()
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", key.raw_key.parse().unwrap());
    for allow_auto in [false, true] {
        f.state
            .db
            .collection::<bson::Document>("api_keys")
            .update_one(
                doc! {"_id":&thread.credential_api_key_id},
                doc! {"$set":{"allow_auto_connected_services":allow_auto}},
            )
            .await
            .unwrap();
        let auth = authenticate_mcp(&f.state, &headers, false).await.unwrap();
        assert_eq!(auth.user_id, f.member);
        assert_eq!(
            auth.assistant_agent_owner_id.as_deref(),
            Some(f.org.as_str())
        );
        assert_eq!(auth.allowed_service_ids, vec![org_service.clone()]);
        let value = result(
            handle_meta_connect(
                &f.state,
                &auth,
                None,
                &json!({"service_id":catalog.id}),
                None,
                false,
            )
            .await,
            true,
        )
        .await;
        let facts = value["existing_connections"].as_array().unwrap();
        assert_eq!(facts.len(), 1, "{value}");
        assert_eq!(facts[0]["connection_id"], org_service);
        assert_eq!(facts[0]["relation"], "organization");
        assert!(!value.to_string().contains("member-personal"));
        assert!(!value.to_string().contains("org-ungranted"));
        assert!(value.get("connect_url").is_none());
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn chat_master_and_platform_catalogs_without_platform_selection_keep_legacy_one_step() {
    let f = orchestrator_fixture("chat_master_legacy_connect").await;
    let auth = authenticate(&f).await;
    for platform_enabled in [false, true] {
        let mut catalog = public_connect_catalog(&f, &format!("master-{platform_enabled}")).await;
        catalog.requires_user_credential = false;
        catalog.auth_method = "bearer".into();
        catalog.service_category = "internal".into();
        catalog.platform_key = Some(crate::models::downstream_service::PlatformKeyConfig {
            purpose: Default::default(),
            enabled: platform_enabled,
            ..Default::default()
        });
        catalog.credential_encrypted = f
            .state
            .encryption_keys
            .encrypt(b"internal-master")
            .await
            .unwrap();
        f.state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                "downstream_services",
            )
            .replace_one(doc! {"_id":&catalog.id}, &catalog)
            .await
            .unwrap();
        let existing = catalog_connection(
            &f,
            &f.owner,
            &catalog.id,
            &format!("existing-{platform_enabled}"),
            true,
        )
        .await;
        for expected in ["connected", "already_connected", "already_connected"] {
            let value = result(
                handle_meta_connect(
                    &f.state,
                    &auth,
                    None,
                    &json!({"service_id":catalog.id}),
                    None,
                    false,
                )
                .await,
                false,
            )
            .await;
            assert_eq!(value["status"], expected, "{value}");
            let facts = value["existing_connections"].as_array().unwrap();
            assert!(facts.iter().any(|fact| fact["connection_id"] == existing));
            assert_eq!(facts.len(), if expected == "connected" { 1 } else { 2 });
            assert!(
                value.get("service_id").is_none(),
                "legacy success must not emit null IDs: {value}"
            );
            if expected == "already_connected" {
                assert_eq!(value["catalog_service_id"], catalog.id);
                assert_eq!(value["service_slug"], catalog.slug);
                assert!(
                    value["instructions"]
                        .as_str()
                        .unwrap()
                        .contains("no separate account to add")
                );
            } else {
                assert!(value.get("service_slug").is_none());
            }
            assert!(value.get("connect_url").is_none());
        }
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>("user_service_connections")
                .count_documents(doc! {"user_id":&f.owner,"service_id":&catalog.id})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>("user_services")
                .count_documents(doc! {"user_id":&f.owner,"catalog_service_id":&catalog.id})
                .await
                .unwrap(),
            1,
            "legacy connects must not create independent unified rows"
        );
        if !platform_enabled {
            let value = result(
                handle_meta_connect(
                    &f.state,
                    &auth,
                    None,
                    &json!({"service_id":catalog.id,"use_platform_key":true}),
                    None,
                    false,
                )
                .await,
                true,
            )
            .await;
            assert_eq!(value["error"], "connect_failed");
            assert!(
                value["existing_connections"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|fact| fact["connection_id"] == existing)
            );
            assert!(value.get("connect_url").is_none());
        }
    }
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("connect_links")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn requirement_only_connect_refuses_only_required_providers_without_a_catalog_credential() {
    let f = orchestrator_fixture("requirement_only_connect").await;
    let mut catalog = seeded_github(&f).await;
    catalog.provider_config_id = None;
    catalog.service_category = "internal".into();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .replace_one(doc! {"_id":&catalog.id}, &catalog)
        .await
        .unwrap();
    let id = catalog_connection(&f, &f.owner, &catalog.id, "existing", true).await;
    let connection =
        crate::services::user_service_service::find_user_service_by_id(&f.state.db, &id)
            .await
            .unwrap()
            .unwrap();
    let auth = authenticate(&f).await;
    for required in [false, true] {
        let updated = f
            .state
            .db
            .collection::<bson::Document>("service_provider_requirements")
            .update_many(
                doc! {"service_id":&catalog.id},
                doc! {"$set":{"required":required}},
            )
            .await
            .unwrap();
        assert!(updated.matched_count > 0);
        for credentialed in [false, true] {
            f.state
                .db
                .collection::<bson::Document>("downstream_services")
                .update_one(
                    doc! {"_id":&catalog.id},
                    doc! {"$set":{"requires_user_credential":credentialed}},
                )
                .await
                .unwrap();
            let unavailable = required && !credentialed;
            let value = result(
                handle_meta_connect(
                    &f.state,
                    &auth,
                    None,
                    &json!({"service_id":catalog.id}),
                    None,
                    false,
                )
                .await,
                unavailable,
            )
            .await;
            assert!(
                value["existing_connections"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|fact| fact["connection_id"] == id)
            );
            let offer = crate::services::assistant_connection_service::connect_own(
                &f.state.db,
                &f.owner,
                &connection,
            )
            .await
            .unwrap();
            if unavailable {
                assert_eq!(value["error"], "connect_unavailable");
                assert_eq!(value["reason"], "provider_requirement_only");
                assert_eq!(offer["reason"], "provider_requirement_only");
                assert_eq!(offer["supported"], false);
                assert!(value.get("connect_url").is_none());
            } else if credentialed {
                assert_eq!(value["status"], "pending_connection");
                assert!(
                    value["connect_url"]
                        .as_str()
                        .is_some_and(|url| url.contains("/connect/"))
                );
                assert_eq!(offer["supported"], true);
            } else {
                assert_eq!(value["status"], "connected");
                assert_eq!(offer["reason"], "no_separate_account");
                assert!(value.get("connect_url").is_none());
            }
        }
    }
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("connect_links")
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_connect_retains_legacy_duplicate_refusal_and_pending_contract() {
    let f = orchestrator_fixture("external_connect_unchanged").await;
    let catalog = seeded_github(&f).await;
    let mut auth = authenticate(&f).await;
    auth.chat = None;
    auth.api_key_id = Some(ordinary_key(&f).await);
    auth.allow_all_services = true;
    // External callers still use the legacy one-step path for this shape.
    f.state
        .db
        .collection::<bson::Document>("downstream_services")
        .update_one(
            doc! {"_id":&catalog.id},
            doc! {"$set":{"service_category":"internal"}},
        )
        .await
        .unwrap();
    let args = json!({"service_id":catalog.id});
    let first = result(
        handle_meta_connect(&f.state, &auth, None, &args, None, false).await,
        false,
    )
    .await;
    assert_eq!(first.as_object().unwrap().len(), 4, "{first}");
    assert!(first.get("existing_connections").is_none());
    let response = handle_meta_connect(&f.state, &auth, None, &args, None, false).await;
    let bytes = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let duplicate: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(duplicate["result"]["isError"], true);
    assert_eq!(
        duplicate["result"]["content"][0]["text"],
        "Conflict: Already connected to this service"
    );
    let mut credentialed = public_connect_catalog(&f, "external-pending").await;
    credentialed.auth_method = "bearer".into();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .replace_one(doc! {"_id":&credentialed.id}, &credentialed)
        .await
        .unwrap();
    let pending = result(
        handle_meta_connect(
            &f.state,
            &auth,
            None,
            &json!({"service_id":credentialed.id}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    assert_eq!(pending["status"], "pending_connection");
    assert_eq!(pending.as_object().unwrap().len(), 8, "{pending}");
    assert!(pending.get("existing_connections").is_none());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn credential_free_chat_connect_is_idempotent_without_unified_rows() {
    let f = orchestrator_fixture("chat_no_auth_legacy").await;
    let mut catalog = public_connect_catalog(&f, "credential-free").await;
    catalog.requires_user_credential = false;
    catalog.auth_method = "none".into();
    catalog.service_category = "internal".into();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .replace_one(doc! {"_id":&catalog.id}, &catalog)
        .await
        .unwrap();
    let auth = authenticate(&f).await;
    let mut legacy_id = None;
    let label = "界".repeat(121);
    for expected in ["connected", "already_connected", "already_connected"] {
        let value = result(
            handle_meta_connect(
                &f.state,
                &auth,
                None,
                &json!({"service_id":catalog.id}),
                None,
                false,
            )
            .await,
            false,
        )
        .await;
        assert_eq!(value["status"], expected, "{value}");
        if expected == "connected" {
            assert_eq!(value["existing_connections"], json!([]));
            let row = f
                .state
                .db
                .collection::<bson::Document>(
                    crate::models::user_service_connection::COLLECTION_NAME,
                )
                .find_one(doc! {"user_id":&f.owner,"service_id":&catalog.id})
                .projection(doc! {"_id":1})
                .await
                .unwrap()
                .unwrap();
            let id = row.get_str("_id").unwrap().to_owned();
            f.state
                .db
                .collection::<bson::Document>(
                    crate::models::user_service_connection::COLLECTION_NAME,
                )
                .update_one(doc! {"_id":&id}, doc! {"$set":{"credential_label":&label}})
                .await
                .unwrap();
            legacy_id = Some(id);
        } else {
            assert_eq!(
                value["existing_connections"],
                json!([{
                    "catalog_service_id":catalog.id,
                    "catalog_service_slug":catalog.slug,
                    "connection_id":legacy_id.as_ref().unwrap(),
                    "connection_slug":catalog.slug,
                    "label":"界".repeat(120),
                    "is_active":true,
                    "owner_type":"person",
                    "relation":"self",
                    "organization_name":null,
                    "account":{"status":"unknown"},
                    "source":"legacy_connection",
                }]),
                "{value}"
            );
        }
        assert_eq!(value["existing_connections_truncated"], false);
        assert!(value.get("service_id").is_none());
        assert!(value.get("connect_url").is_none());
    }
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("user_service_connections")
            .count_documents(doc! {"user_id":&f.owner,"service_id":&catalog.id})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("user_services")
            .count_documents(doc! {"user_id":&f.owner,"catalog_service_id":&catalog.id})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_connect_uses_hosted_link_for_provider_oauth_even_when_already_connected() {
    // Production case: the seeded GitHub catalog gets its credential from an
    // OAuth provider (requires_user_credential=false) and the owner already
    // has a legacy GitHub connection. The legacy connector refused with
    // "Already connected"; a chat must receive a hosted link instead.
    let f = orchestrator_fixture("assistant_provider_connect").await;
    let catalog = seeded_github(&f).await;
    assert!(!catalog.requires_user_credential && catalog.provider_config_id.is_some());
    let legacy =
        legacy_catalog_connection(&f, &f.owner, &catalog.id, "Legacy GitHub credential", true)
            .await;
    let auth = authenticate(&f).await;
    let value = result(
        handle_meta_connect(
            &f.state,
            &auth,
            None,
            &json!({"service_slug":"api-github"}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    assert_eq!(value["status"], "pending_connection", "{value}");
    assert!(
        value["connect_url"]
            .as_str()
            .is_some_and(|url| url.contains("/connect/"))
    );
    let facts = value["existing_connections"].as_array().unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0]["connection_id"], legacy);
    assert_eq!(facts[0]["source"], "legacy_connection");
    assert_eq!(facts[0]["relation"], "self");
    assert_eq!(facts[0]["label"], "Legacy GitHub credential");
    assert_eq!(facts[0]["account"], json!({"status":"unknown"}));
    assert!(!value.to_string().contains("legacy-secret-never-returned"));
    assert!(!value.to_string().contains("legacy-metadata-never-returned"));
    assert_eq!(value["existing_connections_truncated"], false);
    let stored = f
        .state
        .db
        .collection::<crate::models::connect_link::ConnectLink>("connect_links")
        .find_one(doc! {"_id":value["connect_link_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.use_platform_key, Some(false));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_connect_reports_unavailable_providers_and_accepts_configured_device_clients() {
    let f = orchestrator_fixture("assistant_provider_inactive").await;
    let catalog = seeded_github(&f).await;
    catalog_connection(&f, &f.owner, &catalog.id, "existing-offline", true).await;
    let auth = authenticate(&f).await;
    for (active, kind, reason) in [
        (false, "oauth2", "provider_unavailable"),
        (true, "oauth2", "provider_not_configured"),
        (true, "device_code", "provider_not_configured"),
    ] {
        f.state
            .db
            .collection::<bson::Document>("provider_configs")
            .update_one(
                doc! {"_id":catalog.provider_config_id.as_deref().unwrap()},
                doc! {"$set":{"is_active":active,"provider_type":kind,"credential_mode":"admin",
                "client_id_encrypted":bson::Bson::Null,"client_secret_encrypted":bson::Bson::Null}},
            )
            .await
            .unwrap();
        let value = result(
            handle_meta_connect(
                &f.state,
                &auth,
                None,
                &json!({"service_slug":"api-github"}),
                None,
                false,
            )
            .await,
            true,
        )
        .await;
        assert_eq!(value["error"], "connect_unavailable", "{value}");
        assert_eq!(value["reason"], reason, "{kind}: {value}");
        assert_eq!(value["existing_connections"].as_array().unwrap().len(), 1);
        assert!(value.get("connect_url").is_none());
    }
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("connect_links")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    // Public device-code clients need an ID, but no client secret.
    let client_id = f
        .state
        .encryption_keys
        .encrypt(b"public-device-client")
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("provider_configs")
        .update_one(
            doc! {"_id":catalog.provider_config_id.as_deref().unwrap()},
            doc! {"$set":{"client_id_encrypted":bson::Binary {
            subtype:bson::spec::BinarySubtype::Generic, bytes:client_id}}},
        )
        .await
        .unwrap();
    let value = result(
        handle_meta_connect(
            &f.state,
            &auth,
            None,
            &json!({"service_id":catalog.id}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    assert_eq!(value["status"], "pending_connection", "{value}");
    assert!(value["connect_url"].as_str().is_some());
    assert_eq!(value["existing_connections"].as_array().unwrap().len(), 1);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_connect_accepts_catalog_slug_or_id_locks_personal_intent_and_keeps_platform_one_step()
 {
    use crate::services::feature_flag_service as flags;
    let f = orchestrator_fixture("assistant_hosted_connect").await;
    flags::set_platform_override(
        &f.state.db,
        flags::HOSTED_CONNECT_INTENT_FLAG_KEY,
        &flags::FlagTarget::User(f.owner.clone()),
        true,
        &f.owner,
    )
    .await
    .unwrap();
    let mut catalog = public_connect_catalog(&f, "personal-github").await;
    // A credentialed HTTP catalog, as in platform_key_service tests: a
    // no-auth row would take the no-auth platform availability path instead.
    catalog.auth_method = "bearer".into();
    catalog.auth_key_name = "Authorization".into();
    catalog.platform_key = Some(crate::models::downstream_service::PlatformKeyConfig {
        purpose: Default::default(),
        enabled: true,
        audience: crate::models::downstream_service::PlatformKeyAudience::Public,
        ..Default::default()
    });
    catalog.credential_encrypted = f
        .state
        .encryption_keys
        .encrypt(b"fixture-platform-key")
        .await
        .unwrap();
    f.state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .replace_one(doc! {"_id":&catalog.id}, &catalog)
        .await
        .unwrap();
    // An existing organization connection must not suppress personal setup.
    let org = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection("users")
        .insert_one(crate::test_utils::test_user(
            &org,
            crate::models::user::UserType::Org,
        ))
        .await
        .unwrap();
    let connection = connected(&f.state.db, &org, "org-github", "https://example.test").await;
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":connection},
            doc! {"$set":{"catalog_service_id":&catalog.id}},
        )
        .await
        .unwrap();
    let auth = authenticate(&f).await;
    for args in [
        json!({"service_slug":catalog.slug}),
        json!({"service_id":catalog.id}),
    ] {
        let value = result(
            handle_meta_connect(&f.state, &auth, None, &args, None, false).await,
            false,
        )
        .await;
        assert_eq!(value["status"], "pending_connection");
        assert_eq!(value["catalog_service_id"], catalog.id);
        assert_eq!(value["service_slug"], catalog.slug);
        let stored = f
            .state
            .db
            .collection::<crate::models::connect_link::ConnectLink>("connect_links")
            .find_one(doc! {"_id":value["connect_link_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert!(stored.assistant_intent_locked);
        assert_eq!(stored.use_platform_key, Some(false));
    }
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("user_services")
            .count_documents(doc! {"user_id":&f.owner})
            .await
            .unwrap(),
        0,
        "personal intent never provisions without the human completing the link"
    );
    // Platform keys are outside the organization rule: one step, as before.
    let platform = result(
        handle_meta_connect(
            &f.state,
            &auth,
            None,
            &json!({"service_id":catalog.id,"service_slug":catalog.slug,"use_platform_key":true}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    assert_eq!(platform["status"], "connected", "{platform}");
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("user_services")
            .count_documents(doc! {"user_id":&f.owner,"credential_binding":"platform"})
            .await
            .unwrap(),
        1
    );
    let second = result(
        handle_meta_connect(
            &f.state,
            &auth,
            None,
            &json!({"service_id":catalog.id,"use_platform_key":true}),
            None,
            false,
        )
        .await,
        false,
    )
    .await;
    assert_eq!(second["status"], "connected");
    assert_ne!(second["service_id"], platform["service_id"]);
    assert_eq!(second["existing_connections"].as_array().unwrap().len(), 1);
    let response = handle_meta_connect(
        &f.state,
        &auth,
        None,
        &json!({"service_id":catalog.id,"service_slug":"mismatch"}),
        None,
        false,
    )
    .await;
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["result"]["isError"], true);
    assert!(!value.to_string().contains("connect_url"));
    f.state.db.drop().await.unwrap();
}

// #2271: drive both direct native and nyx__call_tool discovery through the
// real authenticated handler. Only synthetic lengths/IDs enter these fixtures.
async fn operation_discovery_fixture(name: &'static str) -> (Fixture, String, Vec<String>) {
    use crate::models::assistant_agent::AgentGrants;
    use crate::services::assistant_team_service as team;
    let f = boxed_chat_fixture(name).await;
    let service = connected(
        &f.state.db,
        &f.owner,
        "discovery-page",
        "http://127.0.0.1:9",
    )
    .await;
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Add(AgentGrants {
            service_ids: vec![service.clone()],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    let mut ids = Vec::new();
    let now = bson::DateTime::now();
    let rows:Vec<_>=(0..120).map(|i| {
        let id=uuid::Uuid::new_v4().to_string(); ids.push(id.clone());
        doc!{"_id":id,"service_id":&service,"name":format!("operation_{i}"),
            "description":"中文 JSON \"value\"\n# Markdown\\path".repeat(8),
            "method":"GET","path":format!("/items/{i}"),"is_active":true,"created_at":now,"updated_at":now}
    }).collect();
    f.state
        .db
        .collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .insert_many(rows)
        .await
        .unwrap();
    ids.sort();
    (f, service, ids)
}

#[tokio::test]
async fn assistant_team_operation_discovery_mcp_pages_are_complete_bounded_and_filtered() {
    let (f, service, expected) = operation_discovery_fixture("discovery_pages").await;
    let auth = authenticate_id(&f, &f.nyxbot_thread).await;
    let hidden = connected(
        &f.state.db,
        &f.owner,
        "ungranted-discovery",
        "http://127.0.0.1:9",
    )
    .await;
    let mut args = json!({"subagent":"worker","service_id":service,"limit":50});
    let mut actual = Vec::new();
    let mut snapshot = None;
    let mut pages = 0usize;
    loop {
        let response = if pages.is_multiple_of(2) {
            call(&f, &auth, "nyxid__get_agent_operations", args.clone()).await
        } else {
            direct_call(&f, &auth, "nyxid__get_agent_operations", args.clone()).await
        };
        let body = axum::body::to_bytes(response.into_body(), 32_000)
            .await
            .unwrap();
        let wire: Value = serde_json::from_slice(&body).unwrap();
        assert!(wire["result"].to_string().len() <= 9200);
        assert_eq!(wire["result"]["isError"], false, "{wire}");
        let value: Value =
            serde_json::from_str(wire["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        if let Some(ref snapshot) = snapshot {
            assert_eq!(&value["snapshot"], snapshot);
        } else {
            snapshot = Some(value["snapshot"].clone());
        }
        assert_eq!(value["services"].as_array().unwrap().len(), 1);
        assert_eq!(value["services"][0]["service_id"], service);
        actual.extend(
            value["services"][0]["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|op| op["endpoint_id"].as_str().unwrap().to_owned()),
        );
        pages += 1;
        if value["complete"] == true {
            assert!(value["next_cursor"].is_null());
            break;
        }
        args["cursor"] = value["next_cursor"].clone();
        assert!(pages < 120);
    }
    assert!(pages > 3);
    assert_eq!(actual, expected);
    for service_id in [hidden, uuid::Uuid::new_v4().to_string()] {
        let value = result(
            call(
                &f,
                &auth,
                "nyxid__get_agent_operations",
                json!({"subagent":"worker","service_id":service_id}),
            )
            .await,
            false,
        )
        .await;
        assert_eq!(value["services"], json!([]));
        assert_eq!(value["complete"], true);
        assert_eq!(value["total"], 0);
    }
    // Discovery remains non-mutating and never creates a confirmation card.
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_team_operation_discovery_live_revocation_and_cursor_binding() {
    use crate::models::assistant_agent::AgentGrants;
    use crate::services::assistant_team_service as team;
    let (f, service, ids) = operation_discovery_fixture("discovery_fences").await;
    let auth = authenticate_id(&f, &f.nyxbot_thread).await;
    let initial = json!({"subagent":"worker","limit":1});
    let first = result(
        call(&f, &auth, "nyxid__get_agent_operations", initial.clone()).await,
        false,
    )
    .await;
    let args = json!({"subagent":"worker","cursor":first["next_cursor"],"limit":1});
    // A specialist cannot borrow NyxBot's cursor, even for its own scope.
    let own = authenticate(&f).await;
    let denied = result(
        call(&f, &own, "nyxid__get_agent_operations", args.clone()).await,
        true,
    )
    .await;
    assert!(denied.get("services").is_none());
    assert_eq!(denied["reason"], "discovery_snapshot_changed");
    assert!(
        denied["message"]
            .as_str()
            .unwrap()
            .contains("without cursor")
    );
    let mut guests = args.clone();
    guests["guests"] = json!(true);
    result(
        call(&f, &auth, "nyxid__get_agent_operations", guests).await,
        true,
    )
    .await;
    let mut filter = args.clone();
    filter["service_id"] = json!(service);
    result(
        call(&f, &auth, "nyxid__get_agent_operations", filter).await,
        true,
    )
    .await;
    // A concurrent contract change invalidates pagination before returning rows.
    f.state
        .db
        .collection::<bson::Document>(crate::models::service_endpoint::COLLECTION_NAME)
        .update_one(doc! {"_id":&ids[0]}, doc! {"$set":{"path":"/changed"}})
        .await
        .unwrap();
    result(
        call(&f, &auth, "nyxid__get_agent_operations", args.clone()).await,
        true,
    )
    .await;
    let first = result(
        call(&f, &auth, "nyxid__get_agent_operations", initial.clone()).await,
        false,
    )
    .await;
    team::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        team::GrantChange::Remove(AgentGrants {
            service_ids: vec![service],
            ..Default::default()
        }),
    )
    .await
    .unwrap();
    let stale = json!({"subagent":"worker","cursor":first["next_cursor"]});
    result(
        call(&f, &auth, "nyxid__get_agent_operations", stale).await,
        true,
    )
    .await;
    let empty = result(
        call(&f, &auth, "nyxid__get_agent_operations", initial).await,
        false,
    )
    .await;
    assert_eq!(empty["services"], json!([]));
    // Cross-owner lookups retain not-found shaping, regardless of cursor.
    let stranger = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &stranger,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let thread = crate::services::assistant_authority_tests::new_orchestrator(
        &f.state,
        &stranger,
        "Read my operations",
    )
    .await;
    let key = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &stranger,
        &thread.id,
    )
    .await
    .unwrap()
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", key.raw_key.parse().unwrap());
    let stranger_auth = authenticate_mcp(&f.state, &headers, false).await.unwrap();
    let denied = result(
        call(
            &f,
            &stranger_auth,
            "nyxid__get_agent_operations",
            json!({"subagent":f.chat.agent_id,"cursor":first["next_cursor"]}),
        )
        .await,
        true,
    )
    .await;
    assert!(denied.get("services").is_none());
    assert_eq!(denied["error"], "not_found");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn approval_gated_native_activity_keeps_tool_bytes_and_reconciles_after_allow() {
    let f = boxed_chat_fixture("approval_activity_native").await;
    let id = connected(
        &f.state.db,
        &f.owner,
        "decision-service",
        "http://127.0.0.1:9",
    )
    .await;
    crate::services::assistant_team_service::set_grants(
        &f.state.db,
        &f.owner,
        &f.chat.agent_id,
        crate::services::assistant_team_service::GrantChange::Add(
            crate::models::assistant_agent::AgentGrants {
                service_ids: vec![id.clone()],
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    let auth = authenticate_id(&f, &f.nyxbot_thread).await;
    let args = json!({"subagent":"worker","services":[id],"access":"read"});
    let response = call(&f, &auth, "nyxid__set_guest_access", args.clone()).await;
    assert_eq!(
        response.extensions().get::<ToolOutcome>(),
        Some(&ToolOutcome::DecisionRequired)
    );
    assert!(tool_failure(&response).is_none());
    let digest = response
        .extensions()
        .get::<ToolResultDigest>()
        .unwrap()
        .0
        .clone();
    let pending = result(response, true).await;
    assert_eq!(pending["error"], "acknowledgement_required");
    assert!(!digest.is_empty());
    let ack = pending["acknowledgement_id"].as_str().unwrap();
    let row = crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.nyxbot_thread)
        .await
        .unwrap();
    let mut a = row.active_turn.unwrap().activities;
    let gated = a
        .iter_mut()
        .find(|a| a.decision.as_ref().is_some_and(|d| d.id == ack))
        .unwrap();
    assert_eq!(gated.decision.as_ref().unwrap().state, "waiting");
    acks::decide(&f.state.db, &f.owner, &f.nyxbot_thread, ack, true)
        .await
        .unwrap();
    let mut args = args;
    args["acknowledgement_id"] = json!(ack);
    result(
        call(&f, &auth, "nyxid__set_guest_access", args).await,
        false,
    )
    .await;
    crate::services::assistant_activity_decision::refresh(
        &f.state.db,
        &f.owner,
        &f.nyxbot_thread,
        vec![gated],
    )
    .await
    .unwrap();
    assert_eq!(
        a.iter().find_map(|a| a.decision.as_ref()).unwrap().state,
        "completed"
    );
    // A provider-authored acknowledgement-shaped result is still a genuine error.
    let forged = rpc_success(
        None,
        json!({"isError":true,"content":[{"type":"text","text":"acknowledgement_required"}],"acknowledgement_id":ack}),
    );
    assert_eq!(
        forged.extensions().get::<ToolOutcome>(),
        Some(&ToolOutcome::Error)
    );
    assert!(tool_failure(&forged).is_some());
    f.state.db.drop().await.unwrap();
}

#[path = "mcp_chat_large_result_tests.rs"]
mod large_results;
