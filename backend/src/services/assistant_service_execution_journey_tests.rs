//! Production gateway callbacks exercised through the owner runtime.
use super::*;
use crate::services::{
    assistant_agent_credential_service as credentials, assistant_nyxagent as engine,
    assistant_nyxagent_runtime as runtime, platform_manager_contract as contract,
};
use axum::{
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{Request, StatusCode},
    response::Response,
};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

async fn provider(f: &Fixture, catalog: &str) {
    let id = Uuid::new_v4().to_string();
    f.state.db.collection::<Document>("provider_configs").insert_one(doc! {
        "_id":&id,"slug":"openai","name":"OpenAI","provider_type":"api_key","is_active":true,
        "created_by":&f.owner,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":catalog},
            doc! {"$set":{"slug":"llm-openai","provider_config_id":id,"service_category":"llm"}},
        )
        .await
        .unwrap();
}
async fn endpoint(f: &Fixture, connection: &str, url: &str) {
    let row =
        crate::services::user_service_service::find_user_service_by_id(&f.state.db, connection)
            .await
            .unwrap()
            .unwrap();
    f.state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(doc! {"_id":row.endpoint_id}, doc! {"$set":{"url":url}})
        .await
        .unwrap();
}
async fn auth(state: &crate::AppState, owner: &str, key: &str) -> crate::mw::auth::AuthUser {
    let key = crate::services::key_service::get_api_key(&state.db, owner, key)
        .await
        .unwrap();
    crate::mw::auth::api_key_auth_user(&state.db, &key, None, None, None)
        .await
        .unwrap()
}
async fn model(
    state: &crate::AppState,
    owner: &str,
    key: &str,
    body: Value,
) -> AppResult<Response> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/v1/llm/openai/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    request.extensions_mut().insert(
        crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
            crate::services::billing::BillingIngress::LlmProvider,
        ),
    );
    Box::pin(crate::handlers::llm_gateway::llm_proxy_request(
        State(state.clone()),
        auth(state, owner, key).await,
        Path(("openai".into(), "chat/completions".into())),
        request,
    ))
    .await
}
fn generation(text: &str) -> Value {
    json!({"model":"m","messages":[{"role":"user","content":text}],
        "tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object",
        "properties":{"prompt":{"type":"string"},"file_id":{"type":"string"}}}}},{"type":"web_search"}]})
}
pub(super) async fn settled(
    f: &Fixture,
    row: &crate::models::assistant_conversation::AssistantConversation,
    result: &engine::TurnResult,
) {
    engine::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        result,
    )
    .await
    .unwrap();
}
fn request(id: Option<&str>, text: &str, ack: Option<&str>) -> engine::TurnRequest {
    engine::TurnRequest {
        acknowledgement_id: ack.map(str::to_owned),
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        conversation_id: id.map(str::to_owned),
        text: text.into(),
        model: None,
        access_mode: None,
    }
}

async fn github_search_named(f: &Fixture, name: &str) -> Value {
    use tower::ServiceExt;
    let key = credentials::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let (_, router) = crate::routes::build_router();
    // Match the server's request task without retaining the fixture's polling
    // frame. Abandoning this helper still cancels the request.
    let response = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(
        router.with_state(f.state.clone()).oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .header("x-api-key", key.raw_key.as_str())
                .body(Body::from(
                    json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":name,"arguments":{"q":"nyxid in:name"}}})
                    .to_string(),
                ))
                .unwrap(),
        ),
    ))
    .await
    .unwrap()
    .unwrap();
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert!(body.get("error").is_none(), "{body}");
    body["result"].clone()
}

async fn github_fixture() -> (Fixture, String, String, MockServer, MockServer) {
    let f = orchestrator_fixture("github_inline_selection_2022").await;
    let mut catalog = test_utils::test_auto_connected_catalog_service();
    catalog.slug = "api-github".into();
    catalog.name = "GitHub".into();
    catalog.requires_user_credential = true;
    let catalog_id = catalog.id.clone();
    f.state
        .db
        .collection("downstream_services")
        .insert_one(catalog)
        .await
        .unwrap();
    let mut operation = test_utils::test_service_endpoint(
        &catalog_id,
        "search_repositories",
        "GET",
        "/search/repositories",
    );
    operation.parameters =
        Some(json!([{"name":"q","in":"query","required":true,"schema":{"type":"string"}}]));
    f.state
        .db
        .collection("service_endpoints")
        .insert_one(operation)
        .await
        .unwrap();
    let chosen_server = MockServer::start().await;
    let other_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(wiremock::matchers::path("/search/repositories"))
        .and(wiremock::matchers::query_param("q", "nyxid in:name"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"total_count":1,"items":[{"name":"NyxID"}]})),
        )
        .mount(&chosen_server)
        .await;
    let chosen = super::compaction::compact_connection(
        &f,
        &f.owner,
        &catalog_id,
        "api-github",
        &chosen_server.uri(),
    )
    .await;
    let other = super::compaction::compact_connection(
        &f,
        &f.owner,
        &catalog_id,
        "github-other",
        &other_server.uri(),
    )
    .await;
    f.state.db.collection::<Document>("service_preferences").insert_one(doc! {
        "_id":&f.owner,"ordered":[&other,&chosen],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
    (f, chosen, other, chosen_server, other_server)
}

async fn github_search(f: &Fixture) -> Value {
    github_search_named(f, "api-github__search_repositories").await
}

#[tokio::test]
async fn github_owner_inline_service_selection_unblocks_retried_mcp_search() {
    let (f, chosen, _, chosen_server, other_server) = github_fixture().await;
    let refused = github_search(&f).await;
    assert_eq!(refused["isError"], true, "{refused}");
    let refusal: Value =
        serde_json::from_str(refused["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        refusal["error"], "service_target_selection_required",
        "{refusal}"
    );
    assert_eq!(refusal["proposed_connection_id"], chosen);
    assert!(chosen_server.received_requests().await.unwrap().is_empty());
    settled(
        &f,
        &f.row,
        &engine::TurnResult {
            text: "Choose a connection".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await;
    let row = engine::begin_turn(
        &f.state.db,
        &f.owner,
        &request(
            Some(&f.row.id),
            &format!(
                "/service {} continue the search",
                refusal["proposed_connection_id"].as_str().unwrap()
            ),
            None,
        ),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let retried = github_search(&f).await;
    assert_eq!(
        retried["isError"], false,
        "owner selection must unblock the same tool: {retried}"
    );
    assert_eq!(chosen_server.received_requests().await.unwrap().len(), 1);
    assert!(other_server.received_requests().await.unwrap().is_empty());
    // Every subsequent tool round and automatic continuation uses this choice.
    // Simulate selection metadata aging past the old 35-minute TTL while a
    // continuation keeps the logical turn live. The account must stay pinned.
    let selection = f.state.db.collection::<Document>(COLLECTION).find_one(
        doc! {"kind":"selection","conversation_id":&row.id,"turn_id":&row.active_turn.as_ref().unwrap().turn_id},
    ).await.unwrap().unwrap();
    let aged = selection.get_datetime("expires_at").unwrap().to_chrono() - Duration::minutes(36);
    f.state
        .db
        .collection::<Document>(COLLECTION)
        .update_one(
            doc! {"_id":selection.get_str("_id").unwrap()},
            doc! {"$set":{"expires_at":bson::DateTime::from_chrono(aged)}},
        )
        .await
        .unwrap();
    let (sender, _) = tokio::sync::broadcast::channel(8);
    let mut events = runtime::Events {
        sender,
        checkpoint: None,
        cursor: 0,
        reset_notified: false,
    };
    let mut continuations = crate::services::assistant_continuation::Continuations::new(2);
    assert!(
        runtime::continue_turn(
            &f.state,
            &row,
            &mut events,
            &mut continuations,
            Some("conv_1234567890abcdef1234567890abcdef"),
            &engine::TurnError::new("tool_budget_exhausted"),
            "Searching repositories"
        )
        .await
        .unwrap()
    );
    assert_eq!(github_search(&f).await["isError"], false);
    let wrong = github_search_named(&f, "github-other__search_repositories").await;
    assert_eq!(wrong["isError"], true, "{wrong}");
    let refusal: Value =
        serde_json::from_str(wrong["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(refusal["reason"], "wrong_connection");
    assert_eq!(refusal["selected_connection_id"], chosen);
    assert!(
        refusal["instructions"]
            .as_str()
            .unwrap()
            .contains("Do not ask the owner to repeat")
    );
    assert_eq!(chosen_server.received_requests().await.unwrap().len(), 2);
    assert!(other_server.received_requests().await.unwrap().is_empty());
    settled(
        &f,
        &row,
        &engine::TurnResult {
            text: "Found NyxID".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await;
    engine::begin_turn(
        &f.state.db,
        &f.owner,
        &request(Some(&row.id), "A new request without a selection", None),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(github_search(&f).await["isError"], true);
    assert_eq!(chosen_server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn github_owner_newline_picker_selection_unblocks_mcp_search() {
    let (f, chosen, _, chosen_server, other_server) = github_fixture().await;
    settled(
        &f,
        &f.row,
        &engine::TurnResult {
            text: "Choose".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await;
    engine::begin_turn(
        &f.state.db,
        &f.owner,
        &request(
            Some(&f.row.id),
            &format!("/service {chosen}\nContinue"),
            None,
        ),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(github_search(&f).await["isError"], false);
    assert_eq!(chosen_server.received_requests().await.unwrap().len(), 1);
    assert!(other_server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn github_owner_channel_selection_uses_original_message_not_sender_prefix() {
    use crate::models::assistant_conversation::TurnOrigin;
    let (f, chosen, _, chosen_server, other_server) = github_fixture().await;
    settled(
        &f,
        &f.row,
        &engine::TurnResult {
            text: "Choose".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await;
    let input = format!("/service {chosen} continue the search");
    let mut start = engine::TurnStart::from(&request(
        Some(&f.row.id),
        &format!("Calvin (owner): {input}"),
        None,
    ));
    start.origin = TurnOrigin::Channel;
    start.question = Some(input);
    engine::begin_turn(&f.state.db, &f.owner, &start, &f.state.encryption_keys)
        .await
        .unwrap();
    assert_eq!(github_search(&f).await["isError"], false);
    assert_eq!(chosen_server.received_requests().await.unwrap().len(), 1);
    assert!(other_server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn invalid_owner_service_selection_has_actionable_reason_without_admitting_turn() {
    let (f, chosen, _, chosen_server, other_server) = github_fixture().await;
    settled(
        &f,
        &f.row,
        &engine::TurnResult {
            text: "Choose".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(doc! {"_id":&chosen}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    for (text, reason) in [
        ("/service".to_owned(), "invalid_selector"),
        (
            format!("/service {} continue", "a".repeat(129)),
            "invalid_selector",
        ),
        (
            format!("/service {} continue", Uuid::new_v4()),
            "connection_unavailable",
        ),
        (format!("/service {chosen} continue"), "connection_disabled"),
    ] {
        let error = engine::begin_turn(
            &f.state.db,
            &f.owner,
            &request(Some(&f.row.id), &text, None),
            &f.state.encryption_keys,
        )
        .await
        .expect_err("invalid selection must not become an ordinary unselected turn");
        let response = error.response_body();
        assert_eq!(response.error, "validation_error");
        assert_eq!(response.details.as_ref().unwrap()["reason"], reason);
        assert!(!response.message.contains("connection picker"));
        assert!(
            engine::get(&f.state.db, &f.owner, &f.row.id)
                .await
                .unwrap()
                .active_turn
                .is_none()
        );
    }
    assert!(chosen_server.received_requests().await.unwrap().is_empty());
    assert!(other_server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn service_selection_does_not_promote_guest_event_or_question_metadata() {
    use crate::models::assistant_conversation::TurnOrigin;
    let f = orchestrator_fixture("untrusted_selection_2022").await;
    let id = connection(&f, &f.owner, "github", None).await;
    for (origin, guest, text) in [
        (TurnOrigin::Channel, true, format!("/service {id}\nRead")),
        (TurnOrigin::Event, false, format!("/service {id}\nRead")),
        (
            TurnOrigin::User,
            false,
            "Quoted /service command".to_owned(),
        ),
    ] {
        let mut start = engine::TurnStart::from(&request(Some(&f.row.id), &text, None));
        start.origin = origin;
        start.guest = guest;
        start.question = Some(format!("/service {id}\nRead"));
        let mut session = f.state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        bind_owner_selection(&f.state.db, &mut session, &f.row, &start, &text)
            .await
            .unwrap();
        assert!(
            f.state
                .db
                .collection::<Document>(COLLECTION)
                .find_one(doc! {"conversation_id":&f.row.id,"kind":"selection"})
                .session(&mut session)
                .await
                .unwrap()
                .is_none()
        );
        session.abort_transaction().await.unwrap();
    }
}

#[tokio::test]
async fn actual_new_and_rotated_key_turns_capture_selection_and_reset() {
    let f = orchestrator_fixture("selection_transactions").await;
    let id = connection(&f, &f.owner, "chosen", None).await;
    let selected = engine::begin_turn(
        &f.state.db,
        &f.owner,
        &request(None, &format!("/service {id}\nRead"), None),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let selection = f
        .state
        .db
        .collection::<Document>(COLLECTION)
        .find_one(doc! {"conversation_id":&selected.id,"kind":"selection"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selection.get_str("connection_id"), Ok(id.as_str()));
    assert_eq!(
        selection.get_str("api_key_id"),
        Ok(selected.credential_api_key_id.as_str())
    );
    settled(
        &f,
        &selected,
        &engine::TurnResult {
            text: "done".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await;
    crate::services::key_service::rotate_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &selected.credential_api_key_id,
    )
    .await
    .unwrap();
    let rotated = engine::begin_turn(
        &f.state.db,
        &f.owner,
        &request(Some(&selected.id), &format!("/service {id}\nAgain"), None),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_ne!(
        rotated.credential_api_key_id,
        selected.credential_api_key_id
    );
    let live = acks::for_key(&f.state.db, &f.owner, Some(&rotated.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION)
            .count_documents(
                doc! {"kind":"selection","turn_id":&live.turn_id,"api_key_id":&live.api_key_id}
            )
            .await
            .unwrap(),
        1
    );
    settled(
        &f,
        &rotated,
        &engine::TurnResult {
            text: "done".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await;
    let plain = engine::begin_turn(
        &f.state.db,
        &f.owner,
        &request(Some(&rotated.id), "Ordinary request", None),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(
        f.state
            .db
            .collection::<Document>(COLLECTION)
            .count_documents(
                doc! {"kind":"selection","turn_id":&plain.active_turn.unwrap().turn_id}
            )
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn real_gateway_connect_failure_records_evidence_and_selects_next_preference() {
    let f = orchestrator_fixture("gateway_real_fallback").await;
    let c = catalog(&f).await;
    provider(&f, &c).await;
    let a = connection(&f, &f.owner, "first", Some(&c)).await;
    let b = connection(&f, &f.owner, "second", Some(&c)).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    endpoint(&f, &a, &unavailable).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1}}),
            ),
        )
        .mount(&server)
        .await;
    endpoint(&f, &b, &server.uri()).await;
    f.state.db.collection::<Document>("service_preferences").insert_one(doc! {"_id":&f.owner,"ordered":[&a,&b],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    assert!(
        model(
            &f.state,
            &f.owner,
            &f.chat.api_key_id,
            generation("initial")
        )
        .await
        .is_err()
    );
    let bucket = f
        .state
        .db
        .collection::<Document>(COLLECTION)
        .find_one(doc! {"kind":"unavailable_bucket","turn_id":&f.chat.turn_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bucket.get_array("entries").unwrap().len(), 1);
    let result = model(
        &f.state,
        &f.owner,
        &f.chat.api_key_id,
        generation("next round"),
    )
    .await
    .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    to_bytes(result.into_body(), 4096).await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    choose(&f, &a).await;
    assert!(
        model(
            &f.state,
            &f.owner,
            &f.chat.api_key_id,
            generation("explicit unavailable")
        )
        .await
        .is_err()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

struct CallbackEgress {
    callback_url: String,
    client: reqwest::Client,
    tool: String,
}
#[async_trait::async_trait]
impl contract::NyxAgentEgress for CallbackEgress {
    async fn request(
        &self,
        _: &contract::NyxAgentExecutionContext,
        key: &contract::RuntimeCredential,
        request: contract::NyxAgentRequest,
    ) -> AppResult<Response> {
        if request.method != "POST" {
            return Ok(Response::new(Body::from("{}")));
        }
        let first = self
            .client
            .post(format!(
                "{}/api/v1/llm/openai/v1/chat/completions",
                self.callback_url
            ))
            .header("x-api-key", key.raw_key.as_str())
            .json(&generation("model round one"))
            .send()
            .await
            .expect("initial or resumed model callback must resolve successfully");
        let status = first.status();
        let body = first.bytes().await.unwrap();
        assert!(body.len() <= if status.is_success() { 4096 } else { 8192 });
        if !status.is_success() {
            let refusal: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(status, StatusCode::CONFLICT, "{refusal}");
            assert_eq!(refusal["error"], "acknowledgement_required", "{refusal}");
            return Err(AppError::Internal("provider callback refused".into()));
        }
        let result = self
            .client
            .get(format!(
                "{}/api/v1/proxy/s/{}/read",
                self.callback_url, self.tool
            ))
            .header("x-api-key", key.raw_key.as_str())
            .send()
            .await
            .expect("tool callback after admitted model use must succeed");
        assert_eq!(result.status(), StatusCode::OK);
        assert!(result.bytes().await.unwrap().len() <= 4096);
        let next = self
            .client
            .post(format!(
                "{}/api/v1/llm/openai/v1/chat/completions",
                self.callback_url
            ))
            .header("x-api-key", key.raw_key.as_str())
            .json(&generation("tool result and next model round"))
            .send()
            .await
            .expect("subsequent model round must retain its bounded turn consent");
        assert_eq!(next.status(), StatusCode::OK);
        assert!(next.bytes().await.unwrap().len() <= 4096);
        let terminal = json!({"type":"response.completed","sequence_number":1,"response":{
            "id":"resp_1234567890abcdef1234567890abcdef_1234567890abcdef1234567890abcdef",
            "conversation":{"id":"conv_1234567890abcdef1234567890abcdef"},"status":"completed",
            "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Completed reply"}]}]}});
        Ok(Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(format!("data: {terminal}\n\n")))
            .unwrap())
    }
}
pub(super) async fn execute(
    f: &Fixture,
    row: &crate::models::assistant_conversation::AssistantConversation,
    egress: &dyn contract::NyxAgentEgress,
) -> Result<engine::TurnResult, engine::TurnError> {
    let mut credential =
        credentials::load_or_provision(&f.state.db, &f.state.encryption_keys, &f.owner, &row.id)
            .await
            .unwrap();
    let (sender, _) = tokio::sync::broadcast::channel(32);
    let mut events = runtime::Events {
        checkpoint: None,
        sender,
        cursor: 0,
        reset_notified: false,
    };
    Box::pin(runtime::execute_owner_turn(
        &f.state,
        egress,
        &runtime::owner_context(row).unwrap(),
        row,
        "Please answer",
        &mut credential,
        None,
        &mut events,
        "reply",
        &mut String::new(),
        &mut None,
    ))
    .await
}

#[tokio::test]
async fn actual_bootstrap_card_allow_resumes_model_tool_model_and_new_turn_asks_again() {
    let f = orchestrator_fixture("bootstrap_round_trip").await;
    mode(&f, AccessMode::Nyxmode).await;
    let owner = org(&f).await;
    let c = catalog(&f).await;
    provider(&f, &c).await;
    let id = connection(&f, &owner, "org-model", Some(&c)).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices":[]})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("tool result"))
        .mount(&server)
        .await;
    endpoint(&f, &id, &server.uri()).await;
    let _tool_id = connected(&f.state.db, &f.owner, "tool", &server.uri()).await;
    let tool = "tool".to_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let callback_url = format!("http://{}", listener.local_addr().unwrap());
    let (_, router) = crate::routes::build_router();
    let router = router.with_state(f.state.clone());
    let _callback_server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    let egress = CallbackEgress {
        callback_url,
        client: reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap(),
        tool,
    };
    let initial = execute(&f, &f.row, &egress).await.unwrap();
    assert!(initial.error.is_none());
    assert!(initial.text.contains("confirmation card"));
    assert!(server.received_requests().await.unwrap().is_empty());
    let card=f.state.db.collection::<Document>("assistant_acknowledgements").find_one(doc! {"conversation_id":&f.row.id,"status":"pending","tool_name":"nyxid.service_execution"}).await.unwrap().unwrap();
    let ack = card.get_str("_id").unwrap();
    assert!(
        card.get_str("summary")
            .unwrap()
            .contains("Organization connection")
    );
    settled(&f, &f.row, &initial).await;
    acks::decide(&f.state.db, &f.owner, &f.row.id, ack, true)
        .await
        .unwrap();
    let resumed = engine::begin_turn(
        &f.state.db,
        &f.owner,
        &request(Some(&f.row.id), "Continue", Some(ack)),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let result = execute(&f, &resumed, &egress).await.unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.text, "Completed reply");
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    settled(&f, &resumed, &result).await;
    let next = engine::begin_turn(
        &f.state.db,
        &f.owner,
        &request(Some(&f.row.id), "Unrelated turn", None),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let result = execute(&f, &next, &egress).await.unwrap();
    assert!(result.text.contains("confirmation card"));
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

#[tokio::test]
async fn selected_gateway_connection_removal_or_scope_change_never_calls_an_alternative() {
    for drift in ["disable", "delete", "scope"] {
        let f = orchestrator_fixture("selected_gateway_drift").await;
        let c = catalog(&f).await;
        provider(&f, &c).await;
        let a = connection(&f, &f.owner, "selected", Some(&c)).await;
        let b = connection(&f, &f.owner, "alternative", Some(&c)).await;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices":[]})))
            .mount(&server)
            .await;
        endpoint(&f, &a, &server.uri()).await;
        endpoint(&f, &b, &server.uri()).await;
        choose(&f, &a).await;
        match drift {
            "disable" => {
                f.state
                    .db
                    .collection::<Document>("user_services")
                    .update_one(doc! {"_id":&a}, doc! {"$set":{"is_active":false}})
                    .await
                    .unwrap();
            }
            "delete" => {
                f.state
                    .db
                    .collection::<Document>("user_services")
                    .delete_one(doc! {"_id":&a})
                    .await
                    .unwrap();
            }
            _ => {
                f.state.db.collection::<Document>("api_keys").update_one(doc! {"_id":&f.chat.api_key_id},doc! {"$set":{"assistant_operation_scopes":{&a:{"revision":1_i64,"catalog_service_id":&c,"operations":[{"rule":{"method":"GET","path_template":"/only"}}]}}}}).await.unwrap();
            }
        }
        assert!(
            model(
                &f.state,
                &f.owner,
                &f.chat.api_key_id,
                generation("selected operation")
            )
            .await
            .is_err(),
            "{drift}"
        );
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "{drift}"
        );
    }
}

#[tokio::test]
async fn gateway_legacy_personal_connection_precedes_org_without_an_org_card() {
    let f = orchestrator_fixture("gateway_legacy_precedence").await;
    mode(&f, AccessMode::Nyxmode).await;
    let c = catalog(&f).await;
    provider(&f, &c).await;
    let owner = org(&f).await;
    let id = connection(&f, &owner, "org-model", Some(&c)).await;
    let personal = MockServer::start().await;
    let organization = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices":[]})))
        .mount(&personal)
        .await;
    endpoint(&f, &id, &organization.uri()).await;
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&c},
            doc! {"$set":{"base_url":personal.uri(),"auth_method":"none"}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<crate::models::user_service_connection::UserServiceConnection>(
            "user_service_connections",
        )
        .insert_one(
            crate::models::user_service_connection::UserServiceConnection {
                secret_context_versions: Default::default(),
                id: Uuid::new_v4().to_string(),
                user_id: f.owner.clone(),
                service_id: c.clone(),
                credential_encrypted: None,
                credential_type: None,
                credential_label: None,
                metadata: None,
                is_active: true,
                state_version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    let response = model(
        &f.state,
        &f.owner,
        &f.chat.api_key_id,
        generation("legacy model"),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(personal.received_requests().await.unwrap().len(), 1);
    assert!(organization.received_requests().await.unwrap().is_empty());
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

struct InterruptedCallback {
    state: crate::AppState,
    callback_url: String,
    client: reqwest::Client,
    stop: bool,
    cancelled_stream: bool,
}
#[async_trait::async_trait]
impl contract::NyxAgentEgress for InterruptedCallback {
    async fn request(
        &self,
        _: &contract::NyxAgentExecutionContext,
        key: &contract::RuntimeCredential,
        request: contract::NyxAgentRequest,
    ) -> AppResult<Response> {
        if request.method != "POST" {
            return Ok(Response::new(Body::from("{}")));
        }
        let callback = self
            .client
            .post(format!(
                "{}/api/v1/llm/openai/v1/chat/completions",
                self.callback_url
            ))
            .header("x-api-key", key.raw_key.as_str())
            .json(&generation("bootstrap"))
            .send()
            .await
            .expect("bootstrap callback must return its policy refusal");
        let status = callback.status();
        let body = callback.bytes().await.unwrap();
        assert!(body.len() <= 8192);
        let refusal: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(status, StatusCode::CONFLICT, "{refusal}");
        assert_eq!(refusal["error"], "acknowledgement_required", "{refusal}");
        if self.stop {
            self.state
                .db
                .collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"credential_api_key_id":&key.api_key_id},
                    doc! {"$set":{"active_turn.stop_requested":true}},
                )
                .await?;
        }
        if self.cancelled_stream {
            let terminal = json!({"type":"response.failed","sequence_number":1,"response":{
                "id":"resp_1234567890abcdef1234567890abcdef_1234567890abcdef1234567890abcdef",
                "conversation":{"id":"conv_1234567890abcdef1234567890abcdef"},
                "status":"failed","output":[],"error":{"code":"cancelled"}}});
            return Ok(Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(format!("data: {terminal}\n\n")))
                .unwrap());
        }
        Err(AppError::Internal("upstream stopped".into()))
    }
}

#[tokio::test]
async fn actual_bootstrap_deny_and_stop_do_not_dispatch_or_turn_failure_into_waiting() {
    for (stop, cancelled_stream) in [(true, false), (false, true)] {
        let f = orchestrator_fixture("bootstrap_stop").await;
        mode(&f, AccessMode::Nyxmode).await;
        let c = catalog(&f).await;
        provider(&f, &c).await;
        let owner = org(&f).await;
        let id = connection(&f, &owner, "org-model", Some(&c)).await;
        let server = MockServer::start().await;
        endpoint(&f, &id, &server.uri()).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let callback_url = format!("http://{}", listener.local_addr().unwrap());
        let (_, router) = crate::routes::build_router();
        let router = router.with_state(f.state.clone());
        let _callback_server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }));
        let outcome = execute(
            &f,
            &f.row,
            &InterruptedCallback {
                state: f.state.clone(),
                callback_url,
                client: reqwest::Client::builder()
                    .no_proxy()
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .unwrap(),
                stop,
                cancelled_stream,
            },
        )
        .await;
        assert!(
            outcome.as_ref().is_err() || outcome.as_ref().is_ok_and(|r| r.error.is_some()),
            "stop={stop}, cancelled_stream={cancelled_stream}: {outcome:?}"
        );
        assert!(
            !outcome
                .as_ref()
                .is_ok_and(|r| r.text.contains("confirmation card") && r.error.is_none())
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        let card = f
            .state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .find_one(doc! {"conversation_id":&f.row.id,"status":"pending"})
            .await
            .unwrap()
            .unwrap();
        let ack = card.get_str("_id").unwrap();
        acks::decide(&f.state.db, &f.owner, &f.row.id, ack, false)
            .await
            .unwrap();
        let settled_result = engine::TurnResult {
            text: String::new(),
            session_id: None,
            response_id: None,
            error: Some(engine::TurnError::new("cancelled")),
        };
        settled(&f, &f.row, &settled_result).await;
        assert!(
            engine::begin_turn(
                &f.state.db,
                &f.owner,
                &request(Some(&f.row.id), "Continue", Some(ack)),
                &f.state.encryption_keys
            )
            .await
            .is_err()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn generic_gateway_org_primary_reaches_consent_before_legacy_codex_fallback() {
    for selected in [false, true] {
        let f = orchestrator_fixture("generic_gateway_org_primary").await;
        mode(&f, AccessMode::Nyxmode).await;
        let c = catalog(&f).await;
        provider(&f, &c).await;
        let owner = org(&f).await;
        let id = connection(&f, &owner, "org-openai", Some(&c)).await;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices":[]})))
            .mount(&server)
            .await;
        endpoint(&f, &id, &server.uri()).await;
        let codex = Uuid::new_v4().to_string();
        f.state.db.collection::<Document>("provider_configs").insert_one(doc! {
            "_id":&codex,"slug":"openai-codex","name":"Codex","provider_type":"oauth2",
            "is_active":true,"created_by":&f.owner,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        }).await.unwrap();
        let mut codex_catalog = test_utils::test_auto_connected_catalog_service();
        codex_catalog.slug = "llm-openai-codex".into();
        codex_catalog.provider_config_id = Some(codex.clone());
        f.state
            .db
            .collection("downstream_services")
            .insert_one(codex_catalog)
            .await
            .unwrap();
        f.state.db.collection::<Document>("user_provider_tokens").insert_one(doc! {
            "_id":Uuid::new_v4().to_string(),"user_id":&f.owner,"provider_config_id":codex,
            "token_type":"oauth2","status":"active","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        }).await.unwrap();
        if selected {
            choose(&f, &id).await;
        }
        let invoke = |ack: Option<String>| {
            let f = &f;
            async move {
                let mut request = Request::builder()
                    .method("POST")
                    .uri("/api/v1/llm/gateway/v1/chat/completions")
                    .header("content-type", "application/json");
                if let Some(ack) = ack {
                    request = request.header(ACK_HEADER, ack);
                }
                let mut request=request.body(Body::from(serde_json::to_vec(
                    &json!({"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}]}),
                ).unwrap())).unwrap();
                request.extensions_mut().insert(
                    crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                        crate::services::billing::BillingIngress::LlmGateway,
                    ),
                );
                crate::handlers::llm_gateway::gateway_request(
                    State(f.state.clone()),
                    auth(&f.state, &f.owner, &f.chat.api_key_id).await,
                    Path("chat/completions".into()),
                    request,
                )
                .await
            }
        };
        let response = invoke(None).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "selected={selected}"
        );
        let card: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        let ack = card["acknowledgement_id"].as_str().unwrap();
        let bound = f
            .state
            .db
            .collection::<Document>(COLLECTION)
            .find_one(doc! {"_id":ack})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bound.get_str("connection_id"), Ok(id.as_str()));
        assert!(server.received_requests().await.unwrap().is_empty());
        acks::decide(&f.state.db, &f.owner, &f.row.id, ack, true)
            .await
            .unwrap();
        let response = invoke(Some(ack.to_owned())).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
