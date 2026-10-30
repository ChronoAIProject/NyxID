//! Local protocol regressions. No live callback capture or provider evidence.
use super::*;
use crate::services::{
    assistant_authority_tests::{Fixture, connected, orchestrator_fixture},
    assistant_nyxagent as engine,
};
use axum::http::HeaderMap;

mod denial;
mod recorder;
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
            "client_metadata":{"unknown-provider-option":"fixed"},"input":input,
            "tools":[{"type":"function","name":"nyx__search_tools","parameters":{"type":"object"}}]}),
        generation_proposal: None,
        preceding_receipt: None,
        tool_results: vec![],
    }
}
fn user(text: &str) -> Value {
    json!({"role":"user","content":text})
}

impl Controlled {
    async fn new(name: &str) -> Self {
        Self::configured(name, true).await
    }
    async fn configured(name: &str, issue_control: bool) -> Self {
        let mut f = orchestrator_fixture(name).await;
        f.state.config.assistant_model_control_v1_enabled = issue_control;
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
                runtime_catalog_id: Some(&catalog_id),
                outer_body: &json!({"input":"Please manage my account","store":true}),
                negotiated_v1: true,
            },
        )
        .await
        .unwrap();
        let token = if let Some(token) = token {
            assert!(issue_control);
            assert!(token.for_target("replaced-runtime-row").is_err());
            assert!(token.for_target(&catalog_id).is_ok());
            token.token
        } else {
            assert!(!issue_control);
            wire::SecretToken::parse("unissued-test-placeholder-never-forwarded".into()).unwrap()
        };
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
        self.observe_request(request(vec![user("initial")])).await
    }
    async fn observe_request(&self, candidate: wire::PrepareRequest) -> String {
        assert!(
            !super::super::assistant_service_execution::stateless_inference(
                "responses",
                &candidate.body
            )
        );
        assert!(matches!(
            prepare(
                &self.f.state,
                &self.f.chat,
                &self.token,
                request_with_body(&candidate.body)
            )
            .await
            .unwrap(),
            wire::PrepareReply::Observe
        ));
        let callback = self
            .callback("responses", &candidate.body, &self.headers(), true)
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
                runtime_catalog_id: Some(&self.target.service.id),
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

fn request_with_body(body: &Value) -> wire::PrepareRequest {
    let mut candidate = request(vec![]);
    candidate.body = body.clone();
    candidate
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
    let (_, private, _) = persistence::authenticated(&c.f.state, &c.f.chat, &c.token, None)
        .await
        .unwrap();
    assert!(private.preconsent.ineligible);
    c.f.state.db.drop().await.unwrap();
    // Activation negatives use a pristine distinct turn, rather than erasing
    // the durable ineligibility deliberately established above.
    let mut c = Controlled::new("p2_exact_only_active_callback").await;
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
fn reusable_responses_configuration_rejects_unknown_executable_and_unbounded_metadata() {
    for (key, value) in [
        ("messages", json!([])),
        ("system", json!("authority")),
        ("thinking", json!({})),
        ("n", json!(2)),
        ("dimensions", json!(3)),
        ("client_metadata", json!("scalar")),
        ("client_metadata", json!({"opaque":{"nested":"value"}})),
        ("client_metadata", json!({"thread_id":"x".repeat(32769)})),
    ] {
        let mut candidate = request(vec![user("initial")]);
        candidate.body[key] = value;
        assert!(
            envelope::envelope(&candidate).is_err(),
            "unsupported configuration: {key}"
        );
    }
}

#[test]
fn explicit_write_keeps_unknown_and_large_metadata_exact_without_automatic_read() {
    for metadata in [
        json!({"unknown-provider-option":"opaque-value"}),
        json!({"x-codex-turn-metadata":"x".repeat(8193)}),
    ] {
        let mut candidate = request(vec![user("initial")]);
        candidate.body["client_metadata"] = metadata.clone();
        assert!(
            !super::super::assistant_service_execution::stateless_inference(
                "responses",
                &candidate.body
            )
        );
        let (fixed, _) = envelope::envelope(&candidate).unwrap();
        assert_eq!(fixed.fixed["client_metadata"], metadata);
        candidate.body["input"] = json!([user("changed transcript")]);
        assert!(envelope::envelope(&candidate).unwrap().0 == fixed);
        for changed in [
            json!({"changed-key":"opaque-value"}),
            json!({"unknown-provider-option":"changed-value"}),
            json!({"x-codex-turn-metadata":"x".repeat(8194)}),
        ] {
            candidate.body["client_metadata"] = changed;
            assert!(envelope::envelope(&candidate).unwrap().0 != fixed);
        }
    }
}

#[test]
fn original_and_native_schema_derived_captures_fit_positive_explicit_write_schema() {
    // Four original local synthetic calls plus four native-schema-derived
    // variants. Neither file is a new capture or provider-success evidence.
    for fixture in [
        include_str!("../../../../docs/fixtures/codex-public-responses-0.160.1.json"),
        include_str!(
            "../../../../docs/fixtures/codex-public-responses-0.160.1-native-schema-derived.json"
        ),
    ] {
        let fixture: Value = serde_json::from_str(fixture).unwrap();
        assert_eq!(fixture["calls"].as_array().unwrap().len(), 4);
        for call in fixture["calls"].as_array().unwrap() {
            let candidate = request_with_body(&call["codex"]);
            let (fixed, input) = envelope::envelope(&candidate).unwrap();
            assert_eq!(
                fixed.fixed["client_metadata"],
                candidate.body["client_metadata"]
            );
            assert_eq!(
                fixed.fixed["prompt_cache_key"],
                candidate.body["prompt_cache_key"]
            );
            assert_eq!(input, candidate.body["input"].as_array().unwrap().clone());
            for metadata in [
                json!({"unknown-provider-option":"opaque-value"}),
                json!({"x-codex-turn-metadata":"x".repeat(8193)}),
            ] {
                let mut derived = request_with_body(&candidate.body);
                derived.body["client_metadata"] = metadata.clone();
                assert_eq!(
                    envelope::envelope(&derived).unwrap().0.fixed["client_metadata"],
                    metadata
                );
                assert!(
                    !super::super::assistant_service_execution::stateless_inference(
                        "responses",
                        &derived.body
                    )
                );
            }
        }
    }
}

#[test]
fn capture_derived_reset_keeps_environment_wrapper_and_exact_saved_user_text() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../docs/fixtures/codex-public-responses-0.160.1.json"
    ))
    .unwrap();
    let initial = fixture["calls"][0]["codex"]["input"].as_array().unwrap();
    assert_eq!(initial.len(), 3);
    assert_eq!(initial[0]["role"], "developer");
    assert!(
        initial[1]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("<environment_context>")
    );
    let mut resumed = initial[..2].to_vec();
    resumed.push(json!({"id":null,"type":"message","role":"user","content":[{"type":"input_text","text":"exact saved Allow narration"}]}));
    assert!(envelope::allowed_resume(initial, &resumed, "exact saved Allow narration").is_ok());
    for change in ["wrapper", "insert", "reorder", "extra_user", "saved_text"] {
        let mut changed = resumed.clone();
        match change {
            "wrapper" => changed[1]["content"][0]["text"] = json!("regenerated environment"),
            "insert" => changed.insert(0, json!({"role":"developer","content":"extra authority"})),
            "reorder" => changed.swap(0, 1),
            "extra_user" => changed.push(user("extra input")),
            _ => changed[2]["content"][0]["text"] = json!("changed saved narration"),
        }
        assert!(
            envelope::allowed_resume(initial, &changed, "exact saved Allow narration").is_err(),
            "{change}"
        );
    }
}

#[test]
fn capture_derived_next_input_uses_complete_serialization_and_bound_host_result() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../docs/fixtures/codex-public-responses-0.160.1.json"
    ))
    .unwrap();
    let previous = fixture["calls"][0]["codex"]["input"].as_array().unwrap();
    let native = fixture["calls"][1]["codex"]["input"].as_array().unwrap();
    let mut output = native[previous.len()..native.len() - 1].to_vec();
    for item in &mut output {
        item["status"] = json!("completed");
        item.as_object_mut().unwrap().remove("content");
    }
    // Synthetic provider-origin completion reconstructed from unchanged
    // captured input shapes. It is not an additional raw provider capture.
    let observed = envelope::verify_completion(
        &json!({"id":"derived-response","object":"response","status":"completed","output":output}),
        &fixture["calls"][0]["codex"],
    )
    .unwrap();
    let completion = ObservedCompletion {
        response_id: observed.response_id,
        output: observed.output,
        calls: observed.calls,
        compaction: None,
    };
    let raw_binding = digest(json!(completion.output));
    let call = &completion.calls[0];
    let result_text = native.last().unwrap()["output"].as_str().unwrap();
    let results = vec![VerifiedToolResult {
        call_id: call.call_id.clone(),
        tool_name: call.name.clone(),
        arguments_binding: digest(json!(call.arguments)),
        model_text_binding: digest(json!(result_text)),
    }];
    let mut next = native.clone();
    next.last_mut().unwrap()["id"] = Value::Null;
    assert!(envelope::next_input(previous, &completion, &next, &results).is_ok());
    for change in [
        "call_id",
        "arguments",
        "result",
        "ciphertext",
        "reorder",
        "mixed_status",
        "result_id",
    ] {
        let mut changed = next.clone();
        match change {
            "call_id" => changed[4]["call_id"] = json!("different-call"),
            "arguments" => changed[4]["arguments"] = json!("{}"),
            "result" => changed[5]["output"] = json!("forged result"),
            "ciphertext" => changed[3]["encrypted_content"] = json!("changed opaque ciphertext"),
            "reorder" => changed.swap(3, 4),
            "mixed_status" => changed[3]["status"] = json!("completed"),
            _ => changed[5]["id"] = json!("unproven-native-result-id"),
        }
        assert!(
            envelope::next_input(previous, &completion, &changed, &results).is_err(),
            "{change}"
        );
    }
    assert_eq!(
        raw_binding,
        digest(json!(completion.output)),
        "next-input projection cannot change receipt output"
    );
    let mut message = fixture["compact"]["codex"]["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["role"] == "assistant")
        .unwrap()
        .clone();
    message["status"] = json!("completed");
    let verified = envelope::verify_completion(&json!({"id":"derived-message","object":"response","status":"completed","output":[message]}), &fixture["calls"][0]["codex"]).unwrap();
    let message_completion = ObservedCompletion {
        response_id: verified.response_id,
        output: verified.output,
        calls: verified.calls,
        compaction: None,
    };
    let mut projected = message_completion.output.clone();
    projected[0].as_object_mut().unwrap().remove("status");
    assert!(envelope::next_input(&[], &message_completion, &projected, &[]).is_ok());
    projected[0]["phase"] = json!("commentary");
    assert!(envelope::next_input(&[], &message_completion, &projected, &[]).is_err());
}

#[tokio::test]
async fn reusable_input_rejects_resource_and_unknown_nested_shapes_before_and_after_activation() {
    let c = Controlled::new("p2_closed_input_schema").await;
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
    let (_, private, _) = persistence::authenticated(&c.f.state, &c.f.chat, &c.token, None)
        .await
        .unwrap();
    assert!(private.preconsent.ineligible);
    c.f.state.db.drop().await.unwrap();
    // Unsupported actual callbacks permanently retire that preconsent lineage.
    // Exercise the active-root negatives on a separately admitted clean turn.
    let mut c = Controlled::new("p2_closed_input_schema_active").await;
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
    let projection = crate::services::assistant_service_execution::HttpProjection {
        model_control_ineligible: kind == "node",
        ..Default::default()
    };
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
        let (_, private, _) = persistence::authenticated(&c.f.state, &c.f.chat, &c.token, None)
            .await
            .unwrap();
        assert!(private.preconsent.ineligible, "{kind}");
        c.f.state.db.drop().await.unwrap();
        let c = Controlled::new("p2_ineligible_route_active").await;
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
            post(crate::handlers::assistant_model_control::status)
                .put(|_: crate::mw::auth::AuthUser| async { StatusCode::OK }),
        )
        .route(
            "/assistant/model-control/v1/status",
            post(crate::handlers::assistant_model_control::status),
        )
        .route(
            "/api/v1/assistant/settings",
            get(|_: crate::mw::auth::AuthUser| async { StatusCode::OK }),
        )
        .with_state(c.f.state.clone());
    for variant in [
        "valid",
        "nested",
        "raw_key_only",
        "capability_only",
        "wrong_capability",
        "duplicate_control",
        "get",
        "management",
        "put",
        "permission_bound",
        "scheduled_invocation",
        "public_nyxbot",
        "shared_entry_transport",
    ] {
        c.f.state.db.collection::<Document>("api_keys").update_one(
            doc! {"_id":&c.f.chat.api_key_id},
            doc! {"$set":{"purpose": if matches!(variant,"permission_bound"|"scheduled_invocation"|"public_nyxbot"|"shared_entry_transport") {variant} else {"general"}}},
        ).await.unwrap();
        let path = if variant == "management" {
            "/api/v1/assistant/settings"
        } else if variant == "nested" {
            "/assistant/model-control/v1/status"
        } else {
            "/api/v1/assistant/model-control/v1/status"
        };
        let mut request = Request::builder()
            .method(if matches!(variant, "get" | "management") {
                "GET"
            } else if variant == "put" {
                "PUT"
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
        if matches!(variant, "valid" | "nested") {
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
            if matches!(
                variant,
                "management"
                    | "put"
                    | "permission_bound"
                    | "scheduled_invocation"
                    | "public_nyxbot"
                    | "shared_entry_transport"
            ) {
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

#[tokio::test]
async fn retired_wire_root_cannot_migrate_allow_or_dispatch_but_saved_deny_retires_it() {
    let c = Controlled::new("p2_retired_wire").await;
    let card = c.observe().await;
    assert!(
        issue(
            &c.f.state,
            IssueRequest {
                chat: &c.f.chat,
                runtime_catalog_id: Some(&c.target.service.id),
                outer_body: &json!({"input":"initial","store":true}),
                negotiated_v1: false
            }
        )
        .await
        .is_err()
    );
    let roots = c.f.state.db.collection::<ModelControlRoot>(COLLECTION_NAME);
    let mut row = roots
        .find_one(doc! {"acknowledgement_id":&card})
        .await
        .unwrap()
        .unwrap();
    let mut private = persistence::open(&c.f.state, &row).await.unwrap();
    // Storage compatibility fixture, not a live old-peer/protocol acceptance.
    for control in &mut private.controls {
        control.claims.protocol = wire::RETIRED_PROTOCOL.into();
    }
    let mut arguments = row.card_arguments.clone().unwrap();
    arguments["protocol"] = json!(wire::RETIRED_PROTOCOL);
    row.card_arguments = Some(arguments.clone());
    private.card_arguments = Some(arguments.clone());
    row.private_ciphertext = persistence::seal(&c.f.state, &row, &private).await.unwrap();
    roots.replace_one(doc! {"_id":&row.id}, &row).await.unwrap();
    c.f.state
        .db
        .collection::<Document>("assistant_acknowledgements")
        .update_one(
            doc! {"_id":&card},
            doc! {"$set":{"arguments_digest":acks::arguments_digest(&arguments)}},
        )
        .await
        .unwrap();
    assert!(
        issue(
            &c.f.state,
            IssueRequest {
                chat: &c.f.chat,
                runtime_catalog_id: Some(&c.target.service.id),
                outer_body: &json!({"input":"initial","store":true}),
                negotiated_v1: true
            }
        )
        .await
        .is_err()
    );
    assert!(
        acks::decide(&c.f.state.db, &c.f.owner, &c.f.row.id, &card, true)
            .await
            .is_err()
    );
    acks::decide(&c.f.state.db, &c.f.owner, &c.f.row.id, &card, false)
        .await
        .unwrap();
    let retained = roots.find_one(doc! {"_id":&row.id}).await.unwrap().unwrap();
    assert_eq!(retained.phase, "denied");
    assert_eq!(retained.control_hashes, row.control_hashes);
    assert!(
        prepare(
            &c.f.state,
            &c.f.chat,
            &c.token,
            request(vec![user("initial")])
        )
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
