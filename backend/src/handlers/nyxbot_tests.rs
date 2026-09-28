use super::*;
use crate::{
    models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        user::{COLLECTION_NAME as USERS, UserType},
    },
    test_utils::{connect_transaction_test_database, test_app_state, test_user},
};
use axum::{Router, response::sse::Event, routing::post};
use std::sync::Arc;
use tokio::sync::Mutex;

const OWNER: &str = "12345678-1234-4123-8123-1234567890ab";
const PARTITION: &str = "conv_0123456789abcdef0123456789abcdef";

type Calls = Arc<Mutex<Vec<Value>>>;

async fn setup(name: &str) -> (AppState, Calls, tokio::task::JoinHandle<()>) {
    let db = connect_transaction_test_database(name).await;
    engine::ensure_indexes(&db).await.unwrap();
    db.collection(USERS)
        .insert_one(test_user(OWNER, UserType::Person))
        .await
        .unwrap();
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let sink = calls.clone();
    let upstream = Router::new().route(
        "/v1/responses",
        post(move |Json(body): Json<Value>| {
            let sink = sink.clone();
            async move {
                sink.lock().await.push(body);
                let session = format!("conv_{}", Uuid::new_v4().simple());
                let response = format!("resp_{}_{}", &session[5..], Uuid::new_v4().simple());
                let stream = async_stream::stream! {
                    let completed = json!({
                        "type": "response.completed", "sequence_number": 1,
                        "response": {"id": response, "conversation": {"id": session},
                            "status": "completed", "output": [{"type": "message",
                            "role": "assistant", "content": [{"type": "output_text",
                            "text": "Here is your answer"}]}]},
                    });
                    yield Ok::<_, Infallible>(Event::default().data(completed.to_string()));
                };
                axum::response::Sse::new(stream).into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    let mut row = crate::models::downstream_service::test_helpers::dummy_service();
    row.id = Uuid::new_v4().to_string();
    row.slug = engine::SERVICE_SLUG.into();
    row.base_url = format!("http://{address}");
    row.service_category = "internal".into();
    row.auth_method = "none".into();
    row.requires_user_credential = false;
    row.forward_access_token = true;
    row.inject_delegation_token = false;
    row.credential_encrypted.clear();
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(row)
        .await
        .unwrap();
    (test_app_state(db), calls, server)
}

async fn key(state: &AppState, name: &str) -> key_service::CreatedApiKey {
    key_service::create_api_key(
        &state.db,
        OWNER,
        name,
        "read proxy",
        None,
        None,
        Some(&[]),
        Some(&[]),
        Some(false),
        Some(false),
        Some(false),
        None,
        None,
        Some("generic"),
        None,
    )
    .await
    .unwrap()
}

/// An active channel as `connect` leaves it, without the gateway round trip.
async fn channel(state: &AppState, transport: &str) -> (NyxbotChannel, String) {
    let route = key(state, "route").await;
    let agent = key(state, "agent").await;
    let now = Utc::now();
    let row = NyxbotChannel {
        id: Uuid::new_v4().to_string(),
        user_id: OWNER.into(),
        channel_bot_id: Uuid::new_v4().to_string(),
        platform: if transport == "gateway" { "telegram" } else { "lark" }.into(),
        bot_label: "Helper bot".into(),
        bot_username: Some("helper_bot".into()),
        transport: transport.into(),
        status: "active".into(),
        last_error: None,
        route_api_key_id: route.id.clone(),
        route_id: None,
        agent_api_key_id: Some(agent.id.clone()),
        agent_key_ciphertext: Some(
            state
                .encryption_keys
                .encrypt(agent.full_key.as_bytes())
                .await
                .unwrap(),
        ),
        gateway_channel_id: None,
        gateway_record_id: None,
        gateway_version: None,
        binding_id: None,
        owner_sender_ids: Vec::new(),
        link_code_hash: None,
        link_code_expires_at: None,
        source_conversation_id: None,
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .insert_one(&row)
        .await
        .unwrap();
    (row, agent.full_key)
}

fn bearer(key: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {key}").parse().unwrap());
    headers
}

fn binding_body(agent_key: &str, owner: &str) -> Value {
    json!({"schema_version": 1, "profile_digest": "sha256:x", "agent_key": agent_key,
        "tools": {"native_mcp_url": "https://gateway.example/mcp", "nyxid_service_slug": "cmaeg"},
        "profile": {"schema_version": 1,
            "metadata": {"name": "NyxBot", "owner": {"subject": owner, "kind": "human"},
                "provider": "nyxbot"},
            "model": {"access": "managed", "service": "llm-nyx", "api_type": "responses",
                "model": "nyxagent/chat", "thinking_effort": "medium"},
            "permission": {"action_mode": "autonomous", "service_scope": []},
            "context": {"system_prompt": "", "skills": [], "mcp_servers": []}}})
}

async fn body_text(response: Response) -> String {
    String::from_utf8(
        axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

fn event(text: &str, sender: &str, event_id: &str) -> Value {
    json!({
        "conversation": PARTITION, "stream": true,
        "input": [{"role": "user", "content": [{"type": "input_text", "text": text}]}],
        "metadata": {"cmaeg.event": event_id},
        "event_context": {"schema_version": 1, "channel_id": "cmaeg1.ch.test",
            "conversation_id": PARTITION,
            "activity": {"schema_version": 1, "event_id": event_id,
                "kind": {"type": "message", "text": text, "attachments": [], "mentions_bot": false},
                "actor": {"id": sender, "display_name": "Alice", "kind": "human"},
                "conversation": {"id": "chat-1", "kind": "private"},
                "occurred_at": "2026-09-28T00:00:00Z",
                "source": {"type": "nyxid_relay", "platform": "telegram", "route_id": "route"}},
            "event_ref": "cmaeg1.ev.sealed-reference"},
    })
}

async fn respond(state: &AppState, agent_key: &str, body: &Value, idempotency: &str) -> Response {
    let mut headers = bearer(agent_key);
    headers.insert("idempotency-key", idempotency.parse().unwrap());
    responses(
        State(state.clone()),
        headers,
        Bytes::from(serde_json::to_vec(body).unwrap()),
    )
    .await
}

#[tokio::test]
async fn provider_binding_requires_the_channel_agent_key_and_its_owner() {
    let (state, _, server) = setup("nyxbot_binding").await;
    let (row, agent_key) = channel(&state, "gateway").await;
    let other = key(&state, "unrelated").await;
    let bind = |headers: HeaderMap, body: Value, id: &str| {
        put_binding(
            State(state.clone()),
            Path(id.to_owned()),
            headers,
            Json(body),
        )
    };
    let response = bind(
        bearer("nyxid_ag_not_a_key"),
        binding_body(&agent_key, OWNER),
        "bnd_1",
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    // A valid key that is not this channel's agent key has no channel.
    let response = bind(
        bearer(&other.full_key),
        binding_body(&other.full_key, OWNER),
        "bnd_1",
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = bind(
        bearer(&agent_key),
        binding_body(&other.full_key, OWNER),
        "bnd_1",
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = bind(
        bearer(&agent_key),
        binding_body(&agent_key, "someone-else"),
        "bnd_1",
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = bind(bearer(&agent_key), binding_body(&agent_key, OWNER), "bnd_1").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
    assert_eq!(body["binding_id"], "bnd_1");
    assert_eq!(body["level"], "l1");
    assert!(
        body["refused"]
            .as_array()
            .unwrap()
            .iter()
            .any(|refusal| refusal["field"] == "model")
    );
    // Re-binding the same ID is idempotent; another ID conflicts.
    let again = bind(bearer(&agent_key), binding_body(&agent_key, OWNER), "bnd_1").await;
    assert_eq!(again.status(), StatusCode::OK);
    let conflict = bind(bearer(&agent_key), binding_body(&agent_key, OWNER), "bnd_2").await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let deleted = delete_binding(
        State(state.clone()),
        Path("bnd_1".into()),
        bearer(&agent_key),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let saved = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert!(saved.binding_id.is_none());
    assert!(!format!("{saved:?}").contains(&agent_key));
    server.abort();
}

#[tokio::test]
async fn gateway_turns_admit_once_answer_only_the_verified_owner_and_keep_context_verbatim() {
    let (state, calls, server) = setup("nyxbot_turns").await;
    let (row, agent_key) = channel(&state, "gateway").await;
    let bound = put_binding(
        State(state.clone()),
        Path("bnd_turns".into()),
        bearer(&agent_key),
        Json(binding_body(&agent_key, OWNER)),
    )
    .await;
    assert_eq!(bound.status(), StatusCode::OK);
    // Unknown partitions ask the gateway to ensure the conversation first.
    let missing = respond(&state, &agent_key, &event("hi", "42", "evt-0"), "evt_0").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(body_text(missing).await.contains("conversation_not_found"));
    let ensured = put_conversation(
        State(state.clone()),
        Path(("bnd_turns".into(), PARTITION.into())),
        bearer(&agent_key),
    )
    .await;
    assert_eq!(ensured.status(), StatusCode::OK);
    // A stranger in a private chat gets a refusal and no turn.
    let stranger = body_text(
        respond(&state, &agent_key, &event("hi", "42", "evt-1"), "evt_1").await,
    )
    .await;
    assert!(stranger.contains("response.created"));
    assert!(stranger.contains("answers only its owner"), "{stranger}");
    assert!(stranger.contains("response.completed"));
    assert!(calls.lock().await.is_empty());
    // The owner links their account with the one-time code.
    let row = load_channel(&state, OWNER, &row.id).await.unwrap();
    let link = refresh_link_code(&state, &row).await.unwrap();
    assert_eq!(
        link.url.as_deref(),
        Some(format!("https://t.me/helper_bot?start={}", link.code.as_str()).as_str())
    );
    let linked = body_text(
        respond(
            &state,
            &agent_key,
            &event(&format!("/start {}", link.code.as_str()), "7", "evt-2"),
            "evt_2",
        )
        .await,
    )
    .await;
    assert!(linked.contains("Linked."), "{linked}");
    let row = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(row.owner_sender_ids, vec!["7".to_owned()]);
    assert!(row.link_code_hash.is_none());
    // The owner's message runs a Full-access NyxBot turn and returns its answer.
    let context = event("What changed today?", "7", "evt-3");
    let answer = body_text(respond(&state, &agent_key, &context, "evt_3").await).await;
    assert!(answer.contains("response.output_item.done"), "{answer}");
    assert!(answer.contains("Here is your answer"));
    assert!(answer.contains("response.completed"));
    {
        let calls = calls.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["input"], "What changed today?");
        let instructions = calls[0]["instructions"].as_str().unwrap();
        assert!(instructions.contains("verified this sender as the owner"));
        assert!(instructions.contains("channel bot through the"));
    }
    let conversation: AssistantConversation = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"user_id": OWNER, "channel.nyxbot_channel_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(conversation.channel.as_ref().unwrap().partition, PARTITION);
    let key = key_service::get_api_key(&state.db, OWNER, &conversation.credential_api_key_id)
        .await
        .unwrap();
    assert!(key.allow_all_services);
    // A retry of the same event never runs a second turn.
    let replay = body_text(respond(&state, &agent_key, &context, "evt_3").await).await;
    assert!(replay.contains("response.completed"));
    assert_eq!(calls.lock().await.len(), 1);
    // The admitted context is served back verbatim for readEventContext.
    let read = get_event_context(
        State(state.clone()),
        Path(("bnd_turns".into(), PARTITION.into(), "evt-3".into())),
        bearer(&agent_key),
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    let served: Value = serde_json::from_str(&body_text(read).await).unwrap();
    assert_eq!(served, context["event_context"]);
    let other = get_event_context(
        State(state.clone()),
        Path(("bnd_other".into(), PARTITION.into(), "evt-3".into())),
        bearer(&agent_key),
    )
    .await;
    assert_eq!(other.status(), StatusCode::NOT_FOUND);
    // Management test turns carry no context and run nothing.
    let test = body_text(
        respond(
            &state,
            &agent_key,
            &json!({"conversation": PARTITION, "stream": true, "input": [], "metadata": {}}),
            "test_1",
        )
        .await,
    )
    .await;
    assert!(test.contains("NyxBot is connected."));
    assert_eq!(calls.lock().await.len(), 1);
    server.abort();
}

#[tokio::test]
async fn direct_relay_accepts_only_nyxids_signed_callback_for_the_route_key() {
    let (state, _, server) = setup("nyxbot_direct").await;
    let (row, _) = channel(&state, "direct").await;
    let body = json!({
        "message_id": "msg-1", "correlation_id": "jti-1", "platform": "lark",
        "reply_token": "not-used", "agent": {"api_key_id": row.route_api_key_id, "name": "route"},
        "conversation": {"id": "route", "platform_id": "chat", "type": "private"},
        "sender": {"platform_id": "stranger"}, "content": {"type": "text", "text": "hello"},
        "timestamp": "2026-09-28T00:00:00Z",
    });
    let bytes = serde_json::to_vec(&body).unwrap();
    let sign = |api_key_id: &str, digest: &str| {
        crate::crypto::jwt::generate_relay_callback_token(
            &state.jwt_keys,
            &state.config,
            "jti-1",
            api_key_id,
            "msg-1",
            "lark",
            digest,
        )
        .unwrap()
    };
    let post = |token: String| {
        let mut headers = HeaderMap::new();
        headers.insert("x-nyxid-callback-token", token.parse().unwrap());
        relay_callback(
            State(state.clone()),
            Path(row.id.clone()),
            headers,
            Bytes::from(bytes.clone()),
        )
    };
    let digest = sha256_hex(&bytes);
    // Wrong key, wrong body digest, and a missing token are all refused.
    for token in [sign("other-key", &digest), sign(&row.route_api_key_id, "00")] {
        assert_eq!(post(token).await.status(), StatusCode::UNAUTHORIZED);
    }
    let unsigned = relay_callback(
        State(state.clone()),
        Path(row.id.clone()),
        HeaderMap::new(),
        Bytes::from(bytes.clone()),
    )
    .await;
    assert_eq!(unsigned.status(), StatusCode::UNAUTHORIZED);
    let accepted = post(sign(&row.route_api_key_id, &digest)).await;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    // Redelivery of the same message is deduplicated.
    let again = post(sign(&row.route_api_key_id, &digest)).await;
    assert_eq!(again.status(), StatusCode::ACCEPTED);
    assert_eq!(
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .count_documents(doc! {"channel_id": &row.id})
            .await
            .unwrap(),
        1
    );
    server.abort();
}
