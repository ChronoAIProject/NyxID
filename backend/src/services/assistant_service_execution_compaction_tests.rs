//! Model transport scope is independent of exact account/action consent.
use super::compatibility::{flag, grant, idle};
use super::*;
use crate::services::{
    assistant_agent_credential_service as credentials, assistant_nyxagent as engine,
    platform_manager_contract as contract,
};
use axum::{body::Body, http::Request, response::Response};
use tower::ServiceExt;

pub(super) async fn compact_connection(
    f: &Fixture,
    owner: &str,
    catalog: &str,
    slug: &str,
    url: &str,
) -> String {
    let id = connected(&f.state.db, owner, slug, url).await;
    let key = Uuid::new_v4().to_string();
    let encrypted = f
        .state
        .encryption_keys
        .encrypt(Uuid::new_v4().to_string().as_bytes())
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_api_keys")
        .insert_one(doc! {
            "_id":&key,"user_id":owner,"label":slug,"credential_type":"api_key",
            "status":"active","credential_encrypted":bson::Binary{
                subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        })
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"catalog_service_id":catalog,"api_key_id":key,
            "auth_method":"bearer","auth_key_name":"Authorization"}},
        )
        .await
        .unwrap();
    id
}

async fn model_call(
    f: &Fixture,
    slug: &str,
    path: &str,
    body: &Value,
    acknowledgement: Option<&str>,
) -> (u16, Value) {
    let key = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/proxy/s/{slug}/v1/{path}"))
        .header("content-type", "application/json")
        .header("x-api-key", key.raw_key.as_str());
    if let Some(id) = acknowledgement {
        request = request.header(ACK_HEADER, id);
    }
    let (_, router) = crate::routes::build_router();
    // Match the server's request task without retaining the fixture's polling
    // frame. Abandoning this helper still cancels the request.
    let response = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(
        router
            .with_state(f.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap()),
    ))
    .await
    .unwrap()
    .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn exact_model_requests() -> Vec<(&'static str, Value)> {
    vec![
        (
            "responses/compact",
            json!({"model":"m","input":"account context"}),
        ),
        ("responses", json!({"model":"m","input":"account context"})),
        (
            "responses",
            json!({"model":"m","store":true,"input":"account context"}),
        ),
        (
            "responses",
            json!({"model":"m","store":false,"previous_response_id":"resp_account_a","input":"Continue"}),
        ),
        (
            "responses",
            json!({"model":"m","store":false,"input":[{"type":"input_file","file_id":"file_account_a"}]}),
        ),
        (
            "chat/completions",
            json!({"model":"m","messages":[{"role":"user","content":"account context"}],"provider_extension":{"account":"account_a"}}),
        ),
    ]
}

#[tokio::test]
async fn personal_stateful_model_transport_preserves_live_and_idle_scope_without_account_fallback()
{
    for (path, body) in exact_model_requests() {
        for live in [true, false] {
            let mut f = orchestrator_fixture("compact_model_scope").await;
            flag(&f, false).await;
            mode(&f, AccessMode::Nyxmode).await;
            let catalog = catalog(&f).await;
            let a = wiremock::MockServer::start().await;
            let b = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path(format!("/v1/{path}")))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                    json!({"id":"cmp_test","object":"response.compaction","output":[]}),
                ))
                .mount(&a)
                .await;
            let first = compact_connection(&f, &f.owner, &catalog, "compact-a", &a.uri()).await;
            compact_connection(&f, &f.owner, &catalog, "compact-b", &b.uri()).await;
            // Model transport remains reachable without adding it to tool grants.
            grant(&f, &Uuid::new_v4().to_string()).await;
            f.state
                .db
                .collection::<Document>("service_preferences")
                .insert_one(doc! {
                    "_id":&f.owner,"ordered":[&first],"created_at":bson::DateTime::now(),
                    "updated_at":bson::DateTime::now(),
                })
                .await
                .unwrap();
            if live {
                choose(&f, &first).await;
            } else {
                idle(&mut f).await;
            }
            assert!(!stateless_inference(path, &body));
            let (status, result) = model_call(&f, "compact-a", path, &body, None).await;
            assert_eq!(status, 200, "live={live}: {result}");
            let (status, result) = model_call(&f, "compact-b", path, &body, None).await;
            assert_eq!(status, 409, "live={live}: {result}");
            assert_eq!(
                result["error"],
                if live {
                    "service_selection_required"
                } else {
                    "service_target_selection_required"
                },
                "{result}",
            );
            assert_eq!(a.received_requests().await.unwrap().len(), 1);
            assert!(b.received_requests().await.unwrap().is_empty());
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>(COLLECTION)
                    .count_documents(doc! {"kind":{"$in":["model_consent","unavailable_bucket"]}})
                    .await
                    .unwrap(),
                0
            );
        }
    }
}

#[tokio::test]
async fn stateful_model_approval_keeps_exact_account_and_semantic_body() {
    for (path, body) in exact_model_requests() {
        let f = orchestrator_fixture("compact_exact_consent").await;
        flag(&f, false).await;
        mode(&f, AccessMode::AskEveryTime).await;
        let catalog = catalog(&f).await;
        let a = wiremock::MockServer::start().await;
        let b = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"output":[]})))
            .mount(&a)
            .await;
        let first = compact_connection(&f, &f.owner, &catalog, "compact-a", &a.uri()).await;
        compact_connection(&f, &f.owner, &catalog, "compact-b", &b.uri()).await;
        grant(&f, &Uuid::new_v4().to_string()).await;
        choose(&f, &first).await;
        let (status, refusal) = model_call(&f, "compact-a", path, &body, None).await;
        assert_eq!(status, 409, "{refusal}");
        assert_eq!(refusal["error"], "acknowledgement_required", "{refusal}");
        let acknowledgement = allow(&f, refusal).await;
        let mut changed_body = body.clone();
        if path == "chat/completions" {
            changed_body["messages"] = json!([{"role":"user","content":"different context"}]);
        } else {
            // Preserve any account handle: removing it can independently admit
            // a stateless request without using this exact-action card.
            changed_body["instructions"] = json!("different context");
        }
        assert!(
            !stateless_inference(path, &changed_body),
            "path={path} original={body} changed={changed_body}"
        );
        let (status, refusal) =
            model_call(&f, "compact-a", path, &changed_body, Some(&acknowledgement)).await;
        assert_eq!(
            status, 409,
            "path={path} original={body} changed={changed_body}: {refusal}"
        );
        assert_eq!(
            refusal["error"], "acknowledgement_invalid",
            "path={path} original={body} changed={changed_body}: {refusal}"
        );
        let mut changed_model = body.clone();
        changed_model["model"] = json!("another-model");
        let (status, refusal) = model_call(
            &f,
            "compact-a",
            path,
            &changed_model,
            Some(&acknowledgement),
        )
        .await;
        assert_eq!(status, 409, "{refusal}");
        assert_eq!(refusal["error"], "acknowledgement_invalid", "{refusal}");
        let (status, refusal) =
            model_call(&f, "compact-b", path, &body, Some(&acknowledgement)).await;
        assert_eq!(status, 409, "{refusal}");
        assert_eq!(refusal["error"], "service_selection_required", "{refusal}");
        assert!(a.received_requests().await.unwrap().is_empty());
        assert!(b.received_requests().await.unwrap().is_empty());
        let (status, result) =
            model_call(&f, "compact-a", path, &body, Some(&acknowledgement)).await;
        assert_eq!(status, 200, "{result}");
        assert_eq!(a.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            f.state
                .db
                .collection::<Document>(COLLECTION)
                .count_documents(doc! {"kind":"model_consent"})
                .await
                .unwrap(),
            0
        );
    }
}

struct CompactEgress {
    callback_url: String,
    path: &'static str,
    body: Value,
}

#[async_trait::async_trait]
impl contract::NyxAgentEgress for CompactEgress {
    async fn request(
        &self,
        _: &contract::NyxAgentExecutionContext,
        credential: &contract::RuntimeCredential,
        request: contract::NyxAgentRequest,
    ) -> AppResult<Response> {
        if request.method != "POST" {
            return Ok(Response::new(Body::from("{}")));
        }
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap()
            .post(format!(
                "{}/api/v1/proxy/s/compact-org/v1/{}",
                self.callback_url, self.path
            ))
            .header("x-api-key", credential.raw_key.as_str())
            .json(&self.body)
            .send()
            .await
            .expect("remote model callback transport");
        let status = response.status().as_u16();
        let result: Value = response.json().await.expect("model callback response body");
        if status == 409 {
            assert_eq!(result["error"], "acknowledgement_required", "{result}");
            return Err(AppError::Internal(
                "model compaction callback refused".into(),
            ));
        }
        assert_eq!(status, 200, "{result}");
        let terminal = json!({"type":"response.completed","sequence_number":1,"response":{
            "id":"resp_1234567890abcdef1234567890abcdef_1234567890abcdef1234567890abcdef",
            "conversation":{"id":"conv_1234567890abcdef1234567890abcdef"},"status":"completed",
            "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Compacted reply"}]}]}});
        Ok(Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(format!("data: {terminal}\n\n")))
            .unwrap())
    }
}

#[tokio::test]
async fn org_stateful_model_transport_full_access_and_auto_resume_keep_exact_consent_without_tool_grant()
 {
    for (path, body) in exact_model_requests() {
        for setting in [AccessMode::Autopilot, AccessMode::Nyxmode] {
            let mut f = orchestrator_fixture("org_compact_runtime").await;
            mode(&f, setting).await;
            let owner = org(&f).await;
            let catalog = catalog(&f).await;
            let provider = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path(format!("/v1/{path}")))
                .respond_with(
                    wiremock::ResponseTemplate::new(200).set_body_json(json!({"output":[]})),
                )
                .mount(&provider)
                .await;
            compact_connection(&f, &owner, &catalog, "compact-org", &provider.uri()).await;
            grant(&f, &Uuid::new_v4().to_string()).await;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let callback_url = format!("http://{}", listener.local_addr().unwrap());
            let (_, router) = crate::routes::build_router();
            let router = router.with_state(f.state.clone());
            let _server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            }));
            let initial = journeys::execute(
                &f,
                &f.row,
                &CompactEgress {
                    callback_url: callback_url.clone(),
                    path,
                    body: body.clone(),
                },
            )
            .await
            .unwrap();
            assert!(initial.error.is_none(), "{:?}", initial.error);
            if setting == AccessMode::Autopilot {
                assert_eq!(initial.text, "Compacted reply");
            } else {
                assert!(
                    initial.text.contains("confirmation card"),
                    "{}",
                    initial.text
                );
                assert!(provider.received_requests().await.unwrap().is_empty());
                let card = f
                    .state
                    .db
                    .collection::<Document>("assistant_acknowledgements")
                    .find_one(doc! {"conversation_id":&f.row.id,"status":"pending",
                    "tool_name":"nyxid.service_execution"})
                    .await
                    .unwrap()
                    .unwrap();
                let acknowledgement = card.get_str("_id").unwrap().to_owned();
                idle(&mut f).await;
                acks::decide(&f.state.db, &f.owner, &f.row.id, &acknowledgement, true)
                    .await
                    .unwrap();
                let resumed = engine::begin_turn(
                    &f.state.db,
                    &f.owner,
                    &engine::TurnRequest {
                        acknowledgement_id: Some(acknowledgement.clone()),
                        retry_of: None,
                        attachment_ids: vec![],
                        agent_id: None,
                        conversation_id: Some(f.row.id.clone()),
                        text: "Continue".into(),
                        model: None,
                        access_mode: None,
                    },
                    &f.state.encryption_keys,
                )
                .await
                .unwrap();
                let result = journeys::execute(
                    &f,
                    &resumed,
                    &CompactEgress {
                        callback_url: callback_url.clone(),
                        path,
                        body: body.clone(),
                    },
                )
                .await
                .unwrap();
                assert!(result.error.is_none(), "{:?}", result.error);
                assert_eq!(result.text, "Compacted reply");
                journeys::settled(&f, &resumed, &result).await;
                let next = engine::begin_turn(
                    &f.state.db,
                    &f.owner,
                    &engine::TurnRequest {
                        acknowledgement_id: None,
                        retry_of: None,
                        attachment_ids: vec![],
                        agent_id: None,
                        conversation_id: Some(f.row.id.clone()),
                        text: "Another turn".into(),
                        model: None,
                        access_mode: None,
                    },
                    &f.state.encryption_keys,
                )
                .await
                .unwrap();
                let result = journeys::execute(
                    &f,
                    &next,
                    &CompactEgress {
                        callback_url: callback_url.clone(),
                        path,
                        body: body.clone(),
                    },
                )
                .await
                .unwrap();
                assert!(result.error.is_none(), "{:?}", result.error);
                assert!(result.text.contains("confirmation card"), "{}", result.text);
                let (status, refusal) =
                    model_call(&f, "compact-org", path, &body, Some(&acknowledgement)).await;
                assert_eq!(status, 409, "{refusal}");
                assert_eq!(refusal["error"], "acknowledgement_invalid", "{refusal}");
            }
            assert_eq!(provider.received_requests().await.unwrap().len(), 1);
            assert_eq!(
                f.state
                    .db
                    .collection::<Document>(COLLECTION)
                    .count_documents(doc! {"kind":"model_consent"})
                    .await
                    .unwrap(),
                0
            );
        }
    }
}

#[tokio::test]
async fn org_owned_specialist_model_transport_keeps_only_the_implicit_scope_exception() {
    let org_fixture =
        crate::services::org_agent_tests::Fixture::new("org_model_implicit_scope").await;
    let (_, thread) = org_fixture.create().await;
    let row = engine::begin_turn(
        &org_fixture.state.db,
        &org_fixture.admin,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
            conversation_id: Some(thread.id.clone()),
            agent_id: None,
            text: "Use my thinking model".into(),
            model: None,
            access_mode: None,
        },
        &org_fixture.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(
        row.agent_owner_id.as_deref(),
        Some(org_fixture.org.as_str())
    );
    let key = crate::services::key_service::get_api_key(
        &org_fixture.state.db,
        &org_fixture.admin,
        &row.credential_api_key_id,
    )
    .await
    .unwrap();
    let auth = crate::mw::auth::api_key_auth_user(&org_fixture.state.db, &key, None, None, None)
        .await
        .unwrap();
    let chat = acks::for_key_with_access(
        &org_fixture.state.db,
        &org_fixture.admin,
        Some(&key.id),
        auth.org_agent_access.as_ref(),
    )
    .await
    .unwrap()
    .unwrap();
    let f = Fixture {
        state: org_fixture.state.clone(),
        owner: org_fixture.admin.clone(),
        nyxbot_thread: row.id.clone(),
        row,
        chat,
        auth,
    };
    let catalog = catalog(&f).await;
    let provider = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"output":[]})))
        .mount(&provider)
        .await;
    let connection =
        compact_connection(&f, &f.owner, &catalog, "specialist-model", &provider.uri()).await;
    assert!(!f.auth.allow_all_services);
    assert!(!f.auth.allowed_service_ids.contains(&connection));
    for (path, body) in exact_model_requests().into_iter().chain(std::iter::once((
        "responses",
        json!({"model":"m","store":false,"input":"stateless"}),
    ))) {
        let (status, result) = model_call(&f, "specialist-model", path, &body, None).await;
        assert_eq!(status, 200, "{path}: {result}");
    }
    let sent = provider.received_requests().await.unwrap().len();
    assert_eq!(sent, 7);
    // This inference catalog does not authorize arbitrary provider resources.
    let (status, refusal) = model_call(
        &f,
        "specialist-model",
        "files",
        &json!({"model":"m","file":"account_a"}),
        None,
    )
    .await;
    assert_eq!(status, 403, "{refusal}");
    assert_eq!(provider.received_requests().await.unwrap().len(), sent);
    // Explicit/all-service access still uses the live organization boundary.
    f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.chat.api_key_id},
            doc! {"$set":{"allow_all_services":true}},
        )
        .await
        .unwrap();
    let (status, refusal) = model_call(
        &f,
        "specialist-model",
        "responses",
        &json!({"model":"m","store":false,"input":"exact model"}),
        None,
    )
    .await;
    assert_eq!(status, 403, "{refusal}");
    assert_eq!(provider.received_requests().await.unwrap().len(), sent);
    f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.chat.api_key_id},
            doc! {"$set":{"allow_all_services":false}},
        )
        .await
        .unwrap();
    org_fixture.revoke(&org_fixture.admin).await;
    let (status, refusal) = model_call(
        &f,
        "specialist-model",
        "responses",
        &json!({"model":"m","store":false,"input":"revoked member"}),
        None,
    )
    .await;
    assert_eq!(status, 403, "{refusal}");
    assert_eq!(provider.received_requests().await.unwrap().len(), sent);
}
