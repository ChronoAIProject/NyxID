//! Local protocol regressions. No live callback capture or provider evidence.
use super::*;
use crate::services::{
    assistant_authority_tests::{Fixture, connected, orchestrator_fixture},
    assistant_nyxagent as engine,
};
use axum::http::HeaderMap;

mod denial;
mod transport;

struct Controlled {
    f: Fixture,
    connection: String,
    target: super::super::proxy_service::ProxyTarget,
    token: wire::SecretToken,
}

fn request(input: Vec<Value>) -> wire::PrepareRequest {
    wire::PrepareRequest {
        protocol: wire::PROTOCOL.into(),
        relative_path: "responses".into(),
        query: None,
        semantic_headers: vec![
            ("accept".into(), "*/*".into()),
            ("content-type".into(), "application/json".into()),
        ],
        body: json!({"model":"model-v1","store":false,"stream":true,
            "client_metadata":{"thread_id":"fixed"},"input":input,
            "tools":[{"type":"function","name":"nyx__search_tools","parameters":{"type":"object"}}]}),
        preceding_receipt: None,
        tool_results: vec![],
    }
}
fn user(text: &str) -> Value {
    json!({"role":"user","content":text})
}

impl Controlled {
    async fn new(name: &str) -> Self {
        let mut f = orchestrator_fixture(name).await;
        f.state.config.assistant_model_control_v1_enabled = true;
        super::super::assistant_settings_service::update(
            &f.state.db,
            &f.owner,
            super::super::assistant_settings_service::Update {
                access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.inference = Some(crate::models::downstream_service::ServiceInference {
            wire_protocol:
                crate::models::downstream_service::InferenceWireProtocol::OpenaiCompletions,
            model_list: true,
            realtime: false,
            voice: None,
        });
        let catalog_id = catalog.id.clone();
        f.state
            .db
            .collection("downstream_services")
            .insert_one(catalog)
            .await
            .unwrap();
        let connection = connected(&f.state.db, &f.owner, "p2-model", "https://example.com").await;
        f.state
            .db
            .collection::<Document>("user_services")
            .update_one(
                doc! {"_id":&connection},
                doc! {"$set":{"catalog_service_id":&catalog_id}},
            )
            .await
            .unwrap();
        let target = super::super::proxy_service::resolve_proxy_target_by_user_service_id(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            &connection,
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
        .target;
        let token = issue(
            &f.state,
            IssueRequest {
                chat: &f.chat,
                runtime_catalog_id: &catalog_id,
                outer_body: &json!({"input":"Please manage my account","store":true}),
                negotiated_v1: true,
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert!(token.for_target("replaced-runtime-row").is_err());
        assert!(token.for_target(&catalog_id).is_ok());
        let token = token.token;
        Self {
            f,
            connection,
            target,
            token,
        }
    }
    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(wire::CONTROL_HEADER, self.token.expose().parse().unwrap());
        headers.insert("accept", "*/*".parse().unwrap());
        headers.insert("content-type", "application/json".parse().unwrap());
        headers
    }
    async fn callback(
        &self,
        path: &str,
        body: &Value,
        headers: &HeaderMap,
        instrumented_transport: bool,
    ) -> AppResult<Option<Callback>> {
        callback(&self.f.state,CallbackRequest {chat:&self.f.chat,connection:&self.connection,
            owner:&self.f.owner,target:&self.target,method:"POST",path,query:None,
            body:&serde_json::to_vec(body).unwrap(),headers,
            semantic_headers:super::super::proxy_service::collect_forward_headers_with_defaults(headers,&[]),
            instrumented_transport,
        }).await
    }
    async fn observe(&self) -> String {
        let candidate = request(vec![user("initial")]);
        assert!(
            !super::super::assistant_service_execution::stateless_inference(
                "responses",
                &candidate.body
            )
        );
        assert!(
            super::super::assistant_service_execution::stateless_model_transport(
                "responses",
                &candidate.body
            )
        );
        assert!(matches!(
            prepare(&self.f.state, &self.f.chat, &self.token, candidate)
                .await
                .unwrap(),
            wire::PrepareReply::Observe
        ));
        let callback = self
            .callback(
                "responses",
                &request(vec![user("initial")]).body,
                &self.headers(),
                true,
            )
            .await
            .unwrap()
            .unwrap();
        let refusal = observe(&self.f.state, &self.f.chat, &callback)
            .await
            .unwrap();
        refusal["acknowledgement_id"].as_str().unwrap().into()
    }
    async fn allow_resume(&mut self) -> String {
        let card = self.observe().await;
        self.resume_card(card).await
    }
    async fn resume_card(&mut self, card: String) -> String {
        let (a, b) = tokio::join!(
            acks::decide(&self.f.state.db, &self.f.owner, &self.f.row.id, &card, true),
            acks::decide(&self.f.state.db, &self.f.owner, &self.f.row.id, &card, true),
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert!(
            acks::decide(&self.f.state.db, &self.f.owner, &self.f.row.id, &card, true)
                .await
                .is_err()
        );
        engine::finish_turn(
            &self.f.state.db,
            &self.f.row,
            &self.f.row.credential_api_key_id,
            &uuid::Uuid::new_v4().to_string(),
            &engine::TurnResult {
                text: "Waiting for decision".into(),
                session_id: None,
                response_id: None,
                error: None,
            },
        )
        .await
        .unwrap();
        self.f.row = engine::begin_turn(
            &self.f.state.db,
            &self.f.owner,
            &engine::TurnRequest {
                acknowledgement_id: Some(card.clone()),
                retry_of: None,
                attachment_ids: vec![],
                agent_id: None,
                conversation_id: Some(self.f.row.id.clone()),
                text: "Saved Allow narration".into(),
                model: None,
                access_mode: None,
            },
            &self.f.state.encryption_keys,
        )
        .await
        .unwrap();
        self.f.chat = acks::for_key(
            &self.f.state.db,
            &self.f.owner,
            Some(&self.f.row.credential_api_key_id),
        )
        .await
        .unwrap()
        .unwrap();
        self.token = issue(
            &self.f.state,
            IssueRequest {
                chat: &self.f.chat,
                runtime_catalog_id: &self.target.service.id,
                outer_body: &json!({"input":"Saved Allow narration","store":true}),
                negotiated_v1: true,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .token;
        card
    }
}

#[tokio::test]
async fn negotiated_unactivated_compact_and_translated_callbacks_keep_exact_only() {
    let mut c = Controlled::new("p2_exact_only_callback").await;
    let body = json!({"model":"model-v1","input":[user("compact")],"store":false});
    assert!(matches!(
        prepare(
            &c.f.state,
            &c.f.chat,
            &c.token,
            wire::PrepareRequest {
                relative_path: "responses/compact".into(),
                ..request(vec![user("compact")])
            }
        )
        .await
        .unwrap(),
        wire::PrepareReply::ExactOnly { .. }
    ));
    assert!(
        c.callback("responses/compact", &body, &c.headers(), true)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        c.callback(
            "responses",
            &request(vec![user("initial")]).body,
            &c.headers(),
            false
        )
        .await
        .unwrap()
        .is_none()
    );
    c.target.auth_method = "aws_sigv4".into();
    assert!(
        c.callback("responses", &body, &c.headers(), true)
            .await
            .unwrap()
            .is_none()
    );
    c.target.auth_method = "none".into();
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let mut step = c.headers();
    step.insert(wire::STEP_HEADER, "unadmitted-step".parse().unwrap());
    assert!(
        c.callback("responses/compact", &body, &step, true)
            .await
            .is_err()
    );
    let mut duplicate = c.headers();
    duplicate.append(wire::CONTROL_HEADER, c.token.expose().parse().unwrap());
    assert!(
        c.callback("responses", &body, &duplicate, true)
            .await
            .is_err()
    );
    c.observe().await;
    assert!(
        c.callback("responses/compact", &body, &c.headers(), true)
            .await
            .is_err()
    );
    assert!(
        c.callback("responses", &body, &c.headers(), false)
            .await
            .is_err()
    );
    c.target.auth_method = "aws_sigv4".into();
    assert!(
        c.callback("responses", &body, &c.headers(), true)
            .await
            .is_err()
    );
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    c.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn saved_allow_changed_transcript_recovers_same_prepared_step_and_claims_once() {
    let mut c = Controlled::new("p2_allow_claim_once").await;
    let card = c.allow_resume().await;
    let step = match prepare(
        &c.f.state,
        &c.f.chat,
        &c.token,
        request(vec![user("Saved Allow narration")]),
    )
    .await
    .unwrap()
    {
        wire::PrepareReply::Prepared { step } => step,
        _ => panic!("expected real saved Allow preparation"),
    };
    // A lost prepare response retrieves the exact encrypted token, not a new
    // ordinal, capability, expiry or logical root.
    let recovered = match status(
        &c.f.state,
        &c.f.chat,
        &c.token,
        wire::StatusRequest {
            protocol: wire::PROTOCOL.into(),
            root_id: step.root_id.clone(),
            step_id: None,
            request_binding: None,
        },
    )
    .await
    .unwrap()
    {
        wire::PrepareReply::Prepared { step } => step,
        _ => panic!("expected same prepared step"),
    };
    assert_eq!(step.step_id, recovered.step_id);
    assert_eq!(step.ordinal, 0);
    assert_eq!(step.expires_at, recovered.expires_at);
    assert!(step.step_bearer.expose() == recovered.step_bearer.expose());
    assert!(
        prepare(
            &c.f.state,
            &c.f.chat,
            &c.token,
            request(vec![user("changed after final preparation")])
        )
        .await
        .is_err()
    );
    let mut headers = c.headers();
    headers.insert(
        wire::STEP_HEADER,
        step.step_bearer.expose().parse().unwrap(),
    );
    let cb = c
        .callback(
            "responses",
            &request(vec![user("Saved Allow narration")]).body,
            &headers,
            true,
        )
        .await
        .unwrap()
        .unwrap();
    let prepared = cb.prepared().unwrap();
    let (a, b) = tokio::join!(prepared.claim(), prepared.claim());
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(
        status(
            &c.f.state,
            &c.f.chat,
            &c.token,
            wire::StatusRequest {
                protocol: wire::PROTOCOL.into(),
                root_id: step.root_id.clone(),
                step_id: Some(step.step_id.clone()),
                request_binding: Some(step.request_binding.clone()),
            }
        )
        .await
        .unwrap(),
        wire::PrepareReply::InProgress { .. }
    ));
    assert!(prepared.claim().await.is_err());
    let saved =
        c.f.state
            .db
            .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
                "assistant_acknowledgements",
            )
            .find_one(doc! {"_id":card})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(saved.status, "used");
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    c.f.state.db.drop().await.unwrap();
}

#[test]
fn reset_transcript_preserves_exact_privileged_scaffold_and_saved_user_input() {
    let initial = vec![
        json!({"role":"system","content":"system"}),
        json!({"role":"developer","content":"developer"}),
        user("initial"),
    ];
    let resumed = vec![
        initial[0].clone(),
        initial[1].clone(),
        user("saved narration"),
    ];
    envelope::allowed_resume(&initial, &resumed, "saved narration").unwrap();
    for changed in [
        vec![
            initial[1].clone(),
            initial[0].clone(),
            user("saved narration"),
        ],
        vec![
            initial[0].clone(),
            json!({"role":"developer","content":"inserted"}),
            initial[1].clone(),
            user("saved narration"),
        ],
        vec![
            initial[0].clone(),
            initial[1].clone(),
            user("saved narration"),
            user("extra"),
        ],
        vec![
            initial[0].clone(),
            initial[1].clone(),
            user("changed narration"),
        ],
    ] {
        assert!(envelope::allowed_resume(&initial, &changed, "saved narration").is_err());
    }
}

#[test]
fn reusable_responses_configuration_rejects_unknown_cross_protocol_and_opaque_metadata() {
    for (key, value) in [
        ("messages", json!([])),
        ("system", json!("authority")),
        ("thinking", json!({})),
        ("n", json!(2)),
        ("dimensions", json!(3)),
        ("client_metadata", json!({"opaque":"value"})),
        ("client_metadata", json!("scalar")),
        ("client_metadata", json!({"thread_id":"x".repeat(2049)})),
    ] {
        let mut candidate = request(vec![user("initial")]);
        candidate.body[key] = value;
        assert!(
            envelope::envelope(&candidate).is_err(),
            "unsupported configuration: {key}"
        );
    }
}

#[tokio::test]
async fn reusable_input_rejects_resource_and_unknown_nested_shapes_before_and_after_activation() {
    let mut c = Controlled::new("p2_closed_input_schema").await;
    let unsupported = vec![
        json!({"role":"user","content":[{"type":"input_image","image_url":"https://example.invalid/image"}]}),
        json!({"role":"user","content":[{"type":"input_file","file_url":"https://example.invalid/file"}]}),
        json!({"role":"user","content":[{"type":"input_text","text":"text","execute":{"url":"https://example.invalid"}}]}),
        json!({"role":"user","content":"text","unknown":{"background":true}}),
        json!({"type":"reasoning","summary":[{"type":"summary_text","text":"text","execute":true}]}),
    ];
    for item in &unsupported {
        let candidate = request(vec![item.clone()]);
        assert!(envelope::envelope(&candidate).is_err());
        assert!(matches!(
            prepare(&c.f.state, &c.f.chat, &c.token, request(vec![item.clone()]))
                .await
                .unwrap(),
            wire::PrepareReply::ExactOnly { .. }
        ));
        assert!(
            c.callback("responses", &candidate.body, &c.headers(), true)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    c.allow_resume().await;
    for item in unsupported {
        let candidate = request(vec![item.clone()]);
        assert!(
            prepare(&c.f.state, &c.f.chat, &c.token, request(vec![item.clone()]))
                .await
                .is_err()
        );
        assert!(
            c.callback("responses", &candidate.body, &c.headers(), true)
                .await
                .is_err()
        );
    }
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    c.f.state.db.drop().await.unwrap();
}

async fn ineligible_route(
    c: &Controlled,
    kind: &str,
    controlled: bool,
) -> AppResult<crate::services::assistant_service_execution::Decision> {
    let mut auth = c.f.auth.clone();
    auth.assistant_chat = Some(std::sync::Arc::new(c.f.chat.clone()));
    let mut headers = if controlled {
        c.headers()
    } else {
        HeaderMap::new()
    };
    headers.insert("content-type", "application/json".parse().unwrap());
    headers.insert("accept", "*/*".parse().unwrap());
    if kind == "override" {
        headers.insert("x-http-method-override", "DELETE".parse().unwrap());
    }
    let mut projection = crate::services::assistant_service_execution::HttpProjection::default();
    projection.model_control_ineligible = kind == "node";
    let mut body = request(vec![user("initial")]).body;
    if kind == "catalog" {
        body.as_object_mut().unwrap().remove("client_metadata");
    }
    Box::pin(
        crate::services::assistant_service_execution::check_http_projected(
            &c.f.state,
            &auth,
            (kind != "catalog").then_some(c.connection.as_str()),
            Some(&c.f.owner),
            &c.target,
            "POST",
            "responses",
            None,
            &serde_json::to_vec(&body).unwrap(),
            None,
            (kind == "pool").then_some("test-pool"),
            &headers,
            &projection,
        ),
    )
    .await
}

#[tokio::test]
async fn negotiated_ineligible_routes_preserve_baseline_and_active_lineage_cannot_downgrade() {
    for kind in ["catalog", "pool", "override", "node"] {
        let c = Controlled::new("p2_ineligible_route_baseline").await;
        if kind == "pool" {
            c.f.state.db.collection::<Document>("service_pools").insert_one(doc!{
                "_id":"test-pool","user_id":&c.f.owner,"slug":"test-pool","name":"Test pool","is_active":true,
                "members":[{"user_service_id":&c.connection}],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            }).await.unwrap();
        }
        let baseline = ineligible_route(&c, kind, false).await.unwrap();
        let negotiated = ineligible_route(&c, kind, true).await.unwrap();
        assert_eq!(baseline.refusal, negotiated.refusal, "{kind}");
        assert_eq!(
            baseline.admission.is_some(),
            negotiated.admission.is_some(),
            "{kind}"
        );
        assert!(
            negotiated
                .admission
                .as_ref()
                .is_none_or(|a| a.model_control.is_none())
        );
        assert_eq!(
            c.f.state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .count_documents(doc! {"tool_name":"nyxid.model_control_v1"})
                .await
                .unwrap(),
            0
        );
        if kind == "catalog" {
            assert!(baseline.refusal.is_none());
            assert_eq!(
                c.f.state
                    .db
                    .collection::<Document>("assistant_acknowledgements")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
        c.observe().await;
        assert!(
            ineligible_route(&c, kind, true).await.is_err(),
            "activated {kind}"
        );
        assert!(
            ineligible_route(&c, kind, false).await.is_err(),
            "missing active control {kind}"
        );
        c.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn private_status_uses_real_conversation_key_extractor_and_requires_separate_capability_post()
{
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        routing::{get, post},
    };
    use tower::ServiceExt;
    let c = Controlled::new("p2_control_endpoint_auth").await;
    let credential = crate::services::assistant_agent_credential_service::load_for_conversation(
        &c.f.state.db,
        &c.f.state.encryption_keys,
        &c.f.owner,
        &c.f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let root =
        c.f.state
            .db
            .collection::<ModelControlRoot>(COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    let app = Router::new()
        .route(
            "/api/v1/assistant/model-control/v1/status",
            post(crate::handlers::assistant_model_control::status),
        )
        .route(
            "/api/v1/assistant/settings",
            get(|_: crate::mw::auth::AuthUser| async { StatusCode::OK }),
        )
        .with_state(c.f.state.clone());
    for variant in [
        "valid",
        "raw_key_only",
        "capability_only",
        "wrong_capability",
        "duplicate_control",
        "get",
        "management",
    ] {
        let path = if variant == "management" {
            "/api/v1/assistant/settings"
        } else {
            "/api/v1/assistant/model-control/v1/status"
        };
        let mut request = Request::builder()
            .method(if matches!(variant, "get" | "management") {
                "GET"
            } else {
                "POST"
            })
            .uri(path)
            .header("content-type", "application/json");
        if variant != "capability_only" {
            request = request.header("x-api-key", credential.raw_key.as_str());
        }
        if variant != "raw_key_only" {
            request = request.header(
                wire::CONTROL_HEADER,
                if variant == "wrong_capability" {
                    "invalid-control"
                } else {
                    c.token.expose()
                },
            );
        }
        if variant == "duplicate_control" {
            request = request.header(wire::CONTROL_HEADER, c.token.expose());
        }
        let response = app
            .clone()
            .oneshot(
                request
                    .body(Body::from(
                        json!({"protocol":wire::PROTOCOL,
            "root_id":root.id,"step_id":null,"request_binding":null})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        if variant == "valid" {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap())
                    .unwrap();
            assert_eq!(body["state"], "observe");
        } else {
            assert!(
                !response.status().is_success(),
                "execution/secret endpoint must reject {variant}"
            );
            if variant == "get" {
                assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
            }
            if variant == "management" {
                assert_eq!(response.status(), StatusCode::FORBIDDEN);
            }
        }
    }
    assert_eq!(
        c.f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    c.f.state.db.drop().await.unwrap();
}
