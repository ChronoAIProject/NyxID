use super::*;
use crate::{
    models::{assistant_agent::AgentGrants, assistant_voice::VoicePreferences},
    services::{assistant_authority_tests::direct_fixture, assistant_team_service as team},
};
use axum::{Extension, Router, routing::post};
use mongodb::bson::{Document, doc};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a8ioAAAAASUVORK5CYII=";

fn invocation(name: &str, arguments: Value) -> Invocation {
    Invocation::parse(name, &arguments.to_string())
        .unwrap()
        .unwrap()
}

async fn mcp_server(state: &mut AppState) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.config.base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/mcp", post(crate::handlers::mcp_transport::mcp_post))
        .layer(Extension(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Mcp,
            ),
        ))
        .with_state(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() })
}

async fn call(state: &AppState, owner: &str, conversation: &str) -> VoiceSession {
    super::super::session::ensure_indexes(&state.db)
        .await
        .unwrap();
    crate::services::feature_flag_service::set_platform_override(
        &state.db,
        crate::services::feature_flag_service::ASSISTANT_VOICE_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::Global,
        true,
        owner,
    )
    .await
    .unwrap();
    super::super::session::admit(&state.db, owner, conversation,
        &uuid::Uuid::new_v4().to_string(),
        serde_json::from_value::<VoicePreferences>(json!({"service_id":uuid::Uuid::new_v4().to_string(), "connection_id":uuid::Uuid::new_v4().to_string(),
            "key_source":"own", "model":"gpt-live-1", "voice":"marin", "input_mode":"push_to_talk", "notify_on_completion":false})).unwrap(),
        "identity".into(), "worker").await.unwrap()
}

#[test]
fn native_mcp_functions_reject_identity_overrides_and_malformed_arguments() {
    for (name, args) in [
        (
            "nyx__search_tools",
            json!({"query":"camera", "agent_id":"other"}),
        ),
        ("nyx__list_connected_services", json!({"user_id":"owner"})),
        (
            "nyx__call_tool",
            json!({"tool_name":"x", "arguments_json":"{}", "approved":true}),
        ),
        (
            "nyx__call_tool",
            json!({"tool_name":"x", "arguments_json":"[]"}),
        ),
        (
            "nyx__call_tool",
            json!({"tool_name":"x", "arguments_json":"{"}),
        ),
    ] {
        assert!(Invocation::parse(name, &args.to_string()).is_err());
    }
    assert!(Invocation::parse("unknown", "{}").unwrap().is_none());
}

#[tokio::test]
async fn voice_tools_use_selected_bot_grants_and_live_revocation_through_real_mcp() {
    Box::pin(async {
        let mut f = direct_fixture("voice_direct_tools_grants").await;
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"temperature":22})))
            .mount(&upstream)
            .await;
        let service = crate::services::assistant_authority_tests::connected(
            &f.state.db,
            &f.owner,
            "sensor",
            &upstream.uri(),
        )
        .await;
        let other = crate::services::assistant_authority_tests::connected(
            &f.state.db,
            &f.owner,
            "private-device",
            &upstream.uri(),
        )
        .await;
        let call = call(&f.state, &f.owner, &f.row.id).await;
        let server = mcp_server(&mut f.state).await;
        let mut tools = Executions::default();
        let list = invocation("nyx__list_connected_services", json!({}));
        let empty = tools
            .invoke(&f.state, &call, "delegation", "list", &list)
            .await
            .unwrap();
        // Ordinary MCP discovery may advertise withheld connections with an
        // explicit access label. Visibility is not execution authority.
        let inventory: Value = serde_json::from_str(empty["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(inventory["services"].as_array().unwrap().iter()
            .find(|service| service["slug"] == "sensor").unwrap()["chat_access"], "acknowledgement_required");
        let read = invocation("nyx__call_tool", json!({"tool_name":"sensor__request",
            "arguments_json":json!({"method":"GET", "path":"/temperature"}).to_string()}));
        let ungranted = tools.invoke(&f.state, &call, "delegation", "ungranted", &read).await.unwrap();
        assert_eq!(ungranted["isError"], true, "{ungranted}");
        assert!(upstream.received_requests().await.unwrap().is_empty());
        let agent = team::agent_for_conversation(&f.state.db, &f.row)
            .await
            .unwrap();
        team::set_grants(
            &f.state.db,
            &f.owner,
            &agent.id,
            team::GrantChange::Add(AgentGrants {
                service_ids: vec![service.clone()],
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        use crate::services::feature_flag_service as flags;
        flags::set_platform_override(&f.state.db, flags::AGENT_OPERATION_SCOPES_FLAG_KEY,
            &flags::FlagTarget::Global, true, &f.owner).await.unwrap();
        crate::services::agent_operation_scope_service::set(&f.state.db, &f.owner, &agent.id, &service,
            &serde_json::from_value(json!({"expected_revision":0, "rules":[{"method":"GET", "path_template":"/temperature"}]})).unwrap(),
            false).await.unwrap();
        let write = invocation("nyx__call_tool", json!({"tool_name":"sensor__request",
            "arguments_json":json!({"method":"POST", "path":"/temperature"}).to_string()}));
        let restricted = tools.invoke(&f.state, &call, "delegation", "write-denied", &write).await.unwrap();
        assert_eq!(restricted["isError"],true,"{restricted}");
        assert!(upstream.received_requests().await.unwrap().is_empty());
        let context = &tools.active["delegation"].row;
        assert_eq!(context.agent_id, f.row.agent_id);
        assert_ne!(context.credential_api_key_id, f.row.credential_api_key_id);
        assert_eq!(
            context.voice_parent_conversation_id.as_deref(),
            Some(f.row.id.as_str())
        );
        let grant = crate::services::key_service::get_api_key(
            &f.state.db,
            &f.owner,
            &context.credential_api_key_id,
        )
        .await
        .unwrap();
        assert!(grant.allowed_service_ids.contains(&service));
        assert!(!grant.allowed_service_ids.contains(&other));
        let result = tools
            .invoke(&f.state, &call, "delegation", "read", &read)
            .await
            .unwrap();
        assert_ne!(result["isError"], true, "{result}");
        assert!(result.to_string().contains("22"), "{result}");
        team::set_grants(
            &f.state.db,
            &f.owner,
            &agent.id,
            team::GrantChange::Remove(AgentGrants {
                service_ids: vec![service],
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        let denied = tools
            .invoke(&f.state, &call, "delegation", "revoked", &read)
            .await
            .unwrap();
        assert_eq!(denied["isError"], true, "{denied}");
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::assistant_voice::REQUESTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "No full NyxAgent task or model invocation"
        );
        tools.close(&f.state).await;
        assert!(tools.active.is_empty());
        assert_eq!(
            f.state
                .db
                .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
                .count_documents(
                    doc! {"voice_parent_conversation_id":&f.row.id, "active_turn":{"$ne":null}}
                )
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            tools
                .invoke(&f.state, &call, "delegation", "closed", &read)
                .await
                .unwrap()["isError"],
            true
        );
        tools.close(&f.state).await;
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
        server.abort();
    })
    .await;
}

#[tokio::test]
async fn voice_tool_context_expiry_settles_the_turn_and_keeps_the_call_open() {
    Box::pin(async {
        let (mut state, _, call) = super::super::tests::setup("voice_tools_expiry").await;
        let server = mcp_server(&mut state).await;
        let mut tools = Executions::default();
        let list = invocation("nyx__list_connected_services", json!({}));
        tools
            .invoke(&state, &call, "delegation", "list", &list)
            .await
            .unwrap();
        let context = tools.active["delegation"].row.id.clone();
        tools.active.get_mut("delegation").unwrap().deadline = tokio::time::Instant::now();
        tools.heartbeat(&state).await.unwrap();
        assert!(tools.active.is_empty());
        assert!(
            engine::get(&state.db, &call.user_id, &context)
                .await
                .unwrap()
                .active_turn
                .is_none()
        );
        assert!(
            !super::super::session::refresh(&state.db, &call)
                .await
                .unwrap()
                .end_requested
        );
        assert_eq!(
            tools
                .invoke(&state, &call, "delegation", "again", &list)
                .await
                .unwrap()["isError"],
            false
        );
        tools.close(&state).await;
        server.abort();
    })
    .await;
}

#[test]
fn mcp_images_become_visual_input_without_fetching_urls_or_repeating_base64() {
    let (output, images) = presentation(json!({"content":[
        {"type":"image","mimeType":"image/png","data":PNG},
        {"type":"image","mimeType":"image/svg+xml","data":"aGVsbG8="},
        {"type":"image","mimeType":"image/png","data":"https://private.invalid/image"},
        {"type":"text","text":"Untrusted tool text"}
    ]}));
    assert_eq!(images.len(), 1);
    assert_eq!(
        images[0]["image_url"],
        format!("data:image/png;base64,{PNG}")
    );
    assert!(!output.to_string().contains(PNG));
    assert!(!output.to_string().contains("aGVsbG8="));
    assert!(output.to_string().contains("could not be supplied"));
    assert_eq!(output["content"][3]["text"], "Untrusted tool text");
}

#[tokio::test]
async fn voice_mcp_client_does_not_follow_redirects_with_bot_credentials() {
    Box::pin(async {
        let (mut state, _, call) = super::super::tests::setup("voice_mcp_no_redirect").await;
        let origin = MockServer::start().await;
        let destination = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(307).insert_header("Location", destination.uri()))
            .mount(&origin)
            .await;
        state.config.base_url = origin.uri();
        let mut tools = Executions::default();
        let list = invocation("nyx__list_connected_services", json!({}));
        let result = tools
            .invoke(&state, &call, "delegation", "list", &list)
            .await
            .unwrap();
        assert_eq!(result["retry_allowed"], false);
        assert!(destination.received_requests().await.unwrap().is_empty());
        assert_eq!(origin.received_requests().await.unwrap().len(), 1);
        assert!(
            tools.active.is_empty(),
            "Uncertain work cannot share a later call's turn"
        );
        let server = mcp_server(&mut state).await;
        let followup = tools
            .invoke(&state, &call, "delegation", "followup", &list)
            .await
            .unwrap();
        assert_eq!(followup["isError"], false, "{followup}");
        assert_eq!(
            origin.received_requests().await.unwrap().len(),
            1,
            "No replay"
        );
        tools.close(&state).await;
        server.abort();
    })
    .await;
}

#[tokio::test]
async fn voice_tool_wait_keeps_coordinator_pollable_and_close_settles_pending_turns() {
    Box::pin(async {
        let (mut state, _, call) = super::super::tests::setup("voice_mcp_worker_cancel").await;
        let origin = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)
            .set_delay(Duration::from_secs(5))).mount(&origin).await;
        state.config.base_url = origin.uri();
        let list = invocation("nyx__list_connected_services", json!({}));
        let mut workers = Workers::default();
        workers.start(&state, &call, "delegation", "list", &list).unwrap();
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                tokio::select! {
                    _ = workers.next() => panic!("The service request is still waiting"),
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {
                        if !origin.received_requests().await.unwrap().is_empty() { break; }
                    }
                }
            }
        }).await.unwrap();
        assert!(workers.pending());
        assert_eq!(workers.status()[0]["call_id"],"list");
        tokio::time::timeout(Duration::from_secs(3), workers.close(&state)).await.unwrap();
        assert!(!workers.pending());
        assert!(workers.status().as_array().unwrap().is_empty());
        assert_eq!(state.db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .count_documents(doc!{"voice_parent_conversation_id":&call.conversation_id,"active_turn":{"$ne":null}}).await.unwrap(),0);
        assert!(!super::super::session::refresh(&state.db,&call).await.unwrap().end_requested);
    }).await;
}

#[tokio::test]
async fn voice_tool_settlement_failure_retains_context_for_call_cleanup() {
    Box::pin(async {
        let (mut state, _, call) = super::super::tests::setup("voice_tools_cleanup_retry").await;
        let server = mcp_server(&mut state).await;
        let mut workers = Workers::default();
        let list = invocation("nyx__list_connected_services", json!({}));
        workers
            .start(&state, &call, "delegation", "list", &list)
            .unwrap();
        assert!(workers.next().await.unwrap().result.is_ok());
        let context = workers.contexts["delegation"].clone();
        let id = context.lock().await.active["delegation"].row.id.clone();
        let mut unavailable_state = state.clone();
        let mut options = mongodb::options::ClientOptions::parse("mongodb://127.0.0.1:1")
            .await
            .unwrap();
        options.server_selection_timeout = Some(Duration::from_millis(100));
        unavailable_state.db = mongodb::Client::with_options(options)
            .unwrap()
            .database("unavailable");
        assert!(
            workers
                .finish(&unavailable_state, "delegation", false)
                .await
                .is_err()
        );
        assert!(workers.contexts.contains_key("delegation"));
        assert!(context.lock().await.active.contains_key("delegation"));
        workers.close(&state).await;
        assert!(context.lock().await.active.is_empty());
        assert!(
            engine::get(&state.db, &call.user_id, &id)
                .await
                .unwrap()
                .active_turn
                .is_none()
        );
        server.abort();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn voice_tool_cancel_after_durable_admission_keeps_exact_cleanup_identity() {
    Box::pin(async {
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{
            Condvar, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        let (mut state, _, call) = super::super::tests::setup("voice_tools_admit_cancel").await;
        let original_db = state.db.clone();
        let committed = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let armed = Arc::new(AtomicBool::new(false));
        let handler = EventHandler::callback({
            let committed = committed.clone();
            let release = release.clone();
            let armed = armed.clone();
            move |event| {
                if let CommandEvent::Succeeded(event) = event
                    && event.command_name == "commitTransaction"
                    && armed.swap(false, Ordering::SeqCst)
                {
                    committed.notify_one();
                    let (lock, cv) = &*release;
                    let _ = cv
                        .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(5), |ready| {
                            !*ready
                        })
                        .unwrap();
                }
            }
        });
        let monitored = crate::test_utils::connect_test_database_with_command_handler(
            "voice_tools_monitor",
            handler,
        )
        .await
        .unwrap();
        state.db = monitored.client().database(original_db.name());
        armed.store(true, Ordering::SeqCst);
        let origin = MockServer::start().await;
        state.config.base_url = origin.uri();
        let workers = Arc::new(tokio::sync::Mutex::new(Workers::default()));
        workers
            .lock()
            .await
            .start(
                &state,
                &call,
                "delegation",
                "list",
                &invocation("nyx__list_connected_services", json!({})),
            )
            .unwrap();
        let running = tokio::spawn({
            let workers = workers.clone();
            async move { workers.lock().await.next().await }
        });
        tokio::time::timeout(Duration::from_secs(4), committed.notified())
            .await
            .unwrap();
        let rows = original_db.collection::<AssistantConversation>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        );
        let admitted = rows
            .find_one(doc! {"voice_parent_conversation_id":&call.conversation_id,
            "active_turn":{"$ne":null}})
            .await
            .unwrap()
            .expect("turn is already durable");
        running.abort();
        {
            let (lock, cv) = &*release;
            *lock.lock().unwrap() = true;
            cv.notify_one();
        }
        assert!(matches!(running.await, Err(error) if error.is_cancelled()));
        let mut workers = workers.lock().await;
        workers.cancel_pending();
        workers.close(&state).await;
        assert!(
            rows.find_one(doc! {"_id":&admitted.id})
                .await
                .unwrap()
                .unwrap()
                .active_turn
                .is_none()
        );
        assert!(
            origin.received_requests().await.unwrap().is_empty(),
            "Cancellation must not send a tool"
        );
    })
    .await;
}

#[tokio::test]
async fn voice_async_tool_reuses_existing_watch_without_a_second_task() {
    Box::pin(async {
        use crate::models::async_service_operation::{AsyncServiceOperation, COLLECTION_NAME as WATCHES};
        use crate::handlers::mcp_transport::async_operations;
        use wiremock::matchers::path;
        let mut f = direct_fixture("voice_async_tool_watch").await;
        let upstream = MockServer::start().await;
        Mock::given(method("POST")).and(path("/executions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"operation_id":"voice-op-1","status":"queued"})))
            .expect(1).mount(&upstream).await;
        Mock::given(method("GET")).and(path("/executions/voice-op-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"succeeded"})))
            .mount(&upstream).await;
        Mock::given(method("GET")).and(path("/executions/voice-op-1/result"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"output":"script finished"})))
            .mount(&upstream).await;
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id=uuid::Uuid::new_v4().to_string(); catalog.slug="chrono-sandbox".into();
        catalog.base_url=upstream.uri(); catalog.auth_method="none".into();
        catalog.requires_user_credential=false; catalog.created_by="admin".into();
        f.state.db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME).insert_one(&catalog).await.unwrap();
        crate::services::catalog_spec_sync::sync_seeded_service_endpoints(&f.state.db).await.unwrap();
        let service=crate::services::assistant_authority_tests::connected(
            &f.state.db,&f.owner,"sandbox",&upstream.uri()).await;
        f.state.db.collection::<Document>(crate::models::user_service::COLLECTION_NAME)
            .update_one(doc!{"_id":&service},doc!{"$set":{"catalog_service_id":&catalog.id}}).await.unwrap();
        let agent=team::agent_for_conversation(&f.state.db,&f.row).await.unwrap();
        team::set_grants(&f.state.db,&f.owner,&agent.id,team::GrantChange::Add(AgentGrants{
            service_ids:vec![service],..Default::default()})).await.unwrap();
        crate::services::feature_flag_service::set_platform_override(&f.state.db,
            crate::services::feature_flag_service::ASSISTANT_GENERATED_ARTIFACTS_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,true,&f.owner).await.unwrap();
        let call=call(&f.state,&f.owner,&f.row.id).await;
        let server=mcp_server(&mut f.state).await;
        let state=f.state.clone();
        let sweeper=tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                // Make polling due only after submission has completed. Advancing
                // a submitting row invokes crash recovery, not normal polling.
                state.db.collection::<Document>(WATCHES).update_many(doc!{"state":"waiting"},
                    doc!{"$set":{"poll_at":bson::DateTime::from_millis(0)}}).await.unwrap();
                Box::pin(async_operations::sweep(&state)).await.unwrap();
            }
        });
        let mut tools=Executions::default();
        let submit=invocation("nyx__call_tool",json!({"tool_name":"sandbox__submit_execution_handler",
            "arguments_json":json!({"Idempotency-Key":"voice-async","script":"print('hello')","language":"python"}).to_string()}));
        let result=tokio::time::timeout(Duration::from_secs(20),
            tools.invoke(&f.state,&call,"delegation","async-call",&submit)).await.unwrap().unwrap();
        assert!(result.to_string().contains("script finished"),"{result}");
        let watch=f.state.db.collection::<AsyncServiceOperation>(WATCHES).find_one(doc!{}).await.unwrap().unwrap();
        assert_eq!(watch.operation_id.as_deref(),Some("voice-op-1"));
        assert_eq!(watch.state,"delivered");
        assert!(watch.artifact_mode.is_none(), "Voice must not enter the web-only attachment workflow");
        assert!(watch.result_encrypted.is_none());
        assert_eq!(f.state.db.collection::<Document>(crate::models::assistant_voice::REQUESTS)
            .count_documents(doc!{}).await.unwrap(),0,"No second Voice task is created");
        tools.finish(&f.state,"delegation",false).await.unwrap();
        sweeper.abort(); server.abort(); upstream.verify().await;
    }).await;
}

#[tokio::test]
async fn voice_service_policy_refuses_new_approval_but_uses_existing_live_grant() {
    Box::pin(async {
        use crate::services::approval_service as approvals;
        let mut f = direct_fixture("voice_service_existing_approval").await;
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"temperature":22})))
            .expect(1)
            .mount(&upstream)
            .await;
        let service = crate::services::assistant_authority_tests::connected(
            &f.state.db,
            &f.owner,
            "sensor",
            &upstream.uri(),
        )
        .await;
        let agent = team::agent_for_conversation(&f.state.db, &f.row)
            .await
            .unwrap();
        team::set_grants(
            &f.state.db,
            &f.owner,
            &agent.id,
            team::GrantChange::Add(AgentGrants {
                service_ids: vec![service.clone()],
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        approvals::set_service_approval_config(
            &f.state.db,
            &f.owner,
            &service,
            "Sensor",
            Some(true),
            Some(&crate::models::service_approval_config::ApprovalMode::Grant),
            None,
            None,
        )
        .await
        .unwrap();
        let call = call(&f.state, &f.owner, &f.row.id).await;
        let server = mcp_server(&mut f.state).await;
        let mut tools = Executions::default();
        let read = invocation(
            "nyx__call_tool",
            json!({"tool_name":"sensor__request",
            "arguments_json":json!({"method":"GET", "path":"/temperature"}).to_string()}),
        );
        let refused = tools
            .invoke(&f.state, &call, "delegation", "needs-approval", &read)
            .await
            .unwrap();
        assert_eq!(refused["isError"], true, "{refused}");
        assert!(
            refused
                .to_string()
                .contains(&AppError::ApprovalUnsupported.to_string()),
            "{refused}"
        );
        assert!(upstream.received_requests().await.unwrap().is_empty());
        let auth = crate::mw::auth::api_key_auth_user_for_voice(
            &f.state.db,
            &tools.active["delegation"].row,
        )
        .await
        .unwrap();
        let now = chrono::Utc::now();
        let grant = crate::models::approval_grant::ApprovalGrant {
            execution_review: None,
            execution_last_used_at: None,
            execution_use: None,
            execution: None,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: f.owner.clone(),
            service_id: service,
            service_name: "Sensor".into(),
            requester_type: auth.approval_requester_type().unwrap().into(),
            requester_id: auth.approval_requester_id(),
            requester_label: None,
            approval_request_id: uuid::Uuid::new_v4().to_string(),
            scope: None,
            granted_at: now,
            expires_at: now + chrono::Duration::hours(1),
            revoked: false,
            org_scoped: false,
        };
        f.state
            .db
            .collection::<crate::models::approval_grant::ApprovalGrant>("approval_grants")
            .insert_one(&grant)
            .await
            .unwrap();
        let allowed = tools
            .invoke(&f.state, &call, "delegation", "existing-grant", &read)
            .await
            .unwrap();
        assert_ne!(allowed["isError"], true, "{allowed}");
        assert!(allowed.to_string().contains("22"), "{allowed}");
        approvals::revoke_grant(&f.state.db, &f.owner, &grant.id)
            .await
            .unwrap();
        let revoked = tools
            .invoke(&f.state, &call, "delegation", "revoked", &read)
            .await
            .unwrap();
        assert_eq!(revoked["isError"], true, "{revoked}");
        for collection in ["approval_requests", "assistant_acknowledgements"] {
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0,
                "{collection}"
            );
        }
        tools.close(&f.state).await;
        server.abort();
        upstream.verify().await;
    })
    .await;
}

#[tokio::test]
async fn voice_action_and_permission_requests_refuse_without_creating_cards() {
    Box::pin(async {
        use crate::services::assistant_acknowledgement_service as acks;
        let (mut state, _, call) = super::super::tests::setup("voice_no_new_approvals").await;
        let server = mcp_server(&mut state).await;
        let mut tools = Executions::default();
        let list = invocation("nyx__list_connected_services", json!({}));
        tools
            .invoke(&state, &call, "delegation", "list", &list)
            .await
            .unwrap();
        let row = &tools.active["delegation"].row;
        let chat = acks::for_key(&state.db, &call.user_id, Some(&row.credential_api_key_id))
            .await
            .unwrap()
            .unwrap();
        assert!(chat.execution_voice_task);
        let need = acks::ConfirmationNeed {
            org_instruction: Default::default(),
            class: acks::OwnerActionClass::Change,
            mode: None,
            webhook: false,
            policy: true,
            delegation: false,
        };
        let refusal = acks::confirm_action(
            &state.db,
            &chat,
            "test_action",
            &json!({}),
            &need,
            "Needs a decision",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(refusal["error"], "approval_unsupported");
        let refusal = acks::confirm_action(
            &state.db,
            &chat,
            "test_action",
            &json!({"acknowledgement_id":"unknown"}),
            &need,
            "Needs a decision",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(refusal["error"], "approval_unsupported");
        let request = acks::request(
            &state.db,
            &chat,
            acks::Request {
                kind: "service",
                service: Some(("service", "catalog", "Test")),
                tool: None,
                arguments: None,
                summary: "New access",
                platform: false,
            },
        )
        .await;
        assert!(matches!(request, Err(AppError::ApprovalUnsupported)));
        for collection in [
            "assistant_acknowledgements",
            "approval_requests",
            crate::models::assistant_sharing::CONSENTS_COLLECTION_NAME,
        ] {
            assert_eq!(
                state
                    .db
                    .collection::<Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0,
                "{collection}"
            );
        }
        tools.close(&state).await;
        server.abort();
    })
    .await;
}
