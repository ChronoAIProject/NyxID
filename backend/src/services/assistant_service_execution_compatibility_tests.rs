//! Rollout compatibility uses real idle conversation keys, never synthetic turns.
use super::*;
use crate::services::{
    assistant_agent_credential_service as credentials, assistant_nyxagent as engine,
    feature_flag_service as flags,
};
use axum::{body::Body, http::Request};
use tower::ServiceExt;

pub(super) async fn idle(f: &mut Fixture) {
    engine::finish_turn(
        &f.state.db,
        &f.row,
        &f.row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Completed".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    f.chat = acks::for_key(&f.state.db, &f.owner, Some(&f.row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert!(!f.chat.turn_live);
    assert!(f.chat.turn_id.is_none());
}

pub(super) async fn flag(f: &Fixture, enabled: bool) {
    flags::set_platform_override(
        &f.state.db,
        flags::ASSISTANT_LIVE_TURN_GATE_FLAG_KEY,
        &flags::FlagTarget::User(f.owner.clone()),
        enabled,
        &f.owner,
    )
    .await
    .unwrap();
}

pub(super) async fn grant(f: &Fixture, id: &str) {
    f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.chat.api_key_id},
            doc! {"$set":{"allow_all_services":false,"allowed_service_ids":[id]}},
        )
        .await
        .unwrap();
}

async fn service(f: &Fixture, owner: &str, url: &str) -> String {
    let catalog = catalog(f).await;
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&catalog},
            doc! {"$set":{"inference":bson::Bson::Null}},
        )
        .await
        .unwrap();
    for (name, method) in [("read", "GET"), ("change", "POST"), ("destroy", "DELETE")] {
        f.state
            .db
            .collection("service_endpoints")
            .insert_one(test_utils::test_service_endpoint(
                &catalog,
                name,
                method,
                &format!("/{name}"),
            ))
            .await
            .unwrap();
    }
    let id = connected(&f.state.db, owner, "idle-service", url).await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"catalog_service_id":catalog}},
        )
        .await
        .unwrap();
    grant(f, &id).await;
    id
}

async fn call(f: &Fixture, mcp: bool, method: &str, name: &str) -> (u16, Value) {
    let key = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let (uri, method, body) = if mcp {
        (
            "/mcp".to_owned(),
            "POST",
            json!({
                "jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":format!("idle-service__{name}"),"arguments":{}},
            })
            .to_string(),
        )
    } else {
        (
            format!("/api/v1/proxy/s/idle-service/{name}"),
            method,
            "{}".to_owned(),
        )
    };
    let (_, router) = crate::routes::build_router();
    let response = router
        .with_state(f.state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("x-api-key", key.raw_key.as_str())
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn assert_call(f: &Fixture, mcp: bool, method: &str, name: &str, allowed: bool) {
    let (status, body) = call(f, mcp, method, name).await;
    if mcp && allowed {
        assert_eq!(status, 200, "{body}");
        assert!(body.get("error").is_none(), "{body}");
        assert_eq!(body["result"]["isError"], false, "{body}");
    } else if mcp {
        assert!(
            body.get("error").is_some() || body["result"]["isError"] == true,
            "{body}"
        );
        assert_eq!(body["error"]["code"], -32603, "{body}");
        assert_eq!(
            body["error"]["message"], "Assistant turn is no longer active",
            "{body}"
        );
    } else {
        assert_eq!(status, if allowed { 200 } else { 409 }, "{body}");
        if !allowed {
            assert_eq!(body["error"], "assistant_turn_required", "{body}");
        }
    }
}

#[tokio::test]
async fn idle_personal_http_preserves_rollout_and_existing_mode_gates() {
    let mut f = orchestrator_fixture("idle_personal_rollout").await;
    mode(&f, AccessMode::Nyxmode).await;
    flag(&f, false).await;
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .mount(&server)
        .await;
    service(&f, &f.owner, &server.uri()).await;
    idle(&mut f).await;
    assert_call(&f, false, "GET", "read", true).await;
    assert_call(&f, false, "POST", "change", true).await;
    assert_call(&f, false, "DELETE", "destroy", false).await;
    mode(&f, AccessMode::AskEveryTime).await;
    assert_call(&f, false, "POST", "change", false).await;
    mode(&f, AccessMode::Autopilot).await;
    assert_call(&f, false, "DELETE", "destroy", true).await;
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    flag(&f, true).await;
    assert_call(&f, false, "GET", "read", false).await;
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    assert_eq!(
        f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {"kind":"action","status":"pending"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION)
            .count_documents(
                doc! {"kind":{"$in":["card_binding","model_consent","unavailable_bucket"]}}
            )
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn idle_mcp_keeps_main_activity_fence_while_direct_executor_respects_rollout() {
    let mut f = orchestrator_fixture("idle_mcp_main_contract").await;
    mode(&f, AccessMode::Nyxmode).await;
    flag(&f, false).await;
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .mount(&server)
        .await;
    let id = service(&f, &f.owner, &server.uri()).await;
    idle(&mut f).await;
    // main's handle_tool_call_with_activity requires a tracked live activity,
    // even with the rollout off. The HTTP/executor gates do not remove it.
    assert_call(&f, true, "GET", "read", false).await;
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        super::native::node_service_call(&f, &id, true)
            .await
            .unwrap(),
        200
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    flag(&f, true).await;
    assert!(matches!(
        super::native::node_service_call(&f, &id, true).await,
        Err(AppError::AssistantTurnRequired)
    ));
    let (_, denied) = call(&f, true, "GET", "read").await;
    assert_eq!(denied["error"]["code"], -32003, "{denied}");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {"status":"pending"})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn idle_org_http_and_mcp_never_inherit_personal_rollout_compatibility() {
    for mcp in [false, true] {
        let mut f = orchestrator_fixture("idle_org_rollout").await;
        mode(&f, AccessMode::Autopilot).await;
        flag(&f, false).await;
        let owner = org(&f).await;
        let server = wiremock::MockServer::start().await;
        service(&f, &owner, &server.uri()).await;
        idle(&mut f).await;
        assert_call(&f, mcp, "GET", "read", false).await;
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(
            f.state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .count_documents(doc! {"status":"pending"})
                .await
                .unwrap(),
            0
        );
    }
}

async fn admit_read(f: &Fixture, id: &str) -> Admission {
    check(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.chat,
        id,
        &f.owner,
        &json!({"kind":"http","method":"GET","path":"/read"}),
        acks::ActionFacts::of(acks::OwnerActionClass::Read),
        None,
        "Read",
        false,
        None,
        false,
        None,
    )
    .await
    .unwrap()
    .admission
    .unwrap()
}

#[tokio::test]
async fn held_idle_personal_admission_rechecks_rollout_key_grant_mode_config_and_turn() {
    for drift in ["flag", "key", "grant", "mode", "config", "turn"] {
        let mut f = orchestrator_fixture("held_idle_personal").await;
        flag(&f, false).await;
        let id = connection(&f, &f.owner, "idle", None).await;
        grant(&f, &id).await;
        idle(&mut f).await;
        let admission = admit_read(&f, &id).await;
        admission.revalidate(&f.state.db, &id, None).await.unwrap();
        match drift {
            "flag" => flag(&f, true).await,
            "key" => {
                f.state
                    .db
                    .collection::<Document>("api_keys")
                    .update_one(
                        doc! {"_id":&f.chat.api_key_id},
                        doc! {"$set":{"is_active":false}},
                    )
                    .await
                    .unwrap();
            }
            "grant" => {
                f.state
                    .db
                    .collection::<Document>("api_keys")
                    .update_one(
                        doc! {"_id":&f.chat.api_key_id},
                        doc! {"$set":{"allowed_service_ids":[]}},
                    )
                    .await
                    .unwrap();
            }
            "mode" => mode(&f, AccessMode::Autopilot).await,
            "config" => {
                f.state.db.collection::<Document>("user_services").update_one(
                doc! {"_id":&id},doc! {"$set":{"default_request_headers":[{"name":"x-account-id","value":"changed","overridable":false}]}},
            ).await.unwrap();
            }
            "turn" => {
                begin_next_turn(&f).await;
            }
            _ => unreachable!(),
        }
        assert!(
            admission.revalidate(&f.state.db, &id, None).await.is_err(),
            "{drift}"
        );
    }
}

async fn begin_next_turn(f: &Fixture) {
    engine::begin_turn(
        &f.state.db,
        &f.owner,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
            agent_id: None,
            conversation_id: Some(f.row.id.clone()),
            text: "Next request".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn live_admission_never_downgrades_to_idle_after_stop_or_a_new_turn() {
    for next_turn in [false, true] {
        let mut f = orchestrator_fixture("live_no_idle_downgrade").await;
        flag(&f, false).await;
        let id = connection(&f, &f.owner, "live", None).await;
        grant(&f, &id).await;
        let admission = admit_read(&f, &id).await;
        if next_turn {
            idle(&mut f).await;
            assert!(admission.revalidate(&f.state.db, &id, None).await.is_err());
            begin_next_turn(&f).await;
        } else {
            f.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":&f.row.id},
                    doc! {"$set":{"active_turn.stop_requested":true}},
                )
                .await
                .unwrap();
        }
        assert!(matches!(
            admission.revalidate(&f.state.db, &id, None).await,
            Err(AppError::AssistantTurnRequired)
        ));
    }
}

#[tokio::test]
async fn mcp_authenticated_live_request_cannot_become_an_idle_admission() {
    let mut f = orchestrator_fixture("mcp_initial_live_fence").await;
    flag(&f, false).await;
    let server = wiremock::MockServer::start().await;
    let id = service(&f, &f.owner, &server.uri()).await;
    let authenticated = f.chat.clone();
    idle(&mut f).await;
    f.chat = authenticated;
    // The executor carries the original authentication snapshot through target
    // resolution. Fresh credentials cannot turn a settled request into an idle
    // legacy call when admission finally runs.
    assert!(matches!(
        native::node_service_call(&f, &id, true).await,
        Err(AppError::AssistantTurnRequired)
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}
