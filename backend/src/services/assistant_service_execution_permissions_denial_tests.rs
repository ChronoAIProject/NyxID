//! Local web/channel replay. No live provider, channel delivery or callback capture.
use super::*;
use crate::{
    handlers::assistant_nyxagent::{AcknowledgementDecision, decide_acknowledgement},
    models::assistant_conversation::{ChannelOrigin, TurnOrigin},
    services::{
        assistant_agent_credential_service as credentials, assistant_nyxagent as engine,
        assistant_nyxagent_runtime as runtime, platform_manager_contract as contract,
    },
};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    response::Response,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn start(id: Option<&str>) -> engine::TurnStart {
    (&engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        conversation_id: id.map(str::to_owned),
        text: String::new(),
        model: None,
        access_mode: None,
    })
        .into()
}

struct ExplanationEgress {
    callback_url: String,
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl contract::NyxAgentEgress for ExplanationEgress {
    async fn request(
        &self,
        _: &contract::NyxAgentExecutionContext,
        key: &contract::RuntimeCredential,
        request: contract::NyxAgentRequest,
    ) -> AppResult<Response> {
        if request.method != "POST" {
            return Ok(Response::new(Body::from("{}")));
        }
        let input = request
            .body
            .as_ref()
            .and_then(|body| body.get("input"))
            .unwrap();
        assert!(input.to_string().contains(acks::DECISION_NARRATION));
        // An independent HTTP request: only the conversation key crosses this hop.
        let response = self
            .client
            .post(format!(
                "{}/api/v1/proxy/s/chrono-llm-public/responses",
                self.callback_url
            ))
            .header("x-api-key", key.raw_key.as_str())
            .json(&callback())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let bytes = response.bytes().await.unwrap();
        assert!(bytes.len() < 4096);
        let result: Value = serde_json::from_slice(&bytes).unwrap();
        let text = result["output_text"].as_str().unwrap();
        assert!(!text.is_empty());
        let terminal = json!({"type":"response.completed","sequence_number":1,"response":{
            "id":"resp_1234567890abcdef1234567890abcdef_1234567890abcdef1234567890abcdef",
            "conversation":{"id":"conv_1234567890abcdef1234567890abcdef"},"status":"completed",
            "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}]}});
        Ok(Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from(format!("data: {terminal}\n\n")))
            .unwrap())
    }
}

async fn explanation(
    f: &Fixture,
    row: &crate::models::assistant_conversation::AssistantConversation,
    egress: &ExplanationEgress,
) -> engine::TurnResult {
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
    let input = engine::turn_input(row, &start(Some(&row.id)));
    assert!(input.contains(acks::DECISION_NARRATION));
    Box::pin(runtime::execute_owner_turn(
        &f.state,
        egress,
        &runtime::owner_context(row).unwrap(),
        row,
        &input,
        &mut credential,
        None,
        &mut events,
        "permissions-reply",
        &mut String::new(),
        &mut None,
    ))
    .await
    .unwrap()
}

async fn replay(channel: bool, cancel: bool) {
    let mut f = orchestrator_fixture("permissions_denial_replay").await;
    mode(&f, AccessMode::AskEveryTime).await;
    compatibility::flag(&f, true).await;
    let destination = ChannelOrigin {
        native_event_id: None,
        nyxbot_channel_id: "sanitized-lark-channel".into(),
        partition: "sanitized-original-chat".into(),
        platform: "lark".into(),
        thread: None,
    };
    if channel {
        journeys::settled(
            &f,
            &f.row,
            &engine::TurnResult {
                text: "Fixture ready".into(),
                session_id: None,
                response_id: None,
                error: None,
            },
        )
        .await;
        let mut ingress = start(None);
        ingress.origin = TurnOrigin::Channel;
        ingress.channel = Some(destination.clone());
        ingress.text = "[sanitized]".into();
        f.row = engine::begin_turn(&f.state.db, &f.owner, ingress, &f.state.encryption_keys)
            .await
            .unwrap();
        f.chat = acks::for_key(&f.state.db, &f.owner, Some(&f.row.credential_api_key_id))
            .await
            .unwrap()
            .unwrap();
    }
    let provider = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"output_text":"The denied action was skipped. No changes were made."}),
        ))
        .mount(&provider)
        .await;
    let effects = MockServer::start().await;
    let c = catalog(&f).await;
    let model_id = connection(&f, &f.owner, "chrono-llm-public", Some(&c)).await;
    let row =
        crate::services::user_service_service::find_user_service_by_id(&f.state.db, &model_id)
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
    let effect_id = connected(&f.state.db, &f.owner, "permissions-effect", &effects.uri()).await;
    f.state
        .db
        .collection::<Document>("api_keys")
        .update_one(
            doc! {"_id":&f.chat.api_key_id},
            doc! {"$addToSet":{"allowed_service_ids":{"$each":[&model_id,&effect_id]}}},
        )
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let callback_url = format!("http://{}", listener.local_addr().unwrap());
    let (_, router) = crate::routes::build_router();
    let state = f.state.clone();
    let _server = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
        axum::serve(listener, router.with_state(state))
            .await
            .unwrap();
    }));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(15))
        .build()
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
    let model_url = format!("{callback_url}/api/v1/proxy/s/chrono-llm-public/responses");
    let effect_url = format!("{callback_url}/api/v1/proxy/s/permissions-effect/write");
    let response = client
        .post(&model_url)
        .header("x-api-key", key.raw_key.as_str())
        .json(&callback())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(response.bytes().await.unwrap().len() < 4096);
    let cards = f
        .state
        .db
        .collection::<Document>("assistant_acknowledgements");
    assert_eq!(
        cards
            .count_documents(doc! {"conversation_id":&f.row.id})
            .await
            .unwrap(),
        0
    );
    let response = client
        .post(&effect_url)
        .header("x-api-key", key.raw_key.as_str())
        .json(&json!({"operation":"sanitized-write"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    let bytes = response.bytes().await.unwrap();
    assert!(bytes.len() < 8192);
    let refusal: Value = serde_json::from_slice(&bytes).unwrap();
    let ack = refusal["acknowledgement_id"].as_str().unwrap().to_owned();
    assert!(effects.received_requests().await.unwrap().is_empty());

    let deny = || {
        decide_acknowledgement(
            State(f.state.clone()),
            test_utils::test_auth_user(&f.owner),
            Path((f.row.id.clone(), ack.clone())),
            Json(
                serde_json::from_value::<AcknowledgementDecision>(json!({"decision":"deny"}))
                    .unwrap(),
            ),
        )
    };
    // The handler's acknowledgement CAS admits exactly one decision/event.
    let (a, b) = tokio::join!(deny(), deny());
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(a.err().or(b.err()), Some(AppError::Conflict(_))));
    assert!(matches!(deny().await, Err(AppError::Conflict(_))));
    let reload = engine::get(&f.state.db, &f.owner, &f.row.id).await.unwrap();
    assert_eq!(reload.pending_events.len(), 1);
    let event = &reload.pending_events[0];
    assert_eq!(event.kind, "action_denied");
    assert!(!event.id.is_empty());
    assert!(event.text.contains("Do not execute or retry"));
    assert!(event.text.contains(acks::DECISION_NARRATION));
    assert_eq!(reload.channel, channel.then_some(destination.clone()));
    let response = client
        .post(&effect_url)
        .header("x-api-key", key.raw_key.as_str())
        .header(ACK_HEADER, &ack)
        .json(&json!({"operation":"sanitized-write"}))
        .send()
        .await
        .unwrap();
    assert!(!response.status().is_success());
    assert!(response.bytes().await.unwrap().len() < 8192);
    assert!(effects.received_requests().await.unwrap().is_empty());
    journeys::settled(
        &f,
        &f.row,
        &engine::TurnResult {
            text: "Waiting for owner decision".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await;
    // Reload/expiry cannot change a denied decision or invent an executable continuation.
    cards.update_one(doc! {"_id":&ack}, doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
    assert!(matches!(
        acks::decide(&f.state.db, &f.owner, &f.row.id, &ack, true).await,
        Err(AppError::Conflict(_))
    ));
    let mut event_start = start(Some(&f.row.id));
    event_start.origin = TurnOrigin::Event;
    let (a, b) = tokio::join!(
        engine::begin_turn(
            &f.state.db,
            &f.owner,
            &event_start,
            &f.state.encryption_keys
        ),
        engine::begin_turn(
            &f.state.db,
            &f.owner,
            &event_start,
            &f.state.encryption_keys
        ),
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let resumed = a.ok().or(b.ok()).unwrap();
    assert!(resumed.pending_events.is_empty());
    assert_eq!(resumed.channel, channel.then_some(destination.clone()));
    let routing = engine::answer_routing(&resumed, resumed.active_turn.as_ref().unwrap());
    assert_eq!(
        routing.primary,
        if channel {
            engine::PrimaryDestination::ChatUpdate {
                origins: vec![destination.clone()],
            }
        } else {
            engine::PrimaryDestination::App
        }
    );
    if cancel {
        engine::request_stop(&f.state.db, &f.owner, &resumed.id)
            .await
            .unwrap();
        let response = client
            .post(&model_url)
            .header("x-api-key", key.raw_key.as_str())
            .json(&callback())
            .send()
            .await
            .unwrap();
        assert!(!response.status().is_success());
        assert!(response.bytes().await.unwrap().len() < 8192);
        journeys::settled(
            &f,
            &resumed,
            &engine::TurnResult {
                text: String::new(),
                session_id: None,
                response_id: None,
                error: Some(engine::TurnError::new("cancelled")),
            },
        )
        .await;
    } else {
        let result = explanation(
            &f,
            &resumed,
            &ExplanationEgress {
                callback_url,
                client,
            },
        )
        .await;
        assert!(result.error.is_none());
        assert_eq!(
            result.text,
            "The denied action was skipped. No changes were made."
        );
        journeys::settled(&f, &resumed, &result).await;
    }
    let saved = engine::get(&f.state.db, &f.owner, &resumed.id)
        .await
        .unwrap();
    assert!(saved.active_turn.is_none());
    assert!(saved.pending_events.is_empty());
    assert_eq!(saved.channel, channel.then_some(destination));
    assert_eq!(
        cards
            .count_documents(doc! {"conversation_id":&f.row.id,"status":"pending"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        cards
            .count_documents(doc! {"conversation_id":&f.row.id,"status":"denied"})
            .await
            .unwrap(),
        1
    );
    assert!(effects.received_requests().await.unwrap().is_empty());
    assert_eq!(
        provider.received_requests().await.unwrap().len(),
        if cancel { 1 } else { 2 }
    );
    let messages = engine::messages(&f.state.db, &f.owner, &resumed.id, 100, None)
        .await
        .unwrap();
    let terminal: Vec<_> = messages
        .iter()
        .filter(|message| {
            message.turn_id == resumed.active_turn.as_ref().unwrap().turn_id
                && message.role == "assistant"
        })
        .collect();
    assert_eq!(terminal.len(), 1);
    assert_eq!(
        terminal[0].status,
        if cancel { "failed" } else { "completed" }
    );
    assert_eq!(
        terminal[0].error_code.as_deref(),
        cancel.then_some("cancelled")
    );
    if !cancel {
        assert!(!terminal[0].text.is_empty());
    }
}

#[tokio::test]
async fn codex_denial_web_continuation_is_terminal_without_a_second_model_card() {
    replay(false, false).await;
}

#[tokio::test]
async fn codex_denial_lark_continuation_preserves_the_original_destination() {
    replay(true, false).await;
}

#[tokio::test]
async fn codex_denial_cancelled_continuation_never_dispatches_or_leaves_a_pending_card() {
    replay(false, true).await;
}
