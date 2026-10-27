use super::*;
use crate::{
    models::{assistant_settings::AccessMode, org_membership::OrgRole, user::UserType},
    services::assistant_authority_tests::{Fixture, connected, orchestrator_fixture},
    test_utils,
};
use axum::http::HeaderMap;
use uuid::Uuid;

#[path = "assistant_service_execution_authority_tests.rs"]
mod authority_tests;
#[path = "assistant_service_execution_compaction_tests.rs"]
mod compaction;
#[path = "assistant_service_execution_compatibility_tests.rs"]
mod compatibility;
#[path = "assistant_service_execution_egress_tests.rs"]
mod egress;
#[path = "assistant_service_execution_journey_tests.rs"]
mod journeys;
#[path = "assistant_service_execution_managed_tests.rs"]
mod managed;
#[path = "assistant_service_execution_native_tests.rs"]
mod native;
#[path = "assistant_service_execution_receipt_tests.rs"]
mod receipts;
#[path = "assistant_service_execution_ssh_session_tests.rs"]
mod ssh_sessions;

#[path = "assistant_service_execution_invocation_tests.rs"]
mod invocations;

async fn mode(f: &Fixture, mode: AccessMode) {
    super::super::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        super::super::assistant_settings_service::Update {
            access_mode: Some(mode),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}
async fn org(f: &Fixture) -> String {
    let id = Uuid::new_v4().to_string();
    f.state
        .db
        .collection("users")
        .insert_one(test_utils::test_user(&id, UserType::Org))
        .await
        .unwrap();
    f.state
        .db
        .collection("org_memberships")
        .insert_one(test_utils::test_membership(
            &id,
            &f.owner,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    id
}
async fn catalog(f: &Fixture) -> String {
    let mut row = test_utils::test_auto_connected_catalog_service();
    row.inference = Some(crate::models::downstream_service::ServiceInference {
        wire_protocol: crate::models::downstream_service::InferenceWireProtocol::OpenaiCompletions,
        model_list: true,
        realtime: false,
        voice: None,
    });
    let id = row.id.clone();
    f.state
        .db
        .collection("downstream_services")
        .insert_one(row)
        .await
        .unwrap();
    id
}
async fn connection(f: &Fixture, owner: &str, slug: &str, catalog: Option<&str>) -> String {
    let id = connected(&f.state.db, owner, slug, "https://example.com").await;
    if let Some(catalog) = catalog {
        f.state
            .db
            .collection::<Document>("user_services")
            .update_one(
                doc! {"_id": &id},
                doc! {"$set": {"catalog_service_id": catalog}},
            )
            .await
            .unwrap();
    }
    id
}
async fn choose(f: &Fixture, id: &str) {
    let mut row = scope(&f.chat).unwrap();
    row.insert(
        "expires_at",
        bson::DateTime::from_chrono(Utc::now() + Duration::minutes(10)),
    );
    let connection = super::super::user_service_service::find_user_service_by_id(&f.state.db, id)
        .await
        .unwrap()
        .unwrap();
    row.extend(doc! {"_id":Uuid::new_v4().to_string(),"kind":"selection","connection_id":id,
        "selection_source":"owner","owner_id":connection.user_id,"catalog_service_id":connection.catalog_service_id});
    f.state
        .db
        .collection::<Document>(COLLECTION)
        .insert_one(row)
        .await
        .unwrap();
}
async fn gate(
    f: &Fixture,
    id: &str,
    owner: &str,
    operation: &Value,
    ack: Option<&str>,
) -> Decision {
    check(
        &f.state.db,
        &f.state.node_ws_manager,
        &f.chat,
        id,
        owner,
        operation,
        acks::ActionFacts::of(acks::OwnerActionClass::Read),
        ack,
        "Read selected service",
        false,
        None,
        false,
        None,
    )
    .await
    .unwrap()
}
async fn allow(f: &Fixture, refusal: Value) -> String {
    let id = refusal["acknowledgement_id"].as_str().unwrap().to_owned();
    acks::decide(&f.state.db, &f.owner, &f.row.id, &id, true)
        .await
        .unwrap();
    id
}
async fn target(f: &Fixture, id: &str) -> super::super::proxy_service::ProxyTarget {
    super::super::proxy_service::resolve_proxy_target_by_user_service_id(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        id,
        None,
        None,
        super::super::proxy_service::ProxyExecutionContext::new(
            None,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .target
}
async fn raw(
    f: &Fixture,
    id: &str,
    owner: &str,
    body: &[u8],
    ack: Option<&str>,
    headers: &HeaderMap,
) -> Decision {
    let mut auth = f.auth.clone();
    auth.assistant_chat = Some(std::sync::Arc::new(f.chat.clone()));
    check_http(
        &f.state,
        &auth,
        Some(id),
        Some(owner),
        &target(f, id).await,
        "POST",
        "/v1/chat/completions",
        None,
        body,
        ack,
        None,
        headers,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn org_reads_confirm_in_auto_and_ask_but_never_full_access() {
    let f = orchestrator_fixture("execution_mode_matrix").await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "org", None).await;
    let personal = connection(&f, &f.owner, "personal", None).await;
    for setting in [
        AccessMode::Nyxmode,
        AccessMode::AskEveryTime,
        AccessMode::Autopilot,
    ] {
        mode(&f, setting).await;
        assert!(
            gate(&f, &personal, &f.owner, &json!({"path":"/read"}), None)
                .await
                .refusal
                .is_none()
        );
        let result = gate(
            &f,
            &id,
            &owner,
            &json!({"path":"/read","mode":format!("{setting:?}")}),
            None,
        )
        .await;
        assert_eq!(result.refusal.is_some(), setting != AccessMode::Autopilot);
    }
}

#[tokio::test]
async fn unrelated_custom_singletons_do_not_compete() {
    let f = orchestrator_fixture("execution_custom_groups").await;
    let a = connection(&f, &f.owner, "mail", None).await;
    let b = connection(&f, &f.owner, "github", None).await;
    assert!(
        gate(&f, &a, &f.owner, &json!({"path":"/mail"}), None)
            .await
            .refusal
            .is_none()
    );
    assert!(
        gate(&f, &b, &f.owner, &json!({"path":"/repos"}), None)
            .await
            .refusal
            .is_none()
    );
}

#[tokio::test]
async fn saved_priority_and_human_selection_are_enforced_before_org_consent() {
    let f = orchestrator_fixture("execution_preference").await;
    let c = catalog(&f).await;
    let owner = org(&f).await;
    let a = connection(&f, &f.owner, "first", Some(&c)).await;
    let b = connection(&f, &f.owner, "second", Some(&c)).await;
    let o = connection(&f, &owner, "org", Some(&c)).await;
    f.state.db.collection::<Document>("service_preferences").insert_one(doc! {"_id":&f.owner,"ordered":[&b,&a,&o],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    let op = json!({"kind":"inference","model":"same","provider":c});
    assert_eq!(
        gate(&f, &a, &f.owner, &op, None).await.refusal.unwrap()["preferred_connection_id"],
        b
    );
    assert_eq!(
        gate(&f, &o, &owner, &op, None).await.refusal.unwrap()["preferred_connection_id"],
        b
    );
    choose(&f, &o).await;
    assert_eq!(
        gate(&f, &a, &f.owner, &op, None).await.refusal.unwrap()["error"],
        "service_selection_required"
    );
    mode(&f, AccessMode::Nyxmode).await;
    assert_eq!(
        gate(&f, &o, &owner, &op, None).await.refusal.unwrap()["error"],
        "acknowledgement_required"
    );
}

#[tokio::test]
async fn exact_consent_binds_body_headers_and_catalog_generation() {
    let f = orchestrator_fixture("execution_exact_binding").await;
    let owner = org(&f).await;
    let c = catalog(&f).await;
    let id = connection(&f, &owner, "org", Some(&c)).await;
    mode(&f, AccessMode::Nyxmode).await;
    let t = target(&f, &id).await;
    let mut op = http_operation(
        &t,
        "POST",
        "/v1/chat/completions",
        None,
        br#"{"model":"m","messages":[{"content":"original"}]}"#,
    );
    bind_headers(&mut op, &[("x-project".into(), "one".into())]);
    let ack = allow(&f, gate(&f, &id, &owner, &op, None).await.refusal.unwrap()).await;
    let mut changed = op.clone();
    bind_headers(&mut changed, &[("x-project".into(), "two".into())]);
    assert_eq!(
        gate(&f, &id, &owner, &changed, Some(&ack))
            .await
            .refusal
            .unwrap()["error"],
        "acknowledgement_invalid"
    );
    changed = op.clone();
    changed["body"] = json!("changed prompt");
    assert_eq!(
        gate(&f, &id, &owner, &changed, Some(&ack))
            .await
            .refusal
            .unwrap()["error"],
        "acknowledgement_invalid"
    );
    changed = op.clone();
    bind_headers(
        &mut changed,
        &[("x-http-method-override".into(), "DELETE".into())],
    );
    assert_eq!(
        gate(&f, &id, &owner, &changed, Some(&ack))
            .await
            .refusal
            .unwrap()["error"],
        "acknowledgement_invalid"
    );
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(doc! {"_id":&c}, doc! {"$set":{"forward_access_token":true}})
        .await
        .unwrap();
    assert_eq!(
        gate(&f, &id, &owner, &op, Some(&ack))
            .await
            .refusal
            .unwrap()["error"],
        "acknowledgement_invalid"
    );
}

#[tokio::test]
async fn held_resolution_cannot_be_authorized_by_a_fresh_database_digest() {
    let f = orchestrator_fixture("execution_held_drift").await;
    let id = connection(&f, &f.owner, "local", None).await;
    let actual = target(&f, &id).await;
    let service = super::super::user_service_service::get_user_service(&f.state.db, &f.owner, &id)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! {"_id":service.endpoint_id},
            doc! {"$set":{"url":"https://changed.example"}},
        )
        .await
        .unwrap();
    assert!(
        verify_resolved(
            &f.state.db,
            &f.state.encryption_keys,
            &f.chat,
            &id,
            &actual,
            "GET",
            "/read"
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn real_connection_failure_qualifies_but_replacement_and_uncertain_write_do_not() {
    let f = orchestrator_fixture("execution_fallback").await;
    let c = catalog(&f).await;
    let a = connection(&f, &f.owner, "a", Some(&c)).await;
    let b = connection(&f, &f.owner, "b", Some(&c)).await;
    f.state.db.collection::<Document>("service_preferences").insert_one(doc! {"_id":&f.owner,"ordered":[&a,&b],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    let op = json!({"kind":"inference","provider":c,"path":"/read"});
    let admission = gate(&f, &a, &f.owner, &op, None).await.admission.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let error = reqwest::Client::new()
        .get(format!("http://{addr}/"))
        .send()
        .await
        .unwrap_err();
    assert!(error.is_connect());
    admission
        .transport_failure(
            &f.state.db,
            &f.chat,
            &super::super::proxy_service::ForwardRequestError::Transport(error),
        )
        .await
        .unwrap();
    assert!(gate(&f, &b, &f.owner, &op, None).await.refusal.is_none());
    admission
        .transport_failure(
            &f.state.db,
            &f.chat,
            &super::super::proxy_service::ForwardRequestError::OutcomeUnknown,
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&a}, doc! {"$inc":{"state_version":1_i64}})
        .await
        .unwrap();
    assert_eq!(
        gate(&f, &b, &f.owner, &op, None).await.refusal.unwrap()["error"],
        "service_priority_required"
    );
}

#[tokio::test]
async fn raw_model_consent_is_bounded_to_turn_model_and_semantic_headers() {
    let f = orchestrator_fixture("execution_model_grant").await;
    let owner = org(&f).await;
    let c = catalog(&f).await;
    let id = connection(&f, &owner, "model", Some(&c)).await;
    mode(&f, AccessMode::Nyxmode).await;
    let body = br#"{"model":"m","messages":[{"role":"user","content":"hello"}]}"#;
    let mut headers = HeaderMap::new();
    headers.insert(
        "traceparent",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
            .parse()
            .unwrap(),
    );
    let ack = allow(
        &f,
        raw(&f, &id, &owner, body, None, &headers)
            .await
            .refusal
            .unwrap(),
    )
    .await;
    assert!(
        raw(&f, &id, &owner, body, Some(&ack), &headers)
            .await
            .refusal
            .is_none()
    );
    headers.insert(
        "traceparent",
        "00-5bf92f3577b34da6a3ce929d0e0e4736-10f067aa0ba902b7-01"
            .parse()
            .unwrap(),
    );
    assert!(
        raw(
            &f,
            &id,
            &owner,
            br#"{"model":"m","messages":[{"role":"tool","content":"result"}]}"#,
            None,
            &headers
        )
        .await
        .refusal
        .is_none()
    );
    assert!(
        raw(&f, &id, &owner, br#"{"model":"another"}"#, None, &headers)
            .await
            .refusal
            .is_some()
    );
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&id}, doc! {"$inc":{"state_version":1_i64}})
        .await
        .unwrap();
    let mut auth = f.auth.clone();
    auth.assistant_chat = Some(std::sync::Arc::new(f.chat.clone()));
    assert!(
        check_http(
            &f.state,
            &auth,
            Some(&id),
            Some(&owner),
            &target(&f, &id).await,
            "POST",
            "/v1/chat/completions",
            None,
            body,
            None,
            None,
            &headers
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn personal_model_preserves_ask_mode_for_non_user_initiating_roles() {
    let f = orchestrator_fixture("execution_event_inference").await;
    let c = catalog(&f).await;
    let id = connection(&f, &f.owner, "model", Some(&c)).await;
    mode(&f, AccessMode::AskEveryTime).await;
    f.state
        .db
        .collection::<Document>("assistant_messages")
        .update_many(
            doc! {"conversation_id":&f.row.id,"role":"user"},
            doc! {"$set":{"role":"event","origin":"event"}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(COLLECTION)
        .delete_many(doc! {"conversation_id":&f.row.id,"kind":"input_admission"})
        .await
        .unwrap();
    assert!(
        raw(
            &f,
            &id,
            &f.owner,
            br#"{"model":"m"}"#,
            None,
            &HeaderMap::new()
        )
        .await
        .refusal
        .is_none()
    );
}

#[tokio::test]
async fn automatic_receipts_require_clean_successful_completion() {
    let f = orchestrator_fixture("execution_auto_receipts").await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "org", None).await;
    mode(&f, AccessMode::Autopilot).await;
    for status in [200, 503] {
        let decision = gate(
            &f,
            &id,
            &owner,
            &json!({"path":"/read","status":status}),
            None,
        )
        .await;
        let response = axum::response::Response::builder()
            .status(status)
            .body(axum::body::Body::from("body"))
            .unwrap();
        let response = decision
            .finish_http(
                response,
                f.state.db.clone(),
                Some(f.chat.clone()),
                "GET",
                None,
            )
            .await;
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
    }
    assert_eq!(
        f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {"conversation_id":&f.row.id,"decided_by":"access_mode"})
            .await
            .unwrap(),
        1
    );
}

#[test]
fn owner_selector_accepts_inline_requests_and_bounds_only_the_reference() {
    for input in [
        "/service connection continue the search",
        "/service connection\nContinue the search",
        "/service\tconnection\tcontinue the search",
        "/service  connection \r\nContinue the search",
    ] {
        assert_eq!(owner_selector(input), Some("connection"), "{input:?}");
    }
    let longest = "a".repeat(128);
    assert_eq!(
        owner_selector(&format!("/service {longest} {}", "request ".repeat(100))),
        Some(longest.as_str())
    );
    for input in [
        "/service".to_owned(),
        "/service \t".to_owned(),
        "/service\nconnection".to_owned(),
        "/service-other connection".to_owned(),
        " /service connection".to_owned(),
        "> /service connection".to_owned(),
        "Quoted\n/service connection".to_owned(),
        format!("/service {} continue", "a".repeat(129)),
    ] {
        assert_eq!(owner_selector(&input), None, "{input:?}");
    }
}

#[test]
fn selector_is_leading_exact_and_header_binding_excludes_only_control_metadata() {
    assert_eq!(
        owner_selector("/service connection\nDo work"),
        Some("connection")
    );
    assert_eq!(owner_selector("Quoted /service connection"), None);
    let mut a = json!({});
    let mut b = json!({});
    bind_headers(
        &mut a,
        &[
            (ACK_HEADER.into(), "one".into()),
            ("x-account".into(), "same".into()),
        ],
    );
    bind_headers(
        &mut b,
        &[
            (ACK_HEADER.into(), "two".into()),
            ("x-account".into(), "same".into()),
        ],
    );
    assert_eq!(a, b);
    bind_headers(&mut b, &[("x-account".into(), "different".into())]);
    assert_ne!(a, b);
}

#[test]
fn held_execution_fingerprint_binds_credentials_within_the_process_and_preserves_pool_projection() {
    let mut target = super::super::proxy_service::ProxyTarget {
        service: test_utils::test_auto_connected_catalog_service(),
        base_url: "https://example.invalid".into(),
        auth_method: "bearer".into(),
        auth_key_name: "Authorization".into(),
        credential: super::super::api_key_credential_service::generate_secret()
            .to_string()
            .into(),
        catalog_default_headers: Vec::new(),
        user_service_default_headers: Vec::new(),
        ws_frame_injections: Vec::new(),
        connection_id: None,
        target_id: None,
        workspace_destinations_pending: false,
    };
    let held = execution_digest(&target);
    let pool = execution_projection_digest(&target, false);
    assert!(
        held == execution_digest(&target),
        "unchanged held authority"
    );
    assert!(pool == execution_projection_digest(&target, false));
    target.credential = super::super::api_key_credential_service::generate_secret()
        .to_string()
        .into();
    assert!(held != execution_digest(&target), "replacement must differ");
    assert!(pool == execution_projection_digest(&target, false));
    target.base_url.push_str("/different");
    assert!(pool != execution_projection_digest(&target, false));
}

#[tokio::test]
async fn guest_1995_route_binding_uses_durable_credential_identity() {
    use crate::services::user_api_key_service::{CreateApiKeyParams, create_api_key};
    let f = orchestrator_fixture("guest_1995_durable_route").await;
    let id = connection(&f, &f.owner, "model-route", None).await;
    let credential = create_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        CreateApiKeyParams {
            label: "model route",
            credential_type: "bearer",
            credential: "test-only-model-key",
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
    f.state.db.collection::<Document>("user_services").update_one(doc! {"_id":&id},
        doc! {"$set":{"api_key_id":&credential.id,"auth_method":"bearer","credential_binding":"user"}}).await.unwrap();
    let mut target = target(&f, &id).await;
    let operation = json!({"kind":"raw_inference","model":"test","input_admission":"first-turn"});
    let bound = guest_model::route(
        &f.state.db,
        &f.chat,
        &f.row.model,
        Some(&id),
        &target,
        &operation,
    )
    .await
    .unwrap();
    let mut next_chat = f.chat.clone();
    next_chat.api_key_id = Uuid::new_v4().to_string();
    let mut next_operation = operation.clone();
    next_operation["input_admission"] = json!("next-turn");
    // A different replica/turn must not depend on held credential MACs or
    // ephemeral refreshed token bytes. Explicit replacement advances the epoch.
    target.credential = "refreshed-token".to_owned().into();
    let next = guest_model::route(
        &f.state.db,
        &next_chat,
        &f.row.model,
        Some(&id),
        &target,
        &next_operation,
    )
    .await
    .unwrap();
    assert!(bound == next);
    f.state
        .db
        .collection::<Document>("user_api_keys")
        .update_one(
            doc! {"_id":&credential.id},
            doc! {"$inc":{"credential_epoch":1_i64}},
        )
        .await
        .unwrap();
    let replaced = guest_model::route(
        &f.state.db,
        &next_chat,
        &f.row.model,
        Some(&id),
        &target,
        &next_operation,
    )
    .await
    .unwrap();
    assert!(
        bound != replaced,
        "credential replacement requires owner establishment"
    );
    f.state.db.drop().await.unwrap();
}

#[test]
fn raw_inference_is_conservative_about_state_and_provider_effects() {
    let good = json!({"model":"test", "input":"hello", "store":false,
        "tools":[{"type":"function", "name":"lookup", "parameters":{"type":"object"}}]});
    assert!(stateless_inference("/v1/responses", &good));
    for (key, value) in [
        ("prompt", json!({"id":"pmpt_1"})),
        ("container", json!("cntr_1")),
        ("background", json!(true)),
        ("store", json!(true)),
        (
            "tools",
            json!([{"type":"mcp","server_url":"https://remote.example"}]),
        ),
        ("model", Value::Null),
        ("input", json!([{"type":"input_file","file_id":"file_1"}])),
    ] {
        let mut bad = good.clone();
        bad[key] = value;
        assert!(!stateless_inference("/v1/responses", &bad), "{key}");
    }
    let mut missing_storage = good;
    missing_storage.as_object_mut().unwrap().remove("store");
    assert!(!stateless_inference("/v1/responses", &missing_storage));
    assert!(!stateless_inference("/v1/responses", &json!([])));
}
