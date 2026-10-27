//! Sanitized regression for the independent Codex -> model-service HTTP hop.
use super::*;
use crate::models::downstream_service::{InferenceWireProtocol, ServiceInference};

#[path = "assistant_service_execution_permissions_denial_tests.rs"]
mod denial;

pub(super) fn callback() -> Value {
    serde_json::from_str(include_str!("fixtures/permissions-codex-responses.json")).unwrap()
}

fn captured_and_derived_generations() -> Vec<Value> {
    [
        include_str!("../../../docs/fixtures/codex-public-responses-0.160.1.json"),
        include_str!(
            "../../../docs/fixtures/codex-public-responses-0.160.1-native-schema-derived.json"
        ),
    ]
    .into_iter()
    .flat_map(|source| {
        let captured: Value = serde_json::from_str(source).unwrap();
        captured["calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|call| call["codex"].clone())
            .collect::<Vec<_>>()
    })
    .collect()
}

#[test]
fn codex_original_and_derived_generations_are_stateless_inference() {
    // Four original Codex calls plus four unchanged native-schema-derived cases.
    let captured = captured_and_derived_generations();
    assert_eq!(captured.len(), 8);
    for body in captured {
        // Check the captured grammar without exposing correlation IDs or bodies.
        let window = body["client_metadata"]["x-codex-window-id"]
            .as_str()
            .unwrap();
        assert!(window.contains(':'));
        assert!(stateless_inference("responses", &body));
        let operation = http_operation(
            &inference_target(),
            "POST",
            "responses",
            None,
            &serde_json::to_vec(&body).unwrap(),
        );
        assert_eq!(operation["kind"], "inference");
    }
}

fn inference_target() -> crate::services::proxy_service::ProxyTarget {
    let mut service = test_utils::test_auto_connected_catalog_service();
    service.slug = "chrono-llm-public".into();
    service.inference = Some(ServiceInference {
        wire_protocol: InferenceWireProtocol::OpenaiCompletions,
        model_list: true,
        realtime: false,
        voice: None,
    });
    crate::services::proxy_service::ProxyTarget {
        service,
        base_url: "https://example.invalid".into(),
        auth_method: "none".into(),
        auth_key_name: String::new(),
        credential: String::new().into(),
        catalog_default_headers: vec![],
        user_service_default_headers: vec![],
        ws_frame_injections: vec![],
        connection_id: None,
        target_id: None,
        workspace_destinations_pending: false,
    }
}

#[test]
fn codex_callback_fixture_is_inference_and_keeps_body_and_header_bindings() {
    let body = callback();
    let metadata = body["client_metadata"].as_object().unwrap();
    assert_eq!(metadata.len(), 7);
    let captured: Value = serde_json::from_str(include_str!(
        "../../../docs/fixtures/codex-public-responses-0.160.1.json"
    ))
    .unwrap();
    let observed = captured["calls"][0]["codex"]["client_metadata"]
        .as_object()
        .unwrap();
    // Compare shape and byte lengths only. Never print captured field values.
    assert_eq!(
        metadata.keys().collect::<Vec<_>>(),
        observed.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        metadata
            .values()
            .map(|v| v.as_str().unwrap().len())
            .collect::<Vec<_>>(),
        observed
            .values()
            .map(|v| v.as_str().unwrap().len())
            .collect::<Vec<_>>()
    );
    let lengths: Vec<_> = metadata
        .values()
        .map(|v| v.as_str().unwrap().len())
        .collect();
    assert_eq!(lengths, [36, 36, 36, 36, 36, 697, 38]);
    assert_eq!(serde_json::to_vec(metadata).unwrap().len(), 1121);
    assert!(stateless_inference("/v1/responses", &body));
    let mut without_metadata = body.clone();
    without_metadata
        .as_object_mut()
        .unwrap()
        .remove("client_metadata");
    assert!(stateless_inference("responses", &without_metadata));
    let target = inference_target();
    let operation = http_operation(
        &target,
        "POST",
        "/responses",
        Some("fixture=1"),
        &serde_json::to_vec(&body).unwrap(),
    );
    assert_eq!(operation["kind"], "inference");
    assert_eq!(operation["provider"], target.service.id);
    assert_eq!(operation["query"], "fixture=1");
    let mut changed = body.clone();
    changed["client_metadata"]["turn_id"] = json!("different");
    let changed = http_operation(
        &target,
        "POST",
        "/responses",
        Some("fixture=1"),
        &serde_json::to_vec(&changed).unwrap(),
    );
    assert_ne!(operation["body"], changed["body"]);
    let mut a = operation.clone();
    let mut b = operation;
    bind_headers(&mut a, &[("x-account".into(), "one".into())]);
    bind_headers(&mut b, &[("x-account".into(), "two".into())]);
    assert_ne!(a["headers"], b["headers"]);
}

#[test]
fn codex_client_metadata_enforces_each_scalar_and_utf8_boundary() {
    for key in [
        "root_turn_id",
        "session_id",
        "thread_id",
        "turn_id",
        "x-codex-installation-id",
        "x-codex-window-id",
    ] {
        for (value, accepted) in [
            ("a".into(), true),
            ("A0_-".repeat(32), true),
            (String::new(), false),
            ("a".repeat(129), false),
            ("é".into(), false),
            ("a/b".into(), false),
            ("a b".into(), false),
            ("a\n".into(), false),
            ("a.b".into(), false),
            ("a:0".into(), key == "x-codex-window-id"),
        ] {
            let mut body = callback();
            body["client_metadata"][key] = json!(value);
            assert_eq!(stateless_inference("responses", &body), accepted, "{key}");
        }
        for value in [
            Value::Null,
            json!(1),
            json!(true),
            json!([]),
            json!({"file_id":"fixture"}),
        ] {
            let mut body = callback();
            body["client_metadata"][key] = value;
            assert!(!stateless_inference("responses", &body), "{key}");
        }
        let mut body = callback();
        body["client_metadata"].as_object_mut().unwrap().remove(key);
        assert!(!stateless_inference("responses", &body), "missing {key}");
    }
    for (value, accepted) in [
        ("00000000-0000-4000-8000-000000000006:0".into(), true),
        (format!("{}:0", "a".repeat(126)), true),
        (format!("{}:0", "a".repeat(127)), false),
        ("id:0123456789".into(), true),
        ("id:".into(), false),
        (":0".into(), false),
        ("id:-1".into(), false),
        ("id:+1".into(), false),
        ("id:1:2".into(), false),
        ("id:１".into(), false),
        ("id:0\n".into(), false),
        ("id:0x1".into(), false),
    ] {
        let mut body = callback();
        body["client_metadata"]["x-codex-window-id"] = json!(value);
        assert_eq!(stateless_inference("responses", &body), accepted);
    }
    for (value, accepted) in [
        (String::new(), true),
        ("m".repeat(1024), true),
        ("m".repeat(1025), false),
        ("é".repeat(512), true),
        (format!("{}a", "é".repeat(512)), false),
    ] {
        let mut body = callback();
        body["client_metadata"]["x-codex-turn-metadata"] = json!(value);
        assert_eq!(stateless_inference("responses", &body), accepted);
    }
    for value in [
        Value::Null,
        json!(1),
        json!(true),
        json!([]),
        json!({"thread_id":"fixture"}),
    ] {
        let mut body = callback();
        body["client_metadata"]["x-codex-turn-metadata"] = value;
        assert!(!stateless_inference("responses", &body));
    }
}

#[test]
fn codex_client_metadata_counts_sorted_compact_json_escaping_at_object_limit() {
    for (escapes, bytes, accepted) in [(114, 2048, true), (115, 2049, false)] {
        let mut body = callback();
        let metadata = body["client_metadata"].as_object_mut().unwrap();
        for (key, value) in metadata.iter_mut() {
            *value = if key == "x-codex-turn-metadata" {
                json!(format!(
                    "{}{}",
                    "\"".repeat(escapes),
                    "m".repeat(1024 - escapes)
                ))
            } else {
                json!("a".repeat(128))
            };
        }
        assert_eq!(serde_json::to_vec(metadata).unwrap().len(), bytes);
        assert_eq!(stateless_inference("responses", &body), accepted);
        // Input key order/whitespace must not affect the canonical byte budget.
        let serialized = serde_json::to_string_pretty(&body).unwrap();
        assert_eq!(
            stateless_inference("responses", &serde_json::from_str(&serialized).unwrap()),
            accepted
        );
    }
}

fn unsafe_callbacks() -> Vec<(&'static str, Value)> {
    let mut cases = Vec::new();
    for (key, value) in [
        ("model", Value::Null),
        ("model", json!(" ")),
        ("store", json!(true)),
        ("background", json!(true)),
        ("conversation", json!("fixture")),
        ("previous_response_id", json!("fixture")),
        ("file_id", json!("fixture")),
        ("unknown_extension", json!("fixture")),
        ("input", json!([{"file_id":"fixture"}])),
        ("input", json!([{"nested":{"thread_id":"fixture"}}])),
        (
            "input",
            json!([{"mcp_servers":[{"server_url":"https://example.invalid"}]}]),
        ),
        ("tools", json!([{"type":"computer_use_preview"}])),
        (
            "tools",
            json!([{"type":"code_interpreter","container":"auto"}]),
        ),
        ("tools", json!([{"type":"mcp","server_label":"fixture"}])),
        ("client_metadata", json!([])),
        ("client_metadata", json!("fixture")),
        ("client_metadata", Value::Null),
    ] {
        let mut body = callback();
        body[key] = value;
        cases.push((key, body));
    }
    let mut unknown = callback();
    unknown["client_metadata"]["unknown"] = json!("fixture");
    cases.push(("unknown metadata key", unknown));
    let mut replacement = callback();
    let metadata = replacement["client_metadata"].as_object_mut().unwrap();
    assert!(metadata.remove("x-codex-turn-metadata").is_some());
    metadata.insert("unknown".into(), json!("fixture"));
    assert_eq!(metadata.len(), 7);
    cases.push(("replacement metadata key", replacement));
    let mut missing = callback();
    missing.as_object_mut().unwrap().remove("store");
    cases.push(("missing store", missing));
    cases
}

#[test]
fn codex_metadata_does_not_admit_stateful_provider_or_unknown_effects() {
    let target = inference_target();
    for (reason, body) in unsafe_callbacks() {
        assert!(!stateless_inference("responses", &body), "{reason}");
        assert_eq!(
            http_operation(
                &target,
                "POST",
                "responses",
                None,
                &serde_json::to_vec(&body).unwrap()
            )["kind"],
            "http",
            "{reason}"
        );
    }
    for path in [
        "responses/compact",
        "responses/fixture",
        "files",
        "conversations",
        "external-write",
        "chat/completions",
    ] {
        assert!(!stateless_inference(path, &callback()), "{path}");
    }
    let body = serde_json::to_vec(&callback()).unwrap();
    assert_eq!(
        http_operation(&target, "DELETE", "responses", None, &body)["kind"],
        "http"
    );
    let mut other = target;
    other.service.inference = None;
    assert_eq!(
        http_operation(&other, "POST", "responses", None, &body)["kind"],
        "http"
    );
}

async fn projected(
    f: &Fixture,
    auth: &crate::mw::auth::AuthUser,
    target: &crate::services::proxy_service::ProxyTarget,
    id: &str,
    body: &[u8],
    path: &str,
    headers: &HeaderMap,
) -> Decision {
    check_http_projected(
        &f.state,
        auth,
        Some(id),
        Some(&f.owner),
        target,
        "POST",
        path,
        None,
        body,
        None,
        None,
        headers,
        &HttpProjection::default(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn codex_callback_projected_read_path_keeps_exact_negative_approvals() {
    let f = orchestrator_fixture("permissions_codex_projection").await;
    mode(&f, AccessMode::AskEveryTime).await;
    let c = catalog(&f).await;
    let id = connection(&f, &f.owner, "chrono-llm-public", Some(&c)).await;
    let resolved = target(&f, &id).await;
    let mut auth = f.auth.clone();
    auth.assistant_chat = Some(std::sync::Arc::new(f.chat.clone()));
    let projection = HttpProjection::default();
    let headers = HeaderMap::new();
    let body = serde_json::to_vec(&callback()).unwrap();
    let admitted = projected(&f, &auth, &resolved, &id, &body, "responses", &headers).await;
    assert!(admitted.refusal.is_none());
    let admission = admitted.admission.unwrap();
    assert!(admission.read);
    assert_eq!(admission.operation_identity["kind"], "raw_inference");
    admission
        .revalidate(&f.state.db, &id, Some(&resolved))
        .await
        .unwrap();
    let cards = f
        .state
        .db
        .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
            "assistant_acknowledgements",
        );
    assert_eq!(
        cards
            .count_documents(doc! {"conversation_id":&f.row.id})
            .await
            .unwrap(),
        0
    );
    for captured in captured_and_derived_generations() {
        let bytes = serde_json::to_vec(&captured).unwrap();
        let decision = projected(&f, &auth, &resolved, &id, &bytes, "responses", &headers).await;
        assert!(decision.refusal.is_none());
        let admission = decision.admission.unwrap();
        assert!(admission.read);
        assert_eq!(admission.operation_identity["kind"], "raw_inference");
        admission
            .revalidate(&f.state.db, &id, Some(&resolved))
            .await
            .unwrap();
    }
    assert_eq!(
        cards
            .count_documents(doc! {"conversation_id":&f.row.id})
            .await
            .unwrap(),
        0,
        "original and derived fixtures must not create an owner action card"
    );
    for (reason, body) in unsafe_callbacks() {
        let bytes = serde_json::to_vec(&body).unwrap();
        let decision = projected(&f, &auth, &resolved, &id, &bytes, "responses", &headers).await;
        assert!(decision.admission.is_none(), "{reason}");
        let refusal = decision
            .refusal
            .expect("unsafe callback needs exact approval");
        let card = cards
            .find_one(doc! {"_id":refusal["acknowledgement_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(card.confirmation.unwrap().risk, "change", "{reason}");
    }
    for path in ["files", "responses/fixture", "external-write"] {
        let decision = projected(&f, &auth, &resolved, &id, &body, path, &headers).await;
        assert!(decision.admission.is_none());
        assert!(decision.refusal.is_some());
    }
    let mut override_headers = HeaderMap::new();
    override_headers.insert("x-http-method-override", "DELETE".parse().unwrap());
    let decision = projected(
        &f,
        &auth,
        &resolved,
        &id,
        &body,
        "responses",
        &override_headers,
    )
    .await;
    let refusal = decision.refusal.unwrap();
    let card = cards
        .find_one(doc! {"_id":refusal["acknowledgement_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(card.confirmation.unwrap().risk, "destructive");
    // The same bytes on an ordinary non-inference service require change consent.
    let ordinary = connection(&f, &f.owner, "ordinary", None).await;
    let decision = check_http_projected(
        &f.state,
        &auth,
        Some(&ordinary),
        Some(&f.owner),
        &target(&f, &ordinary).await,
        "POST",
        "responses",
        None,
        &body,
        None,
        None,
        &headers,
        &projection,
    )
    .await
    .unwrap();
    assert!(decision.refusal.is_some());
    // Changing the live connection cannot reuse the callback's held admission.
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&id}, doc! {"$inc":{"state_version":1_i64}})
        .await
        .unwrap();
    assert!(
        admission
            .revalidate(&f.state.db, &id, Some(&resolved))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn codex_callback_guest_does_not_gain_owner_model_consent() {
    use crate::services::assistant_agent_credential_service as credentials;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    let f = orchestrator_fixture("permissions_codex_guest").await;
    mode(&f, AccessMode::AskEveryTime).await;
    let c = catalog(&f).await;
    let id = connection(&f, &f.owner, "chrono-llm-public", Some(&c)).await;
    let provider = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/responses"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"output":[]})))
        .mount(&provider)
        .await;
    let row = crate::services::user_service_service::find_user_service_by_id(&f.state.db, &id)
        .await
        .unwrap()
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! {"_id":row.endpoint_id},
            doc! {"$set":{"url":provider.uri()}},
        )
        .await
        .unwrap();
    compatibility::grant(&f, &id).await;
    f.state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(doc! {"_id":&f.row.id}, doc! {"$set":{"guest_turn":true}})
        .await
        .unwrap();
    let chat = acks::for_key(&f.state.db, &f.owner, Some(&f.row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert!(chat.guest);
    assert!(
        crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .owner_model_route
            .is_none(),
        "F001 also starts without a saved conversation model route"
    );
    assert!(
        f.state
            .db
            .collection::<Document>("assistant_agents")
            .find_one(doc! {"_id":&chat.agent_id,"user_id":&f.owner})
            .await
            .unwrap()
            .unwrap()
            .get("owner_model_route")
            .is_none_or(|value| value == &bson::Bson::Null),
        "F001 exercises an unbound guest, never authorized route reuse"
    );
    let resolved = target(&f, &id).await;
    assert_eq!(row.catalog_service_id.as_deref(), Some(c.as_str()));
    let linked_catalog = f
        .state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&c})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.service.id, linked_catalog.id);
    assert_eq!(resolved.service.slug, "chrono-llm-public");
    assert_eq!(linked_catalog.slug, "autoplatform");
    assert!(
        linked_catalog.credential_encrypted.is_empty(),
        "the F001 catalog has no eligible default master credential"
    );
    assert!(resolved.service.credential_encrypted.is_empty());
    let mut auth = f.auth.clone();
    auth.assistant_chat = Some(std::sync::Arc::new(chat));
    let decision = check_http_projected(
        &f.state,
        &auth,
        Some(&id),
        Some(&f.owner),
        &resolved,
        "POST",
        "responses",
        None,
        &serde_json::to_vec(&callback()).unwrap(),
        None,
        None,
        &HeaderMap::new(),
        &HttpProjection::default(),
    )
    .await;
    assert!(matches!(decision, Err(AppError::GuestModelRouteRequired)));
    let key = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let mut metadata_free = callback();
    metadata_free
        .as_object_mut()
        .unwrap()
        .remove("client_metadata");
    let mut stateful = callback();
    stateful["store"] = json!(true);
    let mut hosted = callback();
    hosted["tools"] = json!([{"type":"code_interpreter","container":"auto"}]);
    let (_, router) = crate::routes::build_router();
    let router = router.with_state(f.state.clone());
    // First exercise the original fixture's default Use policy. Read/All must
    // not turn this owner callback into guest authority either.
    for access in [None, Some("read"), Some("all")] {
        if let Some(access) = access {
            f.state
                .db
                .collection::<Document>("assistant_agents")
                .update_one(
                    doc! {"_id":&f.chat.agent_id},
                    doc! {"$set":{format!("guest_access.{id}"):access}},
                )
                .await
                .unwrap();
        }
        for body in [
            callback(),
            metadata_free.clone(),
            stateful.clone(),
            hosted.clone(),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/proxy/s/chrono-llm-public/responses")
                        .header("content-type", "application/json")
                        .header("x-api-key", key.raw_key.as_str())
                        .body(Body::from(serde_json::to_vec(&body).unwrap()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
            assert!(
                axum::body::to_bytes(response.into_body(), 8192)
                    .await
                    .unwrap()
                    .len()
                    < 8192
            );
        }
    }
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION)
            .count_documents(doc! {"conversation_id":&f.row.id,"kind":"model_consent"},)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {"conversation_id":&f.row.id})
            .await
            .unwrap(),
        0
    );
    assert!(provider.received_requests().await.unwrap().is_empty());
    assert!(
        crate::services::assistant_nyxagent::get(&f.state.db, &f.owner, &f.row.id)
            .await
            .unwrap()
            .owner_model_route
            .is_none()
    );
    assert!(
        f.state
            .db
            .collection::<Document>("assistant_agents")
            .find_one(doc! {"_id":&f.chat.agent_id,"user_id":&f.owner})
            .await
            .unwrap()
            .unwrap()
            .get("owner_model_route")
            .is_none_or(|value| value == &bson::Bson::Null)
    );
}
